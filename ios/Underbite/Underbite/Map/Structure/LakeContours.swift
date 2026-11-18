import Foundation
import MapKit
@preconcurrency import UnderbiteKit

/// A state's lake contours placed on the map, as the depth contours are (``ContourSet``): the
/// core's lines and labels ("10 ft", depth below the lake surface in the user's unit), each
/// label where the core gave a line room for one.
nonisolated enum LakeContours {
    /// The lines of the layers drawn, placed once; lines of fewer than two points are left out.
    static func place(_ lines: [StateContourLine]) -> ContourSet {
        ContourSet(lines: lines.compactMap(place))
    }

    private static func place(_ line: StateContourLine) -> PlacedContour? {
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
                rotation: ContourSet.rotation(angleDeg: $0.angleDeg))
        }
        // At least a map point across, so that a level or upright line still meets a tile.
        let bounds = MKMapRect(
            x: minX, y: minY, width: max(maxX - minX, 1), height: max(maxY - minY, 1))
        return PlacedContour(points: points, bounds: bounds, major: line.major, label: label)
    }
}
