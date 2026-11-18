import Foundation
import XCTest

@testable import UnderbiteKit

/// What the layer drawer lists, in which order, and what its reordering writes.
final class DrawerSectionsTests: XCTestCase {
    /// The default settings with `shown` switched on.
    private func settings(shown: [String] = []) throws -> Settings {
        var settings = try Settings.defaults()
        for i in settings.map.layers.indices {
            settings.map.layers[i].shown = shown.contains(settings.map.layers[i].id)
        }
        return settings
    }

    private func catalogue(_ settings: Settings) throws -> [LayerInfo] {
        try mapCatalogue(settingsJson: try settings.json())
    }

    private func groups(_ settings: Settings) throws -> [DrawerGroup] {
        DrawerSections.groups(catalogue: try catalogue(settings), settings: settings.map)
    }

    func testTheCatalogueIsListedOnceByGroupWithTheBaseMapsFirst() throws {
        let settings = try settings()
        let catalogue = try catalogue(settings)
        let groups = try groups(settings)
        XCTAssertEqual(groups.first?.id, "base")
        XCTAssertTrue(try XCTUnwrap(groups.first).isBase)
        XCTAssertTrue(groups.dropFirst().allSatisfy { !$0.isBase })
        XCTAssertEqual(groups.first?.layers.map(\.id), catalogue.filter(\.isBase).map(\.id))
        // Every layer once, each under its own group with the core's title.
        let listed = groups.flatMap(\.layers)
        XCTAssertEqual(Set(listed.map(\.id)), Set(catalogue.map(\.id)))
        XCTAssertEqual(listed.count, catalogue.count)
        for group in groups {
            XCTAssertFalse(group.title.isEmpty)
            XCTAssertTrue(group.layers.allSatisfy { $0.group == group.id && $0.groupTitle == group.title })
        }
        // The groups in the catalogue's order.
        var order: [String] = []
        for layer in catalogue where !order.contains(layer.group) { order.append(layer.group) }
        XCTAssertEqual(groups.map(\.id), order)
        XCTAssertTrue(order.contains("water"))
    }

    func testAGroupIsListedInTheSettingsDrawingOrder() throws {
        var settings = try settings()
        let water = try XCTUnwrap(try groups(settings).first { $0.id == "water" })
        XCTAssertEqual(water.layers.first?.id, "sst")
        // SST moved to the top of the drawing order is listed last in its group.
        let sst = try XCTUnwrap(settings.map.layers.first { $0.id == "sst" })
        settings.map.layers.removeAll { $0.id == "sst" }
        settings.map.layers.append(sst)
        let moved = try XCTUnwrap(try groups(settings).first { $0.id == "water" })
        XCTAssertEqual(moved.layers.last?.id, "sst")
        XCTAssertEqual(moved.layers.count, water.layers.count)
    }

    func testAMoveStaysInItsGroupAndKeepsTheOtherLayersInPlace() throws {
        let settings = try settings(shown: ["sst", "chlorophyll", "tide-stations"])
        let water = try XCTUnwrap(try groups(settings).first { $0.id == "water" })
        let ids = water.layers.map(\.id)
        var map = settings.map
        // The first water layer moves to the end of its group.
        DrawerSections.move(ids: ids, fromOffsets: [0], toOffset: ids.count, in: &map)
        let group = Set(ids)
        XCTAssertEqual(
            map.layers.filter { group.contains($0.id) }.map(\.id), Array(ids.dropFirst()) + [ids[0]])
        // Every other layer keeps its index, and the moved one its state.
        for (i, layer) in settings.map.layers.enumerated() where !group.contains(layer.id) {
            XCTAssertEqual(map.layers[i], layer)
        }
        XCTAssertEqual(map.layers.count, settings.map.layers.count)
        XCTAssertEqual(
            map.layers.first { $0.id == ids[0] }, settings.map.layers.first { $0.id == ids[0] })
        // The drawer lists the new order.
        var changed = settings
        changed.map = map
        XCTAssertEqual(try groups(changed).first { $0.id == "water" }?.layers.map(\.id), map.layers.filter { group.contains($0.id) }.map(\.id))
    }

    func testMovesMeanWhatAListsOnMoveMeans() {
        let list = ["a", "b", "c", "d"]
        XCTAssertEqual(DrawerSections.moving(list, fromOffsets: [0], toOffset: 4), ["b", "c", "d", "a"])
        XCTAssertEqual(DrawerSections.moving(list, fromOffsets: [3], toOffset: 0), ["d", "a", "b", "c"])
        XCTAssertEqual(DrawerSections.moving(list, fromOffsets: [2], toOffset: 1), ["a", "c", "b", "d"])
        XCTAssertEqual(DrawerSections.moving(list, fromOffsets: [1, 2], toOffset: 4), ["a", "d", "b", "c"])
        XCTAssertEqual(DrawerSections.moving(list, fromOffsets: [1], toOffset: 2), list)
        XCTAssertEqual(DrawerSections.moving(list, fromOffsets: [9], toOffset: 0), list)
    }

    func testAMoveToTheSamePlaceWritesNothing() throws {
        let settings = try settings()
        let ids = try XCTUnwrap(try groups(settings).first { $0.id == "water" }).layers.map(\.id)
        var map = settings.map
        DrawerSections.move(ids: ids, fromOffsets: [1], toOffset: 1, in: &map)
        XCTAssertEqual(map, settings.map)
    }

    func testAMoveAddsALayerTheSettingsLackHidden() throws {
        var settings = try settings()
        let ids = try XCTUnwrap(try groups(settings).first { $0.id == "water" }).layers.map(\.id)
        settings.map.layers.removeAll { $0.id == "clarity" }
        // The drawer still lists it (on top), and a move gives it a place.
        let listed = try XCTUnwrap(try groups(settings).first { $0.id == "water" }).layers.map(\.id)
        XCTAssertEqual(listed.last, "clarity")
        XCTAssertEqual(Set(listed), Set(ids))
        var map = settings.map
        DrawerSections.move(ids: listed, fromOffsets: [listed.count - 1], toOffset: 0, in: &map)
        let clarity = try XCTUnwrap(map.layers.first { $0.id == "clarity" })
        XCTAssertFalse(clarity.shown)
        XCTAssertNil(clarity.opacity)
        XCTAssertEqual(map.layers.filter { Set(ids).contains($0.id) }.first?.id, "clarity")
        // What is written passes the core's checks.
        var written = settings
        written.map = map
        XCTAssertEqual(validateSettings(json: try written.json()).filter { $0.severity == .error }, [])
    }

    func testTheBatteryModeKeepsAThirdRasterOff() throws {
        let settings = try settings(shown: ["sst", "chlorophyll", "tide-stations"])
        let catalogue = try catalogue(settings)
        func layer(_ id: String) throws -> LayerInfo { try XCTUnwrap(catalogue.first { $0.id == id }) }
        XCTAssertEqual(DrawerSections.shownRasters(in: catalogue).map(\.id), ["sst", "chlorophyll"])
        XCTAssertTrue(
            DrawerSections.batteryBlocks(try layer("true-color-viirs-n20"), catalogue: catalogue, battery: true))
        XCTAssertTrue(
            DrawerSections.batteryBlocks(try layer("clarity"), catalogue: catalogue, battery: true))
        // What is on can be switched off; arrows, stations and base maps do not count.
        XCTAssertFalse(DrawerSections.batteryBlocks(try layer("sst"), catalogue: catalogue, battery: true))
        XCTAssertFalse(
            DrawerSections.batteryBlocks(try layer("hf-radar-currents"), catalogue: catalogue, battery: true))
        XCTAssertFalse(
            DrawerSections.batteryBlocks(try layer("buoys"), catalogue: catalogue, battery: true))
        XCTAssertFalse(
            DrawerSections.batteryBlocks(try layer("noaa-chart"), catalogue: catalogue, battery: true))
        // Without the mode nothing is kept off.
        XCTAssertFalse(
            DrawerSections.batteryBlocks(try layer("clarity"), catalogue: catalogue, battery: false))
        // With one raster on, a second may come.
        let one = try self.catalogue(try self.settings(shown: ["sst"]))
        XCTAssertFalse(
            DrawerSections.batteryBlocks(
                try XCTUnwrap(one.first { $0.id == "clarity" }), catalogue: one, battery: true))
    }

    func testOnlyWhatTheMapDrawsItselfHasAnOpacity() throws {
        let catalogue = try catalogue(try settings())
        func has(_ id: String) throws -> Bool {
            DrawerSections.hasOpacity(try XCTUnwrap(catalogue.first { $0.id == id }))
        }
        XCTAssertTrue(try has("sst"))
        XCTAssertTrue(try has("true-color-viirs-n20"))
        XCTAssertTrue(try has("hf-radar-currents"))
        XCTAssertFalse(try has("tide-stations"))
        XCTAssertFalse(try has("apple-standard"))
        XCTAssertFalse(try has("noaa-chart"))
    }

    func testUnavailableLayersSayWhyAndChartsCurrentsAndDepthsCarryTheNotice() throws {
        let catalogue = try catalogue(try settings(shown: ["wind", "pressure", "waves"]))
        for id in ["wind", "pressure", "waves"] {
            let layer = try XCTUnwrap(catalogue.first { $0.id == id })
            XCTAssertFalse(layer.available, id)
            XCTAssertFalse(layer.shown, id)
            XCTAssertFalse(try XCTUnwrap(layer.unavailableText, id).isEmpty, id)
        }
        for id in ["noaa-chart", "hf-radar-currents"] {
            XCTAssertEqual(catalogue.first { $0.id == id }?.notice, "Not for navigation.", id)
        }
        XCTAssertNil(try XCTUnwrap(catalogue.first { $0.id == "sst" }).notice)
    }

    func testTheLegendIsTheOneTheMapDraws() throws {
        let catalogue = try catalogue(try settings(shown: ["sst"]))
        let sst = try XCTUnwrap(catalogue.first { $0.id == "sst" })
        XCTAssertFalse(sst.legend.isEmpty)
        XCTAssertEqual(DrawerSections.legend(sst, drawn: nil), sst.legend)
        XCTAssertEqual(
            DrawerSections.legend(sst, drawn: MapLegendState(stops: [], fittedCaption: nil)),
            sst.legend)
        let drawn = Array(sst.legend.prefix(2))
        XCTAssertEqual(
            DrawerSections.legend(sst, drawn: MapLegendState(stops: drawn, fittedCaption: "x")),
            drawn)
        // The ends and the middle are labelled.
        let labelled = DrawerSections.labelled(sst.legend)
        XCTAssertEqual(labelled.first, sst.legend.first)
        XCTAssertEqual(labelled.last, sst.legend.last)
        XCTAssertLessThanOrEqual(labelled.count, 3)
        XCTAssertTrue(labelled.allSatisfy { !$0.label.isEmpty })
    }
}
