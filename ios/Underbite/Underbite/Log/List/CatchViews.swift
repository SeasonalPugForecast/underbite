import Foundation
@preconcurrency import UnderbiteKit

/// What became of a fish, as the core's rows name it.
nonisolated enum CatchFate: String, Decodable, CaseIterable, Sendable {
    case released, kept
    case keptForSashimi = "kept_for_sashimi"
}

/// One catch of the list, read from the core's `CatchRow`: every word, size and time is the core's.
nonisolated struct CatchRow: Decodable, Equatable, Identifiable, Sendable {
    var id: String
    var species: String
    /// "28.5 in · 9.5 lb"; none when neither is known.
    var size: String?
    /// "Nov 8 2026 · 6:10 AM", in the catch's own time zone.
    var when: String
    /// The saved spot's name or the area; never coordinates.
    var place: String?
    var fate: CatchFate
    var fateLabel: String
    var hasConditions: Bool
    /// The row in one plain line, for VoiceOver.
    var summary: String
}

/// The log's catches, newest first, read from the core's `CatchList`.
nonisolated struct CatchList: Decodable, Equatable, Sendable {
    /// What the core says when there are no catches.
    struct Empty: Decodable, Equatable, Sendable {
        var title: String
        var body: String
    }

    var rows: [CatchRow]
    var empty: Empty?
    /// The empty state in the plain voice, for VoiceOver, which reads the plain meaning.
    var spokenEmpty: Empty?

    private enum CodingKeys: String, CodingKey {
        case rows, empty
    }

    /// The core's list of `book` in the user's units, clock and voice; `nil` when the log or the
    /// settings cannot be read.
    static func build(book: Data, settingsJSON: String, places: [PlaceName]) -> CatchList? {
        guard var list = decode(book: book, settingsJSON: settingsJSON, places: places)
        else { return nil }
        if list.empty != nil {
            list.spokenEmpty =
                plainVoice(settingsJSON).flatMap {
                    decode(book: book, settingsJSON: $0, places: places)?.empty
                } ?? list.empty
        }
        return list
    }

    private static func decode(book: Data, settingsJSON: String, places: [PlaceName])
        -> CatchList?
    {
        guard
            let json = try? catchListView(book: book, settingsJson: settingsJSON, spots: places)
        else { return nil }
        return try? CatchJSON.decoder.decode(CatchList.self, from: Data(json.utf8))
    }

    /// `settingsJSON` with the plain voice (`presentation.voice`), or `nil` when it has it already
    /// or cannot be read.
    static func plainVoice(_ settingsJSON: String) -> String? {
        guard
            var settings = (try? JSONSerialization.jsonObject(with: Data(settingsJSON.utf8)))
                as? [String: Any]
        else { return nil }
        var presentation = settings["presentation"] as? [String: Any] ?? [:]
        guard presentation["voice"] as? String != "plain" else { return nil }
        presentation["voice"] = "plain"
        settings["presentation"] = presentation
        guard let data = try? JSONSerialization.data(withJSONObject: settings, options: [.sortedKeys])
        else { return nil }
        return String(data: data, encoding: .utf8)
    }
}

/// One catch in full, read from the core's `CatchDetail`.
nonisolated struct CatchDetail: Decodable, Equatable, Sendable {
    /// Lure, method or notes.
    struct Line: Decodable, Equatable, Hashable, Sendable {
        var label: String
        var value: String
    }

    /// A condition at the time of the catch, with its source in `footnote`.
    struct Condition: Decodable, Equatable, Hashable, Sendable {
        var kind: String
        var label: String
        var value: String
        var footnote: String
    }

    var row: CatchRow
    var details: [Line]
    /// The names of the catch's photo files.
    var photos: [String]
    var conditionsTitle: String
    var conditions: [Condition]
    /// Set when the catch has no conditions.
    var noConditions: String?

    /// The core's detail of the catch `id`; `nil` when there is no such catch or it cannot be read.
    static func build(book: Data, id: String, settingsJSON: String, places: [PlaceName])
        -> CatchDetail?
    {
        guard
            let json = try? catchDetailView(
                book: book, id: id, settingsJson: settingsJSON, spots: places)
        else { return nil }
        return try? CatchJSON.decoder.decode(CatchDetail.self, from: Data(json.utf8))
    }
}

nonisolated enum CatchJSON {
    static let decoder: JSONDecoder = {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return decoder
    }()
}

/// The list's filters: by species, by place and by fate, each on the words the core's rows carry,
/// so the plain voice's "Kept" holds the catches kept for sashimi too. A filter left `nil` lets
/// every row through.
nonisolated struct CatchListFilter: Equatable, Sendable {
    var species: String?
    var place: String?
    /// A fate as the rows label it.
    var fate: String?

    var isActive: Bool { species != nil || place != nil || fate != nil }

    func passes(_ row: CatchRow) -> Bool {
        if let species, row.species != species { return false }
        if let place, row.place != place { return false }
        if let fate, row.fateLabel != fate { return false }
        return true
    }

    func apply(_ rows: [CatchRow]) -> [CatchRow] { rows.filter(passes) }

    /// The species of `rows` to choose from, in the order they first appear (newest first).
    static func species(in rows: [CatchRow]) -> [String] {
        unique(rows.map(\.species))
    }

    /// The places of `rows` to choose from, in the order they first appear.
    static func places(in rows: [CatchRow]) -> [String] {
        unique(rows.compactMap(\.place))
    }

    /// The fate labels of `rows` to choose from: released, kept, then kept for sashimi when the
    /// voice names it apart.
    static func fates(in rows: [CatchRow]) -> [String] {
        unique(CatchFate.allCases.flatMap { fate in
            rows.filter { $0.fate == fate }.map(\.fateLabel)
        })
    }

    /// Drops a choice no row has any longer (after a delete, or a change of voice), so a filter
    /// never hides everything for a reason the menu no longer offers.
    func pruned(for rows: [CatchRow]) -> CatchListFilter {
        var out = self
        if let species, !Self.species(in: rows).contains(species) { out.species = nil }
        if let place, !Self.places(in: rows).contains(place) { out.place = nil }
        if let fate, !Self.fates(in: rows).contains(fate) { out.fate = nil }
        return out
    }

    private static func unique(_ values: [String]) -> [String] {
        var seen = Set<String>()
        return values.filter { seen.insert($0).inserted }
    }
}
