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

/// The calls a store made: which, in what order, on what host and how many at once.
private final class Calls: Sendable {
    let running = Shared(0)
    let most = Shared(0)
    let hosts = Shared<[ObjectIdentifier]>([])
    let order = Shared<[String]>([])

    func run(_ what: String, _ host: FetchHost) async {
        running.value += 1
        most.value = max(most.value, running.value)
        hosts.value.append(ObjectIdentifier(host as AnyObject))
        try? await Task.sleep(for: .milliseconds(20))
        order.value.append(what)
        running.value -= 1
    }
}

/// Station lookups and choices through the store: on its own host, in turn with the fetches,
/// kept with the spot. No network: the core's calls run against a stubbed `URLSession`.
@MainActor
final class StationsStoreTests: XCTestCase {
    private var directory: URL!

    override func setUp() {
        directory = FileManager.default.temporaryDirectory
            .appending(path: "underbite-stations-\(UUID().uuidString)", directoryHint: .isDirectory)
        StubProtocol.answer(.failure(URLError(.notConnectedToInternet)))
    }

    override func tearDown() {
        try? FileManager.default.removeItem(at: directory)
    }

    nonisolated(unsafe) private static let noKeys = Keys(
        usgs: nil, copernicusUser: nil, copernicusPassword: nil)
    nonisolated private static let now: Int64 = 1_800_000_000

    private func store(
        in directory: URL, fetcher: SpotStore.Fetcher? = nil,
        discoverer: SpotStore.Discoverer? = nil
    ) -> (SpotStore, URLSessionHost) {
        let files = AppFiles(directory: directory)
        let host = URLSessionHost(session: StubProtocol.session(), storeURL: files.hostDirectory)
        let store = SpotStore(
            files: files, host: host, keys: { Self.noKeys }, clock: { Self.now },
            lowPower: { false },
            fetcher: fetcher ?? { spot, settings, keys, options, at, host in
                try await fetchSpot(
                    spot: spot, settingsJson: settings, keys: keys, options: options, now: at,
                    host: host)
            },
            discoverer: discoverer ?? { spot, settings, keys, at, host in
                try await discoverStations(
                    spot: spot, settingsJson: settings, keys: keys, allowMetered: true, now: at,
                    host: host)
            })
        return (store, host)
    }

    nonisolated private static func found(_ json: String = SpotsLogicTests.stations) -> DiscoveryOutcome {
        DiscoveryOutcome(stationsJson: json, errors: [])
    }

    // MARK: - One host, one ledger

    /// Every metered call the ledger in `directory` counts.
    private func ledger(in directory: URL) -> Int {
        let text = try? String(
            contentsOf: directory.appending(path: "host/usage/ledger.json"), encoding: .utf8)
        guard let data = text?.data(using: .utf8),
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

    /// The ledger files under `directory`.
    private func ledgers(in directory: URL) -> [String] {
        let walk = FileManager.default.enumerator(at: directory, includingPropertiesForKeys: nil)
        return (walk?.allObjects as? [URL] ?? [])
            .filter { $0.lastPathComponent == "ledger.json" }
            .map(\.path)
    }

    func testADiscoveryThroughTheStoreLeavesOneLedgerCountingEveryCall() async {
        // Two places, so neither call can lean on lists the other kept.
        let looking = SeedSpots.croton
        let fetching = SeedSpots.battery
        let lookup = directory.appending(path: "lookup")
        let (alone, _) = store(in: lookup)
        alone.add(looking)
        await alone.discoverStations(for: looking.id)
        let looked = ledger(in: lookup)

        let fetch = directory.appending(path: "fetch")
        let (fetcher, _) = store(in: fetch)
        fetcher.add(fetching)
        await fetcher.refresh()
        let fetched = ledger(in: fetch)
        XCTAssertGreaterThan(looked, 0, "a lookup spends metered calls")
        XCTAssertGreaterThan(fetched, 0, "a fetch spends metered calls")

        // Asked for together, the lookup and the fetch take turns on the store's host: each
        // adds to the other's count, in the one ledger of the store's directory.
        let both = directory.appending(path: "both")
        let (s, _) = store(in: both)
        s.add(fetching)
        s.add(looking)
        async let refreshed: Void = s.refresh(only: [fetching.id])
        async let discovered: Void = s.discoverStations(for: looking.id)
        _ = await (refreshed, discovered)
        XCTAssertEqual(ledger(in: both), looked + fetched)
        XCTAssertEqual(ledgers(in: both).count, 1, "\(ledgers(in: both))")
        XCTAssertNotNil(s.cache(looking.id).stationsJSON)
    }

    func testLookupsAndFetchesUseTheStoresHostOneAtATime() async {
        let calls = Calls()
        let (s, host) = store(
            in: directory,
            fetcher: { spot, _, _, _, _, host in
                await calls.run("fetch " + spot.id, host)
                return SpotFetchOutcome(snapshotJson: "{}", errors: [], suggestion: nil)
            },
            discoverer: { spot, _, _, _, host in
                await calls.run("lookup " + spot.id, host)
                return DiscoveryOutcome(stationsJson: "{}", errors: [])
            })
        for id in ["a", "b", "c"] {
            s.add(Spot(id: id, name: id, latitude: 1, longitude: 2, waterType: "pond"))
        }
        s.setActive("a")
        async let refreshed: Void = s.refresh()
        try? await Task.sleep(for: .milliseconds(5))
        // Asked for while "a" is fetched: it goes before "b" and "c", never beside a fetch.
        await s.discoverStations(for: "c")
        await refreshed
        XCTAssertEqual(calls.most.value, 1)
        XCTAssertEqual(calls.order.value, ["fetch a", "lookup c", "fetch b", "fetch c"])
        XCTAssertEqual(Set(calls.hosts.value), [ObjectIdentifier(host)])
        XCTAssertTrue(s.discovering.isEmpty)
    }

    func testAClosedListDoesNotDropAFetchThatJoinedItsLookup() async {
        let fetched = Shared<[String]>([])
        let (s, _) = store(
            in: directory,
            fetcher: { spot, _, _, _, _, _ in
                fetched.value.append(spot.id)
                return SpotFetchOutcome(snapshotJson: "{}", errors: [], suggestion: nil)
            },
            discoverer: { _, _, _, _, _ in
                try? await Task.sleep(for: .milliseconds(50))
                return DiscoveryOutcome(stationsJson: "{}", errors: [])
            })
        s.add(Spot(id: "a", name: "A", latitude: 1, longitude: 2, waterType: "pond"))
        let list = Task { await s.discoverStations(for: "a") }
        try? await Task.sleep(for: .milliseconds(10))
        async let refreshed: Void = s.refresh()
        list.cancel()
        await list.value
        await refreshed
        XCTAssertEqual(fetched.value, ["a"])
    }

    // MARK: - Kept with the spot

    func testTheLastLookupShowsAtTheNextLaunchWithoutWaiting() async {
        let (s, _) = store(in: directory, discoverer: { _, _, _, _, _ in StationsStoreTests.found() })
        s.add(SeedSpots.battery)
        XCTAssertEqual(s.stationRows(for: SeedSpots.battery.id), [])
        await s.discoverStations(for: SeedSpots.battery.id)
        XCTAssertEqual(s.stationRows(for: SeedSpots.battery.id).first?.title, "Tide")

        let calls = Shared(0)
        let (next, _) = store(
            in: directory,
            discoverer: { _, _, _, _, _ in
                calls.value += 1
                return DiscoveryOutcome(stationsJson: "{}", errors: [])
            })
        let rows = next.stationRows(for: SeedSpots.battery.id)
        XCTAssertEqual(rows.map(\.title), ["Tide", "Tidal current", "River flow"])
        XCTAssertEqual(rows[0].rows[0].distance, "0.4 mi")
        XCTAssertEqual(calls.value, 0)
    }

    func testAFetchKeepsTheStationsItRankedWithTheSpot() async {
        let lookups = Shared(0)
        let (s, _) = store(
            in: directory,
            // A fetch ranks the stations as a lookup does and stores the ranking on its host.
            fetcher: { spot, settings, keys, _, at, host in
                _ = try await discoverStations(
                    spot: spot, settingsJson: settings, keys: keys, allowMetered: false, now: at,
                    host: host)
                return SpotFetchOutcome(snapshotJson: "{}", errors: [], suggestion: nil)
            },
            discoverer: { _, _, _, _, _ in
                lookups.value += 1
                return DiscoveryOutcome(stationsJson: "{}", errors: [])
            })
        s.add(SeedSpots.battery)
        XCTAssertNil(s.cache(SeedSpots.battery.id).stationsJSON)
        let before = s.stationsRevision
        await s.refresh()
        XCTAssertNotNil(s.cache(SeedSpots.battery.id).stationsJSON)
        XCTAssertGreaterThan(s.stationsRevision, before)
        XCTAssertEqual(lookups.value, 0, "no lookup was needed")
    }

    func testAnOfflineLookupKeepsTheStationsItHadAndSaysWhy() async {
        let offline = Shared(false)
        let (s, _) = store(
            in: directory,
            discoverer: { _, _, _, _, _ in
                if offline.value {
                    return DiscoveryOutcome(
                        stationsJson: "{}",
                        errors: [
                            SourceIssue(
                                source: "coops", measurement: nil, kind: "transport",
                                message: "offline")
                        ])
                }
                return StationsStoreTests.found()
            })
        s.add(SeedSpots.battery)
        await s.discoverStations(for: SeedSpots.battery.id)
        offline.value = true
        await s.discoverStations(for: SeedSpots.battery.id)
        XCTAssertFalse(s.stationRows(for: SeedSpots.battery.id).isEmpty)
        XCTAssertEqual(s.stationProblems[SeedSpots.battery.id], "offline")
    }

    func testARemovedSpotKeepsNoStations() async {
        let (s, _) = store(
            in: directory,
            discoverer: { _, _, _, _, _ in
                try? await Task.sleep(for: .milliseconds(30))
                return StationsStoreTests.found()
            })
        s.add(SeedSpots.battery)
        async let looked: Void = s.discoverStations(for: SeedSpots.battery.id)
        try? await Task.sleep(for: .milliseconds(5))
        s.remove(SeedSpots.battery.id)
        await looked
        let files = AppFiles(directory: directory)
        XCTAssertNil(files.text(files.cacheName(SeedSpots.battery.id)))
    }

    // MARK: - Choices

    func testAChoiceIsKeptByTheStoresHostAndTheStationsAreLookedUpAgain() async throws {
        let lookups = Shared(0)
        let (s, host) = store(
            in: directory,
            discoverer: { _, _, _, _, _ in
                lookups.value += 1
                return StationsStoreTests.found()
            })
        s.add(SeedSpots.battery)
        try await s.chooseStation(.exclude, key: "coops:8518750", for: SeedSpots.battery.id)
        XCTAssertEqual(lookups.value, 1)
        let choices = host.storeURL.appending(path: "choices", directoryHint: .isDirectory)
        let kept = try FileManager.default.contentsOfDirectory(
            at: choices, includingPropertiesForKeys: nil)
        XCTAssertEqual(kept.count, 1)
        let text = try String(contentsOf: kept[0], encoding: .utf8)
        XCTAssertTrue(text.contains("8518750"), text)

        do {
            try await s.chooseStation(.pin, key: "coops", for: SeedSpots.battery.id)
            XCTFail("a key that is not one is refused")
        } catch {}
        XCTAssertEqual(lookups.value, 1)
    }
}
