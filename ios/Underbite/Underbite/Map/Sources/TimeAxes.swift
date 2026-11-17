import Foundation
import UnderbiteKit

/// The time axes of the grid layers: which days (or hours) each dataset holds, so the date
/// controls offer only those and the requests never ask past the newest. Each axis is asked for
/// with the core's `time_axis_request` through the fetch host, let through by a `GridPacer`
/// (ERDDAP's limits per server), and kept, in memory and in the fetch host's store, for a day (an
/// hour for an hourly layer, whose newest time moves every hour). What is kept is the response
/// itself, so the labels follow the settings and the time zone whenever they are read.
actor TimeAxes {
    /// One dataset of one layer.
    nonisolated struct Key: Hashable, Sendable {
        let layer: String
        /// None for the layer's primary dataset.
        let dataset: String?

        /// The name it is kept under in the fetch host's store.
        var storeKey: String { "\(layer).\(dataset ?? "primary")" }
    }

    /// An axis response as kept.
    private nonisolated struct Kept: Codable, Sendable {
        let fetched: Int64
        let status: UInt16
        let contentType: String?
        let body: String

        var response: HostResponse {
            HostResponse(
                status: status, body: Data(body.utf8), contentType: contentType, date: nil,
                expires: nil, lastModified: nil, etag: nil, retryAfter: nil,
                rateLimitRemaining: nil)
        }
    }

    static let namespace = "map-axes"
    /// The longest pause the pacer asks for that an axis waits out by itself.
    static let longestWait: UInt32 = 120
    /// How long after a try that brought no axis the next one waits: a launch that found the
    /// network not yet up gets its axis a minute later.
    static let retryAfter: Int64 = 60

    private let host: FetchHost
    private let clock: @Sendable () -> Int64
    private let pause: @Sendable (UInt32) async -> Void
    private let pacer: GridPacer
    private var kept: [Key: Kept] = [:]
    private var running: [Key: Task<Void, Never>] = [:]
    private var nodes: [Key: String] = [:]
    /// When each axis was last asked for.
    private var tried: [Key: Int64] = [:]

    /// - Parameters:
    ///   - pause: waits for the pacer (tests stand in one that does not wait).
    init(
        host: FetchHost, clock: @escaping @Sendable () -> Int64,
        pause: @escaping @Sendable (UInt32) async -> Void = { seconds in
            try? await Task.sleep(for: .seconds(Int(seconds)))
        }
    ) {
        self.host = host
        self.clock = clock
        self.pause = pause
        // The grid loader's pauses, as last saved: a server it found busy waits here too.
        pacer =
            host.load(namespace: "map", key: "pacer")
            .flatMap { try? GridPacer.fromJson(json: $0) } ?? GridPacer()
    }

    /// The times of `key`'s dataset, oldest first, labelled for `settingsJSON` in `timeZone`
    /// (an IANA name): the axis kept, asked for again first when there is none or it is stale
    /// for a layer with `dates`. A stale axis is still used when asking again fails; none when
    /// there is no axis at all.
    func times(
        _ key: Key, dates: LayerDates, settingsJSON: String, timeZone: String
    ) async -> [LayerTime]? {
        let held = kept[key] ?? loadKept(key)
        let now = clock()
        let stale = held.map { GridTimes.isStale(fetched: $0.fetched, now: now, dates: dates) }
        let resting = tried[key].map { now >= $0 && now - $0 < Self.retryAfter } ?? false
        if stale ?? true, !resting {
            await refresh(key, settingsJSON: settingsJSON, timeZone: timeZone)
        }
        guard let axis = kept[key] else { return nil }
        let times = try? parseTimeAxis(
            layer: key.layer, dataset: key.dataset, response: axis.response,
            settingsJson: settingsJSON, timeZone: timeZone)
        return times.flatMap { $0.isEmpty ? nil : $0 }
    }

    /// Asks for `key`'s axis once, however many callers wait for it.
    private func refresh(_ key: Key, settingsJSON: String, timeZone: String) async {
        if let task = running[key] {
            await task.value
            return
        }
        tried[key] = clock()
        let task = Task {
            await self.fetch(key, settingsJSON: settingsJSON, timeZone: timeZone)
        }
        running[key] = task
        await task.value
        running[key] = nil
    }

    private func fetch(_ key: Key, settingsJSON: String, timeZone: String) async {
        guard let node = node(of: key) else { return }
        var decision = try? pacer.maySend(node: node, now: clock())
        if let wait = decision?.waitS, decision?.send == false, wait <= Self.longestWait {
            await pause(wait + 1)
            decision = try? pacer.maySend(node: node, now: clock())
        }
        guard decision?.send == true,
            let request = try? timeAxisRequest(
                layer: key.layer, dataset: key.dataset, now: clock()),
            let response = try? await host.send(request: request)
        else { return }
        let now = clock()
        try? pacer.record(
            node: node, status: response.status,
            retryAfterS: response.retryAfter.flatMap {
                UInt32($0.trimmingCharacters(in: .whitespaces))
            }, now: now)
        // Only an axis the core can read is kept: an error page keeps the last good one.
        guard
            let times = try? parseTimeAxis(
                layer: key.layer, dataset: key.dataset, response: response,
                settingsJson: settingsJSON, timeZone: timeZone), !times.isEmpty
        else { return }
        let entry = Kept(
            fetched: now, status: response.status, contentType: response.contentType,
            body: String(decoding: response.body, as: UTF8.self))
        kept[key] = entry
        if let json = try? JSONEncoder().encode(entry) {
            host.store(
                namespace: Self.namespace, key: key.storeKey,
                value: String(decoding: json, as: UTF8.self))
        }
    }

    private func loadKept(_ key: Key) -> Kept? {
        guard let text = host.load(namespace: Self.namespace, key: key.storeKey),
            let entry = try? JSONDecoder().decode(Kept.self, from: Data(text.utf8))
        else { return nil }
        kept[key] = entry
        return entry
    }

    /// The ERDDAP server of `key`'s dataset, as the pacer names it: the core says which server a
    /// block of the dataset goes to (any block; asking for one fetches nothing and a fresh store
    /// leaves the map's grids alone).
    private func node(of key: Key) -> String? {
        if let node = nodes[key] { return node }
        let node = try? GridStore().gridRequests(
            layer: key.layer, dataset: key.dataset, time: nil, west: 0, south: 0, east: 1,
            north: 1, zoom: 12, now: clock()
        ).first?.node
        nodes[key] = node
        return node
    }
}
