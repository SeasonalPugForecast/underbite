import Foundation
import UnderbiteKit

/// Loads the tiles of the tiled layers (the NOAA chart, GIBS imagery, GEBCO, radar) for the map:
/// the request from the core's `tile_request`, the bytes through the app's fetch host (never a
/// `URLSession` of its own), the verdict from `classify_tile`, kept in a ``TileMemoryCache``.
///
/// A map tile deeper than its layer's tiles is cut from the ancestor `tile_request` names, so the
/// map over-zooms every source up to ``MapPlan/deepestZoom``. Map tiles that share an ancestor
/// share one fetch. A failed tile is not kept: the last tile kept for it is drawn instead, or
/// nothing.
actor TileLoader {
    /// What a tile came back as, for the layer's date controls: `outcome` is the core's verdict
    /// on the response for `date` (none for the newest).
    typealias Report = @Sendable (_ layer: String, _ date: String?, _ outcome: TileOutcome) -> Void

    private let host: FetchHost
    nonisolated let cache: TileMemoryCache
    private let clock: @Sendable () -> Date
    private let pause: @Sendable (UInt32) async -> Void
    private var inFlight: [TileCacheKey: Task<Data?, Never>] = [:]
    private var report: Report?

    /// - Parameters:
    ///   - pause: waits before a retry (tests stand in one that does not wait).
    init(
        host: FetchHost, cache: TileMemoryCache = TileMemoryCache(),
        clock: @escaping @Sendable () -> Date = { Date() },
        pause: @escaping @Sendable (UInt32) async -> Void = { seconds in
            try? await Task.sleep(for: .seconds(Int(seconds)))
        }
    ) {
        self.host = host
        self.cache = cache
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

    /// The fetched tile for `key`: from the cache while fresh, else from the one fetch in flight
    /// for it.
    private func fetched(_ key: TileCacheKey, _ request: HostRequest) async -> Data? {
        if let hit = cache.fresh(key, now: clock()) { return hit }
        if let running = inFlight[key] { return await running.value }
        let task = Task { await self.fetch(key, request) }
        inFlight[key] = task
        let data = await task.value
        inFlight[key] = nil
        return data
    }

    private func fetch(_ key: TileCacheKey, _ request: HostRequest) async -> Data? {
        var attempt: UInt32 = 0
        while true {
            let response: HostResponse
            do {
                response = try await host.send(request: request)
            } catch {
                return cache.any(key)
            }
            let outcome =
                (try? classifyTile(
                    layer: key.layer, status: response.status, contentType: response.contentType,
                    bodyLen: UInt64(response.body.count), attempt: attempt)) ?? .failed
            report?(key.layer, key.date, outcome)
            switch TileDecision(outcome) {
            case .keep:
                let lifetime = TimeInterval(request.maxAgeS ?? 0)
                cache.store(response.body, for: key, lifetime: lifetime, now: clock())
                return response.body
            case .retry(let after):
                await pause(after)
                attempt += 1
            case .fallback:
                return cache.any(key)
            case .nothing:
                return nil
            }
        }
    }
}
