import Foundation
import MapKit
@preconcurrency import UnderbiteKit

/// What the storms overlay draws, in map points, bottom first: NHC's cones, the other lows'
/// bands, the storm's bands, then each hurricane's observed track, guidance bands and official
/// track; over them the marks: the official track's day labels, each model's low at the chosen
/// step with its pressure, and each hurricane's symbol with its name. Built from the core's
/// `StormLayer` (every word and colour is the core's) and placed the short way across the
/// antimeridian; nothing here touches the map view, so it is tested on its own.
nonisolated struct StormDrawing: Equatable, Sendable {
    /// A colour from the core, 0 to 255 per channel.
    struct Colour: Equatable, Sendable {
        var red: UInt8, green: UInt8, blue: UInt8

        init(_ c: MapColour) {
            (red, green, blue) = (c.red, c.green, c.blue)
        }
    }

    /// A point on the map, in map points (`x` may pass the world's width across the
    /// antimeridian).
    struct Point: Equatable, Sendable {
        var x: Double, y: Double

        init(_ p: MKMapPoint) {
            (x, y) = (p.x, p.y)
        }

        var mapPoint: MKMapPoint { MKMapPoint(x: x, y: y) }
    }

    enum Style: Equatable, Sendable {
        /// A translucent band with edges in the colour, `width` points across, dashed.
        case band(width: Double, dash: [Double])
        /// A line on a light casing (NHC's official track).
        case line(width: Double)
        /// A dotted line (the observed track).
        case dotted(width: Double, dash: [Double])
        /// A filled ring under an edge (NHC's cone).
        case cone
    }

    struct Shape: Equatable, Sendable {
        var points: [Point]
        var colour: Colour
        var style: Style
        var minX: Double, minY: Double, maxX: Double, maxY: Double

        init(_ points: [MKMapPoint], colour: MapColour, style: Style) {
            self.points = points.map(Point.init)
            self.colour = Colour(colour)
            self.style = style
            minX = points.map(\.x).min() ?? 0
            maxX = points.map(\.x).max() ?? 0
            minY = points.map(\.y).min() ?? 0
            maxY = points.map(\.y).max() ?? 0
        }

        var bounds: MKMapRect { MKMapRect(x: minX, y: minY, width: maxX - minX, height: maxY - minY) }
    }

    enum MarkKind: Equatable, Sendable {
        /// A model's low: a dot and a label beside it, toward its slot.
        case low(storm: Bool, right: Bool, up: Bool)
        /// A day on NHC's official track: a small dot and its time.
        case day
        /// A cyclone now: its symbol and name.
        case hurricane
    }

    struct Mark: Equatable, Sendable {
        var at: Point
        var kind: MarkKind
        var text: String
        var colour: Colour
        /// Which label survives when two overlap: lower first (the hurricane and the consensus or
        /// official low, then the day marks, then the other lows).
        var rank = 2
    }

    var shapes: [Shape] = []
    var marks: [Mark] = []

    var isEmpty: Bool { shapes.isEmpty && marks.isEmpty }

    /// The drawing of `layer` with the lows marked at step `step`.
    static func make(_ layer: StormLayer, step: Int) -> StormDrawing {
        var out = StormDrawing()
        let positions = { (points: [(Double, Double)]) in
            StormMapRules.mapPoints(points.map { (latitude: $0.0, longitude: $0.1) })
        }
        for h in layer.hurricanes {
            let ring = positions(h.cone.map { ($0.latitude, $0.longitude) })
            if ring.count > 2 {
                out.shapes.append(Shape(ring, colour: h.coneColour, style: .cone))
            }
        }
        let ordered = layer.tracks.filter { !$0.isStorm } + layer.tracks.filter(\.isStorm)
        for track in ordered {
            let points = positions(track.points.map { ($0.latitude, $0.longitude) })
            guard points.count > 1 else { continue }
            let style = Style.band(
                width: StormMapRules.bandWidth(isStorm: track.isStorm),
                dash: StormMapRules.dash(track.role))
            out.shapes.append(Shape(points, colour: track.colour, style: style))
        }
        for h in layer.hurricanes {
            let past = positions(h.past.map { ($0.latitude, $0.longitude) })
            if past.count > 1 {
                let style = Style.dotted(
                    width: StormMapRules.pastWidth, dash: StormMapRules.pastDash)
                out.shapes.append(Shape(past, colour: h.trackColour, style: style))
            }
            for aid in h.aids {
                let points = positions(aid.points.map { ($0.latitude, $0.longitude) })
                guard points.count > 1 else { continue }
                let style = Style.band(
                    width: StormMapRules.aidBandWidth, dash: StormMapRules.dash(aid.role))
                out.shapes.append(Shape(points, colour: aid.colour, style: style))
            }
            let official = positions(h.official.map { ($0.latitude, $0.longitude) })
            if official.count > 1 {
                let style = Style.line(width: StormMapRules.officialWidth)
                out.shapes.append(Shape(official, colour: h.trackColour, style: style))
            }
        }
        for h in layer.hurricanes {
            for point in StormMapRules.dayMarks(h.official) {
                out.marks.append(
                    Mark(
                        at: Point(MKMapPoint(at: point.latitude, point.longitude)), kind: .day,
                        text: point.label, colour: Colour(h.trackColour), rank: 1))
            }
        }
        if layer.steps.indices.contains(step) {
            let time = layer.steps[step].time
            let order = layer.models.map(\.model)
            for track in layer.tracks {
                guard let point = StormMapRules.point(of: track, at: time) else { continue }
                let slot = StormMapRules.labelSlot(order.firstIndex(of: track.model) ?? 0)
                out.marks.append(
                    Mark(
                        at: Point(MKMapPoint(at: point.latitude, point.longitude)),
                        kind: .low(storm: track.isStorm, right: slot.right, up: slot.up),
                        text: point.label, colour: Colour(track.colour),
                        rank: track.role == "consensus" || track.role == "official" ? 0 : 2))
            }
        }
        for h in layer.hurricanes {
            out.marks.append(
                Mark(
                    at: Point(MKMapPoint(at: h.latitude, h.longitude)), kind: .hurricane,
                    text: h.title, colour: Colour(h.colour), rank: 0))
        }
        return out
    }
}

extension MKMapPoint {
    nonisolated init(at latitude: Double, _ longitude: Double) {
        self.init(CLLocationCoordinate2D(latitude: latitude, longitude: longitude))
    }
}
