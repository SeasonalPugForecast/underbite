import Foundation
import XCTest

/// The offline store on a real SQLite file in a directory of its own per test.
final class OfflineStoreTests: XCTestCase {
    /// A directory of its own for each test (XCTest makes an instance per test).
    private let directory = FileManager.default.temporaryDirectory
        .appending(path: "underbite-offline-\(UUID().uuidString)", directoryHint: .isDirectory)

    override func tearDown() {
        try? FileManager.default.removeItem(at: directory)
    }

    /// 2026-11-15T12:00:00Z.
    private let now: Int64 = 1_794_744_000

    private func store(limit: Int64 = 1 << 30) -> OfflineStore {
        OfflineStore(directory: directory, limitBytes: limit)
    }

    private func tile(_ x: UInt32, date: String? = nil) -> OfflineKey {
        // INVARIANT: the layer and dates the tests use are valid key parts.
        .tile(OfflineTileKey(layer: "noaa-chart", date: date, z: 12, x: x, y: 1_539)!)
    }

    /// An image body of `size` bytes, different for each `seed`.
    private func png(
        _ size: Int, seed: UInt8 = 0, lifetime: UInt64 = 86_400, at time: Int64? = nil
    ) -> OfflineEntry {
        var bytes = [UInt8](repeating: seed, count: size)
        bytes[0] = 0x89
        return OfflineEntry(
            data: Data(bytes), contentType: "image/png", etag: "\"e\(seed)\"",
            fetchedAt: time ?? now, lifetime: lifetime)
    }

    private func json(_ text: String, lifetime: UInt64 = 3_600) -> OfflineEntry {
        OfflineEntry(
            data: Data(text.utf8), contentType: "application/json", fetchedAt: now,
            lifetime: lifetime)
    }

    // MARK: Round trip and freshness

    func testATileAndABlobComeBackAsStored() async throws {
        let s = store()
        let entry = png(300, seed: 7)
        let kept = try await s.put(tile(1), entry, now: now)
        XCTAssertTrue(kept)
        let record = try await s.get(tile(1), now: now + 10)
        XCTAssertEqual(record?.data, entry.data)
        XCTAssertEqual(record?.contentType, "image/png")
        XCTAssertEqual(record?.etag, "\"e7\"")
        XCTAssertEqual(record?.fetchedAt, now)
        XCTAssertEqual(record?.expiresAt, now + 86_400)
        XCTAssertEqual(record?.isStale, false)

        let grid = OfflineKey.blob("sst/jplMURSST41/last/40000000/-74000000/1")
        try await s.put(grid, json(#"{"table":{"rows":[]}}"#), now: now)
        let block = try await s.get(grid, now: now)
        XCTAssertEqual(
            block.map { String(decoding: $0.data, as: UTF8.self) }, #"{"table":{"rows":[]}}"#)
        XCTAssertNil(block?.contentType)

        // Another day of the same tile, and another tile, are other rows.
        let missingDay = try await s.get(tile(1, date: "2026-11-14"), now: now)
        XCTAssertNil(missingDay)
        let missingTile = try await s.get(tile(2), now: now)
        XCTAssertNil(missingTile)
    }

    func testAPutReplacesTheCopyThere() async throws {
        let s = store()
        try await s.put(tile(1), png(100, seed: 1), now: now)
        try await s.put(tile(1), png(250, seed: 2, at: now + 60), now: now + 60)
        let record = try await s.get(tile(1), now: now + 60)
        XCTAssertEqual(record?.data.count, 250)
        XCTAssertEqual(record?.fetchedAt, now + 60)
        let usage = try await s.usage()
        XCTAssertEqual(usage.bytes, 250)
        XCTAssertEqual(usage.tiles, 1)
    }

    func testAnExpiredCopyIsReadOnlyWhenStaleCopiesAreAllowed() async throws {
        let s = store()
        try await s.put(tile(1), png(100, lifetime: 3_600), now: now)
        let fresh = try await s.get(tile(1), now: now + 3_599)
        XCTAssertEqual(fresh?.isStale, false)
        let online = try await s.get(tile(1), now: now + 3_600)
        XCTAssertNil(online, "online, an expired copy is fetched again")
        let offline = try await s.get(tile(1), now: now + 90_000, allowingStale: true)
        XCTAssertEqual(offline?.isStale, true)
        XCTAssertEqual(offline?.fetchedAt, now, "a stale copy says when it was fetched")
        let times = try await s.times(of: tile(1))
        XCTAssertEqual(times?.fetchedAt, now)
        XCTAssertEqual(times?.expiresAt, now + 3_600)
    }

    func testRenewingGivesACopyANewLifetime() async throws {
        let s = store()
        try await s.put(tile(1), png(100, lifetime: 60), now: now)
        let renewed = try await s.renew(tile(1), fetchedAt: now + 120, expiresAt: now + 3_720)
        XCTAssertTrue(renewed)
        let record = try await s.get(tile(1), now: now + 130)
        XCTAssertEqual(record?.fetchedAt, now + 120)
        let missing = try await s.renew(tile(9), fetchedAt: now, expiresAt: now + 1)
        XCTAssertFalse(missing)
    }

    func testErrorsAreNeverStored() async throws {
        let s = store()
        let page = OfflineEntry(
            data: Data("<html><body>Service unavailable</body></html>".utf8),
            contentType: "text/html", fetchedAt: now, lifetime: 86_400)
        let keptPage = try await s.put(tile(1), page, now: now)
        XCTAssertFalse(keptPage)
        let portal = OfflineEntry(
            data: Data("<!DOCTYPE html><title>Sign in</title>".utf8), contentType: nil,
            fetchedAt: now, lifetime: 3_600)
        let keptPortal = try await s.put(.blob("https://example.org/harcon.json"), portal, now: now)
        XCTAssertFalse(keptPortal)
        let usage = try await s.usage()
        XCTAssertEqual(usage.tiles + usage.blobs, 0)
        XCTAssertEqual(usage.bytes, 0)
    }

    // MARK: Eviction and pins

    func testTheLeastRecentlyUsedUnpinnedRowsGoFirstAndPinnedRowsStay() async throws {
        // Room for ten 100-byte tiles; past it the store comes down to 950 bytes.
        let s = store(limit: 1_000)
        try await s.pin("harbor", [tile(1), tile(2)])
        for x in UInt32(1)...10 {
            try await s.put(tile(x), png(100, seed: UInt8(x)), now: now + Int64(x))
        }
        // Tile 3 was used since it was stored, so 4 is now the oldest unpinned tile.
        try await s.touch(tile(3), now: now + 20)
        try await s.put(tile(11), png(100, seed: 11), now: now + 21)

        for (x, kept) in [(1, true), (2, true), (3, true), (4, false), (5, false), (6, true)] {
            let record = try await s.get(tile(UInt32(x)), now: now + 22)
            XCTAssertEqual(record != nil, kept, "tile \(x)")
        }
        let usage = try await s.usage()
        XCTAssertEqual(usage.bytes, 900)
        XCTAssertEqual(usage.pinnedBytes, 200)
        let measured = try await s.measuredBytes()
        XCTAssertEqual(measured, usage.bytes)
    }

    func testPinnedRowsStayWhenTheyAloneFillTheBound() async throws {
        let s = store(limit: 250)
        try await s.pin("trip", [tile(1), tile(2), tile(3)])
        for x in UInt32(1)...3 {
            try await s.put(tile(x), png(100, seed: UInt8(x)), now: now)
        }
        // No room for anything unpinned: the new tile is not kept.
        try await s.put(tile(4), png(100, seed: 4), now: now + 1)
        let dropped = try await s.get(tile(4), now: now + 1)
        XCTAssertNil(dropped)
        let usage = try await s.usage()
        XCTAssertEqual(usage.tiles, 3)
        XCTAssertEqual(usage.pinnedBytes, 300)
    }

    func testBlobsAndTilesShareOneOrderOfUse() async throws {
        let s = store(limit: 300)
        try await s.put(.blob("a"), json(String(repeating: "1", count: 100)), now: now)
        try await s.put(tile(1), png(100), now: now)
        try await s.put(.blob("b"), json(String(repeating: "2", count: 100)), now: now)
        _ = try await s.get(.blob("a"), now: now)
        try await s.put(tile(2), png(100), now: now)
        let tileGone = try await s.get(tile(1), now: now)
        XCTAssertNil(tileGone, "the oldest row was the tile")
        let blobKept = try await s.get(.blob("a"), now: now)
        XCTAssertNotNil(blobKept)
    }

    func testUnpinningOneOfTwoPacksKeepsTheirSharedTile() async throws {
        let s = store()
        try await s.pin("harbor", [tile(1), tile(2)])
        try await s.pin("hudson", [tile(2), tile(3)])
        for x in UInt32(1)...3 {
            try await s.put(tile(x), png(100, seed: UInt8(x)), now: now)
        }
        let freed = try await s.unpin("harbor")
        XCTAssertEqual(freed, 100, "only the tile no other pack pins is deleted")
        let alone = try await s.get(tile(1), now: now)
        XCTAssertNil(alone)
        let shared = try await s.get(tile(2), now: now)
        XCTAssertNotNil(shared)
        let packs = try await s.packs()
        XCTAssertEqual(packs, ["hudson"])
        let hudson = try await s.usage(of: "hudson")
        XCTAssertEqual(hudson, OfflinePackUsage(items: 2, stored: 2, bytes: 200))

        let freedLast = try await s.unpin("hudson")
        XCTAssertEqual(freedLast, 200)
        let usage = try await s.usage()
        XCTAssertEqual(usage.tiles, 0)
        XCTAssertEqual(usage.bytes, 0)
    }

    func testPackItemsArePinnedBeforeTheyAreDownloaded() async throws {
        let s = store(limit: 100)
        let grid = "sst/jplMURSST41/last/40000000/-74000000/1"
        let keys = [
            OfflineKey(pack: .tile, key: "noaa-chart/latest/12/1/1539"),
            OfflineKey(pack: .grid, key: grid),
        ].compactMap { $0 }
        try await s.pin("trip", keys)
        let before = try await s.usage(of: "trip")
        XCTAssertEqual(before, OfflinePackUsage(items: 2, stored: 0, bytes: 0))
        // The map's key for the same tile finds the pin.
        try await s.put(tile(1), png(100), now: now)
        try await s.put(.blob(grid), json(String(repeating: "3", count: 100)), now: now)
        let after = try await s.usage(of: "trip")
        XCTAssertEqual(after, OfflinePackUsage(items: 2, stored: 2, bytes: 200))
        // Over the bound, both stay: they are pinned.
        let usage = try await s.usage()
        XCTAssertEqual(usage.tiles + usage.blobs, 2)
    }

    func testLoweringTheBoundAndClearingDropOnlyUnpinnedRows() async throws {
        let s = store()
        try await s.pin("trip", [tile(1)])
        for x in UInt32(1)...4 {
            try await s.put(tile(x), png(100, seed: UInt8(x)), now: now + Int64(x))
        }
        try await s.setLimit(bytes: 300)
        var usage = try await s.usage()
        XCTAssertEqual(usage.bytes, 200, "down to 95% of the bound: tiles 1 and 4")
        XCTAssertEqual(usage.limitBytes, 300)
        let freed = try await s.removeUnpinned()
        XCTAssertEqual(freed, 100)
        usage = try await s.usage()
        XCTAssertEqual(usage.bytes, 100)
        let pinned = try await s.get(tile(1), now: now)
        XCTAssertNotNil(pinned)
    }

    // MARK: The file

    func testTheFileKeepsRowsPinsAndSizeAcrossOpens() async throws {
        do {
            let s = store()
            try await s.pin("trip", [tile(1)])
            try await s.put(tile(1), png(120), now: now)
            try await s.put(tile(2), png(80), now: now + 1)
        }
        let s = store()
        let usage = try await s.usage()
        XCTAssertEqual(usage.bytes, 200)
        XCTAssertEqual(usage.pinnedBytes, 120)
        // The order of use carries over: tiles stored now, even with the clock stepped back, are
        // newer than tile 2 from before.
        try await s.setLimit(bytes: 250)
        try await s.put(tile(3), png(50), now: now - 100)
        try await s.put(tile(4), png(50), now: now - 100)
        let older = try await s.get(tile(2), now: now)
        XCTAssertNil(older)
        let newer = try await s.get(tile(3), now: now - 100)
        XCTAssertNotNil(newer)
    }

    func testTheStoreIsInWriteAheadLogModeInADirectoryLeftOutOfBackups() async throws {
        let s = store()
        try await s.open()
        let mode = try await s.journalMode()
        XCTAssertEqual(mode, "wal")
        let values = try directory.resourceValues(forKeys: [.isExcludedFromBackupKey])
        XCTAssertEqual(values.isExcludedFromBackup, true)
        XCTAssertTrue(
            FileManager.default.fileExists(
                atPath: directory.appending(path: OfflineStore.fileName).path(
                    percentEncoded: false)))
    }

    func testTheSchemaVersionIsCheckedOnOpen() async throws {
        let s = store()
        try await s.put(tile(1), png(100), now: now)
        let path = directory.appending(path: OfflineStore.fileName).path(percentEncoded: false)
        do {
            let other = try OfflineSQLite(path: path)
            try other.run("UPDATE meta SET value = 99 WHERE name = 'schema_version'")
        }
        do {
            try await store().open()
            XCTFail("a newer schema is not opened")
        } catch let error as OfflineStoreError {
            XCTAssertEqual(error, .schema(found: 99, expected: OfflineStore.schemaVersion))
        }
        // The file is left as it was until it is reset.
        do {
            let other = try OfflineSQLite(path: path)
            let version = try other.integer(
                "SELECT value FROM meta WHERE name = 'schema_version'")
            XCTAssertEqual(version, 99)
        }
        let fresh = store()
        try await fresh.reset()
        try await fresh.open()
        let usage = try await fresh.usage()
        XCTAssertEqual(usage.tiles, 0)
    }

    func testAFileThatIsNotTheStoresIsRefused() async throws {
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let path = directory.appending(path: OfflineStore.fileName).path(percentEncoded: false)
        do {
            let other = try OfflineSQLite(path: path)
            try other.run("CREATE TABLE notes (text TEXT)")
        }
        do {
            try await store().open()
            XCTFail("a foreign database is not opened")
        } catch let error as OfflineStoreError {
            XCTAssertEqual(error, .schema(found: 0, expected: OfflineStore.schemaVersion))
        }

        try Data("not a database, not even close to one".utf8).write(
            to: URL(filePath: path), options: .atomic)
        do {
            try await store().open()
            XCTFail("a file that is not a database is not opened")
        } catch OfflineStoreError.open {
        }
    }

    // MARK: Speed

    /// 1,000 tiles of 16 KB (about a chart tile's size) in one write: under two seconds.
    func testAThousandTilesGoInUnderTwoSeconds() async throws {
        let s = store()
        try await s.open()
        let items = (0..<1_000).map { i in
            (key: tile(UInt32(i)), entry: png(16_384, seed: UInt8(i % 251)))
        }
        let start = ContinuousClock.now
        let written = try await s.put(items, now: now)
        let elapsed = ContinuousClock.now - start
        XCTAssertEqual(written, 1_000)
        XCTAssertLessThan(elapsed, .seconds(2), "1,000 tiles took \(elapsed)")
        print("offline store: 1,000 tiles of 16 KB in \(elapsed)")
        let usage = try await s.usage()
        XCTAssertEqual(usage.bytes, 16_384_000)
    }
}
