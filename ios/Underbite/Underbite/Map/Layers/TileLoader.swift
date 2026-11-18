import Foundation
import UnderbiteKit

/// Loads the tiles of the tiled layers (the NOAA chart, GIBS imagery, GEBCO, radar) for the map:
/// the request from the core's `tile_request`, the bytes through the app's fetch host (never a
/// `URLSession` of its own), the verdict from `classify_tile`.
///
/// **Where a tile comes from.** The ``TileMemoryCache`` first, then the device's
/// ``OfflineStore``, then the network (``ReadThrough``):
/// - a copy kept while fresh is drawn without asking;
/// - a stale copy is asked for again online, and drawn when the fetch fails (a transport failure,
///   a server error, an error page): the copy a pack pins too, which only a newer successful
///   fetch replaces;
/// - offline (no route, see ``NetworkRoute``) nothing is asked: any kept copy is drawn, stale or
///   not, and a tile with none is missing.
///
/// An image the core accepts is kept in memory and written to the store with the lifetime the
/// core gave the request (NCDS a day, GEBCO a month), off MapKit's callback: the write goes to
/// the store's actor in a task of its own. A request the core marks `no_store` or gives no
/// lifetime (a GIBS layer's current day) stays in memory only, as do the radar's tiles.
///
/// Every tile reports where it came from to ``MapOfflineStatus``, which badges the layer
/// ("cached <date>", "not available offline").
///
/// A map tile deeper than its layer's tiles is cut from the ancestor `tile_request` names, so the
/// map over-zooms every source up to ``MapPlan/deepestZoom``. Map tiles that share an ancestor
/// share one fetch. A failed tile is not kept.
actor TileLoader {
    /// What a tile came back as, for the layer's date controls: `outcome` is the core's verdict
    /// on the response for `date` (none for the newest).
    typealias Report = @Sendable (_ layer: String, _ date: String?, _ outcome: TileOutcome) -> Void

    private let host: FetchHost
    nonisolated let cache: TileMemoryCache
    private let store: OfflineStore?
    private let status: MapOfflineStatus
    private let offline: @Sendable () -> Bool
    private let clock: @Sendable () -> Date
    private let pause: @Sendable (UInt32) async -> Void
    private var inFlight: [TileCacheKey: Task<Data?, Never>] = [:]
    private var writes: [UUID: Task<Void, Never>] = [:]
    private var report: Report?

    /// - Parameters:
    ///   - store: the tiles kept on the device; none keeps them in memory only.
    ///   - status: where the layers' badges are worked out.
    ///   - offline: whether the device has no route to the network.
    ///   - pause: waits before a retry (tests stand in one that does not wait).
    init(
        host: FetchHost, cache: TileMemoryCache = TileMemoryCache(),
        store: OfflineStore? = MapOffline.store,
        status: MapOfflineStatus = MapOfflineStatus.shared,
        offline: @escaping @Sendable () -> Bool = { NetworkRoute.shared.isOffline },
        clock: @escaping @Sendable () -> Date = { Date() },
        pause: @escaping @Sendable (UInt32) async -> Void = { seconds in
            try? await Task.sleep(for: .seconds(Int(seconds)))
        }
    ) {
        self.host = host
        self.cache = cache
        self.store = store
        self.status = status
        self.offline = offline
        self.clock = clock
        self.pause = pause
    }

    /// Calls `report` with the verdict on every tile response from now on.
    func onReport(_ report: Report?) {
        self.report = report
    }

    /// The image for map tile `z`/`x`/`y` of `layer` on `date`, or none where the layer has
    /// nothing to draw.
    func tile(layer: String, date: String?, z: Int, x: Int, y: Int) async -> Data? {
        guard let z = UInt8(exactly: z), let x = UInt32(exactly: x), let y = UInt32(exactly: y),
            let fetch = try? tileRequest(layer: layer, date: date, z: z, x: x, y: y)
        else { return nil }
        let key = TileCacheKey(layer: layer, date: date, z: fetch.z, x: fetch.x, y: fetch.y)
        guard let data = await fetched(key, fetch.request) else { return nil }
        guard let crop = fetch.crop else { return data }
        return TileImage.crop(data, to: crop, side: Int(mapTileSize()))
    }

    /// Waits for the writes to the store started so far (tests).
    func settle() async {
        while let (id, write) = writes.first {
            await write.value
            writes[id] = nil
        }
    }

    /// The fetched tile for `key`: from memory while fresh, else from the one read in flight
    /// for it.
    private func fetched(_ key: TileCacheKey, _ request: HostRequest) async -> Data? {
        let now = clock()
        if let hit = cache.kept(key, now: now), !hit.isStale {
            let offline = offline()
            say(key, offline ? .stored(fetchedAt: Self.seconds(hit.fetchedAt)) : .current)
            return hit.data
        }
        if let running = inFlight[key] { return await running.value }
        let task = Task { await self.read(key, request) }
        inFlight[key] = task
        let data = await task.value
        inFlight[key] = nil
        return data
    }

    /// A kept copy: the one in the store (put in memory while fresh; a pack may have written a
    /// newer one than memory holds), else the one in memory.
    private func kept(_ key: TileCacheKey, persists: Bool) async -> TileMemoryCache.Kept? {
        let now = clock()
        guard persists, let store, let row = Self.row(key),
            let record = try? await store.get(
                .tile(row), now: Self.seconds(now), allowingStale: true)
        else { return cache.kept(key, now: now) }
        let fetchedAt = Date(timeIntervalSince1970: TimeInterval(record.fetchedAt))
        if !record.isStale {
            cache.store(
                record.data, for: key,
                lifetime: TimeInterval(record.expiresAt) - now.timeIntervalSince1970, now: now,
                fetchedAt: fetchedAt)
        }
        return TileMemoryCache.Kept(
            data: record.data, fetchedAt: fetchedAt, isStale: record.isStale)
    }

    private func read(_ key: TileCacheKey, _ request: HostRequest) async -> Data? {
        let persists = MapOffline.persists(layer: key.layer, request)
        let held = await kept(key, persists: persists)
        let offline = offline()
        switch ReadThrough.step(kept: held.map(\.isStale), offline: offline) {
        case .serve:
            guard let held else { return nil }
            say(key, offline ? .stored(fetchedAt: Self.seconds(held.fetchedAt)) : .current)
            return held.data
        case .missing:
            say(key, .missing)
            return nil
        case .fetch:
            break
        }
        let (data, outcome) = await fetch(key, request, persists: persists)
        if let data {
            say(key, .current)
            return data
        }
        guard ReadThrough.fallsBack(after: outcome) else { return nil }
        if let held {
            say(key, .stored(fetchedAt: Self.seconds(held.fetchedAt)))
            return held.data
        }
        // A request that failed at the transport is offline; a server error is not.
        if outcome == nil { say(key, .missing) }
        return nil
    }

    /// The image from the network, or none with the core's last verdict (none: the request
    /// failed at the transport).
    private func fetch(_ key: TileCacheKey, _ request: HostRequest, persists: Bool) async
        -> (Data?, TileOutcome?)
    {
        var attempt: UInt32 = 0
        while true {
            let response: HostResponse
            do {
                response = try await host.send(request: request)
            } catch {
                return (nil, nil)
            }
            let outcome =
                (try? classifyTile(
                    layer: key.layer, status: response.status, contentType: response.contentType,
                    bodyLen: UInt64(response.body.count), attempt: attempt)) ?? .failed
            report?(key.layer, key.date, outcome)
            switch TileDecision(outcome) {
            case .keep:
                keep(key, request, response, persists: persists)
                return (response.body, outcome)
            case .retry(let after):
                await pause(after)
                attempt += 1
            case .fallback, .nothing:
                return (nil, outcome)
            }
        }
    }

    /// Keeps an accepted image in memory and, unless it is memory-only, writes it to the store
    /// without waiting for the write.
    private func keep(
        _ key: TileCacheKey, _ request: HostRequest, _ response: HostResponse, persists: Bool
    ) {
        let now = clock()
        guard persists, let store, let row = Self.row(key), let lifetime = request.maxAgeS else {
            let lifetime = request.maxAgeS.flatMap { $0 > 0 ? TimeInterval($0) : nil }
            cache.store(
                response.body, for: key, lifetime: lifetime ?? MapOffline.memoryOnlyLifetime,
                now: now)
            return
        }
        cache.store(response.body, for: key, lifetime: TimeInterval(lifetime), now: now)
        let entry = OfflineEntry(
            data: response.body, contentType: response.contentType, etag: response.etag,
            fetchedAt: Self.seconds(now), lifetime: UInt64(lifetime))
        let id = UUID()
        writes[id] = Task.detached(priority: .utility) { [weak self] in
            _ = try? await store.put(.tile(row), entry, now: entry.fetchedAt)
            await self?.wrote(id)
        }
    }

    private func wrote(_ id: UUID) {
        writes[id] = nil
    }

    private func say(_ key: TileCacheKey, _ source: PieceSource) {
        status.report(layer: key.layer, date: key.date, piece: key.text, source)
    }

    /// The store's row for a tile of the map.
    nonisolated static func row(_ key: TileCacheKey) -> OfflineTileKey? {
        OfflineTileKey(layer: key.layer, date: key.date, z: key.z, x: key.x, y: key.y)
    }

    nonisolated static func seconds(_ date: Date) -> Int64 {
        Int64(date.timeIntervalSince1970.rounded(.down))
    }
}
