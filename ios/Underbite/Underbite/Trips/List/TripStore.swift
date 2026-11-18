import Foundation
import Observation
@preconcurrency import UnderbiteKit

/// A saved trip: its id and the core's `Trip` as JSON, the payload of a `.underbite` file. The
/// app keeps no model of its own: the JSON goes to the core as it is.
nonisolated struct SavedTrip: Equatable, Identifiable, Sendable {
    var id: String
    var json: String
}

/// A trip as the list and its page show it, read from the core's trip view: every word and
/// date is the core's.
nonisolated struct TripListing: Equatable, Identifiable, Sendable {
    var id: String
    /// The trip's name.
    var title: String
    /// "Nov 8 to Nov 10"; empty when the view could not be built.
    var dates: String
    /// The first day, `YYYY-MM-DD`, for ordering only.
    var first: String
    /// "Starts in 3 days", "Under way", "This trip is over".
    var status: String
    /// The last day is over at every place.
    var isPast: Bool
    var places: [Place]
    var notes: String
    /// Why the trip cannot be planned, in the core's words (`trip_problem`).
    var problem: String?

    /// A place of the trip, the header of its column in the core's view.
    struct Place: Decodable, Equatable, Sendable {
        var key: String
        var name: String
        var area: String?
        var waterTypeLabel: String
        /// "Approximate location" for a place shared rounded or by area only.
        var approximate: String?
        /// Why the place has no days.
        var problem: String?
    }

    /// The part of the trip view the list reads.
    private struct Head: Decodable {
        var title: String
        var dates: String
        var first: String
        var status: String
        var isPast: Bool
        var places: [Place]
        var notes: String
    }

    /// The part of a trip's own JSON the list falls back on when no view can be built.
    private struct Raw: Decodable {
        var name: String?
        var first: String?
        var notes: String?
    }

    private static let decoder: JSONDecoder = {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return decoder
    }()

    /// The listing of `trip` at Unix time `now`, from the core's trip view without any fetched
    /// data (the list needs none). A trip the view cannot be built for keeps its name and
    /// says what is wrong with it.
    static func build(_ trip: SavedTrip, settingsJSON: String, now: Int64) -> TripListing {
        let problem = tripProblem(tripJson: trip.json)
        if let json = try? tripView(
            tripJson: trip.json, places: [], cachedMetadata: [],
            app: TripAppStatus(pack: .notDownloaded, sharing: .notShared),
            settingsJson: settingsJSON, now: now),
            let head = try? decoder.decode(Head.self, from: Data(json.utf8))
        {
            return TripListing(
                id: trip.id, title: head.title, dates: head.dates, first: head.first,
                status: head.status, isPast: head.isPast, places: head.places, notes: head.notes,
                problem: problem)
        }
        let raw = try? decoder.decode(Raw.self, from: Data(trip.json.utf8))
        return TripListing(
            id: trip.id, title: raw?.name ?? "", dates: "", first: raw?.first ?? "", status: "",
            isPast: false, places: [], notes: raw?.notes ?? "", problem: problem)
    }

    /// The trips not over yet, soonest first (a trip under way before those to come).
    static func upcoming(_ listings: [TripListing]) -> [TripListing] {
        listings.filter { !$0.isPast }.sorted { a, b in
            a.first != b.first ? a.first < b.first : tieBreak(a, b)
        }
    }

    /// The trips that are over, the latest first.
    static func past(_ listings: [TripListing]) -> [TripListing] {
        listings.filter(\.isPast).sorted { a, b in
            a.first != b.first ? a.first > b.first : tieBreak(a, b)
        }
    }

    private static func tieBreak(_ a: TripListing, _ b: TripListing) -> Bool {
        a.title != b.title ? a.title < b.title : a.id < b.id
    }
}

/// The saved trips, in `trips.json` in the app's directory (Application Support), written
/// atomically like the app's other files; a file that cannot be read is set aside as
/// `trips.json.bad` and never written over, and when it cannot be moved no trip is saved at all
/// (`canSave`). Ids and `modified_at` are set here; every check and every word is the core's.
@MainActor
@Observable
final class TripStore {
    /// The one store of the process.
    static let shared = TripStore()

    /// The file the trips are kept in.
    nonisolated static let fileName = "trips.json"
    /// The version this app writes, and the latest it reads.
    nonisolated static let currentVersion = 1

    /// The trips in the order they were added.
    private(set) var trips: [SavedTrip] = []
    /// What the list shows of each trip, built by `rebuild`.
    private(set) var listings: [TripListing] = []
    /// Raised whenever the trips change, so the views rebuild the listings.
    private(set) var revision = 0
    /// Whether `trips.json` could not be read at launch.
    private(set) var unreadable = false
    /// The name the unreadable file was kept under (`trips.json.bad`), or `nil` when it could
    /// not be moved: then it is left where it was and never saved over.
    private(set) var keptAs: String?
    /// Whether trips can be saved: not when `trips.json` could not be read nor moved aside, as
    /// a trip added then would be gone at the next launch.
    private(set) var canSave = true

    @ObservationIgnored private let files: AppFiles
    @ObservationIgnored private let clock: @Sendable () -> Int64
    @ObservationIgnored private let newID: @Sendable () -> String
    @ObservationIgnored private var generation = 0

    /// - Parameters:
    ///   - clock: Unix seconds, for `modified_at` and the listings' `now`.
    ///   - newID: the id of a new trip.
    init(
        files: AppFiles = .standard,
        clock: @escaping @Sendable () -> Int64 = { Int64(Date().timeIntervalSince1970) },
        newID: @escaping @Sendable () -> String = { UUID().uuidString }
    ) {
        self.files = files
        self.clock = clock
        self.newID = newID
        load()
    }

    // MARK: - Reading

    /// Why the file could not be read.
    nonisolated private struct Unreadable: Error {}

    /// Why a trip was not saved: the file cannot be written (`canSave`).
    nonisolated struct Unsaved: Error {}

    private func load() {
        do {
            guard let data = try files.data(Self.fileName) else { return }
            trips = try Self.decode(data, newID: newID)
        } catch {
            unreadable = true
            keptAs = files.setAside(Self.fileName)
            canSave = keptAs != nil
        }
    }

    /// The trips of a `trips.json`: `{"version": 1, "trips": [...]}`, each trip the core's JSON,
    /// read back in the core's own form (a trip the core cannot read stays as it is). A trip
    /// without an id gets one; a later version, or anything else, cannot be read.
    nonisolated static func decode(_ data: Data, newID: () -> String) throws -> [SavedTrip] {
        guard
            let file = try JSONSerialization.jsonObject(with: data) as? [String: Any],
            let list = file["trips"] as? [Any]
        else { throw Unreadable() }
        let version = (file["version"] as? NSNumber)?.intValue ?? 1
        guard version <= currentVersion else { throw Unreadable() }
        return try list.map { item in
            guard var trip = item as? [String: Any] else { throw Unreadable() }
            let id = (trip["id"] as? String).flatMap { $0.isEmpty ? nil : $0 } ?? newID()
            trip["id"] = id
            let read = try text(of: trip)
            return SavedTrip(id: id, json: (try? fuzzTrip(tripJson: read, fuzz: .exact)) ?? read)
        }
    }

    /// The text of a `trips.json` holding `trips`, each written as the core wrote it.
    nonisolated static func encode(_ trips: [SavedTrip]) -> Data {
        let list = trips.map(\.json).joined(separator: ",")
        return Data(#"{"trips":[\#(list)],"version":\#(currentVersion)}"#.utf8)
    }

    /// `object` as compact JSON with sorted keys.
    nonisolated static func text(of object: [String: Any]) throws -> String {
        let data = try JSONSerialization.data(
            withJSONObject: object, options: [.sortedKeys, .withoutEscapingSlashes])
        guard let text = String(data: data, encoding: .utf8) else { throw Unreadable() }
        return text
    }

    // MARK: - Trips

    func trip(_ id: String) -> SavedTrip? { trips.first { $0.id == id } }

    func listing(_ id: String) -> TripListing? { listings.first { $0.id == id } }

    var upcoming: [TripListing] { TripListing.upcoming(listings) }

    var past: [TripListing] { TripListing.past(listings) }

    /// Saves the trip `json` (the core's `Trip`): a trip without an id is a new one and gets one,
    /// a trip with an id replaces the one saved under it (or is added). `modified_at` is now.
    /// Returns the trip's id; throws when the core cannot read it as a trip, or `Unsaved`
    /// when trips cannot be saved.
    @discardableResult
    func save(_ json: String) throws -> String {
        guard canSave else { throw Unsaved() }
        guard var trip = try JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any]
        else { throw Unreadable() }
        let id = (trip["id"] as? String).flatMap { $0.isEmpty ? nil : $0 } ?? newID()
        trip["id"] = id
        trip["modified_at"] = clock()
        // Through the core, so what is kept is a trip it reads, in its own form.
        let saved = try fuzzTrip(tripJson: try Self.text(of: trip), fuzz: .exact)
        let entry = SavedTrip(id: id, json: saved)
        if let index = trips.firstIndex(where: { $0.id == id }) {
            trips[index] = entry
        } else {
            trips.append(entry)
        }
        changed()
        return id
    }

    /// Deletes the trip `id` and what its page kept (`TripData`).
    func remove(_ id: String) {
        guard trips.contains(where: { $0.id == id }) else { return }
        trips.removeAll { $0.id == id }
        listings.removeAll { $0.id == id }
        changed()
        files.remove(TripData.fileName(id))
    }

    private func changed() {
        revision += 1
        saveFile()
    }

    // MARK: - The listings

    /// Builds every trip's listing with the core's trip view at the clock's now, off the main
    /// actor, and publishes them unless a newer build has started.
    func rebuild(settingsJSON: String) async {
        generation += 1
        let mine = generation
        let trips = self.trips
        let now = clock()
        let built = await Task.detached(priority: .userInitiated) {
            trips.map { TripListing.build($0, settingsJSON: settingsJSON, now: now) }
        }.value
        guard mine == generation else { return }
        listings = built
    }

    // MARK: - Saving

    private func saveFile() {
        guard canSave else { return }
        files.write(Self.encode(trips), Self.fileName)
    }
}
