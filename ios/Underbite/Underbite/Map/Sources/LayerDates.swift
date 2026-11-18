import Foundation
import UnderbiteKit

/// The rules behind the dates and times the map's dated layers show: which layer the time
/// controls are for, a satellite layer's day and how it steps back past a day without a picture,
/// which times of a grid layer's time axis the controls offer, how "last clear day" decides, and
/// how the radar loop steps. Every date and time shown is a label from the core; nothing here
/// reads a calendar, touches MapKit or the network, so the rules are tested on their own.

/// The kind of control a dated layer gets.
nonisolated enum DateControl: Equatable, Sendable {
    /// A satellite picture per UTC day (GIBS): a date picker, stepping back past missing days.
    case satelliteDays
    /// A grid with one field per day (ERDDAP): a date picker over the days its time axis holds.
    case gridDays
    /// A grid with one field per hour (HF radar): a slider over the last day's hours.
    case gridHours
    /// The radar loop's frames: a slider and, outside the battery-friendly mode, play.
    case radarFrames

    /// The control for `layer`; none for a layer without dates.
    init?(_ layer: PlannedLayer) {
        switch (layer.kind, layer.info.dates) {
        case (.tiles, .daily): self = .satelliteDays
        case (.tiles, .frames): self = .radarFrames
        case (.gridRaster, .daily), (.gridArrows, .daily): self = .gridDays
        case (.gridRaster, .hourly), (.gridArrows, .hourly): self = .gridHours
        default: return nil
        }
    }

    /// The topmost layer on the map that has dates, and its control: the time controls are for
    /// it.
    static func topmost(_ plan: MapPlan) -> (layer: PlannedLayer, control: DateControl)? {
        for layer in plan.layers.reversed() {
            if let control = DateControl(layer) { return (layer, control) }
        }
        return nil
    }
}

/// A satellite layer's day. It opens on the core's `gibs_default_date` (yesterday in UTC, since
/// today's picture is partial) or on a day picked; when the tiles of the day asked for answer
/// "no data" (the day is not there yet, or never was), the layer asks for the day before, at
/// most ``maxSteps`` times. The day shown is always the day asked for: the date served.
nonisolated struct SatelliteDay: Equatable, Sendable {
    /// The most days a layer steps back from the day chosen.
    static let maxSteps = 7
    /// The "no data" tiles, with no image among them, that make a day count as missing: one
    /// tile could be a hole at the edge of the coverage.
    static let emptyTiles = 2
    /// The days the picker offers, the newest included.
    static let pickerDays = 14

    /// The day chosen: the default one or the day picked.
    let chosen: MapDate
    /// Whether the day was picked rather than the default.
    let picked: Bool
    /// The day asked for: the chosen one or a day before it.
    private(set) var asked: MapDate
    /// Days stepped back from the chosen one.
    private(set) var steps = 0
    /// Whether a tile of the day asked for came back as a picture.
    private(set) var served = false
    /// Whether every day back to the limit answered "no data".
    private(set) var exhausted = false

    init(chosen: MapDate, picked: Bool = false) {
        self.chosen = chosen
        self.picked = picked
        asked = chosen
    }

    /// Takes what the tiles of the day asked for came back as so far (`images` pictures, `empty`
    /// "no data" answers) and returns the day to ask for instead when the day has no picture;
    /// `previous` is the core's `previous_date`. A day that served one picture is kept.
    mutating func update(images: Int, empty: Int, previous: (String) -> MapDate?) -> MapDate? {
        guard !served, !exhausted else { return nil }
        if images > 0 {
            served = true
            return nil
        }
        guard empty >= Self.emptyTiles else { return nil }
        guard steps < Self.maxSteps, let before = previous(asked.date) else {
            exhausted = true
            return nil
        }
        steps += 1
        asked = before
        return before
    }

    /// The days the picker offers, newest first: `newest` and the days before it, as the core's
    /// `previous_date` names them.
    static func days(
        from newest: MapDate, count: Int = pickerDays, previous: (String) -> MapDate?
    ) -> [MapDate] {
        var out = [newest]
        while out.count < count, let before = previous(out[out.count - 1].date) {
            out.append(before)
        }
        return out
    }
}

/// The times of a grid layer's time axis (the core's `parse_time_axis`, oldest first) that the
/// controls offer and the requests take. A time is only ever one the axis holds: a time no longer
/// in it (MUR revises its newest week, a day can drop out) or past its end asks for the newest.
nonisolated enum GridTimes {
    /// How long a daily layer's time axis is kept before it is asked for again.
    static let dailyAxisAge: Int64 = 86_400
    /// How long an hourly layer's time axis is kept: its newest hour moves every hour.
    static let hourlyAxisAge: Int64 = 3_600
    /// How far back an hourly layer's slider reaches.
    static let hourlySpan: Int64 = 86_400

    /// How long an axis of a layer with `dates` is kept before it is asked for again.
    static func axisAge(_ dates: LayerDates) -> Int64 {
        dates == .hourly ? hourlyAxisAge : dailyAxisAge
    }

    /// Whether an axis fetched at `fetched` is to be asked for again at `now`.
    static func isStale(fetched: Int64, now: Int64, dates: LayerDates) -> Bool {
        let age = axisAge(dates)
        return now < fetched || now - fetched >= age
    }

    /// The times the control offers, oldest first: every time of a daily axis; the times of the
    /// last day of an hourly one (at least its newest).
    static func offered(_ axis: [LayerTime], dates: LayerDates, now: Int64) -> [LayerTime] {
        guard dates == .hourly, let newest = axis.last else { return axis }
        let recent = axis.filter { $0.time >= now - hourlySpan }
        return recent.isEmpty ? [newest] : recent
    }

    /// `time` (none: the newest) checked against the axis: a time the axis does not hold asks for
    /// the newest. The newest itself is asked for as none, as the layer opens.
    static func checked(_ time: Int64?, in axis: [LayerTime]) -> Int64? {
        guard let time, let newest = axis.last, time != newest.time,
            axis.contains(where: { $0.time == time })
        else { return nil }
        return time
    }

    /// The time shown, `time` (none: the newest) in the axis.
    static func shown(_ time: Int64?, in axis: [LayerTime]) -> LayerTime? {
        guard let time else { return axis.last }
        return axis.first { $0.time == time }
    }

    /// The time `by` places from `time` (none: the newest) among `offered`, as the request takes
    /// it (none for the newest); nothing beyond the ends.
    static func step(from time: Int64?, by: Int, in offered: [LayerTime]) -> Int64?? {
        guard !offered.isEmpty else { return nil }
        let at = time.flatMap { t in offered.firstIndex { $0.time == t } } ?? offered.count - 1
        let next = at + by
        guard offered.indices.contains(next) else { return nil }
        return .some(next == offered.count - 1 ? nil : offered[next].time)
    }

    /// The times "last clear day" tries, newest first, as the requests take them (none for the
    /// newest): the newest and the days before it, at most `limit`.
    static func clearDayTries(_ axis: [LayerTime], limit: Int = SatelliteDay.maxSteps + 1)
        -> [Int64?]
    {
        let newestFirst = axis.reversed().prefix(limit)
        return newestFirst.enumerated().map { $0.offset == 0 ? nil : $0.element.time }
    }
}

/// Watches one day of a grid layer while "last clear day" looks for one: the day is clear once
/// the values in view fit a colour range; cloudy once blocks arrived and stopped arriving with
/// none; given up on after ``longest`` checks.
nonisolated struct ClearDayWatch: Equatable, Sendable {
    enum Verdict: Equatable, Sendable {
        case waiting
        case clear
        case cloudy
        /// Nothing arrived in time (the server is busy or offline): stop looking.
        case unknown
    }

    /// The pause between two checks.
    static let interval: Duration = .milliseconds(500)
    /// Checks without a new block, after one arrived, before the day counts as cloudy.
    static let quiet = 4
    /// The most checks for one day.
    static let longest = 40

    private var revision: Int?
    private var arrived = false
    private var quietChecks = 0
    private var checks = 0

    /// - Parameters:
    ///   - revision: the layer's grid revision when the day was asked for.
    ///   - arrived: whether the day's blocks are already there or on their way (the day asked
    ///     for is the one on screen): then quiet alone makes it cloudy.
    init(revision: Int, arrived: Bool = false) {
        self.revision = revision
        self.arrived = arrived
    }

    /// One check: whether the view has values now and the layer's grid revision (raised as its
    /// blocks arrive).
    mutating func check(hasValues: Bool, revision: Int) -> Verdict {
        checks += 1
        if hasValues { return .clear }
        if revision != self.revision {
            self.revision = revision
            arrived = true
            quietChecks = 0
        } else {
            quietChecks += 1
        }
        if arrived, quietChecks >= Self.quiet { return .cloudy }
        return checks >= Self.longest ? .unknown : .waiting
    }
}

/// How the radar loop steps, as whether's: one frame a tick, the latest held a little longer, and
/// frames asked for again once a newer composite is due.
nonisolated enum RadarLoopRules {
    /// Ticks the latest frame stays on screen after the loop reaches it.
    static let hold = 3
    /// One frame a tick.
    static let tick: Duration = .milliseconds(500)
    /// How old the frames get before they are asked for again (a composite every five minutes).
    static let refreshAfter: Int64 = 300

    /// The step after `step` in a loop of `count` frames, and the frame it shows.
    static func next(after step: Int, count: Int) -> (step: Int, index: Int) {
        guard count > 0 else { return (0, 0) }
        let next = (step + 1) % (count + hold)
        return (next, min(next, count - 1))
    }

    /// Whether frames asked for at `loaded` are due again at `now`.
    static func isDue(loaded: Int64?, now: Int64) -> Bool {
        guard let loaded else { return true }
        return now < loaded || now - loaded >= refreshAfter
    }
}

/// A map tile deeper than its source's deepest zoom, cut from the ancestor at that zoom: MapKit
/// asks for nothing deeper than an overlay's `maximumZ`, so the overlay over-zooms by itself.
nonisolated struct OverZoomedTile: Equatable, Sendable {
    let z: Int
    let x: Int
    let y: Int
    /// The part of the ancestor the map tile shows; none when it is the tile itself.
    let crop: TileCrop?

    init(z: Int, x: Int, y: Int, deepest: Int) {
        guard z > deepest, deepest >= 0 else {
            (self.z, self.x, self.y, crop) = (z, x, y, nil)
            return
        }
        let levels = z - deepest
        let (ax, ay) = (x >> levels, y >> levels)
        let size = 1 / Double(1 << levels)
        (self.z, self.x, self.y) = (deepest, ax, ay)
        crop = TileCrop(
            left: Double(x - (ax << levels)) * size, top: Double(y - (ay << levels)) * size,
            size: size)
    }
}
