import Foundation

/// Why a payload was not kept.
public enum WidgetStoreError: Error, Equatable, Sendable {
    /// The JSON is not a widget payload (a snapshot, say).
    case notAPayload(String)
    /// A spot's payload where the next trip's belongs, or the other way round.
    case wrongSubject(expected: String, found: String)
    /// The payload has no id to name its file by.
    case noID
    /// The JSON carries a place's coordinates, which never go to the widgets.
    case carriesCoordinates(key: String)
}

/// The payloads the app writes for its widgets and the widgets read, in the shared container:
/// one JSON file per spot, one for the next trip, and the list of saved spots (ids and names)
/// for the widgets' configuration.
///
/// Only the core's widget payloads are kept, as the core wrote them: never a snapshot, never a
/// place's coordinates. Each file is written whole (atomically), so a widget reading while the
/// app writes sees the old payload or the new one. A widget reads only the file it shows.
public final class WidgetPayloadStore: Sendable {
    /// The keys that would carry a place's position: a payload with any of them is refused.
    static let coordinateKeys: Set<String> = ["latitude", "longitude", "lat", "lon", "lng"]
    /// The characters a spot id keeps in its file name; the others are percent-encoded.
    private static let fileNameCharacters = CharacterSet(
        charactersIn: "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_")

    public let container: SharedContainer

    public init(container: SharedContainer = .shared) {
        self.container = container
    }

    /// The app's and the widgets' store, in the shared container (or its fallback).
    public static let shared = WidgetPayloadStore()

    // MARK: Files

    private var spotsFolder: URL {
        container.url.appending(path: "spots", directoryHint: .isDirectory)
    }

    private var nextTripURL: URL { container.url.appending(path: "next-trip.json") }

    private var spotListURL: URL { container.url.appending(path: "spot-list.json") }

    /// The file of the spot `id`.
    func spotURL(id: String) -> URL {
        let name = id.addingPercentEncoding(withAllowedCharacters: Self.fileNameCharacters) ?? id
        return spotsFolder.appending(path: "\(name).json")
    }

    // MARK: Writing (the app)

    /// Keeps a spot's payload (`spotWidget(...)`'s JSON) in its spot's file, replacing the
    /// one before. Returns it decoded.
    @discardableResult
    public func saveSpot(_ json: String) throws -> ViewModel.WidgetPayload {
        let payload = try Self.checked(json, subject: "spot")
        try write(Data(json.utf8), to: spotURL(id: payload.id))
        return payload
    }

    /// Keeps the next trip's payload (`tripWidget(...)`'s JSON), replacing the one before.
    /// Returns it decoded.
    @discardableResult
    public func saveNextTrip(_ json: String) throws -> ViewModel.WidgetPayload {
        let payload = try Self.checked(json, subject: "trip")
        try write(Data(json.utf8), to: nextTripURL)
        return payload
    }

    /// Forgets the next trip's payload (every trip is over, or none is left).
    public func removeNextTrip() throws {
        try remove(nextTripURL)
    }

    /// Forgets the spot `id`'s payload.
    public func removeSpot(id: String) throws {
        try remove(spotURL(id: id))
    }

    /// Forgets the payloads of every spot but `ids` (the saved ones).
    public func keepSpots(_ ids: some Collection<String>) throws {
        let kept = Set(ids.map { spotURL(id: $0).lastPathComponent })
        for file in files(in: spotsFolder) where !kept.contains(file.lastPathComponent) {
            try remove(file)
        }
    }

    /// Keeps the saved spots (ids and names, in the app's order) and the one the app shows,
    /// for the widgets' configuration.
    public func saveSpots(_ spots: WidgetSpots) throws {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        try write(encoder.encode(spots), to: spotListURL)
    }

    // MARK: Reading (the widgets)

    /// The spot `id`'s payload; `nil` when there is none or it does not read.
    public func spot(id: String) -> ViewModel.WidgetPayload? {
        read(spotURL(id: id))
    }

    /// The next trip's payload; `nil` when there is none or it does not read.
    public func nextTrip() -> ViewModel.WidgetPayload? {
        read(nextTripURL)
    }

    /// The ids of the spots with a payload, sorted.
    public func spotIDs() -> [String] {
        files(in: spotsFolder)
            .filter { $0.pathExtension == "json" }
            .compactMap { $0.deletingPathExtension().lastPathComponent.removingPercentEncoding }
            .sorted()
    }

    /// The saved spots as the app last told; none before it has.
    public func spots() -> WidgetSpots {
        guard let data = try? Data(contentsOf: spotListURL),
            let spots = try? JSONDecoder().decode(WidgetSpots.self, from: data)
        else { return WidgetSpots() }
        return spots
    }

    // MARK: Checks

    /// The payload in `json`, when it is a widget payload about `subject` with an id and no
    /// coordinates anywhere in it.
    static func checked(_ json: String, subject: String) throws -> ViewModel.WidgetPayload {
        let payload: ViewModel.WidgetPayload
        do {
            payload = try ViewModel.decodeWidget(json)
        } catch {
            throw WidgetStoreError.notAPayload(String(describing: error))
        }
        guard payload.subject == subject else {
            throw WidgetStoreError.wrongSubject(expected: subject, found: payload.subject)
        }
        guard !payload.id.isEmpty else { throw WidgetStoreError.noID }
        let object = try JSONSerialization.jsonObject(with: Data(json.utf8))
        if let key = coordinateKey(in: object) {
            throw WidgetStoreError.carriesCoordinates(key: key)
        }
        return payload
    }

    /// The first key in `object`, at any depth, that would carry a position.
    static func coordinateKey(in object: Any) -> String? {
        if let dictionary = object as? [String: Any] {
            for (key, value) in dictionary {
                if coordinateKeys.contains(key.lowercased()) { return key }
                if let found = coordinateKey(in: value) { return found }
            }
        } else if let array = object as? [Any] {
            for value in array {
                if let found = coordinateKey(in: value) { return found }
            }
        }
        return nil
    }

    // MARK: Files

    private func write(_ data: Data, to url: URL) throws {
        try FileManager.default.createDirectory(
            at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        try data.write(to: url, options: .atomic)
    }

    private func read(_ url: URL) -> ViewModel.WidgetPayload? {
        guard let data = try? Data(contentsOf: url) else { return nil }
        return try? ViewModel.decodeWidget(data)
    }

    private func remove(_ url: URL) throws {
        do {
            try FileManager.default.removeItem(at: url)
        } catch CocoaError.fileNoSuchFile {
            // Already gone.
        }
    }

    private func files(in folder: URL) -> [URL] {
        (try? FileManager.default.contentsOfDirectory(
            at: folder, includingPropertiesForKeys: nil, options: [.skipsHiddenFiles])) ?? []
    }
}
