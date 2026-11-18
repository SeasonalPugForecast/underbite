import Foundation
@preconcurrency import UnderbiteKit

/// Which saved spots a background refresh fetches for the widgets on the Home and Lock Screen:
/// the spot each installed spot widget shows, and the next trip's places for a next-trip widget.
/// The other spots' widget data is built again from their last snapshots, without a fetch.
nonisolated enum WidgetNeeds {
    /// An installed widget, as WidgetKit lists it.
    struct Installed: Hashable, Sendable {
        /// Its kind; `nil` for a kind this version does not know, which is taken to show a spot.
        var kind: WidgetKind?
        /// The spot it was set to show; a widget whose setting cannot be read follows the app's.
        var choice: WidgetSpotChoice

        init(kind: WidgetKind?, choice: WidgetSpotChoice = .app) {
            self.kind = kind
            self.choice = choice
        }
    }

    /// The ids to fetch, once each, in the order of `installed`: the spot each spot widget shows
    /// among `spots` (a deleted or unknown choice falls back as the widget does), and
    /// `nextTripPlaces` for a next-trip widget.
    static func spotIDs(
        installed: [Installed], spots: WidgetSpots, nextTripPlaces: [String]
    ) -> [String] {
        var ids: [String] = []
        for widget in installed {
            if widget.kind == .nextTrip {
                ids += nextTripPlaces
            } else if let spot = spots.spot(for: widget.choice) {
                ids.append(spot.id)
            }
        }
        var seen = Set<String>()
        return ids.filter { seen.insert($0).inserted }
    }

    /// The places of `trip` (its JSON) that are saved spots (`saved`) and that a forecast can
    /// still reach at Unix time `now`: the next-trip widget's go/no-go reads their snapshots.
    static func tripPlaces(_ trip: String, saved: Set<String>, now: Int64) -> [String] {
        let places = (try? tripSpots(tripJson: trip, now: now)) ?? []
        return places.filter { $0.forecast && saved.contains($0.key) }.map(\.key)
    }
}
