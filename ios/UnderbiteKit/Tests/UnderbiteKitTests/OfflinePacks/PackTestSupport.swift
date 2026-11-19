import Foundation
import XCTest

@testable import UnderbiteKit

/// 2026-11-15T12:00:00Z.
let packNow: Int64 = 1_794_744_000

/// A host that answers with a function, counts what it was asked and how many requests were in
/// flight to each server at once; no network.
final class PackTestHost: FetchHost, @unchecked Sendable {
    typealias Answer = @Sendable (_ request: HostRequest, _ call: Int) async throws -> HostResponse

    private let lock = NSLock()
    private var asked: [String] = []
    private var inFlight: [String: Int] = [:]
    private var most: [String: Int] = [:]
    private var values: [String: String] = [:]
    private let answer: Answer
    private let delay: Duration

    /// `answer` gets the request and how many times its URL was asked before (0 the first
    /// time); each request is held for `delay` first.
    init(delay: Duration = .zero, answer: @escaping Answer) {
        self.delay = delay
        self.answer = answer
    }

    var urls: [String] { lock.withLock { asked } }
    /// The most requests that were in flight at once, by server.
    var mostInFlight: [String: Int] { lock.withLock { most } }

    func send(request: HostRequest) async throws -> HostResponse {
        let server = URL(string: request.url)?.host ?? ""
        let call: Int = lock.withLock {
            let before = asked.filter { $0 == request.url }.count
            asked.append(request.url)
            inFlight[server, default: 0] += 1
            most[server] = max(most[server] ?? 0, inFlight[server] ?? 0)
            return before
        }
        defer { lock.withLock { inFlight[server, default: 1] -= 1 } }
        if delay > .zero { try await Task.sleep(for: delay) }
        return try await answer(request, call)
    }

    func load(namespace: String, key: String) -> String? {
        lock.withLock { values["\(namespace)/\(key)"] }
    }

    func store(namespace: String, key: String, value: String) {
        lock.withLock { values["\(namespace)/\(key)"] = value.isEmpty ? nil : value }
    }

    func saveUsage(ledgerJson: String) throws {}
}

/// The waits a download asked for, in seconds; each moves the clock on instead of waiting.
final class PauseLog: @unchecked Sendable {
    private let lock = NSLock()
    private var waits: [UInt32] = []
    var all: [UInt32] { lock.withLock { waits } }
    func add(_ seconds: UInt32) { lock.withLock { waits.append(seconds) } }
}

/// A path the tests switch.
final class RouteBox: @unchecked Sendable {
    private let lock = NSLock()
    private var route: PackRoute
    init(_ route: PackRoute = .unmetered) { self.route = route }
    var value: PackRoute { lock.withLock { route } }
    func set(_ route: PackRoute) { lock.withLock { self.route = route } }
}

/// Collects a download's events.
final class EventLog: @unchecked Sendable {
    private let lock = NSLock()
    private var events: [PackEvent] = []
    var all: [PackEvent] { lock.withLock { events } }
    func add(_ e: PackEvent) { lock.withLock { events.append(e) } }

    var waits: [Bool] {
        all.compactMap {
            if case .waitingForWifi(let w) = $0 { return w }
            return nil
        }
    }

    var committed: [PackProgress] {
        all.compactMap {
            if case .committed(let p) = $0 { return p }
            return nil
        }
    }
}

/// A download context over `host` and `store` whose waits move `clock`; `onPause` runs at
/// each wait (a test flips the route there).
func packContext(
    host: FetchHost, store: OfflineStore, clock: MovingClock, route: RouteBox = RouteBox(),
    pacer: GridPacer = GridPacer(), pauses: PauseLog = PauseLog(),
    onPause: @escaping @Sendable (UInt32) -> Void = { _ in }
) -> PackContext {
    PackContext(
        host: host, store: store, pacer: pacer, clock: { clock.seconds },
        pause: { seconds in
            pauses.add(seconds)
            clock.advance(Int64(seconds))
            onPause(seconds)
            await Task.yield()
        },
        route: { route.value })
}

/// A PNG-looking tile body.
let tileBody = Data([0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A] + Array(repeating: 7, count: 120))

func tileAnswer(_ status: UInt16 = 200, type: String = "image/png", body: Data = tileBody)
    -> HostResponse
{
    HostResponse(
        status: status, body: body, contentType: type, date: nil, expires: nil,
        lastModified: nil, etag: nil, retryAfter: nil, rateLimitRemaining: nil)
}

/// The settings the tests plan with: the defaults.
func packSettings() throws -> String { defaultSettingsJson() }

/// A small NOAA chart pack around The Battery, zooms 12 to 13, of the newest data.
func batteryChartPack(id: String = "battery-chart", settings: String) throws -> String {
    try packNew(
        id: id, name: "The Battery",
        area: PackBox(west: -74.03, south: 40.695, east: -74.0, north: 40.715),
        minZoom: 12, maxZoom: 13, layers: ["noaa-chart"], dates: .latest,
        spotIds: ["seed-battery"], settingsJson: settings, now: packNow)
}

/// A pack of the MUR SST grid off New York, of the newest data.
func sstPack(settings: String) throws -> String {
    try packNew(
        id: "sst-off-ny", name: "Canyons",
        area: PackBox(west: -74.0, south: 39.0, east: -71.0, north: 41.0),
        minZoom: 6, maxZoom: 10, layers: ["sst"], dates: .latest, spotIds: [],
        settingsJson: settings, now: packNow)
}
