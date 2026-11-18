import Foundation
import XCTest

/// The offline store's keys and rules, apart from SQLite.
final class OfflineRulesTests: XCTestCase {
    // MARK: Keys

    func testTileKeysAreWrittenAsTheCoreNamesThem() throws {
        let dated = try XCTUnwrap(
            OfflineTileKey(layer: "sst-imagery", date: "2026-11-16", z: 7, x: 37, y: 48))
        XCTAssertEqual(dated.text, "sst-imagery/2026-11-16/7/37/48")
        let newest = try XCTUnwrap(
            OfflineTileKey(layer: "noaa-chart", date: nil, z: 14, x: 1, y: 2))
        XCTAssertEqual(newest.text, "noaa-chart/-/14/1/2")
        XCTAssertEqual(newest.dateColumn, "-")
        XCTAssertEqual(OfflineTileKey(text: dated.text), dated)
        XCTAssertEqual(OfflineTileKey(text: newest.text), newest)
    }

    func testPackTileKeysNameTheMapsTiles() throws {
        // A pack plan writes `latest` where the map asks for no date.
        let fromPack = try XCTUnwrap(
            OfflineKey(pack: .tile, key: "noaa-chart/latest/14/4824/6157"))
        let fromMap = try XCTUnwrap(
            OfflineTileKey(layer: "noaa-chart", date: nil, z: 14, x: 4824, y: 6157))
        XCTAssertEqual(fromPack, .tile(fromMap))
        XCTAssertEqual(
            OfflineKey(pack: .tile, key: "true-color/2026-11-16/6/18/24"),
            OfflineTileKey(layer: "true-color", date: "2026-11-16", z: 6, x: 18, y: 24).map {
                OfflineKey.tile($0)
            })
    }

    func testGridAndElevationItemsAreBlobsUnderTheirKeys() {
        let grid = "sst/jplMURSST41/last/40000000/-74000000/1"
        XCTAssertEqual(OfflineKey(pack: .grid, key: grid), .blob(grid))
        let dem = "dem/area/-74000000/40000000/-73900000/40100000/781"
        XCTAssertEqual(OfflineKey(pack: .dem, key: dem), .blob(dem))
        XCTAssertNil(OfflineKey(pack: .grid, key: ""))
    }

    func testMalformedTileKeysAreRefused() {
        for text in [
            "", "a/b/c", "layer/-/7/1", "layer/-/7/1/2/3", "/-/7/1/2", "layer/-/256/1/2",
            "layer/-/7/-1/2", "layer/-/07/1/2", "layer//7/1/2", "layer/-/z/1/2",
        ] {
            XCTAssertNil(OfflineTileKey(text: text), text)
        }
        XCTAssertNil(OfflineTileKey(layer: "a/b", date: nil, z: 1, x: 0, y: 0))
        XCTAssertNil(OfflineTileKey(layer: "a", date: "latest", z: 1, x: 0, y: 0))
        XCTAssertNil(OfflineTileKey(layer: "a", date: "2026/11/16", z: 1, x: 0, y: 0))
    }

    func testPinsNameRowsByKindAndText() throws {
        let tile = try XCTUnwrap(OfflineTileKey(layer: "gebco", date: nil, z: 5, x: 9, y: 12))
        for key in [OfflineKey.tile(tile), .blob("https://example.org/a.json?units=metric")] {
            XCTAssertEqual(OfflineKey(kind: key.kind, text: key.text), key)
        }
    }

    // MARK: What may be kept

    private func entry(_ body: String, _ type: String?) -> OfflineEntry {
        OfflineEntry(data: Data(body.utf8), contentType: type, fetchedAt: 100, expiresAt: 200)
    }

    func testOnlyImagesAreKeptAsTiles() {
        XCTAssertTrue(OfflineRules.admits(entry("PNG", "image/png"), as: .tile))
        XCTAssertTrue(OfflineRules.admits(entry("JPG", " Image/JPEG; q=1"), as: .tile))
        XCTAssertFalse(OfflineRules.admits(entry("PNG", nil), as: .tile))
        XCTAssertFalse(OfflineRules.admits(entry("", "image/png"), as: .tile))
        // The chart service's failures: status 200 with a page or a JSON error.
        XCTAssertFalse(OfflineRules.admits(entry("<html>down</html>", "text/html"), as: .tile))
        XCTAssertFalse(
            OfflineRules.admits(entry(#"{"error":{"code":500}}"#, "text/plain"), as: .tile))
        XCTAssertFalse(
            OfflineRules.admits(entry("\n <!DOCTYPE html><p>busy", "image/png"), as: .tile))
    }

    func testWebPagesAreNeverKeptAsBlobs() {
        XCTAssertTrue(OfflineRules.admits(entry(#"{"table":{}}"#, "application/json"), as: .blob))
        XCTAssertTrue(OfflineRules.admits(entry(#"{"table":{}}"#, nil), as: .blob))
        XCTAssertFalse(OfflineRules.admits(entry("<!doctype html><title>Wi-Fi", nil), as: .blob))
        XCTAssertFalse(OfflineRules.admits(entry("sign in", "text/html; charset=utf-8"), as: .blob))
        XCTAssertFalse(OfflineRules.admits(entry("", nil), as: .blob))
    }

    // MARK: Freshness and use

    func testLifetimesEndAtTheirExpiry() {
        let e = OfflineEntry(data: Data([1]), contentType: nil, fetchedAt: 1_000, lifetime: 86_400)
        XCTAssertEqual(e.expiresAt, 87_400)
        XCTAssertTrue(OfflineRules.isFresh(expiresAt: e.expiresAt, now: 87_399))
        XCTAssertFalse(OfflineRules.isFresh(expiresAt: e.expiresAt, now: 87_400))
        let never = OfflineEntry(data: Data([1]), contentType: nil, fetchedAt: 1_000, lifetime: 0)
        XCTAssertFalse(OfflineRules.isFresh(expiresAt: never.expiresAt, now: 1_000))
        let long = OfflineEntry(data: Data([1]), contentType: nil, fetchedAt: 1, lifetime: .max)
        XCTAssertEqual(long.expiresAt, .max)
    }

    func testStampsOrderEveryUse() {
        XCTAssertEqual(OfflineRules.stamp(now: 100, after: 0), 100_000)
        // Within one millisecond, and with the clock stepped back.
        XCTAssertEqual(OfflineRules.stamp(now: 100, after: 100_000), 100_001)
        XCTAssertEqual(OfflineRules.stamp(now: 50, after: 100_001), 100_002)
        XCTAssertEqual(OfflineRules.stamp(now: .max, after: 0), .max)
    }

    // MARK: Bound and eviction

    func testTheBoundIsInDecimalMegabytes() {
        XCTAssertEqual(OfflineRules.limitBytes(megabytes: 1500), 1_500_000_000)
        XCTAssertEqual(OfflineRules.limitBytes(megabytes: 0), 0)
        XCTAssertEqual(OfflineRules.limitBytes(megabytes: -3), 0)
        XCTAssertEqual(OfflineRules.lowWater(1_000), 950)
        XCTAssertEqual(OfflineRules.lowWater(0), 0)
    }

    private func candidate(_ kind: OfflineKind, _ rowid: Int64, used: Int64, bytes: Int64 = 10)
        -> OfflineRules.Candidate
    {
        OfflineRules.Candidate(kind: kind, rowid: rowid, bytes: bytes, used: used)
    }

    func testCandidatesMergeByLastUse() {
        let tiles = [candidate(.tile, 1, used: 1), candidate(.tile, 2, used: 5)]
        let blobs = [candidate(.blob, 1, used: 3)]
        XCTAssertEqual(
            OfflineRules.oldestFirst(tiles: tiles, blobs: blobs, batch: 10).map(\.used), [1, 3, 5])
    }

    func testAFullBatchBoundsTheMerge() {
        // The tiles' batch is full: a tile past it may be older than blob 9, so blob 9 waits.
        let tiles = [candidate(.tile, 1, used: 1), candidate(.tile, 2, used: 4)]
        let blobs = [candidate(.blob, 1, used: 2), candidate(.blob, 2, used: 9)]
        XCTAssertEqual(
            OfflineRules.oldestFirst(tiles: tiles, blobs: blobs, batch: 2).map(\.used), [1, 2, 4])
    }

    func testVictimsBringTheTotalDownToTheTarget() {
        let ordered = (1...5).map { candidate(.tile, Int64($0), used: Int64($0), bytes: 10) }
        XCTAssertEqual(
            OfflineRules.victims(ordered, total: 100, target: 75).map(\.rowid), [1, 2, 3])
        XCTAssertEqual(OfflineRules.victims(ordered, total: 70, target: 75), [])
        XCTAssertEqual(OfflineRules.victims(ordered, total: 100, target: 0).count, 5)
    }
}
