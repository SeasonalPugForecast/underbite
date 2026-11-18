import Foundation
import XCTest

@testable import UnderbiteKit

/// A fetch host that answers from a list, records the URLs asked and keeps stored values in
/// memory; no network.
private final class ListHost: FetchHost, @unchecked Sendable {
    private let lock = NSLock()
    private var answers: [Result<HostResponse, HostError>]
    private var asked: [String] = []
    private var stored: [String: String] = [:]

    init(_ answers: [Result<HostResponse, HostError>] = []) {
        self.answers = answers
    }

    var urls: [String] { lock.withLock { asked } }

    func answer(_ more: [Result<HostResponse, HostError>]) {
        lock.withLock { answers += more }
    }

    func send(request: HostRequest) async throws -> HostResponse {
        let answer: Result<HostResponse, HostError> = lock.withLock {
            asked.append(request.url)
            return answers.isEmpty ? .failure(.Failed(message: "no answer")) : answers.removeFirst()
        }
        return try answer.get()
    }

    func load(namespace: String, key: String) -> String? {
        lock.withLock { stored["\(namespace)/\(key)"] }
    }

    func store(namespace: String, key: String, value: String) {
        lock.withLock { stored["\(namespace)/\(key)"] = value.isEmpty ? nil : value }
    }

    func saveUsage(ledgerJson: String) throws {}
}

/// A clock the tests move, in Unix seconds.
private final class Seconds: @unchecked Sendable {
    private let lock = NSLock()
    private var now: Int64
    init(_ now: Int64) { self.now = now }
    func read() -> Int64 { lock.withLock { now } }
    func advance(_ seconds: Int64) { lock.withLock { now += seconds } }
}

private func response(_ status: UInt16, _ contentType: String?, _ body: Data)
    -> Result<HostResponse, HostError>
{
    .success(
        HostResponse(
            status: status, body: body, contentType: contentType, date: nil, expires: nil,
            lastModified: nil, etag: nil, retryAfter: nil, rateLimitRemaining: nil))
}

/// An ERDDAP time axis in its JSON table form (made by hand, in the shape the server sends).
private func axis(_ days: [String]) -> Result<HostResponse, HostError> {
    let rows = days.map { #"["\#($0)T12:00:00Z"]"# }.joined(separator: ",")
    let json = #"""
        {"table":{"columnNames":["time"],"columnTypes":["String"],"columnUnits":["UTC"],"rows":[\#(rows)]}}
        """#
    return response(200, "application/json;charset=UTF-8", Data(json.utf8))
}

/// The time axes through the pacer and their keeping, and the radar frames and their tiles.
final class SourcesFetchTests: XCTestCase {
    /// 2026-11-17 12:00 UTC.
    private let now: Int64 = 1_794_916_800
    private let days = ["2026-11-12", "2026-11-13", "2026-11-14", "2026-11-15"]

    private func settingsJSON() throws -> String { try Settings.defaults().json() }

    /// The device's store, empty for each test and shared by its launches.
    private let scratch = ScratchStore()

    private func axes(_ host: ListHost, _ clock: Seconds) -> TimeAxes {
        TimeAxes(
            host: host, store: scratch.store, offline: { false }, clock: { clock.read() },
            pause: { _ in })
    }

    func testAnAxisIsAskedForOnceADayAndLabelledByTheCore() async throws {
        let host = ListHost([axis(days)])
        let clock = Seconds(now)
        let axes = axes(host, clock)
        let key = TimeAxes.Key(layer: "chlorophyll", dataset: nil)
        let json = try settingsJSON()
        let times = try await XCTUnwrapAsync(
            await axes.times(key, dates: .daily, settingsJSON: json, timeZone: "America/New_York"))
        XCTAssertEqual(times.count, 4)
        XCTAssertEqual(times.last?.label, "Nov 15")
        XCTAssertEqual(times.first?.time, 1_794_484_800)
        XCTAssertEqual(host.urls.count, 1)
        XCTAssertTrue(host.urls[0].contains("/griddap/noaacwN20VIIRSchlaDaily.json?time"), host.urls[0])

        // Within the day: no new request.
        clock.advance(86_399)
        _ = await axes.times(key, dates: .daily, settingsJSON: json, timeZone: "UTC")
        XCTAssertEqual(host.urls.count, 1)

        // A day later the axis is asked for again and the newer day appears.
        clock.advance(1)
        host.answer([axis(days + ["2026-11-16"])])
        let next = await axes.times(key, dates: .daily, settingsJSON: json, timeZone: "UTC")
        XCTAssertEqual(host.urls.count, 2)
        XCTAssertEqual(next?.last?.label, "Nov 16")
    }

    func testAKeptAxisOutlivesALaunchAndAFailureKeepsTheLastOne() async throws {
        let host = ListHost([axis(days)])
        let clock = Seconds(now)
        let key = TimeAxes.Key(layer: "sst", dataset: nil)
        let json = try settingsJSON()
        _ = await axes(host, clock).times(key, dates: .daily, settingsJSON: json, timeZone: "UTC")
        XCTAssertEqual(host.urls.count, 1)

        // The next launch reads the kept axis without asking.
        let again = axes(host, clock)
        let kept = await again.times(key, dates: .daily, settingsJSON: json, timeZone: "UTC")
        XCTAssertEqual(kept?.count, 4)
        XCTAssertEqual(host.urls.count, 1)

        // Stale, and the server answers with an error page: the last axis stays.
        clock.advance(90_000)
        host.answer([response(500, "text/html", Data("<html>Error</html>".utf8))])
        let stale = await again.times(key, dates: .daily, settingsJSON: json, timeZone: "UTC")
        XCTAssertEqual(host.urls.count, 2)
        XCTAssertEqual(stale?.count, 4)
        // And it is not asked for again at once.
        _ = await again.times(key, dates: .daily, settingsJSON: json, timeZone: "UTC")
        XCTAssertEqual(host.urls.count, 2)
    }

    func testAnHourlyAxisIsAskedForAgainAfterAnHour() async throws {
        let host = ListHost([axis(days), axis(days)])
        let clock = Seconds(now)
        let axes = axes(host, clock)
        let key = TimeAxes.Key(layer: "hf-radar-currents", dataset: nil)
        let json = try settingsJSON()
        _ = await axes.times(key, dates: .hourly, settingsJSON: json, timeZone: "UTC")
        clock.advance(3_600)
        _ = await axes.times(key, dates: .hourly, settingsJSON: json, timeZone: "UTC")
        XCTAssertEqual(host.urls.count, 2)
    }

    func testABusyServerHoldsTheAxisBack() async throws {
        let host = ListHost([
            response(503, "text/html", Data("busy".utf8)), axis(days),
        ])
        let clock = Seconds(now)
        let axes = axes(host, clock)
        let key = TimeAxes.Key(layer: "chlorophyll", dataset: nil)
        let json = try settingsJSON()
        let none = await axes.times(key, dates: .daily, settingsJSON: json, timeZone: "UTC")
        XCTAssertNil(none)
        XCTAssertEqual(host.urls.count, 1)
        // After the rest between tries the pacer, which heard "busy", still decides.
        clock.advance(TimeAxes.retryAfter + 3_600)
        let times = await axes.times(key, dates: .daily, settingsJSON: json, timeZone: "UTC")
        XCTAssertEqual(times?.count, 4)
        XCTAssertEqual(host.urls.count, 2)
    }

    func testAnAxisNotHadOfflineIsAskedForAgainAfterAMinute() async throws {
        // The first try finds no network; nothing is kept.
        let host = ListHost([.failure(.Failed(message: "offline")), axis(days)])
        let clock = Seconds(now)
        let axes = axes(host, clock)
        let key = TimeAxes.Key(layer: "chlorophyll", dataset: nil)
        let json = try settingsJSON()
        let none = await axes.times(key, dates: .daily, settingsJSON: json, timeZone: "UTC")
        XCTAssertNil(none)
        XCTAssertEqual(host.urls.count, 1)

        // Within the minute it is not asked for again.
        clock.advance(TimeAxes.retryAfter - 1)
        _ = await axes.times(key, dates: .daily, settingsJSON: json, timeZone: "UTC")
        XCTAssertEqual(host.urls.count, 1)

        // A minute after the first try it is.
        clock.advance(1)
        let times = await axes.times(key, dates: .daily, settingsJSON: json, timeZone: "UTC")
        XCTAssertEqual(times?.count, 4)
        XCTAssertEqual(host.urls.count, 2)
        XCTAssertEqual(TimeAxes.retryAfter, 60)
    }

    // MARK: - Radar

    func testTheRadarLoopEndsAtTheLatestCompositeWithTheCoresLabels() async throws {
        let meta = #"{"meta":{"valid":"2026-11-17T11:55:00Z"}}"#
        let host = ListHost([response(200, "application/json", Data(meta.utf8))])
        let frames = RadarFrames(host: host, cache: TileMemoryCache())
        let loop = try await XCTUnwrapAsync(
            await frames.frames(settingsJSON: try settingsJSON(), timeZone: "UTC"))
        XCTAssertGreaterThan(loop.count, 1)
        XCTAssertEqual(loop.last?.minutesAgo, 0)
        XCTAssertEqual(loop.last?.valid, 1_794_916_500)
        XCTAssertTrue(loop.allSatisfy { !$0.label.isEmpty })
        XCTAssertEqual(loop.map(\.valid), loop.map(\.valid).sorted())
    }

    func testAFrameTilePastTheRadarsZoomIsFetchedOnceAtItsAncestor() async throws {
        let png = TilePictures.quarters()
        let host = ListHost([response(200, "image/png", png)])
        let frames = RadarFrames(host: host, cache: TileMemoryCache())
        let valid: Int64 = 1_794_916_500
        let a = await frames.tile(valid: valid, z: 12, x: 1206, y: 1539, deepest: 10)
        let b = await frames.tile(valid: valid, z: 12, x: 1207, y: 1539, deepest: 10)
        XCTAssertNotNil(a)
        XCTAssertNotNil(b)
        XCTAssertEqual(host.urls.count, 1)
        XCTAssertTrue(host.urls[0].contains("USCOMP-N0Q-202611171155/10/301/384.png"), host.urls[0])
        // Under the radar's shallowest zoom there is nothing to fetch.
        let none = await frames.tile(valid: valid, z: 3, x: 2, y: 3, deepest: 10)
        XCTAssertNil(none)
        XCTAssertEqual(host.urls.count, 1)
    }
}

/// `XCTUnwrap` for a value an `await` produced.
private func XCTUnwrapAsync<T>(
    _ value: @autoclosure () async throws -> T?, file: StaticString = #filePath, line: UInt = #line
) async throws -> T {
    let v = try await value()
    return try XCTUnwrap(v, file: file, line: line)
}
