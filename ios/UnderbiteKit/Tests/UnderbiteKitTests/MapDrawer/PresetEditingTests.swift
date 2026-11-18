import Foundation
import XCTest

@testable import UnderbiteKit

/// Picking, saving, renaming, deleting and restoring map presets, as the drawer writes them.
final class PresetEditingTests: XCTestCase {
    private func defaults(shown: [String] = []) throws -> Settings {
        var settings = try Settings.defaults()
        for i in settings.map.layers.indices {
            settings.map.layers[i].shown = shown.contains(settings.map.layers[i].id)
        }
        return settings
    }

    private func presets(_ settings: Settings) throws -> MapPresets {
        try mapPresets(settingsJson: try settings.json())
    }

    private func known(_ settings: Settings) throws -> Set<String> {
        Set(try mapCatalogue(settingsJson: try settings.json()).filter { !$0.isBase }.map(\.id))
    }

    private func entry(_ id: String, _ settings: Settings) throws -> PresetEntry {
        try XCTUnwrap(PresetEditing.entries(try presets(settings)).first { $0.id == id }, id)
    }

    /// `settings` with `change` applied to its map section.
    private func edited(
        _ settings: Settings, _ change: (inout Settings.MapSettings) -> Void
    ) -> Settings {
        var out = settings
        change(&out.map)
        return out
    }

    private func assertValid(_ settings: Settings, file: StaticString = #filePath, line: UInt = #line) throws {
        XCTAssertEqual(
            validateSettings(json: try settings.json()).filter { $0.severity == .error }, [],
            file: file, line: line)
    }

    private func shownIDs(_ settings: Settings) -> [String] {
        settings.map.layers.filter(\.shown).map(\.id)
    }

    func testTheBuiltInPresetsAreListedInTheCoreOrderWithTheCoreNames() throws {
        let settings = try defaults()
        let presets = try presets(settings)
        let entries = PresetEditing.entries(presets)
        XCTAssertEqual(entries.map(\.id), presets.presets.map(\.id))
        XCTAssertTrue(entries.allSatisfy { $0.builtIn && !$0.edited && !$0.name.isEmpty })
        XCTAssertGreaterThanOrEqual(entries.count, 6)
        XCTAssertTrue(entries.contains { $0.id == presets.active })
        XCTAssertFalse(PresetEditing.hasEdits(presets))
    }

    func testABuiltInPresetShowsItsLayersOnTopAndChoosesItsBase() throws {
        var settings = try defaults(shown: ["radar", "tide-stations"])
        // A range the user set stays with its layer.
        let sst = try XCTUnwrap(settings.map.layers.firstIndex { $0.id == "sst" })
        settings.map.layers[sst].range = [10, 20]
        settings.map.layers[sst].opacity = 0.3
        let offshore = try entry("offshore-pelagics", settings)
        let applied = edited(settings) {
            PresetEditing.apply(
                offshore, presets: try! self.presets(settings), known: try! self.known(settings), to: &$0)
        }
        let builtIn = try XCTUnwrap(try presets(settings).presets.first { $0.id == "offshore-pelagics" })
        XCTAssertEqual(applied.map.base, builtIn.base)
        XCTAssertEqual(applied.map.activePreset, "offshore-pelagics")
        XCTAssertEqual(shownIDs(applied), builtIn.layers.map(\.id))
        // Its layers draw on top, in its order; nothing is lost or doubled.
        XCTAssertEqual(
            Array(applied.map.layers.suffix(builtIn.layers.count).map(\.id)), builtIn.layers.map(\.id))
        XCTAssertEqual(applied.map.layers.count, settings.map.layers.count)
        XCTAssertEqual(Set(applied.map.layers.map(\.id)), Set(settings.map.layers.map(\.id)))
        let sstAfter = try XCTUnwrap(applied.map.layers.first { $0.id == "sst" })
        XCTAssertEqual(sstAfter.range, [10, 20])
        // The preset's opacity, here the layer's own.
        XCTAssertEqual(
            sstAfter.opacity,
            builtIn.layers.first { $0.id == "sst" }?.opacity.map(PresetEditing.storedOpacity))
        XCTAssertEqual(PresetEditing.storedOpacity(0.7), 0.7)
        try assertValid(applied)
        // The core agrees on what shows and which preset is active.
        XCTAssertEqual(try presets(applied).active, "offshore-pelagics")
        let catalogue = try mapCatalogue(settingsJson: try applied.json())
        XCTAssertEqual(
            Set(catalogue.filter { $0.shown && !$0.isBase }.map(\.id)), Set(builtIn.layers.map(\.id)))
    }

    func testSavingTheMapMakesAPresetThatBringsItBack() throws {
        var settings = try defaults(shown: ["sst", "clarity", "buoys"])
        let clarity = try XCTUnwrap(settings.map.layers.firstIndex { $0.id == "clarity" })
        settings.map.layers[clarity].opacity = 0.5
        settings.map.base = "noaa-chart"
        let saved = edited(settings) {
            XCTAssertTrue(PresetEditing.save(name: "  Canyon run ", presets: try! self.presets(settings), in: &$0))
        }
        XCTAssertEqual(saved.map.activePreset, "Canyon run")
        XCTAssertEqual(saved.map.presets.map(\.name), ["Canyon run"])
        XCTAssertEqual(saved.map.presets[0].layers, settings.map.layers)
        try assertValid(saved)
        // The core lists it after the built-in ones, with what it shows.
        let entries = PresetEditing.entries(try presets(saved))
        let mine = try XCTUnwrap(entries.last)
        XCTAssertEqual(mine.id, "Canyon run")
        XCTAssertEqual(mine.name, "Canyon run")
        XCTAssertFalse(mine.builtIn)
        XCTAssertEqual(Set(mine.layers.map(\.id)), ["sst", "clarity", "buoys"])
        XCTAssertEqual(try presets(saved).active, "Canyon run")

        // Another preset, then this one again: the layers, opacity and order come back, and the
        // base map stays the one on screen.
        var other = edited(saved) {
            PresetEditing.apply(
                try! self.entry("surf", saved), presets: try! self.presets(saved),
                known: try! self.known(saved), to: &$0)
        }
        XCTAssertNotEqual(shownIDs(other), shownIDs(settings))
        other.map.base = "apple-hybrid"
        let back = edited(other) {
            PresetEditing.apply(
                mine, presets: try! self.presets(other), known: try! self.known(other), to: &$0)
        }
        XCTAssertEqual(back.map.layers, settings.map.layers)
        XCTAssertEqual(back.map.base, "apple-hybrid")
        XCTAssertEqual(back.map.activePreset, "Canyon run")
    }

    func testSavingUnderAnExistingNameReplacesThatPreset() throws {
        let first = edited(try defaults(shown: ["sst"])) {
            PresetEditing.save(name: "Mine", presets: try! self.presets(try! Settings.defaults()), in: &$0)
        }
        var changed = first
        for i in changed.map.layers.indices { changed.map.layers[i].shown = changed.map.layers[i].id == "radar" }
        let second = edited(changed) {
            PresetEditing.save(name: "Mine", presets: try! self.presets(changed), in: &$0)
        }
        XCTAssertEqual(second.map.presets.count, 1)
        XCTAssertEqual(second.map.presets[0].layers.filter(\.shown).map(\.id), ["radar"])
        XCTAssertFalse(PresetEditing.save(name: "   ", presets: try presets(second), in: &changed.map))
    }

    func testAPresetFromANewerVersionKeepsItsUnknownLayersOffTheMap() throws {
        let base = try defaults()
        var json = try XCTUnwrap(
            JSONSerialization.jsonObject(with: Data(try base.json().utf8)) as? [String: Any])
        var map = try XCTUnwrap(json["map"] as? [String: Any])
        let layers: [[String: Any]] = [
            ["id": "kelp-canopy", "shown": true],
            ["id": "sst", "shown": true, "opacity": 0.6],
        ]
        let stored: [String: Any] = ["name": "Later", "layers": layers]
        map["presets"] = [stored]
        json["map"] = map
        let text = String(
            decoding: try JSONSerialization.data(withJSONObject: json, options: [.sortedKeys]),
            as: UTF8.self)
        let settings = try Settings(json: text)
        let presets = try mapPresets(settingsJson: text)
        let later = try XCTUnwrap(PresetEditing.entries(presets).first { $0.id == "Later" })
        let applied = edited(settings) {
            PresetEditing.apply(later, presets: presets, known: try! self.known(settings), to: &$0)
        }
        XCTAssertEqual(shownIDs(applied), ["sst"])
        XCTAssertFalse(applied.map.layers.contains { $0.id == "kelp-canopy" })
        XCTAssertEqual(applied.map.layers.first { $0.id == "sst" }?.opacity, 0.6)
        // The preset itself is kept as it was.
        XCTAssertEqual(applied.map.presets, settings.map.presets)
        XCTAssertEqual(applied.map.layers.count, settings.map.layers.count)
    }

    func testSavingOverABuiltInPresetIsUndoneByRestoreWhichKeepsTheUsersOwn() throws {
        var settings = try defaults(shown: ["sst"])
        let surf = try entry("surf", settings)
        // Typing a built-in preset's name saves over it.
        XCTAssertEqual(PresetEditing.savedID(for: surf.name.uppercased(), presets: try presets(settings)), "surf")
        settings = edited(settings) {
            PresetEditing.save(name: surf.name, presets: try! self.presets(settings), in: &$0)
        }
        settings = edited(settings) {
            PresetEditing.save(name: "Canyon run", presets: try! self.presets(settings), in: &$0)
        }
        try assertValid(settings)
        let presets = try presets(settings)
        let entries = PresetEditing.entries(presets)
        // The user's version takes the built-in's place, under its name.
        XCTAssertEqual(entries.filter { $0.id == "surf" }.count, 1)
        let mineOfSurf = try XCTUnwrap(entries.first { $0.id == "surf" })
        XCTAssertTrue(mineOfSurf.edited)
        XCTAssertEqual(mineOfSurf.name, surf.name)
        XCTAssertEqual(mineOfSurf.layers.map(\.id), ["sst"])
        XCTAssertEqual(entries.last?.id, "Canyon run")
        XCTAssertTrue(PresetEditing.hasEdits(presets))

        let restored = edited(settings) { PresetEditing.restoreBuiltIns(presets: presets, in: &$0) }
        XCTAssertEqual(restored.map.presets.map(\.name), ["Canyon run"])
        XCTAssertEqual(restored.map.layers, settings.map.layers)
        XCTAssertEqual(restored.map.activePreset, settings.map.activePreset)
        let after = PresetEditing.entries(try self.presets(restored))
        XCTAssertEqual(after.first { $0.id == "surf" }, surf)
        XCTAssertFalse(PresetEditing.hasEdits(try self.presets(restored)))
    }

    func testRenamingKeepsNamesApartAndTheActivePresetFollows() throws {
        let settings = edited(try defaults(shown: ["sst"])) {
            PresetEditing.save(name: "Canyon run", presets: try! self.presets(try! Settings.defaults()), in: &$0)
            PresetEditing.save(name: "Ledge", presets: try! self.presets(try! Settings.defaults()), in: &$0)
        }
        let presets = try presets(settings)
        let surf = try entry("surf", settings)
        XCTAssertTrue(PresetEditing.canRename("Canyon run", to: "Canyon", presets: presets))
        XCTAssertTrue(PresetEditing.canRename("Canyon run", to: "Canyon run", presets: presets))
        XCTAssertFalse(PresetEditing.canRename("Canyon run", to: " ", presets: presets))
        XCTAssertFalse(PresetEditing.canRename("Canyon run", to: "ledge", presets: presets))
        XCTAssertFalse(PresetEditing.canRename("Canyon run", to: surf.name, presets: presets))
        XCTAssertFalse(PresetEditing.canRename("Canyon run", to: "surf", presets: presets))

        var map = settings.map
        XCTAssertEqual(map.activePreset, "Ledge")
        XCTAssertTrue(PresetEditing.rename("Ledge", to: " Ledge north ", presets: presets, in: &map))
        XCTAssertEqual(map.presets.map(\.name), ["Canyon run", "Ledge north"])
        XCTAssertEqual(map.activePreset, "Ledge north")
        // A built-in preset keeps its name.
        XCTAssertFalse(PresetEditing.rename("surf", to: "Beach", presets: presets, in: &map))
        XCTAssertFalse(PresetEditing.rename("Ledge", to: "Again", presets: presets, in: &map))
    }

    func testDeletingTheActivePresetFallsBackToTheDefault() throws {
        let fallback = try Settings.defaults().map.activePreset
        var settings = edited(try defaults(shown: ["sst"])) {
            PresetEditing.save(name: "Canyon run", presets: try! self.presets(try! Settings.defaults()), in: &$0)
            PresetEditing.save(name: "surf", presets: try! self.presets(try! Settings.defaults()), in: &$0)
        }
        // The user's version of a built-in preset goes; the built-in one stays active.
        settings = edited(settings) {
            PresetEditing.delete("surf", presets: try! self.presets(settings), fallback: fallback, in: &$0)
        }
        XCTAssertEqual(settings.map.presets.map(\.name), ["Canyon run"])
        XCTAssertEqual(settings.map.activePreset, "surf")
        settings.map.activePreset = "Canyon run"
        settings = edited(settings) {
            PresetEditing.delete(
                "Canyon run", presets: try! self.presets(settings), fallback: fallback, in: &$0)
        }
        XCTAssertEqual(settings.map.presets, [])
        XCTAssertEqual(settings.map.activePreset, fallback)
        try assertValid(settings)
    }

    func testAnOldPresetNameIsTheDefaultPreset() throws {
        var settings = try defaults()
        settings.map.activePreset = "Inshore flats"
        let presets = try presets(settings)
        XCTAssertEqual(presets.active, try Settings.defaults().map.activePreset)
        XCTAssertTrue(PresetEditing.entries(presets).contains { $0.id == presets.active })
    }
}
