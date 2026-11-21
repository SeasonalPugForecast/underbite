import Foundation
import UnderbiteKit

/// Fetches the grid blocks the gridded layers need for the region on screen into the core's
/// `GridStore`: the requests from `grid_requests` (a coarse overview of the whole region first
/// when many blocks are missing, then the blocks nearest the centre, only those the store does
/// not hold fresh). Each pass first hands the store the blocks the ``GridBlockCache`` keeps on the
/// device that the core still counts fresh, and asks `grid_requests` again, so that kept blocks
/// (and an overview they make needless) are not fetched. The rest are each let through by the
/// `GridPacer` (ERDDAP's limits per server, kept across launches through the fetch host's store),
/// sent through the fetch host one at a time per server (ERDDAP answers several at once from one
/// client with "busy", which pauses the server for minutes), the servers side by side, and kept
/// in the cache once the store accepts them.
///
/// Offline (no route, see ``NetworkRoute``) nothing is fetched: every kept block is handed over
/// whatever its age, and the blocks with none are missing. A fetch that fails (at the transport,
/// or the server answers with an error or "busy") hands over the stale kept block instead. Each
/// block reports where it came from to ``MapOfflineStatus``, which badges the layer.
///
/// A new region starts a new pass; the old one sends nothing more (what it has in flight still
/// lands in the store). A server the pacer holds back is asked again once it may be, if the
/// region has not changed by then.
@MainActor
final class GridLoader {
    /// One block to fetch.
    nonisolated struct Job: Sendable {
        let layer: String
        /// The layer's time (Unix seconds as text), none for the newest: the badge's key.
        let time: String?
        let request: GridRequest
    }

    /// What a fetch child hands back: the job's place in its batch and how it went. One plain
    /// value, never a tuple holding strings (that crashed optimized builds in `TaskGroup.offer`).
    nonisolated struct Finished: Sendable {
        let index: Int
        let outcome: Outcome
    }

    /// What a block's fetch came to.
    nonisolated enum Outcome: Sendable {
        case added
        case noData
        case busy(after: UInt32)
        /// The server answered with something the core could not take.
        case failed
        /// The request failed at the transport: the device is offline.
        case unreachable
    }

    /// Servers asked at once, one block each.
    static let parallel = 3
    /// Times a pass asks the store again after adding kept blocks: each round reaches the next
    /// blocks out from the centre (the store lists at most 24 at a time).
    static let cacheRounds = 4
    /// The longest pause the loader waits out by itself before asking again.
    static let longestWait: UInt32 = 120

    let store: GridStore
    let cache: GridBlockCache
    private let host: FetchHost
    private let pacer: GridPacer
    private let clock: @Sendable () -> Int64
    private let offline: @Sendable () -> Bool
    private let status: MapOfflineStatus
    private var generation = 0
    /// The blocks being fetched, by key: a new pass does not ask for them again.
    private var inFlight: Set<String> = []
    /// A block of `layer` landed in the store.
    var onBlock: (String) -> Void = { _ in }
    /// Every block a pass asked for `layer` had no data (its dataset has nothing there or then).
    var onNoData: (String) -> Void = { _ in }

    private static let pacerNamespace = "map"
    private static let pacerKey = "pacer"

    /// - Parameters:
    ///   - cache: the blocks kept on the device; by default in the app's offline store.
    ///   - offline: whether the device has no route to the network.
    ///   - status: where the layers' badges are worked out.
    init(
        host: FetchHost, store: GridStore, clock: @escaping @Sendable () -> Int64,
        cache: GridBlockCache? = nil,
        offline: @escaping @Sendable () -> Bool = { NetworkRoute.shared.isOffline },
        status: MapOfflineStatus = MapOfflineStatus.shared
    ) {
        self.host = host
        self.store = store
        self.clock = clock
        self.cache = cache ?? GridBlockCache(store: MapOffline.store, host: host)
        self.offline = offline
        self.status = status
        pacer =
            host.load(namespace: Self.pacerNamespace, key: Self.pacerKey)
            .flatMap { try? GridPacer.fromJson(json: $0) } ?? GridPacer()
    }

    /// Starts loading what `layers` (the grid ones among them) need for `region`: the kept
    /// blocks first, then the network. The task ends with the pass (tests wait for it).
    @discardableResult
    func load(_ layers: [PlannedLayer], in region: MapRegion) -> Task<Void, Never>? {
        generation += 1
        let current = generation
        guard layers.contains(where: { $0.kind != .tiles }) else { return nil }
        return Task { await self.pass(layers, generation: current, region: region) }
    }

    /// Each grid layer's requests for `region` that are not in flight, in the store's order.
    private func requests(_ layers: [PlannedLayer], in region: MapRegion) -> [[Job]] {
        let now = clock()
        var perLayer: [[Job]] = []
        for layer in layers where layer.kind != .tiles {
            let requests =
                (try? store.gridRequests(
                    layer: layer.id, dataset: layer.dataset, time: layer.time,
                    west: region.west, south: region.south, east: region.east, north: region.north,
                    zoom: region.zoom, now: now)) ?? []
            let time = layer.time.map { String($0) }
            perLayer.append(
                requests.filter { !inFlight.contains($0.key) }.map {
                    Job(layer: layer.id, time: time, request: $0)
                })
        }
        return perLayer
    }

    /// One pass: what the cache holds fresh (offline: at any age), then what is still missing
    /// from the network (offline: nothing, and the layer says so).
    private func pass(_ layers: [PlannedLayer], generation current: Int, region: MapRegion) async {
        let pass = MapLoad.shared.interval("grid-pass")
        defer { pass.end() }
        var tried: Set<String> = []
        var held: Set<String> = []
        let offline = offline()
        for _ in 0..<Self.cacheRounds {
            let untried = requests(layers, in: region).joined().filter {
                !tried.contains($0.request.key)
            }
            guard !untried.isEmpty else { break }
            tried.formUnion(untried.map(\.request.key))
            let restored = await cache.restore(
                untried.map(\.request.key), into: store, now: clock(), allowingStale: offline)
            let byKey = Dictionary(
                restored.map { ($0.key, $0) }, uniquingKeysWith: { first, _ in first })
            held.formUnion(byKey.keys)
            for job in untried {
                guard let block = byKey[job.request.key] else { continue }
                say(job, offline ? .stored(fetchedAt: block.fetchedAt) : .current)
            }
            // The blocks are in the store whatever pass comes next, which will not list them
            // again: say so before giving way to it, or the layer is never drawn again.
            for layer in Set(untried.filter { byKey[$0.request.key] != nil }.map(\.layer)) {
                MapLoad.shared.mark(.block, layer: layer, source: "cache")
                onBlock(layer)
            }
            guard current == generation else { return }
            guard !restored.isEmpty else { break }
        }
        let jobs = Self.interleaved(requests(layers, in: region))
        guard !jobs.isEmpty else { return }
        if offline {
            // Blocks handed over stale are listed again; only those with nothing kept are
            // missing.
            for job in jobs where !held.contains(job.request.key) {
                say(job, .missing)
            }
            return
        }
        await run(jobs, generation: current, layers: layers, region: region)
    }

    /// Each layer's jobs in turn, so that every layer starts with the middle of the screen.
    nonisolated static func interleaved(_ lists: [[Job]]) -> [Job] {
        var out: [Job] = []
        let longest = lists.map(\.count).max() ?? 0
        for i in 0..<longest {
            for list in lists where i < list.count { out.append(list[i]) }
        }
        return out
    }

    private func run(_ jobs: [Job], generation current: Int, layers: [PlannedLayer], region: MapRegion) async {
        var pending = jobs
        var wait: UInt32?
        var asked: [String: Int] = [:]
        var empty: [String: Int] = [:]
        // The blocks the network settled (drawn, or nothing there), and those it could not reach.
        var settled: Set<String> = []
        var unreachable: Set<String> = []
        while !pending.isEmpty, current == generation {
            var batch: [Job] = []
            var i = 0
            while batch.count < Self.parallel, i < pending.count {
                let job = pending[i]
                let node = job.request.node
                if batch.contains(where: { $0.request.node == node }) {
                    i += 1
                    continue
                }
                let decision = try? pacer.maySend(node: node, now: clock())
                guard let decision, decision.send else {
                    // This server waits; the others go on.
                    wait = min(wait ?? .max, decision?.waitS ?? Self.longestWait)
                    pending.removeAll { $0.request.node == node }
                    continue
                }
                pending.remove(at: i)
                batch.append(job)
                inFlight.insert(job.request.key)
            }
            if batch.isEmpty { break }
            let (host, store, cache, pacer, clock) = (host, store, cache, pacer, clock)
            let finished = await withTaskGroup(of: Finished.self) { group in
                for (index, job) in batch.enumerated() {
                    group.addTask {
                        Finished(
                            index: index,
                            outcome: await Self.fetch(
                                job, host: host, store: store, cache: cache, pacer: pacer,
                                clock: clock))
                    }
                }
                var all: [Finished] = []
                for await one in group { all.append(one) }
                return all
            }
            for done in finished {
                let job = batch[done.index]
                let outcome = done.outcome
                inFlight.remove(job.request.key)
                asked[job.layer, default: 0] += 1
                switch outcome {
                case .added:
                    settled.insert(job.request.key)
                    say(job, .current)
                    MapLoad.shared.mark(.block, layer: job.layer, source: "network")
                    onBlock(job.layer)
                case .noData:
                    settled.insert(job.request.key)
                    empty[job.layer, default: 0] += 1
                case .busy(let after):
                    wait = min(wait ?? .max, after)
                    pending.removeAll { $0.request.node == job.request.node }
                case .failed:
                    break
                case .unreachable:
                    unreachable.insert(job.request.key)
                }
            }
        }
        host.store(namespace: Self.pacerNamespace, key: Self.pacerKey, value: pacer.toJson())
        if current == generation {
            await fallBack(
                jobs.filter { !settled.contains($0.request.key) }, unreachable: unreachable)
        }
        for (layer, count) in asked where empty[layer] == count {
            onNoData(layer)
        }
        guard let wait, wait <= Self.longestWait, current == generation else { return }
        Task {
            try? await Task.sleep(for: .seconds(Int(wait) + 1))
            if current == self.generation { _ = self.load(layers, in: region) }
        }
    }

    /// Hands the store the stale kept copies of `jobs` the network did not answer (a failure, a
    /// busy or held-back server); those of `unreachable` with none are missing.
    private func fallBack(_ jobs: [Job], unreachable: Set<String>) async {
        guard !jobs.isEmpty else { return }
        let restored = await cache.restore(
            jobs.map(\.request.key), into: store, now: clock(), allowingStale: true)
        let byKey = Dictionary(
            restored.map { ($0.key, $0) }, uniquingKeysWith: { first, _ in first })
        for job in jobs {
            if let block = byKey[job.request.key] {
                say(job, .stored(fetchedAt: block.fetchedAt))
            } else if unreachable.contains(job.request.key) {
                say(job, .missing)
            }
        }
        for layer in Set(jobs.filter { byKey[$0.request.key] != nil }.map(\.layer)) {
            MapLoad.shared.mark(.block, layer: layer, source: "cache-stale")
            onBlock(layer)
        }
    }

    private func say(_ job: Job, _ source: PieceSource) {
        status.report(layer: job.layer, date: job.time, piece: job.request.key, source)
    }

    /// Fetches one block and hands it to the store, then to the cache once the store took it;
    /// the pacer hears every answer.
    nonisolated private static func fetch(
        _ job: Job, host: FetchHost, store: GridStore, cache: GridBlockCache, pacer: GridPacer,
        clock: @Sendable () -> Int64
    ) async -> Outcome {
        let response: HostResponse
        do {
            response = try await host.send(request: job.request.request)
        } catch {
            return .unreachable
        }
        let now = clock()
        try? pacer.record(
            node: job.request.node, status: response.status,
            retryAfterS: response.retryAfter.flatMap {
                UInt32($0.trimmingCharacters(in: .whitespaces))
            }, now: now)
        do {
            try store.addGrid(key: job.request.key, response: response, now: now)
            await cache.save(
                job.request.key, body: response.body, contentType: response.contentType,
                fetchedAt: now, lifetime: UInt64(job.request.request.maxAgeS ?? 0))
            return .added
        } catch MapError.NoData {
            return .noData
        } catch MapError.RetryLater(let after) {
            return .busy(after: after)
        } catch {
            return .failed
        }
    }
}
