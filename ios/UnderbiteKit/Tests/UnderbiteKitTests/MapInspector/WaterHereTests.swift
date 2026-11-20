import CoreLocation
import Foundation
import XCTest

@testable import UnderbiteKit

/// The point card's "water here": the lookup through the app's host with the real core and
/// made-up 3DHP answers, what the section shows in each state, and what "Add as spot" starts
/// from. Every word is the core's.
final class WaterHereTests: XCTestCase {
    /// 2026-11-19 15:00 UTC.
    private let now: Int64 = 1_795_100_400

    /// A point on the stream below, with the digits a map gives.
    private let tap = CLLocationCoordinate2D(latitude: 41.205_123_456, longitude: -73.900_123_456)

    private func flowlines(named name: String?) -> Data {
        let label = name.map { "\"\($0)\"" } ?? "null"
        return Data(
            """
            {"features":[{"attributes":{"id3dhp":"T1","gnisidlabel":\(label),\
            "featuretypelabel":"Channel Line","streamorder":3,"levelpath":1,"hydrosequence":1,\
            "dnhydrosequence":0,"uphydrosequence":0,"lengthkm":1.1},\
            "geometry":{"paths":[[[-73.9001,41.2],[-73.9001,41.21]]]}}]}
            """.utf8)
    }

    private let noWaterbodies = Data(#"{"features":[]}"#.utf8)

    private func response(_ body: Data, status: UInt16 = 200) -> HostResponse {
        HostResponse(
            status: status, body: body, contentType: "application/json", date: nil, expires: nil,
            lastModified: nil, etag: nil, retryAfter: nil, rateLimitRemaining: nil)
    }

    private func plan(region: String? = "US-NY", tide: [MapPoint] = []) throws -> WaterHerePlan {
        try planWaterHere(
            point: MapPoint(lat: tap.latitude, lon: tap.longitude), region: region,
            tideStations: tide, settingsJson: "{}", now: now)
    }

    /// Answers the 3DHP layers with `lines` and no waterbodies, and counts what was sent.
    private func fakeService(lines: Data, _ log: Log) -> RecordsLookup.Send {
        let bodies = noWaterbodies
        let flowlineAnswer = response(lines)
        let waterbodyAnswer = response(bodies)
        return { request in
            await log.add(request.url)
            if request.url.contains("/MapServer/50/") { return flowlineAnswer }
            if request.url.contains("/MapServer/60/") { return waterbodyAnswer }
            throw HostError.Failed(message: "unexpected request")
        }
    }

    // MARK: - The lookup

    func testTheLookupNamesTheWaterFromTheHostsAnswers() async throws {
        let log = Log()
        let result = await WaterHereLookup.run(
            try plan(), send: fakeService(lines: flowlines(named: "Test Creek"), log),
            sleep: { _ in }, clock: { [now] in now })
        let state = WaterHereState(result)
        let card = try XCTUnwrap(state.card)
        XCTAssertEqual(card.title, "Test Creek")
        XCTAssertEqual(card.kindText, "Stream")
        XCTAssertEqual(card.waterType, "river_stream")
        XCTAssertEqual(card.spotName, "Test Creek")
        XCTAssertTrue(state.showsSection)
        XCTAssertTrue(card.credits.contains { $0.name == "USGS 3DHP" })
        // The links are the region's, with the date each was checked.
        XCTAssertFalse(card.links.isEmpty)
        XCTAssertTrue(card.links.allSatisfy { $0.url.hasPrefix("https://") && !$0.detail.isEmpty })
        XCTAssertNotNil(card.disclaimer)
    }

    func testOnlyTheTwoRoundedQueriesAreSentAndNeverTheExactPoint() async throws {
        let log = Log()
        _ = await WaterHereLookup.run(
            try plan(), send: fakeService(lines: flowlines(named: nil), log), sleep: { _ in },
            clock: { [now] in now })
        let urls = await log.urls
        XCTAssertEqual(urls.count, 2)
        for url in urls {
            XCTAssertTrue(url.contains("hydro.nationalmap.gov"), url)
            XCTAssertFalse(url.contains("41.2051") || url.contains("73.9001"), url)
            // Every coordinate sent has two decimals at most.
            let geometry = try XCTUnwrap(
                url.split(whereSeparator: { "?&".contains($0) }).first {
                    $0.hasPrefix("geometry=")
                })
            for number in geometry.dropFirst("geometry=".count).split(separator: ",") {
                let decimals = number.split(separator: ".").dropFirst().first?.count ?? 0
                XCTAssertLessThanOrEqual(decimals, 2, url)
            }
        }
    }

    func testUnnamedWaterIsUnnamedInTheCoresWordsAndOffersNoName() async throws {
        let result = await WaterHereLookup.run(
            try plan(region: nil), send: fakeService(lines: flowlines(named: nil), Log()),
            sleep: { _ in }, clock: { [now] in now })
        let card = try XCTUnwrap(WaterHereState(result).card)
        XCTAssertEqual(card.title, "Unnamed stream")
        XCTAssertFalse(card.named)
        XCTAssertNil(card.spotName)
        // No saved region: the links are those of the state the point is in, from the core's
        // bundled boundaries, and one state needs no note.
        XCTAssertFalse(card.links.isEmpty)
        XCTAssertTrue(card.links.allSatisfy { $0.detail.hasPrefix("New York State") })
        XCTAssertNil(card.linksNote)
        let prefill = SpotPrefill(card: card, at: tap)
        XCTAssertEqual(prefill.name, "")
        XCTAssertEqual(prefill.waterType, "river_stream")
    }

    func testASavedRegionAndTheStateOfThePointAreBothLinkedWithTheCoresNote() async throws {
        let result = await WaterHereLookup.run(
            try plan(region: "US-NJ"), send: fakeService(lines: flowlines(named: "Test Creek"), Log()),
            sleep: { _ in }, clock: { [now] in now })
        let card = try XCTUnwrap(WaterHereState(result).card)
        XCTAssertEqual(
            card.linksNote, "On the New Jersey\u{2013}New York line: both states' rules are linked")
        XCTAssertTrue(card.links.contains { $0.detail.hasPrefix("New Jersey") })
        XCTAssertTrue(card.links.contains { $0.detail.hasPrefix("New York State") })
    }

    func testNoWaterWithinReachLeavesNoSection() async throws {
        let empty = Data(#"{"features":[]}"#.utf8)
        let result = await WaterHereLookup.run(
            try plan(), send: fakeService(lines: empty, Log()), sleep: { _ in },
            clock: { [now] in now })
        XCTAssertEqual(result.status, .nothing)
        XCTAssertEqual(WaterHereState(result), .hidden)
        XCTAssertFalse(WaterHereState(result).showsSection)
    }

    func testAFailedLookupShowsTheCoresFailureWords() async throws {
        let failing: RecordsLookup.Send = { _ in throw HostError.Failed(message: "offline") }
        let result = await WaterHereLookup.run(
            try plan(), send: failing, sleep: { _ in }, clock: { [now] in now })
        let words = try waterHereWords(settingsJson: "{}")
        XCTAssertEqual(WaterHereState(result), .failed(words.failed))
        XCTAssertEqual(words.failed, "USGS 3DHP could not be reached")
        XCTAssertTrue(WaterHereState(result).showsSection)
    }

    func testTheLoadingWordsAreTheCoresAndShownBeforeTheAnswer() throws {
        let before = WaterHereState(try plan().result())
        XCTAssertEqual(before, .looking)
        XCTAssertTrue(before.showsSection)
        XCTAssertEqual(try waterHereWords(settingsJson: "{}").loading, "Looking up the water here…")
    }

    func testACancelledLookupSendsNothing() async throws {
        let log = Log()
        let plan = try plan()
        let send = fakeService(lines: flowlines(named: "Test Creek"), log)
        let clock: @Sendable () -> Int64 = { [now] in now }
        let task = Task { () -> WaterHereResult in
            // Cancelled before it gets here: the sleep ends at once.
            try? await Task.sleep(for: .seconds(30))
            return await WaterHereLookup.run(plan, send: send, sleep: { _ in }, clock: clock)
        }
        task.cancel()
        let result = await task.value
        let urls = await log.urls
        XCTAssertTrue(urls.isEmpty)
        XCTAssertEqual(result.status, .looking)
    }

    // MARK: - What the card hands on

    func testAddAsSpotStartsFromThePlaceTheNameAndTheInferredWaterType() async throws {
        let result = await WaterHereLookup.run(
            try plan(), send: fakeService(lines: flowlines(named: "Test Creek"), Log()),
            sleep: { _ in }, clock: { [now] in now })
        let card = try XCTUnwrap(WaterHereState(result).card)
        let prefill = SpotPrefill(card: card, at: tap)
        // The exact place stays on the device; it is what the spot is saved at.
        XCTAssertEqual(prefill.latitude, tap.latitude)
        XCTAssertEqual(prefill.longitude, tap.longitude)
        XCTAssertEqual(prefill.name, "Test Creek")
        XCTAssertEqual(prefill.waterType, "river_stream")
    }

    func testOnlyTideStationsAreHandedToTheCore() {
        func pin(_ kind: PinKind, _ lat: Double) -> MapPin {
            MapPin(
                id: "\(kind)", kind: kind, layer: "x", name: "n", label: "", role: .normal,
                latitude: lat, longitude: -74)
        }
        let points = WaterHereLookup.tideStations([
            pin(.tide, 40.7), pin(.buoy, 40.4), pin(.riverGauge, 41.2), pin(.spot, 40.6),
        ])
        XCTAssertEqual(points, [MapPoint(lat: 40.7, lon: -74)])
    }

    func testTheRegionIsTheSavedOneOrNone() {
        XCTAssertEqual(WaterHereLookup.region(" us-ny "), "us-ny")
        XCTAssertNil(WaterHereLookup.region(""))
        XCTAssertNil(WaterHereLookup.region("  "))
        XCTAssertNil(WaterHereLookup.region(nil))
    }
}

/// The point card's sheet: it starts at the first glance, rises to the middle height when the
/// "water here" section appears, and stays where the person put it.
final class PointSheetStopsTests: XCTestCase {
    func testItStartsAtTheGlanceAndRisesWhenTheSectionAppears() {
        var stops = PointSheetStops()
        XCTAssertEqual(stops.stop, .glance)
        stops.water(shown: false)
        XCTAssertEqual(stops.stop, .glance)
        stops.water(shown: true)
        XCTAssertEqual(stops.stop, .water)
        XCTAssertFalse(stops.moved)
        // The sheet reports the height it was sent to: not a move by the person.
        stops.rested(at: .water)
        XCTAssertFalse(stops.moved)
    }

    func testItStaysWhereThePersonPutIt() {
        var dragged = PointSheetStops()
        dragged.rested(at: .full)
        dragged.water(shown: true)
        XCTAssertEqual(dragged.stop, .full)
        // Dragged back down to the glance, it stays there too.
        dragged.rested(at: .glance)
        dragged.water(shown: true)
        XCTAssertEqual(dragged.stop, .glance)
        // Moved by the person after rising, it is theirs.
        var risen = PointSheetStops()
        risen.water(shown: true)
        risen.rested(at: .glance)
        XCTAssertTrue(risen.moved)
        risen.water(shown: true)
        XCTAssertEqual(risen.stop, .glance)
    }

    func testANewPointStartsAgain() {
        var stops = PointSheetStops()
        stops.rested(at: .full)
        stops.reset()
        XCTAssertEqual(stops, PointSheetStops())
        stops.water(shown: true)
        XCTAssertEqual(stops.stop, .water)
    }
}

private actor Log {
    private(set) var urls: [String] = []

    func add(_ url: String) {
        urls.append(url)
    }
}
