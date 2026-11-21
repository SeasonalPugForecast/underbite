import Foundation
import MapKit
@preconcurrency import UnderbiteKit

/// The colour roles of a beach forecast zone, by the name the core gives its rip current risk
/// (`low`, `moderate`, `high`) or `none` for a zone with no forecast issued. A role names a
/// category the forecasters wrote; it is not a verdict, so there is no green: the lowest
/// category is a neutral slate, the next amber, the highest red, and a zone with nothing issued
/// is outlined and not filled.
nonisolated enum BeachRole: String, Sendable, CaseIterable {
    case low, moderate, high, none

    /// The role for the core's name; anything else is a zone with nothing to show.
    init(core: String) {
        self = BeachRole(rawValue: core) ?? .none
    }

    /// The fill and edge colour, 0xRRGGBB.
    var rgb: UInt32 {
        switch self {
        case .low: 0x6B7F94
        case .moderate: 0xE39A1B
        case .high: 0xC62F2F
        case .none: 0x4A525C
        }
    }

    /// Whether the zone is filled; a zone with no forecast is only outlined (dashed).
    var filled: Bool { self != .none }

    /// How much of the colour the fill shows.
    var fillAlpha: Double {
        switch self {
        case .low: 0.28
        case .moderate: 0.38
        case .high: 0.45
        case .none: 0
        }
    }
}

/// One shore zone of the beach forecast in map points, with what it takes to draw it and to tell
/// whether a tap fell inside it.
nonisolated struct BeachShape: Sendable {
    /// `okx:nyz178`.
    let id: String
    let name: String
    let role: BeachRole
    /// The zone's outlines: polygons of rings, the first ring of each the outer boundary and the
    /// others holes.
    let polygons: [[[MapXY]]]
    /// The bounds of every ring, in map points.
    let bounds: MKMapRect

    init(id: String, name: String, role: BeachRole, polygons: [[[MapXY]]]) {
        self.id = id
        self.name = name
        self.role = role
        self.polygons = polygons
        var minX = Double.infinity, minY = Double.infinity
        var maxX = -Double.infinity, maxY = -Double.infinity
        for point in polygons.joined().joined() {
            minX = min(minX, point.x)
            minY = min(minY, point.y)
            maxX = max(maxX, point.x)
            maxY = max(maxY, point.y)
        }
        bounds =
            minX.isFinite
            ? MKMapRect(x: minX, y: minY, width: maxX - minX, height: maxY - minY) : .null
    }

    /// The shape of a zone `plan_beach_forecast` found; none when it has no usable outline.
    init?(_ pin: BeachZonePin) {
        func xy(_ p: MapPoint) -> MapXY { MapXY(latitude: p.lat, longitude: p.lon) }
        let polygons = pin.polygons.compactMap { polygon -> [[MapXY]]? in
            let rings = polygon.rings.filter { $0.count > 2 }.map { $0.map(xy) }
            return rings.isEmpty ? nil : rings
        }
        guard !polygons.isEmpty else { return nil }
        self.init(id: pin.id, name: pin.name, role: BeachRole(core: pin.role), polygons: polygons)
    }

    /// Whether map point (`x`, `y`) is inside the zone: inside an outer ring and not inside one
    /// of its holes (even-odd, ring by ring).
    func contains(x: Double, y: Double) -> Bool {
        guard bounds.contains(MKMapPoint(x: x, y: y)) else { return false }
        for polygon in polygons {
            var inside = false
            for ring in polygon {
                var j = ring.count - 1
                for i in ring.indices {
                    let a = ring[i], b = ring[j]
                    if (a.y > y) != (b.y > y), x < (b.x - a.x) * (y - a.y) / (b.y - a.y) + a.x {
                        inside.toggle()
                    }
                    j = i
                }
            }
            if inside { return true }
        }
        return false
    }
}

nonisolated enum BeachZones {
    /// The catalogue id of the layer.
    static let layer = "beach-forecast"

    /// The shapes of the zones found, in the order given (the core's, by zone id); a zone with
    /// no outline is left out.
    static func shapes(_ pins: [BeachZonePin]) -> [BeachShape] {
        pins.compactMap(BeachShape.init)
    }

    /// The zone under map point (`x`, `y`): the smallest whose outline holds it.
    static func hit(_ shapes: [BeachShape], x: Double, y: Double) -> BeachShape? {
        var best: (shape: BeachShape, size: Double)?
        for shape in shapes where shape.contains(x: x, y: y) {
            let size = shape.bounds.width * shape.bounds.height
            if size < (best?.size ?? .infinity) { best = (shape, size) }
        }
        return best?.shape
    }
}
