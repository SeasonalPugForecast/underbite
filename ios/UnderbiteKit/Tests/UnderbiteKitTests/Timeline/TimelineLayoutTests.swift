import Foundation
import XCTest

@testable import UnderbiteKit

/// Where the day timeline places things, against the recorded sample page.
final class TimelineLayoutTests: XCTestCase {
    private func timeline() throws -> ViewModel.Timeline {
        try XCTUnwrap(ViewModel.decode(Fixtures.text("sample-view")).view?.timeline)
    }

    private func lane(_ kind: String, in timeline: ViewModel.Timeline) throws
        -> ViewModel.TimelineLane
    {
        try XCTUnwrap(timeline.lanes.first { $0.kind == kind })
    }

    func testTheAxisSpansTheViewsStartAndEndAtAFixedHourWidth() throws {
        let timeline = try timeline()
        XCTAssertEqual(TimelineLayout.x(timeline.startT, start: timeline.startT), 0)
        let hours = Double(timeline.endT - timeline.startT) / 3_600
        XCTAssertEqual(
            TimelineLayout.width(timeline), hours * TimelineLayout.hourWidth, accuracy: 0.001)
        // A wider screen never changes the hour width.
        XCTAssertEqual(TimelineLayout.hourStarts(timeline).count, Int(hours.rounded(.up)))
    }

    func testRowsAlignOnTimeNotOnIndex() throws {
        let timeline = try timeline()
        // A 6-minute point and an hourly point at the same instant land on the same x.
        let t = timeline.startT + 5 * 3_600
        XCTAssertEqual(
            TimelineLayout.x(t, start: timeline.startT),
            5 * TimelineLayout.hourWidth, accuracy: 0.001)
        XCTAssertEqual(
            TimelineLayout.x(t + 360, start: timeline.startT),
            5.1 * TimelineLayout.hourWidth, accuracy: 0.001)
    }

    func testADaylightSavingDayIsLaidOutByInstants() {
        // A 23-hour day is 23 hour widths, not 24.
        let timeline = ViewModel.Timeline(
            startT: 1_000_000, endT: 1_000_000 + 23 * 3_600, stepMinutes: 60, xTicks: [], lanes: [])
        XCTAssertEqual(TimelineLayout.width(timeline), 23 * TimelineLayout.hourWidth, accuracy: 0.001)
        XCTAssertEqual(TimelineLayout.hourStarts(timeline).count, 23)
    }

    func testLanesWithoutDataAreNotDrawn() throws {
        let timeline = try timeline()
        let kinds = TimelineLayout.visibleLanes(
            timeline, shownCards: ["tide", "current", "wind", "pressure"]
        ).map(\.kind)
        XCTAssertEqual(kinds, ["tide", "current", "light", "solunar"])
    }

    func testAHiddenCardHidesItsRow() throws {
        let timeline = try timeline()
        let kinds = TimelineLayout.visibleLanes(timeline, shownCards: ["current"]).map(\.kind)
        XCTAssertFalse(kinds.contains("tide"))
        XCTAssertTrue(kinds.contains("current"))
        // Light and solunar follow their own settings, not a card.
        XCTAssertTrue(kinds.contains("light"))
    }

    func testTheNowMarkerIsTheViewsNow() throws {
        let timeline = try timeline()
        XCTAssertEqual(TimelineLayout.now(timeline), 1_794_000_360)
    }

    func testTappingAnHourSnapsToItsStart() throws {
        let timeline = try timeline()
        let x = TimelineLayout.hourWidth * 5.6
        XCTAssertEqual(
            TimelineLayout.hour(atX: x, timeline: timeline), timeline.startT + 5 * 3_600)
        XCTAssertNil(TimelineLayout.hour(atX: -1, timeline: timeline))
        XCTAssertNil(
            TimelineLayout.hour(atX: TimelineLayout.width(timeline) + 1, timeline: timeline))
    }

    func testAnHoursReadingsAreTheViewsLabels() throws {
        let timeline = try timeline()
        let tide = try lane("tide", in: timeline)
        let high = try XCTUnwrap(tide.chart.markers.first { $0.kind == "high" })
        let hour = high.t - (high.t - timeline.startT) % 3_600
        let lines = TimelineLayout.readings(of: tide, hour: hour, stepMinutes: timeline.stepMinutes)
        XCTAssertTrue(lines.contains(high.label), "\(lines)")
        XCTAssertTrue(lines.contains { $0.contains("ft") })

        let solunar = try lane("solunar", in: timeline)
        let band = try XCTUnwrap(solunar.chart.bands.first)
        let inside = band.startT - (band.startT - timeline.startT) % 3_600
        XCTAssertEqual(
            TimelineLayout.readings(of: solunar, hour: inside, stepMinutes: timeline.stepMinutes),
            [band.label])
        XCTAssertEqual(
            TimelineLayout.readings(
                of: solunar, hour: timeline.startT, stepMinutes: timeline.stepMinutes), [])
    }

    func testEachLaneCarriesItsOwnCreditsAndNotices() throws {
        let timeline = try timeline()
        for kind in ["tide", "current"] {
            let lane = try lane(kind, in: timeline)
            XCTAssertTrue(lane.notices.contains { $0.text == "Not for navigation." }, kind)
            XCTAssertFalse(lane.credits.isEmpty, kind)
        }
    }

    func testTheRangeScalesSeriesAndMarkerValues() throws {
        let tide = try lane("tide", in: try timeline())
        let range = try XCTUnwrap(TimelineLayout.range(of: tide))
        XCTAssertLessThan(range.lowerBound, range.upperBound)
        XCTAssertNil(TimelineLayout.range(of: try lane("light", in: try timeline())))
    }
}
