import Foundation
import UnderbiteKit

/// Where the day timeline puts things. Foundation only, so the package tests cover it. Everything
/// that is text comes from the view model; this file places spans, points and the hour selection
/// on a time axis measured in instants, so a 23 or 25 hour day lays out correctly.
nonisolated enum TimelineLayout {
    /// The width of one hour, fixed: a wider screen shows more hours, never wider ones.
    static let hourWidth: Double = 44
    static let secondsPerHour: Int64 = 3_600

    /// The card behind a lane, when the lane follows a card the user can hide.
    static func card(of lane: ViewModel.TimelineLane) -> String? {
        switch lane.kind {
        case "tide", "current", "wind", "pressure": lane.kind
        default: nil
        }
    }

    static func hasData(_ lane: ViewModel.TimelineLane) -> Bool {
        let chart = lane.chart
        return !(chart.series.isEmpty && chart.bands.isEmpty && chart.markers.isEmpty)
    }

    /// The lanes to draw: those with data whose card (where there is one) the page still shows.
    static func visibleLanes(
        _ timeline: ViewModel.Timeline, shownCards: Set<String>
    ) -> [ViewModel.TimelineLane] {
        timeline.lanes.filter { lane in
            guard hasData(lane) else { return false }
            if let card = card(of: lane) { return shownCards.contains(card) }
            return true
        }
    }

    static func x(_ t: Int64, start: Int64, hourWidth: Double = hourWidth) -> Double {
        Double(t - start) / Double(secondsPerHour) * hourWidth
    }

    static func width(_ timeline: ViewModel.Timeline, hourWidth: Double = hourWidth) -> Double {
        x(timeline.endT, start: timeline.startT, hourWidth: hourWidth)
    }

    /// The instants of the hour grid, from the start of the span.
    static func hourStarts(_ timeline: ViewModel.Timeline) -> [Int64] {
        guard timeline.endT > timeline.startT else { return [] }
        return Array(stride(from: timeline.startT, to: timeline.endT, by: Int(secondsPerHour)))
    }

    /// The view's "now", taken from the first lane that carries it.
    static func now(_ timeline: ViewModel.Timeline) -> Int64? {
        timeline.lanes.lazy.compactMap(\.chart.nowT).first
    }

    /// The start of the hour under horizontal position `x`; nil outside the span.
    static func hour(
        atX x: Double, timeline: ViewModel.Timeline, hourWidth: Double = hourWidth
    ) -> Int64? {
        guard x >= 0, x < width(timeline, hourWidth: hourWidth) else { return nil }
        return timeline.startT + Int64(x / hourWidth) * secondsPerHour
    }

    /// The value range a lane's series and on-curve markers span, for scaling; nil with none.
    static func range(of lane: ViewModel.TimelineLane) -> ClosedRange<Double>? {
        var values = lane.chart.series.flatMap { $0.points.map(\.value) }
        values += lane.chart.markers.compactMap(\.value)
        guard let low = values.min(), let high = values.max() else { return nil }
        return low...high
    }

    /// What a lane says about the hour starting at `hour`: the series' value nearest the start,
    /// the bands that overlap the hour and the markers inside it, all in the view's words.
    static func readings(
        of lane: ViewModel.TimelineLane, hour: Int64, stepMinutes: Int
    ) -> [String] {
        let tolerance = max(Int64(stepMinutes) * 60, secondsPerHour) / 2
        var out: [String] = []
        for series in lane.chart.series {
            guard
                let point = series.points.min(by: {
                    abs($0.t - hour) < abs($1.t - hour)
                }), abs(point.t - hour) <= tolerance
            else { continue }
            out.append(
                series.label == lane.label || series.label.isEmpty
                    ? point.label : "\(series.label) \(point.label)")
        }
        let end = hour + secondsPerHour
        out += lane.chart.bands.filter { $0.startT < end && $0.endT > hour }.map(\.label)
        out += lane.chart.markers.filter { $0.t >= hour && $0.t < end }.map(\.label)
        var seen = Set<String>()
        return out.filter { seen.insert($0).inserted }
    }
}
