import Foundation
import XCTest

@testable import UnderbiteKit

/// A colour range set in the drawer: picked from the core's round values and drawn over the
/// range fitted to the view.
final class LayerRangeTests: XCTestCase {
    private func settings(fahrenheit: Bool = false) throws -> Settings {
        var settings = try Settings.defaults()
        for i in settings.map.layers.indices {
            settings.map.layers[i].shown = settings.map.layers[i].id == "sst"
        }
        settings.units.waterTemp = fahrenheit ? "F" : "C"
        return settings
    }

    func testTheEndsAreTheCoresRoundValuesWithinTheLayersOwnRange() throws {
        let settings = try settings()
        let own = try XCTUnwrap(LayerRange.ownRange(of: "sst", settings: settings))
        XCTAssertLessThan(own.low, own.high)
        let choices = LayerRange.choices(for: "sst", settings: settings)
        XCTAssertGreaterThan(choices.count, 2)
        XCTAssertTrue(choices.allSatisfy { $0.value >= own.low - 1e-9 && $0.value <= own.high + 1e-9 })
        XCTAssertTrue(choices.allSatisfy { !$0.label.isEmpty })
        // In the user's unit, as the core writes it.
        let fahrenheit = LayerRange.choices(for: "sst", settings: try self.settings(fahrenheit: true))
        XCTAssertNotEqual(fahrenheit.map(\.label), choices.map(\.label))
        // Layers whose range is not set here offer nothing.
        XCTAssertEqual(LayerRange.choices(for: "tide-stations", settings: settings), [])
        XCTAssertEqual(LayerRange.choices(for: "true-color-viirs-n20", settings: settings), [])
    }

    func testTheRangeSetHereWinsOverTheFittedOne() throws {
        var settings = try settings()
        let own = try XCTUnwrap(LayerRange.ownRange(of: "sst", settings: settings))
        let choices = LayerRange.choices(for: "sst", settings: settings)
        let low = choices[1].value
        let high = choices[choices.count - 2].value
        let fitted = ValueRange(low: low + 0.5, high: high - 0.5)
        // Without a range of the user's, the fitted one is drawn and the legend says so.
        let before = try settings.json()
        assertSame(try mapRange(settingsJson: before, layer: "sst", fitted: fitted), fitted)
        XCTAssertNotNil(try mapFittedCaption(settingsJson: before, layer: "sst", fitted: fitted))

        XCTAssertTrue(LayerRange.set("sst", low: low, own: own, in: &settings.map))
        XCTAssertEqual(LayerRange.userRange(of: "sst", in: settings.map)?.low, low)
        XCTAssertEqual(LayerRange.userRange(of: "sst", in: settings.map)?.high, own.high)
        XCTAssertTrue(LayerRange.set("sst", high: high, own: own, in: &settings.map))
        let after = try settings.json()
        XCTAssertEqual(validateSettings(json: after).filter { $0.severity == .error }, [])
        let drawn = try mapRange(settingsJson: after, layer: "sst", fitted: fitted)
        XCTAssertEqual(drawn.low, low, accuracy: 1e-6)
        XCTAssertEqual(drawn.high, high, accuracy: 1e-6)
        XCTAssertNil(try mapFittedCaption(settingsJson: after, layer: "sst", fitted: fitted))
        // The catalogue's legend spans it too, and the own range is still the layer's.
        let sst = try XCTUnwrap(try mapCatalogue(settingsJson: after).first { $0.id == "sst" })
        let legend = try mapLegend(settingsJson: after, layer: "sst", range: fitted)
        XCTAssertEqual(sst.legend.first?.label, legend.first?.label)
        XCTAssertEqual(sst.legend.last?.label, legend.last?.label)
        XCTAssertEqual(LayerRange.ownRange(of: "sst", settings: settings), own)

        // Fitting again hands the colours back to the view.
        LayerRange.clear("sst", in: &settings.map)
        XCTAssertNil(LayerRange.userRange(of: "sst", in: settings.map))
        assertSame(try mapRange(settingsJson: try settings.json(), layer: "sst", fitted: fitted), fitted)
    }

    private func assertSame(
        _ a: ValueRange, _ b: ValueRange, file: StaticString = #filePath, line: UInt = #line
    ) {
        XCTAssertEqual(a.low, b.low, accuracy: 1e-6, file: file, line: line)
        XCTAssertEqual(a.high, b.high, accuracy: 1e-6, file: file, line: line)
    }

    func testTheLowEndStaysUnderTheHighOne() throws {
        var settings = try settings()
        let own = try XCTUnwrap(LayerRange.ownRange(of: "sst", settings: settings))
        XCTAssertTrue(LayerRange.set("sst", low: own.low + 2, high: own.low + 4, own: own, in: &settings.map))
        let before = settings.map
        XCTAssertFalse(LayerRange.set("sst", low: own.low + 5, own: own, in: &settings.map))
        XCTAssertFalse(LayerRange.set("sst", high: own.low + 1, own: own, in: &settings.map))
        XCTAssertFalse(LayerRange.set("no-such-layer", low: 1, own: own, in: &settings.map))
        XCTAssertEqual(settings.map, before)
    }
}
