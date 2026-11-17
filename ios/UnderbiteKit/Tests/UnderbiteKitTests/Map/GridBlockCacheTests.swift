import Foundation
import XCTest

@testable import UnderbiteKit

/// A fetch host whose store is a dictionary; it sends nothing.
private final class MemoryHost: FetchHost, @unchecked Sendable {
    private let lock = NSLock()
    private var values: [String: String] = [:]

    func value(_ namespace: String, _ key: String) -> String? {
        lock.withLock { values["\(namespace)/\(key)"] }
    }

    func remove(_ namespace: String, _ key: String) {
        _ = lock.withLock { values.removeValue(forKey: "\(namespace)/\(key)") }
    }

    func send(request: HostRequest) async throws -> HostResponse {
        throw HostError.Failed(message: "no network in tests")
    }

    func load(namespace: String, key: String) -> String? { value(namespace, key) }

    func store(namespace: String, key: String, value: String) {
        lock.withLock { values["\(namespace)/\(key)"] = value.isEmpty ? nil : value }
    }

    func saveUsage(ledgerJson: String) throws {}
}

/// 2026-11-15T12:00:00Z.
private let now: Int64 = 1_794_744_000

/// A hand-made ERDDAP `.json` table of MUR SST: two by two cells from `south`, `west`.
func murTable(south: Double = 40, west: Double = -74, value: Double = 16) -> Data {
    var rows: [String] = []
    for i in 0..<2 {
        for j in 0..<2 {
            let (lat, lon) = (south + 0.5 * Double(i), west + 0.5 * Double(j))
            rows.append("[\"2026-11-07T09:00:00Z\", \(lat), \(lon), \(value), 1]")
        }
    }
    let table = """
        {"table": {"columnNames": ["time", "latitude", "longitude", "analysed_sst", "mask"],
        "columnTypes": ["String", "float", "float", "double", "byte"],
        "columnUnits": ["UTC", "degrees_north", "degrees_east", "degree_C", null],
        "rows": [\(rows.joined(separator: ", "))]}}
        """
    return Data(table.utf8)
}

/// The bookkeeping of the blocks kept on the device: sizes, the least recently used dropped
/// first, the stored form and the names bodies are stored under.
final class GridBlockIndexTests: XCTestCase {
    func testTheLeastRecentlyUsedBlocksAreDroppedPastTheCapacity() {
        var index = GridBlockIndex()
        XCTAssertEqual(index.put("a", bytes: 400, fetchedAt: 1, capacity: 1000), [])
        XCTAssertEqual(index.put("b", bytes: 400, fetchedAt: 2, capacity: 1000), [])
        // A third does not fit: the oldest goes.
        XCTAssertEqual(index.put("c", bytes: 400, fetchedAt: 3, capacity: 1000), ["a"])
        XCTAssertEqual(index.totalBytes, 800)
        // b is used, so c is now the oldest.
        index.touch("b")
        XCTAssertEqual(index.put("d", bytes: 400, fetchedAt: 4, capacity: 1000), ["c"])
        XCTAssertEqual(Set(index.entries.keys), ["b", "d"])
        // A larger block makes room for itself.
        XCTAssertEqual(Set(index.put("e", bytes: 1000, fetchedAt: 5, capacity: 1000)), ["b", "d"])
        XCTAssertEqual(index.entry("e")?.fetchedAt, 5)
        XCTAssertEqual(index.totalBytes, 1000)
    }

    func testABlockIsReplacedUnderItsKeyAndOneTooLargeIsNotKept() {
        var index = GridBlockIndex()
        _ = index.put("a", bytes: 300, fetchedAt: 1, capacity: 1000)
        XCTAssertEqual(index.put("a", bytes: 500, fetchedAt: 9, capacity: 1000), [])
        XCTAssertEqual(index.entry("a")?.bytes, 500)
        XCTAssertEqual(index.entry("a")?.fetchedAt, 9)
        XCTAssertEqual(index.totalBytes, 500)
        // Larger than everything: not kept, and what was kept under its key goes.
        XCTAssertEqual(index.put("a", bytes: 1001, fetchedAt: 10, capacity: 1000), ["a"])
        XCTAssertNil(index.entry("a"))
        XCTAssertEqual(index.put("b", bytes: 1001, fetchedAt: 10, capacity: 1000), [])
        index.touch("missing")
        index.remove("missing")
        XCTAssertTrue(index.entries.isEmpty)
    }

    func testTheIndexIsStoredAsJSONAndAnotherVersionStartsAfresh() {
        var index = GridBlockIndex()
        _ = index.put("sst/jplMURSST41/last/40/-74/1", bytes: 70_000, fetchedAt: now)
        let again = GridBlockIndex.stored(index.json())
        XCTAssertEqual(again, index)
        XCTAssertEqual(GridBlockIndex.stored(nil), GridBlockIndex())
        XCTAssertEqual(GridBlockIndex.stored("not json"), GridBlockIndex())
        let other = index.json().replacingOccurrences(of: "\"version\":1", with: "\"version\":2")
        XCTAssertNotEqual(other, index.json())
        XCTAssertEqual(GridBlockIndex.stored(other), GridBlockIndex())
        XCTAssertEqual(GridBlockIndex.capacity, 50 * 1024 * 1024)
    }

    func testABodyIsStoredUnderTheKeyWithDots() {
        XCTAssertEqual(
            GridBlockIndex.name(of: "sst/jplMURSST41/last/40/-74/1"), "sst.jplMURSST41.last.40.-74.1")
        XCTAssertEqual(
            GridBlockIndex.name(of: "great-lakes-sst/GLSEA_ACSPO_GCS/1794683995/40/-84/1"),
            "great-lakes-sst.GLSEA_ACSPO_GCS.1794683995.40.-84.1")
        XCTAssertNil(GridBlockIndex.name(of: ""))
        XCTAssertNil(GridBlockIndex.name(of: "a.b/c"))
        XCTAssertNil(GridBlockIndex.name(of: "/sst"))
        XCTAssertNil(GridBlockIndex.name(of: "sst/a b"))
    }
}

/// Blocks kept through the fetch host's store and handed back to the core's store while the
/// core says they are fresh.
final class GridBlockCacheTests: XCTestCase {
    private let key = "sst/jplMURSST41/last/40/-74/1"
    private let bodyName = "sst.jplMURSST41.last.40.-74.1"

    func testAKeptBlockGoesBackIntoTheStoreWhileFresh() async throws {
        let host = MemoryHost()
        let cache = GridBlockCache(host: host)
        await cache.save(key, body: murTable(), fetchedAt: now - 60)
        XCTAssertNotNil(host.value("grid", bodyName))
        XCTAssertNotNil(host.value("grid", "index"))

        // A new launch: a new cache on the same host, a new store.
        let store = GridStore()
        let later = GridBlockCache(host: host)
        let added = await later.restore([key, "sst/jplMURSST41/last/41/-74/1"], into: store, now: now)
        XCTAssertEqual(added, [key])
        XCTAssertEqual(store.blockCount(), 1)
        // The store holds it with its own fetch time: still fresh, so not asked again.
        let asked = try store.gridRequests(
            layer: "sst", dataset: nil, time: nil, west: -73.9, south: 40.1, east: -73.1,
            north: 40.9, zoom: 9, now: now)
        XCTAssertTrue(asked.isEmpty, "\(asked.map(\.key))")
    }

    func testAStaleBlockIsLeftForTheNetwork() async throws {
        let host = MemoryHost()
        let cache = GridBlockCache(host: host)
        // The newest day is kept three hours.
        await cache.save(key, body: murTable(), fetchedAt: now - 3 * 3_600)
        let store = GridStore()
        let added = await cache.restore([key], into: store, now: now)
        XCTAssertEqual(added, [])
        XCTAssertEqual(store.blockCount(), 0)
        // It stays until the fetch replaces it.
        let kept = await cache.snapshot
        XCTAssertNotNil(kept.entry(key))
    }

    func testALostOrUnreadableBodyIsForgotten() async throws {
        let host = MemoryHost()
        let cache = GridBlockCache(host: host)
        let other = "sst/jplMURSST41/last/41/-74/1"
        await cache.save(key, body: Data("not an ERDDAP table".utf8), fetchedAt: now)
        await cache.save(other, body: murTable(south: 41), fetchedAt: now)
        host.remove("grid", "sst.jplMURSST41.last.41.-74.1")
        let store = GridStore()
        let added = await cache.restore([key, other], into: store, now: now)
        XCTAssertEqual(added, [])
        let kept = await cache.snapshot
        XCTAssertTrue(kept.entries.isEmpty)
        XCTAssertNil(host.value("grid", bodyName))
        // The index is written back with the next flush.
        await cache.flush()
        XCTAssertTrue(GridBlockIndex.stored(host.value("grid", "index")).entries.isEmpty)
    }

    func testOldBodiesAreDeletedWhenTheCacheIsFull() async throws {
        let host = MemoryHost()
        let body = murTable()
        let cache = GridBlockCache(host: host, capacity: body.count * 2)
        let keys = (0..<3).map { "sst/jplMURSST41/last/4\($0)/-74/1" }
        for k in keys {
            await cache.save(k, body: body, fetchedAt: now)
        }
        XCTAssertNil(host.value("grid", "sst.jplMURSST41.last.40.-74.1"))
        XCTAssertNotNil(host.value("grid", "sst.jplMURSST41.last.41.-74.1"))
        XCTAssertNotNil(host.value("grid", "sst.jplMURSST41.last.42.-74.1"))
        let stored = GridBlockIndex.stored(host.value("grid", "index"))
        XCTAssertEqual(Set(stored.entries.keys), Set(keys[1...]))
        XCTAssertEqual(stored.totalBytes, body.count * 2)
    }
}
