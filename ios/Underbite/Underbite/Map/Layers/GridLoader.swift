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
/// A new region starts a new pass; the old one sends nothing more (what it has in flight still
/// lands in the store). A server the pacer holds back is asked again once it may be, if the
/// region has not changed by then.
@MainActor
final class GridLoader {
    /// One block to fetch.
    nonisolated struct Job: Sendable {
        let layer: String
        let request: GridRequest
    }

    /// What a block's fetch came to.
    /// What a fetch child hands back: the job's place in its batch and how it went. One plain
    /// value, never a tuple holding strings (that crashed optimized builds in `TaskGroup.offer`).
    nonisolated struct Finished: Sendable {
        let index: Int
        let outcome: Outcome
    }

    nonisolated enum Outcome: Sendable {
        case added
        case noData
        case busy(after: UInt32)
        case failed
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
    private var generation = 0
    /// The blocks being fetched, by key: a new pass does not ask for them again.
    private var inFlight: Set<String> = []
    /// A block of `layer` landed in the store.
    var onBlock: (String) -> Void = { _ in }
    /// Every block a pass asked for `layer` had no data (its dataset has nothing there or then).
    var onNoData: (String) -> Void = { _ in }

    private static let pacerNamespace = "map"
    private static let pacerKey = "pacer"

    /// - Parameter cache: the blocks kept on the device; by default kept through `host`.
    init(
        host: FetchHost, store: GridStore, clock: @escaping @Sendable () -> Int64,
        cache: GridBlockCache? = nil
    ) {
        self.host = host
        self.store = store
        self.clock = clock
        self.cache = cache ?? GridBlockCache(host: host)
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
            perLayer.append(
                requests.filter { !inFlight.contains($0.key) }.map { Job(layer: layer.id, request: $0) })
        }
        return perLayer
    }

    /// One pass: what the cache holds fresh, then what is still missing from the network.
    private func pass(_ layers: [PlannedLayer], generation current: Int, region: MapRegion) async {
        var tried: Set<String> = []
        for _ in 0..<Self.cacheRounds {
            let untried = requests(layers, in: region).joined().filter {
                !tried.contains($0.request.key)
            }
            guard !untried.isEmpty else { break }
            tried.formUnion(untried.map(\.request.key))
            let restored = Set(
                await cache.restore(untried.map(\.request.key), into: store, now: clock()))
            // The blocks are in the store whatever pass comes next, which will not list them
            // again: say so before giving way to it, or the layer is never drawn again.
            for layer in Set(untried.filter { restored.contains($0.request.key) }.map(\.layer)) {
                onBlock(layer)
            }
            guard current == generation else { return }
            guard !restored.isEmpty else { break }
        }
        let jobs = Self.interleaved(requests(layers, in: region))
        guard !jobs.isEmpty else {
            await cache.flush()
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
                    onBlock(job.layer)
                case .noData:
                    empty[job.layer, default: 0] += 1
                case .busy(let after):
                    wait = min(wait ?? .max, after)
                    pending.removeAll { $0.request.node == job.request.node }
                case .failed:
                    break
                }
            }
        }
        host.store(namespace: Self.pacerNamespace, key: Self.pacerKey, value: pacer.toJson())
        await cache.flush()
        for (layer, count) in asked where empty[layer] == count {
            onNoData(layer)
        }
        guard let wait, wait <= Self.longestWait, current == generation else { return }
        Task {
            try? await Task.sleep(for: .seconds(Int(wait) + 1))
            if current == self.generation { _ = self.load(layers, in: region) }
        }
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
            return .failed
        }
        let now = clock()
        try? pacer.record(
            node: job.request.node, status: response.status,
            retryAfterS: response.retryAfter.flatMap {
                UInt32($0.trimmingCharacters(in: .whitespaces))
            }, now: now)
        do {
            try store.addGrid(key: job.request.key, response: response, now: now)
            await cache.save(job.request.key, body: response.body, fetchedAt: now)
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
