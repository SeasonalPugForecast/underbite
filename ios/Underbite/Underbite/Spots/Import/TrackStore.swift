import Foundation
import Observation

/// A track imported from a file: its name, the file it came from and its segments, as the core
/// read them. The package tests compile this file too.
nonisolated struct SavedTrack: Codable, Equatable, Identifiable, Sendable {
    /// One point of a segment.
    struct Point: Codable, Equatable, Sendable {
        /// Degrees north.
        var latitude: Double
        /// Degrees east.
        var longitude: Double
        /// The time of the point, Unix seconds.
        var time: Int64?
    }

    var id: String
    /// The name the file gives it.
    var name: String?
    /// The name of the file it was imported from.
    var file: String
    /// When it was imported, Unix seconds.
    var importedAt: Int64
    /// Segments in file order.
    var segments: [[Point]]

    /// Whether `other` holds the same name and points: imported twice from one file.
    func sameTrack(as other: SavedTrack) -> Bool {
        name == other.name && segments == other.segments
    }
}

/// The imported tracks, in `tracks.json` in the app's directory (Application Support, so it is in
/// the device backup), written atomically like the app's other files; a file that cannot be read
/// is set aside as `tracks.json.bad` and never written over, and when it cannot be moved no track
/// is saved at all (`canSave`). Nothing of it leaves the device.
@MainActor
@Observable
final class TrackStore {
    /// The one store of the process.
    #if DEBUG
        static let shared = TrackStore(files: launchFiles(ProcessInfo.processInfo.arguments))
    #else
        static let shared = TrackStore(files: .standard)
    #endif

    /// The app's directory; in debug builds, a temporary one of its own for an offline sample
    /// run (`-sample`), so it never touches the tracks.
    nonisolated static func launchFiles(_ arguments: [String]) -> AppFiles {
        #if DEBUG
            if arguments.contains("-sample") {
                return AppFiles(
                    directory: FileManager.default.temporaryDirectory.appending(
                        path: "underbite-tracks-\(UUID().uuidString)", directoryHint: .isDirectory))
            }
        #endif
        return .standard
    }

    /// The file the tracks are kept in.
    nonisolated static let fileName = "tracks.json"
    /// The version this app writes, and the latest it reads.
    nonisolated static let currentVersion = 1

    /// The tracks in the order they were imported.
    private(set) var tracks: [SavedTrack] = []
    /// Raised whenever the tracks change, so the views that draw them follow.
    private(set) var revision = 0
    /// Whether `tracks.json` could not be read at launch.
    private(set) var unreadable = false
    /// The name the unreadable file was kept under (`tracks.json.bad`), or `nil` when it could
    /// not be moved: then it is left where it was and never saved over.
    private(set) var keptAs: String?
    /// Whether tracks can be saved: not when `tracks.json` could not be read nor moved aside, as
    /// a track imported then would be gone at the next launch.
    private(set) var canSave = true

    @ObservationIgnored let files: AppFiles

    init(files: AppFiles = .standard) {
        self.files = files
        load()
    }

    // MARK: - The file

    /// Why the file could not be read.
    nonisolated private struct Unreadable: Error {}

    /// Why a track was not saved: the file cannot be written (`canSave`).
    nonisolated struct Unsaved: Error {}

    /// The layout of `tracks.json`.
    nonisolated private struct File: Codable {
        var version: Int
        var tracks: [SavedTrack]
    }

    private func load() {
        do {
            guard let data = try files.data(Self.fileName) else { return }
            tracks = try Self.decode(data)
        } catch {
            unreadable = true
            keptAs = files.setAside(Self.fileName)
            canSave = keptAs != nil
        }
    }

    /// The tracks of a `tracks.json`: `{"tracks": [...], "version": 1}`; a later version, or
    /// anything else, cannot be read.
    nonisolated static func decode(_ data: Data) throws -> [SavedTrack] {
        let file = try JSONDecoder().decode(File.self, from: data)
        guard file.version <= currentVersion else { throw Unreadable() }
        return file.tracks
    }

    /// The text of a `tracks.json` holding `tracks`.
    nonisolated static func encode(_ tracks: [SavedTrack]) throws -> Data {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        return try encoder.encode(File(version: currentVersion, tracks: tracks))
    }

    // MARK: - Tracks

    func track(_ id: String) -> SavedTrack? { tracks.first { $0.id == id } }

    /// Saves `new` after the tracks already here, leaving out any already saved with the same
    /// name and points. Returns the ids of those saved; throws `Unsaved` when tracks cannot be
    /// saved.
    @discardableResult
    func add(_ new: [SavedTrack]) throws -> [String] {
        guard canSave else { throw Unsaved() }
        var added: [String] = []
        for track in new where !tracks.contains(where: { $0.sameTrack(as: track) }) {
            tracks.append(track)
            added.append(track.id)
        }
        if !added.isEmpty { changed() }
        return added
    }

    /// Deletes the track `id`.
    func remove(_ id: String) {
        guard canSave, tracks.contains(where: { $0.id == id }) else { return }
        tracks.removeAll { $0.id == id }
        changed()
    }

    private func changed() {
        revision += 1
        if let data = try? Self.encode(tracks) { files.write(data, Self.fileName) }
    }
}
