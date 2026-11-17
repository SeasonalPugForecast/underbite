import Foundation
import XCTest

@testable import UnderbiteKit

/// The pages the store builds: one per spot on screen (each iPad window may show its own), the
/// list kept when one spot's saved data cannot be read, the reason a page is empty, saved spots
/// read leniently, and the core's words for an unreadable file. No network: the core's calls
/// run against a stubbed `URLSession`.
@MainActor
final class SpotStorePagesTests: XCTestCase {
    /// A directory of its own for each test (XCTest makes an instance per test).
    private let directory = FileManager.default.temporaryDirectory
        .appending(path: "underbite-pages-\(UUID().uuidString)", directoryHint: .isDirectory)

    override func setUp() {
        StubProtocol.answer(.failure(URLError(.notConnectedToInternet)))
    }

    override func tearDown() {
        try? FileManager.default.removeItem(at: directory)
    }

    nonisolated private static let noKeys = Keys(
        usgs: nil, copernicusUser: nil, copernicusPassword: nil)

    private var files: AppFiles { AppFiles(directory: directory) }

    private func store(fetcher: SpotStore.Fetcher? = nil) -> SpotStore {
        let host = URLSessionHost(session: StubProtocol.session(), storeURL: files.hostDirectory)
        return SpotStore(
            files: files, host: host, keys: { Self.noKeys }, clock: { 1_800_000_000 },
            lowPower: { false },
            fetcher: fetcher ?? { _, _, _, _, _, _ in
                SpotFetchOutcome(snapshotJson: "{}", errors: [], suggestion: nil)
            },
            discoverer: { _, _, _, _, _ in DiscoveryOutcome(stationsJson: "{}", errors: []) })
    }

    /// A real snapshot of `spot` (every source unreachable), as a fetch would have kept it.
    private func snapshot(of spot: Spot) async throws -> String {
        try await fetchSpot(
            spot: spot.info, settingsJson: defaultSettingsJson(), keys: Self.noKeys,
            options: FetchOptions(allowMetered: true, inferWaterType: false), now: 1_800_000_000,
            host: URLSessionHost(
                session: StubProtocol.session(), storeURL: directory.appending(path: "probe"))
        ).snapshotJson
    }

    private func keepCache(_ json: String?, for id: String) {
        var held = SpotCache()
        held.snapshotJSON = json
        held.lastSuccess = 1_800_000_000
        files.write(held, files.cacheName(id))
    }

    private func page(_ json: String) throws -> ViewModel.Page {
        try ViewModel.decode(json)
    }

    // MARK: - A page per spot

    func testTwoWindowsOnDifferentSpotsEachGetTheirOwnPage() async throws {
        let battery = SeedSpots.battery
        let croton = SeedSpots.croton
        keepCache(try await snapshot(of: battery), for: battery.id)
        keepCache(try await snapshot(of: croton), for: croton.id)
        let s = store()
        s.add(battery)
        s.add(croton)
        let (left, right) = (UUID(), UUID())
        await s.show(battery.id, viewer: left)
        await s.show(croton.id, viewer: right)
        // The window that showed The Battery first still has its page, not the other's.
        for (id, name) in [(battery.id, "The Battery"), (croton.id, "Croton River")] {
            let built = try page(s.pageJSON(for: id))
            XCTAssertEqual(built.spots[built.active].id, id)
            XCTAssertEqual(built.view?.header.name, name)
        }
        XCTAssertEqual(s.listing.map(\.id), [battery.id, croton.id])
        XCTAssertEqual(s.heldCaches, [battery.id, croton.id])

        // A rebuild (the minute tick, a refresh) keeps both.
        await s.rebuildPage()
        XCTAssertEqual(try page(s.pageJSON(for: battery.id)).view?.header.name, "The Battery")
        XCTAssertEqual(try page(s.pageJSON(for: croton.id)).view?.header.name, "Croton River")

        // The first window closes: its page and cache go at the next build; the other stays.
        s.hide(viewer: left)
        await s.rebuildPage()
        XCTAssertEqual(s.pageJSON(for: battery.id), "")
        XCTAssertEqual(s.heldCaches, [croton.id])
        XCTAssertEqual(try page(s.pageJSON(for: croton.id)).view?.header.name, "Croton River")
        XCTAssertEqual(s.listing.map(\.id), [battery.id, croton.id], "the list keeps every spot")
    }

    func testTwoWindowsOnOneSpotShareItsPage() async throws {
        let battery = SeedSpots.battery
        keepCache(try await snapshot(of: battery), for: battery.id)
        let s = store()
        s.add(battery)
        s.add(SeedSpots.croton)
        let (left, right) = (UUID(), UUID())
        await s.show(battery.id, viewer: left)
        await s.show(battery.id, viewer: right)
        await s.show(SeedSpots.croton.id, viewer: UUID())
        s.hide(viewer: left)
        await s.rebuildPage()
        XCTAssertEqual(
            try page(s.pageJSON(for: battery.id)).view?.header.name, "The Battery",
            "still shown in the other window")
    }

    // MARK: - Empty pages say why

    func testAnUnreadableSnapshotKeepsTheListAndSaysWhy() async throws {
        let battery = SeedSpots.battery
        keepCache("{ not a snapshot", for: battery.id)
        keepCache(try await snapshot(of: SeedSpots.croton), for: SeedSpots.croton.id)
        let s = store()
        s.add(battery)
        s.add(SeedSpots.croton)
        await s.show(battery.id, viewer: UUID())
        XCTAssertEqual(s.listing.map(\.id), [battery.id, SeedSpots.croton.id])
        let built = try page(s.pageJSON(for: battery.id))
        XCTAssertNil(built.view)
        let problem = try XCTUnwrap(built.problem)
        XCTAssertFalse(problem.title.isEmpty)
        XCTAssertTrue(
            problem.reason.hasPrefix("The saved conditions could not be read"), problem.reason)
        XCTAssertEqual(s.listing(for: battery.id)?.status.error, problem.reason)
        XCTAssertNil(s.listing(for: SeedSpots.croton.id)?.status.error)
    }

    func testAFailedFetchIsTheReasonAnEmptyPageGives() async throws {
        let s = store(fetcher: { _, _, _, _, _, _ in
            throw FetchError.Settings(message: "units: unknown unit `furlongs`")
        })
        s.add(SeedSpots.battery)
        await s.show(SeedSpots.battery.id, viewer: UUID())
        await s.refresh()
        let built = try page(s.pageJSON(for: SeedSpots.battery.id))
        XCTAssertNil(built.view)
        XCTAssertEqual(built.problem?.reason, "units: unknown unit `furlongs`")
    }

    func testAPageThatCannotBeBuiltLeavesTheList() async throws {
        let host = URLSessionHost(session: StubProtocol.session(), storeURL: files.hostDirectory)
        // The page of a spot with a snapshot fails; the list, built without one, does not.
        let s = SpotStore(
            files: files, host: host, keys: { Self.noKeys }, clock: { 1_800_000_000 },
            lowPower: { false },
            pageBuilder: { spots, active, settings, now in
                if spots.contains(where: { $0.snapshotJson != nil }) {
                    throw ViewError.Invalid(what: "view", message: "broken")
                }
                return try viewModel(
                    spots: spots, active: active, settingsJson: settings, now: now)
            })
        keepCache("{}", for: SeedSpots.battery.id)
        s.add(SeedSpots.battery)
        s.add(SeedSpots.croton)
        await s.show(SeedSpots.battery.id, viewer: UUID())
        XCTAssertEqual(s.pageJSON(for: SeedSpots.battery.id), "")
        XCTAssertEqual(s.listing.map(\.id), [SeedSpots.battery.id, SeedSpots.croton.id])
    }

    // MARK: - Files

    func testAnOddSpotLoadsWithDefaultsAndTheFileIsNotSetAside() throws {
        // Written by hand or by an older version: only the position and a few keys.
        let state = """
            {"version": 1, "spots": [
              {"id": "a", "name": "A", "latitude": 40.7033, "longitude": -74.017,
               "waterType": "tidal_river_estuary", "waterTypeChosen": true, "timeZone": "",
               "pinned": false, "favourite": false, "order": 0},
              {"latitude": 41.2, "longitude": -73.9, "name": "Odd", "pinned": "yes"}
            ]}
            """
        files.write(Data(state.utf8), "state.json")
        let s = store()
        XCTAssertTrue(s.fileProblems.isEmpty)
        XCTAssertFalse(FileManager.default.fileExists(atPath: files.url("state.json.bad").path))
        XCTAssertEqual(s.spots.map(\.name), ["A", "Odd"])
        let odd = s.spots[1]
        XCTAssertEqual(odd.latitude, 41.2)
        XCTAssertFalse(odd.id.isEmpty)
        XCTAssertEqual(odd.waterType, Spot.unknownWaterType)
        XCTAssertFalse(odd.waterTypeChosen, "a fetch may suggest the type")
        XCTAssertFalse(odd.pinned)
        XCTAssertEqual(odd.order, 0)
        XCTAssertNil(odd.areaID)
    }

    func testFileProblemsAreWordedByTheCore() {
        XCTAssertEqual(
            FileProblem(file: .spots, keptAs: "state.json.bad").text,
            fileProblemText(file: .spots, keptAs: "state.json.bad"))
        XCTAssertTrue(
            FileProblem(file: .settings, keptAs: "settings.json.bad").text
                .contains("kept as settings.json.bad"))
        XCTAssertTrue(FileProblem(file: .spots, keptAs: nil).text.contains("never saved over"))
    }
}
