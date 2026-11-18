import Foundation

/// The tiles and blobs kept on the device for the map and for offline use, in one SQLite file
/// laid out like MBTiles: `tiles` by layer, date and tile; `blobs` by the core's key (grid
/// blocks, elevation areas, station metadata responses); `pack_items`, the rows each offline pack
/// pins; and `meta`, the schema version and the byte count. Keys are the core's (see
/// ``OfflineKey``), so the map, the packs and the fetch fallback find each other's copies.
///
/// **Bound.** Bodies count against ``limitBytes`` (`map.offline_max_mb`). Past it, unpinned rows
/// go by least recent use until the store is back under 95% of it; pinned rows never go, so when
/// packs alone fill the bound nothing unpinned is kept.
///
/// **Freshness.** A copy is fresh until the lifetime it was stored with ends. Stale copies are
/// kept as the fallback: offline, or after a failed fetch, a read allowing them returns them with
/// the time they were fetched.
///
/// **Where.** The file is in `Application Support/offline` (``standardDirectory``), a directory
/// excluded from backup. Caches would let the system purge pack data the user downloaded to use
/// without signal, so pinned rows have to live in Application Support. Unpinned rows stay in the
/// same file rather than a second one in Caches: pinning and unpinning then flip rows in one
/// transaction instead of copying bodies between files, and the store bounds itself. The data is
/// downloaded again on a restored device, so it stays out of backups.
///
/// **Concurrency.** The actor owns the one connection; nothing else opens the file. The file is
/// in write-ahead-log mode so a write does not block a read for long, and `synchronous=NORMAL`
/// (safe in WAL mode: a crash may lose the last writes, never corrupt the file).
actor OfflineStore {
    /// The schema this store reads and writes; a file of another version is not opened.
    nonisolated static let schemaVersion: Int64 = 1

    /// The database's file name in its directory.
    nonisolated static let fileName = "offline.sqlite"

    /// `Application Support/offline` of the app.
    nonisolated static var standardDirectory: URL {
        URL.applicationSupportDirectory.appending(path: "offline", directoryHint: .isDirectory)
    }

    /// The most candidates read from each table per round of eviction.
    nonisolated private static let evictionBatch = 256

    let directory: URL
    private(set) var limitBytes: Int64
    private var db: OfflineSQLite?
    /// The bytes of every stored body, as `meta` records them.
    private var bytes: Int64 = 0
    /// The last use stamp given.
    private var lastStamp: Int64 = 0

    /// A store in `directory` bounded to `limitBytes`; the file is opened on first use.
    init(
        directory: URL = OfflineStore.standardDirectory,
        limitBytes: Int64 = OfflineRules.limitBytes(
            megabytes: OfflineRules.defaultLimitMegabytes)
    ) {
        self.directory = directory
        self.limitBytes = max(0, limitBytes)
    }

    var fileURL: URL { directory.appending(path: Self.fileName, directoryHint: .notDirectory) }

    // MARK: Opening

    /// Opens the file now (it is opened on first use otherwise): creates it with the current
    /// schema, or checks the version of the one there.
    func open() throws {
        _ = try connection()
    }

    /// Closes the file and deletes it with its log; the next use starts an empty store. For a
    /// file that is not this store's (``OfflineStoreError/schema(found:expected:)``) or cannot
    /// be read.
    func reset() throws {
        db = nil
        bytes = 0
        lastStamp = 0
        for suffix in ["", "-wal", "-shm"] {
            let url = directory.appending(
                path: Self.fileName + suffix, directoryHint: .notDirectory)
            if FileManager.default.fileExists(atPath: url.path(percentEncoded: false)) {
                try FileManager.default.removeItem(at: url)
            }
        }
    }

    private func connection() throws -> OfflineSQLite {
        if let db { return db }
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        // Set on every open: some file operations clear it.
        var excluded = URLResourceValues()
        excluded.isExcludedFromBackup = true
        var folder = directory
        try folder.setResourceValues(excluded)

        let db = try OfflineSQLite(path: fileURL.path(percentEncoded: false))
        let tables = Set(
            try db.rows("SELECT name FROM sqlite_master WHERE type = 'table'") { $0.text(0) }
                .compactMap { $0 })
        let isNew = tables.isEmpty
        if !isNew {
            var found: Int64 = 0
            if tables.contains("meta") {
                found = try db.integer("SELECT value FROM meta WHERE name = 'schema_version'") ?? 0
            }
            guard found == Self.schemaVersion else {
                throw OfflineStoreError.schema(found: found, expected: Self.schemaVersion)
            }
        }
        let mode = try db.rows("PRAGMA journal_mode = WAL") { $0.text(0) }.first ?? nil
        guard mode?.lowercased() == "wal" else {
            throw OfflineStoreError.open("the file cannot be put in write-ahead-log mode")
        }
        try db.run("PRAGMA synchronous = NORMAL")
        if isNew {
            try db.transaction {
                for statement in Self.schema { try db.run(statement) }
                try db.run(
                    "INSERT INTO meta (name, value) VALUES ('schema_version', ?), ('bytes', 0)",
                    [.int(Self.schemaVersion)])
            }
        }
        bytes = try db.integer("SELECT value FROM meta WHERE name = 'bytes'") ?? 0
        lastStamp = try max(
            db.integer("SELECT MAX(last_used_at) FROM tiles") ?? 0,
            db.integer("SELECT MAX(last_used_at) FROM blobs") ?? 0)
        self.db = db
        return db
    }

    /// Version 1. Times are Unix seconds, except `last_used_at`, a use stamp in milliseconds
    /// (``OfflineRules/stamp(now:after:)``). A pin names a tile by ``OfflineTileKey/text``,
    /// which ``tilePinned`` rebuilds from the tile's columns.
    nonisolated private static let schema = [
        "CREATE TABLE meta (name TEXT PRIMARY KEY NOT NULL, value) WITHOUT ROWID",
        """
        CREATE TABLE tiles (
            layer TEXT NOT NULL, date TEXT NOT NULL,
            z INTEGER NOT NULL, x INTEGER NOT NULL, y INTEGER NOT NULL,
            data BLOB NOT NULL, content_type TEXT, etag TEXT,
            fetched_at INTEGER NOT NULL, expires_at INTEGER NOT NULL,
            last_used_at INTEGER NOT NULL,
            PRIMARY KEY (layer, date, z, x, y))
        """,
        "CREATE INDEX tiles_last_used ON tiles (last_used_at)",
        """
        CREATE TABLE blobs (
            key TEXT PRIMARY KEY NOT NULL, data BLOB NOT NULL,
            fetched_at INTEGER NOT NULL, expires_at INTEGER NOT NULL,
            last_used_at INTEGER NOT NULL)
        """,
        "CREATE INDEX blobs_last_used ON blobs (last_used_at)",
        """
        CREATE TABLE pack_items (
            pack_id TEXT NOT NULL, kind TEXT NOT NULL, key TEXT NOT NULL,
            PRIMARY KEY (pack_id, kind, key)) WITHOUT ROWID
        """,
        "CREATE INDEX pack_items_key ON pack_items (kind, key)",
    ]

    /// Whether a pack pins the `tiles` row `t`.
    nonisolated private static let tilePinned = """
        EXISTS (SELECT 1 FROM pack_items AS p WHERE p.kind = 'tile'
            AND p.key = t.layer || '/' || t.date || '/' || t.z || '/' || t.x || '/' || t.y)
        """

    /// Whether a pack pins the `blobs` row `b`.
    nonisolated private static let blobPinned = """
        EXISTS (SELECT 1 FROM pack_items AS p WHERE p.kind = 'blob' AND p.key = b.key)
        """

    // MARK: Reading

    /// The copy kept for `key`: while fresh at `now`, or at any age when `allowingStale` (offline,
    /// or after a failed fetch). Reading it marks it used.
    func get(_ key: OfflineKey, now: Int64, allowingStale: Bool = false) throws -> OfflineRecord? {
        let db = try connection()
        let found: [(Int64, OfflineRecord)]
        switch key {
        case .tile(let tile):
            found = try db.rows(
                """
                SELECT rowid, data, content_type, etag, fetched_at, expires_at FROM tiles
                WHERE layer = ? AND date = ? AND z = ? AND x = ? AND y = ?
                """, Self.values(tile)
            ) { row in
                (row.int(0), Self.record(row, from: 1, contentType: row.text(2), now: now))
            }
        case .blob(let name):
            found = try db.rows(
                "SELECT rowid, data, NULL, NULL, fetched_at, expires_at FROM blobs WHERE key = ?",
                [.text(name)]
            ) { row in (row.int(0), Self.record(row, from: 1, contentType: nil, now: now)) }
        }
        guard let (rowid, record) = found.first, allowingStale || !record.isStale else {
            return nil
        }
        try markUsed(key.kind, rowid: rowid, now: now)
        return record
    }

    /// When the copy kept for `key` was fetched and until when it is fresh, without reading it
    /// or marking it used.
    func times(of key: OfflineKey) throws -> (fetchedAt: Int64, expiresAt: Int64)? {
        let db = try connection()
        switch key {
        case .tile(let tile):
            return try db.rows(
                """
                SELECT fetched_at, expires_at FROM tiles
                WHERE layer = ? AND date = ? AND z = ? AND x = ? AND y = ?
                """, Self.values(tile)
            ) { ($0.int(0), $0.int(1)) }.first
        case .blob(let name):
            return try db.rows(
                "SELECT fetched_at, expires_at FROM blobs WHERE key = ?", [.text(name)]
            ) { ($0.int(0), $0.int(1)) }.first
        }
    }

    /// Marks the copy kept for `key` used at `now` (it is dropped after those used before it);
    /// false when there is none.
    @discardableResult
    func touch(_ key: OfflineKey, now: Int64) throws -> Bool {
        guard let rowid = try rowid(of: key) else { return false }
        try markUsed(key.kind, rowid: rowid, now: now)
        return true
    }

    // MARK: Writing

    /// Keeps `entry` for `key`, replacing the copy there, then drops unpinned rows if the store
    /// passed its bound. An entry ``OfflineRules/admits(_:as:)`` refuses is not kept. Returns
    /// whether it was written.
    @discardableResult
    func put(_ key: OfflineKey, _ entry: OfflineEntry, now: Int64) throws -> Bool {
        try put([(key, entry)], now: now) == 1
    }

    /// Keeps every entry of `items` in one transaction, as ``put(_:_:now:)`` keeps one; returns
    /// how many were written. A row written may still be dropped at once when the store has no
    /// room for unpinned rows.
    @discardableResult
    func put(_ items: [(key: OfflineKey, entry: OfflineEntry)], now: Int64) throws -> Int {
        let db = try connection()
        let admitted = items.filter { OfflineRules.admits($0.entry, as: $0.key.kind) }
        guard !admitted.isEmpty else { return 0 }
        var total = bytes
        var stamp = lastStamp
        try db.transaction {
            for (key, entry) in admitted {
                stamp = OfflineRules.stamp(now: now, after: stamp)
                total -= try storedBytes(key, db) ?? 0
                try write(key, entry, stamp: stamp, db)
                total += Int64(entry.data.count)
            }
            try evict(&total, target: nil, db)
            try saveBytes(total, db)
        }
        bytes = total
        lastStamp = stamp
        return admitted.count
    }

    /// Gives the copy kept for `key` a new fetch time and lifetime, as when the server answered
    /// that it has not changed; false when there is none.
    @discardableResult
    func renew(_ key: OfflineKey, fetchedAt: Int64, expiresAt: Int64) throws -> Bool {
        guard let rowid = try rowid(of: key) else { return false }
        try connection().run(
            "UPDATE \(Self.table(key.kind)) SET fetched_at = ?, expires_at = ? WHERE rowid = ?",
            [.int(fetchedAt), .int(expiresAt), .int(rowid)])
        return true
    }

    private func write(_ key: OfflineKey, _ entry: OfflineEntry, stamp: Int64, _ db: OfflineSQLite)
        throws
    {
        let times: [OfflineSQLValue] = [.int(entry.fetchedAt), .int(entry.expiresAt), .int(stamp)]
        switch key {
        case .tile(let tile):
            let type: OfflineSQLValue = entry.contentType.map { .text($0) } ?? .null
            let etag: OfflineSQLValue = entry.etag.map { .text($0) } ?? .null
            try db.run(
                """
                INSERT INTO tiles (layer, date, z, x, y, data, content_type, etag,
                    fetched_at, expires_at, last_used_at)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                ON CONFLICT (layer, date, z, x, y) DO UPDATE SET data = excluded.data,
                    content_type = excluded.content_type, etag = excluded.etag,
                    fetched_at = excluded.fetched_at, expires_at = excluded.expires_at,
                    last_used_at = excluded.last_used_at
                """, Self.values(tile) + [.blob(entry.data), type, etag] + times)
        case .blob(let name):
            try db.run(
                """
                INSERT INTO blobs (key, data, fetched_at, expires_at, last_used_at)
                VALUES (?, ?, ?, ?, ?)
                ON CONFLICT (key) DO UPDATE SET data = excluded.data,
                    fetched_at = excluded.fetched_at, expires_at = excluded.expires_at,
                    last_used_at = excluded.last_used_at
                """, [.text(name), .blob(entry.data)] + times)
        }
    }

    // MARK: Packs

    /// Pins the rows of `keys` to `pack`: no eviction drops them while a pack pins them. Keys
    /// not stored yet are pinned too (a pack is pinned before it is downloaded).
    func pin(_ pack: String, _ keys: [OfflineKey]) throws {
        guard !pack.isEmpty, !keys.isEmpty else { return }
        let db = try connection()
        try db.transaction {
            for key in keys {
                try db.run(
                    "INSERT OR IGNORE INTO pack_items (pack_id, kind, key) VALUES (?, ?, ?)",
                    [.text(pack), .text(key.kind.rawValue), .text(key.text)])
            }
        }
    }

    /// Unpins everything `pack` pins and deletes the rows no other pack pins; returns the bytes
    /// freed.
    @discardableResult
    func unpin(_ pack: String) throws -> Int64 {
        let db = try connection()
        var total = bytes
        try db.transaction {
            let alone = try db.rows(
                """
                SELECT kind, key FROM pack_items AS p WHERE pack_id = ?
                AND NOT EXISTS (SELECT 1 FROM pack_items AS q
                    WHERE q.kind = p.kind AND q.key = p.key AND q.pack_id <> p.pack_id)
                """, [.text(pack)]
            ) { row in
                row.text(0).flatMap(OfflineKind.init(rawValue:)).flatMap { kind in
                    row.text(1).flatMap { OfflineKey(kind: kind, text: $0) }
                }
            }
            for case let key? in alone {
                guard let size = try storedBytes(key, db) else { continue }
                try delete(key, db)
                total -= size
            }
            try db.run("DELETE FROM pack_items WHERE pack_id = ?", [.text(pack)])
            try saveBytes(total, db)
        }
        let freed = bytes - total
        bytes = total
        return freed
    }

    /// The packs that pin rows, by id.
    func packs() throws -> [String] {
        try connection().rows("SELECT DISTINCT pack_id FROM pack_items ORDER BY pack_id") {
            $0.text(0)
        }.compactMap { $0 }
    }

    /// What `pack` pins and how much of it is stored.
    func usage(of pack: String) throws -> OfflinePackUsage {
        let db = try connection()
        let keys = try db.rows(
            "SELECT kind, key FROM pack_items WHERE pack_id = ?", [.text(pack)]
        ) { row in
            row.text(0).flatMap(OfflineKind.init(rawValue:)).flatMap { kind in
                row.text(1).flatMap { OfflineKey(kind: kind, text: $0) }
            }
        }
        var stored = 0
        var size: Int64 = 0
        for case let key? in keys {
            guard let one = try storedBytes(key, db) else { continue }
            stored += 1
            size += one
        }
        return OfflinePackUsage(items: keys.count, stored: stored, bytes: size)
    }

    // MARK: Size

    /// What the store holds.
    func usage() throws -> OfflineUsage {
        let db = try connection()
        let pinned =
            try db.integer(
                """
                SELECT (SELECT COALESCE(SUM(length(t.data)), 0) FROM tiles AS t
                    WHERE \(Self.tilePinned))
                + (SELECT COALESCE(SUM(length(b.data)), 0) FROM blobs AS b
                    WHERE \(Self.blobPinned))
                """) ?? 0
        let tiles = try db.integer("SELECT COUNT(*) FROM tiles") ?? 0
        let blobs = try db.integer("SELECT COUNT(*) FROM blobs") ?? 0
        return OfflineUsage(
            bytes: bytes, pinnedBytes: pinned, tiles: Int(tiles), blobs: Int(blobs),
            limitBytes: limitBytes)
    }

    /// Bounds the store to `limitBytes` (`map.offline_max_mb` changed), dropping unpinned rows
    /// at once if it is past it.
    func setLimit(bytes limit: Int64) throws {
        limitBytes = max(0, limit)
        try evictNow(target: nil)
    }

    /// Drops every unpinned row; returns the bytes freed.
    @discardableResult
    func removeUnpinned() throws -> Int64 {
        let before = bytes
        try evictNow(target: 0)
        return before - bytes
    }

    /// The bytes of every body, counted from the rows rather than kept (tests).
    func measuredBytes() throws -> Int64 {
        try connection().integer(
            """
            SELECT (SELECT COALESCE(SUM(length(data)), 0) FROM tiles)
            + (SELECT COALESCE(SUM(length(data)), 0) FROM blobs)
            """) ?? 0
    }

    /// The connection's journal mode (tests).
    func journalMode() throws -> String? {
        try connection().rows("PRAGMA journal_mode") { $0.text(0) }.first ?? nil
    }

    // MARK: Eviction

    private func evictNow(target: Int64?) throws {
        let db = try connection()
        var total = bytes
        try db.transaction {
            try evict(&total, target: target, db)
            try saveBytes(total, db)
        }
        bytes = total
    }

    /// Drops unpinned rows, least recently used first, until `total` is down to `target`; with
    /// no target, only when `total` is past the bound, and then down to its low water.
    private func evict(_ total: inout Int64, target: Int64?, _ db: OfflineSQLite) throws {
        let target = target ?? (total > limitBytes ? OfflineRules.lowWater(limitBytes) : total)
        while total > target {
            let batch = Self.evictionBatch
            let ordered = OfflineRules.oldestFirst(
                tiles: try candidates(.tile, batch, db), blobs: try candidates(.blob, batch, db),
                batch: batch)
            let victims = OfflineRules.victims(ordered, total: total, target: target)
            if victims.isEmpty { return }
            for victim in victims {
                try db.run(
                    "DELETE FROM \(Self.table(victim.kind)) WHERE rowid = ?", [.int(victim.rowid)])
                total -= victim.bytes
            }
        }
    }

    private func candidates(_ kind: OfflineKind, _ limit: Int, _ db: OfflineSQLite) throws
        -> [OfflineRules.Candidate]
    {
        let sql =
            switch kind {
            case .tile:
                """
                SELECT rowid, length(data), last_used_at FROM tiles AS t
                WHERE NOT \(Self.tilePinned) ORDER BY last_used_at LIMIT ?
                """
            case .blob:
                """
                SELECT rowid, length(data), last_used_at FROM blobs AS b
                WHERE NOT \(Self.blobPinned) ORDER BY last_used_at LIMIT ?
                """
            }
        return try db.rows(sql, [.int(Int64(limit))]) {
            OfflineRules.Candidate(kind: kind, rowid: $0.int(0), bytes: $0.int(1), used: $0.int(2))
        }
    }

    // MARK: Rows

    private func rowid(of key: OfflineKey) throws -> Int64? {
        let db = try connection()
        switch key {
        case .tile(let tile):
            return try db.integer(
                """
                SELECT rowid FROM tiles WHERE layer = ? AND date = ? AND z = ? AND x = ? AND y = ?
                """, Self.values(tile))
        case .blob(let name):
            return try db.integer("SELECT rowid FROM blobs WHERE key = ?", [.text(name)])
        }
    }

    private func storedBytes(_ key: OfflineKey, _ db: OfflineSQLite) throws -> Int64? {
        switch key {
        case .tile(let tile):
            return try db.integer(
                """
                SELECT length(data) FROM tiles
                WHERE layer = ? AND date = ? AND z = ? AND x = ? AND y = ?
                """, Self.values(tile))
        case .blob(let name):
            return try db.integer("SELECT length(data) FROM blobs WHERE key = ?", [.text(name)])
        }
    }

    private func delete(_ key: OfflineKey, _ db: OfflineSQLite) throws {
        switch key {
        case .tile(let tile):
            try db.run(
                "DELETE FROM tiles WHERE layer = ? AND date = ? AND z = ? AND x = ? AND y = ?",
                Self.values(tile))
        case .blob(let name):
            try db.run("DELETE FROM blobs WHERE key = ?", [.text(name)])
        }
    }

    private func markUsed(_ kind: OfflineKind, rowid: Int64, now: Int64) throws {
        let stamp = OfflineRules.stamp(now: now, after: lastStamp)
        try connection().run(
            "UPDATE \(Self.table(kind)) SET last_used_at = ? WHERE rowid = ?",
            [.int(stamp), .int(rowid)])
        lastStamp = stamp
    }

    private func saveBytes(_ total: Int64, _ db: OfflineSQLite) throws {
        try db.run("UPDATE meta SET value = ? WHERE name = 'bytes'", [.int(total)])
    }

    nonisolated private static func table(_ kind: OfflineKind) -> String {
        switch kind {
        case .tile: "tiles"
        case .blob: "blobs"
        }
    }

    nonisolated private static func values(_ tile: OfflineTileKey) -> [OfflineSQLValue] {
        [
            .text(tile.layer), .text(tile.dateColumn), .int(Int64(tile.z)), .int(Int64(tile.x)),
            .int(Int64(tile.y)),
        ]
    }

    /// The record in columns `first` (data), +2 (etag), +3 (fetched at), +4 (expires at).
    nonisolated private static func record(
        _ row: OfflineSQLite.Row, from first: Int32, contentType: String?, now: Int64
    ) -> OfflineRecord {
        let expiresAt = row.int(first + 4)
        return OfflineRecord(
            data: row.blob(first), contentType: contentType, etag: row.text(first + 2),
            fetchedAt: row.int(first + 3), expiresAt: expiresAt,
            isStale: !OfflineRules.isFresh(expiresAt: expiresAt, now: now))
    }
}
