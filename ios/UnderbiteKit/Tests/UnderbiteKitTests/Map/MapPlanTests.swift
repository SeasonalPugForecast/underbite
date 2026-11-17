import Foundation
import XCTest

@testable import UnderbiteKit

/// What the map draws and in which order, from the core's catalogue for the settings.
final class MapPlanTests: XCTestCase {
    /// The default settings with `shown` switched on, in the default order unless `order` (bottom
    /// first) moves them to the top, and the base `base`.
    private func settings(
        shown: [String], order: [String] = [], base: String = "apple-standard",
        batteryMode: String = "off"
    ) throws -> Settings {
        var settings = try Settings.defaults()
        settings.map.base = base
        settings.map.batteryMode = batteryMode
        for i in settings.map.layers.indices {
            settings.map.layers[i].shown = shown.contains(settings.map.layers[i].id)
        }
        let moved = order.compactMap { id in settings.map.layers.first { $0.id == id } }
        settings.map.layers = settings.map.layers.filter { !order.contains($0.id) } + moved
        return settings
    }

    private func plan(_ settings: Settings, battery: Bool = false) throws -> MapPlan {
        let catalogue = try mapCatalogue(settingsJson: try settings.json())
        return MapPlan.make(catalogue: catalogue, settings: settings.map, battery: battery)
    }

    func testLayersAreDrawnInTheSettingsOrderBottomFirst() throws {
        let shown = ["sst", "true-color-viirs-n20", "hf-radar-currents", "tide-stations"]
        let first = try plan(
            settings(shown: shown, order: ["true-color-viirs-n20", "sst", "hf-radar-currents"]))
        XCTAssertEqual(first.layers.map(\.id), ["true-color-viirs-n20", "sst", "hf-radar-currents"])
        XCTAssertEqual(first.layers.map(\.kind), [.tiles, .gridRaster, .gridArrows])
        // Stations are drawn by their module, but they are on screen and credited.
        XCTAssertTrue(first.shown.contains { $0.id == "tide-stations" })

        let moved = try plan(
            settings(shown: shown, order: ["true-color-viirs-n20", "hf-radar-currents", "sst"]))
        XCTAssertEqual(moved.layers.map(\.id), ["true-color-viirs-n20", "hf-radar-currents", "sst"])
    }

    func testTheChartBaseIsDrawnFirstAndEverythingAboveItsLabels() throws {
        let plan = try plan(settings(shown: ["sst"], base: "noaa-chart"))
        XCTAssertEqual(plan.base?.id, "noaa-chart")
        XCTAssertEqual(plan.layers.map(\.id), ["noaa-chart", "sst"])
        XCTAssertTrue(plan.layers.allSatisfy(\.aboveLabels))
        XCTAssertEqual(plan.notice, "Not for navigation.")
        let chart = try XCTUnwrap(plan.base)
        XCTAssertEqual(plan.credits.first, chart.credit)
        XCTAssertTrue(chart.credit.contains("NOAA"), chart.credit)
    }

    func testAppleBasesDrawNothingThemselvesAndSitUnderTheLabels() throws {
        let plan = try plan(settings(shown: ["sst"], base: "apple-satellite"))
        XCTAssertEqual(plan.base?.draw, .appleSatellite)
        XCTAssertEqual(plan.layers.map(\.id), ["sst"])
        XCTAssertFalse(plan.layers[0].aboveLabels)
        // MUR SST is no chart, current or depth: only the map's own notice shows.
        XCTAssertNil(plan.notice)
        XCTAssertEqual(plan.legend?.id, "sst")
        XCTAssertEqual(plan.credits.count, 2)
    }

    func testCurrentsCallForTheBanner() throws {
        let plan = try plan(settings(shown: ["hf-radar-currents"]))
        XCTAssertEqual(plan.notice, "Not for navigation.")
        // Arrows have no legend bar of their own here.
        XCTAssertNil(plan.legend)
    }

    func testEveryCreditShowsOnceInDrawingOrder() throws {
        let plan = try plan(
            settings(
                shown: ["sst", "sst-imagery", "true-color-viirs-n20"],
                order: ["sst", "sst-imagery", "true-color-viirs-n20"]))
        let expected = [plan.base?.credit] + plan.layers.map(\.info.credit)
        XCTAssertEqual(plan.credits, expected.compactMap { $0 })
        XCTAssertEqual(Set(plan.credits).count, plan.credits.count)
    }

    func testTheBatteryModeKeepsTheTopTwoRastersAndTheArrows() throws {
        let order = [
            "sst", "chlorophyll", "hf-radar-currents", "true-color-viirs-n20", "sst-imagery",
        ]
        let plan = try plan(settings(shown: order, order: order), battery: true)
        XCTAssertEqual(plan.layers.map(\.id), ["hf-radar-currents", "true-color-viirs-n20", "sst-imagery"])
        XCTAssertEqual(plan.held, ["sst", "chlorophyll"])
        XCTAssertFalse(plan.shown.contains { $0.id == "sst" })
        // What is held back is not credited either.
        XCTAssertFalse(plan.credits.contains(try XCTUnwrap(
            mapCatalogue(settingsJson: try Settings.defaults().json()).first { $0.id == "sst" }).credit))

        let off = try self.plan(settings(shown: order, order: order), battery: false)
        XCTAssertEqual(off.layers.count, 5)
        XCTAssertEqual(off.held, [])
    }

    func testTheChartBaseIsNotCountedAgainstTheBattery() throws {
        let plan = try plan(
            settings(shown: ["sst", "chlorophyll"], order: ["sst", "chlorophyll"], base: "noaa-chart"),
            battery: true)
        XCTAssertEqual(plan.layers.map(\.id), ["noaa-chart", "sst", "chlorophyll"])
    }

    func testTheBatteryModeFollowsLowPowerOnAuto() {
        XCTAssertTrue(MapPlan.batteryOn(setting: "auto", lowPower: true))
        XCTAssertFalse(MapPlan.batteryOn(setting: "auto", lowPower: false))
        XCTAssertTrue(MapPlan.batteryOn(setting: "on", lowPower: false))
        XCTAssertFalse(MapPlan.batteryOn(setting: "off", lowPower: true))
    }

    func testLayersWithoutASourceAreNeverDrawn() throws {
        let plan = try plan(settings(shown: ["wind", "pressure", "waves"]))
        XCTAssertEqual(plan.layers, [])
        XCTAssertEqual(plan.shown.map(\.id), ["apple-standard"])
    }

    func testOpacityComesFromTheSettingsOrAPreview() throws {
        var settings = try settings(shown: ["sst"])
        let i = try XCTUnwrap(settings.map.layers.firstIndex { $0.id == "sst" })
        settings.map.layers[i].opacity = 0.4
        let catalogue = try mapCatalogue(settingsJson: try settings.json())
        let saved = MapPlan.make(catalogue: catalogue, settings: settings.map, battery: false)
        XCTAssertEqual(saved.layers[0].opacity, 0.4, accuracy: 0.001)
        let previewed = MapPlan.make(
            catalogue: catalogue, settings: settings.map, battery: false, opacities: ["sst": 0.9])
        XCTAssertEqual(previewed.layers[0].opacity, 0.9, accuracy: 0.001)
        // An opacity changes the renderer, not the overlay.
        XCTAssertEqual(saved.layers[0].overlayKey, previewed.layers[0].overlayKey)
    }

    func testADateTimeOrRangeMakesANewOverlay() throws {
        let settings = try settings(shown: ["sst-imagery", "sst"])
        let catalogue = try mapCatalogue(settingsJson: try settings.json())
        let plain = MapPlan.make(catalogue: catalogue, settings: settings.map, battery: false)
        let dated = MapPlan.make(
            catalogue: catalogue, settings: settings.map, battery: false,
            dates: ["sst-imagery": "2026-11-15"], times: ["sst": 1_794_700_800])
        XCTAssertEqual(dated.layers.first { $0.id == "sst-imagery" }?.date, "2026-11-15")
        XCTAssertEqual(dated.layers.first { $0.id == "sst" }?.time, 1_794_700_800)
        XCTAssertNotEqual(plain.layers.map(\.overlayKey), dated.layers.map(\.overlayKey))
        // A date is for tiles, a time for grids.
        let crossed = MapPlan.make(
            catalogue: catalogue, settings: settings.map, battery: false,
            dates: ["sst": "2026-11-15"], times: ["sst-imagery": 1])
        XCTAssertEqual(crossed.layers.map(\.overlayKey), plain.layers.map(\.overlayKey))
    }

    // MARK: - Overlay order

    func testUnchangedOverlaysStay() {
        let steps = OverlayOrder.steps(current: ["a", "b"], wanted: ["a", "b"])
        XCTAssertEqual(steps.remove, [])
        XCTAssertTrue(steps.insert.isEmpty)
    }

    func testNewOverlaysGoAboveTheirNeighbourBelow() {
        let steps = OverlayOrder.steps(current: ["a", "c"], wanted: ["x", "a", "b", "c", "d"])
        XCTAssertEqual(steps.remove, [])
        XCTAssertEqual(steps.insert.map(\.key), ["x", "b", "d"])
        XCTAssertEqual(steps.insert.map(\.above), [nil, "a", "c"])
    }

    func testGoneOverlaysAreRemovedAlone() {
        let steps = OverlayOrder.steps(current: ["a", "b", "c"], wanted: ["a", "c"])
        XCTAssertEqual(steps.remove, ["b"])
        XCTAssertTrue(steps.insert.isEmpty)
    }

    func testAReorderPutsTheKeptOverlaysBackInOrder() {
        let steps = OverlayOrder.steps(current: ["a", "b", "c"], wanted: ["c", "a"])
        XCTAssertEqual(steps.remove, ["a", "b", "c"])
        XCTAssertEqual(steps.insert.map(\.key), ["c", "a"])
        XCTAssertEqual(steps.insert.map(\.above), [nil, "c"])
    }

    func testTheZoomOfAViewIsTheTileZoomItsPointsMatch() {
        let world = MapRegion.worldMapPoints
        XCTAssertEqual(MapRegion.zoom(viewPoints: 256, mapPoints: world), 0, accuracy: 1e-9)
        // 390 points showing 390 / 256 tiles of zoom 10.
        XCTAssertEqual(
            MapRegion.zoom(viewPoints: 390, mapPoints: world / 1024 * 390 / 256), 10,
            accuracy: 1e-9)
        XCTAssertEqual(MapRegion.zoom(viewPoints: 0, mapPoints: world), 0)
    }
}
