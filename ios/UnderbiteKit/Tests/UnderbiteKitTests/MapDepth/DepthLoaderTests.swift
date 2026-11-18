import Foundation
import MapKit
import XCTest

@testable import UnderbiteKit

/// A fetch host that answers every request with the recorded NCEI elevation export (or a status
/// of its own), records the URLs asked and keeps its store in a dictionary; no network.
private final class ElevationHost: FetchHost, @unchecked Sendable {
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
            status: status, body: body, contentType: status == 200 ? "image/tiff" : "text/plain",
            date: nil, expires: nil, lastModified: nil, etag: nil, retryAfter: nil,
            rateLimitRemaining: nil)
    }

    func load(namespace: String, key: String) -> String? {
        lock.withLock { values["\(namespace)/\(key)"] }
    }

    func store(namespace: String, key: String, value: String) {
        lock.withLock { values["\(namespace)/\(key)"] = value.isEmpty ? nil : value }
    }

    func saveUsage(ledgerJson: String) throws {}
}

/// The recorded export of the harbour (40.0 to 40.8 N, 74.2 to 73.0 W, 240 by 160 cells), from
/// the repository's fixtures.
func recordedElevation() throws -> Data {
    let root = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent()  // MapDepth
        .deletingLastPathComponent()  // UnderbiteKitTests
        .deletingLastPathComponent()  // Tests
        .deletingLastPathComponent()  // UnderbiteKit
        .deletingLastPathComponent()  // ios
        .deletingLastPathComponent()
    let url = root.appending(path: "tests/fixtures/raw/bathymetry/dem-global-mosaic-export-f32.tif")
    return try Data(contentsOf: url)
}

/// 2026-11-15T12:00:00Z.
private let now: Int64 = 1_794_744_000

/// The elevation for the view on screen: one area per view and zoom step, a little larger than
/// the view, used again while the store holds it.
@MainActor
final class DepthLoaderTests: XCTestCase {
    /// The Lower Bay at zoom 11.
    private let bay = MapRegion(west: -74.15, south: 40.42, east: -73.95, north: 40.58, zoom: 11)

    private func loader(_ host: ElevationHost, _ store: GridStore = GridStore()) -> DepthLoader {
        DepthLoader(host: host, store: store, clock: { now })
    }

    // MARK: - Areas

    func testAnAreaIsTheViewWithAMarginAndServesItsOwnZoomStep() {
        let area = DepthArea.around(bay)
        XCTAssertEqual(area.west, -74.18, accuracy: 1e-9)
        XCTAssertEqual(area.east, -73.92, accuracy: 1e-9)
        XCTAssertEqual(area.south, 40.396, accuracy: 1e-9)
        XCTAssertEqual(area.north, 40.604, accuracy: 1e-9)
        XCTAssertTrue(area.covers(bay))
        var panned = bay
        panned.west += 0.02
        panned.east += 0.02
        XCTAssertTrue(area.covers(panned))
        panned.west += 0.05
        panned.east += 0.05
        XCTAssertFalse(area.covers(panned), "past the margin")
        var closer = bay
        closer.zoom = 12.2
        XCTAssertFalse(area.covers(closer), "a finer zoom step needs finer cells")
        var slightly = bay
        slightly.zoom = 11.9
        XCTAssertTrue(area.covers(slightly))
        let world = DepthArea.around(
            MapRegion(west: -180, south: -80, east: 180, north: 80, zoom: 1))
        XCTAssertEqual([world.west, world.south, world.east, world.north], [-180, -85, 180, 85])
    }

    func testTheAreasHeldKeepTheNewestAndForgetTheDropped() {
        var held = DepthAreas()
        let areas = (0..<10).map { i in
            DepthArea(
                west: -74 + Double(i), south: 40, east: -73 + Double(i), north: 41, zoom: 9)
        }
        for area in areas { held.hold(area) }
        XCTAssertEqual(held.areas, Array(areas.suffix(DepthAreas.capacity)))
        held.hold(areas[5])
        XCTAssertEqual(held.areas.last, areas[5])
        XCTAssertEqual(held.areas.count, DepthAreas.capacity)
        let inside = MapRegion(west: -68.8, south: 40.2, east: -68.2, north: 40.8, zoom: 9.5)
        XCTAssertEqual(held.covering(inside), areas[5])
        held.forget(areas[5])
        XCTAssertNil(held.covering(inside))
    }

    // MARK: - Loading

    func testAViewIsAskedForOnceAndASmallPanUsesTheSameArea() async throws {
        let host = ElevationHost(body: try recordedElevation())
        let store = GridStore()
        let depth = loader(host, store)
        var states: [DepthLoader.State] = []
        depth.onChange = { states.append($0) }
        await depth.load(bay)?.value
        XCTAssertEqual(host.urls.count, 1)
        let url = try XCTUnwrap(host.urls.first)
        XCTAssertTrue(url.contains("DEM_global_mosaic/ImageServer/exportImage"), url)
        XCTAssertTrue(url.contains("bbox=-74.18,40.396,-73.92,40.604"), url)
        let area = DepthArea.around(bay)
        XCTAssertEqual(states, [.loading(area), .ready(area)])
        XCTAssertEqual(store.blockCount(), 1)

        var panned = bay
        panned.west += 0.02
        panned.east += 0.02
        XCTAssertNil(depth.load(panned), "the area held covers it")
        XCTAssertEqual(host.urls.count, 1)
        XCTAssertEqual(depth.state, .ready(area))

        var closer = bay
        closer.zoom = 12.5
        await depth.load(closer)?.value
        XCTAssertEqual(host.urls.count, 2, "a finer zoom step asks again")
        XCTAssertEqual(depth.state, .ready(DepthArea.around(closer)))
    }

    func testAnAreaTheStoreDroppedIsAskedForAgain() async throws {
        let host = ElevationHost(body: try recordedElevation())
        let store = GridStore()
        let depth = loader(host, store)
        await depth.load(bay)?.value
        store.clear()
        await depth.load(bay)?.value
        XCTAssertEqual(host.urls.count, 2)
        XCTAssertEqual(host.urls[0], host.urls[1])
        XCTAssertEqual(store.blockCount(), 1)
        XCTAssertEqual(depth.state, .ready(DepthArea.around(bay)))
    }

    func testAViewTooWideForOneAreaFetchesNothing() throws {
        let host = ElevationHost(body: try recordedElevation())
        let depth = loader(host)
        let coast = MapRegion(west: -80, south: 35, east: -70, north: 45, zoom: 6)
        XCTAssertNil(depth.load(coast))
        XCTAssertEqual(depth.state, .tooWide)
        XCTAssertTrue(host.urls.isEmpty)
    }

    func testAViewThatFitsOnlyWithoutTheMarginIsAskedWithoutIt() async throws {
        let host = ElevationHost(body: try recordedElevation())
        let depth = loader(host)
        // Four and a half degrees wide: with the margin it would be more than five.
        let wide = MapRegion(west: -75, south: 39, east: -70.5, north: 41.5, zoom: 7.5)
        await depth.load(wide)?.value
        XCTAssertEqual(host.urls.count, 1)
        XCTAssertTrue(host.urls[0].contains("bbox=-75.0,39.0,-70.5,41.5"), host.urls[0])
        XCTAssertEqual(depth.state, .ready(DepthArea.around(wide, margin: 0)))
    }

    func testAFailedFetchSaysSoAndTheNextViewAsksAgain() async throws {
        let host = ElevationHost(status: 500, body: Data("error".utf8))
        let store = GridStore()
        let depth = loader(host, store)
        await depth.load(bay)?.value
        XCTAssertEqual(depth.state, .failed(DepthArea.around(bay)))
        XCTAssertEqual(store.blockCount(), 0)
        await depth.load(bay)?.value
        XCTAssertEqual(host.urls.count, 2)
    }

    // MARK: - What the store then draws

    func testTheAreaFetchedShadesTheBayAndDrawsItsContours() async throws {
        let host = ElevationHost(body: try recordedElevation())
        let store = GridStore()
        await loader(host, store).load(bay)?.value
        let settings = try Settings.defaults().json()
        // The tile over the Lower Bay at zoom 11.
        let png = try XCTUnwrap(try store.renderDepthTile(z: 11, x: 602, y: 771, settingsJson: settings))
        XCTAssertEqual(Array(png.prefix(4)), [0x89, 0x50, 0x4E, 0x47])
        let lines = try store.contourLines(
            west: bay.west, south: bay.south, east: bay.east, north: bay.north, zoom: bay.zoom,
            settingsJson: settings)
        XCTAssertFalse(lines.isEmpty)
        let contours = ContourSet(lines)
        XCTAssertEqual(contours.lines.count, lines.filter { $0.points.count >= 2 }.count)
        XCTAssertTrue(contours.lines.contains { $0.label != nil })
        XCTAssertTrue(
            contours.lines.compactMap(\.label).allSatisfy { !$0.text.isEmpty },
            "every label is the core's text")
    }

    func testTheHarbourAndTheShelfDrawMajorLinesAmongTheMinor() async throws {
        let host = ElevationHost(body: try recordedElevation())
        let store = GridStore()
        // The whole recorded export at zoom 9, out to the shelf (deeper than 80 m).
        let harbour = MapRegion(west: -74.2, south: 40.0, east: -73.0, north: 40.8, zoom: 9)
        await loader(host, store).load(harbour)?.value
        let area = DepthArea.around(harbour)
        let lines = try store.contourLines(
            west: area.west, south: area.south, east: area.east, north: area.north,
            zoom: area.zoom, settingsJson: try Settings.defaults().json())
        let contours = ContourSet(lines)
        XCTAssertTrue(contours.lines.contains { $0.major })
        XCTAssertTrue(contours.lines.contains { !$0.major })
        XCTAssertLessThanOrEqual(contours.vertexCount, 60_000)
    }
}
