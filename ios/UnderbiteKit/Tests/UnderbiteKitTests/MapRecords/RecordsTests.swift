import Foundation
import MapKit
import XCTest

@testable import UnderbiteKit

/// The species records and beach forecasts on the map, with the real core and made-up answers:
/// blurred records as circles and never points, nonindigenous records marked, marks grouped and
/// met by taps, the lookup's rounds and waits, its state kept, and no observer in what crosses.
final class RecordsTests: XCTestCase {
    /// A directory of its own for each test (XCTest makes an instance per test).
    private let directory = FileManager.default.temporaryDirectory
        .appending(path: "underbite-records-\(UUID().uuidString)", directoryHint: .isDirectory)

    override func tearDown() {
        try? FileManager.default.removeItem(at: directory)
    }

    /// 2026-11-17 12:00 UTC.
    private let now: Int64 = 1_794_916_800

    /// New York Harbor and the lower Hudson, about 0.3 degree across.
    private let harbor = NearbyArea(west: -74.2, south: 40.45, east: -73.85, north: 40.75)

    private func pin(
        _ id: String, source: String = "gbif", lat: Double = 40.6, lon: Double = -74.0,
        radius: Double? = 30, obscured: Bool = false
    ) -> RecordPin {
        RecordPin(
            id: id, source: source, sourceName: source, taxon: "Morone saxatilis",
            commonName: "Striped Bass", title: "Striped Bass", lat: lat, lon: lon, radiusM: radius,
            obscured: obscured, introduced: source == "nas",
            introducedLabel: source == "nas" ? "Introduced species" : nil,
            seen: "Seen Nov 1 2026", position: nil, licence: "CC0",
            licenceUrl: nil, link: nil, linkTitle: nil, credit: "GBIF.org", creditUrl: nil,
            photo: nil, accessibility: "Striped Bass")
    }

    // MARK: - Points and areas

    func testBlurredRecordsAreCirclesOfTheirRadiusAndNeverPoints() {
        let split = RecordMarks.split([
            pin("gbif:1"),
            pin("inaturalist:2", source: "inaturalist", radius: 28_000, obscured: true),
            pin("inaturalist:3", source: "inaturalist", radius: nil, obscured: true),
        ])
        XCTAssertEqual(split.points.map(\.id), ["gbif:1"])
        XCTAssertEqual(split.areas.map(\.id), ["inaturalist:2"])
        let area = split.areas[0]
        XCTAssertEqual(area.radiusMetres, 28_000)
        XCTAssertEqual(area.radius, 28_000 * MKMapPointsPerMeterAtLatitude(40.6), accuracy: 1e-6)
        XCTAssertFalse(area.invasive)
    }

    func testNonindigenousRecordsAreMarkedAsSuch() {
        let split = RecordMarks.split([
            pin("usgs-nas:1", source: "usgs-nas"), pin("gbif:2", lat: 40.7),
            pin("usgs-nas:3", source: "usgs-nas", radius: 5_000, obscured: true),
        ])
        XCTAssertEqual(split.points.map(\.kind), [.invasive, .record])
        XCTAssertEqual(split.areas.map(\.invasive), [true])
    }

    func testPositionsOutOfRangeAreLeftOut() {
        let split = RecordMarks.split([pin("gbif:1", lat: 91), pin("gbif:2", lon: .nan)])
        XCTAssertTrue(split.points.isEmpty)
        XCTAssertTrue(split.areas.isEmpty)
    }

    func testLargerCirclesAreDrawnFirst() {
        let split = RecordMarks.split([
            pin("a", radius: 100, obscured: true), pin("b", radius: 9_000, obscured: true),
            pin("c", radius: 2_000, obscured: true),
        ])
        XCTAssertEqual(split.areas.map(\.id), ["b", "c", "a"])
    }

    // MARK: - Grouping and taps

    func testMarksGroupOnlyWithTheirOwnKind() {
        let perPoint = RecordMarks.mapPointsPerPoint(zoom: 12)
        let items = [
            RecordItem(id: "1", kind: .record, latitude: 40.6, longitude: -74.0),
            RecordItem(id: "2", kind: .record, latitude: 40.6001, longitude: -74.0001),
            RecordItem(id: "3", kind: .invasive, latitude: 40.6001, longitude: -74.0),
            RecordItem(id: "4", kind: .record, latitude: 40.7, longitude: -74.0),
        ]
        let marks = RecordMarks.clusters(items, mapPointsPerPoint: perPoint)
        XCTAssertEqual(marks.map { $0.items.map(\.id) }, [["1", "2"], ["3"], ["4"]])
        XCTAssertEqual(marks.map(\.kind), [.record, .invasive, .record])
        // Zoomed far out, the records join; the nonindigenous one stays apart.
        let far = RecordMarks.clusters(
            items, mapPointsPerPoint: RecordMarks.mapPointsPerPoint(zoom: 6))
        XCTAssertEqual(far.map { $0.items.map(\.id) }, [["1", "2", "4"], ["3"]])
    }

    func testABeachGroupTakesItsHighestLevel() {
        let mark = RecordMark(items: [
            RecordItem(id: "a", kind: .beach, latitude: 27, longitude: -82.5, level: "low"),
            RecordItem(id: "b", kind: .beach, latitude: 27, longitude: -82.5, level: "high"),
            RecordItem(id: "c", kind: .beach, latitude: 27, longitude: -82.5, level: "unknown"),
        ])
        XCTAssertEqual(mark.level, "high")
        XCTAssertLessThan(RecordMarks.rank("unknown"), RecordMarks.rank("very_low"))
        XCTAssertLessThan(RecordMarks.rank("moderate"), RecordMarks.rank("high"))
    }

    func testATapMeetsTheNearestMarkWithinReach() {
        let perPoint = RecordMarks.mapPointsPerPoint(zoom: 12)
        let a = RecordItem(id: "a", kind: .record, latitude: 40.6, longitude: -74.0)
        let marks = RecordMarks.clusters([a], mapPointsPerPoint: perPoint)
        let near = MapXY(x: a.at.x + 15 * perPoint, y: a.at.y)
        let far = MapXY(x: a.at.x + 30 * perPoint, y: a.at.y)
        XCTAssertEqual(
            RecordMarks.hit(marks, at: near, mapPointsPerPoint: perPoint)?.id, marks[0].id)
        XCTAssertNil(RecordMarks.hit(marks, at: far, mapPointsPerPoint: perPoint))
    }

    func testASmallCircleIsMetInsideALargeOneOnlyNearItsEdge() {
        let perPoint = RecordMarks.mapPointsPerPoint(zoom: 12)
        let small = RecordArea(
            id: "small", latitude: 40.6, longitude: -74.0, radiusMetres: 500, invasive: false)
        let large = RecordArea(
            id: "large", latitude: 40.6, longitude: -74.0, radiusMetres: 27_000, invasive: false)
        XCTAssertLessThan(small.radius, RecordMarks.wholeAreaRadius * perPoint)
        XCTAssertGreaterThan(large.radius, RecordMarks.wholeAreaRadius * perPoint)
        let centre = small.at
        XCTAssertEqual(
            RecordMarks.hitAreas([large, small], at: centre, mapPointsPerPoint: perPoint).map(\.id),
            ["small"])
        let edge = MapXY(x: large.at.x + large.radius - 5 * perPoint, y: large.at.y)
        XCTAssertEqual(
            RecordMarks.hitAreas([large, small], at: edge, mapPointsPerPoint: perPoint).map(\.id),
            ["large"])
        let inside = MapXY(x: large.at.x + large.radius / 2, y: large.at.y)
        XCTAssertTrue(
            RecordMarks.hitAreas([large, small], at: inside, mapPointsPerPoint: perPoint).isEmpty)
    }

    func testLayersAreDrawnFromTheirShallowestZoom() {
        XCTAssertFalse(RecordMarks.drawn(minZoom: 10, zoom: 9.5))
        XCTAssertTrue(RecordMarks.drawn(minZoom: 10, zoom: 10))
        XCTAssertTrue(RecordMarks.drawn(minZoom: 10, zoom: 9.995))
        XCTAssertTrue(RecordMarks.drawn(minZoom: nil, zoom: 2))
    }

    func testANextPageAddsOnlyNewRecords() {
        let merged = RecordMarks.merge([pin("a"), pin("b")], [pin("b"), pin("c")])
        XCTAssertEqual(merged.map(\.id), ["a", "b", "c"])
    }

    // MARK: - The query

    func testTheNASStateComesFromAUSRegion() {
        XCTAssertEqual(RecordsLookup.nasState(region: "US-NY"), "NY")
        XCTAssertEqual(RecordsLookup.nasState(region: " us-nj "), "NJ")
        XCTAssertNil(RecordsLookup.nasState(region: ""))
        XCTAssertNil(RecordsLookup.nasState(region: "CA-ON"))
        XCTAssertNil(RecordsLookup.nasState(region: "US-NYC"))
        XCTAssertNil(RecordsLookup.nasState(region: "US-1A"))
        let query = RecordsLookup.query(
            scientificName: "Morone saxatilis", page: 2, region: "US-NY")
        XCTAssertEqual(query.nasState, "NY")
        XCTAssertEqual(query.page, 2)
        XCTAssertTrue(query.sources.isEmpty)
        XCTAssertNil(RecordsLookup.query(scientificName: "x", page: 1, region: "").nasState)
    }

    func testTheAreaIsTheRegionOnScreen() {
        let region = MapRegion(west: -74.2, south: 40.45, east: -73.85, north: 40.75, zoom: 11)
        XCTAssertEqual(RecordsLookup.area(region), harbor)
    }

    // MARK: - The lookup with the core

    /// An iNaturalist page as the service answers (a made-up one): a record under CC BY-NC with
    /// an open photo, a blurred one, one with no licence, and an observer the core must drop.
    private let inaturalist = Data(
        #"""
        {"total_results": 3, "page": 1, "per_page": 3, "results": [
          {"id": 9100001, "uri": "https://www.inaturalist.org/observations/9100001",
           "observed_on": "2026-11-01", "quality_grade": "research", "license_code": "cc-by-nc",
           "geojson": {"type": "Point", "coordinates": [-74.0, 40.6]},
           "positional_accuracy": 10, "public_positional_accuracy": 10, "obscured": false,
           "user": {"login": "harborwatcher", "name": "Pat Example"},
           "taxon": {"id": 51392, "name": "Morone saxatilis", "rank": "species",
                     "preferred_common_name": "Striped Bass"},
           "photos": [{"id": 9000001, "url": "https://example.invalid/photos/9000001/square.jpg",
                       "license_code": "cc-by-nc", "attribution": "(c) Pat Example"}]},
          {"id": 9100002, "uri": "https://www.inaturalist.org/observations/9100002",
           "observed_on": "2026-10-30", "quality_grade": "research", "license_code": "cc-by-nc",
           "geojson": {"type": "Point", "coordinates": [-73.95, 40.65]},
           "positional_accuracy": 20000, "public_positional_accuracy": 28000, "obscured": true,
           "user": {"login": "harborwatcher", "name": "Pat Example"},
           "taxon": {"id": 51392, "name": "Morone saxatilis", "rank": "species",
                     "preferred_common_name": "Striped Bass"},
           "photos": []},
          {"id": 9100003, "uri": "https://www.inaturalist.org/observations/9100003",
           "observed_on": "2026-10-16", "quality_grade": "research", "license_code": null,
           "geojson": {"type": "Point", "coordinates": [-74.05, 40.55]},
           "positional_accuracy": 5, "public_positional_accuracy": 5, "obscured": false,
           "taxon": {"id": 51392, "name": "Morone saxatilis", "rank": "species",
                     "preferred_common_name": "Striped Bass"},
           "photos": []}
        ]}
        """#.utf8)

    private let gbif = Data(
        #"{"offset":0,"limit":300,"endOfRecords":true,"count":0,"results":[]}"#.utf8)

    private func response(_ status: UInt16, _ body: Data) -> HostResponse {
        HostResponse(
            status: status, body: body, contentType: "application/json", date: nil, expires: nil,
            lastModified: nil, etag: nil, retryAfter: nil, rateLimitRemaining: nil)
    }

    /// Answers iNaturalist with its page and GBIF with an empty one; counts what was sent.
    private func fakeServices(_ log: Log) -> RecordsLookup.Send {
        let inaturalist = inaturalist, gbif = gbif
        return { request in
            await log.add(request.url)
            if request.url.contains("inaturalist.org") {
                return HostResponse(
                    status: 200, body: inaturalist, contentType: "application/json", date: nil,
                    expires: nil, lastModified: nil, etag: nil, retryAfter: nil,
                    rateLimitRemaining: nil)
            }
            if request.url.contains("gbif.org") {
                return HostResponse(
                    status: 200, body: gbif, contentType: "application/json", date: nil,
                    expires: nil, lastModified: nil, etag: nil, retryAfter: nil,
                    rateLimitRemaining: nil)
            }
            throw HostError.Failed(message: "offline")
        }
    }

    func testRecordsCarryTheirLicenceAndNoObserver() async throws {
        let log = Log()
        let plan = try planRecords(
            area: harbor,
            query: RecordsLookup.query(scientificName: "Morone saxatilis", page: 1, region: ""),
            stateJson: "", settingsJson: "{}", now: now)
        let results = await RecordsLookup.run(
            plan, send: fakeServices(log), sleep: { _ in }, clock: { [now] in now })
        let urls = await log.urls
        XCTAssertTrue(urls.contains { $0.contains("inaturalist.org") })
        XCTAssertTrue(urls.contains { $0.contains("gbif.org") })
        XCTAssertFalse(urls.contains { $0.contains("nas.er.usgs.gov") }, "no state, no NAS")

        // The record without a licence is not shown.
        XCTAssertEqual(results.records.count, 2)
        let open = try XCTUnwrap(results.records.first { !$0.obscured })
        XCTAssertEqual(open.licence, "CC BY-NC")
        XCTAssertEqual(open.source, "inaturalist")
        XCTAssertNotNil(open.link)
        let photo = try XCTUnwrap(open.photo)
        XCTAssertEqual(photo.licence, "CC BY-NC")
        XCTAssertTrue(photo.caption.contains("CC BY-NC"))

        let blurred = try XCTUnwrap(results.records.first { $0.obscured })
        XCTAssertNil(blurred.photo)
        let split = RecordMarks.split(results.records)
        XCTAssertEqual(split.points.map(\.id), [open.id])
        XCTAssertEqual(split.areas.map(\.id), [blurred.id])
        XCTAssertEqual(split.areas[0].radiusMetres, 28_000)

        // Nobody's name or login crosses, in any field.
        let everything = String(describing: results)
        XCTAssertFalse(everything.contains("Pat Example"))
        XCTAssertFalse(everything.contains("harborwatcher"))

        XCTAssertFalse(results.notes.isEmpty, "what records are")
        XCTAssertNotNil(results.stateJson, "the iNaturalist pace is kept")
    }

    @MainActor
    func testTheLookupStateIsKeptBetweenLookups() async throws {
        let store = NearbyStateStore(files: AppFiles(directory: directory))
        XCTAssertEqual(store.load(), "")
        let (plan, base) = try store.plan { kept in
            try planRecords(
                area: harbor,
                query: RecordsLookup.query(scientificName: "Morone saxatilis", page: 1, region: ""),
                stateJson: kept, settingsJson: "{}", now: now)
        }
        let at = now
        let results = await RecordsLookup.run(
            plan, send: fakeServices(Log()), sleep: { _ in }, clock: { at })
        let state = try XCTUnwrap(results.stateJson)
        store.save(state, base: base)
        let kept = store.load()
        XCTAssertFalse(kept.isEmpty)

        // The next lookup reads it: a second iNaturalist request a moment later waits its turn.
        let (next, _) = try store.plan { kept in
            try planRecords(
                area: harbor,
                query: RecordsLookup.query(scientificName: "Morone saxatilis", page: 1, region: ""),
                stateJson: kept, settingsJson: "{}", now: now)
        }
        let wait = next.requests().first { $0.source == "inaturalist" }?.delayMs
        XCTAssertNotNil(wait)
    }

    func testAViewTooWideIsRefusedWithTheCoresWords() {
        let wide = NearbyArea(west: -75.5, south: 40.0, east: -73.0, north: 41.5)
        XCTAssertThrowsError(
            try planRecords(
                area: wide,
                query: RecordsLookup.query(scientificName: "Morone saxatilis", page: 1, region: ""),
                stateJson: "", settingsJson: "{}", now: now)
        ) { error in
            guard case NearbyError.ZoomIn(let message) = error else {
                return XCTFail("\(error)")
            }
            XCTAssertFalse(message.isEmpty)
        }
    }

    func testANamedStateAsksTheNonindigenousDatabase() throws {
        let plan = try planRecords(
            area: harbor,
            query: RecordsLookup.query(
                scientificName: "Morone saxatilis", page: 1, region: "US-NY"),
            stateJson: "", settingsJson: "{}", now: now)
        XCTAssertTrue(plan.requests().contains { $0.source == RecordMarks.invasiveSource })
    }

    func testBeachForecastsOutsideTheGulfAskNothingAndSaySo() async throws {
        let plan = try planBlooms(area: harbor, settingsJson: "{}", now: now)
        XCTAssertTrue(plan.requests().isEmpty)
        let log = Log()
        let results = await RecordsLookup.run(
            plan, send: fakeServices(log), sleep: { _ in }, clock: { [now] in now })
        let urls = await log.urls
        XCTAssertTrue(urls.isEmpty)
        XCTAssertTrue(results.blooms.isEmpty)
        XCTAssertFalse(results.notices.isEmpty)
    }

    func testAFailedRequestIsAnsweredByNothing() async throws {
        let plan = try planBlooms(
            area: NearbyArea(west: -82.9, south: 27.2, east: -82.4, north: 27.6),
            settingsJson: "{}", now: now)
        XCTAssertFalse(plan.requests().isEmpty)
        let log = Log()
        let results = await RecordsLookup.run(
            plan, send: fakeServices(log), sleep: { _ in }, clock: { [now] in now })
        let urls = await log.urls
        XCTAssertEqual(urls.count, 1, "no retry after a failure")
        XCTAssertTrue(results.blooms.isEmpty)
    }

    // MARK: - Rounds

    func testQueuesRunSideBySideEachInOrderAfterItsWait() async {
        let log = Log()
        let waits = Log()
        func request(_ id: String, queue: String, delay: UInt32) -> NearbyRequest {
            NearbyRequest(
                id: id, source: queue, queue: queue, delayMs: delay,
                request: HostRequest(
                    url: "https://example.invalid/\(id)", headers: [], method: .get, body: nil,
                    fresh: false, noStore: false, maxAgeS: nil))
        }
        let requests = [
            request("1", queue: "a", delay: 0), request("2", queue: "b", delay: 0),
            request("3", queue: "a", delay: 1_000), request("4", queue: "a", delay: 0),
        ]
        let ok = response(200, Data())
        let answers = await RecordsLookup.round(
            requests,
            send: { request in
                await log.add(request.url)
                if request.url.hasSuffix("/4") { throw HostError.TimedOut(message: "slow") }
                return ok
            },
            sleep: { duration in await waits.add("\(duration)") })
        XCTAssertEqual(answers.map(\.id), ["1", "2", "3"], "in the requests' order, no failed one")
        let urls = await log.urls
        let a = urls.filter { ["1", "3", "4"].contains(String($0.last ?? " ")) }
        XCTAssertEqual(a.map { String($0.last ?? " ") }, ["1", "3", "4"])
        let slept = await waits.urls
        XCTAssertEqual(slept, ["\(Duration.milliseconds(1_000))"])
    }
}

/// What a fake service was asked, in order.
private actor Log {
    private(set) var urls: [String] = []

    func add(_ url: String) {
        urls.append(url)
    }
}
