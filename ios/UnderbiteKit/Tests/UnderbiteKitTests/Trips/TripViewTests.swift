import Foundation
import XCTest

@testable import UnderbiteKit

/// The trip view from the real core on this machine, decoded into the Swift types: the places
/// and days in order, every cell with its basis worded, and what a later core adds ignored.
final class TripViewTests: XCTestCase {
    private static let spots = [SeedSpots.battery, SeedSpots.croton]

    /// Three days at the test places from the sample snapshot's day.
    private static var tripJSON: String {
        var draft = TripDraft(first: "2026-11-06", days: 3, places: spots.map(\.id))
        draft.id = "trip-view-test"
        draft.name = "Hudson stripers"
        return draft.json(spots: spots, areas: [], settingsJSON: "{}")
    }

    /// The Battery with the sample snapshot, the Croton River with nothing fetched yet.
    private func view() throws -> ViewModel.TripView {
        try ViewModel.buildTrip(
            tripJSON: Self.tripJSON,
            places: [
                TripPlaceData(
                    key: SeedSpots.battery.id, snapshotJson: SampleSnapshot.json, historyJson: nil)
            ],
            cachedMetadata: [], settingsJSON: defaultSettingsJson(),
            now: Date(timeIntervalSince1970: TimeInterval(SampleSnapshot.fetchedAt + 360)))
    }

    func testTheGridHasADayRowPerDateAndACellPerPlace() throws {
        let view = try view()
        XCTAssertEqual(view.title, "Hudson stripers")
        XCTAssertEqual(view.places.map(\.key), Self.spots.map(\.id))
        XCTAssertEqual(view.places.map(\.name), ["The Battery", "Croton River"])
        XCTAssertEqual(view.days.map(\.date), ["2026-11-06", "2026-11-07", "2026-11-08"])
        for day in view.days {
            XCTAssertFalse(day.label.isEmpty)
            XCTAssertFalse(day.accessibility.isEmpty)
            XCTAssertEqual(day.places.count, 2)
            for (index, placeDay) in day.places.enumerated() {
                let placeDay = try XCTUnwrap(placeDay)
                // A river has no tide cell and a tidal place with no flow source has no flow cell.
                let common = ["verdict", "windows", "water_temp", "sun", "moon", "solunar"]
                let expected =
                    index == 0
                    ? ["verdict", "windows", "tides", "water_temp", "sun", "moon", "solunar"]
                    : ["verdict", "windows", "flow", "water_temp", "sun", "moon", "solunar"]
                XCTAssertTrue(Set(common).isSubset(of: Set(expected)))
                XCTAssertEqual(placeDay.cells.map(\.kind), expected)
                XCTAssertFalse(placeDay.accessibility.isEmpty)
                for cell in placeDay.cells {
                    XCTAssertFalse(cell.label.isEmpty, cell.kind)
                    XCTAssertFalse(cell.value.isEmpty, cell.kind)
                    XCTAssertFalse(cell.basisLabel.isEmpty, "\(cell.kind) says what it rests on")
                    XCTAssertFalse(cell.accessibility.isEmpty, cell.kind)
                }
            }
        }
        XCTAssertEqual(view.days[0].places.map { $0?.place }, Self.spots.map { Optional($0.id) })
        XCTAssertFalse(view.attribution.isEmpty, "the sources are credited")
    }

    func testEachCellSaysWhatItRestsOn() throws {
        let view = try view()
        let battery = try XCTUnwrap(view.days[0].places[0])
        let tides = try XCTUnwrap(battery.cell("tides"))
        XCTAssertEqual(tides.basis, "certain", tides.basisLabel)
        XCTAssertEqual(tides.lines.count, 2, "the day's high and low: \(tides.lines)")
        XCTAssertTrue(tides.lines[0].contains("4:30 PM"), tides.lines[0])
        XCTAssertEqual(try XCTUnwrap(battery.cell("sun")).basis, "certain")

        let croton = try XCTUnwrap(view.days[0].places[1])
        let flow = try XCTUnwrap(croton.cell("flow"))
        XCTAssertTrue(flow.isUnavailable, "nothing fetched for the river yet")
        XCTAssertFalse(flow.isTypical)
        XCTAssertTrue(flow.basisLabel.lowercased().contains("unavailable"), flow.basisLabel)
        XCTAssertEqual(try XCTUnwrap(croton.cell("sun")).basis, "certain")
        XCTAssertNil(croton.cell("no-such-kind"))
    }

    func testFieldsALaterCoreAddsAreIgnored() throws {
        let json = try tripView(
            tripJson: Self.tripJSON, places: [], cachedMetadata: [],
            app: TripAppStatus(pack: .notDownloaded, sharing: .notShared),
            settingsJson: defaultSettingsJson(), now: SampleSnapshot.fetchedAt)
        let later = try Fixtures.string(Fixtures.addingUnknownFields(Fixtures.object(json)))
        XCTAssertEqual(try ViewModel.decodeTrip(later), try ViewModel.decodeTrip(json))
    }

    func testATypicalCellIsReadAsHistory() throws {
        let view = try ViewModel.decodeTrip(Self.typical)
        let cell = try XCTUnwrap(view.days[0].places[0]?.cell("flow"))
        XCTAssertTrue(cell.isTypical)
        XCTAssertFalse(cell.isUnavailable)
        XCTAssertEqual(cell.source, "USGS")
        XCTAssertNil(view.days[0].places[1], "a place without the day")
        XCTAssertEqual(view.place("croton")?.problem, "No days for this place.")
        XCTAssertNil(view.place("elsewhere"))
        XCTAssertNil(view.compact.verdict)
        XCTAssertEqual(view.checklist.first?.done, false)
    }

    /// A hand-made trip view in the core's shape: one typical flow cell, one missing place day.
    private static let typical = #"""
        {"title":"T","dates":"Nov 8","first":"2026-11-08","status":"Starts in 3 days",
         "is_past":false,"is_upcoming":true,
         "places":[{"key":"battery","name":"The Battery","water_type_label":"Tidal river"},
                   {"key":"croton","name":"Croton River","water_type_label":"River",
                    "problem":"No days for this place."}],
         "days":[{"date":"2026-11-08","label":"Sun Nov 8","accessibility":"Sun Nov 8.",
                  "places":[{"place":"battery","is_past":false,"is_today":false,"verdict":"go",
                             "storm":false,"reasons":[],"windows":[],"accessibility":"A.",
                             "cells":[{"kind":"flow","label":"River flow","value":"706 cfs",
                                       "lines":["Most years 400 to 1,200 cfs"],"basis":"typical",
                                       "basis_label":"typical for these dates, 1991–2020, station records",
                                       "source":"USGS","accessibility":"River flow."}]},
                            null]}],
         "checklist":[{"kind":"item","id":"a","text":"Waders","done":false}],
         "attribution":[{"source":"USGS","text":"USGS Water Data"}],
         "notices":[{"kind":"not_for_navigation","text":"Not for navigation."}],
         "notes":"",
         "compact":{"title":"Next trip: T","when":"Starts in 3 days","headline":"No verdict yet",
                    "line":"","storm":false,"accessibility":"Next trip."}}
        """#
}
