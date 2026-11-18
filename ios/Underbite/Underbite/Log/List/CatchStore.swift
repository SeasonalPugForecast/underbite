import Foundation
import Observation
@preconcurrency import UnderbiteKit

/// The catch log, in `catches.json` in the app's directory (Application Support, so it is in
/// the device backup), written atomically like the app's other files; a file that cannot be read
/// is set aside as `catches.json.bad` and never written over. The file is the core's own text of
/// the log: every change goes through the core, which returns the new bytes. Ids and the time of
/// each change are set here; every check and every word is the core's.
///
/// Photos of catches are files in `photos/` beside it, named by the editor; deleting a catch
/// deletes its photos.
@MainActor
@Observable
final class CatchStore {
    /// The one store of the process.
    #if DEBUG
        static let shared = CatchStore(files: launchFiles(ProcessInfo.processInfo.arguments))
    #else
        static let shared = CatchStore(files: .standard)
    #endif

    /// The app's directory; in debug builds, a temporary one of its own for an offline sample run
    /// (`-sample`) or the empty log of a screenshot (`-emptyLog`), so neither touches the log.
    nonisolated static func launchFiles(_ arguments: [String]) -> AppFiles {
        #if DEBUG
            if arguments.contains("-sample") || arguments.contains("-emptyLog") {
                return AppFiles(
                    directory: FileManager.default.temporaryDirectory.appending(
                        path: "underbite-log-\(UUID().uuidString)", directoryHint: .isDirectory))
            }
        #endif
        return .standard
    }

    /// The file the log is kept in.
    nonisolated static let fileName = "catches.json"
    /// The folder the photos of catches are kept in.
    nonisolated static let photosFolder = "photos"

    /// The log as the core wrote it.
    private(set) var book: Data
    /// Catches in the log, not counting deleted ones.
    private(set) var count = 0
    /// What the list shows, built by `rebuild`.
    private(set) var list: CatchList?
    /// Raised whenever the log changes, so the views rebuild.
    private(set) var revision = 0
    /// Whether `catches.json` could not be read at launch.
    private(set) var unreadable = false
    /// The name the unreadable file was kept under (`catches.json.bad`), or `nil` when it could
    /// not be moved: then it is left where it was and never saved over.
    private(set) var keptAs: String?
    /// Why the last change was refused, in the core's words.
    private(set) var problem: String?

    @ObservationIgnored let files: AppFiles
    @ObservationIgnored private let clock: @Sendable () -> Int64
    @ObservationIgnored private let newID: @Sendable () -> String
    @ObservationIgnored private var writable = true
    @ObservationIgnored private var generation = 0

    /// - Parameters:
    ///   - clock: Unix seconds, the time of each change.
    ///   - newID: the id of a new catch.
    init(
        files: AppFiles = .standard,
        clock: @escaping @Sendable () -> Int64 = { Int64(Date().timeIntervalSince1970) },
        newID: @escaping @Sendable () -> String = { UUID().uuidString }
    ) {
        self.files = files
        self.clock = clock
        self.newID = newID
        book = logNew()
        load()
    }

    // MARK: - Reading

    private func load() {
        do {
            guard let data = try files.data(Self.fileName) else { return }
            let loaded = try logLoad(book: data)
            book = loaded.book
            count = Int(loaded.counts.catches)
        } catch {
            unreadable = true
            keptAs = files.setAside(Self.fileName)
            writable = keptAs != nil
        }
    }

    // MARK: - Changes

    /// Saves the editor's fields: a catch with an empty id is a new one and gets one, any other
    /// replaces the catch of that id (keeping its conditions). Returns the catch's id; throws the
    /// core's `LogError` when a field cannot be read.
    @discardableResult
    func save(_ input: CatchInput, settingsJSON: String) throws -> String {
        var input = input
        if input.id.isEmpty { input.id = newID() }
        let change = try logSave(book: book, input: input, settingsJson: settingsJSON, now: clock())
        apply(change.book, changed: change.changed)
        return input.id
    }

    /// Attaches the conditions at `at` (the catch time when `nil`) from a spot's snapshot to the
    /// catch `id`. Returns whether any were near enough in time to attach.
    @discardableResult
    func attachConditions(to id: String, snapshotJSON: String, at: Int64? = nil) throws -> Bool {
        let change = try logAttachConditions(
            book: book, id: id, snapshotJson: snapshotJSON, at: at, now: clock())
        apply(change.book, changed: change.changed)
        return change.changed
    }

    /// Deletes the catch `id` and its photos. The core keeps a record that it was deleted, with
    /// no position, notes or photos, so a later sync carries the deletion.
    func delete(_ id: String) {
        do {
            let deleted = try logDelete(book: book, id: id, now: clock())
            guard deleted.deleted else { return }
            for name in deleted.photos { removePhoto(name) }
            apply(deleted.book, changed: true)
        } catch {
            problem = Self.message(error)
        }
    }

    private func apply(_ bytes: Data, changed: Bool) {
        problem = nil
        guard changed else { return }
        book = bytes
        count = (try? logLoad(book: bytes).counts.catches).map(Int.init) ?? count
        revision += 1
        if writable { files.write(bytes, Self.fileName) }
    }

    /// The words of a core error: its message, else a plain description.
    nonisolated static func message(_ error: any Error) -> String {
        switch error as? LogError {
        case .Book(let message), .Field(_, let message), .Invalid(let message):
            message
        case .NotFound(let id):
            "There is no catch with the id \(id)."
        case nil:
            error.localizedDescription
        }
    }

    // MARK: - Photos

    /// Where the photo `name` of a catch is kept.
    func photoURL(_ name: String) -> URL {
        files.url(Self.photosFolder + "/" + name)
    }

    /// Keeps a picked photo for a catch in `photos/`: scaled down, upright and without the
    /// picture's metadata (`CatchPhotos`), worked out off the main actor. Returns its file name.
    func addPhoto(_ data: Data) async throws -> String {
        let files = self.files
        let name = CatchPhotos.newName()
        try await Task.detached(priority: .userInitiated) {
            let jpeg = try CatchPhotos.stripped(data)
            guard files.write(jpeg, Self.photosFolder + "/" + name) else {
                throw CatchPhotos.Failure.unwritable
            }
        }.value
        return name
    }

    /// Removes photo files no catch keeps: those an edit dropped, or those an editor added and
    /// was then cancelled.
    func removePhotos(_ names: [String]) {
        for name in names { removePhoto(name) }
    }

    private func removePhoto(_ name: String) {
        // A name is one file in the folder, never a path out of it.
        guard !name.isEmpty, !name.contains("/"), !name.contains("\\"), name != ".", name != ".."
        else { return }
        files.remove(Self.photosFolder + "/" + name)
    }

    // MARK: - Views

    /// Builds the list with the core's view off the main actor, and publishes it unless a newer
    /// build has started.
    func rebuild(settingsJSON: String, places: [PlaceName]) async {
        generation += 1
        let mine = generation
        let book = self.book
        let built = await Task.detached(priority: .userInitiated) {
            CatchList.build(book: book, settingsJSON: settingsJSON, places: places)
        }.value
        guard mine == generation else { return }
        list = built
    }

    /// The catch `id` in full, from the core's view; `nil` when there is no such catch.
    func detail(_ id: String, settingsJSON: String, places: [PlaceName]) -> CatchDetail? {
        CatchDetail.build(book: book, id: id, settingsJSON: settingsJSON, places: places)
    }

    // MARK: - Export

    /// The files the log can be exported as.
    enum ExportFormat: String, CaseIterable, Sendable {
        case csv, gpx
    }

    /// Writes the log as `format` to a new temporary folder and returns the file, for the share
    /// sheet. The caller removes it with `discardExport` once the sheet is done.
    func exportFile(_ format: ExportFormat) throws -> URL {
        let text =
            switch format {
            case .csv: try logExportCsv(book: book)
            case .gpx: try logExportGpx(book: book)
            }
        let folder = FileManager.default.temporaryDirectory.appending(
            path: "underbite-export-\(UUID().uuidString)", directoryHint: .isDirectory)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        let file = folder.appending(path: "catches.\(format.rawValue)", directoryHint: .notDirectory)
        do {
            try Data(text.utf8).write(to: file, options: [.atomic])
        } catch {
            try? FileManager.default.removeItem(at: folder)
            throw error
        }
        return file
    }

    /// Removes an export `exportFile` made, with its folder.
    nonisolated static func discardExport(_ file: URL) {
        let folder = file.deletingLastPathComponent()
        guard folder.lastPathComponent.hasPrefix("underbite-export-") else {
            try? FileManager.default.removeItem(at: file)
            return
        }
        try? FileManager.default.removeItem(at: folder)
    }
}
