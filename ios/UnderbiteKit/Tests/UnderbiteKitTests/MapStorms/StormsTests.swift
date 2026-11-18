import Foundation
import MapKit
import XCTest

@testable import UnderbiteKit

/// The storms layer on the map: the catalogue's entry, the core's layer for the built-in samples,
/// how its tracks, cone and marks are placed for drawing (the short way across the antimeridian),
/// which point marks a step, which forecast points get a day label and which tap hits a hurricane.
final class StormsTests: XCTestCase {
    private let black = MapColour(red: 0, green: 0, blue: 0, alpha: 255)

    private func settingsJSON() throws -> String { try Settings.defaults().json() }

    /// A sample outlook the app's debug builds draw, written by the core's tests.
    private func sample(_ name: String) throws -> (json: String, now: Int64) {
        let url = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .appendingPathComponent("../../../../Underbite/Underbite/Map/Storms")
            .appendingPathComponent("storms-sample-\(name).json")
        let json = try String(contentsOf: url, encoding: .utf8)
        let object = try XCTUnwrap(
            JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any])
        let now = try XCTUnwrap(object["fetched_at"] as? NSNumber).int64Value
        return (json, now)
    }

    private func layer(_ name: String) throws -> StormLayer {
        let (json, now) = try sample(name)
        return try stormLayerOf(
            outlookJson: json, settingsJson: try settingsJSON(), timeZone: "America/New_York",
            now: now)
    }

    private func point(_ latitude: Double, _ longitude: Double, _ time: Int64) -> StormTrackPoint {
        StormTrackPoint(
            latitude: latitude, longitude: longitude, time: time, label: "L", spoken: "L")
    }

    // MARK: - The catalogue and the core's words

    func testTheCatalogueListsTheStormsLayerInTheWeatherGroup() throws {
        let catalogue = try mapCatalogue(settingsJson: try settingsJSON())
        let storms = catalogue.filter { $0.draw == .storms }
        XCTAssertEqual(storms.map(\.id), ["storms"])
        let layer = try XCTUnwrap(storms.first)
        XCTAssertEqual(layer.group, "weather")
        XCTAssertFalse(layer.shown, "off until switched on")
        XCTAssertNotNil(layer.notice)
        XCTAssertTrue(layer.credit.contains("National Hurricane Center"))
        XCTAssertTrue(layer.credit.contains("Open-Meteo.com"))
        XCTAssertTrue(layer.legendCaption?.contains("NHC's forecast cone") ?? false)
        XCTAssertEqual(layer.dates, .undated)
        // The map view draws nothing of it itself.
        XCTAssertNil(PlannedLayer.Kind(layer.draw))
        XCTAssertFalse(MapPlan.isRaster(layer.draw))

        let words = try stormLayerWords(settingsJson: try settingsJSON())
        XCTAssertEqual(words.notice, "Not for navigation.")
        XCTAssertFalse(words.loading.isEmpty)
        XCTAssertTrue(words.safety.contains("National Hurricane Center"))
    }

    func testTheNoreasterSampleHasEachModelsLowsAndTheStormMarked() throws {
        let layer = try layer("noreaster")
        XCTAssertEqual(layer.models.map(\.short), ["ECMWF", "GFS", "GEM", "ICON"])
        XCTAssertEqual(layer.tracks.filter(\.isStorm).count, 4)
        XCTAssertTrue(layer.hurricanes.isEmpty, "hurricanes are absent when none are active")
        XCTAssertNil(layer.coneNote)
        XCTAssertTrue(layer.summary?.hasPrefix("A storm passes near the map's centre") ?? false)
        XCTAssertTrue(layer.steps.first?.label.hasSuffix("now") ?? false)

        let drawing = StormDrawing.make(layer, step: 0)
        XCTAssertEqual(drawing.shapes.count, layer.tracks.count)
        // The storm's bands go on top of the other lows', wider.
        let widths = drawing.shapes.map { shape -> Double in
            if case .band(let width, _) = shape.style { return width }
            return 0
        }
        XCTAssertEqual(Array(widths.suffix(4)), Array(repeating: 12, count: 4))
        XCTAssertTrue(widths.dropLast(4).allSatisfy { $0 == 6 })
        // Two days on, each model's lows are marked with their pressure; a step later they move.
        let lows = { (step: Int) in
            StormDrawing.make(layer, step: step).marks.filter {
                if case .low = $0.kind { return true }
                return false
            }
        }
        XCTAssertGreaterThanOrEqual(lows(8).count, 4)
        XCTAssertTrue(lows(8).allSatisfy { $0.text.hasSuffix(" inHg") })
        XCTAssertNotEqual(lows(9), lows(8))
        XCTAssertEqual(StormDrawing.make(layer, step: 8).shapes, drawing.shapes)
    }

    func testJosephineHasHerConeUnderHerTracksAndHerSymbolOnTop() throws {
        let layer = try layer("josephine")
        let h = try XCTUnwrap(layer.hurricanes.first)
        XCTAssertEqual(h.id, "al102026")
        XCTAssertTrue(h.title.contains("Josephine"))
        XCTAssertNotNil(h.advisoryUrl)
        XCTAssertTrue(layer.coneNote?.contains("not where its impacts reach") ?? false)
        XCTAssertEqual(layer.legend.last?.kind, .cone)

        let drawing = StormDrawing.make(layer, step: 0)
        XCTAssertEqual(drawing.shapes.first?.style, .cone)
        if case .line? = drawing.shapes.last?.style {} else {
            XCTFail("NHC's official track is drawn last")
        }
        XCTAssertTrue(drawing.shapes.contains { $0.style == .dotted(width: 2.5, dash: [0.01, 5]) })
        XCTAssertEqual(drawing.marks.last?.kind, .hurricane)
        XCTAssertEqual(drawing.marks.last?.text, h.title)
        let days = drawing.marks.filter { $0.kind == .day }
        XCTAssertEqual(days.count, StormMapRules.dayMarks(h.official).count)
        XCTAssertGreaterThan(days.count, 1)
    }

    // MARK: - Rules

    func testEachModelHasItsOwnDash() {
        let roles = ["gfs", "gem", "icon", "ukmet", "hafs", "consensus", "other"]
        XCTAssertEqual(StormMapRules.dash("ecmwf"), [])
        XCTAssertEqual(StormMapRules.dash("official"), [])
        let dashes = roles.map(StormMapRules.dash)
        XCTAssertTrue(dashes.allSatisfy { !$0.isEmpty })
        XCTAssertEqual(Set(dashes.map { $0.description }).count, roles.count)
    }

    func testAStepMarksTheNearestPointWithinThreeHours() {
        let track = StormTrack(
            model: "ecmwf_ifs025", short: "ECMWF", role: "ecmwf", colour: black, isStorm: true,
            points: [point(40, -74, 0), point(41, -72, 21_600), point(42, -70, 43_200)])
        XCTAssertEqual(StormMapRules.point(of: track, at: 21_600)?.time, 21_600)
        XCTAssertEqual(StormMapRules.point(of: track, at: 10_800)?.time, 0, "the earlier on a tie")
        XCTAssertEqual(StormMapRules.point(of: track, at: 54_000)?.time, 43_200)
        XCTAssertNil(StormMapRules.point(of: track, at: 60_000))
        XCTAssertEqual(StormMapRules.startStep(12, count: 5), 4)
        XCTAssertEqual(StormMapRules.startStep(nil, count: 5), 0)
        XCTAssertEqual(StormMapRules.startStep(3, count: 0), 0)
    }

    func testOneDayLabelADayAfterTheStormsPosition() {
        let hour: Int64 = 3_600
        let official = [0, 12, 24, 36, 48, 72, 96, 120].map { h in
            StormForecastPoint(
                latitude: 30, longitude: -60, time: Int64(h) * hour, label: "\(h)", wind: "",
                spoken: "")
        }
        XCTAssertEqual(StormMapRules.dayMarks(official).map(\.label), ["24", "48", "72", "96", "120"])
        XCTAssertEqual(StormMapRules.dayMarks([]), [])
    }

    func testTracksAcrossTheAntimeridianGoTheShortWay() {
        let world = MKMapSize.world.width
        let points = StormMapRules.mapPoints([
            (latitude: 20, longitude: 178), (latitude: 21, longitude: -179),
            (latitude: 22, longitude: -176),
        ])
        XCTAssertEqual(points.count, 3)
        // Each step is a few degrees east, not most of the world west.
        XCTAssertLessThan(points[1].x - points[0].x, world / 50)
        XCTAssertGreaterThan(points[1].x, points[0].x)
        XCTAssertGreaterThan(points[2].x, world, "past the world's edge, drawn again one world over")

        let frame = StormMapRules.frame(
            [(latitude: 10, longitude: 179), (latitude: 12, longitude: -179)], around: 179)
        XCTAssertLessThan(frame.width, world / 100)
        XCTAssertEqual(StormMapRules.longitude(-179, near: 179), 181)
        XCTAssertEqual(StormMapRules.continuous([179, -179, -170]), [179, 181, 190])
    }

    func testATapNearAHurricaneSymbolOpensIt() throws {
        let layer = try layer("josephine")
        let h = try XCTUnwrap(layer.hurricanes.first)
        let at = MKMapPoint(CLLocationCoordinate2D(latitude: h.latitude, longitude: h.longitude))
        let perPoint = 2_000.0
        let near = MKMapPoint(x: at.x + 20 * perPoint, y: at.y)
        let far = MKMapPoint(x: at.x + 40 * perPoint, y: at.y)
        XCTAssertEqual(
            StormMapRules.hurricane(at: near, in: layer.hurricanes, mapPointsPerPoint: perPoint)?
                .id, h.id)
        XCTAssertNil(
            StormMapRules.hurricane(at: far, in: layer.hurricanes, mapPointsPerPoint: perPoint))
        // On the next copy of the world too.
        let wrapped = MKMapPoint(x: near.x + MKMapSize.world.width, y: near.y)
        XCTAssertNotNil(
            StormMapRules.hurricane(at: wrapped, in: layer.hurricanes, mapPointsPerPoint: perPoint))
    }
}
