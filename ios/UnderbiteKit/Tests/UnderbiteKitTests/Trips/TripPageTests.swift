import Foundation
import XCTest

@testable import UnderbiteKit

/// A trip's page: built from the saved spots' snapshots and what is kept for the trip, the
/// core's planned requests sent through the page's own sender, and what came back kept in the
/// trip's file. No network: the sender is a stand-in.
@MainActor
final class TripPageTests: XCTestCase {
    /// A directory of its own for each test (XCTest makes an instance per test).
    private let directory = FileManager.default.temporaryDirectory
        .appending(path: "underbite-trip-page-\(UUID().uuidString)", directoryHint: .isDirectory)

    override func setUp() {
        StubProtocol.answer(.failure(URLError(.notConnectedToInternet)))
    }

    override func tearDown() {
        try? FileManager.default.removeItem(at: directory)
    }

    nonisolated private static let noKeys = Keys(
        usgs: nil, copernicusUser: nil, copernicusPassword: nil)
    /// Just after the sample snapshot was fetched.
    nonisolated private static let now = SampleSnapshot.fetchedAt + 360

    private var files: AppFiles { AppFiles(directory: directory) }

    private func spotStore() -> SpotStore {
        let host = URLSessionHost(session: StubProtocol.session(), storeURL: files.hostDirectory)
        let s = SpotStore(
            files: files, host: host, keys: { Self.noKeys }, clock: { Self.now },
            lowPower: { false },
            fetcher: { _, _, _, _, _, _ in
                SpotFetchOutcome(snapshotJson: SampleSnapshot.json, errors: [], suggestion: nil)
            },
            discoverer: { _, _, _, _, _ in DiscoveryOutcome(stationsJson: "{}", errors: []) })
        s.add(SeedSpots.battery)
        s.add(SeedSpots.croton)
        return s
    }

    /// The Battery's cache holds the sample snapshot, fetched `age` seconds before now.
    private func keepBattery(age: Int64 = 0) {
        var held = SpotCache()
        held.snapshotJSON = SampleSnapshot.json
        held.lastSuccess = Self.now - age
        files.write(held, files.cacheName(SeedSpots.battery.id))
    }

    private func trip() -> SavedTrip {
        var draft = TripDraft(
            first: "2026-11-06", days: 3, places: [SeedSpots.battery.id, SeedSpots.croton.id])
        draft.id = "trip-page-test"
        draft.name = "Hudson stripers"
        return SavedTrip(
            id: draft.id,
            json: draft.json(
                spots: [SeedSpots.battery, SeedSpots.croton], areas: [], settingsJSON: "{}"))
    }

    /// Records what it is asked and answers every request with a 404.
    private final class Recorder: @unchecked Sendable {
        private let lock = NSLock()
        private var urls: [String] = []
        var asked: [String] { lock.withLock { urls } }

        func send(_ request: HostRequest) async throws -> HostResponse {
            lock.withLock { urls.append(request.url) }
            return HostResponse(
                status: 404, body: Data(), contentType: nil, date: nil, expires: nil,
                lastModified: nil, etag: nil, retryAfter: nil, rateLimitRemaining: nil)
        }
    }

    private func page(_ recorder: Recorder) -> TripPage {
        TripPage(
            tripID: "trip-page-test", files: files, send: { try await recorder.send($0) },
            keys: { Self.noKeys }, clock: { Self.now }, lowPower: { false })
    }

    func testThePageIsBuiltFromTheSavedSpotsSnapshots() async throws {
        keepBattery()
        let spots = spotStore()
        let page = page(Recorder())
        await page.build(trip(), spots: spots)
        let view = try XCTUnwrap(page.view)
        XCTAssertNil(page.problem)
        XCTAssertEqual(view.places.map(\.name), ["The Battery", "Croton River"])
        XCTAssertEqual(view.days.count, 3)
        let tides = try XCTUnwrap(view.days[0].places[0]?.cell("tides"))
        XCTAssertEqual(tides.basis, "certain", "from the snapshot's tide table")
        XCTAssertEqual(
            try XCTUnwrap(view.days[0].places[1]?.cell("flow")).basis, "unavailable",
            "the river has no snapshot")
    }

    func testATripTheCoreCannotReadSaysWhy() async {
        let page = page(Recorder())
        await page.build(SavedTrip(id: "trip-page-test", json: "{}"), spots: spotStore())
        XCTAssertNil(page.view)
        XCTAssertFalse(page.problem?.isEmpty ?? true)
    }

    func testARefreshFetchesOnlyTheDueSpotsAndSendsThePlannedRequests() async throws {
        keepBattery(age: 0)
        let spots = spotStore()
        let recorder = Recorder()
        let page = page(recorder)
        await page.refresh(trip(), spots: spots, all: false)
        // The Croton River had never been fetched; The Battery was fresh.
        XCTAssertNotNil(spots.cache(SeedSpots.croton.id).snapshotJSON)
        XCTAssertEqual(spots.cache(SeedSpots.croton.id).lastSuccess, Self.now)
        // The Battery's tide station's constants, so every day gets its tides.
        let asked = recorder.asked
        XCTAssertTrue(
            asked.contains { $0.contains("tidesandcurrents.noaa.gov") && $0.contains("8518750") },
            "\(asked)")
        XCTAssertNotNil(page.view, "built again after the refresh")
        XCTAssertFalse(page.refreshing)
        // Every request failed: no station metadata was kept.
        XCTAssertTrue(TripData.read("trip-page-test", files: files).metadata.isEmpty)
    }
}
