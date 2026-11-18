import Foundation
@preconcurrency import UnderbiteKit

/// The catch editor's fields as the user fills them in: time, place, species, size, fate, lure,
/// method, notes and photos. Sizes stay as typed (the core reads them in the user's units); the
/// draft turns into the core's `CatchInput` and knows when the catch needs its conditions again.
nonisolated struct CatchDraft: Equatable, Sendable {
    /// Where the catch was.
    enum Place: Hashable, Sendable {
        /// At a saved spot.
        case spot(String)
        /// Where the catch was logged before this edit, at no saved spot.
        case stored
        /// Where the phone is.
        case here
    }

    /// A position, degrees.
    struct Position: Equatable, Sendable {
        var latitude: Double
        var longitude: Double
    }

    /// The catch's id; empty for a new catch, which the store gives one.
    var id = ""
    var caughtAt: Date
    var place: Place?
    /// The fix of the phone, once read, for `.here`.
    var here: Position?
    /// A place name the user typed, for a catch at no saved spot.
    var area = ""
    /// A catalogue species id.
    var species: String?
    /// A species the user typed, for one not in the catalogue.
    var speciesText = ""
    var lengthText = ""
    var weightText = ""
    var ouncesText = ""
    var fate: FateId = .released
    var lure = ""
    var method = ""
    var notes = ""
    /// Photo file names in the app's container, in order.
    var photos: [String] = []
    /// The catch as saved, when this is an edit.
    private(set) var original: CatchInput?

    /// A new catch at `now`, at `spot` when there is one, else where the phone is.
    init(now: Date, spot: Spot?) {
        caughtAt = now
        place = spot.map { .spot($0.id) } ?? .here
    }

    /// The editor's fields of a saved catch. A catch at a spot that is no longer saved keeps its
    /// position.
    init(editing input: CatchInput, spots: [Spot]) {
        id = input.id
        caughtAt = Date(timeIntervalSince1970: TimeInterval(input.caughtAt))
        if let spotID = input.spotId, spots.contains(where: { $0.id == spotID }) {
            place = .spot(spotID)
        } else {
            place = .stored
        }
        area = input.area ?? ""
        species = input.species
        speciesText = input.speciesText ?? ""
        lengthText = input.lengthText
        weightText = input.weightText
        ouncesText = input.ouncesText
        fate = input.fate
        lure = input.lure ?? ""
        method = input.method ?? ""
        notes = input.notes ?? ""
        photos = input.photos
        original = input
    }

    var isNew: Bool { original == nil }

    /// The catch time in whole Unix seconds.
    var caughtAtSeconds: Int64 { Int64(caughtAt.timeIntervalSince1970.rounded(.down)) }

    /// The saved spot the catch is at, if any.
    func spot(in spots: [Spot]) -> Spot? {
        guard case .spot(let id) = place else { return nil }
        return spots.first { $0.id == id }
    }

    /// Where the catch is, or `nil` while the phone's position is not known.
    func position(spots: [Spot]) -> Position? {
        switch place {
        case .spot:
            spot(in: spots).map { Position(latitude: $0.latitude, longitude: $0.longitude) }
        case .stored:
            original.map { Position(latitude: $0.latitude, longitude: $0.longitude) }
        case .here:
            here
        case nil:
            nil
        }
    }

    /// The IANA zone the catch time is read in: the spot's, else the one the catch was saved
    /// with, else the phone's.
    func timeZone(spots: [Spot], phone: TimeZone = .current) -> String {
        if let zone = spot(in: spots)?.timeZone, !zone.isEmpty { return zone }
        if place == .stored, let zone = original?.timeZone, !zone.isEmpty { return zone }
        return phone.identifier
    }

    /// The core's input for these fields; `nil` while the place is not known.
    func input(spots: [Spot], phone: TimeZone = .current) -> CatchInput? {
        guard let position = position(spots: spots) else { return nil }
        let atSpot = spot(in: spots)
        return CatchInput(
            id: id, caughtAt: caughtAtSeconds, timeZone: timeZone(spots: spots, phone: phone),
            latitude: position.latitude, longitude: position.longitude,
            spotId: atSpot?.id, area: atSpot == nil ? Self.text(area) : nil,
            species: species, speciesText: species == nil ? Self.text(speciesText) : nil,
            lengthText: lengthText, weightText: weightText, ouncesText: ouncesText,
            fate: fate, lure: Self.text(lure), method: Self.text(method), notes: Self.text(notes),
            photos: photos)
    }

    /// Whether the catch takes the conditions of the snapshot when it is saved: a new catch
    /// always, an edit only when its time or position changed, so an edit of the notes (or a
    /// catch whose spot was since removed) never swaps the conditions of the moment for later
    /// ones.
    func needsConditions(spots: [Spot]) -> Bool {
        guard let original else { return true }
        guard let input = input(spots: spots) else { return false }
        return input.caughtAt != original.caughtAt || input.latitude != original.latitude
            || input.longitude != original.longitude
    }

    /// The saved spot whose snapshot holds the catch's conditions: the catch's spot, else the
    /// nearest one within `within` metres of where the catch is.
    func snapshotSpot(spots: [Spot], within: Double = 1_000) -> Spot? {
        if let spot = spot(in: spots) { return spot }
        guard let position = position(spots: spots) else { return nil }
        return
            spots
            .map { ($0, Self.distance(position, Position(latitude: $0.latitude, longitude: $0.longitude))) }
            .filter { $0.1 <= within }
            .min { $0.1 < $1.1 }?.0
    }

    // MARK: - Fate

    /// The fate the picker shows among `options`: a catch kept for sashimi shows as kept where
    /// the voice does not offer sashimi.
    func shownFate(_ options: [FateId]) -> FateId {
        if options.contains(fate) { return fate }
        return fate == .keptForSashimi ? .kept : fate
    }

    /// Picks `chosen` in the picker; choosing kept keeps a catch kept for sashimi as it was.
    mutating func pick(_ chosen: FateId) {
        if chosen == .kept, fate == .keptForSashimi { return }
        fate = chosen
    }

    // MARK: - Helpers

    private static func text(_ value: String) -> String? {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? nil : trimmed
    }

    /// The great-circle distance in metres.
    static func distance(_ a: Position, _ b: Position) -> Double {
        let radius = 6_371_000.0
        let rad = Double.pi / 180
        let dLat = (b.latitude - a.latitude) * rad
        let dLon = (b.longitude - a.longitude) * rad
        let h =
            sin(dLat / 2) * sin(dLat / 2)
            + cos(a.latitude * rad) * cos(b.latitude * rad) * sin(dLon / 2) * sin(dLon / 2)
        return 2 * radius * asin(min(1, sqrt(h)))
    }
}

/// The conditions a catch will be saved with, as the core's detail words them: worked out on a
/// copy of the log, so the editor shows exactly what saving keeps.
nonisolated struct CatchConditionsPreview: Equatable, Sendable {
    /// "Conditions".
    var title: String
    /// Each with its source.
    var conditions: [CatchDetail.Condition]
    /// The core's words when there are none.
    var none: String?

    /// The id a new catch is worked out under.
    static let draftID = "draft"

    /// The conditions of `input` saved into `book`, with those of `snapshotJSON` attached when
    /// `attach` is set (an edit that keeps its time and place keeps the conditions it has).
    /// `nil` when the core cannot read the log or the settings.
    static func build(
        book: Data, input: CatchInput, snapshotJSON: String?, attach: Bool, settingsJSON: String,
        now: Int64
    ) -> CatchConditionsPreview? {
        var input = input
        if input.id.isEmpty { input.id = draftID }
        // The size and the photos do not change the conditions; blank, a size typed wrong does
        // not hide them.
        input.lengthText = ""
        input.weightText = ""
        input.ouncesText = ""
        input.photos = []
        guard
            var scratch = try? logSave(book: book, input: input, settingsJson: settingsJSON, now: now)
                .book
        else { return nil }
        if attach, let snapshotJSON,
            let attached = try? logAttachConditions(
                book: scratch, id: input.id, snapshotJson: snapshotJSON, at: nil, now: now)
        {
            scratch = attached.book
        }
        guard
            let detail = CatchDetail.build(
                book: scratch, id: input.id, settingsJSON: settingsJSON, places: [])
        else { return nil }
        return CatchConditionsPreview(
            title: detail.conditionsTitle, conditions: detail.conditions, none: detail.noConditions)
    }
}

/// A species of the catalogue to choose for a catch, as the core's species list names it.
nonisolated struct SpeciesChoice: Decodable, Equatable, Hashable, Identifiable, Sendable {
    var id: String
    /// "Striped bass".
    var name: String
    /// "Morone saxatilis".
    var scientificName: String

    /// The catalogue by name in the user's voice; empty when the settings cannot be read.
    static func catalogue(settingsJSON: String) -> [SpeciesChoice] {
        guard let json = try? speciesList(edits: "", settingsJson: settingsJSON),
            let rows = try? CatchJSON.decoder.decode([SpeciesChoice].self, from: Data(json.utf8))
        else { return [] }
        return rows
    }

    /// The species of `choices` whose common or scientific name holds `search`, all of them for
    /// a blank search.
    static func matching(_ search: String, in choices: [SpeciesChoice]) -> [SpeciesChoice] {
        let search = search.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !search.isEmpty else { return choices }
        return choices.filter {
            $0.name.localizedCaseInsensitiveContains(search)
                || $0.scientificName.localizedCaseInsensitiveContains(search)
        }
    }
}
