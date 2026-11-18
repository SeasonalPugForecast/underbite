import Foundation
import MapKit
import XCTest

@testable import UnderbiteKit

/// The breaks layers on the map: what is shown and whether their grid layer is drawn, what they
/// say in place of lines, the threshold over the core's stops and the lines placed for drawing.
@MainActor
final class BreaksTests: XCTestCase {
    private func settings(_ change: (inout Settings) -> Void = { _ in }) throws -> Settings {
        var settings = try Settings.defaults()
        change(&settings)
        return settings
    }

    private func show(_ ids: Set<String>, in settings: inout Settings) {
        for i in settings.map.layers.indices where ids.contains(settings.map.layers[i].id) {
            settings.map.layers[i].shown = true
        }
    }

    /// What the map shows for `settings`, as the module sees it.
    private func shown(_ settings: Settings, battery: Bool = false) throws -> BreaksShown {
        let json = try settings.json()
        let catalogue = try mapCatalogue(settingsJson: json)
        let plan = MapPlan.make(catalogue: catalogue, settings: settings.map, battery: battery)
        let words = try breaksWords(settingsJson: json)
        let sources = Dictionary(uniqueKeysWithValues: words.layers.map { ($0.id, $0.source) })
        return BreaksShown(plan.shown, drawn: plan.layers, sources: sources)
    }

    // MARK: - What is shown

    func testTheCataloguesBreaksLayersFollowTheGridLayerTheyAreFoundIn() throws {
        let json = try settings().json()
        let catalogue = try mapCatalogue(settingsJson: json)
        let breaks = catalogue.filter { $0.draw == .breaks }
        XCTAssertEqual(breaks.map(\.id), ["sst-breaks", "chlorophyll-breaks"])
        XCTAssertTrue(breaks.allSatisfy { $0.group == "water" && $0.notice != nil })
        XCTAssertTrue(breaks.allSatisfy { !$0.credit.isEmpty && $0.legend.isEmpty })
        XCTAssertEqual(breaks.map(\.minZoom), [7, 5])

        let words = try breaksWords(settingsJson: json)
        XCTAssertEqual(words.layers.map(\.source), ["sst", "chlorophyll"])

        let both = try shown(
            try settings { self.show(["sst", "sst-breaks", "chlorophyll-breaks"], in: &$0) })
        XCTAssertEqual(both.layers.map(\.id), ["sst-breaks", "chlorophyll-breaks"])
        let sst = try XCTUnwrap(both.layer("sst-breaks"))
        XCTAssertEqual(sst.source, "sst")
        XCTAssertTrue(sst.sourceDrawn)
        XCTAssertNotNil(sst.sourceKey)
        XCTAssertEqual(sst.minZoom, 7)
        let chlorophyll = try XCTUnwrap(both.layer("chlorophyll-breaks"))
        XCTAssertFalse(chlorophyll.sourceDrawn, "chlorophyll itself is off")
        XCTAssertNil(chlorophyll.sourceKey)
        XCTAssertEqual(both.findable.map(\.id), ["sst-breaks"])

        let none = try shown(try settings { self.show(["sst"], in: &$0) })
        XCTAssertFalse(none.any)
    }

    func testAGridLayerHeldBackByTheBatteryModeLeavesItsBreaksWithoutLines() throws {
        // The battery-friendly mode keeps the two topmost rasters: chlorophyll and clarity.
        let settings = try settings {
            self.show(["sst", "chlorophyll", "clarity", "sst-breaks"], in: &$0)
        }
        let full = try shown(settings)
        XCTAssertTrue(try XCTUnwrap(full.layer("sst-breaks")).sourceDrawn)
        let saving = try shown(settings, battery: true)
        XCTAssertFalse(try XCTUnwrap(saving.layer("sst-breaks")).sourceDrawn)
        XCTAssertTrue(saving.findable.isEmpty)
    }

    func testAnotherDayOfTheGridLayerChangesWhatTheBreaksAreFoundIn() throws {
        let settings = try settings { self.show(["sst", "sst-breaks"], in: &$0) }
        let json = try settings.json()
        let catalogue = try mapCatalogue(settingsJson: json)
        let words = try breaksWords(settingsJson: json)
        let sources = Dictionary(uniqueKeysWithValues: words.layers.map { ($0.id, $0.source) })
        let newest = MapPlan.make(catalogue: catalogue, settings: settings.map, battery: false)
        let older = MapPlan.make(
            catalogue: catalogue, settings: settings.map, battery: false,
            times: ["sst": 1_794_819_600])
        let a = BreaksShown(newest.shown, drawn: newest.layers, sources: sources)
        let b = BreaksShown(older.shown, drawn: older.layers, sources: sources)
        XCTAssertNotEqual(a.layer("sst-breaks")?.sourceKey, b.layer("sst-breaks")?.sourceKey)
        XCTAssertNotEqual(a, b, "the module looks again")
    }

    // MARK: - In place of lines

    func testWhatALayerSaysFollowsItsGridLayerTheZoomAndTheCore() throws {
        let words = try breaksWords(settingsJson: try settings().json())
        let layer = try XCTUnwrap(words.layers.first)
        var shown = BreaksShown.Layer(
            id: "sst-breaks", source: "sst", opacity: 0.9, minZoom: 7, sourceDrawn: true,
            sourceKey: "sst")
        let set = { (state: BreaksState, text: String?) in
            BreakSet(lines: [], state: state, stateText: text, fromText: nil, time: nil)
        }
        XCTAssertTrue(BreaksStatus.deepEnough(zoom: 6.8, minZoom: 7))
        XCTAssertFalse(BreaksStatus.deepEnough(zoom: 6.5, minZoom: 7))

        XCTAssertEqual(BreaksStatus.of(shown, zoom: 6, set: nil), .zoomIn)
        XCTAssertEqual(BreaksStatus.of(shown, zoom: 9, set: nil), .loading)
        XCTAssertNil(BreaksStatus.of(shown, zoom: 9, set: set(.drawn, nil)))
        XCTAssertEqual(BreaksStatus.of(shown, zoom: 9, set: set(.noBreaks, "x")), .noBreaks)
        XCTAssertEqual(BreaksStatus.of(shown, zoom: 9, set: set(.zoomIn, "x")), .zoomIn)
        XCTAssertEqual(BreaksStatus.of(shown, zoom: 9, set: set(.waiting, "x")), .loading)
        let cloudy = set(.fewClearCells, "Few clear cells on Nov 4.")
        XCTAssertEqual(BreaksStatus.of(shown, zoom: 9, set: cloudy), .fewClearCells)
        XCTAssertEqual(
            BreaksStatus.fewClearCells.text(words, layer: layer, set: cloudy),
            "Few clear cells on Nov 4.")
        shown.sourceDrawn = false
        XCTAssertEqual(BreaksStatus.of(shown, zoom: 9, set: nil), .needsSource)
        XCTAssertEqual(BreaksStatus.needsSource.text(words, layer: layer, set: nil), layer.needsSource)
        XCTAssertEqual(BreaksStatus.zoomIn.text(words, layer: layer, set: nil), words.zoomIn)
        XCTAssertEqual(BreaksStatus.loading.text(words, layer: layer, set: nil), words.loading)
    }

    func testTheStoreSaysWhatItHoldsForAView() throws {
        let json = try settings().json()
        let store = GridStore()
        let waiting = try store.breaks(
            layer: "sst-breaks", west: -73, south: 39.5, east: -72, north: 40.5, zoom: 9,
            settingsJson: json)
        XCTAssertEqual(waiting.state, .waiting)
        XCTAssertTrue(waiting.lines.isEmpty)
        let wide = try store.breaks(
            layer: "sst-breaks", west: -75, south: 37, east: -70, north: 42, zoom: 5.5,
            settingsJson: json)
        XCTAssertEqual(wide.state, .zoomIn)
        XCTAssertEqual(wide.stateText, try breaksWords(settingsJson: json).zoomIn)
        XCTAssertThrowsError(
            try store.breaks(
                layer: "sst", west: -73, south: 39.5, east: -72, north: 40.5, zoom: 9,
                settingsJson: json))
    }

    // MARK: - The threshold

    func testTheThresholdMovesOverTheCoresStopsAndIsStoredCanonical() throws {
        var settings = try settings()
        let json = try settings.json()
        let stops = BreaksThreshold.stops(layer: "sst-breaks", settingsJSON: json)
        XCTAssertGreaterThan(stops.count, 5)
        XCTAssertTrue(stops.allSatisfy { $0.label.hasSuffix(" °F per mi") }, "\(stops.map(\.label))")
        XCTAssertEqual(stops.map(\.value), stops.map(\.value).sorted())
        // The default, 0.5 °C per km, sits at the stop for 1.5 °F per mile.
        let value = try XCTUnwrap(BreaksThreshold.value(layer: "sst-breaks", in: settings.map))
        let at = try XCTUnwrap(BreaksThreshold.position(of: value, in: stops))
        XCTAssertEqual(stops[at].label, "1.5 °F per mi")

        BreaksThreshold.set(2, stops: stops, layer: "sst-breaks", in: &settings.map)
        XCTAssertEqual(settings.map.breaks.sstCPerKm, stops[2].value)
        let saved = try Settings(json: try settings.json())
        XCTAssertEqual(saved.map.breaks.sstCPerKm, stops[2].value, "the core accepts it")
        XCTAssertEqual(
            try breaksWords(settingsJson: try settings.json()).layers[0].thresholdText,
            stops[2].label)

        let chlorophyll = BreaksThreshold.stops(layer: "chlorophyll-breaks", settingsJSON: json)
        XCTAssertTrue(chlorophyll.allSatisfy { $0.label.hasPrefix("×") })
        BreaksThreshold.set(0, stops: chlorophyll, layer: "chlorophyll-breaks", in: &settings.map)
        XCTAssertEqual(settings.map.breaks.chlorophyllLog10PerKm, chlorophyll[0].value)

        let before = settings.map
        BreaksThreshold.set(99, stops: stops, layer: "sst-breaks", in: &settings.map)
        BreaksThreshold.set(0, stops: stops, layer: "sst", in: &settings.map)
        XCTAssertEqual(settings.map, before, "nothing outside the stops or the breaks layers")
        XCTAssertNil(BreaksThreshold.position(of: 1, in: []))
        XCTAssertTrue(BreaksThreshold.stops(layer: "sst", settingsJSON: json).isEmpty)
    }

    func testTheThresholdFollowsTheUnits() throws {
        let metric = try settings {
            $0.units.waterTemp = "C"
            $0.units.distance = "km"
        }
        let stops = BreaksThreshold.stops(layer: "sst-breaks", settingsJSON: try metric.json())
        XCTAssertTrue(stops.allSatisfy { $0.label.hasSuffix(" °C per km") }, "\(stops.map(\.label))")
        let half = try XCTUnwrap(stops.first { $0.label == "0.5 °C per km" })
        XCTAssertEqual(half.value, 0.5, accuracy: 1e-9)
    }

    // MARK: - The lines

    func testLinesArePlacedWeakestFirstAndWeightedByTheCoresEmphasis() {
        let point = { (lat: Double, lon: Double) in MapPoint(lat: lat, lon: lon) }
        let line = { (points: [MapPoint], emphasis: Double) in
            BreakLine(points: points, strengthText: "", value: 1, emphasis: emphasis)
        }
        let lines = BreakLines([
            line([point(40, -73), point(40.1, -73)], 0.9),
            line([point(40, -72)], 0.5),
            line([point(39.5, -72.5), point(39.6, -72.4), point(39.7, -72.4)], 0.1),
        ])
        XCTAssertEqual(lines.lines.map(\.emphasis), [0.1, 0.9], "a lone point is left out")
        XCTAssertEqual(lines.vertexCount, 5)
        XCTAssertGreaterThan(lines.lines[1].bounds.height, 0)
        XCTAssertEqual(lines.lines[1].bounds.width, 1, "an upright line is a map point wide")

        XCTAssertEqual(BreakLines.width(emphasis: 0), BreakLines.thinnest)
        XCTAssertEqual(BreakLines.width(emphasis: 1), BreakLines.heaviest)
        XCTAssertEqual(BreakLines.width(emphasis: 7), BreakLines.heaviest)
        XCTAssertLessThan(BreakLines.width(emphasis: 0.2), BreakLines.width(emphasis: 0.6))

        let north = MKMapRect(
            origin: MKMapPoint(CLLocationCoordinate2D(latitude: 40.2, longitude: -73.1)),
            size: MKMapSize(width: 1_000, height: 1_000))
        XCTAssertTrue(lines.lines(in: north, margin: 0).isEmpty)
        let around = lines.lines[1].bounds.insetBy(dx: -10, dy: -10)
        XCTAssertEqual(lines.lines(in: around, margin: 0).count, 1)
    }
}
