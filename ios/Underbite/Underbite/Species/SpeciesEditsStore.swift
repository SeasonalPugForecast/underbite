import Foundation
import Observation
@preconcurrency import UnderbiteKit

/// The user's edits of species profiles, in `species-edits.json` in the app's directory
/// (Application Support), written atomically like the app's other files. The file is the core's
/// own text of the edits: every change goes through the core (`species_edit`, `species_reset`,
/// `species_import`), which returns the new text, and every species call is passed it. A file
/// that cannot be read is set aside as `species-edits.json.bad` and never written over; when it
/// cannot be moved, no edit is saved at all (`canSave`). The package tests compile this file too.
@MainActor
@Observable
final class SpeciesEditsStore {
    /// The one store of the process.
    #if DEBUG
        static let shared = SpeciesEditsStore(files: launchFiles(ProcessInfo.processInfo.arguments))
    #else
        static let shared = SpeciesEditsStore(files: .standard)
    #endif

    /// The app's directory; in debug builds, a temporary one of its own for an offline sample
    /// run (`-sample`), so it never touches the edits.
    nonisolated static func launchFiles(_ arguments: [String]) -> AppFiles {
        #if DEBUG
            if arguments.contains("-sample") {
                return AppFiles(
                    directory: FileManager.default.temporaryDirectory.appending(
                        path: "underbite-species-\(UUID().uuidString)", directoryHint: .isDirectory))
            }
        #endif
        return .standard
    }

    /// The file the edits are kept in.
    nonisolated static let fileName = "species-edits.json"

    /// The edits as the core wrote them; empty for none.
    private(set) var edits = ""
    /// Raised whenever the edits change, so the views read them again.
    private(set) var revision = 0
    /// Whether `species-edits.json` could not be read at launch.
    private(set) var unreadable = false
    /// The name the unreadable file was kept under, or `nil` when it could not be moved: then
    /// it is left where it was and never saved over.
    private(set) var keptAs: String?
    /// Whether edits can be saved: not when the file could not be read nor moved aside.
    private(set) var canSave = true

    @ObservationIgnored private let files: AppFiles

    init(files: AppFiles = .standard) {
        self.files = files
        load()
    }

    /// Why an edit was not saved: the file cannot be written (`canSave`).
    nonisolated struct Unsaved: Error {}

    /// Why the file could not be read.
    nonisolated private struct Unreadable: Error {}

    private func load() {
        do {
            guard let text = try files.utf8(Self.fileName) else { return }
            try Self.check(text)
            edits = text
        } catch {
            unreadable = true
            keptAs = files.setAside(Self.fileName)
            canSave = keptAs != nil
        }
    }

    /// Throws unless the core reads `text` as edits (blank text is none).
    nonisolated static func check(_ text: String) throws {
        do {
            _ = try speciesList(edits: text, settingsJson: defaultSettingsJson())
        } catch SpeciesError.Edits {
            throw Unreadable()
        }
    }

    // MARK: - Changes

    /// Sets one field of the species `id` from the editor's `values` (`species_edit`). Throws
    /// the core's `SpeciesError.Field` with the words to show when a value is refused, and
    /// `Unsaved` when edits cannot be saved; nothing changes then.
    func edit(id: String, field: String, values: [String], settingsJSON: String) throws {
        guard canSave else { throw Unsaved() }
        replace(
            try speciesEdit(
                edits: edits, id: id, field: field, values: values, settingsJson: settingsJSON))
    }

    /// Resets one field of `id` to the catalogue's value, or every field when `field` is nil.
    func reset(id: String, field: String?) throws {
        guard canSave else { throw Unsaved() }
        replace(try speciesReset(edits: edits, id: id, field: field))
    }

    /// Imports a shared profile file: the fields the user has not edited take the sender's
    /// values; fields both edited keep the user's unless `takeTheirs`. Returns what was done.
    @discardableResult
    func importProfile(_ bytes: Data, takeTheirs: Bool, settingsJSON: String) throws
        -> ProfileImportPlan
    {
        guard canSave else { throw Unsaved() }
        let imported = try speciesImport(
            bytes: bytes, edits: edits, takeTheirs: takeTheirs, settingsJson: settingsJSON)
        replace(imported.edits)
        return imported.plan
    }

    private func replace(_ text: String) {
        guard text != edits else { return }
        edits = text
        revision += 1
        files.write(Data(text.utf8), Self.fileName)
    }
}
