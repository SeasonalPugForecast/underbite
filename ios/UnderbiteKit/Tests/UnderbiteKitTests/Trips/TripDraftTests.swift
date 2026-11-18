import Foundation
import XCTest

@testable import UnderbiteKit

/// The trip editor's draft: edits written onto the trip's JSON, what it does not edit kept, and
/// the core's checks of the result.
final class TripDraftTests: XCTestCase {
    private static let spots = [SeedSpots.battery, SeedSpots.croton, SeedSpots.buoy]

    private func object(_ json: String) throws -> [String: Any] {
        try XCTUnwrap(JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any])
    }

    private func json(_ draft: TripDraft, settings: String = "{}") -> String {
        draft.json(spots: Self.spots, areas: [], settingsJSON: settings)
    }

    func testANewTripAtSavedSpotsIsOneTheCoreCanPlan() throws {
        var draft = TripDraft(first: "2027-07-12", places: [SeedSpots.croton.id])
        draft.name = "  Croton week  "
        draft.toggle(SeedSpots.battery.id)
        XCTAssertEqual(draft.last, "2027-07-14", "three days by default")
        let text = json(draft)
        XCTAssertNil(tripProblem(tripJson: text))
        let trip = try object(text)
        XCTAssertEqual(trip["name"] as? String, "Croton week")
        XCTAssertEqual(trip["id"] as? String, "", "the store gives a new trip its id")
        let places = try XCTUnwrap(trip["places"] as? [[String: Any]])
        XCTAssertEqual(places.compactMap { $0["spot_id"] as? String }, ["seed-croton", "seed-battery"])
        XCTAssertEqual(places.first?["time_zone"] as? String, "America/New_York")
        XCTAssertEqual(places.first?["water_type"] as? String, "river_stream")
        XCTAssertEqual(places.first?["fuzz"] as? String, "exact")
        XCTAssertEqual((places.first?["latitude"] as? NSNumber)?.doubleValue, SeedSpots.croton.latitude)
    }

    func testTheCoreSaysWhatIsWrong() {
        var draft = TripDraft(first: "2027-07-12", places: [SeedSpots.battery.id])
        draft.name = "Trip"
        draft.last = "2027-07-11"
        XCTAssertEqual(tripProblem(tripJson: json(draft)), "The last day is before the first.")
        draft.last = "2027-08-12"
        XCTAssertEqual(tripProblem(tripJson: json(draft)), "The trip is longer than 31 days.")
        draft.last = "2027-08-11"
        XCTAssertNil(tripProblem(tripJson: json(draft)), "31 days is allowed")
        draft.toggle(SeedSpots.battery.id)
        XCTAssertEqual(tripProblem(tripJson: json(draft)), "The trip has no place.")
        // A spot whose zone is not known yet.
        var spot = SeedSpots.battery
        spot.timeZone = ""
        draft.toggle(spot.id)
        let text = draft.json(spots: [spot], areas: [], settingsJSON: "{}")
        XCTAssertEqual(tripProblem(tripJson: text), "The Battery: unknown time zone.")
    }

    func testEditingKeepsWhatTheEditorDoesNotChange() throws {
        let shared = """
            {"id": "t1", "name": "Shared", "first": "2027-07-12", "last": "2027-07-13",
             "method": "kayak", "species": ["morone-saxatilis"], "notes": "", "modified_at": 5,
             "checklist": [{"id": "c1", "text": "Bait", "done": true}],
             "places": [{"spot_id": "friend-spot", "name": "Friend's spot", "time_zone": "America/New_York",
                         "water_type": "tidal_river_estuary", "latitude": 40.7, "longitude": -74.02,
                         "fuzz": "rounded_1km", "region": "US-NY"}]}
            """
        var draft = TripDraft(json: shared)
        XCTAssertEqual(draft.id, "t1")
        XCTAssertEqual(draft.method, "kayak")
        XCTAssertEqual(draft.species, "morone-saxatilis")
        XCTAssertEqual(draft.placeIDs, ["friend-spot"])
        XCTAssertEqual(draft.otherPlaces(besides: Self.spots).map(\.name), ["Friend's spot"])
        draft.name = "Shared, edited"
        draft.toggle(SeedSpots.battery.id)
        let trip = try object(json(draft))
        XCTAssertEqual(trip["id"] as? String, "t1")
        XCTAssertEqual((trip["modified_at"] as? NSNumber)?.intValue, 5)
        XCTAssertEqual((trip["checklist"] as? [[String: Any]])?.first?["text"] as? String, "Bait")
        let places = try XCTUnwrap(trip["places"] as? [[String: Any]])
        XCTAssertEqual(places.count, 2)
        XCTAssertEqual(places[0]["fuzz"] as? String, "rounded_1km", "a shared place stays as it was")
        XCTAssertEqual(places[0]["region"] as? String, "US-NY")
        XCTAssertEqual(places[1]["spot_id"] as? String, SeedSpots.battery.id)
        XCTAssertNil(tripProblem(tripJson: json(draft)))
    }

    func testSpeciesAreIdsTheCoreReads() {
        XCTAssertEqual(
            TripDraft.speciesIDs("Morone saxatilis, striped-bass;  Morone  SAXATILIS \n, ,Pomatomus_saltatrix"),
            ["morone-saxatilis", "striped-bass", "pomatomus-saltatrix"])
        XCTAssertEqual(TripDraft.speciesIDs(" , ;"), [])
        var draft = TripDraft(first: "2027-07-12", places: [SeedSpots.battery.id])
        draft.name = "Trip"
        draft.species = "Bluefish (Pomatomus saltatrix)!, Salmo trutta"
        XCTAssertNil(tripProblem(tripJson: json(draft)))
    }

    func testTheTripCarriesACopyOfTheChosenLimits() throws {
        let settings = """
            {"limits": [{"name": "Kayak", "max_wind_kn": 12.0, "no_lightning": true},
                        {"name": "Boat", "max_wave_m": 1.2}]}
            """
        XCTAssertEqual(TripDraft.profileNames(settings), ["Kayak", "Boat"])
        var draft = TripDraft(first: "2027-07-12", places: [SeedSpots.battery.id])
        draft.name = "Trip"
        draft.limits = "Boat"
        let trip = try object(json(draft, settings: settings))
        let limits = try XCTUnwrap(trip["limits"] as? [String: Any])
        XCTAssertEqual(limits["name"] as? String, "Boat")
        XCTAssertEqual((limits["max_wave_m"] as? NSNumber)?.doubleValue, 1.2)
        XCTAssertNil(tripProblem(tripJson: json(draft, settings: settings)))

        // Its copy stays while the same name is chosen, even once the profile changed.
        var again = TripDraft(json: json(draft, settings: settings))
        XCTAssertEqual(again.limits, "Boat")
        let changed = #"{"limits": [{"name": "Boat", "max_wave_m": 2.0}]}"#
        let kept = try object(again.json(spots: Self.spots, areas: [], settingsJSON: changed))
        XCTAssertEqual(((kept["limits"] as? [String: Any])?["max_wave_m"] as? NSNumber)?.doubleValue, 1.2)

        again.limits = nil
        XCTAssertNil(try object(json(again, settings: settings))["limits"])
    }

    func testANewPlaceTakesItsAreaByName() throws {
        var spot = SeedSpots.croton
        let area = Area(name: "Lower Hudson")
        spot.areaID = area.id
        var draft = TripDraft(first: "2027-07-12", places: [spot.id])
        draft.name = "Trip"
        let trip = try object(draft.json(spots: [spot], areas: [area], settingsJSON: "{}"))
        let place = try XCTUnwrap((trip["places"] as? [[String: Any]])?.first)
        XCTAssertEqual(place["area"] as? String, "Lower Hudson")
    }
}
