import Foundation
@preconcurrency import UnderbiteKit

/// A species of the catalogue as the core's species list names it, with the user's edits laid
/// over it. The package tests compile this file too.
nonisolated struct SpeciesListRow: Decodable, Equatable, Hashable, Identifiable, Sendable {
    /// The catalogue id (`morone-saxatilis`): what a trip or a catch keeps.
    var id: String
    /// "Striped bass".
    var name: String
    /// "Morone saxatilis".
    var scientificName: String
    /// "Salt water", in the settings' voice.
    var waterLabel: String?
    /// The user has edited the profile.
    var edited: Bool
}

/// Where a value of a profile came from, as the core words it.
nonisolated struct SpeciesSource: Decodable, Equatable, Hashable, Sendable {
    /// "FishBase", "NOAA Fisheries", "Your edit".
    var name: String
    /// `cc_by_nc4`, `public_domain`, `user_edited`, ...
    var licence: String
    /// "CC BY-NC 4.0".
    var licenceLabel: String
    var link: String?
    /// The record's own reference.
    var reference: String?
    /// The citation the source asks for.
    var citation: String?
}

/// One field of a profile page.
nonisolated struct SpeciesFieldRow: Decodable, Equatable, Hashable, Sendable {
    /// The field the editor changes (`spawn_temp`, `lures`, ...); none for the scientific name
    /// and the other names, which cannot be edited.
    var field: String?
    var label: String
    /// The value in the user's units, or the core's words for none.
    var value: String
    /// None when no source gives the field.
    var source: SpeciesSource?
    /// The value is an edit, not the catalogue's.
    var edited: Bool
    /// What the editor starts from, in the form `species_edit` reads.
    var edit: [String]
}

/// A source of a page, once, with its licence.
nonisolated struct SpeciesCredit: Decodable, Equatable, Hashable, Sendable {
    var source: String
    var text: String
    var link: String?
    var licence: String?
}

/// A species profile as the core's `ProfilePage`: every value with its source and licence.
nonisolated struct SpeciesProfilePage: Decodable, Equatable, Sendable {
    var id: String
    var name: String
    var scientificName: String
    var fields: [SpeciesFieldRow]
    /// Any field is edited.
    var edited: Bool
    var credits: [SpeciesCredit]
    /// The CC BY-NC notice when any value is from FishBase.
    var notices: [String]

    /// The page of the core's JSON, or `nil` when it cannot be read.
    static func decode(_ json: String) -> SpeciesProfilePage? {
        try? SpeciesJSON.decoder.decode(SpeciesProfilePage.self, from: Data(json.utf8))
    }
}

/// The three seasons of a species as the core's `SeasonView`, kept apart.
nonisolated struct SpeciesSeasons: Decodable, Equatable, Sendable {
    var id: String
    var name: String
    var now: Now
    var typical: Typical
    var legal: Legal

    /// The biological season: where the water puts the species now.
    struct Now: Decodable, Equatable, Sendable {
        var title: String
        var stageLabel: String
        /// "water 58 °F, spawn usually 60–65 °F".
        var text: String
        /// Safety text, shown whole.
        var caution: String?
        var rows: [Row]
        var credits: [SpeciesCredit]
    }

    struct Row: Decodable, Equatable, Hashable, Sendable {
        var label: String
        var value: String
        var detail: String?
    }

    /// The typical season: twelve months, January first.
    struct Typical: Decodable, Equatable, Sendable {
        var title: String
        var months: [Month]
        var note: String?
    }

    struct Month: Decodable, Equatable, Hashable, Sendable {
        var month: Int
        var label: String
        var stageLabel: String
        var water: String?
        var observations: String?
        var basis: String
    }

    /// The legal season: official pages and the user's notes, with the disclaimer.
    struct Legal: Decodable, Equatable, Sendable {
        var title: String
        var region: String?
        var links: [LegalLink]
        /// Why there are no links.
        var empty: String?
        var notesTitle: String
        var notes: String?
        /// Always shown.
        var disclaimer: String
    }

    struct LegalLink: Decodable, Equatable, Hashable, Sendable {
        var title: String
        /// "NYSDEC, link checked 2026-11-10".
        var detail: String
        var link: String
    }
}

/// The decoder of the core's species JSON (snake case keys).
nonisolated enum SpeciesJSON {
    static let decoder: JSONDecoder = {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return decoder
    }()
}

/// The core's species calls with the stored edits (`species-edits.json`'s text, empty for
/// none) and the settings, read into the types above.
nonisolated enum SpeciesCatalogue {
    /// The catalogue by name; empty when the settings or the edits cannot be read.
    static func rows(edits: String, settingsJSON: String) -> [SpeciesListRow] {
        guard let json = try? speciesList(edits: edits, settingsJson: settingsJSON),
            let rows = try? SpeciesJSON.decoder.decode([SpeciesListRow].self, from: Data(json.utf8))
        else { return [] }
        return rows
    }

    /// The profile page of `id`.
    static func profile(id: String, edits: String, settingsJSON: String) throws
        -> SpeciesProfilePage
    {
        let json = try speciesProfile(id: id, edits: edits, settingsJson: settingsJSON)
        return try SpeciesJSON.decoder.decode(SpeciesProfilePage.self, from: Data(json.utf8))
    }

    /// The seasons of `id`: now from the spot's snapshot when there is one, and the official
    /// pages of `region` (ISO 3166-2, `US-NY`).
    static func seasons(
        id: String, edits: String, snapshotJSON: String?, region: String?, notes: String = "",
        settingsJSON: String, now: Int64
    ) throws -> SpeciesSeasons {
        let region = region?.trimmingCharacters(in: .whitespacesAndNewlines).uppercased()
        let json = try speciesSeasons(
            id: id, edits: edits, snapshotJson: snapshotJSON,
            region: region?.isEmpty == false ? region : nil, notes: notes,
            settingsJson: settingsJSON, now: now)
        return try SpeciesJSON.decoder.decode(SpeciesSeasons.self, from: Data(json.utf8))
    }

    /// The choices of the `water`, `flow` and `light` fields; none for the others.
    static func choices(field: String, settingsJSON: String) -> [UnderbiteKit.SpeciesChoice] {
        (try? speciesEditChoices(field: field, settingsJson: settingsJSON)) ?? []
    }

    /// The symbol of the unit the editor's numbers of `field` are in ("°F", "ft"); empty for a
    /// field without a unit. Temperatures are in the water temperature's unit, depths in the
    /// tide's, as the core reads them.
    static func unitLabel(field: String, settingsJSON: String) -> String {
        let group: String
        switch SpeciesEditKind(field: field) {
        case .temperature: group = "water_temp"
        case .depth: group = "tide"
        default: return ""
        }
        guard let units = try? unitOptions(settingsJson: settingsJSON),
            let found = units.first(where: { $0.id == group })
        else { return "" }
        return found.options.first { $0.id == found.selected }?.label ?? ""
    }

    /// The core's text of a species error: why a value was refused, or what could not be read.
    static func message(of error: any Error) -> String {
        switch error {
        case SpeciesError.Field(_, let message), SpeciesError.Invalid(let message),
            SpeciesError.Edits(let message):
            message
        case let error as SpeciesError: String(describing: error)
        default: ShareText.message(of: error)
        }
    }
}

/// What the editor of a field looks like, from the field's id.
nonisolated enum SpeciesEditKind: Equatable, Sendable {
    /// Common names, habitat, lures, regions: one entry per value.
    case texts
    /// Water: one of the core's choices.
    case one
    /// Tide and current, light: any of the core's choices.
    case many
    /// Preferred, tolerated and spawning temperature: low and high.
    case temperature
    /// Depth: low and high for spring, summer, autumn and winter in turn.
    case depth

    init(field: String) {
        switch field {
        case "water": self = .one
        case "flow", "light": self = .many
        case "preferred_temp", "tolerated_temp", "spawn_temp": self = .temperature
        case "depth": self = .depth
        default: self = .texts
        }
    }
}

/// Searching the catalogue.
nonisolated enum SpeciesSearch {
    /// The rows whose common or scientific name holds `query`, ignoring case and accents
    /// ("STRIPED", "mórone"); every row for a blank query. Each word of the query must be in
    /// one of the names.
    static func matching(_ query: String, in rows: [SpeciesListRow]) -> [SpeciesListRow] {
        let words = query.split(whereSeparator: \.isWhitespace).map(String.init)
        guard !words.isEmpty else { return rows }
        return rows.filter { row in
            words.allSatisfy { word in
                contains(row.name, word) || contains(row.scientificName, word)
            }
        }
    }

    private static func contains(_ text: String, _ word: String) -> Bool {
        text.range(of: word, options: [.caseInsensitive, .diacriticInsensitive]) != nil
    }
}

/// What a species picker has chosen: catalogue ids, never names.
nonisolated struct SpeciesSelection: Equatable, Sendable {
    /// The chosen ids, in the order they were chosen.
    var ids: [String]
    /// More than one may be chosen (a trip); else one or none (a catch).
    var allowsMany: Bool

    func contains(_ id: String) -> Bool { ids.contains(id) }

    /// Chooses `id`: one more for a trip (or one less when it was chosen), the only one for a
    /// catch.
    mutating func choose(_ id: String) {
        if allowsMany {
            if let index = ids.firstIndex(of: id) {
                ids.remove(at: index)
            } else {
                ids.append(id)
            }
        } else {
            ids = [id]
        }
    }

    /// The chosen ids the catalogue does not have (a trip someone shared from a newer build),
    /// kept so that choosing others does not drop them.
    func unknown(in rows: [SpeciesListRow]) -> [String] {
        let known = Set(rows.map(\.id))
        return ids.filter { !known.contains($0) }
    }

    /// The rows of the chosen ids, in the chosen order.
    func chosenRows(in rows: [SpeciesListRow]) -> [SpeciesListRow] {
        ids.compactMap { id in rows.first { $0.id == id } }
    }
}
