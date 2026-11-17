import Foundation
import UnderbiteKit

/// Fetches the grid blocks the gridded layers need for the region on screen into the core's
/// `GridStore`: the requests from `grid_requests` (nearest the centre first, only those the store
/// does not hold fresh), each let through by the `GridPacer` (ERDDAP's limits per server, kept
/// across launches through the fetch host's store), sent through the fetch host one at a time per
/// server (ERDDAP answers several at once from one client with "busy", which pauses the server for
/// minutes), the servers side by side.
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
    nonisolated enum Outcome: Sendable {
        case added
        case noData
        case busy(after: UInt32)
        case failed
    }

    /// Servers asked at once, one block each.
    static let parallel = 3
    /// The longest pause the loader waits out by itself before asking again.
    static let longestWait: UInt32 = 120

    let store: GridStore
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

    init(host: FetchHost, store: GridStore, clock: @escaping @Sendable () -> Int64) {
        self.host = host
        self.store = store
        self.clock = clock
        pacer =
            host.load(namespace: Self.pacerNamespace, key: Self.pacerKey)
            .flatMap { try? GridPacer.fromJson(json: $0) } ?? GridPacer()
    }

    /// Starts fetching what `layers` (the grid ones among them) need for `region`.
    func load(_ layers: [PlannedLayer], in region: MapRegion) {
        generation += 1
        let current = generation
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
        let jobs = Self.interleaved(perLayer)
        guard !jobs.isEmpty else { return }
        Task { await self.run(jobs, generation: current, layers: layers, region: region) }
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
            let (host, store, pacer, clock) = (host, store, pacer, clock)
            let outcomes = await withTaskGroup(of: (Job, Outcome).self) { group in
                for job in batch {
                    group.addTask {
                        (job, await Self.fetch(job, host: host, store: store, pacer: pacer, clock: clock))
                    }
                }
                var all: [(Job, Outcome)] = []
                for await outcome in group { all.append(outcome) }
                return all
            }
            for (job, outcome) in outcomes {
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
        for (layer, count) in asked where empty[layer] == count {
            onNoData(layer)
        }
        guard let wait, wait <= Self.longestWait, current == generation else { return }
        Task {
            try? await Task.sleep(for: .seconds(Int(wait) + 1))
            if current == self.generation { self.load(layers, in: region) }
        }
    }

    /// Fetches one block and hands it to the store; the pacer hears every answer.
    nonisolated private static func fetch(
        _ job: Job, host: FetchHost, store: GridStore, pacer: GridPacer,
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
