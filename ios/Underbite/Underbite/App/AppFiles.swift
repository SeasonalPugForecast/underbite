import Foundation

/// The JSON files of the app in one directory: spots and areas, the settings, a cache per spot.
/// Every write is atomic, so a crash leaves the old file or the new one. The directory lasts:
/// it is in Application Support (not Caches), and the call ledger of the fetch host sits in its
/// `host` subdirectory. Like whether's, it is not excluded from backups.
nonisolated struct AppFiles: Sendable {
    let directory: URL

    init(directory: URL) {
        self.directory = directory
    }

    /// `Application Support/underbite` of the app.
    static var standard: AppFiles {
        AppFiles(
            directory: URL.applicationSupportDirectory.appending(
                path: "underbite", directoryHint: .isDirectory))
    }

    /// Where the fetch host keeps station lists, rankings and the call ledger.
    var hostDirectory: URL { directory.appending(path: "host", directoryHint: .isDirectory) }

    func url(_ name: String) -> URL {
        directory.appending(path: name, directoryHint: .notDirectory)
    }

    /// The cache file of the spot `id`; the id is percent-encoded, so it is one file name.
    func cacheName(_ id: String) -> String {
        let safe = id.addingPercentEncoding(withAllowedCharacters: .alphanumerics) ?? "spot"
        return "cache/" + safe + ".json"
    }

    func exists(_ name: String) -> Bool {
        FileManager.default.fileExists(atPath: url(name).path)
    }

    func text(_ name: String) -> String? {
        try? String(contentsOf: url(name), encoding: .utf8)
    }

    /// The bytes of `name`, or `nil` when there is no such file; throws when the file is there
    /// but cannot be read.
    func data(_ name: String) throws -> Data? {
        guard exists(name) else { return nil }
        return try Data(contentsOf: url(name))
    }

    /// The text of `name`, or `nil` when there is no such file; throws when the file is there
    /// but cannot be read as UTF-8.
    func utf8(_ name: String) throws -> String? {
        guard let data = try data(name) else { return nil }
        guard let text = String(data: data, encoding: .utf8) else {
            throw CocoaError(.fileReadInapplicableStringEncoding)
        }
        return text
    }

    /// `name` decoded, or `nil` when there is no such file; throws when the file is there but
    /// cannot be read or decoded.
    func decode<T: Decodable>(_ type: T.Type, _ name: String) throws -> T? {
        guard let data = try data(name) else { return nil }
        return try JSONDecoder().decode(type, from: data)
    }

    /// `name` decoded, or `nil` when it is missing or unreadable: for files that are only a
    /// cache of what a fetch gets again.
    func read<T: Decodable>(_ type: T.Type, _ name: String) -> T? {
        try? decode(type, name)
    }

    /// Moves `name` aside, to `name.bad` (or `name.2.bad`, `name.3.bad`, ... when that is taken),
    /// so it is never written over; returns the name it is kept under, or `nil` when it could
    /// not be moved.
    func setAside(_ name: String) -> String? {
        for attempt in 1...100 {
            let kept = attempt == 1 ? name + ".bad" : "\(name).\(attempt).bad"
            guard !exists(kept) else { continue }
            do {
                try FileManager.default.moveItem(at: url(name), to: url(kept))
                return kept
            } catch {
                return nil
            }
        }
        return nil
    }

    @discardableResult
    func write(_ data: Data, _ name: String) -> Bool {
        let file = url(name)
        do {
            try FileManager.default.createDirectory(
                at: file.deletingLastPathComponent(), withIntermediateDirectories: true)
            try data.write(to: file, options: [.atomic])
            return true
        } catch {
            return false
        }
    }

    @discardableResult
    func write<T: Encodable>(_ value: T, _ name: String) -> Bool {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        guard let data = try? encoder.encode(value) else { return false }
        return write(data, name)
    }

    func remove(_ name: String) {
        try? FileManager.default.removeItem(at: url(name))
    }
}
