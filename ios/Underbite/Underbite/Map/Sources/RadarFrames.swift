import Foundation
import UnderbiteKit

/// The radar loop's frames and their tiles, as whether's loop: the latest composite's time from
/// the core's `radar_meta_request`, the frames ending there from `radar_frames` (labelled in the
/// settings' clock), and each frame's tiles from `radar_frame_request`, through the fetch host,
/// judged by `classify_tile` and kept in the map's tile memory. A map tile deeper than the
/// radar's deepest zoom is cut from the frame's tile at that zoom.
actor RadarFrames {
    /// The layer the frames belong to.
    static let layer = "radar"
    /// Tries for one tile when the server asks to be asked again.
    static let attempts: UInt32 = 3

    private let host: FetchHost
    private let cache: TileMemoryCache
    private let clock: @Sendable () -> Date
    private var inFlight: [TileCacheKey: Task<Data?, Never>] = [:]

    init(
        host: FetchHost, cache: TileMemoryCache, clock: @escaping @Sendable () -> Date = { Date() }
    ) {
        self.host = host
        self.cache = cache
        self.clock = clock
    }

    /// The loop ending at the latest composite, oldest first, labelled for `settingsJSON` in
    /// `timeZone` (an IANA name); none when the latest composite's time cannot be had.
    func frames(settingsJSON: String, timeZone: String) async -> [RadarFrame]? {
        guard let meta = try? await host.send(request: radarMetaRequest()), meta.status == 200,
            let latest = try? radarLatestValid(body: meta.body),
            let frames = try? radarFrames(
                latest: latest, settingsJson: settingsJSON, timeZone: timeZone),
            !frames.isEmpty
        else { return nil }
        return frames
    }

    /// The image for map tile `z`/`x`/`y` of the frame valid at `valid`, the radar's tiles going
    /// no deeper than `deepest`; none where the frame has nothing to draw.
    func tile(valid: Int64, z: Int, x: Int, y: Int, deepest: Int) async -> Data? {
        let source = OverZoomedTile(z: z, x: x, y: y, deepest: deepest)
        guard let sz = UInt8(exactly: source.z), let sx = UInt32(exactly: source.x),
            let sy = UInt32(exactly: source.y),
            let request = try? radarFrameRequest(valid: valid, z: sz, x: sx, y: sy)
        else { return nil }
        let key = TileCacheKey(layer: Self.layer, date: String(valid), z: sz, x: sx, y: sy)
        guard let data = await fetched(key, request) else { return nil }
        guard let crop = source.crop else { return data }
        return TileImage.crop(data, to: crop, side: Int(mapTileSize()))
    }

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
        for attempt in 0..<Self.attempts {
            guard let response = try? await host.send(request: request) else {
                return cache.any(key)
            }
            let outcome =
                (try? classifyTile(
                    layer: Self.layer, status: response.status, contentType: response.contentType,
                    bodyLen: UInt64(response.body.count), attempt: attempt)) ?? .failed
            switch TileDecision(outcome) {
            case .keep:
                cache.store(
                    response.body, for: key, lifetime: TimeInterval(request.maxAgeS ?? 0),
                    now: clock())
                return response.body
            case .retry(let after):
                try? await Task.sleep(for: .seconds(Int(after)))
            case .fallback:
                return cache.any(key)
            case .nothing:
                return nil
            }
        }
        return cache.any(key)
    }
}
