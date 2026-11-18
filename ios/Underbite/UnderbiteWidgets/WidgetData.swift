import Foundation
import UnderbiteKit

/// Why a widget shows its placeholder instead of a payload's values.
nonisolated enum WidgetGap: Hashable, Sendable {
    /// The system's placeholder while the widget loads.
    case loading
    /// The app has not told the widgets about a spot yet.
    case noSpot
    /// The spot is known but the app has not written its payload yet.
    case noPayload
    /// The payload has nothing for this widget at this place (no tide at a river gauge).
    case noData
    /// No trip lies ahead.
    case noTrip
    /// The payload's last entry is over: the values would be out of date.
    case outdated
}

/// What a widget draws from one instant: a payload's entry with the places it belongs to, or
/// the reason it has none.
nonisolated enum WidgetContent: Hashable, Sendable {
    case entry(ViewModel.WidgetEntry, places: [ViewModel.WidgetPlace])
    case gap(WidgetGap)
}

/// One instant of a widget's timeline: when it starts, the spot's or the trip's name as the
/// payload or the spot list gives it, and what it shows.
nonisolated struct WidgetMoment: Hashable, Sendable {
    var date: Date
    var name: String?
    var content: WidgetContent
}

/// A widget's timeline and when to ask for the next one.
nonisolated struct WidgetSchedule: Hashable, Sendable {
    var moments: [WidgetMoment]
    var reload: Date
}

/// How a widget finds and reads its payload, and turns it into a timeline. Only the chosen
/// spot's file (or the next trip's) is read and decoded: a widget extension lives in about
/// 30 MB of memory (the limit iOS reports when it ends one), so it never loads every spot.
/// Nothing here formats a value: the payloads carry every word and number.
nonisolated enum WidgetData {
    /// How long an hourly entry stays current after it begins.
    static let entrySpan: TimeInterval = 3600
    /// When a widget without a payload asks again; the app reloads the widgets itself after it
    /// writes new payloads, so this only catches a reload the system dropped.
    static let retry: TimeInterval = 3600

    /// The saved spot a widget set to `choice` shows: the chosen one while it is saved, else the
    /// app's spot, else the first; a chosen id the list does not know yet (the app has not
    /// written the list) is still tried.
    static func spotID(for choice: WidgetSpotChoice, in spots: WidgetSpots) -> String? {
        if let spot = spots.spot(for: choice) { return spot.id }
        if case .spot(let id) = choice { return id }
        return nil
    }

    /// The payload `kind` reads for `choice`, with the name to show while it has none.
    static func load(
        _ kind: WidgetKind, choice: WidgetSpotChoice, store: WidgetPayloadStore
    ) -> (payload: ViewModel.WidgetPayload?, name: String?, gap: WidgetGap) {
        if kind == .nextTrip {
            return (store.nextTrip(), nil, .noTrip)
        }
        let spots = store.spots()
        guard let id = spotID(for: choice, in: spots) else { return (nil, nil, .noSpot) }
        let name = spots.spots.first { $0.id == id }?.name
        return (store.spot(id: id), name, .noPayload)
    }

    /// The timeline for `kind` from `payload` at `now`: the entry current now and every later
    /// one, then a gap once the last has run its hour, so old values are never shown as
    /// current. Without a payload, `missing` until the app writes one.
    static func schedule(
        _ kind: WidgetKind, payload: ViewModel.WidgetPayload?, name: String?, missing: WidgetGap,
        now: Date
    ) -> WidgetSchedule {
        guard let payload, let last = payload.lastEntryDate else {
            return WidgetSchedule(
                moments: [WidgetMoment(date: now, name: name, content: .gap(missing))],
                reload: now.addingTimeInterval(retry))
        }
        let over = last.addingTimeInterval(entrySpan)
        let outdated = WidgetMoment(date: over, name: payload.name, content: .gap(.outdated))
        guard now < over else {
            var moment = outdated
            moment.date = now
            return WidgetSchedule(moments: [moment], reload: now.addingTimeInterval(retry))
        }
        let moments = payload.entries(from: now).map { entry in
            WidgetMoment(
                date: entry.date, name: payload.name,
                content: kind.hasData(in: entry)
                    ? .entry(entry, places: payload.places) : .gap(.noData))
        }
        return WidgetSchedule(moments: moments + [outdated], reload: over)
    }

    /// The moment `kind` shows at `now` (the widget gallery's and a snapshot's).
    static func moment(
        _ kind: WidgetKind, payload: ViewModel.WidgetPayload?, name: String?, missing: WidgetGap,
        now: Date
    ) -> WidgetMoment {
        let moments = schedule(kind, payload: payload, name: name, missing: missing, now: now).moments
        return moments.last { $0.date <= now } ?? moments[0]
    }
}

// MARK: - Drawing in unit space

/// A point of a widget's drawing, `x` and `y` from 0 to 1 (`y` up).
nonisolated struct PlotPoint: Hashable, Sendable {
    var x: Double
    var y: Double
}

/// A labelled mark on the tide curve: a high or a low, with the core's label.
nonisolated struct TideMark: Hashable, Sendable {
    var kind: String
    var at: PlotPoint
    var label: String
}

/// A span along the tide curve's time axis (night), from 0 to 1.
nonisolated struct TideSpan: Hashable, Sendable {
    var start: Double
    var end: Double
}

/// A time label along the axis.
nonisolated struct TideTick: Hashable, Sendable {
    var x: Double
    var label: String
}

/// The part of a place's tide chart an entry shows, in unit space: the curve, the nights, the
/// highs and lows, now, and the major hour labels. Values keep the chart's display units; only
/// their place on the drawing is computed here.
nonisolated struct TideWindow: Hashable, Sendable {
    var curve: [PlotPoint]
    var nights: [TideSpan]
    var marks: [TideMark]
    var now: Double?
    var ticks: [TideTick]

    /// The window of `chart` from `start` to `end` (Unix seconds) with now at `now`. The curve
    /// keeps one point past each edge so it runs to the frame; it is scaled to the window's own
    /// lowest and highest values with a little room above and below.
    init(chart: ViewModel.Chart, start: Int64, end: Int64, now: Int64?) {
        let span = Double(max(end - start, 1))
        func x(_ t: Int64) -> Double { Double(t - start) / span }

        let points = (chart.series.first { $0.role == "predicted" } ?? chart.series.first)?.points ?? []
        let first = points.lastIndex { $0.t <= start } ?? 0
        let last = points.firstIndex { $0.t >= end } ?? (points.count - 1)
        let shown = first <= last ? Array(points[first...last]) : []
        let inside = shown.filter { $0.t >= start && $0.t <= end }.map(\.value)
        let values = inside.isEmpty ? shown.map(\.value) : inside
        let low = values.min() ?? 0
        let high = values.max() ?? 1
        let room = max(high - low, 0.1) * 0.12
        let bottom = low - room
        let height = max(high - low, 0.1) + 2 * room
        func y(_ value: Double) -> Double { (value - bottom) / height }

        curve = shown.map { PlotPoint(x: x($0.t), y: y($0.value)) }
        nights = chart.bands
            .filter { $0.kind == "night" && $0.endT > start && $0.startT < end }
            .map { TideSpan(start: max(x($0.startT), 0), end: min(x($0.endT), 1)) }
        marks = chart.markers.compactMap { marker in
            guard ["high", "low"].contains(marker.kind), marker.t >= start, marker.t <= end,
                let value = marker.value
            else { return nil }
            return TideMark(kind: marker.kind, at: PlotPoint(x: x(marker.t), y: y(value)), label: marker.label)
        }
        self.now = now.flatMap { $0 >= start && $0 <= end ? x($0) : nil }
        ticks = chart.xTicks
            .filter { $0.major && $0.t >= start && $0.t <= end }
            .map { TideTick(x: x($0.t), label: $0.label) }
    }
}

/// A week of river flow as a line from 0 to 1, oldest at the left; flat values sit in the
/// middle.
nonisolated enum FlowLine {
    static func points(_ values: [Double]) -> [PlotPoint] {
        guard values.count >= 2, let low = values.min(), let high = values.max() else { return [] }
        let range = high - low
        let step = 1 / Double(values.count - 1)
        return values.enumerated().map { index, value in
            PlotPoint(x: Double(index) * step, y: range > 0 ? (value - low) / range : 0.5)
        }
    }
}

// MARK: - Credits

/// The sources and the notices a widget shows at its foot: every part it draws credits its
/// sources, and each notice (not for navigation, provisional data) is shown whole, once.
nonisolated struct WidgetCredits: Hashable, Sendable {
    var credits: [String] = []
    var notices: [String] = []

    /// Adds `card`'s credit and notices, skipping ones already there.
    mutating func add(_ card: ViewModel.WidgetCard) {
        add(credit: card.credit)
        for notice in card.notices where !notices.contains(notice.text) {
            notices.append(notice.text)
        }
    }

    /// Adds the sources a credit names ("Computed on device · NOAA CO-OPS" names two), each
    /// once, so parts that share a source do not repeat it.
    mutating func add(credit: String) {
        for source in credit.components(separatedBy: Self.separator) {
            let source = source.trimmingCharacters(in: .whitespaces)
            if !source.isEmpty, !credits.contains(source) { credits.append(source) }
        }
    }

    /// How the core separates the sources in a credit, and the footer puts them back together.
    static let separator = " · "

    /// The sources in one line.
    var line: String { credits.joined(separator: Self.separator) }

    /// What a trip's widget credits: the trip's sources, then those of each place's parts it
    /// draws (`parts` per place, the largest sizes).
    static func trip(
        _ trip: ViewModel.WidgetTrip, places: [ViewModel.WidgetPlaceEntry], parts: Bool
    ) -> WidgetCredits {
        var credits = WidgetCredits()
        credits.add(credit: trip.credit)
        if parts {
            for place in places {
                for card in [place.tide?.card, place.flow?.card, place.snack].compactMap({ $0 }) {
                    credits.add(card)
                }
            }
        }
        return credits
    }
}
