import Foundation
@preconcurrency import UnderbiteKit

/// The trip-day Live Activity's content, from the core: whether today is one of a trip's days
/// is the trip view's to say, and the content (the next tide change, the slack, the sunset, the
/// words around them and when it goes stale and ends) is `tripActivity(...)`'s, built from the
/// places' last snapshots. Nothing is fetched here.
nonisolated enum TripActivityContent {
    /// Whether today is one of the trip's days at one of its places, each in its own zone.
    static func isTripDay(_ view: ViewModel.TripView) -> Bool {
        view.days.contains { day in day.places.contains { $0?.isToday == true } }
    }

    /// Whether today is one of `trip`'s days at Unix time `now`, from its view without data
    /// (the days need none).
    static func isTripDay(_ trip: SavedTrip, settingsJSON: String, now: Int64) -> Bool {
        guard
            let view = try? ViewModel.buildTrip(
                tripJSON: trip.json, places: [], cachedMetadata: [], settingsJSON: settingsJSON,
                now: Date(timeIntervalSince1970: TimeInterval(now)))
        else { return false }
        return isTripDay(view)
    }

    /// The content for `trip` at Unix time `now`, from its places' snapshots (`snapshot`, by
    /// place key: saved spots only) and what is kept for the trip; `nil` when the core cannot
    /// build it. Today need not be a trip day.
    static func activity(
        _ trip: SavedTrip, settingsJSON: String, snapshot: (String) -> String?, files: AppFiles,
        now: Int64
    ) -> ViewModel.TripDayActivity? {
        let keys = TripPage.placeKeys(trip, now: now)
        let places = TripData.read(trip.id, files: files).places(keys, snapshot: snapshot)
        guard
            let json = try? tripActivity(
                tripJson: trip.json, places: places, settingsJson: settingsJSON, now: now)
        else { return nil }
        return try? ViewModel.decodeTripActivity(json)
    }

    /// The content to show for `trip` at Unix time `now`: `nil` when today is none of its days
    /// (`view` says so when given, else a view without data is built).
    static func current(
        _ trip: SavedTrip, view: ViewModel.TripView? = nil, settingsJSON: String,
        snapshot: (String) -> String?, files: AppFiles, now: Int64
    ) -> ViewModel.TripDayActivity? {
        let today =
            view.map(isTripDay) ?? isTripDay(trip, settingsJSON: settingsJSON, now: now)
        guard today else { return nil }
        return activity(trip, settingsJSON: settingsJSON, snapshot: snapshot, files: files, now: now)
    }
}
