import Foundation
@preconcurrency import UnderbiteKit

/// What worked, read from the core's `InsightsView`: the catches that pass the filters counted
/// along each condition. Every word, label and sentence is the core's; a bar's length is its
/// `fraction`, so no number is written here.
nonisolated struct InsightsPage: Decodable, Equatable, Sendable {
    /// One value of a condition and how many catches had it.
    struct Bar: Decodable, Equatable, Identifiable, Sendable {
        /// A stable id of the value ("rising", "high-2", "from-12").
        var key: String
        var label: String
        var count: Int
        /// The count against the largest bar of its section, 0 to 1.
        var fraction: Double
        /// The one value with more catches than any other.
        var peak: Bool
        /// What VoiceOver reads for the bar.
        var spoken: String

        var id: String { key }
    }

    /// The catches counted along one condition.
    struct Section: Decodable, Equatable, Identifiable, Sendable {
        /// "tide_direction", "water_temperature", ...
        var kind: String
        var title: String
        /// Empty when too few catches have the condition recorded.
        var bars: [Bar]
        var sentence: String?
        var tooFew: String?
        var unknown: String?

        var id: String { kind }
        /// Whether the section has bars to draw.
        var charted: Bool { !bars.isEmpty }
    }

    var total: Int
    var headline: String
    /// Set when there are no catches at all.
    var empty: String?
    /// Set when there are some, but too few to tell anything.
    var tooFew: String?
    /// Counts say when fish were caught, not when fishing is best.
    var notice: String
    var sections: [Section]

    /// The sections with bars, in the core's order, then those with too few catches, so the
    /// charts come first.
    var arranged: [Section] {
        sections.filter(\.charted) + sections.filter { !$0.charted }
    }

    /// The core's page for the catches of `book` that pass `filter`, in the user's units and
    /// voice; `nil` when the log or the settings cannot be read.
    static func build(book: Data, filter: CatchFilter, settingsJSON: String) -> InsightsPage? {
        guard let json = try? insightsJson(book: book, filter: filter, settingsJson: settingsJSON)
        else { return nil }
        return try? CatchJSON.decoder.decode(InsightsPage.self, from: Data(json.utf8))
    }

    /// `sections` in rows of `columns`, for a grid that keeps each row's charts side by side.
    static func rows(_ sections: [Section], columns: Int) -> [[Section]] {
        let width = max(columns, 1)
        return stride(from: 0, to: sections.count, by: width).map {
            Array(sections[$0..<min($0 + width, sections.count)])
        }
    }
}

/// Which catches what worked counts: a species, a place, and a span of days. A choice left
/// `nil` lets every catch through.
nonisolated struct InsightsFilter: Equatable, Hashable, Sendable {
    /// A place catches are filed under: a saved spot, or an area typed with the catch.
    enum Place: Equatable, Hashable, Sendable {
        case spot(String)
        case area(String)
    }

    /// A catalogue species id.
    var species: String?
    var place: Place?
    /// The first day counted.
    var from: Date?
    /// The last day counted.
    var until: Date?

    var isActive: Bool { species != nil || place != nil || from != nil || until != nil }

    /// The core's filter: `from` from the start of its day, `until` to the end of its day, both
    /// days as `calendar` reads them.
    func catchFilter(calendar: Calendar) -> CatchFilter {
        var spot: String?
        var area: String?
        switch place {
        case .spot(let id): spot = id
        case .area(let name): area = name
        case nil: break
        }
        let start = from.map { Int64(calendar.startOfDay(for: $0).timeIntervalSince1970) }
        let end = until.flatMap { day in
            calendar.date(byAdding: .day, value: 1, to: calendar.startOfDay(for: day))
        }.map { Int64($0.timeIntervalSince1970) }
        return CatchFilter(
            species: species, spotId: spot, area: area, from: start, until: end, method: nil)
    }

    /// Drops a species or place `choices` no longer offers (after a delete), so the filter never
    /// hides every catch for a reason the controls cannot show.
    func pruned(for choices: InsightsChoices) -> InsightsFilter {
        var out = self
        if let species, !choices.species.contains(where: { $0.id == species }) {
            out.species = nil
        }
        if let place, !choices.places.contains(where: { $0.place == place }) {
            out.place = nil
        }
        return out
    }
}

/// What the filters offer: the species and places the log's catches have, in the order the
/// list first shows them (newest first).
nonisolated struct InsightsChoices: Equatable, Sendable {
    /// A place to choose, with the name the list gives it.
    struct PlaceChoice: Equatable, Hashable, Sendable {
        var place: InsightsFilter.Place
        var name: String
    }

    var species: [SpeciesChoice]
    var places: [PlaceChoice]

    /// The choices of `rows`: a species is offered when the catalogue has it (a species typed
    /// by hand has no id to count by); a place is the saved spot of that name, else the area.
    init(rows: [CatchRow], catalogue: [SpeciesChoice], spots: [PlaceName]) {
        var seenSpecies = Set<String>()
        species = rows.compactMap { row in
            guard seenSpecies.insert(row.species).inserted else { return nil }
            return catalogue.first { $0.name == row.species }
        }
        var seenPlaces = Set<String>()
        places = rows.compactMap(\.place).compactMap { name in
            guard seenPlaces.insert(name).inserted else { return nil }
            if let spot = spots.first(where: { $0.name == name }) {
                return PlaceChoice(place: .spot(spot.spotId), name: name)
            }
            return PlaceChoice(place: .area(name), name: name)
        }
    }
}
