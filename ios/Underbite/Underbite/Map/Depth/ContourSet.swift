import Foundation
import MapKit
@preconcurrency import UnderbiteKit

/// One depth contour in MapKit map points, ready to draw: its points, the box round them, whether
/// it is a major line (drawn heavier) and its label, if the core gave it room for one.
nonisolated struct PlacedContour: Sendable {
    let points: [MKMapPoint]
    let bounds: MKMapRect
    let major: Bool
    let label: PlacedLabel?
}

/// A contour's label: the core's text ("30 ft"), where it goes and how it turns.
nonisolated struct PlacedLabel: Sendable {
    let text: String
    let at: MKMapPoint
    /// The turn of the text from level, radians, clockwise on the map (map points grow south).
    let rotation: Double
}

/// The depth contours on screen, as the core's `contour_lines` gave them for the view: placed
/// once in map points, then picked out per tile as the renderer asks.
nonisolated struct ContourSet: Sendable {
    let lines: [PlacedContour]

    static let empty = ContourSet(lines: [])

    init(lines: [PlacedContour]) {
        self.lines = lines
    }

    /// The core's lines placed on the map; lines of fewer than two points are left out.
    init(_ lines: [ContourLine]) {
        self.lines = lines.compactMap(Self.place)
    }

    /// The turn from level of text that runs `angleDeg` clockwise from true north (the core's
    /// 0 to 180, 90 being level): Web Mercator keeps angles, so a bearing on the ground is the
    /// same angle on the map.
    static func rotation(angleDeg: Double) -> Double {
        (angleDeg - 90) * .pi / 180
    }

    private static func place(_ line: ContourLine) -> PlacedContour? {
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
        let label = line.labelAt.map {
            PlacedLabel(
                text: line.label,
                at: MKMapPoint(CLLocationCoordinate2D(latitude: $0.lat, longitude: $0.lon)),
                rotation: rotation(angleDeg: $0.angleDeg))
        }
        // At least a map point across, so that a level or upright line still meets a tile.
        let bounds = MKMapRect(
            x: minX, y: minY, width: max(maxX - minX, 1), height: max(maxY - minY, 1))
        return PlacedContour(points: points, bounds: bounds, major: line.major, label: label)
    }

    /// The lines that reach into `rect` once grown by `margin` map points on each side (a line
    /// just outside a tile is still drawn into it by its width).
    func lines(in rect: MKMapRect, margin: Double) -> [PlacedContour] {
        let area = rect.insetBy(dx: -margin, dy: -margin)
        return lines.filter { $0.bounds.intersects(area) }
    }

    /// The labels within `reach` map points of `rect` (a label's text runs past its anchor).
    func labels(near rect: MKMapRect, reach: Double) -> [PlacedLabel] {
        let area = rect.insetBy(dx: -reach, dy: -reach)
        return lines.compactMap(\.label).filter { area.contains($0.at) }
    }

    /// How many points the lines hold.
    var vertexCount: Int { lines.reduce(0) { $0 + $1.points.count } }
}
