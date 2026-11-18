import Foundation

/// The trip view from the Rust core (`underbite_core::view::trip`), as `tripView(...)` returns
/// it: a trip's places as columns and its days as rows of worded cells, each saying what it
/// rests on. Every value and word is already formatted: draw it, never convert or format it.
///
/// As with the page, unknown fields are ignored and enum-like fields (kinds, bases, verdicts)
/// stay strings, so what a later core adds never fails the trip.
extension ViewModel {
    /// Decodes the JSON `tripView(...)` returns.
    public static func decodeTrip(_ json: String) throws -> TripView {
        try decoder().decode(TripView.self, from: Data(json.utf8))
    }

    /// Builds the view of `tripJSON` from what the app holds for its places and decodes it.
    /// Reads nothing itself; call it off the main actor, since the core computes the tides of
    /// every day.
    public static func buildTrip(
        tripJSON: String, places: [TripPlaceData], cachedMetadata: [CachedResponse],
        app: TripAppStatus = TripAppStatus(pack: .notDownloaded, sharing: .notShared),
        settingsJSON: String, now: Date = .now
    ) throws -> TripView {
        let json = try tripView(
            tripJson: tripJSON, places: places, cachedMetadata: cachedMetadata, app: app,
            settingsJson: settingsJSON, now: Int64(now.timeIntervalSince1970))
        return try decodeTrip(json)
    }

    /// A trip as shown: its places, its days, its checklist and its credits.
    public struct TripView: Decodable, Sendable, Hashable {
        /// The trip's name.
        public var title: String
        /// "Nov 8 to Nov 10"
        public var dates: String
        /// The first day, `YYYY-MM-DD`, for ordering only.
        public var first: String
        /// "Starts in 3 days", "Under way", "This trip is over".
        public var status: String
        public var isPast: Bool
        public var isUpcoming: Bool
        /// The columns of the grid, in the trip's order.
        public var places: [TripPlaceColumn]
        /// One per date, first to last.
        public var days: [TripDayRow]
        public var checklist: [ChecklistRow]
        /// Every source a cell rests on.
        public var attribution: [Credit]
        /// Not for navigation, approximate tides, the traditional cue: safety text, whole.
        public var notices: [Notice]
        /// The trip's notes, as the user wrote them.
        public var notes: String
        /// The widget form.
        public var compact: TripCompact

        /// The column whose key is `key`.
        public func place(_ key: String) -> TripPlaceColumn? {
            places.first { $0.key == key }
        }
    }

    /// The "next trip: go/no-go" form for widgets. `verdict` is `go`, `marginal`, `no_go` or
    /// `unknown`.
    public struct TripCompact: Decodable, Sendable, Hashable {
        public var title: String
        public var when: String
        public var verdict: String?
        public var headline: String
        public var line: String
        public var storm: Bool
        public var accessibility: String
    }

    /// One place, the header of its column.
    public struct TripPlaceColumn: Decodable, Sendable, Hashable, Identifiable {
        /// The place's key in the trip: its spot id.
        public var key: String
        public var name: String
        public var area: String?
        public var waterTypeLabel: String
        /// "Approximate location" for a place shared rounded or by area only.
        public var approximate: String?
        /// Why the place has no days.
        public var problem: String?

        public var id: String { key }
    }

    /// One date across the trip's places.
    public struct TripDayRow: Decodable, Sendable, Hashable, Identifiable {
        /// `YYYY-MM-DD`
        public var date: String
        /// "Sun Nov 8"
        public var label: String
        /// One per column, in the same order; `nil` where the place has no such day.
        public var places: [TripPlaceDay?]
        /// The whole row as one sentence, the plain meaning first.
        public var accessibility: String

        public var id: String { date }
    }

    /// One place on one date. `verdict` is `go`, `marginal`, `no_go` or `unknown`, absent when
    /// the day has none.
    public struct TripPlaceDay: Decodable, Sendable, Hashable {
        /// The column's key.
        public var place: String
        /// The day is over in the place's zone.
        public var isPast: Bool
        /// The day is today in the place's zone.
        public var isToday: Bool
        public var verdict: String?
        /// A storm ahead or a hurricane cone over the place on this day.
        public var storm: Bool
        /// One per kind, in the core's order (verdict, windows, tides, flow, water temperature,
        /// sun, moon, solunar).
        public var cells: [TripCell]
        /// The verdict's reasons, storms first; safety reasons carry their links.
        public var reasons: [GoNoGoReasonVm]
        /// The day's fishing windows with their factors.
        public var windows: [WindowVm]
        /// The place's day as one sentence, the plain meaning first.
        public var accessibility: String

        /// The cell of `kind` (`tides`, `flow`, ...), if the day has one.
        public func cell(_ kind: String) -> TripCell? {
            cells.first { $0.kind == kind }
        }
    }

    /// One measurement of one place on one day.
    ///
    /// `kind` is `verdict`, `windows`, `tides`, `flow`, `water_temp`, `sun`, `moon` or `solunar`;
    /// `basis` is `certain`, `forecast`, `typical` (past years' values: history, never a
    /// forecast) or `unavailable`.
    public struct TripCell: Decodable, Sendable, Hashable {
        public var kind: String
        /// "Tide", "River flow", "Snack times".
        public var label: String
        /// The short value for the grid: "Go", "706 cfs", "Low 1:59 AM"; "—" when unavailable.
        public var value: String
        /// The details: every high and low, the range against the user's, past years' spread.
        public var lines: [String]
        public var basis: String
        /// "certain", "forecast, issued Nov 8, 9:30 AM", "typical for these dates, 1991–2020,
        /// station records", "unavailable: outside the forecast". Always shown.
        public var basisLabel: String
        /// The source's name, where there is one.
        public var source: String?
        /// Label, basis, value and lines; the plain meaning first.
        public var accessibility: String

        /// The value is past years' values for these dates.
        public var isTypical: Bool { basis == "typical" }
        /// The cell has no value.
        public var isUnavailable: Bool { basis == "unavailable" }
    }

    /// One line of the checklist. `kind` is `licence`, `regulations`, `disclaimer`, `item`,
    /// `gear`, `offline_pack` or `sharing`; `done` is absent where ticking does not apply.
    public struct ChecklistRow: Decodable, Sendable, Hashable, Identifiable {
        public var kind: String
        /// Stable across builds.
        public var id: String
        public var text: String
        public var detail: String?
        public var link: String?
        public var done: Bool?
    }
}
