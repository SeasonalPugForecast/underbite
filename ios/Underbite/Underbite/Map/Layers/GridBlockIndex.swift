import Foundation

/// What the device keeps of the grid blocks it fetched: per block key (the core's, as
/// `grid_requests` names it), when the block was fetched, its size and when it was last used,
/// bounded to ``GridBlockIndex/capacity`` bytes with the least recently used dropped first.
///
/// The bodies themselves are stored beside it by ``GridBlockCache``; this is only the
/// bookkeeping, so it can be tested without files. Whether a block may still be drawn is the
/// core's rule (`grid_block_is_fresh`), never decided here.
nonisolated struct GridBlockIndex: Codable, Equatable, Sendable {
    /// One kept block.
    struct Entry: Codable, Equatable, Sendable {
        /// When the block was fetched, Unix seconds: the time the core judges it by.
        var fetchedAt: Int64
        /// The size of its body, bytes.
        var bytes: Int
        /// When it was last stored or used, by the index's own counter.
        var used: UInt64
    }

    /// The format of the stored index; an index of another version is started afresh.
    static let currentVersion = 1

    /// 50 MB: a few screens of every grid layer at their finest stride (a 1° MUR block at full
    /// resolution is about 600 KB of JSON, the same block at a wide view's stride 70 KB).
    static let capacity = 50 << 20

    private(set) var version = Self.currentVersion
    private(set) var entries: [String: Entry] = [:]
    /// The use counter: raised on every store and use.
    private(set) var clock: UInt64 = 0

    init() {}

    /// The bytes of every kept block.
    var totalBytes: Int { entries.values.reduce(0) { $0 + $1.bytes } }

    func entry(_ key: String) -> Entry? { entries[key] }

    /// Keeps the block `key` of `bytes` fetched at `fetchedAt`, replacing what was kept under the
    /// key, and drops the least recently used others until all fit in `capacity`. Returns the
    /// keys dropped, whose bodies are to be deleted: `key` itself, alone, when it is larger than
    /// `capacity` (it is not kept).
    mutating func put(
        _ key: String, bytes: Int, fetchedAt: Int64, capacity: Int = Self.capacity
    ) -> [String] {
        guard bytes <= capacity else {
            return entries.removeValue(forKey: key) != nil ? [key] : []
        }
        clock += 1
        entries[key] = Entry(fetchedAt: fetchedAt, bytes: max(0, bytes), used: clock)
        var total = totalBytes
        var dropped: [String] = []
        let oldestFirst = entries.filter { $0.key != key }.sorted {
            ($0.value.used, $0.key) < ($1.value.used, $1.key)
        }
        for (other, entry) in oldestFirst where total > capacity {
            entries[other] = nil
            total -= entry.bytes
            dropped.append(other)
        }
        return dropped
    }

    /// The block `key` was used: it is dropped after those used before it.
    mutating func touch(_ key: String) {
        guard entries[key] != nil else { return }
        clock += 1
        entries[key]?.used = clock
    }

    /// Forgets the block `key` (its body is gone or unreadable).
    mutating func remove(_ key: String) {
        entries[key] = nil
    }

    /// The index as stored.
    func json() -> String {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        return (try? encoder.encode(self)).flatMap { String(data: $0, encoding: .utf8) } ?? ""
    }

    /// The stored index, or a new one when there is none, it cannot be read or it is of another
    /// version (its bodies are then overwritten as blocks are fetched again).
    static func stored(_ json: String?) -> GridBlockIndex {
        guard let data = json?.data(using: .utf8),
            let index = try? JSONDecoder().decode(GridBlockIndex.self, from: data),
            index.version == currentVersion
        else { return GridBlockIndex() }
        return index
    }

    /// The name a block's body is stored under: the key with its `/` as `.` (keys hold letters,
    /// digits, `-` and `_` between the slashes, so the name maps back to one key), or none for a
    /// key the store could not name.
    static func name(of key: String) -> String? {
        guard !key.isEmpty, !key.contains("."), !key.hasPrefix("/") else { return nil }
        let name = key.replacingOccurrences(of: "/", with: ".")
        let allowed = name.unicodeScalars.allSatisfy {
            ("a"..."z").contains($0) || ("A"..."Z").contains($0) || ("0"..."9").contains($0)
                || $0 == "-" || $0 == "_" || $0 == "."
        }
        return allowed ? name : nil
    }
}
