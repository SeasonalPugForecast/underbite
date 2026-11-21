import Foundation
import UnderbiteKit

/// The time axes of the grid layers: which days (or hours) each dataset holds, so the date
/// controls offer only those and the requests never ask past the newest. Each axis is asked for
/// with the core's `time_axis_request` through the fetch host, let through by a `GridPacer`
/// (ERDDAP's limits per server), and kept, in memory and in the device's ``OfflineStore`` (a blob
/// under ``Key/storeKey``), for a day (an hour for an hourly layer, whose newest time moves every
/// hour). What is kept is the response itself, so the labels follow the settings and the time
/// zone whenever they are read. Offline (no route, see ``NetworkRoute``) the kept axis is used
/// whatever its age, without asking. Axes an earlier version kept in the fetch host's store are
/// moved into the device's store when first read.
actor TimeAxes {
    /// One dataset of one layer.
    nonisolated struct Key: Hashable, Sendable {
        let layer: String
        /// None for the layer's primary dataset.
        let dataset: String?

        /// The blob it is kept under in the device's store.
        var storeKey: String { "axis/\(layer)/\(dataset ?? "primary")" }

        /// The name an earlier version kept it under in the fetch host's store.
        var legacyKey: String { "\(layer).\(dataset ?? "primary")" }
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

    /// Where an earlier version kept the axes: the fetch host's store.
    static let legacyNamespace = "map-axes"
    /// The longest pause the pacer asks for that an axis waits out by itself.
    static let longestWait: UInt32 = 120
    /// How long after a try that brought no axis the next one waits: a launch that found the
    /// network not yet up gets its axis a minute later.
    static let retryAfter: Int64 = 60

    private let host: FetchHost
    private let store: OfflineStore
    private let offline: @Sendable () -> Bool
    private let clock: @Sendable () -> Int64
    private let pause: @Sendable (UInt32) async -> Void
    private let pacer: GridPacer
    private var kept: [Key: Kept] = [:]
    private var running: [Key: Task<Void, Never>] = [:]
    private var nodes: [Key: String] = [:]
    /// When each axis was last asked for.
    private var tried: [Key: Int64] = [:]

    /// - Parameters:
    ///   - store: where the axes are kept on the device.
    ///   - offline: whether the device has no route to the network.
    ///   - pause: waits for the pacer (tests stand in one that does not wait).
    init(
        host: FetchHost, store: OfflineStore = MapOffline.store,
        offline: @escaping @Sendable () -> Bool = { NetworkRoute.shared.isOffline },
        clock: @escaping @Sendable () -> Int64,
        pause: @escaping @Sendable (UInt32) async -> Void = { seconds in
            try? await Task.sleep(for: .seconds(Int(seconds)))
        }
    ) {
        self.host = host
        self.store = store
        self.offline = offline
        self.clock = clock
        self.pause = pause
        // The grid loader's pauses, as last saved: a server it found busy waits here too.
        pacer =
            host.load(namespace: "map", key: "pacer")
            .flatMap { try? GridPacer.fromJson(json: $0) } ?? GridPacer()
    }

    /// The times of `key`'s dataset, oldest first, labelled for `settingsJSON` in `timeZone`
    /// (an IANA name): the axis kept, asked for again first when there is none or it is stale
    /// for a layer with `dates`. A stale axis is still used when asking again fails or the
    /// device is offline; none when there is no axis at all.
    func times(
        _ key: Key, dates: LayerDates, settingsJSON: String, timeZone: String
    ) async -> [LayerTime]? {
        let wait = MapLoad.shared.interval("axis", layer: key.layer)
        defer { wait.end() }
        var held = kept[key]
        var source = "memory"
        if held == nil {
            held = await loadKept(key)
            source = "store"
        }
        let now = clock()
        let stale = held.map { GridTimes.isStale(fetched: $0.fetched, now: now, dates: dates) }
        let resting = tried[key].map { now >= $0 && now - $0 < Self.retryAfter } ?? false
        if stale ?? true, !resting, !offline() {
            await refresh(key, dates: dates, settingsJSON: settingsJSON, timeZone: timeZone)
        }
        guard let axis = kept[key] else {
            MapLoad.shared.mark(.axis, layer: key.layer, source: "none")
            return nil
        }
        if axis.fetched != held?.fetched { source = "network" }
        MapLoad.shared.mark(.axis, layer: key.layer, source: source)
        let times = try? parseTimeAxis(
            layer: key.layer, dataset: key.dataset, response: axis.response,
            settingsJson: settingsJSON, timeZone: timeZone)
        return times.flatMap { $0.isEmpty ? nil : $0 }
    }

    /// Asks for `key`'s axis once, however many callers wait for it.
    private func refresh(_ key: Key, dates: LayerDates, settingsJSON: String, timeZone: String)
        async
    {
        if let task = running[key] {
            await task.value
            return
        }
        tried[key] = clock()
        let task = Task {
            await self.fetch(key, dates: dates, settingsJSON: settingsJSON, timeZone: timeZone)
        }
        running[key] = task
        await task.value
        running[key] = nil
    }

    private func fetch(_ key: Key, dates: LayerDates, settingsJSON: String, timeZone: String)
        async
    {
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
        await save(key, entry, lifetime: GridTimes.axisAge(dates))
    }

    private func save(_ key: Key, _ entry: Kept, lifetime: Int64) async {
        guard let json = try? JSONEncoder().encode(entry) else { return }
        let row = OfflineEntry(
            data: json, contentType: "application/json", fetchedAt: entry.fetched,
            lifetime: UInt64(max(0, lifetime)))
        _ = try? await store.put(.blob(key.storeKey), row, now: clock())
    }

    /// The axis kept on the device, or one an earlier version kept in the fetch host's store
    /// (moved into the device's store, and deleted there).
    private func loadKept(_ key: Key) async -> Kept? {
        if let record = try? await store.get(
            .blob(key.storeKey), now: clock(), allowingStale: true),
            let entry = try? JSONDecoder().decode(Kept.self, from: record.data)
        {
            kept[key] = entry
            return entry
        }
        guard let text = host.load(namespace: Self.legacyNamespace, key: key.legacyKey) else {
            return nil
        }
        host.store(namespace: Self.legacyNamespace, key: key.legacyKey, value: "")
        guard let entry = try? JSONDecoder().decode(Kept.self, from: Data(text.utf8)) else {
            return nil
        }
        kept[key] = entry
        // The lifetime is the store's bookkeeping; whether the axis is stale is decided by
        // ``GridTimes/isStale(fetched:now:dates:)`` when it is read.
        await save(key, entry, lifetime: 0)
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
