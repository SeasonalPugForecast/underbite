import Foundation
import XCTest

@testable import UnderbiteKit

/// 2026-11-15T12:00:00Z.
private let now: Int64 = 1_794_744_000

/// Grid blocks kept as blobs of the device's store under the core's keys, handed back to the
/// core's store while the core says they are fresh, and at any age when asked to.
final class GridBlockCacheTests: XCTestCase {
    private let key = "sst/jplMURSST41/last/40/-74/1"
    private let scratch = ScratchStore()

    private func save(_ cache: GridBlockCache, _ key: String, _ body: Data, at: Int64) async {
        await cache.save(
            key, body: body, contentType: "application/json;charset=UTF-8", fetchedAt: at,
            lifetime: 10_800)
    }

    func testAKeptBlockGoesBackIntoTheStoreWhileFresh() async throws {
        let cache = GridBlockCache(store: scratch.store)
        await save(cache, key, murTable(), at: now - 60)
        let record = try await scratch.store.get(.blob(key), now: now)
        XCTAssertEqual(record?.fetchedAt, now - 60)

        // A new launch: a new cache over the same store, a new grid store.
        let grids = GridStore()
        let later = GridBlockCache(store: scratch.store)
        let added = await later.restore([key, "sst/jplMURSST41/last/41/-74/1"], into: grids, now: now)
        XCTAssertEqual(added, [.init(key: key, fetchedAt: now - 60, fresh: true)])
        XCTAssertEqual(grids.blockCount(), 1)
        // The store holds it with its own fetch time: still fresh, so not asked again.
        let asked = try grids.gridRequests(
            layer: "sst", dataset: nil, time: nil, west: -73.9, south: 40.1, east: -73.1,
            north: 40.9, zoom: 9, now: now)
        XCTAssertTrue(asked.isEmpty, "\(asked.map(\.key))")
    }

    func testAStaleBlockIsLeftForTheNetworkUnlessStaleOnesAreAllowed() async throws {
        let cache = GridBlockCache(store: scratch.store)
        // The newest MUR day is kept three hours (it is revised daily).
        await save(cache, key, murTable(), at: now - 4 * 3_600)
        let grids = GridStore()
        let none = await cache.restore([key], into: grids, now: now)
        XCTAssertEqual(none, [])
        XCTAssertEqual(grids.blockCount(), 0)
        let stale = await cache.restore([key], into: grids, now: now, allowingStale: true)
        XCTAssertEqual(stale, [.init(key: key, fetchedAt: now - 4 * 3_600, fresh: false)])
        XCTAssertEqual(grids.blockCount(), 1)
        // In the grid store with its own time: asked for again online.
        let asked = try grids.gridRequests(
            layer: "sst", dataset: nil, time: nil, west: -73.9, south: 40.1, east: -73.1,
            north: 40.9, zoom: 9, now: now)
        XCTAssertTrue(asked.map(\.key).contains(key))
    }

    func testAnUnreadableBodyIsSkippedAndAWebPageNeverKept() async throws {
        let cache = GridBlockCache(store: scratch.store)
        await save(cache, key, Data("not an ERDDAP table".utf8), at: now)
        await cache.save(
            "sst/jplMURSST41/last/41/-74/1", body: Data("<html>busy</html>".utf8),
            contentType: "text/html", fetchedAt: now, lifetime: 10_800)
        let usage = try await scratch.store.usage()
        XCTAssertEqual(usage.blobs, 1)
        let added = await cache.restore(
            [key, "sst/jplMURSST41/last/41/-74/1"], into: GridStore(), now: now,
            allowingStale: true)
        XCTAssertEqual(added, [])
    }

    func testBlocksKeptByAnEarlierVersionMoveIntoTheStore() async throws {
        let host = OfflineHost()
        let name = "sst.jplMURSST41.last.40.-74.1"
        host.store(namespace: "grid", key: name, value: String(decoding: murTable(), as: UTF8.self))
        host.store(
            namespace: "grid", key: "index",
            value: #"{"clock":1,"entries":{"\#(key)":{"bytes":500,"fetchedAt":\#(now - 60),"used":1}},"version":1}"#)
        let cache = GridBlockCache(store: scratch.store, host: host)
        let grids = GridStore()
        let added = await cache.restore([key], into: grids, now: now)
        XCTAssertEqual(added.map(\.key), [key])
        XCTAssertEqual(added.first?.fetchedAt, now - 60)
        XCTAssertNil(host.value("grid", name))
        XCTAssertNil(host.value("grid", "index"))
        let record = try await scratch.store.get(.blob(key), now: now, allowingStale: true)
        XCTAssertNotNil(record)
    }

    func testLegacyNamesMapKeysWithDots() {
        XCTAssertEqual(
            LegacyIndex.name(of: "great-lakes-sst/GLSEA_ACSPO_GCS/1794683995/40/-84/1"),
            "great-lakes-sst.GLSEA_ACSPO_GCS.1794683995.40.-84.1")
        XCTAssertNil(LegacyIndex.name(of: ""))
        XCTAssertNil(LegacyIndex.name(of: "a.b/c"))
        XCTAssertNil(LegacyIndex.name(of: "sst/a b"))
        XCTAssertEqual(LegacyIndex.entries("not json").count, 0)
    }
}

/// The grid loader without a route, or when the network fails: kept blocks at any age, nothing
/// fetched offline, the layer badged.
@MainActor
final class GridOfflineTests: XCTestCase {
    /// Six 1 degree MUR blocks at zoom 9.
    private let region = MapRegion(west: -74.5, south: 40.2, east: -72.5, north: 41.8, zoom: 9)
    private let scratch = ScratchStore()
    private let status = MapOfflineStatus()

    private func layers() throws -> [PlannedLayer] {
        var settings = try Settings.defaults()
        for i in settings.map.layers.indices {
            settings.map.layers[i].shown = settings.map.layers[i].id == "sst"
        }
        let catalogue = try mapCatalogue(settingsJson: try settings.json())
        return MapPlan.make(catalogue: catalogue, settings: settings.map, battery: false).layers
    }

    private func requests() throws -> [GridRequest] {
        try GridStore().gridRequests(
            layer: "sst", dataset: nil, time: nil, west: region.west, south: region.south,
            east: region.east, north: region.north, zoom: region.zoom, now: now)
    }

    private func loader(_ host: OfflineHost, _ grids: GridStore, offline: Bool) -> GridLoader {
        GridLoader(
            host: host, store: grids, clock: { now },
            cache: GridBlockCache(store: scratch.store), offline: { offline }, status: status)
    }

    private var badge: LayerBadge? { status.badge(layer: "sst", date: nil) }

    /// Keeps the first `count` fine blocks, fetched `age` seconds ago.
    private func keep(_ count: Int, age: Int64) async throws {
        let cache = GridBlockCache(store: scratch.store)
        for request in try requests()[1...].prefix(count) {
            await cache.save(
                request.key, body: murTable(), contentType: "application/json",
                fetchedAt: now - age, lifetime: 10_800)
        }
    }

    func testOfflineStaleBlocksAreDrawnNothingIsFetchedAndTheLayerBadged() async throws {
        try await keep(6, age: 2 * 86_400)
        let host = OfflineHost()
        let grids = GridStore()
        let loads = loader(host, grids, offline: true)
        var landed: [String] = []
        loads.onBlock = { landed.append($0) }
        await loads.load(try layers(), in: region)?.value
        XCTAssertEqual(host.urls, [])
        XCTAssertEqual(grids.blockCount(), 6)
        XCTAssertEqual(landed, ["sst"])
        await eventually { self.badge == .cached(fetchedAt: now - 2 * 86_400) }
    }

    func testOfflineWithNothingKeptTheLayerIsNotAvailable() async throws {
        let host = OfflineHost()
        let grids = GridStore()
        await loader(host, grids, offline: true).load(try layers(), in: region)?.value
        XCTAssertEqual(host.urls, [])
        XCTAssertEqual(grids.blockCount(), 0)
        await eventually { self.badge == .notAvailable }
    }

    func testStaleBlocksAreFetchedAgainOnline() async throws {
        try await keep(6, age: 4 * 3_600)
        let expected = try requests()
        let host = OfflineHost(expected.map { _ in hostResponse(200, "application/json", murTable()) })
        let grids = GridStore()
        await loader(host, grids, offline: false).load(try layers(), in: region)?.value
        XCTAssertEqual(host.urls, expected.map(\.request.url))
        let record = try await scratch.store.get(.blob(expected[1].key), now: now)
        XCTAssertEqual(record?.fetchedAt, now)
        await eventually { self.status.layers[MapOfflineStatus.key(layer: "sst", date: nil)] != nil }
        XCTAssertNil(badge)
    }

    func testAFailedFetchWithARouteDrawsTheStaleBlocksBadged() async throws {
        try await keep(6, age: 4 * 3_600)
        // The network fails at the transport for every block.
        let host = OfflineHost()
        let grids = GridStore()
        await loader(host, grids, offline: false).load(try layers(), in: region)?.value
        XCTAssertFalse(host.urls.isEmpty)
        XCTAssertEqual(grids.blockCount(), 6)
        await eventually { self.badge == .cached(fetchedAt: now - 4 * 3_600) }
    }
}

/// Time axes kept in the device's store: drawn offline at any age without asking.
final class TimeAxesOfflineTests: XCTestCase {
    /// 2026-11-17 12:00 UTC.
    private let t0: Int64 = 1_794_916_800
    private let scratch = ScratchStore()

    private func axis(_ days: [String]) -> Result<HostResponse, HostError> {
        let rows = days.map { #"["\#($0)T12:00:00Z"]"# }.joined(separator: ",")
        let json = #"""
            {"table":{"columnNames":["time"],"columnTypes":["String"],"columnUnits":["UTC"],"rows":[\#(rows)]}}
            """#
        return hostResponse(200, "application/json;charset=UTF-8", Data(json.utf8))
    }

    func testAKeptAxisIsUsedOfflineWithoutAsking() async throws {
        let host = OfflineHost([axis(["2026-11-14", "2026-11-15"])])
        let clock = MovingClock(t0)
        let route = RouteSwitch()
        let key = TimeAxes.Key(layer: "chlorophyll", dataset: nil)
        let json = try Settings.defaults().json()
        let first = TimeAxes(
            host: host, store: scratch.store, offline: { route.isOffline },
            clock: { clock.seconds }, pause: { _ in })
        let times = await first.times(key, dates: .daily, settingsJSON: json, timeZone: "UTC")
        XCTAssertEqual(times?.count, 2)
        let record = try await scratch.store.get(.blob(key.storeKey), now: t0)
        XCTAssertEqual(record?.expiresAt, t0 + 86_400)

        // Days later, without a route, a new launch reads the kept axis.
        clock.advance(5 * 86_400)
        route.set(offline: true)
        let later = TimeAxes(
            host: host, store: scratch.store, offline: { route.isOffline },
            clock: { clock.seconds }, pause: { _ in })
        let kept = await later.times(key, dates: .daily, settingsJSON: json, timeZone: "UTC")
        XCTAssertEqual(kept?.count, 2)
        XCTAssertEqual(host.urls.count, 1)
    }

    func testAnAxisKeptByAnEarlierVersionMovesIntoTheStore() async throws {
        let host = OfflineHost()
        let key = TimeAxes.Key(layer: "sst", dataset: nil)
        let body = #"{"table":{"columnNames":["time"],"columnTypes":["String"],"columnUnits":["UTC"],"rows":[["2026-11-15T09:00:00Z"]]}}"#
        let kept: [String: Any] = [
            "fetched": t0 - 60, "status": 200, "contentType": "application/json", "body": body,
        ]
        let text = String(decoding: try JSONSerialization.data(withJSONObject: kept), as: UTF8.self)
        host.store(namespace: "map-axes", key: "sst.primary", value: text)
        let t0 = t0
        let axes = TimeAxes(
            host: host, store: scratch.store, offline: { false }, clock: { t0 },
            pause: { _ in })
        let times = await axes.times(
            key, dates: .daily, settingsJSON: try Settings.defaults().json(), timeZone: "UTC")
        XCTAssertEqual(times?.count, 1)
        // Fresh as kept: not asked for; moved into the store and deleted from the host's.
        XCTAssertEqual(host.urls, [])
        XCTAssertNil(host.value("map-axes", "sst.primary"))
        let record = try await scratch.store.get(.blob(key.storeKey), now: t0, allowingStale: true)
        XCTAssertEqual(record?.fetchedAt, t0 - 60)
    }
}
