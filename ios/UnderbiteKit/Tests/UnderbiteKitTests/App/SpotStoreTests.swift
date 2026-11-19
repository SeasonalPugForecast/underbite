import Foundation
import XCTest

@testable import UnderbiteKit

/// A clock and a call counter the tests share with the store's `@Sendable` closures.
private final class Box<Value>: @unchecked Sendable {
    private let lock = NSLock()
    private var held: Value
    init(_ value: Value) { held = value }
    var value: Value {
        get { lock.withLock { held } }
        set { lock.withLock { held = newValue } }
    }
}

/// The spot store: ordering, files, settings, the refresh and its metered-call count. No network:
/// the real fetch runs against a stubbed `URLSession`.
@MainActor
final class SpotStoreTests: XCTestCase {
    private var directory: URL!

    override func setUp() {
        directory = FileManager.default.temporaryDirectory
            .appending(path: "underbite-store-\(UUID().uuidString)", directoryHint: .isDirectory)
        StubProtocol.answer(.failure(URLError(.notConnectedToInternet)))
    }

    override func tearDown() {
        try? FileManager.default.removeItem(at: directory)
    }

    nonisolated(unsafe) private static let noKeys = Keys(usgs: nil, copernicusUser: nil, copernicusPassword: nil)

    private var files: AppFiles { AppFiles(directory: directory) }

    private func store(
        now: Box<Int64> = Box(1_800_000_000), lowPower: Bool = false,
        fetcher: SpotStore.Fetcher? = nil
    ) -> SpotStore {
        let host = URLSessionHost(session: StubProtocol.session(), storeURL: files.hostDirectory)
        return SpotStore(
            files: files, host: host, keys: { Self.noKeys }, clock: { now.value }, lowPower: { lowPower },
            fetcher: fetcher ?? { spot, settings, keys, options, at, host in
                try await fetchSpot(
                    spot: spot, settingsJson: settings, keys: keys, options: options, now: at,
                    host: host)
            })
    }

    private func outcome(_ snapshot: String = "{}") -> SpotFetchOutcome {
        SpotFetchOutcome(snapshotJson: snapshot, errors: [], suggestion: nil)
    }

    // MARK: - Ordering and persistence

    func testPinnedSpotsComeFirstThenTheUsersOrderThenName() {
        let spots = [
            Spot(name: "Zed", latitude: 0, longitude: 0, waterType: "pond", order: 0),
            Spot(name: "Beta", latitude: 0, longitude: 0, waterType: "pond", order: 1),
            Spot(name: "Alpha", latitude: 0, longitude: 0, waterType: "pond", order: 1),
            Spot(name: "Pin", latitude: 0, longitude: 0, waterType: "pond", pinned: true, order: 9),
        ]
        XCTAssertEqual(Spot.arranged(spots).map(\.name), ["Pin", "Zed", "Alpha", "Beta"])
    }

    func testAddedSpotsGoLastAndReorderingSticks() {
        let s = store()
        s.add(Spot(id: "a", name: "A", latitude: 1, longitude: 2, waterType: "pond"))
        s.add(Spot(id: "b", name: "B", latitude: 1, longitude: 2, waterType: "pond"))
        s.add(Spot(id: "c", name: "C", latitude: 1, longitude: 2, waterType: "pond"))
        XCTAssertEqual(s.arranged.map(\.id), ["a", "b", "c"])
        s.reorder(["c", "a", "b"])
        XCTAssertEqual(s.arranged.map(\.id), ["c", "a", "b"])
        s.setPinned("b", true)
        XCTAssertEqual(s.arranged.map(\.id), ["b", "c", "a"])
    }

    func testTheStateSurvivesARestartWithExactCoordinates() async {
        let s = store()
        let area = s.addArea(named: "Upper Hudson")
        s.add(
            Spot(
                id: "a", name: "A", latitude: 41.2250277777778, longitude: -73.8588888888889,
                waterType: "river_stream"))
        s.assign("a", toArea: area.id)
        s.setFavourite("a", true)
        s.setRefreshMinutes(30)
        await s.refresh()

        let again = store()
        XCTAssertEqual(again.spots, s.spots)
        XCTAssertEqual(again.areas, s.areas)
        XCTAssertEqual(again.activeID, "a")
        XCTAssertEqual(again.refreshMinutes, 30)
        XCTAssertEqual(again.spots.first?.latitude, 41.2250277777778)
        XCTAssertEqual(again.cache("a"), s.cache("a"))
    }

    func testARemovedSpotLosesItsCacheFile() async {
        let s = store(fetcher: { _, _, _, _, _, _ in SpotFetchOutcome(snapshotJson: "{}", errors: [], suggestion: nil) })
        s.add(Spot(id: "a", name: "A", latitude: 1, longitude: 2, waterType: "pond"))
        await s.refresh()
        let file = files.url(files.cacheName("a"))
        XCTAssertTrue(FileManager.default.fileExists(atPath: file.path))
        s.remove("a")
        XCTAssertFalse(FileManager.default.fileExists(atPath: file.path))
        XCTAssertNil(s.activeID)
    }

    func testTheHostStoreIsInTheDurableDirectory() {
        XCTAssertEqual(files.hostDirectory.deletingLastPathComponent(), directory)
        XCTAssertTrue(AppFiles.standard.directory.path.contains("Application Support"))
    }

    // MARK: - Settings

    private func sampleSettings() throws -> String {
        let url = try XCTUnwrap(
            Bundle.module.url(
                forResource: "sample-settings", withExtension: "json", subdirectory: "Resources"))
        return try String(contentsOf: url, encoding: .utf8)
    }

    func testSettingsAreMigratedOnEveryLoad() throws {
        let sample = try sampleSettings()
        files.write(Data(sample.utf8), "settings.json")
        let s = store()
        let migrated = try migrateSettings(json: sample)
        XCTAssertEqual(s.settingsJSON, migrated.json)
        // Migrated settings are what is on disk afterwards.
        XCTAssertEqual(files.text("settings.json"), migrated.json)
        // Loading the migrated file changes nothing.
        XCTAssertEqual(store().settingsJSON, migrated.json)
    }

    func testOldSettingsGainTheirDefaultsAndUnreadableOnesLeaveTheDefaults() throws {
        files.write(Data(#"{"version": 1}"#.utf8), "settings.json")
        XCTAssertEqual(store().settingsJSON, try migrateSettings(json: #"{"version": 1}"#).json)
        files.write(Data("not json".utf8), "settings.json")
        XCTAssertEqual(store().settingsJSON, defaultSettingsJson())
    }

    func testChangedSettingsAreSavedAfterADebounceOrAtOnce() throws {
        let s = store()
        let sample = try sampleSettings()
        try s.updateSettings(sample)
        XCTAssertNil(files.text("settings.json"))
        s.saveSettingsNow()
        XCTAssertEqual(files.text("settings.json"), s.settingsJSON)
        XCTAssertThrowsError(try s.updateSettings("not json"))
    }

    // MARK: - Refresh

    func testRefreshIsDueByTheClock() async {
        let now = Box<Int64>(1_800_000_000)
        let calls = Box(0)
        let fetcher: SpotStore.Fetcher = { _, _, _, _, _, _ in
            calls.value += 1
            return SpotFetchOutcome(snapshotJson: "{}", errors: [], suggestion: nil)
        }
        let s = store(now: now, fetcher: fetcher)
        s.add(Spot(id: "a", name: "A", latitude: 1, longitude: 2, waterType: "pond"))
        s.add(Spot(id: "b", name: "B", latitude: 3, longitude: 4, waterType: "pond"))

        await s.refreshOnOpen()
        XCTAssertEqual(calls.value, 2, "never fetched: both are due")
        XCTAssertEqual(s.cache("a").lastSuccess, 1_800_000_000)

        now.value += 5 * 60
        await s.refreshOnOpen()
        XCTAssertEqual(calls.value, 2, "five minutes is under ten")

        now.value += 6 * 60
        await s.refreshOnOpen()
        XCTAssertEqual(calls.value, 4, "eleven minutes is over ten")
    }

    func testLowPowerModeWaitsFourTimesAsLong() async {
        let now = Box<Int64>(1_800_000_000)
        let calls = Box(0)
        let s = store(
            now: now, lowPower: true,
            fetcher: { _, _, _, _, _, _ in
                calls.value += 1
                return SpotFetchOutcome(snapshotJson: "{}", errors: [], suggestion: nil)
            })
        s.add(Spot(id: "a", name: "A", latitude: 1, longitude: 2, waterType: "pond"))
        await s.refreshOnOpen()
        now.value += 30 * 60
        await s.refreshOnOpen()
        XCTAssertEqual(calls.value, 1)
        now.value += 11 * 60
        await s.refreshOnOpen()
        XCTAssertEqual(calls.value, 2)
    }

    func testDueFollowsTheRefreshInterval() {
        XCTAssertEqual(RefreshPolicy.dueAfter(minutes: 15, lowPower: false), 600)
        XCTAssertEqual(RefreshPolicy.dueAfter(minutes: 60, lowPower: false), 2400)
        XCTAssertEqual(RefreshPolicy.dueAfter(minutes: 15, lowPower: true), 2400)
        let due = RefreshPolicy.due([("a", 100), ("b", nil), ("c", 1000)], now: 1100, after: 600)
        XCTAssertEqual(due, ["a", "b"])
    }

    func testTheShownSpotIsFetchedFirstAndOneAtATime() async {
        let order = Box<[String]>([])
        let running = Box(0)
        let most = Box(0)
        let s = store(fetcher: { spot, _, _, _, _, _ in
            running.value += 1
            most.value = max(most.value, running.value)
            try? await Task.sleep(for: .milliseconds(20))
            order.value.append(spot.id)
            running.value -= 1
            return SpotFetchOutcome(snapshotJson: "{}", errors: [], suggestion: nil)
        })
        for id in ["a", "b", "c"] {
            s.add(Spot(id: id, name: id, latitude: 1, longitude: 2, waterType: "pond"))
        }
        s.setActive("c")
        await s.refresh()
        XCTAssertEqual(order.value, ["c", "a", "b"])
        XCTAssertEqual(most.value, 1)
    }

    func testARefreshAskedForWhileOneRunsJoinsIt() async {
        let calls = Box<[String]>([])
        let s = store(fetcher: { spot, _, _, _, _, _ in
            calls.value.append(spot.id)
            try? await Task.sleep(for: .milliseconds(30))
            return SpotFetchOutcome(snapshotJson: "{}", errors: [], suggestion: nil)
        })
        for id in ["a", "b"] {
            s.add(Spot(id: id, name: id, latitude: 1, longitude: 2, waterType: "pond"))
        }
        async let first: Void = s.refresh(only: ["a"])
        try? await Task.sleep(for: .milliseconds(10))
        await s.refresh(only: ["b"])
        await first
        XCTAssertEqual(calls.value.sorted(), ["a", "b"])
        XCTAssertTrue(s.refreshing.isEmpty)
    }

    func testACancelledRefreshSkipsTheSpotsStillWaiting() async {
        let calls = Box(0)
        let s = store(fetcher: { _, _, _, _, _, _ in
            calls.value += 1
            try? await Task.sleep(for: .milliseconds(100))
            return SpotFetchOutcome(snapshotJson: "{}", errors: [], suggestion: nil)
        })
        for id in ["a", "b", "c"] {
            s.add(Spot(id: id, name: id, latitude: 1, longitude: 2, waterType: "pond"))
        }
        let task = Task { await s.refresh() }
        try? await Task.sleep(for: .milliseconds(30))
        task.cancel()
        await task.value
        XCTAssertEqual(calls.value, 1)
    }

    func testAnOfflineFetchKeepsTheOlderSnapshotAndSaysWhy() async {
        let offline = Box(false)
        let s = store(fetcher: { _, _, _, _, _, _ in
            if offline.value {
                return SpotFetchOutcome(
                    snapshotJson: "new",
                    errors: [
                        SourceIssue(
                            source: "coops", measurement: nil, kind: "transport", message: "offline")
                    ], suggestion: nil, unanswered: true)
            }
            return SpotFetchOutcome(snapshotJson: "old", errors: [], suggestion: nil)
        })
        s.add(Spot(id: "a", name: "A", latitude: 1, longitude: 2, waterType: "pond"))
        await s.refresh()
        offline.value = true
        await s.refresh()
        XCTAssertEqual(s.cache("a").snapshotJSON, "old")
        XCTAssertEqual(s.cache("a").error, "offline")
    }

    func testKeysAreReadAtFetchTime() async {
        let seen = Box<[String?]>([])
        let key = Box<String?>(nil)
        let host = URLSessionHost(session: StubProtocol.session(), storeURL: files.hostDirectory)
        let s = SpotStore(
            files: files, host: host, keys: { Keys(usgs: key.value, copernicusUser: nil, copernicusPassword: nil) }, clock: { 1_800_000_000 },
            lowPower: { false },
            fetcher: { _, _, keys, _, _, _ in
                seen.value.append(keys.usgs)
                return SpotFetchOutcome(snapshotJson: "{}", errors: [], suggestion: nil)
            })
        s.add(Spot(id: "a", name: "A", latitude: 1, longitude: 2, waterType: "pond"))
        await s.refresh()
        key.value = "k1"
        await s.refresh()
        XCTAssertEqual(seen.value, [nil, "k1"])
        let saved = [files.text("state.json"), files.text(files.cacheName("a"))].compactMap { $0 }
        XCTAssertFalse(saved.contains { $0.contains("k1") })
    }

    // MARK: - The core's own fetch

    private func ledger(in directory: URL) -> Int {
        let text = try? String(
            contentsOf: directory.appending(path: "host/usage/ledger.json"), encoding: .utf8)
        return Self.used(text)
    }

    /// Every `used` count in a ledger file.
    private static func used(_ json: String?) -> Int {
        guard let data = json?.data(using: .utf8),
            let object = try? JSONSerialization.jsonObject(with: data)
        else { return 0 }
        func sum(_ value: Any) -> Int {
            if let dictionary = value as? [String: Any] {
                return dictionary.reduce(0) { total, entry in
                    total + (entry.key == "used" ? (entry.value as? Int ?? 0) : sum(entry.value))
                }
            }
            if let array = value as? [Any] { return array.reduce(0) { $0 + sum($1) } }
            return 0
        }
        return sum(object)
    }

    private func fetched(_ spots: [Spot], in directory: URL) async -> Int {
        self.directory = directory
        let s = store()
        for spot in spots { s.add(spot) }
        await s.refresh()
        return ledger(in: directory)
    }

    func testTwoFetchesBothCountInTheCallLedger() async throws {
        let base = FileManager.default.temporaryDirectory
            .appending(path: "underbite-ledger-\(UUID().uuidString)", directoryHint: .isDirectory)
        defer { try? FileManager.default.removeItem(at: base) }
        let battery = SeedSpots.battery
        let croton = SeedSpots.croton
        let a = await fetched([battery], in: base.appending(path: "a"))
        let b = await fetched([croton], in: base.appending(path: "b"))
        let both = await fetched([battery, croton], in: base.appending(path: "both"))
        XCTAssertGreaterThan(a, 0, "a fetch spends metered calls")
        XCTAssertGreaterThan(b, 0)
        XCTAssertEqual(both, a + b, "one at a time, each fetch adds to the other's count")
    }

    func testOnlyRoundedCoordinatesLeaveThePhone() async {
        let s = store()
        s.add(SeedSpots.battery)
        await s.refresh()
        let urls = StubProtocol.received.compactMap { $0.url?.absoluteString }
        XCTAssertFalse(urls.isEmpty)
        for exact in ["40.7033", "74.017"] {
            XCTAssertFalse(urls.contains { $0.contains(exact) }, exact)
        }
        let bodies = StubProtocol.received.compactMap { $0.httpBody.flatMap { String(data: $0, encoding: .utf8) } }
        XCTAssertFalse(bodies.contains { $0.contains("40.7033") })
    }

    // MARK: - The page

    func testThePageHasTheListAndAnUpdatedTime() async throws {
        let now = Box<Int64>(1_800_000_000)
        // A real snapshot (every source unreachable) stands in for a fetched one.
        let snapshot = try await fetchSpot(
            spot: SeedSpots.battery.info, settingsJson: defaultSettingsJson(), keys: Self.noKeys,
            options: FetchOptions(allowMetered: true, inferWaterType: false), now: now.value,
            host: URLSessionHost(session: StubProtocol.session(), storeURL: directory.appending(path: "probe"))
        ).snapshotJson
        let s = store(now: now, fetcher: { _, _, _, _, _, _ in
            SpotFetchOutcome(snapshotJson: snapshot, errors: [], suggestion: nil)
        })
        s.add(SeedSpots.battery)
        s.add(SeedSpots.croton)
        await s.refreshOnOpen()
        now.value += 6 * 60
        await s.rebuildPage()
        XCTAssertEqual(s.listing.map(\.id), ["seed-battery", "seed-croton"])
        let battery = try XCTUnwrap(s.listing(for: "seed-battery"))
        XCTAssertFalse(battery.waterTypeLabel.isEmpty)
        XCTAssertEqual(battery.status.lastSuccess, 1_800_000_000)
        XCTAssertFalse(try XCTUnwrap(battery.status.updated).isEmpty)
        XCTAssertTrue(s.pageJSON.contains("\"view\""))
    }

    // MARK: - Location and seeds

    func testTheLocationIsNotSavedUntilTheUserSavesItAsASpot() throws {
        let s = store()
        XCTAssertTrue(s.noteFix(latitude: 40.7033, longitude: -74.0170))
        XCTAssertFalse(s.noteFix(latitude: 40.7040, longitude: -74.0175), "under a kilometre")
        XCTAssertTrue(s.noteFix(latitude: 40.72, longitude: -74.0170), "about 2 km north")
        XCTAssertFalse(s.noteFix(latitude: 40.80, longitude: -74.0170, accuracy: -1), "no accuracy")
        XCTAssertNil(files.text("state.json"))
        XCTAssertTrue(s.spots.isEmpty)
        let saved = try XCTUnwrap(s.saveCurrentPlace(named: "Here", waterType: "bay_inshore", timeZone: "America/New_York"))
        XCTAssertEqual(saved.latitude, 40.72)
        XCTAssertTrue(try XCTUnwrap(files.text("state.json")).contains("40.72"))
    }

    func testASpotIsAddedOnlyOnce() {
        let s = store()
        s.add(Spot(id: "a", name: "A", latitude: 1, longitude: 2, waterType: "pond"))
        s.add(Spot(id: "a", name: "Again", latitude: 1, longitude: 2, waterType: "pond"))
        XCTAssertEqual(s.spots.map(\.name), ["A"])
    }

    func testTheSeededPlacesAreAddedOnce() {
        let s = store()
        s.addSeedSpots()
        s.addSeedSpots()
        XCTAssertEqual(s.arranged.map(\.name), ["The Battery", "Croton River", "Buoy 44025"])
        XCTAssertEqual(s.activeID, "seed-battery")
        XCTAssertEqual(SeedSpots.buoy.latitude, 40.258)
    }
}
