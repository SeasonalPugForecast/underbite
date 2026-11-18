import Foundation
import XCTest

@testable import UnderbiteKit

/// A fetch host that answers from a list and records what was asked; no network.
private final class ScriptedHost: FetchHost, @unchecked Sendable {
    private let lock = NSLock()
    private var answers: [Result<HostResponse, HostError>]
    private var asked: [String] = []
    private let delay: Duration

    init(_ answers: [Result<HostResponse, HostError>], delay: Duration = .zero) {
        self.answers = answers
        self.delay = delay
    }

    var urls: [String] { lock.withLock { asked } }

    func send(request: HostRequest) async throws -> HostResponse {
        let answer: Result<HostResponse, HostError> = lock.withLock {
            asked.append(request.url)
            return answers.isEmpty ? .failure(.Failed(message: "no answer")) : answers.removeFirst()
        }
        if delay > .zero { try? await Task.sleep(for: delay) }
        return try answer.get()
    }

    func load(namespace: String, key: String) -> String? { nil }
    func store(namespace: String, key: String, value: String) {}
    func saveUsage(ledgerJson: String) throws {}
}

/// A clock the tests move.
private final class TestClock: @unchecked Sendable {
    private let lock = NSLock()
    private var now = Date(timeIntervalSince1970: 1_794_000_000)
    func read() -> Date { lock.withLock { now } }
    func advance(_ seconds: TimeInterval) { lock.withLock { now += seconds } }
}

private func response(
    _ status: UInt16, _ contentType: String?, _ body: Data
) -> Result<HostResponse, HostError> {
    .success(
        HostResponse(
            status: status, body: body, contentType: contentType, date: nil, expires: nil,
            lastModified: nil, etag: nil, retryAfter: nil, rateLimitRemaining: nil))
}

/// Tiles through the loader: the core's requests and verdicts, the cache, the fallback, the
/// over-zoom cut and one fetch per ancestor.
final class TileLoaderTests: XCTestCase {
    private let png = TilePictures.quarters()
    /// The device's store, empty for each test.
    private let scratch = ScratchStore()

    private func loader(_ host: ScriptedHost, clock: TestClock = TestClock()) -> TileLoader {
        TileLoader(
            host: host, store: scratch.store, status: MapOfflineStatus(), offline: { false },
            clock: { clock.read() }, pause: { _ in })
    }

    func testAChartTileIsFetchedOnceAndThenServedFromMemory() async throws {
        let host = ScriptedHost([response(200, "image/png", png)])
        let tiles = loader(host)
        let first = await tiles.tile(layer: "noaa-chart", date: nil, z: 12, x: 1206, y: 1539)
        let again = await tiles.tile(layer: "noaa-chart", date: nil, z: 12, x: 1206, y: 1539)
        XCTAssertEqual(first, png)
        XCTAssertEqual(again, png)
        XCTAssertEqual(host.urls.count, 1)
        // The chart's levels are the map's zoom less two, row before column.
        XCTAssertTrue(host.urls[0].hasSuffix("/10/1539/1206.png"), host.urls[0])
    }

    func testAFailedTileIsNotKept() async throws {
        let host = ScriptedHost([
            response(500, "text/html", Data("down".utf8)), response(200, "image/png", png),
        ])
        let tiles = loader(host)
        let failed = await tiles.tile(layer: "noaa-chart", date: nil, z: 12, x: 1206, y: 1539)
        XCTAssertNil(failed)
        let next = await tiles.tile(layer: "noaa-chart", date: nil, z: 12, x: 1206, y: 1539)
        XCTAssertEqual(next, png)
        XCTAssertEqual(host.urls.count, 2)
    }

    func testAChartFailureFallsBackToTheLastTileKept() async throws {
        let errorPage = Data("<html>Application Error</html>".utf8)
        let host = ScriptedHost([
            response(200, "image/png", png),
            // A day later, the service answers with its error page twice.
            response(200, "text/html", errorPage), response(200, "text/html", errorPage),
        ])
        let clock = TestClock()
        let tiles = loader(host, clock: clock)
        _ = await tiles.tile(layer: "noaa-chart", date: nil, z: 12, x: 1206, y: 1539)
        clock.advance(2 * 86_400)
        let stale = await tiles.tile(layer: "noaa-chart", date: nil, z: 12, x: 1206, y: 1539)
        XCTAssertEqual(stale, png)
        // The error page was asked again once before falling back.
        XCTAssertEqual(host.urls.count, 3)
    }

    func testNoTileAndNoFallbackDrawsNothing() async {
        let host = ScriptedHost([.failure(.TimedOut(message: "slow"))])
        let tile = await loader(host).tile(layer: "noaa-chart", date: nil, z: 12, x: 1206, y: 1539)
        XCTAssertNil(tile)
    }

    func testAMissingSatelliteDayDrawsNothingAndIsReported() async throws {
        let host = ScriptedHost([response(404, "text/xml", Data("<x/>".utf8))])
        let tiles = loader(host)
        let reports = Reports()
        await tiles.onReport { layer, date, outcome in reports.add(layer, date, outcome) }
        let tile = await tiles.tile(
            layer: "true-color-viirs-n20", date: "2026-11-16", z: 6, x: 18, y: 24)
        XCTAssertNil(tile)
        XCTAssertEqual(reports.all, ["true-color-viirs-n20 2026-11-16 noData"])
        XCTAssertTrue(host.urls[0].contains("2026-11-16"), host.urls[0])
    }

    func testDeeperTilesAreCutFromTheSourcesDeepestTile() async throws {
        let host = ScriptedHost([response(200, "image/png", png)])
        let tiles = loader(host)
        // MUR imagery from GIBS stops at zoom 7: zoom 9 tile (150, 193) is a part of (37, 48).
        let cut = await tiles.tile(layer: "sst-imagery", date: "2026-11-16", z: 9, x: 150, y: 193)
        let deep = try XCTUnwrap(cut)
        let ancestor = try XCTUnwrap(
            tileRequest(layer: "sst-imagery", date: "2026-11-16", z: 7, x: 37, y: 48))
        XCTAssertEqual(host.urls, [ancestor.request.url])
        XCTAssertEqual(TilePictures.pixel(deep, x: 0, y: 0)?.width, 256)
        // (150, 193) is column 2 of 4 and row 1 of 4 in its ancestor: the top-right quarter.
        XCTAssertEqual(TilePictures.pixel(deep, x: 128, y: 128)?.rgba, [0, 255, 0, 255])
        // Its neighbour shares the fetch.
        let sibling = await tiles.tile(layer: "sst-imagery", date: "2026-11-16", z: 9, x: 151, y: 193)
        XCTAssertNotNil(sibling)
        XCTAssertEqual(host.urls.count, 1)
    }

    func testTilesSharingAnAncestorShareOneFetchInFlight() async throws {
        let host = ScriptedHost([response(200, "image/png", png)], delay: .milliseconds(100))
        let tiles = loader(host)
        async let a = tiles.tile(layer: "sst-imagery", date: nil, z: 10, x: 300, y: 386)
        async let b = tiles.tile(layer: "sst-imagery", date: nil, z: 10, x: 301, y: 387)
        let (first, second) = await (a, b)
        XCTAssertNotNil(first)
        XCTAssertNotNil(second)
        XCTAssertEqual(host.urls.count, 1)
    }

    func testOtherDaysAreOtherTiles() async throws {
        let host = ScriptedHost([response(200, "image/png", png), response(200, "image/png", png)])
        let tiles = loader(host)
        _ = await tiles.tile(layer: "sst-imagery", date: "2026-11-16", z: 6, x: 18, y: 24)
        _ = await tiles.tile(layer: "sst-imagery", date: "2026-11-15", z: 6, x: 18, y: 24)
        XCTAssertEqual(host.urls.count, 2)
        XCTAssertNotEqual(host.urls[0], host.urls[1])
    }
}

/// The reports a loader sent.
private final class Reports: @unchecked Sendable {
    private let lock = NSLock()
    private var lines: [String] = []
    var all: [String] { lock.withLock { lines } }
    func add(_ layer: String, _ date: String?, _ outcome: TileOutcome) {
        lock.withLock { lines.append("\(layer) \(date ?? "-") \(outcome)") }
    }
}
