import Foundation
import XCTest

@testable import UnderbiteKit

/// A value the tests share with the preview's `@Sendable` closures.
private final class Shared<Value>: @unchecked Sendable {
    private let lock = NSLock()
    private var held: Value
    init(_ value: Value) { held = value }
    var value: Value {
        get { lock.withLock { held } }
        set { lock.withLock { held = newValue } }
    }
}

/// The conditions of a place that is not saved: the preview fetches and builds its page itself
/// and leaves the store's saved state alone. No network: the core's calls run against a stubbed
/// `URLSession`.
@MainActor
final class ConditionsPreviewTests: XCTestCase {
    private let directory = FileManager.default.temporaryDirectory
        .appending(path: "underbite-preview-\(UUID().uuidString)", directoryHint: .isDirectory)

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
        fetcher: SpotStore.Fetcher? = nil, pageBuilder: SpotStore.PageBuilder? = nil
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
            pageBuilder: pageBuilder ?? { _, _, _, _ in "{}" })
    }

    /// Every file under the app's directory, relative, with its size.
    private func listing() -> [String] {
        let walk = FileManager.default.enumerator(
            at: directory, includingPropertiesForKeys: [.fileSizeKey, .isDirectoryKey])
        return (walk?.allObjects as? [URL] ?? []).map { url in
            let size = (try? url.resourceValues(forKeys: [.fileSizeKey]).fileSize) ?? 0
            return url.path.replacingOccurrences(of: directory.path, with: "") + " \(size)"
        }.sorted()
    }

    private func hostHoldsTheBattery() -> Bool {
        let walk = FileManager.default.enumerator(
            at: files.hostDirectory, includingPropertiesForKeys: nil)
        let texts = (walk?.allObjects as? [URL] ?? []).compactMap {
            try? String(contentsOf: $0, encoding: .utf8)
        }
        return texts.contains { $0.contains("-74.017,") || $0.contains("-74.017}") }
    }

    private let croton = SpotPrefill(
        latitude: 41.2250278, longitude: -73.8588, name: "Croton River", waterType: "river_stream")

    // MARK: - Nothing is saved

    func testAPreviewAddsNoSpotAndKeepsNothingInTheStore() async {
        let seen = Shared<[SpotInfo]>([])
        let built = Shared<[[SpotEntry]]>([])
        let s = store(
            fetcher: { spot, _, _, _, _, _ in
                seen.value.append(spot)
                return SpotFetchOutcome(snapshotJson: "{}", errors: [], suggestion: nil)
            },
            pageBuilder: { entries, _, _, _ in
                built.value.append(entries)
                return "{\"preview\":true}"
            })
        let before = listing()
        let preview = s.makePreview(croton, place: "41.2250, -73.8588")
        await preview.load()

        XCTAssertEqual(preview.pageJSON, "{\"preview\":true}")
        XCTAssertEqual(seen.value.count, 1)
        XCTAssertTrue(seen.value[0].id.hasPrefix("preview-"))
        XCTAssertEqual(seen.value[0].name, "Croton River")
        XCTAssertEqual(seen.value[0].waterType, "river_stream")
        XCTAssertEqual(built.value.count, 1)
        XCTAssertEqual(built.value[0].count, 1, "the page holds the preview's spot alone")

        XCTAssertTrue(s.spots.isEmpty)
        XCTAssertNil(s.activeID)
        XCTAssertNil(s.spot(preview.id))
        XCTAssertTrue(s.pages.isEmpty)
        XCTAssertTrue(s.listing.isEmpty)
        XCTAssertTrue(s.refreshing.isEmpty)
        XCTAssertTrue(s.stationProblems.isEmpty)
        XCTAssertTrue(s.pageJSON(for: preview.id).isEmpty)
        XCTAssertTrue(s.heldCaches.isEmpty, "no snapshot is cached")
        XCTAssertEqual(listing(), before, "no file of the app's changed")
        XCTAssertTrue(store().spots.isEmpty)
    }

    func testAPreviewOfUnknownWaterAsksTheCoreAndKeepsNoPins() async {
        let options = Shared<[FetchOptions]>([])
        let s = store(fetcher: { _, _, _, option, _, _ in
            options.value.append(option)
            return SpotFetchOutcome(
                snapshotJson: "{}", errors: [],
                suggestion: WaterTypeSuggestion(waterType: "tidal_river_estuary", reasons: []))
        })
        let preview = s.makePreview(
            SpotPrefill(latitude: 40.7033, longitude: -74.017, name: "", waterType: nil),
            place: "40.7033, -74.0170")
        await preview.load()
        XCTAssertEqual(options.value.map(\.inferWaterType), [true])
        XCTAssertEqual(options.value.map(\.refreshMetadata), [false])
        XCTAssertEqual(preview.spot.waterType, "tidal_river_estuary")
        XCTAssertEqual(preview.savePrefill.waterType, "tidal_river_estuary")
        XCTAssertTrue(s.spots.isEmpty)
        XCTAssertNil(
            storedStations(spotId: preview.id, host: URLSessionHost(storeURL: files.hostDirectory)))
    }

    func testClosingAPreviewClearsItsSnapshotFromTheHost() async {
        let s = store()
        let preview = s.makePreview(
            SpotPrefill(
                latitude: 40.7033, longitude: -74.017, name: "The Battery",
                waterType: "tidal_river_estuary"),
            place: "40.7033, -74.0170")
        await preview.load()
        XCTAssertTrue(hostHoldsTheBattery(), "the fetch kept its snapshot on the host")
        preview.close()
        XCTAssertFalse(hostHoldsTheBattery())
        XCTAssertTrue(s.spots.isEmpty)
    }

    // MARK: - Save as spot

    func testSaveAsSpotStartsFromThePlaceTheNameAndTheType() {
        let s = store()
        let preview = s.makePreview(croton, place: "41.2250, -73.8588")
        XCTAssertEqual(preview.savePrefill, croton)
        XCTAssertEqual(preview.savePrefill.latitude, 41.2250278)
        XCTAssertEqual(preview.savePrefill.longitude, -73.8588)
        XCTAssertEqual(preview.savePrefill.name, "Croton River")
        XCTAssertEqual(preview.savePrefill.waterType, "river_stream")
        let unknown = s.makePreview(
            SpotPrefill(latitude: 1, longitude: 2, name: "", waterType: nil), place: "1, 2")
        XCTAssertNil(unknown.savePrefill.waterType, "the core suggests one when saved")
    }

    // MARK: - Closing

    func testClosingAPreviewCancelsItsFetch() async {
        let started = Shared(false)
        let cancelled = Shared(false)
        let built = Shared(0)
        let s = store(
            fetcher: { _, _, _, _, _, _ in
                started.value = true
                for _ in 0..<1000 where !Task.isCancelled {
                    try? await Task.sleep(for: .milliseconds(5))
                }
                cancelled.value = Task.isCancelled
                return SpotFetchOutcome(snapshotJson: "{}", errors: [], suggestion: nil)
            },
            pageBuilder: { _, _, _, _ in
                built.value += 1
                return "{}"
            })
        let preview = s.makePreview(croton, place: "41.2250, -73.8588")
        let loading = Task { await preview.load() }
        for _ in 0..<500 where !started.value {
            try? await Task.sleep(for: .milliseconds(10))
        }
        XCTAssertTrue(started.value)
        preview.close()
        await loading.value
        XCTAssertTrue(cancelled.value, "the fetch was told to stop")
        XCTAssertEqual(built.value, 0, "no page is built after the preview closed")
        XCTAssertEqual(preview.pageJSON, "")
        XCTAssertTrue(preview.closed)
        XCTAssertFalse(preview.refreshing)
        XCTAssertTrue(s.spots.isEmpty)
    }
}
