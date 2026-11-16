import Foundation
import XCTest

@testable import UnderbiteKit

/// Grouping, reordering, typed coordinates and the station list of the Spots screens.
@MainActor
final class SpotsLogicTests: XCTestCase {
    private func spot(
        _ id: String, area: String? = nil, pinned: Bool = false, order: Int = 0
    ) -> Spot {
        Spot(
            id: id, name: id.uppercased(), latitude: 40.7, longitude: -74.0,
            waterType: "river_stream", areaID: area, pinned: pinned, order: order)
    }

    func testSectionsListPinnedFirstThenAreasInOrderThenTheRest() {
        let areas = [Area(id: "b", name: "Bay", order: 1), Area(id: "a", name: "Hudson", order: 0)]
        let spots = [
            spot("s1", area: "b", order: 0), spot("s2", area: "a", order: 1),
            spot("s3", area: "a", pinned: true, order: 2), spot("s4", order: 3),
            spot("s5", area: "gone", order: 4),
        ]
        let sections = SpotGrouping.sections(spots: spots, areas: areas)
        XCTAssertEqual(
            sections.map(\.id), ["pinned", "area-a", "area-b", "unassigned"])
        XCTAssertEqual(sections[0].spots.map(\.id), ["s3"])
        XCTAssertEqual(sections[1].spots.map(\.id), ["s2"])
        XCTAssertEqual(sections[2].spots.map(\.id), ["s1"])
        // A spot whose area was removed is listed with the others.
        XCTAssertEqual(sections[3].spots.map(\.id), ["s4", "s5"])
    }

    func testAnEmptyAreaStillHasASection() {
        let sections = SpotGrouping.sections(
            spots: [], areas: [Area(id: "a", name: "Montauk", order: 0)])
        XCTAssertEqual(sections.map(\.id), ["area-a"])
        XCTAssertTrue(sections[0].spots.isEmpty)
    }

    func testMovingWithinASectionLeavesTheOtherSectionsInPlace() {
        let all = ["a", "x", "b", "y", "c"]
        let section = ["a", "b", "c"]
        // The last row to the top (the list's own offsets: to 0).
        XCTAssertEqual(
            SpotGrouping.reordered(all: all, section: section, moving: IndexSet(integer: 2), to: 0),
            ["c", "x", "a", "y", "b"])
        // The first row below the second (destination counts before the removal).
        XCTAssertEqual(
            SpotGrouping.reordered(all: all, section: section, moving: IndexSet(integer: 0), to: 2),
            ["b", "x", "a", "y", "c"])
        // To the end.
        XCTAssertEqual(
            SpotGrouping.reordered(all: all, section: section, moving: IndexSet(integer: 0), to: 3),
            ["b", "x", "c", "y", "a"])
        // A move that changes nothing, and a row that is not there.
        XCTAssertEqual(
            SpotGrouping.reordered(all: all, section: section, moving: IndexSet(integer: 1), to: 2),
            all)
        XCTAssertEqual(
            SpotGrouping.reordered(all: all, section: section, moving: IndexSet(integer: 9), to: 0),
            all)
    }

    func testReorderingGoesThroughTheStoreByIdNotByPosition() {
        let directory = FileManager.default.temporaryDirectory
            .appending(path: "underbite-spots-\(UUID().uuidString)", directoryHint: .isDirectory)
        defer { try? FileManager.default.removeItem(at: directory) }
        let files = AppFiles(directory: directory)
        let store = SpotStore(
            files: files, host: URLSessionHost(storeURL: files.hostDirectory),
            keys: { Keys(usgs: nil, copernicusUser: nil, copernicusPassword: nil) },
            fetcher: { _, _, _, _, _, _ in throw URLError(.notConnectedToInternet) })
        for id in ["a", "b", "c"] { store.add(spot(id)) }
        let all = store.arranged.map(\.id)
        store.reorder(
            SpotGrouping.reordered(all: all, section: all, moving: IndexSet(integer: 2), to: 0))
        XCTAssertEqual(store.arranged.map(\.id), ["c", "a", "b"])
        // A spot removed after the move was computed leaves the others where they are.
        store.remove("a")
        XCTAssertEqual(store.arranged.map(\.id), ["c", "b"])
    }

    func testTypedCoordinates() throws {
        let both = try XCTUnwrap(TypedCoordinates.parse("40.7033, -74.0170"))
        XCTAssertEqual(both.latitude, 40.7033, accuracy: 1e-9)
        XCTAssertEqual(both.longitude, -74.0170, accuracy: 1e-9)
        XCTAssertNotNil(TypedCoordinates.parse("40.7033 -74.0170"))
        XCTAssertNotNil(TypedCoordinates.parse("-33.9; 151.2"))
        XCTAssertNil(TypedCoordinates.parse("91, 0"))
        XCTAssertNil(TypedCoordinates.parse("0, 181"))
        XCTAssertNil(TypedCoordinates.parse("40.7"))
        XCTAssertNil(TypedCoordinates.parse("north, west"))
        XCTAssertNil(TypedCoordinates.parse("1, 2, 3"))
        XCTAssertNil(TypedCoordinates.parse("nan, 0"))
    }

    func testWaterKindsCoverTheCoreAndLabelUnknownOnes() {
        XCTAssertEqual(WaterKind.all.count, 10)
        XCTAssertEqual(WaterKind.label("tidal_river_estuary"), "Tidal river or estuary")
        XCTAssertEqual(WaterKind.label("salt_pond"), "Salt Pond")
    }

    private static let stations = """
        {
          "water_temperature": {
            "chain": {"sources": ["coops"], "limits": {"max_distance_m": 50000, "max_age_s": 86400}},
            "stations": [
              {"station": {"source": "coops", "id": "8518750", "name": "The Battery", "kind": "tide_gauge",
                           "latitude": 40.7, "longitude": -74.0, "bin": null, "reference_id": null},
               "distance_m": 300.0, "relation": "nearby", "coverage": "reported", "primary": true,
               "pinned": true, "reasons": [{"reason": "pinned"}]}
            ],
            "dropped": [
              {"station": {"source": "coops", "id": "8519483", "name": "Bergen Point", "kind": {"other": "tide_gauge"},
                           "latitude": 40.6, "longitude": -74.1, "bin": null, "reference_id": null},
               "distance_m": 9000.0, "reason": {"reason": "excluded"}},
              {"station": {"source": "ndbc", "id": "44025", "name": "Long Island", "kind": "buoy",
                           "latitude": 40.2, "longitude": -73.1, "bin": null, "reference_id": null},
               "distance_m": 90000.0, "reason": {"reason": "too_far", "distance_m": 90000.0, "limit_m": 50000.0}}
            ]
          },
          "tidal_current": {
            "chain": {"sources": ["coops"], "limits": {"max_distance_m": 50000, "max_age_s": 86400}},
            "stations": [
              {"station": {"source": "coops", "id": "NYH1927", "name": "The Narrows", "kind": "current",
                           "latitude": 40.6, "longitude": -74.0, "bin": 13, "reference_id": null},
               "distance_m": 1200.0, "relation": "nearby", "coverage": "listed", "primary": true,
               "pinned": false, "reasons": []}
            ],
            "dropped": []
          },
          "tide": {
            "chain": {"sources": ["coops"], "limits": {"max_distance_m": 50000, "max_age_s": 86400}},
            "stations": [
              {"station": {"source": "coops", "id": "8518750", "name": "The Battery", "kind": "tide_gauge",
                           "latitude": 40.7, "longitude": -74.0, "bin": null, "reference_id": null},
               "distance_m": 300.0, "relation": "nearby", "coverage": "reported", "primary": true,
               "pinned": false, "reasons": []}
            ],
            "dropped": []
          },
          "waves": {"chain": {"sources": [], "limits": {"max_distance_m": 1, "max_age_s": 1}},
                    "stations": [], "dropped": []}
        }
        """

    func testTheStationListReadsPinsExclusionsAndBinsInTheCoresOrder() {
        let listing = StationListing(json: Self.stations)
        XCTAssertEqual(listing.groups.map(\.title), ["Tide", "Tidal current", "Water temperature"])
        let temperature = listing.groups[2].rows
        XCTAssertEqual(temperature.map(\.key), ["coops:8518750", "coops:8519483"])
        XCTAssertEqual(temperature.map(\.pinned), [true, false])
        // Only an exclusion is a row among the dropped ones; "too far" is not a choice.
        XCTAssertEqual(temperature.map(\.excluded), [false, true])
        XCTAssertEqual(listing.groups[1].rows[0].key, "coops:NYH1927:13")
        XCTAssertFalse(listing.isEmpty)
    }

    func testAnUnreadableListIsEmpty() {
        XCTAssertTrue(StationListing(json: nil).isEmpty)
        XCTAssertTrue(StationListing(json: "not json").isEmpty)
        XCTAssertTrue(StationListing(json: "{}").groups.isEmpty)
    }

    func testStationSwitchesMapToTheCoresChoices() {
        XCTAssertEqual(StationChoice.pinSwitched(on: true), .pin)
        XCTAssertEqual(StationChoice.pinSwitched(on: false), .clear)
        XCTAssertEqual(StationChoice.excludeSwitched(on: true), .exclude)
        XCTAssertEqual(StationChoice.excludeSwitched(on: false), .clear)
    }

    func testTheCoreAcceptsTheKeysTheRowsBuild() throws {
        let directory = FileManager.default.temporaryDirectory
            .appending(path: "underbite-pins-\(UUID().uuidString)", directoryHint: .isDirectory)
        defer { try? FileManager.default.removeItem(at: directory) }
        let host = URLSessionHost(storeURL: directory)
        let rows = StationListing(json: Self.stations).groups.flatMap(\.rows)
        for row in rows {
            try pinStation(spotId: "spot-1", key: row.key, now: 1_800_000_000, host: host)
            try excludeStation(spotId: "spot-1", key: row.key, now: 1_800_000_000, host: host)
            try clearStation(spotId: "spot-1", key: row.key, now: 1_800_000_000, host: host)
        }
    }
}
