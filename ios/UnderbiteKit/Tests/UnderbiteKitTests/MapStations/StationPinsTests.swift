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
            label: "Tide station", observed: true, reference: true)
        let made = MapPin(station)
        XCTAssertTrue(made.observed)
        XCTAssertTrue(made.reference)
        let point = MKMapPoint(
            CLLocationCoordinate2D(latitude: battery.latitude, longitude: battery.longitude))
        XCTAssertEqual(made.x, point.x, accuracy: 1e-6)
        XCTAssertEqual(made.y, point.y, accuracy: 1e-6)
        XCTAssertEqual(made.kind, .tide)
        XCTAssertEqual(made.role, .none)
        XCTAssertEqual(made.label, "Tide station")
        XCTAssertNil(made.spotID)
    }

    func testStationMarksAreDimmedOnlyUnderALayerThatFillsAreas() {
        let area: Set = ["sst", "beach-forecast"]
        XCTAssertTrue(StationPins.dimsStations(shown: ["tide-stations", "sst"], areaFilling: area))
        XCTAssertFalse(StationPins.dimsStations(shown: ["tide-stations"], areaFilling: area))
        XCTAssertFalse(StationPins.dimsStations(shown: ["sst"], areaFilling: []))
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

    /// A pin at `points` screen points east and south of the middle of a 56-point cell at
    /// `perPoint` map points a point: pins built so are in one cell, or not, whatever the place.
    private func inCell(
        _ id: String, dx: Double, dy: Double = 0, perPoint: Double, kind: PinKind = .tide,
        layer: String = "tide-stations", observed: Bool = false, reference: Bool = false
    ) -> MapPin {
        let cell = StationPins.cellPoints * perPoint
        let home = MKMapPoint(
            CLLocationCoordinate2D(latitude: battery.latitude, longitude: battery.longitude))
        let centre = MKMapPoint(
            x: ((home.x / cell).rounded(.down) + 0.5) * cell,
            y: ((home.y / cell).rounded(.down) + 0.5) * cell)
        let at = MKMapPoint(x: centre.x + dx * perPoint, y: centre.y + dy * perPoint).coordinate
        return MapPin(
            id: id, kind: kind, layer: layer, name: id, label: "", role: .none,
            latitude: at.latitude, longitude: at.longitude, observed: observed,
            reference: reference)
    }

    func testAnObservingStationLeadsAGaugeOrPredictionNextToIt() {
        let perPoint = StationPins.mapPointsPerPoint(zoom: 9)
        let predicted = inCell("coops:1", dx: 0, perPoint: perPoint, reference: true)
        let gauge = inCell(
            "usgs:USGS-1", dx: 8, perPoint: perPoint, kind: .riverGauge, layer: "usgs-gauges",
            observed: true)
        let sub = inCell("coops:2", dx: -3, perPoint: perPoint)
        let grouped = StationPins.clusters([predicted, sub, gauge], mapPointsPerPoint: perPoint)
        // Observations first, then a harmonic station, then the nearest the middle.
        XCTAssertEqual(grouped.count, 1)
        XCTAssertEqual(grouped[0].pins.map(\.id), ["usgs:USGS-1", "coops:1", "coops:2"])
        let without = StationPins.clusters([sub, predicted], mapPointsPerPoint: perPoint)
        XCTAssertEqual(without[0].pins.map(\.id), ["coops:1", "coops:2"])
        let alone = StationPins.clusters([sub, inCell("coops:3", dx: 2, perPoint: perPoint)],
            mapPointsPerPoint: perPoint)
        XCTAssertEqual(alone[0].pins.map(\.id), ["coops:3", "coops:2"])
    }

    func testPinsInOneCellShareAMarkUntilZoomedIn() {
        let perPoint = StationPins.mapPointsPerPoint(zoom: 10)
        let first = inCell("a", dx: 0, perPoint: perPoint)
        let near = inCell("b", dx: 12, perPoint: perPoint)
        let far = inCell("c", dx: 80, perPoint: perPoint)

        let grouped = StationPins.clusters([first, near, far], mapPointsPerPoint: perPoint)
        XCTAssertEqual(grouped.map { $0.pins.map(\.id) }, [["a", "b"], ["c"]])
        // The mark stands at its lead pin.
        XCTAssertEqual(grouped[0].x, first.x, accuracy: 1e-6)
        XCTAssertNil(grouped[0].single)
        XCTAssertEqual(grouped[1].single?.id, "c")

        // Sixteen times closer in, 12 points become 192: three cells apart.
        let apart = StationPins.clusters(
            [first, near, far], mapPointsPerPoint: perPoint / 16)
        XCTAssertEqual(apart.count, 3)

        // The same pins at the same zoom group the same way, in any order.
        XCTAssertEqual(
            StationPins.clusters([far, near, first], mapPointsPerPoint: perPoint), grouped)
    }

    func testAZoomedOutHarbourHasFewMarksAndTheSpotsSurvive() {
        let perPoint = StationPins.mapPointsPerPoint(zoom: 10)
        var pins: [MapPin] = []
        for i in 0..<20 {
            for j in 0..<20 {
                pins.append(
                    pin(
                        "coops:\(i)-\(j)", 40.5 + Double(i) * 0.01, -74.2 + Double(j) * 0.012))
            }
        }
        let spots = [
            MapPin.spot(id: "s1", name: "Pier", latitude: 40.61, longitude: -74.09),
            MapPin.spot(id: "s2", name: "Jetty", latitude: 40.69, longitude: -74.01),
        ]
        let marks = StationPins.clusters(spots + pins, mapPointsPerPoint: perPoint)
        // A view 390 by 844 points holds at most 7 by 16 cells.
        XCTAssertLessThan(marks.count, 80)
        XCTAssertEqual(marks.flatMap(\.pins).count, pins.count + spots.count)
        for spot in spots {
            XCTAssertTrue(marks.contains { $0.pins.first?.id == spot.id }, spot.id)
        }
    }

    func testAStationAtASpotIsAlwaysAMarkOfItsOwn() {
        let perPoint = StationPins.mapPointsPerPoint(zoom: 10)
        let station = inCell("coops:1", dx: 0, perPoint: perPoint)
        let spot = MapPin.spot(
            id: "s1", name: "Pier", latitude: station.latitude, longitude: station.longitude)
        let observed = inCell("coops:2", dx: 4, perPoint: perPoint, observed: true)
        let marks = StationPins.clusters([spot, station, observed], mapPointsPerPoint: perPoint)
        // The spot and the station at it are both always shown; the other folds into the nearer.
        XCTAssertEqual(marks.count, 2)
        XCTAssertEqual(Set(marks.compactMap { $0.pins.first?.id }), ["spot:s1", "coops:1"])
    }

    func testTwoSpotsInACellEachTakeTheNearestPin() {
        let perPoint = 100.0
        let one = inCell("spot-left", dx: -20, perPoint: perPoint)
        let two = inCell("spot-right", dx: 20, perPoint: perPoint)
        let between = inCell("coops:x", dx: 14, perPoint: perPoint)
        let spotOne = MapPin.spot(id: "left", name: "L", latitude: one.latitude, longitude: one.longitude)
        let spotTwo = MapPin.spot(id: "right", name: "R", latitude: two.latitude, longitude: two.longitude)
        let marks = StationPins.clusters([spotOne, spotTwo, between], mapPointsPerPoint: perPoint)
        XCTAssertEqual(
            Set(marks.map { $0.pins.map(\.id) }), [["spot:left"], ["spot:right", "coops:x"]])
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

    func testTheListOfPinsOnScreenHoldsSpotsFirstThenTheNearestTheMiddle() {
        let perPoint = StationPins.mapPointsPerPoint(zoom: 11)
        let region = MapRegion(west: -74.1, south: 40.6, east: -73.9, north: 40.8, zoom: 11)
        let middle = pin("coops:middle", 40.7, -74.0)
        let edge = pin("coops:edge", 40.79, -73.91, kind: .buoy, layer: "buoys")
        let outside = pin("coops:outside", 41.5, -74.0)
        let spot = MapPin.spot(id: "s1", name: "Pier", latitude: 40.62, longitude: -74.08)
        let marks = StationPins.clusters(
            [spot, middle, edge, outside], mapPointsPerPoint: perPoint)
        let listed = StationPins.onScreen(marks, in: region)
        XCTAssertEqual(listed.map(\.id), ["spot:s1", "coops:middle", "coops:edge"])
        XCTAssertEqual(StationPins.onScreen(marks, in: region, limit: 2).count, 2)

        // A pin folded into another mark is listed too, and the active spot (not drawn) is not.
        let folded = StationPins.clusters(
            [spot, middle, edge, pin("coops:next", 40.7001, -74.0001)],
            mapPointsPerPoint: perPoint, apart: spot.id)
        XCTAssertTrue(
            StationPins.onScreen(folded, in: region).map(\.id).contains("coops:next"))
        XCTAssertFalse(StationPins.onScreen(folded, in: region).map(\.id).contains("spot:s1"))
    }

    func testAScreenReaderHearsTheNameThenTheCoresWords() {
        let station = MapPin(
            id: "coops:8518750", kind: .tide, layer: "tide-stations", name: "The Battery",
            label: "Tide station", role: .none, latitude: battery.latitude,
            longitude: battery.longitude)
        XCTAssertEqual(StationPins.spoken(station), "The Battery, Tide station")
        let spot = MapPin.spot(id: "s1", name: "Pier", latitude: 40.7, longitude: -74.0)
        XCTAssertEqual(StationPins.spoken(spot), "Pier")
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
