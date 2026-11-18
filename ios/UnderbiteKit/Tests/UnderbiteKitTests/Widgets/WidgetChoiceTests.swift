import Foundation
import XCTest

@testable import UnderbiteKit

/// Which spot a widget shows and whether its entry has what it draws.
final class WidgetChoiceTests: XCTestCase {
    private let battery = WidgetSpot(id: "battery", name: "The Battery")
    private let croton = WidgetSpot(id: "croton", name: "Croton River")

    func testAChoiceKeepsItsID() {
        XCTAssertEqual(WidgetSpotChoice(id: nil), .app)
        XCTAssertEqual(WidgetSpotChoice(id: ""), .app)
        XCTAssertEqual(WidgetSpotChoice(id: WidgetSpotChoice.appID), .app)
        XCTAssertEqual(WidgetSpotChoice(id: "croton"), .spot("croton"))
        XCTAssertEqual(WidgetSpotChoice.app.id, "@app")
        XCTAssertEqual(WidgetSpotChoice(id: WidgetSpotChoice.spot("croton").id), .spot("croton"))
    }

    func testAWidgetShowsItsSpotElseTheAppsElseTheFirst() {
        let spots = WidgetSpots(spots: [battery, croton], appSpotID: "croton")
        XCTAssertEqual(spots.spot(for: .spot("battery")), battery)
        XCTAssertFalse(spots.followsApp(.spot("battery")))
        XCTAssertEqual(spots.spot(for: .app), croton)
        XCTAssertTrue(spots.followsApp(.app))
        // A spot deleted since: the app's.
        XCTAssertEqual(spots.spot(for: .spot("gone")), croton)
        XCTAssertTrue(spots.followsApp(.spot("gone")))
        // The app's spot deleted too: the first.
        let noApp = WidgetSpots(spots: [battery, croton], appSpotID: "gone")
        XCTAssertEqual(noApp.spot(for: .app), battery)
        XCTAssertNil(WidgetSpots().spot(for: .app))
        XCTAssertNil(WidgetSpots().spot(for: .spot("battery")))
    }

    func testAnEntityLookupKeepsTheIdsOrder() {
        let spots = WidgetSpots(spots: [battery, croton])
        XCTAssertEqual(spots.spots(for: ["croton", "gone", "battery"]), [croton, battery])
        XCTAssertEqual(spots.spots(for: [WidgetSpotChoice.appID]), [])
    }

    func testEachWidgetKnowsWhatItDraws() throws {
        let battery = try ViewModel.decodeWidget(Fixtures.text("sample-widget-battery"))
        let croton = try ViewModel.decodeWidget(Fixtures.text("sample-widget-croton"))
        let trip = try ViewModel.decodeWidget(Fixtures.text("sample-widget-trip"))
        let b = try XCTUnwrap(battery.entries.first)
        let c = try XCTUnwrap(croton.entries.first)
        let t = try XCTUnwrap(trip.entries.first)
        XCTAssertTrue(WidgetKind.tide.hasData(in: b))
        XCTAssertFalse(WidgetKind.flow.hasData(in: b))
        XCTAssertTrue(WidgetKind.snack.hasData(in: b))
        XCTAssertFalse(WidgetKind.nextTrip.hasData(in: b))
        XCTAssertFalse(WidgetKind.tide.hasData(in: c))
        XCTAssertTrue(WidgetKind.flow.hasData(in: c))
        XCTAssertTrue(WidgetKind.nextTrip.hasData(in: t))
        // The trip's second place is the Croton; there is no third.
        XCTAssertTrue(WidgetKind.flow.hasData(in: t, place: 1))
        XCTAssertFalse(WidgetKind.tide.hasData(in: t, place: 2))
        XCTAssertEqual(WidgetKind.allCases.filter(\.showsSpot), [.tide, .flow, .snack])
        XCTAssertEqual(WidgetKind.nextTrip.rawValue, "next_trip")
    }
}
