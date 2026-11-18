import AppIntents
import UnderbiteKit
import WidgetKit

/// A saved spot, or "the app's spot", as Edit Widget lists it: an id and a name, never where
/// the spot is.
struct WidgetSpotEntity: AppEntity {
    static let typeDisplayRepresentation: TypeDisplayRepresentation = "Spot"
    static let defaultQuery = WidgetSpotQuery()

    /// A saved spot's id, or `WidgetSpotChoice.appID`.
    var id: String
    var name: String

    var displayRepresentation: DisplayRepresentation {
        DisplayRepresentation(title: "\(name)")
    }

    init(id: String, name: String) {
        self.id = id
        self.name = name
    }

    init(_ spot: WidgetSpot) {
        self.init(id: spot.id, name: spot.name)
    }

    /// The spot the app shows, which the widget follows (the default).
    static let app = WidgetSpotEntity(id: WidgetSpotChoice.appID, name: "The app's spot")
}

/// "The app's spot", then the saved spots in the app's order, from the list the app keeps in
/// the shared container.
struct WidgetSpotQuery: EnumerableEntityQuery {
    func allEntities() async throws -> [WidgetSpotEntity] {
        [.app] + WidgetPayloadStore.shared.spots().spots.map(WidgetSpotEntity.init)
    }

    func entities(for identifiers: [String]) async throws -> [WidgetSpotEntity] {
        let spots = WidgetPayloadStore.shared.spots()
        return identifiers.compactMap { id in
            if id == WidgetSpotChoice.appID { return .app }
            return spots.spots(for: [id]).first.map(WidgetSpotEntity.init)
        }
    }

    func defaultResult() async -> WidgetSpotEntity? { .app }
}

/// Edit Widget for the tide, flow and snack time widgets: the spot. A widget never set (or set
/// to a spot since deleted) shows the app's spot.
struct SpotWidgetIntent: WidgetConfigurationIntent {
    static let title: LocalizedStringResource = "Spot"
    static let description = IntentDescription("Choose the spot the widget shows.")

    @Parameter(title: "Spot")
    var spot: WidgetSpotEntity?

    init() {}

    init(spot: WidgetSpotEntity?) {
        self.spot = spot
    }

    static var parameterSummary: some ParameterSummary {
        Summary("\(\.$spot)")
    }

    /// The choice, as the payload store reads it.
    var choice: WidgetSpotChoice { WidgetSpotChoice(id: spot?.id) }
}
