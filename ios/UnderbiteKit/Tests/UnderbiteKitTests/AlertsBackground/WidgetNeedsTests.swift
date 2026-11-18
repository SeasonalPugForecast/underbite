import Foundation
import XCTest

@testable import UnderbiteKit

/// Which saved spots a background refresh fetches for the installed widgets.
final class WidgetNeedsTests: XCTestCase {
    private let spots = WidgetSpots(
        spots: [
            WidgetSpot(id: "battery", name: "The Battery"),
            WidgetSpot(id: "croton", name: "Croton River"),
            WidgetSpot(id: "buoy", name: "Buoy 44025"),
        ],
        appSpotID: "croton")

    func testNoWidgetsNeedNothing() {
        XCTAssertEqual(
            WidgetNeeds.spotIDs(installed: [], spots: spots, nextTripPlaces: ["battery"]), [])
    }

    func testEachSpotWidgetNeedsItsSpotOnce() {
        let installed = [
            WidgetNeeds.Installed(kind: .tide, choice: .spot("battery")),
            WidgetNeeds.Installed(kind: .flow),
            WidgetNeeds.Installed(kind: .snack, choice: .spot("battery")),
            // A spot deleted since: the widget follows the app's spot.
            WidgetNeeds.Installed(kind: .tide, choice: .spot("gone")),
            // A kind this version does not know is taken to show a spot.
            WidgetNeeds.Installed(kind: nil, choice: .spot("buoy")),
        ]
        XCTAssertEqual(
            WidgetNeeds.spotIDs(installed: installed, spots: spots, nextTripPlaces: []),
            ["battery", "croton", "buoy"])
    }

    func testTheNextTripWidgetNeedsTheTripsPlaces() {
        let installed = [
            WidgetNeeds.Installed(kind: .nextTrip), WidgetNeeds.Installed(kind: .flow),
        ]
        XCTAssertEqual(
            WidgetNeeds.spotIDs(
                installed: installed, spots: spots, nextTripPlaces: ["battery", "croton"]),
            ["battery", "croton"])
    }

    func testWithNoSpotsASpotWidgetNeedsNothing() {
        XCTAssertEqual(
            WidgetNeeds.spotIDs(
                installed: [WidgetNeeds.Installed(kind: .tide)], spots: WidgetSpots(),
                nextTripPlaces: []),
            [])
    }

    func testATripsPlacesAreTheSavedOnesAForecastReaches() {
        let now: Int64 = 1_794_139_200
        let today = TripDates.day(
            of: Date(timeIntervalSince1970: TimeInterval(now)),
            in: TimeZone(identifier: "America/New_York") ?? .gmt)
        let saved = [SeedSpots.battery, SeedSpots.croton]
        let soon = TripSeeds.trip(today: today, spots: saved)
        XCTAssertEqual(
            WidgetNeeds.tripPlaces(soon, saved: [SeedSpots.croton.id], now: now),
            [SeedSpots.croton.id])
        // A month away: no forecast reaches it yet.
        let later = TripSeeds.trip(today: TripDates.adding(30, to: today) ?? today, spots: saved)
        XCTAssertEqual(WidgetNeeds.tripPlaces(later, saved: Set(saved.map(\.id)), now: now), [])
        XCTAssertEqual(WidgetNeeds.tripPlaces("not a trip", saved: [], now: now), [])
    }
}
