import CoreLocation
import Foundation
import UnderbiteKit

/// One row of the point inspector: a layer on screen and what it has at the point, every word
/// the core's.
nonisolated struct PointRow: Equatable, Identifiable, Sendable {
    /// The layer's catalogue id.
    let layer: String
    /// The layer's title.
    let label: String
    /// The value in the user's units, "No value here", or that a picture has no value.
    let value: String
    /// When the value is valid; empty for a picture layer.
    let time: String
    /// The layer's credit (safety text).
    let source: String

    var id: String { layer }

    init(layer: String, label: String, value: String, time: String, source: String) {
        self.layer = layer
        self.label = label
        self.value = value
        self.time = time
        self.source = source
    }

    /// A row as the core's `inspect` words it.
    init(_ row: InspectorRow) {
        self.init(
            layer: row.layer, label: row.label, value: row.valueText, time: row.timeText,
            source: row.source)
    }
}

/// What the point inspector lists at a tapped point: the layers on screen, topmost first, each
/// with its value from the grids already held for the view (the core's `inspect`; a tap never
/// fetches), the picture layers as imagery only, and the nearest station on screen. Nothing here
/// formats: the words come from the core.
nonisolated enum PointInspection {
    /// The layers to ask `inspect` for, topmost first: every overlay on screen but the pictures.
    /// The core answers for the gridded ones (and depth) and leaves the others out, so a layer
    /// that gains a value at a point in the core needs nothing here.
    static func valueLayers(shown: [LayerInfo]) -> [String] {
        shown.reversed().filter { !$0.isBase && $0.draw != .tiles }.map(\.id)
    }

    /// The inspector's rows, topmost first: each overlay on screen with the row `inspect` gave
    /// for it, a picture layer (tiles) as `imageryOnly` with its credit. Base maps, and layers
    /// with no row (pins, a grid with no block over the point yet), are left out.
    static func rows(shown: [LayerInfo], values: [InspectorRow], imageryOnly: String) -> [PointRow] {
        let byLayer = Dictionary(
            values.map { ($0.layer, $0) }, uniquingKeysWith: { first, _ in first })
        return shown.reversed().compactMap { layer -> PointRow? in
            guard !layer.isBase else { return nil }
            if layer.draw == .tiles {
                return PointRow(
                    layer: layer.id, label: layer.title, value: imageryOnly, time: "",
                    source: layer.credit)
            }
            return byLayer[layer.id].map(PointRow.init)
        }
    }

    /// The station nearest `point` among `pins` (saved spots are not stations), by distance on
    /// the ground; none without a station.
    static func nearestStation(
        _ pins: [MapPin], to point: CLLocationCoordinate2D
    ) -> MapPin? {
        let here = CLLocation(latitude: point.latitude, longitude: point.longitude)
        return
            pins
            .filter { $0.kind != .spot }
            .min {
                here.distance(from: CLLocation(latitude: $0.latitude, longitude: $0.longitude))
                    < here.distance(from: CLLocation(latitude: $1.latitude, longitude: $1.longitude))
            }
    }

    /// `count` points from text such as `40.45,-73.93` or `40.70,-74.02,40.45,-73.93` (a launch
    /// argument); none unless every number is there and in range.
    static func points(_ text: String, count: Int) -> [CLLocationCoordinate2D]? {
        let parts = text.split(separator: ",").map {
            Double($0.trimmingCharacters(in: .whitespaces))
        }
        guard parts.count == count * 2, parts.allSatisfy({ $0 != nil }) else { return nil }
        let numbers = parts.compactMap { $0 }
        let points = stride(from: 0, to: numbers.count, by: 2).map {
            CLLocationCoordinate2D(latitude: numbers[$0], longitude: numbers[$0 + 1])
        }
        guard
            points.allSatisfy({ (-90...90).contains($0.latitude) && (-180...180).contains($0.longitude) })
        else { return nil }
        return points
    }
}

/// A line being measured: where it starts (a point on the map, a saved spot or the phone's
/// location) and where it ends, once known. The distance and bearing are the core's.
nonisolated struct MeasureLine: Equatable, Sendable {
    /// One end of the line.
    struct End: Equatable, Sendable {
        enum Kind: Equatable, Sendable {
            /// A point on the map.
            case point
            /// A saved spot, by id.
            case spot(String)
            /// The phone's location.
            case here
        }

        var kind: Kind
        var latitude: Double
        var longitude: Double

        var coordinate: CLLocationCoordinate2D {
            CLLocationCoordinate2D(latitude: latitude, longitude: longitude)
        }

        var mapPoint: MapPoint { MapPoint(lat: latitude, lon: longitude) }

        static func point(_ c: CLLocationCoordinate2D) -> End {
            End(kind: .point, latitude: c.latitude, longitude: c.longitude)
        }
    }

    var from: End
    var to: End?

    /// A tap on the map: the line ends there.
    mutating func tap(_ coordinate: CLLocationCoordinate2D) {
        to = .point(coordinate)
    }

    /// The ends to draw a line between; none until both are known.
    var coordinates: [CLLocationCoordinate2D] {
        guard let to else { return [] }
        return [from.coordinate, to.coordinate]
    }

    /// The core's distance, true bearing and "not for navigation" note for the line; none until
    /// it has an end, or for settings the core cannot read.
    func text(settingsJSON: String) -> MeasureText? {
        guard let to else { return nil }
        return try? measure(from: from.mapPoint, to: to.mapPoint, settingsJson: settingsJSON)
    }
}
