import Foundation
import MapKit
@preconcurrency import UnderbiteKit

/// One break in MapKit map points, ready to draw: its points, the box round them and its weight
/// against the threshold (the core's `emphasis`, 0 at the threshold, 1 at four times it).
nonisolated struct PlacedBreak: Sendable {
    let points: [MKMapPoint]
    let bounds: MKMapRect
    let emphasis: Double
}

/// The breaks of one layer on screen, as the core's `GridStore.breaks` gave them for the view:
/// placed once in map points, weakest first so that the strongest are drawn on top, then picked
/// out per tile as the renderer asks.
nonisolated struct BreakLines: Sendable {
    let lines: [PlacedBreak]

    static let empty = BreakLines(lines: [])

    init(lines: [PlacedBreak]) {
        self.lines = lines
    }

    /// The core's lines placed on the map; lines of fewer than two points are left out.
    init(_ lines: [BreakLine]) {
        self.lines = lines.compactMap(Self.place).sorted { $0.emphasis < $1.emphasis }
    }

    /// The width of a line in points for its weight: thin at the threshold, heavy at four times
    /// it.
    static func width(emphasis: Double) -> Double {
        thinnest + (heaviest - thinnest) * min(max(emphasis, 0), 1)
    }

    /// Line widths in points, at the threshold and at full weight.
    static let thinnest = 1.6
    static let heaviest = 4.5

    private static func place(_ line: BreakLine) -> PlacedBreak? {
        guard line.points.count >= 2 else { return nil }
        let points = line.points.map {
            MKMapPoint(CLLocationCoordinate2D(latitude: $0.lat, longitude: $0.lon))
        }
        var minX = Double.infinity, minY = Double.infinity
        var maxX = -Double.infinity, maxY = -Double.infinity
        for p in points {
            minX = min(minX, p.x)
            maxX = max(maxX, p.x)
            minY = min(minY, p.y)
            maxY = max(maxY, p.y)
        }
        // At least a map point across, so that a level or upright line still meets a tile.
        let bounds = MKMapRect(
            x: minX, y: minY, width: max(maxX - minX, 1), height: max(maxY - minY, 1))
        return PlacedBreak(points: points, bounds: bounds, emphasis: line.emphasis)
    }

    /// The lines that reach into `rect` once grown by `margin` map points on each side (a line
    /// just outside a tile is still drawn into it by its width).
    func lines(in rect: MKMapRect, margin: Double) -> [PlacedBreak] {
        let area = rect.insetBy(dx: -margin, dy: -margin)
        return lines.filter { $0.bounds.intersects(area) }
    }

    /// How many points the lines hold.
    var vertexCount: Int { lines.reduce(0) { $0 + $1.points.count } }
}
