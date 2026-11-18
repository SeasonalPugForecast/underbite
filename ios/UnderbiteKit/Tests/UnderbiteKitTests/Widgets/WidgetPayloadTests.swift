import Foundation
import XCTest

@testable import UnderbiteKit

/// The core's widget and Live Activity payloads (`sample-widget-*.json`,
/// `sample-trip-activity.json`, written by the core's own tests) decode as the widgets read them.
final class WidgetPayloadTests: XCTestCase {
    /// When the samples were built: 2026-11-08 12:00 UTC.
    private let generated: Int64 = 1_794_139_200

    private func payload(_ name: String) throws -> ViewModel.WidgetPayload {
        try ViewModel.decodeWidget(Fixtures.text(name))
    }

    func testTheBatteryHasItsTideEveryHourWithItsCredit() throws {
        let p = try payload("sample-widget-battery")
        XCTAssertTrue(p.isSpot)
        XCTAssertEqual(p.id, "battery")
        XCTAssertEqual(p.name, "The Battery")
        XCTAssertEqual(p.generatedAt, generated)
        XCTAssertEqual(p.places.map(\.id), ["battery"])
        let chart = try XCTUnwrap(p.places[0].tideChart)
        XCTAssertFalse(chart.series.isEmpty)
        XCTAssertEqual(chart.nowT, generated)
        XCTAssertEqual(p.places[0].flowSpark, [])

        XCTAssertEqual(p.entries.count, 13)
        XCTAssertEqual(p.entries.first?.t, generated)
        XCTAssertEqual(p.entries.dropFirst().map { $0.t - generated }, (1...12).map { Int64($0) * 3_600 })
        for entry in p.entries {
            let tide = try XCTUnwrap(entry.places.first?.tide)
            XCTAssertTrue(tide.card.credit.contains("NOAA CO-OPS"), tide.card.credit)
            XCTAssertEqual(tide.card.notices.map(\.kind), ["not_for_navigation"])
            XCTAssertLessThan(tide.startT, entry.t)
            XCTAssertGreaterThan(tide.endT, entry.t)
            XCTAssertNil(entry.places.first?.flow)
            XCTAssertNil(entry.trip)
        }
        let first = try XCTUnwrap(p.entries.first?.places.first?.tide)
        XCTAssertEqual(first.stage, "high_slack")
        XCTAssertEqual(first.card.headline.unit, "ft MLLW")
        XCTAssertEqual(first.card.state, .fresh)
    }

    func testTheCrotonHasItsFlowWithTrendAndNormal() throws {
        let p = try payload("sample-widget-croton")
        XCTAssertEqual(p.id, "croton")
        XCTAssertNil(p.places[0].tideChart)
        XCTAssertFalse(p.places[0].flowSpark.isEmpty)
        let entry = try XCTUnwrap(p.entries.first?.places.first)
        XCTAssertNil(entry.tide)
        let flow = try XCTUnwrap(entry.flow)
        XCTAssertEqual(flow.card.headline.unit, "cfs")
        XCTAssertEqual(flow.card.credit, "USGS")
        XCTAssertEqual(flow.trend?.direction, "falling")
        XCTAssertEqual(flow.normal, "Normal")
        XCTAssertEqual(flow.card.notices.map(\.kind), ["provisional"])
        XCTAssertNotNil(entry.snack)
    }

    func testATripHasItsGoNoGoAndTwoPlaces() throws {
        let p = try payload("sample-widget-trip")
        XCTAssertTrue(p.isTrip)
        XCTAssertEqual(p.id, "harbor")
        XCTAssertEqual(p.name, "Harbor weekend")
        XCTAssertEqual(p.places.map(\.id), ["battery", "croton"])
        for entry in p.entries {
            XCTAssertEqual(entry.places.count, 2)
            let trip = try XCTUnwrap(entry.trip)
            XCTAssertEqual(trip.compact.title, "Next trip: Harbor weekend")
            XCTAssertFalse(trip.credit.isEmpty)
        }
        XCTAssertNotNil(p.entries[0].places[0].tide)
        XCTAssertNotNil(p.entries[0].places[1].flow)
    }

    func testTheEntryAtATimeIsTheLastBegun() throws {
        let p = try payload("sample-widget-battery")
        func at(_ offset: Int64) -> Date { Date(timeIntervalSince1970: TimeInterval(generated + offset)) }
        // Before the first, the first; then the last begun; past the last, the last.
        XCTAssertEqual(p.entry(at: at(-600))?.t, generated)
        XCTAssertEqual(p.entry(at: at(0))?.t, generated)
        XCTAssertEqual(p.entry(at: at(3_599))?.t, generated)
        XCTAssertEqual(p.entry(at: at(3_600))?.t, generated + 3_600)
        XCTAssertEqual(p.entry(at: at(30 * 3_600))?.t, generated + 12 * 3_600)
        XCTAssertEqual(p.entries(from: at(5_400)).count, 12)
        XCTAssertEqual(p.entries(from: at(5_400)).first?.t, generated + 3_600)
        XCTAssertEqual(p.lastEntryDate, at(12 * 3_600))
        XCTAssertEqual(p.entries[1].date, at(3_600))
    }

    func testUnknownFieldsAreIgnored() throws {
        for name in ["sample-widget-battery", "sample-widget-croton", "sample-widget-trip"] {
            let original = try Fixtures.text(name)
            let later = try Fixtures.string(Fixtures.addingUnknownFields(Fixtures.object(original)))
            XCTAssertEqual(try ViewModel.decodeWidget(later), try ViewModel.decodeWidget(original), name)
        }
        let activity = try Fixtures.text("sample-trip-activity")
        let later = try Fixtures.string(Fixtures.addingUnknownFields(Fixtures.object(activity)))
        XCTAssertEqual(
            try ViewModel.decodeTripActivity(later), try ViewModel.decodeTripActivity(activity))
    }

    func testTheTripDayActivityHasItsTargetsAndWords() throws {
        let a = try ViewModel.decodeTripActivity(Fixtures.text("sample-trip-activity"))
        XCTAssertEqual(a.title, "Harbor weekend")
        XCTAssertEqual(a.place, "The Battery")
        XCTAssertEqual(a.generatedAt, generated)
        let tide = try XCTUnwrap(a.nextTideChange)
        XCTAssertEqual(tide.label, "High tide in")
        XCTAssertGreaterThan(tide.at, generated)
        XCTAssertEqual(tide.date, Date(timeIntervalSince1970: TimeInterval(tide.at)))
        XCTAssertEqual(a.slack?.label, "Slack in")
        XCTAssertEqual(a.sunset?.label, "Sunset in")
        XCTAssertEqual(a.targets.map(\.label), ["High tide in", "Slack in", "Sunset in"])
        XCTAssertTrue(a.credit.contains("NOAA CO-OPS"), a.credit)
        XCTAssertEqual(a.notices, [ViewModel.ActivityNotice(kind: "not_for_navigation", text: "Not for navigation.")])
        XCTAssertFalse(a.stale.isEmpty)

        // Stale from the first target on; over at sunset.
        XCTAssertEqual(a.staleAt, tide.at)
        XCTAssertFalse(a.isStale(at: Date(timeIntervalSince1970: TimeInterval(tide.at - 1))))
        XCTAssertTrue(a.isStale(at: a.staleDate))
        XCTAssertEqual(a.endsAt, a.sunset?.at)
        XCTAssertFalse(a.isOver(at: a.staleDate))
        XCTAssertTrue(a.isOver(at: a.endDate))
    }

    /// ActivityKit carries the content state as it encodes it: it comes back the same, and well
    /// under the 4 KB a Live Activity may hold.
    func testTheActivityContentRoundTripsAndIsSmall() throws {
        let a = try ViewModel.decodeTripActivity(Fixtures.text("sample-trip-activity"))
        let data = try JSONEncoder().encode(a)
        XCTAssertEqual(try JSONDecoder().decode(ViewModel.TripDayActivity.self, from: data), a)
        XCTAssertLessThan(data.count, 2_048)
    }

    func testAPayloadThatIsNotOneFails() {
        XCTAssertThrowsError(try ViewModel.decodeWidget("{"))
        XCTAssertThrowsError(try ViewModel.decodeWidget(SampleSnapshot.json))
        XCTAssertThrowsError(try ViewModel.decodeTripActivity("{}"))
    }
}
