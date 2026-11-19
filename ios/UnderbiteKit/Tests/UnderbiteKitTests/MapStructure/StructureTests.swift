import Foundation
import MapKit
import XCTest

@testable import UnderbiteKit

/// A file of the repository's recorded fixtures.
private func fixture(_ path: String) throws -> Data {
    let root = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent()  // MapStructure
        .deletingLastPathComponent()  // UnderbiteKitTests
        .deletingLastPathComponent()  // Tests
        .deletingLastPathComponent()  // UnderbiteKit
        .deletingLastPathComponent()  // ios
        .deletingLastPathComponent()
    return try Data(contentsOf: root.appending(path: "tests/fixtures/raw/" + path))
}

/// The harbour band's layer list as the service gives it, cut to the point layers and one other.
private let harbourLayers = Data(
    #"{"layers":[{"id":32,"name":"Harbor.Fishing_Facility_point"},{"id":33,"name":"Harbor.Obstruction_point"},{"id":34,"name":"Harbor.Underwater_Awash_Rock_point"},{"id":36,"name":"Harbor.Wreck_point"},{"id":158,"name":"Wreck_area"}]}"#
        .utf8)

/// ENC Direct's transient answer: status 200 and an error body.
private let transient = Data(
    #"{"error":{"code":500,"message":"Could not find service 'encdirect/enc_harbour' of type 'MapServer'.","details":[]}}"#
        .utf8)

/// A fetch host that answers from the recorded ENC Direct, NYSDEC and Indiana fixtures by URL,
/// records what was asked and keeps nothing; no network.
private final class StructureHost: FetchHost, @unchecked Sendable {
    private let lock = NSLock()
    private var asked: [HostRequest] = []
    /// How many more wreck queries answer with the transient error body.
    private var failures: Int

    let wrecks: Data
    let obstructions: Data
    let empty: Data
    let reefs: Data
    let lake: Data

    init(failures: Int = 0) throws {
        self.failures = failures
        wrecks = try fixture("ncds/encdirect-enc-harbour-wreck-point-query-lower-bay-2026-11-17.json")
        obstructions = try fixture(
            "ncds/encdirect-enc-harbour-obstruction-point-query-bight-2026-11-17.json")
        empty = try fixture(
            "ncds/encdirect-enc-approach-wreck-point-query-offshore-empty-2026-11-17.json")
        reefs = try fixture("state-maps/ny-artificial-reefs-socrata-2026-11-17.json")
        lake = try fixture("state-maps/in-lake-bathymetry-mill-pond-lake.synthetic.geojson")
    }

    var requests: [HostRequest] { lock.withLock { asked } }
    var urls: [String] { requests.map(\.url) }

    func send(request: HostRequest) async throws -> HostResponse {
        let url = request.url
        let body: Data = lock.withLock {
            asked.append(request)
            if url.contains("encdirect"), url.hasSuffix("MapServer?f=json") { return harbourLayers }
            if url.contains("data.ny.gov") { return reefs }
            if url.contains("gisdata.in.gov") { return lake }
            // Later pages are empty.
            if !url.contains("resultOffset=0") { return empty }
            if url.contains("MapServer/36/query") {
                if failures > 0 {
                    failures -= 1
                    return transient
                }
                return wrecks
            }
            if url.contains("MapServer/33/query") { return obstructions }
            return empty
        }
        return HostResponse(
            status: 200, body: body, contentType: "application/json", date: nil, expires: nil,
            lastModified: nil, etag: nil, retryAfter: nil, rateLimitRemaining: nil)
    }

    func load(namespace: String, key: String) -> String? { nil }
    func store(namespace: String, key: String, value: String) {}
    func saveUsage(ledgerJson: String) throws {}
}

/// Records the pauses asked for, without waiting.
private final class Pauses: @unchecked Sendable {
    private let lock = NSLock()
    private var seen: [Duration] = []
    var all: [Duration] { lock.withLock { seen } }
    func record(_ duration: Duration) { lock.withLock { seen.append(duration) } }
}

/// 2026-11-17T12:00:00Z.
private let now: Int64 = 1_794_916_800

/// Structure on the map: boxes asked for and kept, the ENC Direct flow through the core, the
/// reefs and lake contours, the marks made of them, their grouping and the taps they take.
@MainActor
final class StructureTests: XCTestCase {
    /// The Lower Bay at zoom 13.5 (the harbour band).
    private let bay = MapRegion(west: -74.06, south: 40.54, east: -73.98, north: 40.60, zoom: 13.5)

    private func loader(_ host: StructureHost, pauses: Pauses = Pauses(), clock: @escaping @Sendable () -> Int64 = { now })
        -> StructureLoader
    {
        StructureLoader(host: host, clock: clock, pause: { pauses.record($0) })
    }

    private let points = StructureLoader.Wants(points: true)

    // MARK: - Boxes

    func testABoxIsTheViewWithAMarginGrownToTiles() {
        let box = StructureBox.around(bay, zoom: 13)
        XCTAssertTrue(box.covers(bay))
        XCTAssertLessThan(box.west, bay.west - 0.019)
        XCTAssertGreaterThan(box.east, bay.east + 0.019)
        XCTAssertLessThan(box.south, bay.south)
        XCTAssertGreaterThan(box.north, bay.north)
        // On the tile edges of zoom 13: a whole number of tiles across.
        let tiles = (box.east - box.west) / 360 * pow(2, 13)
        XCTAssertEqual(tiles, tiles.rounded(), accuracy: 1e-6)
        // The same view asks for the same box; one a little apart is served by it.
        XCTAssertEqual(StructureBox.around(bay, zoom: 13).key, box.key)
        var nudged = bay
        nudged.west += 0.005
        nudged.east += 0.005
        XCTAssertTrue(box.covers(nudged))
        // Far away it does not cover.
        var far = bay
        far.west += 0.5
        far.east += 0.5
        XCTAssertFalse(box.covers(far))
    }

    func testABoxStaysInsideTheWorld() {
        let edge = MapRegion(west: -180, south: -86, east: -179, north: 86, zoom: 2)
        let box = StructureBox.around(edge, zoom: 2)
        XCTAssertGreaterThanOrEqual(box.west, -180)
        XCTAssertLessThanOrEqual(box.east, 180)
        XCTAssertGreaterThanOrEqual(box.south, -StructureBox.maxLatitude)
        XCTAssertLessThanOrEqual(box.north, StructureBox.maxLatitude)
        XCTAssertLessThan(box.west, box.east)
        XCTAssertLessThan(box.south, box.north)
    }

    func testHeldBoxesServeTheViewsTheyCoverAndTheNewestAreKept() {
        var held = StructureHeld<Int>(limit: 2)
        let box = StructureBox.around(bay, zoom: 13)
        held.hold(1, box: box, tag: "harbour")
        XCTAssertEqual(held.covering(bay, tag: "harbour"), 1)
        XCTAssertNil(held.covering(bay, tag: "approach"), "a tag of its own")
        var far = bay
        far.west += 1
        far.east += 1
        held.hold(2, box: StructureBox.around(far, zoom: 13), tag: "harbour")
        held.hold(3, box: StructureBox.around(far, zoom: 12), tag: "harbour")
        XCTAssertEqual(held.count, 2)
        XCTAssertNil(held.covering(bay, tag: "harbour"), "the oldest went")
        XCTAssertEqual(held.covering(far, tag: "harbour"), 3, "the newest first")
    }

    // MARK: - Zoom

    func testZoomStepsAndShallowestZooms() {
        XCTAssertEqual(StructureMarks.zoomStep(13.5), 13)
        XCTAssertEqual(StructureMarks.zoomStep(12.8), 13, "a view settled a hair short counts")
        XCTAssertEqual(StructureMarks.zoomStep(-1), 0)
        XCTAssertEqual(StructureMarks.zoomStep(40), 22)
        XCTAssertTrue(StructureMarks.drawn(minZoom: 9, at: 8.8))
        XCTAssertFalse(StructureMarks.drawn(minZoom: 9, at: 8.5))
        XCTAssertTrue(StructureMarks.drawn(minZoom: nil, at: 1))
    }

    // MARK: - Charted points

    func testPointsComeThroughTheBandsOwnLayerIds() async throws {
        let host = try StructureHost()
        let loader = loader(host)
        await loader.load(bay, wants: points, settingsJSON: "{}")?.value
        let urls = host.urls
        XCTAssertTrue(urls[0].hasSuffix("/encdirect/enc_harbour/MapServer?f=json"), urls[0])
        // The ids read from the list, never others: wrecks 36, obstructions 33, rocks 34.
        XCTAssertTrue(urls.contains { $0.contains("enc_harbour/MapServer/36/query") })
        XCTAssertTrue(urls.contains { $0.contains("enc_harbour/MapServer/33/query") })
        XCTAssertTrue(urls.contains { $0.contains("enc_harbour/MapServer/34/query") })
        XCTAssertFalse(urls.contains { $0.contains("MapServer/32/") || $0.contains("MapServer/158/") })
        // Asked for the box, not the view, rounded outward to hundredths of a degree.
        let box = StructureBox.around(bay, zoom: 13)
        let sent = try XCTUnwrap(
            urls[1].components(separatedBy: CharacterSet(charactersIn: "?&"))
                .first { $0.hasPrefix("geometry=") }?
                .dropFirst("geometry=".count).split(separator: ",").map { String($0) })
        XCTAssertEqual(sent.count, 4, urls[1])
        XCTAssertTrue(sent.allSatisfy { ($0.split(separator: ".").last?.count ?? 0) <= 2 }, urls[1])
        let edges = sent.compactMap(Double.init)
        XCTAssertTrue(
            edges[0] <= box.west && edges[1] <= box.south && edges[2] >= box.east
                && edges[3] >= box.north, urls[1])
        // Five wrecks and five obstructions; the full pages asked a second page each.
        XCTAssertEqual(loader.points.count, 10)
        XCTAssertEqual(loader.points.filter { $0.kind == .wreck }.count, 5)
        XCTAssertEqual(loader.points.filter { $0.kind == .obstruction }.count, 5)
        XCTAssertEqual(urls.filter { $0.contains("resultOffset=5") }.count, 2)
        XCTAssertFalse(loader.busy)
        for point in loader.points {
            XCTAssertFalse(point.title.isEmpty)
            // No charted depth: the core's reason instead.
            XCTAssertFalse(point.depthNote.isEmpty)
        }
        XCTAssertTrue(loader.points.contains { $0.category?.lowercased().contains("dangerous wreck") == true })
    }

    func testAViewInsideAHeldBoxAsksNothingAndTheLayerListIsKept() async throws {
        let host = try StructureHost()
        let loader = loader(host)
        await loader.load(bay, wants: points, settingsJSON: "{}")?.value
        let first = host.urls.count
        var panned = bay
        panned.west += 0.005
        panned.east += 0.005
        panned.zoom = 14.2
        XCTAssertNil(loader.load(panned, wants: points, settingsJSON: "{}"), "the box covers it")
        XCTAssertEqual(host.urls.count, first)
        XCTAssertEqual(loader.points.count, 10)
        // Out of the box: the points again, the list of layers not.
        var away = bay
        away.west -= 0.3
        away.east -= 0.3
        await loader.load(away, wants: points, settingsJSON: "{}")?.value
        let more = host.urls.dropFirst(first)
        XCTAssertFalse(more.isEmpty)
        XCTAssertFalse(more.contains { $0.hasSuffix("MapServer?f=json") })
    }

    func testTheSameBoxIsNotAskedTwiceWhileItIsBeingFetched() async throws {
        let host = try StructureHost()
        let loader = loader(host)
        let first = loader.load(bay, wants: points, settingsJSON: "{}")
        XCTAssertTrue(loader.busy)
        XCTAssertNil(loader.load(bay, wants: points, settingsJSON: "{}"))
        await first?.value
        XCTAssertEqual(host.urls.filter { $0.hasSuffix("MapServer?f=json") }.count, 1)
        XCTAssertEqual(host.urls.filter { $0.contains("36/query") && $0.contains("resultOffset=0") }.count, 1)
    }

    func testNoPointsAreAskedAboveTheShallowestBand() async throws {
        let host = try StructureHost()
        let loader = loader(host)
        await loader.load(bay, wants: points, settingsJSON: "{}")?.value
        XCTAssertFalse(loader.points.isEmpty)
        var wide = bay
        wide.zoom = 8
        XCTAssertNil(loader.load(wide, wants: points, settingsJSON: "{}"))
        XCTAssertTrue(loader.points.isEmpty, "nothing drawn at that zoom")
        // Switched off: nothing either.
        XCTAssertNil(loader.load(bay, wants: .init(), settingsJSON: "{}"))
        XCTAssertTrue(loader.points.isEmpty)
    }

    func testATransientErrorIsAskedAgainPastTheCacheAfterThePauseTheCoreGives() async throws {
        let host = try StructureHost(failures: 1)
        let pauses = Pauses()
        let loader = loader(host, pauses: pauses)
        await loader.load(bay, wants: points, settingsJSON: "{}")?.value
        let wrecks = host.requests.filter { $0.url.contains("36/query") && $0.url.contains("resultOffset=0") }
        XCTAssertEqual(wrecks.count, 2)
        XCTAssertFalse(wrecks[0].fresh)
        XCTAssertTrue(wrecks[1].fresh, "the error body may be in the cache")
        XCTAssertEqual(pauses.all, [.seconds(3)])
        XCTAssertEqual(loader.points.filter { $0.kind == .wreck }.count, 5)
    }

    func testABoxThatFailsIsLeftAloneForAMinute() async throws {
        let host = try StructureHost(failures: 100)
        let clock = TestClock(now)
        let loader = loader(host, clock: { clock.now })
        await loader.load(bay, wants: points, settingsJSON: "{}")?.value
        // Retried once (the core's rule for a transient body), then given up.
        XCTAssertEqual(host.urls.filter { $0.contains("36/query") }.count, 2)
        // The kinds that answered are shown.
        XCTAssertEqual(loader.points.filter { $0.kind == .obstruction }.count, 5)
        let asked = host.urls.count
        XCTAssertNil(loader.load(bay, wants: points, settingsJSON: "{}"))
        XCTAssertEqual(host.urls.count, asked)
        clock.now += StructureLoader.coolOff + 1
        await loader.load(bay, wants: points, settingsJSON: "{}")?.value
        XCTAssertGreaterThan(host.urls.count, asked)
    }

    func testAFailedBoxIsAskedForAgainByItselfAfterTheCoolOff() async throws {
        let host = try StructureHost(failures: 2)
        let clock = TestClock(now)
        let loader = StructureLoader(
            host: host, clock: { clock.now },
            pause: { clock.now += $0.components.seconds })
        await loader.load(bay, wants: points, settingsJSON: "{}")?.value
        XCTAssertTrue(loader.points.filter { $0.kind == .wreck }.isEmpty)
        for _ in 0..<300 where loader.points.filter({ $0.kind == .wreck }).isEmpty {
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTAssertEqual(loader.points.filter { $0.kind == .wreck }.count, 5)
        let wrecks = host.urls.filter { $0.contains("36/query") && $0.contains("resultOffset=0") }
        XCTAssertEqual(wrecks.count, 3, "two that failed, then the retry")
    }

    func testNewSettingsWordThePointsAgain() async throws {
        let host = try StructureHost()
        let loader = loader(host)
        await loader.load(bay, wants: points, settingsJSON: "{}")?.value
        let asked = host.urls.count
        let defaults = try Settings.defaults().json()
        await loader.load(bay, wants: points, settingsJSON: defaults)?.value
        XCTAssertGreaterThan(host.urls.count, asked)
    }

    // MARK: - Reefs and lake contours

    func testTheReefsAreAskedOnceAndCarryTheStatesDepths() async throws {
        let host = try StructureHost()
        let loader = loader(host)
        let wants = StructureLoader.Wants(reefs: true)
        await loader.load(bay, wants: wants, settingsJSON: "{}")?.value
        XCTAssertEqual(loader.reefs.count, 16)
        XCTAssertTrue(loader.reefs.contains { $0.name == "Atlantic Beach Reef" })
        XCTAssertTrue(loader.reefs.allSatisfy { $0.corners.count == 4 })
        let atlantic = try XCTUnwrap(loader.reefs.first { $0.name == "Atlantic Beach Reef" })
        let depth = try XCTUnwrap(atlantic.depthText)
        XCTAssertTrue(depth.contains("55") && depth.contains("64"), depth)
        var away = bay
        away.west -= 1
        away.east -= 1
        XCTAssertNil(loader.load(away, wants: wants, settingsJSON: "{}"))
        XCTAssertEqual(host.urls.filter { $0.contains("data.ny.gov") }.count, 1)
        XCTAssertNil(loader.load(away, wants: .init(), settingsJSON: "{}"))
        XCTAssertTrue(loader.reefs.isEmpty)
    }

    func testLakeContoursComeForTheStatesDataOnly() async throws {
        let host = try StructureHost()
        let loader = loader(host)
        let wants = StructureLoader.Wants(contours: ["lake-contours-in"])
        // Over the Lower Bay: outside Indiana's data, nothing is asked.
        XCTAssertNil(loader.load(bay, wants: wants, settingsJSON: "{}"))
        XCTAssertEqual(loader.contours["lake-contours-in"], [])
        XCTAssertTrue(host.urls.isEmpty)
        let lake = MapRegion(west: -86.51, south: 39.49, east: -86.49, north: 39.51, zoom: 15)
        await loader.load(lake, wants: wants, settingsJSON: "{}")?.value
        XCTAssertTrue(host.urls.allSatisfy { $0.contains("gisdata.in.gov") })
        let lines = try XCTUnwrap(loader.contours["lake-contours-in"])
        XCTAssertFalse(lines.isEmpty)
        XCTAssertTrue(lines.allSatisfy { $0.label.hasSuffix("ft") }, lines.map(\.label).description)
        XCTAssertTrue(lines.contains { $0.major })
        let placed = LakeContours.place(lines)
        XCTAssertEqual(placed.lines.count, lines.filter { $0.points.count >= 2 }.count)
        // Switched off: the lines go.
        XCTAssertNil(loader.load(lake, wants: .init(), settingsJSON: "{}"))
        XCTAssertNil(loader.contours["lake-contours-in"])
    }

    // MARK: - Marks

    private func parsedPoints() throws -> [StructurePoint] {
        let host = try StructureHost()
        let response = HostResponse(
            status: 200, body: host.wrecks, contentType: "application/json", date: nil,
            expires: nil, lastModified: nil, etag: nil, retryAfter: nil, rateLimitRemaining: nil)
        return try parseStructurePoints(kind: .wreck, response: response, attempt: 0, settingsJson: "{}").points
    }

    private func reef(_ name: String, lat: Double, lon: Double) -> ReefInfo {
        let d = 0.004
        return ReefInfo(
            name: name, lat: lat, lon: lon,
            corners: [
                MapPoint(lat: lat + d, lon: lon - d), MapPoint(lat: lat + d, lon: lon + d),
                MapPoint(lat: lat - d, lon: lon + d), MapPoint(lat: lat - d, lon: lon - d),
            ], bodyOfWater: "Atlantic Ocean", depthText: nil, comments: nil)
    }

    func testMarksLeadWithReefsAndKeepEachPointOnce() throws {
        let wrecks = try parsedPoints()
        XCTAssertEqual(wrecks.count, 5)
        let reefs = [reef("Rockaway Reef", lat: 40.54, lon: -73.84)]
        let marks = StructureMarks.marks(
            points: wrecks + wrecks, pointsLayer: "enc-structure-points", reefs: reefs,
            reefsLayer: "artificial-reefs-ny")
        XCTAssertEqual(marks.count, 6)
        XCTAssertEqual(marks.first?.kind, .reef)
        XCTAssertEqual(marks.first?.title, "Rockaway Reef")
        XCTAssertEqual(marks.first?.subtitle, "Atlantic Ocean")
        XCTAssertTrue(marks.dropFirst().allSatisfy { $0.kind == .wreck && $0.layer == "enc-structure-points" })
        // A layer not drawn makes no marks.
        XCTAssertEqual(
            StructureMarks.marks(points: wrecks, pointsLayer: nil, reefs: reefs, reefsLayer: "artificial-reefs-ny").count, 1)
        XCTAssertTrue(StructureMarks.areas(reefs: reefs, reefsLayer: nil).isEmpty)
    }

    func testKindsAreToldApartWithoutColour() {
        let outlines = StructureMarkKind.allCases.map { "\($0.outline)\($0.symbol)" }
        XCTAssertEqual(Set(outlines).count, outlines.count)
        let symbols = StructureMarkKind.allCases.map(\.symbol)
        XCTAssertEqual(Set(symbols).count, symbols.count, "each kind has its own symbol")
        for kind in StructureMarkKind.allCases {
            XCTAssertGreaterThanOrEqual(kind.colour.contrast(with: kind.glyph), 3, "\(kind)")
        }
    }

    func testNearMarksShareOneAndATapMeetsTheNearest() throws {
        let wrecks = try parsedPoints()
        let marks = StructureMarks.marks(
            points: wrecks, pointsLayer: "enc-structure-points", reefs: [], reefsLayer: nil)
        // Zoomed far out, the Lower Bay's wrecks are one mark; zoomed in, five.
        let out = StructureMarks.clusters(marks, mapPointsPerPoint: StructureMarks.mapPointsPerPoint(zoom: 7))
        XCTAssertEqual(out.count, 1)
        XCTAssertEqual(out[0].marks.count, 5)
        let perPoint = StructureMarks.mapPointsPerPoint(zoom: 15)
        let near = StructureMarks.clusters(marks, mapPointsPerPoint: perPoint)
        XCTAssertEqual(near.count, 5)
        let target = near[2]
        let hit = StructureMarks.hit(
            near, x: target.x + 10 * perPoint, y: target.y, mapPointsPerPoint: perPoint)
        XCTAssertEqual(hit?.id, target.id)
        XCTAssertNil(
            StructureMarks.hit(near, x: target.x + 400 * perPoint, y: target.y + 400 * perPoint, mapPointsPerPoint: perPoint))
    }

    func testATapInsideAReefAreaFindsIt() throws {
        let area = try XCTUnwrap(StructureArea(reef("Kismet Reef", lat: 40.6, lon: -73.2), layer: "artificial-reefs-ny"))
        let inside = MKMapPoint(CLLocationCoordinate2D(latitude: 40.601, longitude: -73.199))
        let outside = MKMapPoint(CLLocationCoordinate2D(latitude: 40.61, longitude: -73.2))
        XCTAssertTrue(area.contains(x: inside.x, y: inside.y))
        XCTAssertFalse(area.contains(x: outside.x, y: outside.y))
        XCTAssertEqual(area.id, "reef/Kismet Reef")
        XCTAssertNil(
            StructureArea(
                ReefInfo(name: "x", lat: 0, lon: 0, corners: [], bodyOfWater: nil, depthText: nil, comments: nil),
                layer: "artificial-reefs-ny"))
    }
}

/// A clock the test moves.
private final class TestClock: @unchecked Sendable {
    private let lock = NSLock()
    private var value: Int64

    init(_ start: Int64) {
        value = start
    }

    var now: Int64 {
        get { lock.withLock { value } }
        set { lock.withLock { value = newValue } }
    }
}
