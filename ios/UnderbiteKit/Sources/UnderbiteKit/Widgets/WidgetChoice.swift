import Foundation

/// The widgets the extension offers.
public enum WidgetKind: String, CaseIterable, Sendable, Codable {
    /// A spot's tide curve with its highs and lows.
    case tide
    /// A spot's river flow with its trend and against normal.
    case flow
    /// A spot's next snack time (fishing window).
    case snack
    /// The next trip's go/no-go.
    case nextTrip = "next_trip"

    /// Whether the widget is configured with a spot (the next trip's is not).
    public var showsSpot: Bool { self != .nextTrip }

    /// Whether `entry` has what this widget draws for the place at `place` (the next trip's
    /// widget needs the trip's go/no-go). Without it the widget shows its placeholder.
    public func hasData(in entry: ViewModel.WidgetEntry, place: Int = 0) -> Bool {
        let part = entry.places.indices.contains(place) ? entry.places[place] : nil
        switch self {
        case .tide: return part?.tide != nil
        case .flow: return part?.flow != nil
        case .snack: return part?.snack != nil
        case .nextTrip: return entry.trip != nil
        }
    }
}

/// Which spot a widget was set to show (Edit Widget): the one the app shows, or a saved spot.
public enum WidgetSpotChoice: Hashable, Sendable {
    /// The spot the app shows: the widget follows it.
    case app
    /// A saved spot, by id.
    case spot(String)

    /// The id the widget's configuration keeps for "the app's". No spot has it.
    public static let appID = "@app"

    /// The choice a configuration's id names; none (a widget never configured) is the app's.
    public init(id: String?) {
        guard let id, !id.isEmpty, id != Self.appID else {
            self = .app
            return
        }
        self = .spot(id)
    }

    /// The id the configuration keeps.
    public var id: String {
        switch self {
        case .app: Self.appID
        case .spot(let id): id
        }
    }
}

/// A saved spot as the widgets' configuration lists it: its id and name, never where it is.
public struct WidgetSpot: Codable, Sendable, Hashable, Identifiable {
    public var id: String
    public var name: String

    public init(id: String, name: String) {
        self.id = id
        self.name = name
    }
}

/// The saved spots the app tells its widgets about, in the app's order, and the one it shows.
public struct WidgetSpots: Codable, Sendable, Hashable {
    public var spots: [WidgetSpot]
    /// The spot the app shows, which "the app's" widgets follow.
    public var appSpotID: String?

    public init(spots: [WidgetSpot] = [], appSpotID: String? = nil) {
        self.spots = spots
        self.appSpotID = appSpotID
    }

    /// The spot a widget set to `choice` shows: the chosen spot while it is saved, else (the
    /// app's, or a spot deleted since) the app's spot, else the first; `nil` with no spots.
    public func spot(for choice: WidgetSpotChoice) -> WidgetSpot? {
        if case .spot(let id) = choice, let chosen = spots.first(where: { $0.id == id }) {
            return chosen
        }
        if let appSpotID, let shown = spots.first(where: { $0.id == appSpotID }) {
            return shown
        }
        return spots.first
    }

    /// Whether a widget set to `choice` follows the app's spot: it was set to, or its spot was
    /// deleted.
    public func followsApp(_ choice: WidgetSpotChoice) -> Bool {
        guard case .spot(let id) = choice else { return true }
        return !spots.contains { $0.id == id }
    }

    /// The spots for the configuration's ids that are saved, in the ids' order (an entity
    /// lookup); "the app's" id is left to the caller, which names it.
    public func spots(for ids: [String]) -> [WidgetSpot] {
        ids.compactMap { id in spots.first { $0.id == id } }
    }
}
