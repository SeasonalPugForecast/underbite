import Foundation
import MapKit
import XCTest

@testable import UnderbiteKit

/// The station and spot pins: the core's roles and kinds in the app's marks, the grouping at a
/// zoom, and which mark a tap meets.
final class StationPinsTests: XCTestCase {
    /// The Battery (CO-OPS 8518750).
    private let battery = (latitude: 40.7006, longitude: -74.0142)

    private func pin(
        _ id: String, _ latitude: Double, _ longitude: Double, kind: PinKind = .tide,
        layer: String = "tide-stations"
    ) -> MapPin {
        MapPin(
            id: id, kind: kind, layer: layer, name: id, label: "", role: .none,
            latitude: latitude, longitude: longitude)
    }

    /// The map point `points` screen points east of `from` at `perPoint` map points a point.
    private func east(of from: MapPin, points: Double, perPoint: Double, id: String) -> MapPin {
        let at = MKMapPoint(x: from.x + points * perPoint, y: from.y).coordinate
        return pin(id, at.latitude, at.longitude)
    }

    func testEveryRoleTheCoreNamesHasItsOwnMark() {
        let core = stationPinRoles()
        XCTAssertFalse(core.isEmpty)
        for name in core {
            XCTAssertNotNil(PinRole(rawValue: name), "\(name) has no colour in the app")
        }
        XCTAssertEqual(Set(PinRole.allCases.map(\.rawValue)), Set(core))
        XCTAssertEqual(PinRole(core: "a_role_from_later"), .unknown)
    }

    func testStatesAreNeverColourAlone() {
        // The graded states each have their own colour...
        let graded: [PinRole] = [
            .reporting, .low, .normal, .elevated, .floodMinor, .floodModerate, .floodMajor,
        ]
        XCTAssertEqual(Set(graded.map(\.colour)).count, graded.count)
        // ...and what matters most is marked besides: no state is hollow, attention has a badge.
        XCTAssertEqual(PinRole.unknown.mark, .unknown)
        XCTAssertEqual(PinRole.noData.mark, .hollow)
        for role in [PinRole.elevated, .floodMinor, .floodModerate, .floodMajor] {
            XCTAssertEqual(role.mark, .alert, role.rawValue)
        }
        for role in [PinRole.none, .reporting, .low, .normal] {
            XCTAssertEqual(role.mark, .solid, role.rawValue)
        }
    }

    func testSymbolsStandOutOnTheirFill() {
        for role in PinRole.allCases {
            // Graphics need 3:1 against what is next to them.
            XCTAssertGreaterThanOrEqual(
                role.colour.contrast(with: role.glyph), 3, "\(role.rawValue) symbol")
        }
        // A hollow pin's outline and symbol stand on white.
        for role in [PinRole.unknown, .noData] {
            XCTAssertGreaterThanOrEqual(role.colour.contrast(with: .white), 4.5, role.rawValue)
        }
    }

    func testEachKindHasItsOwnShapeAndSymbol() {
        let core = ["tide", "current", "buoy", "river_gauge", "forecast_gauge"]
        let kinds = core.map(PinKind.init(core:))
        XCTAssertEqual(kinds, [.tide, .current, .buoy, .riverGauge, .forecastGauge])
        let all = kinds + [.spot]
        XCTAssertEqual(Set(all.map(\.symbol)).count, all.count)
        XCTAssertEqual(Set(all.map { "\($0.shape)" }).count, all.count)
        // The core never makes a spot, and a kind from later still shows.
        XCTAssertEqual(PinKind(core: "spot"), .other)
        XCTAssertEqual(PinKind(core: "lighthouse"), .other)
    }

    func testAStationPinIsPlacedWhereMapKitPutsIt() {
        let station = StationPin(
            id: "coops:8518750", kind: "tide", layer: "tide-stations", name: "The Battery",
            lat: battery.latitude, lon: battery.longitude, stateRole: "none",
            label: "Tide station")
        let made = MapPin(station)
        let point = MKMapPoint(
            CLLocationCoordinate2D(latitude: battery.latitude, longitude: battery.longitude))
        XCTAssertEqual(made.x, point.x, accuracy: 1e-6)
        XCTAssertEqual(made.y, point.y, accuracy: 1e-6)
        XCTAssertEqual(made.kind, .tide)
        XCTAssertEqual(made.role, .none)
        XCTAssertEqual(made.label, "Tide station")
        XCTAssertNil(made.spotID)
    }

    func testTheZoomGivesMapKitsScale() {
        // A view 390 points wide showing 1/1024 of the world.
        let mapPoints = MapRegion.worldMapPoints / 1024
        let zoom = MapRegion.zoom(viewPoints: 390, mapPoints: mapPoints)
        XCTAssertEqual(
            StationPins.mapPointsPerPoint(zoom: zoom), mapPoints / 390, accuracy: 1e-6)
    }

    func testOnlyTheShownLayersAndSpotsFirst() {
        let tide = pin("coops:8518750", battery.latitude, battery.longitude)
        let buoy = pin("ndbc:44025", 40.251, -73.164, kind: .buoy, layer: "buoys")
        let spot = MapPin.spot(id: "s1", name: "Pier", latitude: 40.70, longitude: -74.02)
        XCTAssertEqual(spot.layer, StationPins.spotsLayer)
        XCTAssertEqual(spot.spotID, "s1")
        let shown = StationPins.shown(
            stations: [tide, buoy], spots: [spot], layers: ["tide-stations", "spots"])
        XCTAssertEqual(shown.map(\.id), ["spot:s1", "coops:8518750"])
    }

    func testTheFewStationsLeadTheManyGauges() {
        let gauge = pin(
            "usgs:USGS-01376500", 40.71, -74.01, kind: .riverGauge, layer: "usgs-gauges")
        let tide = pin("coops:8518750", battery.latitude, battery.longitude)
        let buoy = pin("ndbc:44065", 40.369, -73.703, kind: .buoy, layer: "buoys")
        let other = pin("usgs:USGS-01300000", 40.9, -73.8, kind: .riverGauge, layer: "usgs-gauges")
        let layers: Set<String> = ["tide-stations", "buoys", "usgs-gauges"]
        let shown = StationPins.shown(
            stations: [gauge, other, buoy, tide], spots: [], layers: layers)
        // Kind first, then the core's order within a kind.
        XCTAssertEqual(
            shown.map(\.id),
            ["coops:8518750", "ndbc:44065", "usgs:USGS-01376500", "usgs:USGS-01300000"])
        // So the tide station leads the group it shares with a gauge next to it.
        let perPoint = StationPins.mapPointsPerPoint(zoom: 9)
        let grouped = StationPins.clusters(shown, mapPointsPerPoint: perPoint)
        XCTAssertEqual(grouped.first?.pins.map(\.id), ["coops:8518750", "usgs:USGS-01376500"])
    }

    func testPinsCloseOnScreenShareAMarkUntilZoomedIn() {
        let perPoint = StationPins.mapPointsPerPoint(zoom: 10)
        let first = pin("a", battery.latitude, battery.longitude)
        let near = east(of: first, points: 12, perPoint: perPoint, id: "b")
        let far = east(of: first, points: 80, perPoint: perPoint, id: "c")

        let grouped = StationPins.clusters([first, near, far], mapPointsPerPoint: perPoint)
        XCTAssertEqual(grouped.map { $0.pins.map(\.id) }, [["a", "b"], ["c"]])
        // The mark stands at its first pin, so it does not move as others join.
        XCTAssertEqual(grouped[0].x, first.x)
        XCTAssertNil(grouped[0].single)
        XCTAssertEqual(grouped[1].single?.id, "c")

        // Four times closer in, 12 points become 48: apart.
        let apart = StationPins.clusters([first, near, far], mapPointsPerPoint: perPoint / 4)
        XCTAssertEqual(apart.map { $0.pins.map(\.id) }, [["a"], ["b"], ["c"]])

        // The same pins at the same zoom group the same way.
        XCTAssertEqual(
            StationPins.clusters([first, near, far], mapPointsPerPoint: perPoint), grouped)
    }

    func testAPinJoinsTheNearestMark() {
        let perPoint = 1.0
        let left = pin("left", 0, 0)
        let right = MapPin(
            id: "right", kind: .tide, layer: "tide-stations", name: "", label: "", role: .none,
            latitude: MKMapPoint(x: left.x + 40, y: left.y).coordinate.latitude,
            longitude: MKMapPoint(x: left.x + 40, y: left.y).coordinate.longitude)
        let between = MapPin(
            id: "between", kind: .tide, layer: "tide-stations", name: "", label: "", role: .none,
            latitude: MKMapPoint(x: left.x + 25, y: left.y).coordinate.latitude,
            longitude: MKMapPoint(x: left.x + 25, y: left.y).coordinate.longitude)
        let grouped = StationPins.clusters([left, right, between], mapPointsPerPoint: perPoint)
        XCTAssertEqual(grouped.map { $0.pins.map(\.id) }, [["left"], ["right", "between"]])
    }

    func testTheActiveSpotStandsApartAndIsNotDrawn() {
        let perPoint = StationPins.mapPointsPerPoint(zoom: 10)
        let spot = MapPin.spot(
            id: "s1", name: "Pier", latitude: battery.latitude, longitude: battery.longitude)
        let station = east(of: spot, points: 5, perPoint: perPoint, id: "coops:8518750")
        let grouped = StationPins.clusters(
            [spot, station], mapPointsPerPoint: perPoint, apart: spot.id)
        XCTAssertEqual(grouped.count, 2)
        XCTAssertEqual(grouped.first { $0.drawn }?.single?.id, "coops:8518750")
        XCTAssertEqual(grouped.first { !$0.drawn }?.single?.id, "spot:s1")
    }

    func testATapMeetsTheNearestMarkWithinReach() {
        let perPoint = StationPins.mapPointsPerPoint(zoom: 12)
        let a = pin("a", battery.latitude, battery.longitude)
        let b = east(of: a, points: 60, perPoint: perPoint, id: "b")
        let marks = StationPins.clusters([a, b], mapPointsPerPoint: perPoint)
        XCTAssertEqual(marks.count, 2)

        func tap(_ points: Double) -> String? {
            StationPins.hit(
                marks, x: a.x + points * perPoint, y: a.y, mapPointsPerPoint: perPoint)?
                .single?.id
        }
        XCTAssertEqual(tap(0), "a")
        XCTAssertEqual(tap(20), "a")
        XCTAssertEqual(tap(40), "b")
        XCTAssertEqual(tap(-StationPins.reach - 1), nil)
        XCTAssertNil(StationPins.hit([], x: a.x, y: a.y, mapPointsPerPoint: perPoint))
    }

    func testASpotAtAStationIsFound() {
        let station = pin("coops:8518750", battery.latitude, battery.longitude)
        // About 11 m north, and about 1.1 km north.
        let spots = [
            (id: "far", latitude: battery.latitude + 0.01, longitude: battery.longitude),
            (id: "near", latitude: battery.latitude + 0.0001, longitude: battery.longitude),
        ]
        XCTAssertEqual(StationPins.spot(at: station, among: spots), "near")
        XCTAssertNil(StationPins.spot(at: station, among: Array(spots.prefix(1))))
    }

    func testTheSameQueryIsNotAskedTwice() {
        let region = MapRegion(west: -74.2, south: 40.5, east: -73.8, north: 40.9, zoom: 11)
        let a = PinQuery(region: region, layers: ["tide-stations"], settingsJSON: "{}")
        XCTAssertEqual(a, PinQuery(region: region, layers: ["tide-stations"], settingsJSON: "{}"))
        XCTAssertNotEqual(
            a, PinQuery(region: region, layers: ["tide-stations", "buoys"], settingsJSON: "{}"))
        var moved = region
        moved.west -= 0.01
        XCTAssertNotEqual(a, PinQuery(region: moved, layers: ["tide-stations"], settingsJSON: "{}"))
    }
}
