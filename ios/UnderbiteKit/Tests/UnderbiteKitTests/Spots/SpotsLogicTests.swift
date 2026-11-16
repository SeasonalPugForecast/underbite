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

    // MARK: - Words from the core

    private func emptyStore(_ directory: URL) -> SpotStore {
        let files = AppFiles(directory: directory)
        return SpotStore(
            files: files, host: URLSessionHost(storeURL: files.hostDirectory),
            keys: { Keys(usgs: nil, copernicusUser: nil, copernicusPassword: nil) },
            clock: { 1_800_000_000 },
            fetcher: { _, _, _, _, _, _ in throw URLError(.notConnectedToInternet) },
            discoverer: { _, _, _, _, _ in throw URLError(.notConnectedToInternet) })
    }

    private func temporaryDirectory() -> URL {
        FileManager.default.temporaryDirectory
            .appending(path: "underbite-spots-\(UUID().uuidString)", directoryHint: .isDirectory)
    }

    func testTheWaterPickerListsTheCoresKindsInItsOrder() {
        let directory = temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let options = emptyStore(directory).waterTypeOptions
        XCTAssertEqual(options.count, 10)
        XCTAssertEqual(options.first?.id, "river_stream")
        XCTAssertEqual(
            options.first { $0.id == "tidal_river_estuary" }?.label, "Tidal river or estuary")
        XCTAssertEqual(options.first { $0.id == "ice" }?.label, "Ice fishing")
    }

    func testMeasurementTitlesComeFromTheCoreById() throws {
        let titles = try measurementTitles(settingsJson: defaultSettingsJson())
        XCTAssertEqual(titles.first?.id, "tide")
        XCTAssertEqual(titles.first { $0.id == "discharge" }?.title, "River flow")
        XCTAssertEqual(titles.first { $0.id == "ph" }?.title, "pH")
    }

    /// A discovery as the core writes it (Unix time 1_800_000_000): a USGS site's kind is an
    /// object, the others are strings.
    nonisolated static let stations = """
        {
          "tide": {
            "chain": {"sources": ["coops"], "limits": {"max_distance_m": 50000.0, "max_age_s": 86400}},
            "stations": [
              {"station": {"source": "coops", "id": "8518750", "name": "The Battery", "kind": "tide_harmonic",
                           "latitude": 40.7006, "longitude": -74.0142, "bin": null, "reference_id": null},
               "distance_m": 643.7, "relation": "nearby", "coverage": "reported", "last_value_at": 1799999640,
               "age_s": 360, "primary": true, "pinned": false,
               "reasons": [{"reason": "reported", "at": 1799999640}, {"reason": "distance", "distance_m": 643.7}]}
            ],
            "dropped": [
              {"station": {"source": "ndbc", "id": "44025", "name": "Long Island", "kind": "buoy",
                           "latitude": 40.25, "longitude": -73.16, "bin": null, "reference_id": null},
               "distance_m": 90000.0, "reason": {"reason": "too_far", "distance_m": 90000.0, "limit_m": 50000.0}},
              {"station": {"source": "coops", "id": "8519483", "name": "Bergen Point", "kind": "water_level",
                           "latitude": 40.64, "longitude": -74.14, "bin": null, "reference_id": null},
               "distance_m": 9000.0, "reason": {"reason": "excluded"}}
            ]
          },
          "tidal_current": {
            "chain": {"sources": ["coops"], "limits": {"max_distance_m": 50000.0, "max_age_s": 86400}},
            "stations": [
              {"station": {"source": "coops", "id": "NYH1927", "name": "The Narrows", "kind": "current_harmonic",
                           "latitude": 40.6, "longitude": -74.04, "bin": 13, "reference_id": null},
               "distance_m": 1200.0, "relation": "nearby", "coverage": "listed", "primary": true,
               "pinned": false, "reasons": [{"reason": "listed"}, {"reason": "distance", "distance_m": 1200.0}]}
            ],
            "dropped": []
          },
          "discharge": {
            "chain": {"sources": ["usgs"], "limits": {"max_distance_m": 50000.0, "max_age_s": 86400}},
            "stations": [
              {"station": {"source": "usgs", "id": "USGS-01375000", "name": "Croton River at New Croton Dam",
                           "kind": {"usgs_site": "stream"}, "latitude": 41.22, "longitude": -73.86,
                           "bin": null, "reference_id": null},
               "distance_m": 2000.0, "relation": "upstream", "coverage": "listed", "primary": true,
               "pinned": true,
               "reasons": [{"reason": "pinned"}, {"reason": "listed"},
                           {"reason": "relation", "relation": "upstream"},
                           {"reason": "distance", "distance_m": 2000.0}]}
            ],
            "dropped": []
          },
          "waves": {"chain": {"sources": [], "limits": {"max_distance_m": 1.0, "max_age_s": 1}},
                    "stations": [], "dropped": []}
        }
        """

    func testStationRowsAreTheCoresWordsInItsOrder() throws {
        let groups = try stationRows(
            stationsJson: Self.stations, settingsJson: defaultSettingsJson(), now: 1_800_000_000)
        XCTAssertEqual(groups.map(\.title), ["Tide", "Tidal current", "River flow"])
        let battery = groups[0].rows[0]
        XCTAssertEqual(battery.key, "coops:8518750")
        XCTAssertEqual(battery.distance, "0.4 mi")
        XCTAssertEqual(battery.relation, "Nearby")
        XCTAssertEqual(battery.detail, "NOAA CO-OPS 8518750 · 0.4 mi · Nearby")
        XCTAssertEqual(battery.reasons, ["Latest value 6 min ago"])
        // The exclusion stays a row, with its switch on; the far buoy is left out with why.
        XCTAssertEqual(groups[0].rows.map(\.key), ["coops:8518750", "coops:8519483"])
        XCTAssertEqual(groups[0].rows.map(\.excluded), [false, true])
        XCTAssertEqual(groups[0].leftOutTitle, "Luna passed over 1")
        XCTAssertEqual(
            groups[0].leftOut.first?.reasons, ["56 mi from your spot, beyond your 31 mi limit"])
        XCTAssertEqual(groups[1].rows[0].key, "coops:NYH1927:13")
        // A USGS site, whose kind is an object, reads like the others.
        XCTAssertEqual(groups[2].rows[0].key, "usgs:USGS-01375000")
        XCTAssertEqual(groups[2].rows[0].relation, "Upstream")
        XCTAssertTrue(groups[2].rows[0].pinned)
    }

    func testUnreadableStationsGiveNoRows() throws {
        XCTAssertThrowsError(
            try stationRows(stationsJson: "not json", settingsJson: defaultSettingsJson(), now: 0))
        XCTAssertEqual(
            try stationRows(stationsJson: "{}", settingsJson: defaultSettingsJson(), now: 0), [])
    }

    func testStationSwitchesMapToTheCoresChoices() {
        XCTAssertEqual(StationChoice.pinSwitched(on: true), .pin)
        XCTAssertEqual(StationChoice.pinSwitched(on: false), .clear)
        XCTAssertEqual(StationChoice.excludeSwitched(on: true), .exclude)
        XCTAssertEqual(StationChoice.excludeSwitched(on: false), .clear)
    }

    func testAChoiceNotYetRankedShowsOnItsRow() throws {
        let groups = try stationRows(
            stationsJson: Self.stations, settingsJson: defaultSettingsJson(), now: 1_800_000_000)
        let battery = groups[0].rows[0]
        let excluded = groups[0].rows[1]
        XCTAssertEqual(
            StationSwitches.of(battery, pending: [:]), StationSwitches(pinned: false, excluded: false))
        XCTAssertEqual(
            StationSwitches.of(battery, pending: [battery.key: .pin]),
            StationSwitches(pinned: true, excluded: false))
        // Clearing an exclusion turns its switch off before the core ranks again.
        XCTAssertEqual(
            StationSwitches.of(excluded, pending: [excluded.key: .clear]),
            StationSwitches(pinned: false, excluded: false))
        XCTAssertEqual(
            StationSwitches.of(excluded, pending: [battery.key: .pin]),
            StationSwitches(pinned: false, excluded: true))
    }

    func testTheCoreAcceptsTheKeysTheRowsGive() throws {
        let directory = temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let host = URLSessionHost(storeURL: directory)
        let groups = try stationRows(
            stationsJson: Self.stations, settingsJson: defaultSettingsJson(), now: 1_800_000_000)
        for row in groups.flatMap({ $0.rows + $0.leftOut }) {
            try pinStation(spotId: "spot-1", key: row.key, now: 1_800_000_000, host: host)
            try excludeStation(spotId: "spot-1", key: row.key, now: 1_800_000_000, host: host)
            try clearStation(spotId: "spot-1", key: row.key, now: 1_800_000_000, host: host)
        }
    }

    // MARK: - Areas

    func testAreasAreRenamedAndReorderedInTheStore() {
        let directory = temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let store = emptyStore(directory)
        let hudson = store.addArea(named: "Hudson")
        let bay = store.addArea(named: "Bay")
        let sound = store.addArea(named: "Sound")
        XCTAssertEqual(store.arrangedAreas.map(\.name), ["Hudson", "Bay", "Sound"])

        store.renameArea(bay.id, to: "  Raritan Bay ")
        store.renameArea(hudson.id, to: "   ")
        XCTAssertEqual(store.arrangedAreas.map(\.name), ["Hudson", "Raritan Bay", "Sound"])

        let ids = store.arrangedAreas.map(\.id)
        store.reorderAreas(
            SpotGrouping.reordered(all: ids, section: ids, moving: IndexSet(integer: 2), to: 0))
        XCTAssertEqual(store.arrangedAreas.map(\.id), [sound.id, hudson.id, bay.id])
        // An order naming only some areas puts the others after them, as they were.
        store.reorderAreas([bay.id])
        XCTAssertEqual(store.arrangedAreas.map(\.id), [bay.id, sound.id, hudson.id])

        // The spots list the areas in that order, and both survive a launch.
        store.add(spot("s1", area: hudson.id))
        let sections = SpotGrouping.sections(spots: store.spots, areas: store.areas)
        XCTAssertEqual(sections.map(\.id), ["area-" + bay.id, "area-" + sound.id, "area-" + hudson.id])
        let again = emptyStore(directory)
        XCTAssertEqual(again.arrangedAreas.map(\.name), ["Raritan Bay", "Sound", "Hudson"])
    }
}
