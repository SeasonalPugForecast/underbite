import Foundation
import XCTest

@testable import UnderbiteKit

/// A fetch host that answers every request with a small MUR table, records the URLs asked, and
/// keeps its store in a dictionary; no network.
private final class ScriptedHost: FetchHost, @unchecked Sendable {
    private let lock = NSLock()
    private var values: [String: String] = [:]
    private var asked: [String] = []

    var urls: [String] { lock.withLock { asked } }

    func send(request: HostRequest) async throws -> HostResponse {
        lock.withLock { asked.append(request.url) }
        return GridBlockCache.response(String(decoding: murTable(), as: UTF8.self))
    }

    func load(namespace: String, key: String) -> String? {
        lock.withLock { values["\(namespace)/\(key)"] }
    }

    func store(namespace: String, key: String, value: String) {
        lock.withLock { values["\(namespace)/\(key)"] = value.isEmpty ? nil : value }
    }

    func saveUsage(ledgerJson: String) throws {}
}

/// 2026-11-15T12:00:00Z.
private let now: Int64 = 1_794_744_000

/// The loader's order: blocks kept on the device first, then only what is still missing from the
/// network, an overview ahead of the fine blocks when many are missing.
@MainActor
final class GridLoaderTests: XCTestCase {
    /// Six 1 degree MUR blocks at zoom 9.
    private let region = MapRegion(west: -74.5, south: 40.2, east: -72.5, north: 41.8, zoom: 9)

    private func layers() throws -> [PlannedLayer] {
        var settings = try Settings.defaults()
        for i in settings.map.layers.indices {
            settings.map.layers[i].shown = settings.map.layers[i].id == "sst"
        }
        let catalogue = try mapCatalogue(settingsJson: try settings.json())
        let plan = MapPlan.make(catalogue: catalogue, settings: settings.map, battery: false)
        XCTAssertEqual(plan.layers.map(\.id), ["sst"])
        return plan.layers
    }

    /// The store's requests for the region: the overview first, then the six fine blocks.
    private func requests(_ store: GridStore) throws -> [GridRequest] {
        try store.gridRequests(
            layer: "sst", dataset: nil, time: nil, west: region.west, south: region.south,
            east: region.east, north: region.north, zoom: region.zoom, now: now)
    }

    private func loader(_ host: ScriptedHost, _ store: GridStore) -> GridLoader {
        GridLoader(host: host, store: store, clock: { now })
    }

    func testAColdLoadAsksForTheOverviewFirstAndKeepsEveryBlock() async throws {
        let host = ScriptedHost()
        let store = GridStore()
        let expected = try requests(GridStore())
        XCTAssertEqual(expected.count, 7)
        XCTAssertTrue(expected[0].key.hasSuffix("/6"), expected[0].key)
        let grids = loader(host, store)
        var landed: [String] = []
        grids.onBlock = { landed.append($0) }
        await grids.load(try layers(), in: region)?.value
        XCTAssertEqual(host.urls, expected.map(\.request.url))
        XCTAssertEqual(store.blockCount(), 7)
        XCTAssertEqual(landed.count, 7)
        let kept = await grids.cache.snapshot
        XCTAssertEqual(Set(kept.entries.keys), Set(expected.map(\.key)))
        XCTAssertTrue(kept.entries.values.allSatisfy { $0.fetchedAt == now })
    }

    func testAWarmLoadDrawsFromTheDeviceAndFetchesNothing() async throws {
        let host = ScriptedHost()
        await loader(host, GridStore()).load(try layers(), in: region)?.value
        let sent = host.urls.count
        // A new launch: a new store and loader over the same fetch host.
        let store = GridStore()
        let grids = loader(host, store)
        var landed: [String] = []
        grids.onBlock = { landed.append($0) }
        await grids.load(try layers(), in: region)?.value
        XCTAssertEqual(host.urls.count, sent)
        XCTAssertEqual(store.blockCount(), 7)
        XCTAssertEqual(landed, ["sst"])
        XCTAssertTrue(try requests(store).isEmpty)
    }

    func testKeptBlocksCoverTheirPartAndOnlyTheMissingOnesAreFetched() async throws {
        let host = ScriptedHost()
        let expected = try requests(GridStore())
        let fine = Array(expected[1...])
        // Three of the six fine blocks are kept, fetched a minute ago; no overview is.
        let cache = GridBlockCache(host: host)
        for request in fine[..<3] {
            await cache.save(request.key, body: murTable(), fetchedAt: now - 60)
        }
        let store = GridStore()
        await loader(host, store).load(try layers(), in: region)?.value
        // Three missing are too few for an overview: only they are fetched.
        XCTAssertEqual(Set(host.urls), Set(fine[3...].map(\.request.url)))
        XCTAssertEqual(host.urls.count, 3)
        XCTAssertEqual(store.blockCount(), 6)
        let kept = GridBlockIndex.stored(host.load(namespace: "grid", key: "index"))
        XCTAssertEqual(Set(kept.entries.keys), Set(fine.map(\.key)))
    }

    func testStaleKeptBlocksAreFetchedAgain() async throws {
        let host = ScriptedHost()
        let expected = try requests(GridStore())
        let cache = GridBlockCache(host: host)
        for request in expected {
            await cache.save(request.key, body: murTable(), fetchedAt: now - 4 * 3_600)
        }
        let store = GridStore()
        let grids = loader(host, store)
        await grids.load(try layers(), in: region)?.value
        XCTAssertEqual(host.urls, expected.map(\.request.url))
        let kept = await grids.cache.snapshot
        XCTAssertTrue(kept.entries.values.allSatisfy { $0.fetchedAt == now })
    }
}
