import CoreTransferable
import Foundation
import UniformTypeIdentifiers
@preconcurrency import UnderbiteKit

extension UTType {
    /// A trip or a spot sent between people: the core's JSON in a `.underbite` file. Declared in
    /// `Underbite-Info.plist` (exported, and opened by the app).
    nonisolated static let underbiteFile = UTType(exportedAs: "dev.savery.underbite.file")
}

/// A `.underbite` file to send: its name and the text the core wrote. Plain values, not isolated
/// to the main actor, so the system can ask for the file from any thread. The package tests
/// compile this file too.
nonisolated struct UnderbiteFile: Transferable, Equatable, Sendable {
    /// "Hudson stripers.underbite".
    let fileName: String
    /// The file's text, as the core wrote it.
    let data: Data

    /// The file's extension.
    static let pathExtension = "underbite"
    /// The start of the name of each sent file's folder.
    static let folderPrefix = "underbite-sent-"

    static var transferRepresentation: some TransferRepresentation {
        FileRepresentation(exportedContentType: .underbiteFile) { file in
            SentTransferredFile(try file.write())
        }
    }

    /// Writes the file into a folder of its own under `directory`, so two files with one name
    /// never meet, and returns its URL. The file is left there, as the share sheet may read it
    /// after it is dismissed; `sweep` removes it when the next share starts or at the next
    /// launch, since it may hold a place's exact position.
    func write(in directory: URL = .temporaryDirectory) throws -> URL {
        let folder = directory.appending(
            path: Self.folderPrefix + UUID().uuidString, directoryHint: .isDirectory)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        let url = folder.appending(path: fileName, directoryHint: .notDirectory)
        try data.write(to: url, options: .atomic)
        return url
    }

    /// Removes the folders of files sent before from `directory`, and with `exports` those of
    /// catch log exports left behind too (`CatchStore.exportFile`): when a share starts, and at
    /// launch with the exports.
    static func sweep(in directory: URL = .temporaryDirectory, exports: Bool = false) {
        let manager = FileManager.default
        guard let names = try? manager.contentsOfDirectory(atPath: directory.path) else { return }
        for name in names {
            let sent = name.hasPrefix(folderPrefix)
            let exported = exports && name.hasPrefix(CatchStore.exportPrefix)
            guard sent || exported else { continue }
            try? manager.removeItem(at: directory.appending(path: name, directoryHint: .isDirectory))
        }
    }

    /// The file of the trip `tripJSON` (the core's `Trip`), its places reduced to `fuzz` by the
    /// core, named after the trip.
    static func trip(_ tripJSON: String, fuzz: ShareFuzz) throws -> UnderbiteFile {
        let text = try tripFile(tripJson: tripJSON, fuzz: fuzz)
        return UnderbiteFile(fileName: fileName(for: name(of: tripJSON)), data: Data(text.utf8))
    }

    /// The `name` of a trip's JSON; empty when it has none.
    static func name(of json: String) -> String {
        let object = try? JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any]
        return (object?["name"] as? String) ?? ""
    }

    /// `name` as a file name with the `.underbite` extension: characters a file name cannot
    /// hold become `-`, leading dots (a hidden file) and spaces go, and it is cut to 80
    /// characters. An empty name is the app's.
    static func fileName(for name: String) -> String {
        let forbidden = CharacterSet(charactersIn: "/\\:?%*|\"<>")
            .union(.newlines).union(.controlCharacters)
        var base = name.unicodeScalars
            .map { forbidden.contains($0) ? "-" : String($0) }
            .joined()
            .trimmingCharacters(in: .whitespaces)
        while base.hasPrefix(".") || base.hasPrefix(" ") { base.removeFirst() }
        base = String(base.prefix(80)).trimmingCharacters(in: .whitespaces)
        if base.isEmpty { base = "Underbite" }
        return base + "." + pathExtension
    }
}

/// The core's words of sharing and receiving files.
nonisolated enum ShareText {
    /// The words in the settings' voice (the default settings' when those cannot be read).
    static func words(settingsJSON: String) -> ShareWords? {
        (try? shareWords(settingsJson: settingsJSON))
            ?? (try? shareWords(settingsJson: defaultSettingsJson()))
    }

    /// The core's text of a share error.
    static func message(of error: any Error) -> String {
        switch error {
        case ShareError.NeedsNewerVersion(let message), ShareError.DatesBackwards(let message),
            ShareError.TooLong(let message), ShareError.Unreadable(let message),
            ShareError.TooLarge(let message):
            message
        case ShareError.Unsupported(_, let message): message
        case ShareError.Invalid(_, let message): message
        // Not the core's error: what it says of a file it cannot read.
        default: unreadableMessage
        }
    }

    private static var unreadableMessage: String {
        do {
            _ = try readFile(bytes: Data(), settingsJson: defaultSettingsJson())
            return ""
        } catch let error as ShareError {
            return message(of: error)
        } catch {
            return ""
        }
    }
}
