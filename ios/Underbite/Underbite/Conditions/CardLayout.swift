import Foundation
import UnderbiteKit

/// One card of the customize list: its id as the settings write it, the title in the user's
/// voice and whether it shows. `available` is false for a card this version does not build yet.
nonisolated struct CardChoice: Identifiable, Equatable, Sendable {
    var id: String
    var title: String
    var shown: Bool
    var available: Bool

    init(id: String, title: String, shown: Bool, available: Bool) {
        self.id = id
        self.title = title
        self.shown = shown
        self.available = available
    }

    init(_ info: CardInfo) {
        self.init(id: info.id, title: info.title, shown: info.shown, available: info.available)
    }
}

/// Every card of a layout in one list. The editor shows and reorders the cards this version
/// builds; the others keep their places and their visibility.
nonisolated struct CardChoices: Equatable, Sendable {
    var all: [CardChoice]

    /// The cards the editor offers.
    var editable: [CardChoice] { all.filter(\.available) }

    /// Puts `edited` (the editable cards, in their new order) back into the editable places.
    mutating func setEditable(_ edited: [CardChoice]) {
        var next = edited.makeIterator()
        for index in all.indices where all[index].available {
            if let choice = next.next() { all[index] = choice }
        }
    }

    /// What the settings keep: a card this version does not build and that is not shown is in
    /// neither list, so it stays off when a later version builds it.
    private var kept: [CardChoice] { all.filter { $0.available || $0.shown } }
    var order: [String] { kept.map(\.id) }
    var hidden: [String] { kept.filter { !$0.shown }.map(\.id) }
}

nonisolated extension Settings.Cards {
    /// The layout written for `waterType` (`"river_stream"`, ...), if any.
    func layout(for waterType: String) -> Settings.CardLayout? {
        switch waterType {
        case "river_stream": river
        case "lake_reservoir": lake
        case "tidal_river_estuary": tidal
        case "bay_inshore": bay
        case "surf": surf
        case "nearshore": nearshore
        case "offshore": offshore
        case "great_lakes": greatLakes
        case "pond": pond
        case "ice": ice
        default: nil
        }
    }

    mutating func setLayout(_ layout: Settings.CardLayout?, for waterType: String) {
        switch waterType {
        case "river_stream": river = layout
        case "lake_reservoir": lake = layout
        case "tidal_river_estuary": tidal = layout
        case "bay_inshore": bay = layout
        case "surf": surf = layout
        case "nearshore": nearshore = layout
        case "offshore": offshore = layout
        case "great_lakes": greatLakes = layout
        case "pond": pond = layout
        case "ice": ice = layout
        default: break
        }
    }
}

/// Reading and writing the card layout the way the configuration defines it: a base order and
/// hidden list, and an optional layout per water type that replaces them for that type. The core
/// stays the authority on what a water type shows (`cardCatalogue`); this only decides where an
/// edit is written.
nonisolated enum CardLayoutEditing {
    /// Titles do not depend on the water type.
    private static let titleType = "lake_reservoir"

    /// The cards of `waterType` as the core resolves them, or of the base layout when `nil`.
    static func choices(settings: Settings, waterType: String?) throws -> CardChoices {
        let json = try settings.json()
        if let waterType {
            return CardChoices(
                all: try cardCatalogue(waterType: waterType, settingsJson: json).map(CardChoice.init))
        }
        let catalogue = try cardCatalogue(waterType: titleType, settingsJson: json)
        let known = Dictionary(catalogue.map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })
        var seen = Set<String>()
        let ordered = settings.cards.order.filter { known[$0] != nil && seen.insert($0).inserted }
        let rest = catalogue.map(\.id).filter { !seen.contains($0) }
        let hidden = Set(settings.cards.hidden)
        return CardChoices(
            all: (ordered + rest).compactMap { id in
                guard let info = known[id] else { return nil }
                var choice = CardChoice(info)
                choice.shown = seen.contains(id) && !hidden.contains(id)
                return choice
            })
    }

    /// Whether `waterType` shows other cards than the base layout does (a layout of its own,
    /// or one the core ships for it, such as rivers and ponds).
    static func usesOwnLayout(settings: Settings, waterType: String) throws -> Bool {
        let shown = try choices(settings: settings, waterType: waterType).all
            .filter(\.shown).map(\.id)
        let hidden = Set(settings.cards.hidden)
        return shown != settings.cards.order.filter { !hidden.contains($0) }
    }

    /// Writes `choices` to the layout of `waterType` when it is `ownLayout`, else to the base
    /// layout (and to the type's written layout, which then has the same cards, so that it
    /// keeps following the base).
    static func apply(
        _ choices: CardChoices, to settings: inout Settings, waterType: String?, ownLayout: Bool
    ) {
        let layout = Settings.CardLayout(order: choices.order, hidden: choices.hidden)
        if let waterType, ownLayout {
            settings.cards.setLayout(layout, for: waterType)
            return
        }
        settings.cards.order = layout.order
        settings.cards.hidden = layout.hidden
        if let waterType, settings.cards.layout(for: waterType) != nil {
            settings.cards.setLayout(layout, for: waterType)
        }
    }

    /// Gives `waterType` a layout of its own (a copy of `choices`), or takes it back to the
    /// base layout.
    static func setOwnLayout(
        _ own: Bool, choices: CardChoices, in settings: inout Settings, waterType: String
    ) throws {
        if own {
            settings.cards.setLayout(
                Settings.CardLayout(order: choices.order, hidden: choices.hidden), for: waterType)
            return
        }
        settings.cards.setLayout(nil, for: waterType)
        // A layout the core ships (rivers) stays in force until one that follows the base is written.
        if try usesOwnLayout(settings: settings, waterType: waterType) {
            settings.cards.setLayout(
                Settings.CardLayout(order: settings.cards.order, hidden: settings.cards.hidden),
                for: waterType)
        }
    }

    /// The default layout again: the type's own when `ownLayout`, else the base.
    static func restoreDefaults(
        in settings: inout Settings, waterType: String?, ownLayout: Bool
    ) throws {
        if let waterType, ownLayout {
            settings.cards.setLayout(nil, for: waterType)
            return
        }
        let defaults = try Settings.defaults().cards
        settings.cards.order = defaults.order
        settings.cards.hidden = defaults.hidden
        if let waterType { settings.cards.setLayout(defaults.layout(for: waterType), for: waterType) }
    }

    /// `settings` with card `id` shown or hidden for `waterType`, written where its edits go.
    static func setting(
        _ id: String, shown: Bool, in settings: Settings, waterType: String
    ) throws -> Settings {
        var settings = settings
        var all = try choices(settings: settings, waterType: waterType)
        guard let index = all.all.firstIndex(where: { $0.id == id }) else { return settings }
        all.all[index].shown = shown
        let own = try usesOwnLayout(settings: settings, waterType: waterType)
        apply(all, to: &settings, waterType: waterType, ownLayout: own)
        return settings
    }
}
