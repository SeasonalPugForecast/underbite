import CoreLocation
import Foundation
import XCTest

@testable import UnderbiteKit

/// The point inspector's rows, the nearest station, the measured line and the launch points.
final class PointInspectionTests: XCTestCase {
    /// Off Sandy Hook, in the water.
    private let offSandyHook = CLLocationCoordinate2D(latitude: 40.45, longitude: -73.93)

    private func catalogue() throws -> [LayerInfo] {
        try mapCatalogue(settingsJson: defaultSettingsJson())
    }

    /// The catalogue's layers `ids`, bottom first, as the plan lists what is on screen.
    private func shown(_ ids: [String]) throws -> [LayerInfo] {
        let all = try catalogue()
        return try ids.map { id in try XCTUnwrap(all.first { $0.id == id }, id) }
    }

    private func row(_ layer: String, _ value: String) -> InspectorRow {
        InspectorRow(
            layer: layer, label: "title of \(layer)", valueText: value, source: "credit of \(layer)",
            timeText: "Nov 7")
    }

    // MARK: - Rows

    func testTheCoreIsAskedForEveryOverlayButPicturesTopmostFirst() throws {
        let layers = try shown(["noaa-chart", "sst-imagery", "sst", "hf-radar-currents", "tide-stations"])
        XCTAssertEqual(
            PointInspection.valueLayers(shown: layers), ["tide-stations", "hf-radar-currents", "sst"])
    }

    func testRowsFollowTheLayersTopmostFirstWithPicturesAsImageryOnly() throws {
        let layers = try shown(["apple-standard", "sst-imagery", "sst", "hf-radar-currents", "tide-stations"])
        let words = try pointWords(
            point: MapPoint(lat: offSandyHook.latitude, lon: offSandyHook.longitude),
            settingsJson: defaultSettingsJson())
        // The core's rows come in any order; currents have no block over the point yet.
        let rows = PointInspection.rows(
            shown: layers, values: [row("sst", "61 °F")], imageryOnly: words.imageryOnly)
        XCTAssertEqual(rows.map(\.layer), ["sst", "sst-imagery"])
        XCTAssertEqual(rows[0], PointRow(row("sst", "61 °F")))
        XCTAssertEqual(rows[0].value, "61 °F")
        XCTAssertEqual(rows[0].source, "credit of sst")
        // The picture layer: its title and credit from the catalogue, the core's words.
        let imagery = try XCTUnwrap(layers.first { $0.id == "sst-imagery" })
        XCTAssertEqual(rows[1].label, imagery.title)
        XCTAssertEqual(rows[1].source, imagery.credit)
        XCTAssertEqual(rows[1].value, words.imageryOnly)
        XCTAssertFalse(words.imageryOnly.isEmpty)
        XCTAssertEqual(rows[1].time, "")
    }

    func testBaseMapsHaveNoRowEvenTheChart() throws {
        let layers = try shown(["noaa-chart", "sst"])
        let rows = PointInspection.rows(shown: layers, values: [row("sst", "16 °C")], imageryOnly: "x")
        XCTAssertEqual(rows.map(\.layer), ["sst"])
    }

    func testAnEmptyStoreGivesNoValuesAndNothingIsFetched() throws {
        // A tap reads the grids held for the view only: a store without blocks has no rows.
        let rows = try GridStore().inspect(
            lat: offSandyHook.latitude, lon: offSandyHook.longitude, layers: ["sst", "hf-radar-currents"],
            settingsJson: defaultSettingsJson(), timeZone: "America/New_York")
        XCTAssertEqual(rows, [])
    }

    func testThePointIsWordedByTheCore() throws {
        let words = try pointWords(
            point: MapPoint(lat: 40.7033, lon: -74.0170), settingsJson: defaultSettingsJson())
        XCTAssertTrue(words.point.contains("40.7033"), words.point)
        XCTAssertTrue(words.point.contains("74.0170"), words.point)
    }

    // MARK: - The nearest station

    private func pin(_ id: String, _ latitude: Double, _ longitude: Double, kind: PinKind = .tide) -> MapPin {
        MapPin(
            id: id, kind: kind, layer: kind == .spot ? StationPins.spotsLayer : "tide-stations", name: id,
            label: "", role: .none, latitude: latitude, longitude: longitude,
            spotID: kind == .spot ? id : nil)
    }

    func testTheNearestStationIsTheClosestOnTheGround() {
        let sandyHook = pin("coops:8531680", 40.4669, -74.0094)
        let battery = pin("coops:8518750", 40.7006, -74.0142)
        let buoy = pin("ndbc:44065", 40.369, -73.703, kind: .buoy)
        XCTAssertEqual(
            PointInspection.nearestStation([battery, buoy, sandyHook], to: offSandyHook)?.id,
            sandyHook.id)
    }

    func testASavedSpotIsNotAStation() {
        let spot = pin("seed-spot", 40.451, -73.931, kind: .spot)
        let battery = pin("coops:8518750", 40.7006, -74.0142)
        XCTAssertEqual(PointInspection.nearestStation([spot, battery], to: offSandyHook)?.id, battery.id)
        XCTAssertNil(PointInspection.nearestStation([spot], to: offSandyHook))
        XCTAssertNil(PointInspection.nearestStation([], to: offSandyHook))
    }

    // MARK: - Measuring

    func testALineIsMeasuredOnceItHasAnEnd() throws {
        let json = defaultSettingsJson()
        var line = MeasureLine(from: .point(CLLocationCoordinate2D(latitude: 40.7033, longitude: -74.0170)))
        XCTAssertNil(line.text(settingsJSON: json))
        XCTAssertEqual(line.coordinates.count, 0)
        line.tap(offSandyHook)
        XCTAssertEqual(line.to, .point(offSandyHook))
        XCTAssertEqual(line.coordinates.count, 2)
        let text = try XCTUnwrap(line.text(settingsJSON: json))
        let core = try UnderbiteKit.measure(
            from: MapPoint(lat: 40.7033, lon: -74.0170),
            to: MapPoint(lat: offSandyHook.latitude, lon: offSandyHook.longitude), settingsJson: json)
        XCTAssertEqual(text, core)
        XCTAssertFalse(text.distance.isEmpty)
        XCTAssertTrue(text.line.contains(text.distance))
        XCTAssertTrue(text.line.contains(text.bearing))
        // Every measurement carries the core's whole "not for navigation" note.
        XCTAssertEqual(text.notice, try appWords(settingsJson: json).mapNotice)
    }

    func testATapMovesTheEndAndKeepsTheStart() {
        let spot = MeasureLine.End(kind: .spot("seed-battery"), latitude: 40.7033, longitude: -74.0170)
        var line = MeasureLine(from: spot)
        line.tap(offSandyHook)
        let further = CLLocationCoordinate2D(latitude: 40.3, longitude: -73.8)
        line.tap(further)
        XCTAssertEqual(line.from, spot)
        XCTAssertEqual(line.to, .point(further))
        XCTAssertEqual(line.coordinates.last?.latitude, 40.3)
    }

    // MARK: - Launch points

    func testLaunchPointsAreReadWhole() throws {
        let one = try XCTUnwrap(PointInspection.points("40.45,-73.93", count: 1))
        XCTAssertEqual(one.count, 1)
        XCTAssertEqual(one[0].latitude, 40.45)
        XCTAssertEqual(one[0].longitude, -73.93)
        let two = try XCTUnwrap(PointInspection.points("40.7033,-74.0170,40.45,-73.93", count: 2))
        XCTAssertEqual(two.map(\.latitude), [40.7033, 40.45])
        XCTAssertNil(PointInspection.points("40.45", count: 1))
        XCTAssertNil(PointInspection.points("40.45,-73.93", count: 2))
        XCTAssertNil(PointInspection.points("40.45,west", count: 1))
        XCTAssertNil(PointInspection.points("95,-73.93", count: 1))
    }
}

