import CoreGraphics
import Foundation
import ImageIO
import UnderbiteKit
import UniformTypeIdentifiers

#if canImport(UIKit)
    import UIKit
#endif

/// A fetched tile's place in the cache: the layer, the day asked for and the tile fetched (for an
/// over-zoomed map tile, its ancestor at the layer's deepest zoom, which every map tile under it
/// shares).
nonisolated struct TileCacheKey: Hashable, Sendable {
    let layer: String
    /// `YYYY-MM-DD`, or none for the source's newest.
    let date: String?
    let z: UInt8
    let x: UInt32
    let y: UInt32

    /// `sst-imagery|2026-11-16|7/37/48`; `-` for no date.
    var text: String { "\(layer)|\(date ?? "-")|\(z)/\(x)/\(y)" }
}

/// Fetched tiles in memory, by ``TileCacheKey``, as whether's radar tile cache: thread-safe,
/// bounded by size (the least recently used tile goes first) and emptied on a memory warning. A
/// tile is fresh for the lifetime it was stored with; once stale it is still kept as the fallback
/// for a failed fetch (the chart service fails now and then) until the bound drops it. Failed
/// responses are never stored. Nothing is dropped except by those two rules, so what was stored
/// is what is read.
nonisolated final class TileMemoryCache: @unchecked Sendable {
    private struct Entry {
        let data: Data
        let fetched: Date
        let expires: Date
        var used: UInt64
    }

    /// A tile kept: its body and when it was fetched.
    struct Kept: Equatable, Sendable {
        let data: Data
        let fetchedAt: Date
        /// Whether it was past its lifetime when it was read.
        let isStale: Bool
    }

    private let lock = NSLock()
    private var tiles: [String: Entry] = [:]
    private var bytes = 0
    private var clock: UInt64 = 0
    private let costLimit: Int
    private let notifications: NotificationCenter
    private var warning: NSObjectProtocol?

    /// The notification that empties the cache: the system's memory warning on iOS.
    static var memoryWarning: Notification.Name {
        #if canImport(UIKit)
            UIApplication.didReceiveMemoryWarningNotification
        #else
            Notification.Name("TileMemoryCache.memoryWarning")
        #endif
    }

    init(
        costLimit: Int = 48 << 20, notifications: NotificationCenter = .default,
        emptiedBy warning: Notification.Name = TileMemoryCache.memoryWarning
    ) {
        self.costLimit = costLimit
        self.notifications = notifications
        self.warning = notifications.addObserver(forName: warning, object: nil, queue: nil) {
            [weak self] _ in self?.removeAll()
        }
    }

    deinit {
        if let warning { notifications.removeObserver(warning) }
    }

    /// The tile for `key` while it is fresh at `now`.
    func fresh(_ key: TileCacheKey, now: Date) -> Data? {
        guard let entry = touch(key.text), now < entry.expires else { return nil }
        return entry.data
    }

    /// The tile for `key`, fresh or not: what a failed fetch falls back to.
    func any(_ key: TileCacheKey) -> Data? {
        touch(key.text)?.data
    }

    /// The tile for `key`, fresh or not, with when it was fetched and whether it is stale at
    /// `now`.
    func kept(_ key: TileCacheKey, now: Date) -> Kept? {
        guard let entry = touch(key.text) else { return nil }
        return Kept(data: entry.data, fetchedAt: entry.fetched, isStale: now >= entry.expires)
    }

    /// Keeps `data` for `key`, fresh for `lifetime` seconds from `now`; a lifetime of zero or
    /// less, or an empty body, keeps nothing. `fetchedAt` is when it was fetched, `now` unless
    /// it was read from the device's store.
    func store(
        _ data: Data, for key: TileCacheKey, lifetime: TimeInterval, now: Date,
        fetchedAt: Date? = nil
    ) {
        guard lifetime > 0, !data.isEmpty else { return }
        lock.lock()
        defer { lock.unlock() }
        if let old = tiles[key.text] { bytes -= old.data.count }
        clock += 1
        tiles[key.text] = Entry(
            data: data, fetched: fetchedAt ?? now, expires: now.addingTimeInterval(lifetime),
            used: clock)
        bytes += data.count
        while bytes > costLimit, let oldest = tiles.min(by: { $0.value.used < $1.value.used }) {
            bytes -= oldest.value.data.count
            tiles[oldest.key] = nil
        }
    }

    func removeAll() {
        lock.lock()
        defer { lock.unlock() }
        tiles.removeAll()
        bytes = 0
    }

    /// The entry for `text`, marked as just used.
    private func touch(_ text: String) -> Entry? {
        lock.lock()
        defer { lock.unlock() }
        guard var entry = tiles[text] else { return nil }
        clock += 1
        entry.used = clock
        tiles[text] = entry
        return entry
    }
}

/// What to do with a tile response, from the core's `classify_tile`.
nonisolated enum TileDecision: Equatable, Sendable {
    /// Draw the body and keep it.
    case keep
    /// Ask again after this many seconds.
    case retry(after: UInt32)
    /// Draw the last tile kept for it, if any, else nothing.
    case fallback
    /// Draw nothing: the source has nothing there (a cloudy or missing day, too deep a zoom).
    case nothing

    /// The longest wait for a retry while MapKit waits for the tile; a source asking for longer
    /// gets the fallback this time and is asked again when the map next needs the tile.
    static let longestRetry: UInt32 = 10

    init(_ outcome: TileOutcome) {
        switch outcome {
        case .image: self = .keep
        case .noData, .tooDeep: self = .nothing
        case .retry(let after) where after <= Self.longestRetry: self = .retry(after: after)
        case .retry, .failed: self = .fallback
        }
    }
}

/// Cutting the part of an ancestor tile that an over-zoomed map tile shows.
nonisolated enum TileImage {
    /// The square `crop` (fractions of the image's side, from its top-left corner) of the image
    /// in `data`, scaled to `side` pixels, as PNG (transparency kept); none when the data is not
    /// an image.
    static func crop(_ data: Data, to crop: TileCrop, side: Int) -> Data? {
        guard side > 0,
            let source = CGImageSourceCreateWithData(data as CFData, nil),
            let image = CGImageSourceCreateImageAtIndex(source, 0, nil)
        else { return nil }
        let width = Double(image.width), height = Double(image.height)
        // At least one source pixel: far past the source's zoom a map tile is one colour.
        let size = max(crop.size * width, 1)
        let rect = CGRect(
            x: min(max(crop.left * width, 0), width - size),
            y: min(max(crop.top * height, 0), height - size),
            width: size, height: size
        ).integral
        guard let part = image.cropping(to: rect),
            let space = CGColorSpace(name: CGColorSpace.sRGB),
            let context = CGContext(
                data: nil, width: side, height: side, bitsPerComponent: 8, bytesPerRow: 0,
                space: space, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
        else { return nil }
        // The source's cells stay sharp-edged when scaled up, as the map draws its own tiles.
        context.interpolationQuality = .none
        context.draw(part, in: CGRect(x: 0, y: 0, width: side, height: side))
        guard let scaled = context.makeImage() else { return nil }
        let out = NSMutableData()
        guard
            let destination = CGImageDestinationCreateWithData(
                out as CFMutableData, UTType.png.identifier as CFString, 1, nil)
        else { return nil }
        CGImageDestinationAddImage(destination, scaled, nil)
        return CGImageDestinationFinalize(destination) ? out as Data : nil
    }
}
