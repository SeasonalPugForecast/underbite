import Foundation
import XCTest

@testable import UnderbiteKit

/// A background refresh: the rules checked through the alert check, their notifications posted,
/// the widgets' spots fetched whole once, every payload written and the widgets reloaded once;
/// the fetch budget, and a cancellation that leaves the files as they were.
final class BackgroundRefreshTests: XCTestCase {
    private let root = FileManager.default.temporaryDirectory
        .appending(path: "underbite-background-\(UUID().uuidString)", directoryHint: .isDirectory)

    override func tearDown() {
        try? FileManager.default.removeItem(at: root)
    }

    private var files: AppFiles {
        AppFiles(directory: root.appending(path: "app", directoryHint: .isDirectory))
    }

    private var payloads: WidgetPayloadStore {
        WidgetPayloadStore(
            container: SharedContainer(
                url: root.appending(path: "widgets", directoryHint: .isDirectory),
                backing: .temporary))
    }

    /// Just after the sample's water temperature (14.6 °C), while it is fresh.
    private let now = SampleSnapshot.fetchedAt + 400

    /// The sample snapshot, as a fetch of spot `id` returns it.
    private static func snapshot(of id: String) -> String {
        SampleSnapshot.json.replacingOccurrences(
            of: #"{"spot":{"id":"battery""#, with: #"{"spot":{"id":"\#(id)""#)
    }

    /// The Battery under 20 °C.
    private let rule = AlertRuleDocument(
        id: "cool-battery", name: "Cool at the Battery", target: .spot(SeedSpots.battery.id),
        allOf: [
            AlertRuleDocument.Condition(kind: "measurement", metric: "water_temperature", below: 20)
        ])

    private func settings(_ rules: [AlertRuleDocument]) throws -> String {
        try rules.reduce(defaultSettingsJson()) { json, rule in
            try putAlertRule(settingsJson: json, ruleJson: rule.json())
        }
    }

    /// What a refresh asked for and did.
    private final class Recorder: @unchecked Sendable {
        private let lock = NSLock()
        private var alertFetches: [String] = []
        private var spotFetches: [String] = []
        private var posts: [[AlertNotice]] = []
        private var reloads = 0
        private var held: [CheckedContinuation<Void, Never>] = []
        /// Fetches wait until released (they ignore cancellation, as the core's do).
        var holdFetches = false
        var budget = FetchBudget(mayStartFor: .seconds(60), waitFor: .seconds(60))

        var alert: [String] { lock.withLock { alertFetches } }
        var whole: [String] { lock.withLock { spotFetches } }
        var posted: [[AlertNotice]] { lock.withLock { posts } }
        var reloaded: Int { lock.withLock { reloads } }
        var waiting: Int { lock.withLock { held.count } }

        func release() {
            let all = lock.withLock {
                defer { held = [] }
                return held
            }
            for continuation in all { continuation.resume() }
        }

        private func hold() async {
            guard lock.withLock({ holdFetches }) else { return }
            await withCheckedContinuation { continuation in
                lock.withLock { held.append(continuation) }
            }
        }

        func refresh(
            files: AppFiles, payloads: WidgetPayloadStore,
            snapshot: @escaping @Sendable (String) -> String
        ) -> BackgroundRefresh {
            BackgroundRefresh(
                alertFetch: { fetch, _, _ in
                    self.lock.withLock { self.alertFetches.append(fetch.spot.id) }
                    await self.hold()
                    return SpotFetchOutcome(
                        snapshotJson: snapshot(fetch.spot.id), errors: [], suggestion: nil)
                },
                spotFetch: { spot, _, _ in
                    self.lock.withLock { self.spotFetches.append(spot.id) }
                    await self.hold()
                    return SpotFetchOutcome(
                        snapshotJson: snapshot(spot.id), errors: [], suggestion: nil)
                },
                gridSend: { _ in throw URLError(.notConnectedToInternet) },
                post: { notices in self.lock.withLock { self.posts.append(notices) } },
                payloads: payloads,
                reloadWidgets: { self.lock.withLock { self.reloads += 1 } },
                files: files,
                budget: lock.withLock { budget })
        }
    }

    private func input(
        settings: String, spots: [Spot] = [SeedSpots.battery, SeedSpots.croton],
        trips: [SavedTrip] = [], widgets: [WidgetNeeds.Installed] = []
    ) -> BackgroundRefresh.Input {
        BackgroundRefresh.Input(
            settingsJSON: settings, spots: spots, appSpotID: spots.first?.id, trips: trips,
            widgets: widgets, timeZone: "America/New_York", now: now)
    }

    private func refresh(_ recorder: Recorder) -> BackgroundRefresh {
        recorder.refresh(files: files, payloads: payloads) { Self.snapshot(of: $0) }
    }

    func testARuleIsCheckedPostedAndEveryWidgetWrittenWithOneReload() async throws {
        let recorder = Recorder()
        let outcome = await refresh(recorder).run(input(settings: try settings([rule])))
        // Only what the rule reads, at the spot it watches.
        XCTAssertEqual(recorder.alert, [SeedSpots.battery.id])
        XCTAssertEqual(recorder.whole, [])
        XCTAssertEqual(outcome.fetched, [SeedSpots.battery.id])
        // Fresh water under 20 °C: one notification, posted once, in the core's words.
        XCTAssertEqual(outcome.notices.map(\.ruleId), ["cool-battery"])
        XCTAssertFalse(outcome.notices[0].title.isEmpty)
        XCTAssertFalse(outcome.notices[0].timeSensitive)
        XCTAssertEqual(recorder.posted, [outcome.notices])
        XCTAssertNotNil(AlertStateFile(files: files).read())
        // The Battery's payload from the fresh snapshot; the Croton has none to build from.
        XCTAssertEqual(outcome.payloads, [SeedSpots.battery.id])
        XCTAssertEqual(payloads.spot(id: SeedSpots.battery.id)?.id, SeedSpots.battery.id)
        XCTAssertEqual(
            payloads.spots().spots.map(\.id), [SeedSpots.battery.id, SeedSpots.croton.id])
        XCTAssertEqual(payloads.spots().appSpotID, SeedSpots.battery.id)
        XCTAssertNil(payloads.nextTrip())
        XCTAssertEqual(recorder.reloaded, 1)
        XCTAssertTrue(outcome.reloaded)
        XCTAssertFalse(outcome.cancelled)

        // Still holding at the next refresh: told once, not again.
        let again = await refresh(recorder).run(input(settings: try settings([rule])))
        XCTAssertEqual(again.notices, [])
        XCTAssertEqual(recorder.posted.count, 1)
        XCTAssertEqual(recorder.reloaded, 2)
    }

    func testWithoutRulesOrWidgetsNothingIsFetched() async throws {
        let recorder = Recorder()
        files.write(
            SpotCache(snapshotJSON: Self.snapshot(of: SeedSpots.croton.id), lastSuccess: now),
            files.cacheName(SeedSpots.croton.id))
        let outcome = await refresh(recorder).run(input(settings: defaultSettingsJson()))
        XCTAssertEqual(recorder.alert + recorder.whole, [])
        XCTAssertEqual(recorder.posted.count, 0)
        // The payloads are built again from the kept snapshots, so their hours move on.
        XCTAssertEqual(outcome.payloads, [SeedSpots.croton.id])
        XCTAssertEqual(recorder.reloaded, 1)
    }

    func testAWidgetsSpotIsFetchedWholeAndOnlyOnce() async throws {
        let recorder = Recorder()
        // A widget set to the Croton, one following the app's spot (the Battery, which the
        // rule watches too), and one of a kind this version does not know (the app's spot).
        let widgets = [
            WidgetNeeds.Installed(kind: .flow, choice: .spot(SeedSpots.croton.id)),
            WidgetNeeds.Installed(kind: .tide),
            WidgetNeeds.Installed(kind: nil),
        ]
        let outcome = await refresh(recorder).run(
            input(settings: try settings([rule]), spots: SeedSpots.all, widgets: widgets))
        // The Battery's rule fetch is the widget's whole fetch; the Croton follows.
        XCTAssertEqual(recorder.alert, [])
        XCTAssertEqual(recorder.whole, [SeedSpots.battery.id, SeedSpots.croton.id])
        XCTAssertEqual(outcome.fetchedWhole, [SeedSpots.battery.id, SeedSpots.croton.id])
        XCTAssertEqual(outcome.notices.map(\.ruleId), ["cool-battery"])
        // The buoy has no snapshot and no widget: not fetched, no payload.
        XCTAssertEqual(outcome.payloads, [SeedSpots.battery.id, SeedSpots.croton.id])
        XCTAssertEqual(recorder.reloaded, 1)
    }

    func testTheNextTripsWidgetFetchesItsPlacesAndItsPayloadIsWritten() async throws {
        let recorder = Recorder()
        let spots = [SeedSpots.battery, SeedSpots.croton]
        let today = TripDates.day(
            of: Date(timeIntervalSince1970: TimeInterval(now)),
            in: TimeZone(identifier: "America/New_York") ?? .gmt)
        let trip = SavedTrip(id: TripSeeds.id, json: TripSeeds.trip(today: today, spots: spots))
        let outcome = await refresh(recorder).run(
            input(
                settings: defaultSettingsJson(), spots: spots, trips: [trip],
                widgets: [WidgetNeeds.Installed(kind: .nextTrip)]))
        XCTAssertEqual(recorder.whole, [SeedSpots.battery.id, SeedSpots.croton.id])
        XCTAssertEqual(outcome.nextTrip, TripSeeds.id)
        XCTAssertEqual(payloads.nextTrip()?.id, TripSeeds.id)

        // The trip deleted: its payload goes, and the widget shows its placeholder.
        let after = await refresh(recorder).run(
            input(settings: defaultSettingsJson(), spots: spots))
        XCTAssertNil(after.nextTrip)
        XCTAssertNil(payloads.nextTrip())
    }

    func testADeletedSpotsPayloadIsRemoved() async throws {
        let recorder = Recorder()
        try payloads.saveSpot(Fixtures.text("sample-widget-croton"))
        _ = await refresh(recorder).run(
            input(settings: defaultSettingsJson(), spots: [SeedSpots.battery]))
        XCTAssertEqual(payloads.spotIDs(), [])
        XCTAssertEqual(payloads.spots().spots.map(\.id), [SeedSpots.battery.id])
    }

    func testNoFetchStartsOnceTheBudgetIsSpentAndTheCheckStillRuns() async throws {
        let recorder = Recorder()
        recorder.budget = FetchBudget(mayStartFor: .zero, waitFor: .zero)
        files.write(
            SpotCache(snapshotJSON: Self.snapshot(of: SeedSpots.battery.id), lastSuccess: now),
            files.cacheName(SeedSpots.battery.id))
        let outcome = await refresh(recorder).run(
            input(
                settings: try settings([rule]),
                widgets: [WidgetNeeds.Installed(kind: .flow, choice: .spot(SeedSpots.croton.id))]))
        XCTAssertEqual(recorder.alert + recorder.whole, [])
        XCTAssertEqual(outcome.fetched, [])
        // Evaluated with the kept snapshot; the state kept, the widgets written and reloaded.
        XCTAssertNil(outcome.checkProblem)
        XCTAssertNotNil(AlertStateFile(files: files).read())
        XCTAssertEqual(outcome.payloads, [SeedSpots.battery.id])
        XCTAssertEqual(recorder.reloaded, 1)
        XCTAssertFalse(outcome.cancelled)
    }

    func testAFetchStillUnderWayWhenTheBudgetEndsIsNotWaitedFor() async throws {
        let recorder = Recorder()
        recorder.holdFetches = true
        recorder.budget = FetchBudget(mayStartFor: .milliseconds(300), waitFor: .milliseconds(300))
        defer { recorder.release() }
        files.write(
            SpotCache(snapshotJSON: Self.snapshot(of: SeedSpots.battery.id), lastSuccess: now),
            files.cacheName(SeedSpots.battery.id))
        let outcome = await refresh(recorder).run(input(settings: try settings([rule])))
        // The held fetch was dropped; the rule was checked against the kept snapshot.
        XCTAssertEqual(recorder.alert, [SeedSpots.battery.id])
        XCTAssertEqual(recorder.waiting, 1)
        XCTAssertEqual(outcome.fetched, [])
        XCTAssertNil(outcome.checkProblem)
        XCTAssertNotNil(AlertStateFile(files: files).read())
        XCTAssertEqual(outcome.payloads, [SeedSpots.battery.id])
        XCTAssertEqual(recorder.reloaded, 1)
        XCTAssertFalse(outcome.cancelled)
    }

    func testCancellingDuringAFetchReturnsAtOnceAndLeavesTheFilesAsTheyWere() async throws {
        let recorder = Recorder()
        recorder.holdFetches = true
        defer { recorder.release() }
        try payloads.saveSpot(Fixtures.text("sample-widget-croton"))
        let refresher = refresh(recorder)
        let given = input(settings: try settings([rule]))
        let task = Task { await refresher.run(given) }
        while recorder.waiting == 0 { try await Task.sleep(for: .milliseconds(10)) }
        task.cancel()
        // The fetch is still held: the refresh did not wait for it.
        let outcome = await task.value
        XCTAssertEqual(recorder.waiting, 1)
        XCTAssertTrue(outcome.cancelled)
        XCTAssertEqual(outcome.notices, [])
        XCTAssertEqual(recorder.posted.count, 0)
        XCTAssertEqual(recorder.reloaded, 0)
        // Nothing evaluated, nothing written: the next refresh starts from the same files.
        XCTAssertNil(AlertStateFile(files: files).read())
        XCTAssertEqual(payloads.spotIDs(), ["croton"])
        XCTAssertEqual(payloads.spots(), WidgetSpots())
    }

    func testABrokenRuleSetStillUpdatesTheWidgets() async throws {
        let recorder = Recorder()
        files.write(
            SpotCache(snapshotJSON: Self.snapshot(of: SeedSpots.battery.id), lastSuccess: now),
            files.cacheName(SeedSpots.battery.id))
        files.write(Data("not a state".utf8), AlertStateFile.name)
        // The core cannot read the state: it is set aside and the check starts over.
        let outcome = await refresh(recorder).run(input(settings: try settings([rule])))
        XCTAssertNil(outcome.checkProblem)
        XCTAssertEqual(files.text(AlertStateFile.name + ".bad"), "not a state")
        XCTAssertEqual(outcome.payloads, [SeedSpots.battery.id])
        XCTAssertEqual(recorder.reloaded, 1)
    }
}

/// Waiting for work that ignores cancellation only until the task is cancelled.
final class AbandoningTests: XCTestCase {
    func testTheWorksResultComesBack() async throws {
        let value = try await Abandoning.run { 42 }
        XCTAssertEqual(value, 42)
        do {
            _ = try await Abandoning.run { () async throws -> Int in throw URLError(.timedOut) }
            XCTFail("no error")
        } catch let error as URLError {
            XCTAssertEqual(error.code, .timedOut)
        }
    }

    func testACancelledWaitEndsBeforeTheWork() async throws {
        let started = Flag(), release = Flag()
        let task = Task {
            try await Abandoning.run { () async -> Int in
                started.set()
                // Ignores cancellation, as a fetch through the core does.
                while !release.isSet { try? await Task.sleep(for: .milliseconds(5)) }
                return 1
            }
        }
        while !started.isSet { try await Task.sleep(for: .milliseconds(5)) }
        task.cancel()
        do {
            _ = try await task.value
            XCTFail("not cancelled")
        } catch is CancellationError {}
        release.set()
    }

    func testWorkIsNotStartedOnceCancelled() async throws {
        let started = Flag()
        let task = Task {
            withUnsafeCurrentTask { $0?.cancel() }
            return try await Abandoning.run { () async -> Int in
                started.set()
                return 1
            }
        }
        do {
            _ = try await task.value
            XCTFail("not cancelled")
        } catch is CancellationError {}
        try await Task.sleep(for: .milliseconds(50))
        XCTAssertFalse(started.isSet)
    }

    private final class Flag: @unchecked Sendable {
        private let lock = NSLock()
        private var value = false
        var isSet: Bool { lock.withLock { value } }
        func set() { lock.withLock { value = true } }
    }
}
