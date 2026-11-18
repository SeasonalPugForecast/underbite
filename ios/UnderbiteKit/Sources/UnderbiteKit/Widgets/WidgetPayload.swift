import Foundation

/// The widget and Live Activity payloads from the Rust core (`underbite_core::view::widget`), as
/// `spotWidget(...)`, `tripWidget(...)` and `tripActivity(...)` return them: entries built
/// ahead for the next hours, every value and word already formatted, each part with its credit
/// line. Draw them, never convert or format them; only the countdown is the system timer's.
///
/// As with the page, unknown fields are ignored and enum-like fields stay strings, so what a
/// later core adds never fails a widget.
extension ViewModel {
    /// Decodes a widget payload's JSON.
    public static func decodeWidget(_ json: String) throws -> WidgetPayload {
        try decodeWidget(Data(json.utf8))
    }

    /// Decodes a widget payload's JSON, as the payload store keeps it.
    public static func decodeWidget(_ data: Data) throws -> WidgetPayload {
        try decoder().decode(WidgetPayload.self, from: data)
    }

    /// Decodes the trip-day Live Activity's JSON.
    public static func decodeTripActivity(_ json: String) throws -> TripDayActivity {
        try decoder().decode(TripDayActivity.self, from: Data(json.utf8))
    }

    /// Everything a widget reads for one spot or one trip.
    public struct WidgetPayload: Decodable, Sendable, Hashable {
        /// `spot` or `trip`.
        public var subject: String
        /// The spot's or the trip's id.
        public var id: String
        /// The spot's or the trip's name.
        public var name: String
        /// When the payload was built, Unix seconds.
        public var generatedAt: Int64
        /// One for a spot; for a trip, its first two places with data (the largest sizes show
        /// both).
        public var places: [WidgetPlace]
        /// The first at `generatedAt`, then one on each local hour for 12 hours.
        public var entries: [WidgetEntry]

        public var isSpot: Bool { subject == "spot" }
        public var isTrip: Bool { subject == "trip" }

        /// The entry current at `date`: the last one that has begun, else the first.
        public func entry(at date: Date) -> WidgetEntry? {
            let t = Int64(date.timeIntervalSince1970.rounded(.down))
            return entries.last { $0.t <= t } ?? entries.first
        }

        /// The entry current at `date` and every later one, for a timeline built after the
        /// payload was.
        public func entries(from date: Date) -> [WidgetEntry] {
            guard let current = entry(at: date) else { return [] }
            return entries.filter { $0.t >= current.t }
        }

        /// When the last entry begins: the payload has nothing newer to show after it.
        public var lastEntryDate: Date? {
            entries.last.map { Date(timeIntervalSince1970: TimeInterval($0.t)) }
        }
    }

    /// What a place's entries share.
    public struct WidgetPlace: Decodable, Sendable, Hashable, Identifiable {
        /// The spot's id.
        public var id: String
        public var name: String
        /// The tide from 3 h before the first entry to 21 h after the last, every 30 minutes,
        /// with the highs and lows labelled and night shaded. Each entry's tide says which part
        /// to show and marks now at its own `t` (the chart's `nowT` is the first entry's).
        public var tideChart: Chart?
        /// A week of river flow (or stage) in display units, oldest first; empty without a gauge.
        public var flowSpark: [Double]
    }

    /// What the widgets show from one instant on.
    public struct WidgetEntry: Decodable, Sendable, Hashable {
        /// When the entry becomes current, Unix seconds.
        public var t: Int64
        /// One per place of the payload, in its order.
        public var places: [WidgetPlaceEntry]
        /// The trip's go/no-go; trip payloads only.
        public var trip: WidgetTrip?

        public var date: Date { Date(timeIntervalSince1970: TimeInterval(t)) }
    }

    /// One place at an entry's instant; a part is absent when the place has no such data.
    public struct WidgetPlaceEntry: Decodable, Sendable, Hashable {
        public var tide: WidgetTide?
        public var flow: WidgetFlow?
        /// The next fishing window: "snack time" in Luna's voice.
        public var snack: WidgetCard?
    }

    /// A card's compact form with what a widget shows beside it.
    public struct WidgetCard: Decodable, Sendable, Hashable {
        /// "Tide", "River flow", "Snack times".
        public var title: String
        public var headline: Quantity
        /// At most two.
        public var lines: [String]
        /// Stale and far values are greyed with the reason.
        public var state: CardState
        /// The sources' names: "NOAA CO-OPS". Always shown.
        public var credit: String
        /// "Not for navigation." with tides: safety text, shown whole.
        public var notices: [Notice]
        public var accessibility: String
    }

    /// The tide at an entry. `stage` is `rising`, `falling`, `high_slack`, `low_slack` or
    /// `unknown`, for the arrow.
    public struct WidgetTide: Decodable, Sendable, Hashable {
        public var card: WidgetCard
        public var stage: String
        /// The part of the place's `tideChart` to show, Unix seconds.
        public var startT: Int64
        public var endT: Int64
    }

    /// The river flow at an entry.
    public struct WidgetFlow: Decodable, Sendable, Hashable {
        public var card: WidgetCard
        /// The direction for the arrow, and "Falling 0.5 cfs in 14 h".
        public var trend: Trend?
        /// Against normal for the date, short: "Normal", "Below normal".
        public var normal: String?
    }

    /// The next trip's go/no-go at an entry.
    public struct WidgetTrip: Decodable, Sendable, Hashable {
        public var compact: TripCompact
        /// The sources' names the trip rests on.
        public var credit: String
    }
}

// MARK: - The trip-day Live Activity

extension ViewModel {
    /// The trip-day Live Activity's content at one place: target instants for the system timer
    /// and every word around them. It is the activity's content state as it is (Codable, so
    /// ActivityKit can carry it), and small: with the attributes, well under the 4 KB ActivityKit
    /// allows.
    public struct TripDayActivity: Codable, Sendable, Hashable {
        /// The trip's name.
        public var title: String
        /// The place the times are for.
        public var place: String
        /// When the content was built, Unix seconds.
        public var generatedAt: Int64
        /// The next high or low.
        public var nextTideChange: ActivityTarget?
        /// The next slack water; current stations only.
        public var slack: ActivityTarget?
        /// Today's sunset while it is ahead.
        public var sunset: ActivityTarget?
        /// When the content goes out of date, Unix seconds: the first target, else `endsAt`.
        public var staleAt: Int64
        /// What to show once stale.
        public var stale: String
        /// When to end the activity, Unix seconds: today's sunset, else the end of the day.
        public var endsAt: Int64
        /// The sources' names. Always shown.
        public var credit: String
        /// "Not for navigation." and any other tide or current notice: safety text, shown whole.
        public var notices: [ActivityNotice]
        public var accessibility: String

        /// The targets there are, in the core's order (tide, slack, sunset).
        public var targets: [ActivityTarget] {
            [nextTideChange, slack, sunset].compactMap { $0 }
        }

        /// The content's stale date, for ActivityKit.
        public var staleDate: Date { Date(timeIntervalSince1970: TimeInterval(staleAt)) }
        /// When the activity ends.
        public var endDate: Date { Date(timeIntervalSince1970: TimeInterval(endsAt)) }
        /// Whether the content is out of date at `date`.
        public func isStale(at date: Date) -> Bool { date >= staleDate }
        /// Whether the activity is over at `date`.
        public func isOver(at date: Date) -> Bool { date >= endDate }
    }

    /// A time the Live Activity counts down to, with its words.
    public struct ActivityTarget: Codable, Sendable, Hashable {
        /// The instant the system timer counts down to, Unix seconds.
        public var at: Int64
        /// What stands beside the timer: "High tide in".
        public var label: String
        /// The target in words, for the compact views and once stale: "High 5.6 ft MLLW at
        /// 7:07 AM".
        public var text: String

        /// The instant, for the system timer (`Text(timerInterval:)`).
        public var date: Date { Date(timeIntervalSince1970: TimeInterval(at)) }
    }

    /// A Live Activity's fixed notice: plain, never in Luna's voice (as ``Notice``, carried by
    /// ActivityKit too).
    public struct ActivityNotice: Codable, Sendable, Hashable {
        public var kind: String
        public var text: String
    }
}
