import Foundation
import XCTest

@testable import UnderbiteKit

/// A fetch host that answers every request with one body (or a status of its own), records the
/// URLs asked and keeps its store in a dictionary; no network.
private final class SourcesHost: FetchHost, @unchecked Sendable {
    private let lock = NSLock()
    private var asked: [String] = []
    private var values: [String: String] = [:]
    private let status: UInt16
    private let body: Data

    init(status: UInt16 = 200, body: Data) {
        self.status = status
        self.body = body
    }

    var urls: [String] { lock.withLock { asked } }

    func send(request: HostRequest) async throws -> HostResponse {
        lock.withLock { asked.append(request.url) }
        return HostResponse(
            status: status, body: body, contentType: "application/json", date: nil,
            expires: nil, lastModified: nil, etag: nil, retryAfter: nil, rateLimitRemaining: nil)
    }

    func load(namespace: String, key: String) -> String? {
        lock.withLock { values["\(namespace)/\(key)"] }
    }

    func store(namespace: String, key: String, value: String) {
        lock.withLock { values["\(namespace)/\(key)"] = value.isEmpty ? nil : value }
    }

    func saveUsage(ledgerJson: String) throws {}
}

/// The recorded list of the elevation models under The Battery, from the repository's fixtures.
private func recordedSources() throws -> Data {
    let root = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent()  // MapDepth
        .deletingLastPathComponent()  // UnderbiteKitTests
        .deletingLastPathComponent()  // Tests
        .deletingLastPathComponent()  // UnderbiteKit
        .deletingLastPathComponent()  // ios
        .deletingLastPathComponent()
    let url = root.appending(
        path: "tests/fixtures/raw/bathymetry/dem-global-mosaic-query-battery.json")
    return try Data(contentsOf: url)
}

/// What the depths in the view are measured from: the models at its centre, asked for once.
@MainActor
final class DepthDatumLoaderTests: XCTestCase {
    /// The harbour off The Battery at zoom 13.
    private let battery = MapRegion(
        west: -74.04, south: 40.69, east: -73.99, north: 40.72, zoom: 13)

    func testTheDatumsOfTheViewComeFromTheModelsAtItsCentreAskedForOnce() async throws {
        let host = SourcesHost(body: try recordedSources())
        let store = GridStore()
        let loader = DepthDatumLoader(host: host, store: store)
        var seen: [DepthDatums?] = []
        loader.onChange = { seen.append($0) }
        let settings = try Settings.defaults().json()

        let task = loader.load(battery, settingsJSON: settings)
        let before = try XCTUnwrap(loader.datums)
        XCTAssertTrue(before.datums.isEmpty, "no model known yet")
        XCTAssertFalse(before.text.isEmpty)
        await task?.value
        XCTAssertEqual(host.urls.count, 1)
        XCTAssertTrue(host.urls[0].contains("DEM_global_mosaic/ImageServer/query?"), host.urls[0])
        let after = try XCTUnwrap(loader.datums)
        XCTAssertEqual(after.datums.first, "NAVD 88", "the finest model first")
        XCTAssertTrue(after.text.contains("NAVD 88"), after.text)
        XCTAssertNotEqual(after.text, before.text)
        XCTAssertEqual(seen.count, 2)

        // The same view, or a pan nearby, knows the models already.
        XCTAssertNil(loader.load(battery, settingsJSON: settings))
        var panned = battery
        panned.west += 0.01
        panned.east += 0.01
        XCTAssertNil(loader.load(panned, settingsJSON: settings))
        XCTAssertEqual(host.urls.count, 1)
        XCTAssertEqual(loader.datums?.datums, after.datums)
    }

    func testAFailedListIsAskedForAgainByALaterView() async throws {
        let host = SourcesHost(status: 500, body: Data("error".utf8))
        let loader = DepthDatumLoader(host: host, store: GridStore())
        let settings = try Settings.defaults().json()
        await loader.load(battery, settingsJSON: settings)?.value
        XCTAssertEqual(loader.datums?.datums, [])
        XCTAssertFalse(loader.datums?.text.isEmpty ?? true, "the line still says what it can")
        await loader.load(battery, settingsJSON: settings)?.value
        XCTAssertEqual(host.urls.count, 2)
    }
}
