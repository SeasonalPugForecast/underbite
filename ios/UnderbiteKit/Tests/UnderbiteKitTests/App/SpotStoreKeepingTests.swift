import Foundation
import XCTest

@testable import UnderbiteKit

/// A value the tests share with the store's `@Sendable` closures.
private final class Shared<Value>: @unchecked Sendable {
    private let lock = NSLock()
    private var held: Value
    init(_ value: Value) { held = value }
    var value: Value {
        get { lock.withLock { held } }
        set { lock.withLock { held = newValue } }
    }
}

/// What the store keeps of a spot and for how long: the water type a fetch suggests, nothing
/// of a deleted spot, one fetch at a time per spot, only the shown spot's cache in memory, and
/// the core's own words for an error. No network: the core's calls run against a stubbed
/// `URLSession`.
@MainActor
final class SpotStoreKeepingTests: XCTestCase {
    /// A directory of its own for each test (XCTest makes an instance per test).
    private let directory = FileManager.default.temporaryDirectory
        .appending(path: "underbite-keeping-\(UUID().uuidString)", directoryHint: .isDirectory)

    override func setUp() {
        StubProtocol.answer(.failure(URLError(.notConnectedToInternet)))
    }

    override func tearDown() {
        try? FileManager.default.removeItem(at: directory)
    }

    nonisolated private static let noKeys = Keys(
        usgs: nil, copernicusUser: nil, copernicusPassword: nil)

    private var files: AppFiles { AppFiles(directory: directory) }

    private func store(
        fetcher: SpotStore.Fetcher? = nil, discoverer: SpotStore.Discoverer? = nil
    ) -> SpotStore {
        let host = URLSessionHost(session: StubProtocol.session(), storeURL: files.hostDirectory)
        return SpotStore(
            files: files, host: host, keys: { Self.noKeys }, clock: { 1_800_000_000 },
            lowPower: { false },
            fetcher: fetcher ?? { spot, settings, keys, options, at, host in
                try await fetchSpot(
                    spot: spot, settingsJson: settings, keys: keys, options: options, now: at,
                    host: host)
            },
            discoverer: discoverer ?? { _, _, _, _, _ in
                DiscoveryOutcome(stationsJson: "{}", errors: [])
            })
    }

    /// Waits until `condition` holds, at most a few seconds; whether it does.
    private func until(_ condition: () -> Bool) async -> Bool {
        for _ in 0..<500 {
            if condition() { return true }
            try? await Task.sleep(for: .milliseconds(10))
        }
        return condition()
    }

    // MARK: - The water type a fetch suggests

    func testASuggestedWaterTypeIsTakenUntilTheUserChoosesOne() async {
        let seen = Shared<[String]>([])
        let inferred = Shared<[Bool]>([])
        let s = store(fetcher: { spot, _, _, options, _, _ in
            seen.value.append(spot.id + " " + spot.waterType)
            inferred.value.append(options.inferWaterType)
            return SpotFetchOutcome(
                snapshotJson: "{}", errors: [],
                suggestion: WaterTypeSuggestion(
                    waterType: "tidal_river_estuary", reasons: ["in an NWS coastal marine zone"]))
        })
        // Added with "Suggest one for me": a river until a fetch says otherwise.
        s.add(
            Spot(
                id: "a", name: "A", latitude: 40.7033, longitude: -74.017,
                waterType: "river_stream", waterTypeChosen: false))
        s.add(Spot(id: "b", name: "B", latitude: 41.2, longitude: -73.9, waterType: "pond"))
        await s.refresh()
        XCTAssertEqual(s.spot("a")?.waterType, "tidal_river_estuary")
        XCTAssertEqual(s.spot("a")?.waterTypeChosen, false, "still the user's to choose")
        XCTAssertEqual(s.cache("a").suggestion?.reasons, ["in an NWS coastal marine zone"])
        XCTAssertEqual(s.spot("b")?.waterType, "pond", "a chosen type stays")

        // The next fetch plans for the suggested type and infers no more; a restart keeps it.
        await s.refresh(only: ["a"])
        XCTAssertEqual(seen.value, ["a river_stream", "b pond", "a tidal_river_estuary"])
        XCTAssertEqual(inferred.value, [true, false, false])
        XCTAssertEqual(store().spot("a")?.waterType, "tidal_river_estuary")
    }

    // MARK: - A deleted spot

    /// The text of every file the store's fetch host holds.
    private func hostTexts() -> [String] {
        let walk = FileManager.default.enumerator(
            at: files.hostDirectory, includingPropertiesForKeys: nil)
        return (walk?.allObjects as? [URL] ?? []).compactMap {
            try? String(contentsOf: $0, encoding: .utf8)
        }
    }

    /// Whether a host file holds the Battery's exact longitude (its snapshot does).
    private func hostHoldsTheBattery() -> Bool {
        hostTexts().contains { $0.contains("-74.017,") || $0.contains("-74.017}") }
    }

    func testARemovedSpotLeavesNothingOfItsPositionOnTheHost() async {
        let s = store()
        s.add(SeedSpots.battery)
        await s.refresh()
        XCTAssertTrue(hostHoldsTheBattery(), "the fetch kept its snapshot on the host")
        s.remove(SeedSpots.battery.id)
        XCTAssertFalse(hostHoldsTheBattery())
        let host = URLSessionHost(storeURL: files.hostDirectory)
        XCTAssertNil(storedStations(spotId: SeedSpots.battery.id, host: host))
    }

    func testASpotRemovedWhileItIsFetchedKeepsNothing() async {
        let started = Shared(false)
        let go = Shared(false)
        let s = store(fetcher: { spot, settings, keys, options, at, host in
            started.value = true
            while !go.value { try? await Task.sleep(for: .milliseconds(5)) }
            return try await fetchSpot(
                spot: spot, settingsJson: settings, keys: keys, options: options, now: at,
                host: host)
        })
        s.add(SeedSpots.battery)
        let refresh = Task { await s.refresh() }
        let fetching = await until { started.value }
        XCTAssertTrue(fetching)
        s.remove(SeedSpots.battery.id)
        go.value = true
        await refresh.value
        let cache = files.url(files.cacheName(SeedSpots.battery.id))
        XCTAssertFalse(FileManager.default.fileExists(atPath: cache.path), "no cache written back")
        XCTAssertFalse(hostHoldsTheBattery(), "the snapshot the fetch kept is forgotten")
        XCTAssertTrue(s.spots.isEmpty)
    }

    func testASpotRemovedWhileItsStationsAreLookedUpKeepsNoRankings() async {
        let started = Shared(false)
        let go = Shared(false)
        let s = store(discoverer: { spot, settings, keys, at, host in
            started.value = true
            while !go.value { try? await Task.sleep(for: .milliseconds(5)) }
            return try await discoverStations(
                spot: spot, settingsJson: settings, keys: keys, allowMetered: true, now: at,
                host: host)
        })
        s.add(SeedSpots.battery)
        let lookup = Task { await s.discoverStations(for: SeedSpots.battery.id) }
        let looking = await until { started.value }
        XCTAssertTrue(looking)
        s.remove(SeedSpots.battery.id)
        go.value = true
        await lookup.value
        let host = URLSessionHost(storeURL: files.hostDirectory)
        XCTAssertNil(storedStations(spotId: SeedSpots.battery.id, host: host))
        let cache = files.url(files.cacheName(SeedSpots.battery.id))
        XCTAssertFalse(FileManager.default.fileExists(atPath: cache.path))
    }

    // MARK: - One fetch per spot at a time

    func testASpotBeingFetchedIsNotQueuedAgain() async {
        let calls = Shared(0)
        let started = Shared(false)
        let go = Shared(false)
        let s = store(fetcher: { _, _, _, _, _, _ in
            calls.value += 1
            started.value = true
            while !go.value { try? await Task.sleep(for: .milliseconds(5)) }
            return SpotFetchOutcome(snapshotJson: "{}", errors: [], suggestion: nil)
        })
        s.add(Spot(id: "a", name: "A", latitude: 1, longitude: 2, waterType: "pond"))
        let first = Task { await s.refreshOnOpen() }
        let fetching = await until { started.value }
        XCTAssertTrue(fetching)
        // Opened again while the only spot is fetched: it is due by its data, but under way.
        let second = Task { await s.refreshOnOpen() }
        try? await Task.sleep(for: .milliseconds(100))
        go.value = true
        await first.value
        await second.value
        XCTAssertEqual(calls.value, 1)
        XCTAssertTrue(s.refreshing.isEmpty)
    }

    func testTheQueueSkipsTheSpotBeingFetched() {
        var queue = RefreshQueue()
        queue.add(["a", "b"], except: ["a"])
        queue.add(["b", "c"])
        XCTAssertEqual(queue.waiting, ["b", "c"])
    }

    // MARK: - Caches

    func testARefreshHoldsOnlyTheShownSpotsCache() async throws {
        // A real snapshot (every source unreachable) stands in for a fetched one.
        let snapshot = try await fetchSpot(
            spot: SeedSpots.battery.info, settingsJson: defaultSettingsJson(), keys: Self.noKeys,
            options: FetchOptions(allowMetered: true, inferWaterType: false),
            now: 1_800_000_000,
            host: URLSessionHost(
                session: StubProtocol.session(), storeURL: directory.appending(path: "probe"))
        ).snapshotJson
        let fetches = Shared(0)
        let fetcher: SpotStore.Fetcher = { _, _, _, _, _, _ in
            fetches.value += 1
            return SpotFetchOutcome(snapshotJson: snapshot, errors: [], suggestion: nil)
        }
        let s = store(fetcher: fetcher)
        for id in ["a", "b", "c"] {
            s.add(Spot(id: id, name: id, latitude: 1, longitude: 2, waterType: "pond"))
        }
        await s.refreshOnOpen()
        XCTAssertEqual(fetches.value, 3)
        XCTAssertEqual(s.heldCaches, ["a"], "only the shown spot's snapshot is in memory")
        XCTAssertEqual(
            s.listing.map(\.status.lastSuccess), [1_800_000_000, 1_800_000_000, 1_800_000_000])

        // After a restart the statuses come from the files: none is due, only one is held.
        let again = store(fetcher: fetcher)
        await again.refreshOnOpen()
        XCTAssertEqual(fetches.value, 3)
        XCTAssertEqual(again.heldCaches, ["a"])
        XCTAssertEqual(
            again.listing.map(\.status.lastSuccess),
            [1_800_000_000, 1_800_000_000, 1_800_000_000])
        XCTAssertEqual(again.cache("b").lastSuccess, 1_800_000_000, "read when asked for")
    }

    // MARK: - Errors in the core's words

    func testAFailedFetchOrLookupSaysWhatTheCoreSaid() async {
        let s = store(
            fetcher: { _, _, _, _, _, _ in
                throw FetchError.Invalid(what: "water type", message: "`lake` is not a water type")
            },
            discoverer: { _, _, _, _, _ in
                throw FetchError.Settings(message: "units: unknown unit `furlongs`")
            })
        s.add(Spot(id: "a", name: "A", latitude: 1, longitude: 2, waterType: "pond"))
        await s.refresh()
        XCTAssertEqual(s.cache("a").error, "`lake` is not a water type")
        await s.discoverStations(for: "a")
        XCTAssertEqual(s.stationProblems["a"], "units: unknown unit `furlongs`")
    }
}
