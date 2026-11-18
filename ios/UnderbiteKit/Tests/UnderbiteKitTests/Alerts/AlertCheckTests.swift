import Foundation
import XCTest

@testable import UnderbiteKit

/// "Check now": the core's plan, the fetches and grid requests it asks for, the evaluation
/// against the kept state, and the state file it keeps (set aside when it cannot be read).
final class AlertCheckTests: XCTestCase {
    private let directory = FileManager.default.temporaryDirectory
        .appending(path: "underbite-alerts-\(UUID().uuidString)", directoryHint: .isDirectory)

    override func tearDown() {
        try? FileManager.default.removeItem(at: directory)
    }

    private var files: AppFiles { AppFiles(directory: directory) }
    private var stateFile: AlertStateFile { AlertStateFile(files: files) }
    private let now: Int64 = 1_800_000_000

    private func settings(with rules: [AlertRuleDocument]) throws -> String {
        var json = defaultSettingsJson()
        for rule in rules {
            json = try putAlertRule(settingsJson: json, ruleJson: try rule.json())
        }
        return json
    }

    /// Records what a check asks for; every fetch fails as when the phone is offline, every
    /// grid request answers 503.
    private final class Recorder: @unchecked Sendable {
        private let lock = NSLock()
        private var fetches: [AlertFetch] = []
        private var grids: [HostRequest] = []

        var fetched: [AlertFetch] { lock.withLock { fetches } }
        var sent: [HostRequest] { lock.withLock { grids } }

        func check() -> AlertCheck {
            AlertCheck(
                spotFetch: { fetch, _, _ in
                    self.lock.withLock { self.fetches.append(fetch) }
                    throw URLError(.notConnectedToInternet)
                },
                gridSend: { request in
                    self.lock.withLock { self.grids.append(request) }
                    return HostResponse(
                        status: 503, body: Data(), contentType: nil, date: nil, expires: nil,
                        lastModified: nil, etag: nil, retryAfter: nil, rateLimitRemaining: nil)
                })
        }
    }

    private func rows(_ settings: String) throws -> [AlertRuleRow] {
        try stateFile.using { state in
            try alertRules(
                settingsJson: settings,
                spots: [NamedPlace(id: SeedSpots.croton.id, name: "Croton River")], trips: [],
                state: state, timeZone: "America/New_York", now: now)
        }
    }

    func testACheckFetchesTheRulesSpotAndKeepsWhatItFound() async throws {
        let settings = try settings(with: [AlertSeeds.rule])
        let recorder = Recorder()
        XCTAssertEqual(try rows(settings).first?.state, .notChecked)
        let outcome = try await recorder.check().run(
            settingsJSON: settings, spots: [SeedSpots.croton, SeedSpots.battery], trips: [],
            lastSnapshot: { _ in nil }, state: stateFile, timeZone: "America/New_York", now: now)
        // Only the spot a rule watches, asking for what its conditions read.
        XCTAssertEqual(recorder.fetched.map(\.spot.id), [SeedSpots.croton.id])
        XCTAssertEqual(recorder.fetched.first?.rules, [AlertSeeds.id])
        XCTAssertTrue(recorder.fetched.first?.measurements.contains("discharge") ?? false)
        XCTAssertTrue(recorder.sent.isEmpty)
        // No data, no notification; the state is kept and the list says what the check found.
        XCTAssertEqual(outcome.run.notifications, [])
        XCTAssertNotNil(stateFile.read())
        let row = try XCTUnwrap(try rows(settings).first)
        XCTAssertNotEqual(row.state, .notChecked)
        XCTAssertNotNil(row.checked)
    }

    func testAnOffRuleIsNotFetched() async throws {
        var rule = AlertSeeds.rule
        rule.enabled = false
        let settings = try settings(with: [rule])
        let recorder = Recorder()
        _ = try await recorder.check().run(
            settingsJSON: settings, spots: [SeedSpots.croton], trips: [],
            lastSnapshot: { _ in nil }, state: stateFile, timeZone: "America/New_York", now: now)
        XCTAssertTrue(recorder.fetched.isEmpty)
        XCTAssertEqual(try rows(settings).first?.state, .off)
    }

    func testABoxRuleSendsItsGridRequest() async throws {
        let box = AlertRuleDocument(
            id: "box", name: "Warm water",
            target: .box(south: 40.2, west: -73.9, north: 40.6, east: -73.4),
            allOf: [
                AlertRuleDocument.Condition(kind: "measurement", metric: "sea_surface_temp", above: 20)
            ])
        let settings = try settings(with: [box])
        let recorder = Recorder()
        let outcome = try await recorder.check().run(
            settingsJSON: settings, spots: [], trips: [], lastSnapshot: { _ in nil },
            state: stateFile, timeZone: "America/New_York", now: now)
        XCTAssertTrue(recorder.fetched.isEmpty)
        XCTAssertEqual(recorder.sent.count, 1)
        XCTAssertEqual(outcome.run.notifications, [])
    }

    func testTheStateIsKeptAndReadBack() {
        XCTAssertNil(stateFile.read())
        XCTAssertTrue(stateFile.write(Data("{}".utf8)))
        XCTAssertEqual(stateFile.read(), Data("{}".utf8))
    }

    func testAStateTheCoreCannotReadIsSetAside() throws {
        let settings = try settings(with: [AlertSeeds.rule])
        files.write(Data("not a state".utf8), AlertStateFile.name)
        var given: [Data?] = []
        let rows = try stateFile.using { state -> [AlertRuleRow] in
            given.append(state)
            return try alertRules(
                settingsJson: settings, spots: [], trips: [], state: state,
                timeZone: "America/New_York", now: now)
        }
        XCTAssertEqual(rows.count, 1)
        XCTAssertEqual(given, [Data("not a state".utf8), nil])
        XCTAssertFalse(files.exists(AlertStateFile.name))
        XCTAssertEqual(files.text(AlertStateFile.name + ".bad"), "not a state")
    }
}
