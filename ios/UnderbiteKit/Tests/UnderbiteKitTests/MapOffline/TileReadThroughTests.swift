import Foundation
import XCTest

@testable import UnderbiteKit

/// The rules of reading through the device's store: which step a kept copy leads to, which
/// responses are written to the store and how a layer is badged.
final class ReadThroughRuleTests: XCTestCase {
    func testAFreshCopyIsDrawnAStaleOneAskedAgainAndOfflineAnyCopyIsDrawn() {
        XCTAssertEqual(ReadThrough.step(kept: false, offline: false), .serve)
        XCTAssertEqual(ReadThrough.step(kept: true, offline: false), .fetch)
        XCTAssertEqual(ReadThrough.step(kept: nil, offline: false), .fetch)
        XCTAssertEqual(ReadThrough.step(kept: false, offline: true), .serve)
        XCTAssertEqual(ReadThrough.step(kept: true, offline: true), .serve)
        XCTAssertEqual(ReadThrough.step(kept: nil, offline: true), .missing)
    }

    func testAFailureFallsBackAndNoDataDoesNot() {
        XCTAssertTrue(ReadThrough.fallsBack(after: nil))
        XCTAssertTrue(ReadThrough.fallsBack(after: .failed))
        XCTAssertTrue(ReadThrough.fallsBack(after: .retry(afterS: 30)))
        XCTAssertFalse(ReadThrough.fallsBack(after: .noData))
        XCTAssertFalse(ReadThrough.fallsBack(after: .tooDeep))
        XCTAssertFalse(ReadThrough.fallsBack(after: .image))
    }

    func testTodaysSatelliteTilesAndRadarStayInMemory() throws {
        let chart = try XCTUnwrap(tileRequest(layer: "noaa-chart", date: nil, z: 12, x: 1206, y: 1539))
        XCTAssertTrue(MapOffline.persists(layer: "noaa-chart", chart.request))
        XCTAssertEqual(chart.request.maxAgeS, 86_400)
        let gebco = try XCTUnwrap(tileRequest(layer: "gebco-relief", date: nil, z: 8, x: 75, y: 96))
        XCTAssertTrue(MapOffline.persists(layer: "gebco-relief", gebco.request))
        XCTAssertGreaterThan(gebco.request.maxAgeS ?? 0, 86_400)
        // A satellite layer's current day: `no_store`, no lifetime.
        let today = try XCTUnwrap(tileRequest(layer: "sst-imagery", date: nil, z: 6, x: 18, y: 24))
        XCTAssertTrue(today.request.noStore)
        XCTAssertFalse(MapOffline.persists(layer: "sst-imagery", today.request))
        let dated = try XCTUnwrap(
            tileRequest(layer: "sst-imagery", date: "2026-11-16", z: 6, x: 18, y: 24))
        XCTAssertTrue(MapOffline.persists(layer: "sst-imagery", dated.request))
        let radar = try XCTUnwrap(tileRequest(layer: "radar", date: nil, z: 6, x: 18, y: 24))
        XCTAssertFalse(MapOffline.persists(layer: "radar", radar.request))
    }

    func testALayerIsBadgedOnceWithItsOldestKeptCopy() {
        var pieces = LayerPieces()
        XCTAssertNil(pieces.badge)
        pieces.record("a", .current)
        XCTAssertNil(pieces.badge)
        pieces.record("b", .missing)
        XCTAssertEqual(pieces.badge, .notAvailable)
        pieces.record("c", .stored(fetchedAt: 200))
        pieces.record("d", .stored(fetchedAt: 100))
        XCTAssertEqual(pieces.badge, .cached(fetchedAt: 100))
        // Fetched again: the badge follows what is drawn now.
        pieces.record("d", .current)
        pieces.record("c", .current)
        XCTAssertEqual(pieces.badge, .notAvailable)
        pieces.record("b", .current)
        XCTAssertNil(pieces.badge)
    }

    func testTheRecordStaysBounded() {
        var pieces = LayerPieces()
        for i in 0..<(LayerPieces.capacity + 10) {
            pieces.record("\(i)", .current)
        }
        pieces.record("last", .stored(fetchedAt: 5))
        XCTAssertLessThanOrEqual(pieces.pieces.count, LayerPieces.capacity)
        XCTAssertEqual(pieces.badge, .cached(fetchedAt: 5))
    }
}

/// Tiles through the loader and the device's store: fresh copies drawn without asking, stale ones
/// asked for again online and drawn offline with the layer badged, writes through on success.
@MainActor
final class TileReadThroughTests: XCTestCase {
    /// 2026-11-15T12:00:00Z.
    private let t0: Int64 = 1_794_744_000
    private let old = TilePictures.quarters(side: 64)
    private let new = TilePictures.quarters(side: 128)
    private let scratch = ScratchStore()
    private let status = MapOfflineStatus()

    private func loader(
        _ host: OfflineHost, route: RouteSwitch, clock: MovingClock
    ) -> TileLoader {
        TileLoader(
            host: host, store: scratch.store, status: status, offline: { route.isOffline },
            clock: { clock.date }, pause: { _ in })
    }

    /// The chart's tile 12/1206/1539 as the store keeps it.
    private func chartRow() throws -> OfflineTileKey {
        let fetch = try XCTUnwrap(tileRequest(layer: "noaa-chart", date: nil, z: 12, x: 1206, y: 1539))
        return try XCTUnwrap(
            OfflineTileKey(layer: "noaa-chart", date: nil, z: fetch.z, x: fetch.x, y: fetch.y))
    }

    /// Keeps `data` for the chart tile, fetched at `at`, fresh for a day.
    private func keep(_ data: Data, at: Int64) async throws {
        let entry = OfflineEntry(
            data: data, contentType: "image/png", fetchedAt: at, lifetime: 86_400)
        let written = try await scratch.store.put(.tile(try chartRow()), entry, now: at)
        XCTAssertTrue(written)
    }

    private func chart(_ tiles: TileLoader) async -> Data? {
        await tiles.tile(layer: "noaa-chart", date: nil, z: 12, x: 1206, y: 1539)
    }

    private var chartBadge: LayerBadge? { status.badge(layer: "noaa-chart", date: nil) }

    func testAFreshKeptTileIsDrawnWithoutAsking() async throws {
        try await keep(old, at: t0 - 3_600)
        let host = OfflineHost()
        let tiles = loader(host, route: RouteSwitch(), clock: MovingClock(t0))
        let tile = await chart(tiles)
        XCTAssertEqual(tile, old)
        XCTAssertEqual(host.urls, [])
        await eventually { self.status.layers[MapOfflineStatus.key(layer: "noaa-chart", date: nil)] != nil }
        XCTAssertNil(chartBadge)
    }

    func testAStaleKeptTileIsFetchedAgainOnlineAndReplaced() async throws {
        try await keep(old, at: t0 - 2 * 86_400)
        let host = OfflineHost([hostResponse(200, "image/png", new, etag: "\"n\"")])
        let tiles = loader(host, route: RouteSwitch(), clock: MovingClock(t0))
        let tile = await chart(tiles)
        XCTAssertEqual(tile, new)
        XCTAssertEqual(host.urls.count, 1)
        await tiles.settle()
        let record = try await scratch.store.get(.tile(try chartRow()), now: t0)
        XCTAssertEqual(record?.data, new)
        XCTAssertEqual(record?.fetchedAt, t0)
        XCTAssertEqual(record?.expiresAt, t0 + 86_400)
        XCTAssertEqual(record?.etag, "\"n\"")
        XCTAssertNil(chartBadge)
    }

    func testAStaleKeptTileIsDrawnOfflineWithoutAskingAndTheLayerBadged() async throws {
        try await keep(old, at: t0 - 2 * 86_400)
        let host = OfflineHost([hostResponse(200, "image/png", new)])
        let tiles = loader(host, route: RouteSwitch(offline: true), clock: MovingClock(t0))
        let tile = await chart(tiles)
        XCTAssertEqual(tile, old)
        XCTAssertEqual(host.urls, [])
        await eventually { self.chartBadge == .cached(fetchedAt: self.t0 - 2 * 86_400) }
    }

    func testAFreshKeptTileIsBadgedOfflineToo() async throws {
        try await keep(old, at: t0 - 60)
        let tiles = loader(OfflineHost(), route: RouteSwitch(offline: true), clock: MovingClock(t0))
        let drawn1 = await chart(tiles)
        XCTAssertEqual(drawn1, old)
        await eventually { self.chartBadge == .cached(fetchedAt: self.t0 - 60) }
    }

    func testOfflineWithNothingKeptTheLayerIsNotAvailable() async throws {
        let host = OfflineHost([hostResponse(200, "image/png", new)])
        let tiles = loader(host, route: RouteSwitch(offline: true), clock: MovingClock(t0))
        let tile = await chart(tiles)
        XCTAssertNil(tile)
        XCTAssertEqual(host.urls, [])
        await eventually { self.chartBadge == .notAvailable }
    }

    func testATransportFailureWithARouteDrawsTheStaleCopyOrSaysNotAvailable() async throws {
        try await keep(old, at: t0 - 2 * 86_400)
        let host = OfflineHost()
        let tiles = loader(host, route: RouteSwitch(), clock: MovingClock(t0))
        let drawn2 = await chart(tiles)
        XCTAssertEqual(drawn2, old)
        XCTAssertEqual(host.urls.count, 1)
        await eventually { self.chartBadge == .cached(fetchedAt: self.t0 - 2 * 86_400) }

        let other = await tiles.tile(layer: "gebco-relief", date: nil, z: 8, x: 75, y: 96)
        XCTAssertNil(other)
        await eventually { self.status.badge(layer: "gebco-relief", date: nil) == .notAvailable }
    }

    func testAServerErrorWithARouteDrawsTheStaleCopyBadged() async throws {
        try await keep(old, at: t0 - 2 * 86_400)
        let page = Data("<html>Application Error</html>".utf8)
        let host = OfflineHost([
            hostResponse(200, "text/html", page), hostResponse(200, "text/html", page),
        ])
        let tiles = loader(host, route: RouteSwitch(), clock: MovingClock(t0))
        let drawn3 = await chart(tiles)
        XCTAssertEqual(drawn3, old)
        // The error page was asked again once, then the kept copy drawn; the page is not kept.
        XCTAssertEqual(host.urls.count, 2)
        await tiles.settle()
        let record = try await scratch.store.get(
            .tile(try chartRow()), now: t0, allowingStale: true)
        XCTAssertEqual(record?.data, old)
        await eventually { self.chartBadge == .cached(fetchedAt: self.t0 - 2 * 86_400) }
    }

    func testAFetchedTileIsWrittenThroughAndDrawnOfflineAfterARelaunch() async throws {
        let host = OfflineHost([hostResponse(200, "image/png", new)])
        let clock = MovingClock(t0)
        let first = loader(host, route: RouteSwitch(), clock: clock)
        let drawn4 = await chart(first)
        XCTAssertEqual(drawn4, new)
        await first.settle()

        // A later launch, without a route: nothing in memory, the store answers.
        clock.advance(3 * 86_400)
        let again = loader(host, route: RouteSwitch(offline: true), clock: clock)
        let drawn5 = await chart(again)
        XCTAssertEqual(drawn5, new)
        XCTAssertEqual(host.urls.count, 1)
        await eventually { self.chartBadge == .cached(fetchedAt: self.t0) }
    }

    func testTodaysSatelliteTileIsNeverWrittenToTheStore() async throws {
        let host = OfflineHost([hostResponse(200, "image/png", new)])
        let tiles = loader(host, route: RouteSwitch(), clock: MovingClock(t0))
        let tile = await tiles.tile(layer: "sst-imagery", date: nil, z: 6, x: 18, y: 24)
        XCTAssertEqual(tile, new)
        await tiles.settle()
        let usage = try await scratch.store.usage()
        XCTAssertEqual(usage.tiles, 0)
        // Kept in memory: drawn again without asking.
        let drawn6 = await tiles.tile(layer: "sst-imagery", date: nil, z: 6, x: 18, y: 24)
        XCTAssertEqual(drawn6, new)
        XCTAssertEqual(host.urls.count, 1)
    }

    func testAPinnedTileIsDrawnPastItsLifetimeAndReplacedOnlyBySuccess() async throws {
        try await keep(old, at: t0 - 30 * 86_400)
        try await scratch.store.pin("harbour", [.tile(try chartRow())])
        let page = Data("<html>Application Error</html>".utf8)
        let host = OfflineHost([
            hostResponse(200, "text/html", page), hostResponse(200, "text/html", page),
        ])
        let clock = MovingClock(t0)
        let tiles = loader(host, route: RouteSwitch(), clock: clock)
        let drawn7 = await chart(tiles)
        XCTAssertEqual(drawn7, old)
        await tiles.settle()
        let kept = try await scratch.store.get(.tile(try chartRow()), now: t0, allowingStale: true)
        XCTAssertEqual(kept?.data, old)

        // A later successful fetch replaces it, and the pack still pins it.
        host.answer([hostResponse(200, "image/png", new)])
        clock.advance(60)
        let later = loader(host, route: RouteSwitch(), clock: clock)
        let drawn8 = await chart(later)
        XCTAssertEqual(drawn8, new)
        await later.settle()
        let replaced = try await scratch.store.get(.tile(try chartRow()), now: t0 + 60)
        XCTAssertEqual(replaced?.data, new)
        let usage = try await scratch.store.usage(of: "harbour")
        XCTAssertEqual(usage.stored, 1)
    }
}
