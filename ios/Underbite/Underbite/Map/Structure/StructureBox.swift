import Foundation
import MapKit

/// A box of the map the structure layers are asked for: the view with a margin, grown to the
/// edges of the Web Mercator tiles of its whole zoom, so that views a little apart ask for the
/// same box and a box once fetched serves every view inside it.
nonisolated struct StructureBox: Hashable, Sendable {
    let west: Double
    let south: Double
    let east: Double
    let north: Double
    /// The whole zoom whose tiles it is made of.
    let zoom: Int

    /// The view's share added on each side before growing to the tiles.
    static let margin = 0.25
    /// The latitude Web Mercator stops at.
    static let maxLatitude = 85.051_128_78

    init(west: Double, south: Double, east: Double, north: Double, zoom: Int) {
        self.west = west
        self.south = south
        self.east = east
        self.north = north
        self.zoom = zoom
    }

    /// The box for `region` at whole zoom `zoom`.
    static func around(_ region: MapRegion, zoom: Int, margin: Double = margin) -> StructureBox {
        let z = max(0, min(zoom, 22))
        let n = pow(2, Double(z))
        let dx = (region.east - region.west) * margin
        let dy = (region.north - region.south) * margin
        let west = max(-180, region.west - dx), east = min(180, region.east + dx)
        let south = max(-maxLatitude, region.south - dy)
        let north = min(maxLatitude, region.north + dy)
        let x0 = (tileX(west, n: n)).rounded(.down)
        let x1 = (tileX(east, n: n)).rounded(.up)
        // Tile rows grow southwards.
        let y0 = (tileY(north, n: n)).rounded(.down)
        let y1 = (tileY(south, n: n)).rounded(.up)
        return StructureBox(
            west: lon(x0, n: n), south: lat(max(y1, y0 + 1), n: n),
            east: lon(max(x1, x0 + 1), n: n), north: lat(y0, n: n), zoom: z)
    }

    /// Whether the box holds all of `region`.
    func covers(_ region: MapRegion) -> Bool {
        let e = 1e-9
        return region.west >= west - e && region.east <= east + e && region.south >= south - e
            && region.north <= north + e
    }

    /// A text that names the box, for caching by box.
    var key: String {
        String(format: "%d/%.6f,%.6f,%.6f,%.6f", zoom, west, south, east, north)
    }

    private static func tileX(_ lon: Double, n: Double) -> Double {
        (lon + 180) / 360 * n
    }

    private static func tileY(_ lat: Double, n: Double) -> Double {
        let r = lat * .pi / 180
        return (1 - log(tan(r) + 1 / cos(r)) / .pi) / 2 * n
    }

    private static func lon(_ x: Double, n: Double) -> Double {
        min(180, max(-180, x / n * 360 - 180))
    }

    private static func lat(_ y: Double, n: Double) -> Double {
        let value = atan(sinh(.pi * (1 - 2 * y / n))) * 180 / .pi
        return min(maxLatitude, max(-maxLatitude, value))
    }
}

/// What was fetched for a few boxes, the newest last: a box that covers the view serves it
/// again. A tag tells apart what differs other than the box (a band, a layer and zoom).
nonisolated struct StructureHeld<Value: Sendable>: Sendable {
    private var entries: [(tag: String, box: StructureBox, value: Value)] = []
    /// How many boxes are kept.
    let limit: Int

    init(limit: Int = 6) {
        self.limit = limit
    }

    /// The value of the newest box with `tag` that covers `region`.
    func covering(_ region: MapRegion, tag: String) -> Value? {
        entries.last { $0.tag == tag && $0.box.covers(region) }?.value
    }

    /// Keeps `value` for `box`, dropping the oldest box past the limit.
    mutating func hold(_ value: Value, box: StructureBox, tag: String) {
        entries.removeAll { $0.tag == tag && $0.box == box }
        entries.append((tag, box, value))
        if entries.count > limit { entries.removeFirst(entries.count - limit) }
    }

    mutating func removeAll() {
        entries.removeAll()
    }

    var count: Int { entries.count }
}
