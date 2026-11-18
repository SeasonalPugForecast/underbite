import Foundation
import XCTest

@testable import UnderbiteKit

/// What the lookups were sent and waited for; no network.
private final class Sent: @unchecked Sendable {
    private let lock = NSLock()
    private var urls: [String] = []
    private var waits: [Duration] = []
    private var events: [String] = []

    func sent(_ url: String) { lock.withLock { urls.append(url); events.append("send " + url) } }
    func waited(_ d: Duration) { lock.withLock { waits.append(d); events.append("wait") } }
    var all: [String] { lock.withLock { urls } }
    var pauses: [Duration] { lock.withLock { waits } }
    var log: [String] { lock.withLock { events } }
}

private func ok(_ body: String) -> HostResponse {
    HostResponse(
        status: 200, body: Data(body.utf8), contentType: "application/json", date: nil,
        expires: nil, lastModified: nil, etag: nil, retryAfter: nil, rateLimitRemaining: nil)
}

private func request(_ id: String, queue: String, delay: UInt32 = 0) -> NearbyRequest {
    NearbyRequest(
        id: id, source: queue, queue: queue, delayMs: delay,
        request: HostRequest(
            url: "https://example.org/\(id)", headers: [], method: .get, body: nil, fresh: false,
            noStore: false, maxAgeS: nil))
}

/// One boat ramp near The Battery, in the shape Overpass answers (made by hand).
private let overpassRamp = #"""
    {"version":0.6,"elements":[{"type":"node","id":1,"lat":40.7006,"lon":-73.9969,"tags":{"leisure":"slipway","access":"yes"}}]}
    """#
private let overpassEmpty = #"{"version":0.6,"elements":[]}"#
private let padusEmpty = #"{"type":"FeatureCollection","features":[]}"#

/// The lookups' rounds, queues and waits; the kept state and its setting aside.
final class NearbyLookupTests: XCTestCase {
    /// 2026-11-17 15:00 UTC.
    private let now: Int64 = 1_794_927_600
    /// The Battery and the lower Hudson's mouth.
    private let battery = NearbyArea(west: -74.05, south: 40.68, east: -73.99, north: 40.74)

    private let directory = FileManager.default.temporaryDirectory
        .appending(path: "underbite-nearby-\(UUID().uuidString)", directoryHint: .isDirectory)

    override func tearDown() {
        try? FileManager.default.removeItem(at: directory)
    }


    private func settingsJSON() throws -> String { try Settings.defaults().json() }

    func testRequestsAreQueuedInOrderOfFirstAppearance() {
        let queues = NearbyLookup.queues([
            request("1", queue: "padus"), request("2", queue: "overpass"),
            request("3", queue: "padus"), request("4", queue: "overpass"),
        ])
        XCTAssertEqual(queues.map { $0.map(\.id) }, [["1", "3"], ["2", "4"]])
    }

    func testAQueueWaitsItsDelaysAndSkipsWaitsTooLongToSitThrough() async {
        let sent = Sent()
        let long = NearbyLookup.longestDelayMs + 1
        let answers = await NearbyLookup.round(
            [
                request("a", queue: "overpass"), request("b", queue: "overpass", delay: 1_500),
                request("c", queue: "overpass", delay: long),
            ],
            send: { r in
                sent.sent(r.url)
                return ok("{}")
            },
            pause: { sent.waited($0) })
        XCTAssertEqual(answers.map(\.id), ["a", "b"], "the long wait is left unanswered")
        XCTAssertEqual(
            sent.log,
            ["send https://example.org/a", "wait", "send https://example.org/b"],
            "one at a time, the wait before its request")
        XCTAssertEqual(sent.pauses, [.milliseconds(1_500)])
    }

    func testARoundKeepsWhatCameInTimeAndStopsAServiceThatHangs() async {
        let started = Date()
        let answers = await NearbyLookup.round(
            [request("fast", queue: "padus"), request("slow", queue: "overpass")],
            send: { r in
                if r.url.hasSuffix("/slow") {
                    try? await Task.sleep(for: .seconds(20))
                    return Task.isCancelled ? nil : ok("{}")
                }
                return ok("{}")
            },
            pause: { _ in }, deadline: .milliseconds(300))
        XCTAssertEqual(answers.map(\.id), ["fast"])
        XCTAssertLessThan(Date().timeIntervalSince(started), 5, "the hanging request was stopped")
    }

    func testAFailedSendIsLeftUnanswered() async {
        let answers = await NearbyLookup.round(
            [request("a", queue: "padus"), request("b", queue: "gbif")],
            send: { r in r.url.hasSuffix("/a") ? nil : ok("{}") }, pause: { _ in })
        XCTAssertEqual(answers.map(\.id), ["b"])
    }

    @MainActor
    func testAccessRunsToTheEndAndTheNextLookupAnswersOpenStreetMapFromTheState() async throws {
        let store = NearbyStateStore(files: AppFiles(directory: directory))
        let json = try settingsJSON()
        let plan = try planAccess(
            area: battery, includeParking: false, stateJson: "", settingsJson: json, now: now)
        let sent = Sent()
        let firstCell = plan.requests().first { $0.source == "overpass" }?.request.url
        XCTAssertNotNil(firstCell)
        let progress = Sent()
        let at = now
        let results = await NearbyLookup.run(
            plan,
            send: { r in
                sent.sent(r.url)
                if r.url.contains("PADUS") { return ok(padusEmpty) }
                return ok(r.url == firstCell ? overpassRamp : overpassEmpty)
            },
            pause: { _ in }, clock: { at },
            progress: { _ in progress.sent("round") })
        XCTAssertTrue(plan.requests().isEmpty, "nothing left to ask")
        XCTAssertFalse(progress.all.isEmpty)
        XCTAssertEqual(sent.all.filter { $0.contains("PADUS") }.count, 1)
        let ramps = results.access.filter { $0.kind == "ramp" }
        XCTAssertEqual(ramps.count, 1)
        XCTAssertEqual(ramps.first?.access, "open")
        XCTAssertEqual(ramps.first?.kindLabel, "Boat ramp")
        XCTAssertTrue(
            results.credits.contains { $0.source == "overpass" && $0.text.contains("OpenStreetMap") })
        XCTAssertTrue(results.notes.contains { $0.contains("not as permission") })
        let state = try XCTUnwrap(results.stateJson)
        XCTAssertTrue(store.save(state, base: ""))
        XCTAssertEqual(store.load(), state, "with nothing kept, kept as the core wrote it")

        // An hour later the same view asks PAD-US only; the ramp comes from the kept state.
        let (later, base) = try store.plan { kept in
            try planAccess(
                area: battery, includeParking: false, stateJson: kept, settingsJson: json,
                now: now + 3_600)
        }
        XCTAssertFalse(base.isEmpty)
        XCTAssertEqual(later.requests().map(\.source), ["padus"])
        let cached = later.results()
        XCTAssertEqual(cached.access.filter { $0.kind == "ramp" }.count, 1)
        XCTAssertTrue(
            cached.notices.contains { $0.contains("saved on this phone") }, "\(cached.notices)")
    }

    func testAViewTooWideForTheLookupSaysZoomIn() throws {
        let wide = NearbyArea(west: -75, south: 40, east: -73, north: 41.5)
        XCTAssertThrowsError(
            try planAccess(
                area: wide, includeParking: false, stateJson: "", settingsJson: settingsJSON(),
                now: now)
        ) { error in
            guard case NearbyError.ZoomIn(let message) = error else {
                return XCTFail("\(error)")
            }
            XCTAssertFalse(message.isEmpty)
        }
        let words = try nearbyWords(settingsJson: settingsJSON())
        XCTAssertFalse(words.findAccess.isEmpty)
        XCTAssertNotEqual(words.findAccess, words.findingAccess)
    }
}
