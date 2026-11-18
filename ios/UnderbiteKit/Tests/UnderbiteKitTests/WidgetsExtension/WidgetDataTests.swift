import Foundation
import XCTest

@testable import UnderbiteKit

/// How the widgets find their payload, build their timelines from its entries and place the
/// tide and the flow on their drawings.
final class WidgetDataTests: XCTestCase {
    private var folder: URL!
    private var store: WidgetPayloadStore!
    private var battery: ViewModel.WidgetPayload!
    private var croton: ViewModel.WidgetPayload!
    private var trip: ViewModel.WidgetPayload!

    override func setUpWithError() throws {
        folder = FileManager.default.temporaryDirectory
            .appending(path: "underbite-widget-data-\(UUID().uuidString)", directoryHint: .isDirectory)
        store = WidgetPayloadStore(container: SharedContainer(url: folder, backing: .temporary))
        battery = try ViewModel.decodeWidget(Fixtures.text("sample-widget-battery"))
        croton = try ViewModel.decodeWidget(Fixtures.text("sample-widget-croton"))
        trip = try ViewModel.decodeWidget(Fixtures.text("sample-widget-trip"))
    }

    override func tearDownWithError() throws {
        try? FileManager.default.removeItem(at: folder)
    }

    private func date(_ t: Int64) -> Date { Date(timeIntervalSince1970: TimeInterval(t)) }

    // MARK: Finding the payload

    func testAWidgetReadsItsSpotElseTheAppsAndTriesAnIDTheListDoesNotKnowYet() {
        let spots = WidgetSpots(
            spots: [WidgetSpot(id: "battery", name: "The Battery"), WidgetSpot(id: "croton", name: "Croton River")],
            appSpotID: "croton")
        XCTAssertEqual(WidgetData.spotID(for: .spot("battery"), in: spots), "battery")
        XCTAssertEqual(WidgetData.spotID(for: .app, in: spots), "croton")
        XCTAssertEqual(WidgetData.spotID(for: .spot("gone"), in: spots), "croton")
        // Before the app has written the list.
        XCTAssertEqual(WidgetData.spotID(for: .spot("battery"), in: WidgetSpots()), "battery")
        XCTAssertNil(WidgetData.spotID(for: .app, in: WidgetSpots()))
    }

    func testOnlyTheChosenSpotsPayloadIsRead() throws {
        // Nothing yet: no spot to show.
        var loaded = WidgetData.load(.tide, choice: .app, store: store)
        XCTAssertNil(loaded.payload)
        XCTAssertEqual(loaded.gap, .noSpot)

        try store.saveSpots(
            WidgetSpots(spots: [WidgetSpot(id: "battery", name: "The Battery")], appSpotID: "battery"))
        loaded = WidgetData.load(.tide, choice: .app, store: store)
        XCTAssertNil(loaded.payload)
        XCTAssertEqual(loaded.name, "The Battery")
        XCTAssertEqual(loaded.gap, .noPayload)

        try store.saveSpot(Fixtures.text("sample-widget-battery"))
        try store.saveSpot(Fixtures.text("sample-widget-croton"))
        XCTAssertEqual(WidgetData.load(.tide, choice: .app, store: store).payload?.id, "battery")
        // A spot the list no longer has (deleted) shows the app's.
        XCTAssertEqual(WidgetData.load(.flow, choice: .spot("croton"), store: store).payload?.id, "battery")
        try store.saveSpots(
            WidgetSpots(
                spots: [WidgetSpot(id: "battery", name: "The Battery"), WidgetSpot(id: "croton", name: "Croton River")],
                appSpotID: "battery"))
        XCTAssertEqual(WidgetData.load(.flow, choice: .spot("croton"), store: store).payload?.id, "croton")

        // The next trip's widget reads the trip's file whatever it was set to.
        XCTAssertNil(WidgetData.load(.nextTrip, choice: .app, store: store).payload)
        XCTAssertEqual(WidgetData.load(.nextTrip, choice: .app, store: store).gap, .noTrip)
        try store.saveNextTrip(Fixtures.text("sample-widget-trip"))
        XCTAssertEqual(WidgetData.load(.nextTrip, choice: .spot("croton"), store: store).payload?.id, "harbor")
    }

    // MARK: Timelines

    func testATimelineIsThePayloadsEntriesThenAGapOnceTheLastHourIsOver() throws {
        let now = date(battery.generatedAt)
        let schedule = WidgetData.schedule(.tide, payload: battery, name: nil, missing: .noPayload, now: now)
        XCTAssertEqual(schedule.moments.count, battery.entries.count + 1)
        XCTAssertEqual(schedule.moments.map(\.date).dropLast(), battery.entries.map(\.date)[...])
        XCTAssertEqual(schedule.moments.first?.name, "The Battery")
        for (moment, entry) in zip(schedule.moments, battery.entries) {
            XCTAssertEqual(moment.content, .entry(entry, places: battery.places))
        }
        let last = try XCTUnwrap(battery.lastEntryDate)
        let over = last.addingTimeInterval(3600)
        XCTAssertEqual(schedule.moments.last, WidgetMoment(date: over, name: "The Battery", content: .gap(.outdated)))
        XCTAssertEqual(schedule.reload, over)
    }

    func testALaterTimelineStartsAtTheEntryCurrentThen() {
        let now = date(battery.generatedAt + 5400)
        let schedule = WidgetData.schedule(.snack, payload: battery, name: nil, missing: .noPayload, now: now)
        XCTAssertEqual(schedule.moments.first?.date, date(battery.generatedAt + 3600))
        XCTAssertEqual(schedule.moments.count, battery.entries.count)
        XCTAssertEqual(
            WidgetData.moment(.snack, payload: battery, name: nil, missing: .noPayload, now: now).date,
            date(battery.generatedAt + 3600))
    }

    func testAPayloadPastItsLastEntryIsNeverShownAsCurrent() throws {
        let last = try XCTUnwrap(battery.lastEntryDate)
        let now = last.addingTimeInterval(7200)
        let schedule = WidgetData.schedule(.tide, payload: battery, name: nil, missing: .noPayload, now: now)
        XCTAssertEqual(schedule.moments, [WidgetMoment(date: now, name: "The Battery", content: .gap(.outdated))])
        XCTAssertEqual(schedule.reload, now.addingTimeInterval(WidgetData.retry))
    }

    func testAKindThePlaceLacksIsAGapAtEveryEntry() {
        let now = date(croton.generatedAt)
        let tide = WidgetData.schedule(.tide, payload: croton, name: nil, missing: .noPayload, now: now)
        XCTAssertTrue(tide.moments.dropLast().allSatisfy { $0.content == .gap(.noData) })
        let flow = WidgetData.schedule(.flow, payload: croton, name: nil, missing: .noPayload, now: now)
        XCTAssertTrue(flow.moments.dropLast().allSatisfy { if case .entry = $0.content { true } else { false } })
        let noTrip = WidgetData.schedule(.nextTrip, payload: battery, name: nil, missing: .noTrip, now: now)
        XCTAssertTrue(noTrip.moments.dropLast().allSatisfy { $0.content == .gap(.noData) })
    }

    func testWithoutAPayloadTheWidgetWaitsWithTheSpotsName() {
        let now = date(1_794_139_200)
        let schedule = WidgetData.schedule(.flow, payload: nil, name: "Croton River", missing: .noPayload, now: now)
        XCTAssertEqual(
            schedule.moments, [WidgetMoment(date: now, name: "Croton River", content: .gap(.noPayload))])
        XCTAssertEqual(schedule.reload, now.addingTimeInterval(WidgetData.retry))
    }

    // MARK: Drawings

    func testTheTideWindowIsTheEntrysPartOfTheChart() throws {
        let chart = try XCTUnwrap(battery.places[0].tideChart)
        let entry = battery.entries[0]
        let tide = try XCTUnwrap(entry.places[0].tide)
        let window = TideWindow(chart: chart, start: tide.startT, end: tide.endT, now: entry.t)

        // The curve runs from the left edge to the right one, inside the height.
        XCTAssertLessThanOrEqual(try XCTUnwrap(window.curve.first?.x), 0)
        XCTAssertGreaterThanOrEqual(try XCTUnwrap(window.curve.last?.x), 1)
        let inside = window.curve.filter { (0...1).contains($0.x) }
        XCTAssertTrue(inside.allSatisfy { (0...1).contains($0.y) })
        XCTAssertEqual(inside.count, Int((tide.endT - tide.startT) / 1800) + 1)

        // Now three hours into the day's window.
        XCTAssertEqual(try XCTUnwrap(window.now), 3.0 / 24.0, accuracy: 1e-9)
        // The highs and lows in the window, with the chart's words.
        XCTAssertEqual(window.marks.map(\.label), ["High 5.6 ft", "Low 0.0 ft", "High 4.7 ft", "Low 0.2 ft"])
        XCTAssertEqual(window.marks.map(\.kind), ["high", "low", "high", "low"])
        let high = window.marks[0].at.y, low = window.marks[1].at.y
        XCTAssertGreaterThan(high, low)
        // The nights it crosses, cut at its edges; only major hours are labelled.
        XCTAssertEqual(window.nights.count, 2)
        XCTAssertEqual(try XCTUnwrap(window.nights.first?.start), 0)
        XCTAssertTrue(window.nights.allSatisfy { 0 <= $0.start && $0.start < $0.end && $0.end <= 1 })
        let majors = chart.xTicks.filter { $0.major && $0.t >= tide.startT && $0.t <= tide.endT }
        XCTAssertEqual(window.ticks.map(\.label), majors.map(\.label))
        XCTAssertFalse(window.ticks.isEmpty)

        // Now outside the window is not drawn.
        XCTAssertNil(TideWindow(chart: chart, start: tide.startT, end: tide.endT, now: tide.endT + 1).now)
    }

    func testTheFlowLineSpansTheWeekAndItsRange() throws {
        let spark = croton.places[0].flowSpark
        let points = FlowLine.points(spark)
        XCTAssertEqual(points.count, spark.count)
        XCTAssertEqual(points.first?.x, 0)
        XCTAssertEqual(try XCTUnwrap(points.last?.x), 1, accuracy: 1e-9)
        XCTAssertEqual(points.map(\.y).min(), 0)
        XCTAssertEqual(points.map(\.y).max(), 1)
        XCTAssertEqual(FlowLine.points([3, 3, 3]).map(\.y), [0.5, 0.5, 0.5])
        XCTAssertEqual(FlowLine.points([3]), [])
        XCTAssertEqual(FlowLine.points([]), [])
    }

    // MARK: Credits

    func testATripCreditsEverySourceItDrawsAndEachNoticeOnce() throws {
        let entry = trip.entries[0]
        let tripPart = try XCTUnwrap(entry.trip)
        let compact = WidgetCredits.trip(tripPart, places: entry.places, parts: false)
        XCTAssertEqual(compact.credits, ["Computed on device", "NOAA CO-OPS"])
        XCTAssertEqual(compact.line, "Computed on device · NOAA CO-OPS")
        XCTAssertEqual(compact.notices, [])

        let full = WidgetCredits.trip(tripPart, places: entry.places, parts: true)
        // Each source once, however many parts rest on it.
        XCTAssertEqual(full.credits, ["Computed on device", "NOAA CO-OPS", "USGS"])
        XCTAssertEqual(
            full.notices,
            ["Not for navigation.", "A traditional cue, not a promise.", "Provisional data, subject to revision."])
    }
}
