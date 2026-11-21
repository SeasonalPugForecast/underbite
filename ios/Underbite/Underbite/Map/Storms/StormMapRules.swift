import Foundation
import MapKit
@preconcurrency import UnderbiteKit

/// How the storms layer draws what the core gives it: each model's dash, so colour is never the
/// only thing telling the tracks apart, the widths and opacities of the bands, lines and cone,
/// which point of a low marks a time step, which official forecast points get a day label, and
/// how a track or cone across the antimeridian is drawn the short way. Layout only: every value
/// and word comes from the core's `StormLayer`.
nonisolated enum StormMapRules {
    /// A low's point marks a step when it is at most this far from the step's time, seconds.
    static let nearestWithin: Int64 = 3 * 3_600

    /// The dash pattern of a band's edges in points, on and off, drawn with round caps (so a
    /// near-zero dash is a dot): ECMWF solid, GFS long dashes, GEM dots, ICON dash-dot; for a
    /// hurricane's guidance UKMET dash-dash-dot, HAFS short even dashes, the consensus long
    /// dashes with short gaps; any other model short dashes. Empty is solid. The band's fill
    /// between the edges is always continuous.
    static func dash(_ role: String) -> [Double] {
        switch role {
        case "ecmwf", "official", "": []
        case "gfs": [7, 4]
        case "gem": [0.01, 3.5]
        case "icon": [8, 3.5, 0.01, 3.5]
        case "ukmet": [10, 4, 2, 4]
        case "hafs": [3, 3]
        case "consensus": [12, 3]
        default: [3, 3.5]
        }
    }

    /// A low's band, edge to edge, in points: the storm's wider than the other lows'.
    static func bandWidth(isStorm: Bool) -> Double { isStorm ? 12 : 6 }
    /// A hurricane guidance model's band, as narrow as a low that is not the storm.
    static let aidBandWidth = 6.0
    /// The width of a band's two edges, in points, in the model's full colour.
    static let edgeWidth = 1.5
    /// The opacity of a band's fill, so bands that cross show through each other.
    static let fillAlpha = 0.28
    /// NHC's official track: a solid line this wide on a light casing, in points.
    static let officialWidth = 3.5
    static let casingWidth = 2.0
    /// The observed track: dots this wide, this far apart (on and off, in points).
    static let pastWidth = 2.5
    static let pastDash: [Double] = [0.01, 5]
    /// NHC's forecast cone: filled at this opacity under a crisp edge this wide.
    static let coneFillAlpha = 0.18
    static let coneEdgeWidth = 1.5
    /// A hurricane's symbol, in points across, and how close a tap must come to it.
    static let symbolSize = 30.0
    static let tapRadius = 26.0

    /// The point of `track` at `time`: the one at that time, else the nearest within
    /// ``nearestWithin`` (the earlier on a tie), else none (the low is not there yet, or gone).
    static func point(of track: StormTrack, at time: Int64) -> StormTrackPoint? {
        var best: StormTrackPoint?
        for point in track.points {
            let off = abs(point.time - time)
            guard off <= nearestWithin else { continue }
            if let kept = best, abs(kept.time - time) <= off { continue }
            best = point
        }
        return best
    }

    /// The step to show: `wanted` clamped to the steps, else the first (now).
    static func startStep(_ wanted: Int?, count: Int) -> Int {
        guard count > 0 else { return 0 }
        return min(max(wanted ?? 0, 0), count - 1)
    }

    /// Where a model's low label sits around its mark, so the models' labels do not pile up when
    /// the lows are close: right and up, right and down, left and up, left and down, by the
    /// model's place in the legend.
    static func labelSlot(_ index: Int) -> (right: Bool, up: Bool) {
        let i = ((index % 4) + 4) % 4
        return (right: i < 2, up: i % 2 == 0)
    }

    /// Which labels to draw: `rects` are the labels' plates in screen space, each paired with its
    /// rank (lower first). Labels are placed in order of rank, then in the order given, and one
    /// that overlaps a label already placed (within `gap` points) is left out. Returns, for each
    /// rect in the order given, whether its label is kept. A label's dot is always drawn: only the
    /// words are dropped, and the legend carries the values.
    static func keptLabels(_ rects: [CGRect], ranks: [Int], gap: CGFloat = 2) -> [Bool] {
        let order = rects.indices.sorted {
            let (a, b) = (ranks.indices.contains($0) ? ranks[$0] : 0, ranks.indices.contains($1) ? ranks[$1] : 0)
            return a == b ? $0 < $1 : a < b
        }
        var kept = [Bool](repeating: false, count: rects.count)
        var placed: [CGRect] = []
        for i in order {
            let rect = rects[i].insetBy(dx: -gap / 2, dy: -gap / 2)
            if placed.contains(where: { $0.intersects(rect) }) { continue }
            placed.append(rect)
            kept[i] = true
        }
        return kept
    }

    /// The official forecast points that get a labelled mark: one a day, every 24 hours after
    /// the first (within an hour), so the labels do not crowd. The first is the storm itself,
    /// marked with its symbol.
    static func dayMarks(_ official: [StormForecastPoint]) -> [StormForecastPoint] {
        guard let first = official.first else { return [] }
        let day: Int64 = 86_400, slack: Int64 = 3_600
        return official.filter { point in
            let after = point.time - first.time
            guard after >= day - slack else { return false }
            let off = after % day
            return off <= slack || off >= day - slack
        }
    }

    // MARK: Across the antimeridian

    /// `longitude` moved by whole turns to within 180° of `reference`.
    static func longitude(_ longitude: Double, near reference: Double) -> Double {
        longitude - 360 * ((longitude - reference) / 360).rounded()
    }

    /// `longitudes` as one continuous run, each within 180° of the one before (the first as it
    /// is): a track from 179° E to 179° W goes on to 181° instead of back across the world.
    static func continuous(_ longitudes: [Double]) -> [Double] {
        var out: [Double] = []
        out.reserveCapacity(longitudes.count)
        for lon in longitudes {
            out.append(out.last.map { longitude(lon, near: $0) } ?? lon)
        }
        return out
    }

    /// The map points of positions whose longitudes are already continuous, all moved by the
    /// same whole turns so the westernmost lies in the world map: those east of 180° get an `x`
    /// past the world's width. The renderer draws such a shape again one world to the west, so
    /// it shows the short way on both sides of the antimeridian.
    static func mapPoints(unwrapped positions: [(latitude: Double, longitude: Double)])
        -> [MKMapPoint]
    {
        guard let west = positions.map(\.longitude).min() else { return [] }
        let turns = ((west + 180) / 360).rounded(.down)
        let world = MKMapSize.world.width
        return positions.map { position in
            let shifted = position.longitude - 360 * turns
            let wrapped = longitude(shifted, near: 0)
            var point = MKMapPoint(
                CLLocationCoordinate2D(latitude: position.latitude, longitude: wrapped))
            point.x += (shifted - wrapped) / 360 * world
            return point
        }
    }

    /// The map points of a line or ring, drawn the short way across the antimeridian.
    static func mapPoints(_ positions: [(latitude: Double, longitude: Double)]) -> [MKMapPoint] {
        let longitudes = continuous(positions.map(\.longitude))
        return mapPoints(
            unwrapped: zip(positions, longitudes).map { (latitude: $0.latitude, longitude: $1) })
    }

    /// The smallest map rect around `positions` the short way round from `reference` (a
    /// longitude): each taken within 180° of it, so a storm across the antimeridian is framed
    /// beside the reference, not across the world.
    static func frame(_ positions: [(latitude: Double, longitude: Double)], around reference: Double)
        -> MKMapRect
    {
        let near = positions.map {
            (latitude: $0.latitude, longitude: longitude($0.longitude, near: reference))
        }
        return mapPoints(unwrapped: near).reduce(MKMapRect.null) {
            $0.union(MKMapRect(origin: $1, size: MKMapSize(width: 1, height: 1)))
        }
    }

    /// What frames a hurricane alone: its position, official track and cone, the short way
    /// round from its position.
    static func frame(_ hurricane: StormHurricane) -> MKMapRect {
        let positions =
            [(latitude: hurricane.latitude, longitude: hurricane.longitude)]
            + hurricane.official.map { (latitude: $0.latitude, longitude: $0.longitude) }
            + hurricane.cone.map { (latitude: $0.latitude, longitude: $0.longitude) }
        return frame(positions, around: hurricane.longitude)
    }

    /// The hurricane whose symbol a tap at `point` falls on, the nearest when several: within
    /// ``tapRadius`` points, on any copy of the world.
    static func hurricane(
        at point: MKMapPoint, in hurricanes: [StormHurricane], mapPointsPerPoint: Double
    ) -> StormHurricane? {
        let world = MKMapSize.world.width
        let reach = tapRadius * mapPointsPerPoint
        var best: (StormHurricane, Double)?
        for h in hurricanes {
            let at = MKMapPoint(CLLocationCoordinate2D(latitude: h.latitude, longitude: h.longitude))
            var dx = abs(at.x - point.x).truncatingRemainder(dividingBy: world)
            dx = min(dx, world - dx)
            let distance = (dx * dx + (at.y - point.y) * (at.y - point.y)).squareRoot()
            guard distance <= reach else { continue }
            if let kept = best, kept.1 <= distance { continue }
            best = (h, distance)
        }
        return best?.0
    }
}
