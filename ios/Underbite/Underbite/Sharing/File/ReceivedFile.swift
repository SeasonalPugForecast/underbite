import Foundation
@preconcurrency import UnderbiteKit

/// What can be done with a received file.
nonisolated enum ReceiveChoice: Equatable, Sendable {
    /// Add the trip or spot as it is.
    case add
    /// Add it beside the one with its id already here, under a new id.
    case addCopy
    /// Replace the trip with its id already here, through the core's merge, so a place never
    /// gets more precise or less precise than this device holds it.
    case replace
}

/// A `.underbite` file someone sent, as its preview shows it: the core decodes it and words it.
/// The package tests compile this file too.
nonisolated struct ReceivedFile: Identifiable, Equatable, Sendable {
    /// What the file holds.
    enum Content: Equatable, Sendable {
        /// A trip: the core's JSON and its listing, read from the core's trip view.
        case trip(json: String, listing: TripListing)
        /// A spot: the place and, when it has coordinates, the spot to add.
        case spot(ReceivedPlace)
        /// A file that cannot be used, with the core's text saying why.
        case problem(String)
    }

    let id = UUID()
    /// "Trip to add", "Spot to add"; none for a file that cannot be used.
    var heading: String?
    var content: Content
    /// "Approximate location" when a place was shared rounded or by area only.
    var approximate: String?

    static func == (a: ReceivedFile, b: ReceivedFile) -> Bool {
        a.heading == b.heading && a.content == b.content && a.approximate == b.approximate
    }

    /// The largest file the core reads (`share::encode`'s limit); one byte more is read, so the
    /// core says the file is too large without the whole of it being loaded.
    static let maxBytes = 1_000_000

    // MARK: - Reading

    /// Reads the file at `url` (from Files, Mail or AirDrop) and decodes it. A URL another app
    /// lends is security-scoped: access is asked for while reading and given back after. The
    /// copy the system puts in the app's `Documents/Inbox` is removed once read.
    static func open(
        _ url: URL, settingsJSON: String, now: Int64, inbox: URL? = defaultInbox
    ) -> ReceivedFile {
        let data: Data
        do {
            data = try read(url)
        } catch {
            data = Data()
        }
        if let inbox, url.standardizedFileURL.path.hasPrefix(inbox.standardizedFileURL.path + "/") {
            try? FileManager.default.removeItem(at: url)
        }
        return read(data, settingsJSON: settingsJSON, now: now)
    }

    /// Where the system copies a document it opens in the app.
    static var defaultInbox: URL? {
        FileManager.default.urls(for: .documentDirectory, in: .userDomainMask).first?
            .appending(path: "Inbox", directoryHint: .isDirectory)
    }

    /// The first `limit` bytes of the file at `url` (`maxBytes + 1` unless another limit is
    /// given).
    static func read(_ url: URL, limit: Int = maxBytes + 1) throws -> Data {
        let scoped = url.startAccessingSecurityScopedResource()
        defer { if scoped { url.stopAccessingSecurityScopedResource() } }
        let handle = try FileHandle(forReadingFrom: url)
        defer { try? handle.close() }
        return try handle.read(upToCount: limit) ?? Data()
    }

    /// Decodes `data` with the core: a trip or a spot to preview, or the core's text on why it
    /// cannot be used (a newer version's file, a kind this build cannot open yet).
    static func read(_ data: Data, settingsJSON: String, now: Int64) -> ReceivedFile {
        let words = ShareText.words(settingsJSON: settingsJSON)
        let shared: SharedFile
        do {
            shared = try readFile(bytes: data, settingsJson: settingsJSON)
        } catch {
            return ReceivedFile(heading: nil, content: .problem(message(of: error)))
        }
        switch shared.kind {
        case .trip:
            guard let json = shared.tripJson else {
                return ReceivedFile(heading: nil, content: .problem(message(of: Unusable())))
            }
            let id = tripID(json) ?? ""
            let listing = TripListing.build(
                SavedTrip(id: id, json: json), settingsJSON: settingsJSON, now: now)
            return ReceivedFile(
                heading: words?.receiveTrip, content: .trip(json: json, listing: listing),
                approximate: shared.approximate)
        case .spot:
            let place = ReceivedPlace(
                placeJSON: shared.placeJson ?? "{}", spot: shared.spot, notes: shared.notes ?? "",
                settingsJSON: settingsJSON)
            return ReceivedFile(
                heading: words?.receiveSpot, content: .spot(place), approximate: shared.approximate)
        }
    }

    private static func message(of error: any Error) -> String { ShareText.message(of: error) }

    /// A file the core read but that holds nothing to add.
    private struct Unusable: Error {}

    /// The `id` of a trip's JSON.
    static func tripID(_ json: String) -> String? {
        let object = try? JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any]
        return (object?["id"] as? String).flatMap { $0.isEmpty ? nil : $0 }
    }

    // MARK: - Choices

    /// What the preview offers, given the trips and spots saved here: a new trip or spot is
    /// added; a trip already here is added as a copy or replaces this device's; a spot already
    /// here is added as a copy (never replaced, so a rounded copy never moves an exact spot);
    /// a spot shared by area only has nothing to add.
    func choices(trips: [SavedTrip], spots: [Spot]) -> [ReceiveChoice] {
        switch content {
        case .trip(let json, _):
            let id = Self.tripID(json)
            return trips.contains { $0.id == id } ? [.addCopy, .replace] : [.add]
        case .spot(let place):
            guard let spot = place.spot else { return [] }
            return spots.contains { $0.id == spot.id } ? [.addCopy] : [.add]
        case .problem:
            return []
        }
    }

    /// Whether the trip or spot is already on this device.
    func exists(trips: [SavedTrip], spots: [Spot]) -> Bool {
        choices(trips: trips, spots: spots).contains(.addCopy)
    }

    /// The trip JSON to save for `choice`: as received, without its id (the store gives the
    /// copy one), or the core's merge of the received trip over `mine`.
    static func tripToSave(_ received: String, choice: ReceiveChoice, mine: String?) throws
        -> String
    {
        switch choice {
        case .add:
            return received
        case .addCopy:
            guard
                var object = try JSONSerialization.jsonObject(with: Data(received.utf8))
                    as? [String: Any]
            else { throw Unusable() }
            object.removeValue(forKey: "id")
            return try TripStore.text(of: object)
        case .replace:
            guard let mine else { return received }
            // This device's trip is the base: every field the sender changed is taken, and a
            // place keeps the precision it has here.
            return try mergeTrips(baseJson: mine, mineJson: mine, theirsJson: received).tripJson
        }
    }

    /// The spot to add for `choice`: the received one, under a new id for a copy; none for a
    /// trip or a spot shared by area only.
    func spotToAdd(_ choice: ReceiveChoice, newID: () -> String = { UUID().uuidString }) -> Spot? {
        guard case .spot(let place) = content, var spot = place.spot else { return nil }
        if choice == .addCopy { spot.id = newID() }
        return spot
    }

    /// Adds the trip or spot for `choice`: a trip through `trips`, a spot through `spots`
    /// (whose stations are then looked up and data fetched). Returns the id it was saved
    /// under.
    @MainActor
    @discardableResult
    func accept(
        _ choice: ReceiveChoice, trips: TripStore, spots: SpotStore,
        newID: () -> String = { UUID().uuidString }
    ) throws -> String? {
        guard case .spot = content else { return try acceptTrip(choice, trips: trips) }
        guard let spot = spotToAdd(choice, newID: newID) else { return nil }
        spots.add(spot)
        Task { await spots.refresh(only: [spot.id]) }
        return spot.id
    }

    /// Saves the trip for `choice` in `trips` and returns its id; none for a spot.
    @MainActor
    @discardableResult
    func acceptTrip(_ choice: ReceiveChoice, trips: TripStore) throws -> String? {
        guard case .trip(let json, _) = content else { return nil }
        let mine = Self.tripID(json).flatMap { trips.trip($0)?.json }
        return try trips.save(Self.tripToSave(json, choice: choice, mine: mine))
    }
}

/// A received spot as its preview shows it.
nonisolated struct ReceivedPlace: Equatable, Sendable {
    var name: String
    /// The group the sender keeps it under ("Lower Hudson").
    var area: String?
    /// The kind of water, in the settings' voice.
    var waterTypeLabel: String
    /// What the sender wrote about it.
    var notes: String
    /// The spot to add; none when shared by area only.
    var spot: Spot?

    /// The part of the core's `TripPlace` JSON the preview reads.
    private struct Head: Decodable {
        var name: String?
        var area: String?
        var waterType: String?
    }

    init(name: String, area: String?, waterTypeLabel: String, notes: String, spot: Spot?) {
        self.name = name
        self.area = area
        self.waterTypeLabel = waterTypeLabel
        self.notes = notes
        self.spot = spot
    }

    init(placeJSON: String, spot info: SpotInfo?, notes: String, settingsJSON: String) {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        let head = try? decoder.decode(Head.self, from: Data(placeJSON.utf8))
        let waterType = head?.waterType ?? info?.waterType ?? ""
        let options = (try? waterTypeOptions(settingsJson: settingsJSON)) ?? []
        self.init(
            name: head?.name ?? info?.name ?? "",
            area: head?.area,
            waterTypeLabel: options.first { $0.id == waterType }?.label ?? "",
            notes: notes,
            spot: info.map { info in
                Spot(
                    id: info.id, name: info.name, latitude: info.latitude,
                    longitude: info.longitude, waterType: info.waterType, timeZone: info.timeZone)
            })
    }
}

#if DEBUG
    extension ReceivedFile {
        /// The file of screenshots and tests (`-receiveFile sample`): a three-day trip at the
        /// test places, from a week after `today`, shared at about 1 km.
        static func sample(today: String, settingsJSON: String, now: Int64) -> ReceivedFile {
            let spots = [SeedSpots.battery, SeedSpots.croton]
            var draft = TripDraft(
                first: TripDates.adding(7, to: today) ?? today, days: 3, places: spots.map(\.id))
            draft.id = "5EED0000-0000-4000-8000-000000000002"
            draft.name = "Harbor and Croton"
            draft.species = "morone-saxatilis"
            draft.method = "shore"
            draft.notes = "The Battery on the morning flood, then the Croton mouth at dusk."
            let json = draft.json(spots: spots, areas: [], settingsJSON: settingsJSON)
            guard let file = try? UnderbiteFile.trip(json, fuzz: .rounded1Km) else {
                return ReceivedFile(heading: nil, content: .problem(""))
            }
            return read(file.data, settingsJSON: settingsJSON, now: now)
        }
    }
#endif
