import Foundation
import XCTest

@testable import UnderbiteKit

/// Sharing a map layer preset as a file and adding a received one, with the real core: what the
/// file holds, how a received preset reads, and that a name already here is never taken over
/// without being asked.
final class PresetSharingTests: XCTestCase {
    /// 2027-01-15 08:00 UTC.
    private static let now: Int64 = 1_800_000_000

    private func presets(_ settings: Settings) throws -> MapPresets {
        try mapPresets(settingsJson: try settings.json())
    }

    private func entry(_ id: String, _ settings: Settings) throws -> PresetEntry {
        try XCTUnwrap(PresetEditing.entries(try presets(settings)).first { $0.id == id }, id)
    }

    private func object(_ data: Data) throws -> [String: Any] {
        try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
    }

    private func layers(_ json: String) throws -> [Settings.MapLayer] {
        try JSONDecoder().decode([Settings.MapLayer].self, from: Data(json.utf8))
    }

    /// The user's own "Night flats": radar hidden, clarity at 70 % on a set range, spots on.
    private func withNightFlats() throws -> Settings {
        var settings = try Settings.defaults()
        let stored = """
            {"name": "Night flats", "layers": [
              {"id": "radar", "shown": false},
              {"id": "clarity", "shown": true, "opacity": 0.7, "ramp": "turbidity",
               "range": [0.05, 1.5]},
              {"id": "spots", "shown": true}
            ]}
            """
        settings.map.presets.append(
            try JSONDecoder().decode(Settings.MapPreset.self, from: Data(stored.utf8)))
        return settings
    }

    private func readBack(_ file: UnderbiteFile, settings: Settings) throws -> ReceivedFile {
        ReceivedFile.read(file.data, settingsJSON: try settings.json(), now: Self.now)
    }

    private func presetIn(_ file: ReceivedFile) throws -> ReceivedPreset {
        guard case .preset(let preset) = file.content else {
            XCTFail("\(file)")
            throw CancellationError()
        }
        return preset
    }

    private func assertValid(_ settings: Settings, file: StaticString = #filePath, line: UInt = #line)
        throws
    {
        XCTAssertEqual(
            validateSettings(json: try settings.json()).filter { $0.severity == .error }, [],
            file: file, line: line)
    }

    // MARK: - Sharing

    func testABuiltInPresetIsSharedAsTheLayersItShowsUnderItsName() throws {
        let settings = try Settings.defaults()
        let flats = try entry("inshore-flats", settings)
        let file = try PresetSharing.file(for: flats, map: settings.map)
        XCTAssertEqual(file.fileName, "\(flats.name).underbite")
        let v = try object(file.data)
        XCTAssertEqual(v["kind"] as? String, "layers")
        let payload = try XCTUnwrap(v["layers"] as? [String: Any])
        XCTAssertEqual(payload["name"] as? String, flats.name)
        XCTAssertEqual(Set(payload.keys), ["name", "layers"], "no places, keys or dates")
        let shared = try XCTUnwrap(payload["layers"] as? [[String: Any]])
        XCTAssertEqual(shared.compactMap { $0["id"] as? String }, flats.layers.map(\.id))
        XCTAssertTrue(shared.allSatisfy { $0["shown"] as? Bool == true })
    }

    func testTheUsersOwnPresetIsSharedAsSavedAndReadBackTheSame() throws {
        let settings = try withNightFlats()
        let night = try entry("Night flats", settings)
        let file = try PresetSharing.file(for: night, map: settings.map)
        let text = String(decoding: file.data, as: UTF8.self)
        XCTAssertTrue(text.contains("0.7"), text)
        XCTAssertFalse(text.contains("0.699"), "opacity as typed")

        let read = try readBack(file, settings: try Settings.defaults())
        XCTAssertEqual(read.heading, PresetSharing.words(settingsJSON: try settings.json())?.receiveHeading)
        XCTAssertEqual(read.choices(trips: [], spots: []), [], "added by its own preview")
        let preset = try presetIn(read)
        XCTAssertEqual(preset.name, "Night flats")
        XCTAssertEqual(preset.notes, [])
        XCTAssertEqual(try layers(preset.layersJSON), settings.map.presets.last?.layers)
        XCTAssertEqual(preset.shownIDs, ["clarity", "spots"])
    }

    func testLayersThisBuildLacksAreLeftOutAndNamed() throws {
        let settings = try Settings.defaults()
        let text = try encodeLayerPreset(
            name: "Glow",
            layersJson: #"[{"id":"glow-worms","shown":true},{"id":"radar","shown":true}]"#)
        let preset = try presetIn(
            try readBack(UnderbiteFile(fileName: "Glow.underbite", data: Data(text.utf8)), settings: settings))
        XCTAssertEqual(preset.shownIDs, ["radar"])
        XCTAssertEqual(preset.notes.count, 1)
        XCTAssertTrue(preset.notes[0].contains("glow-worms"), preset.notes[0])
    }

    func testAPresetFileFromANewerVersionShowsTheCoresText() throws {
        let newer = Data(#"{"format":"underbite","kind":"layers","v":99,"layers":{}}"#.utf8)
        let read = ReceivedFile.read(newer, settingsJSON: try Settings.defaults().json(), now: Self.now)
        guard case .problem(let message) = read.content else { return XCTFail("\(read)") }
        XCTAssertTrue(message.contains("newer version"), message)
        XCTAssertNil(read.heading)
    }

    // MARK: - Adding

    func testAFreeNameIsAddedAsTheUsersOwnPreset() throws {
        let sender = try withNightFlats()
        let file = try PresetSharing.file(for: try entry("Night flats", sender), map: sender.map)
        var settings = try Settings.defaults()
        let received = try presetIn(try readBack(file, settings: settings))
        let all = try presets(settings)
        XCTAssertNil(PresetSharing.existing(named: received.name, presets: all))
        let before = settings.map
        XCTAssertEqual(
            PresetSharing.add(received, as: received.name, replacing: false, presets: all, in: &settings.map),
            "Night flats")
        XCTAssertEqual(settings.map.presets.last?.name, "Night flats")
        XCTAssertEqual(settings.map.presets.last?.layers, sender.map.presets.last?.layers)
        XCTAssertEqual(settings.map.layers, before.layers, "the map stays as it is")
        XCTAssertEqual(settings.map.activePreset, before.activePreset)
        try assertValid(settings)
    }

    func testANameAlreadyHereIsNeverTakenOverWithoutBeingAsked() throws {
        let defaults = try Settings.defaults()
        let flats = try entry("inshore-flats", defaults)
        let file = try PresetSharing.file(for: flats, map: defaults.map)
        var settings = try withNightFlats()
        let received = try presetIn(try readBack(file, settings: settings))
        let all = try presets(settings)
        // The built-in preset's name, in any case, and the user's own are taken.
        XCTAssertEqual(PresetSharing.existing(named: received.name, presets: all), "inshore-flats")
        XCTAssertEqual(PresetSharing.existing(named: "  NIGHT flats ", presets: all), "Night flats")
        XCTAssertFalse(PresetSharing.canAdd(as: received.name, presets: all))
        XCTAssertFalse(PresetSharing.canAdd(as: " ", presets: all))
        XCTAssertTrue(PresetSharing.canAdd(as: "Flats from Sam", presets: all))

        let before = settings.map
        XCTAssertNil(
            PresetSharing.add(received, as: received.name, replacing: false, presets: all, in: &settings.map))
        XCTAssertEqual(settings.map, before)

        // Under another name: a preset of the user's own.
        XCTAssertEqual(
            PresetSharing.add(received, as: " Flats from Sam ", replacing: false, presets: all, in: &settings.map),
            "Flats from Sam")
        XCTAssertEqual(settings.map.presets.last?.name, "Flats from Sam")
        try assertValid(settings)
    }

    func testReplacingPutsTheReceivedLayersInPlaceOfThePresetOfThatName() throws {
        let sender = try withNightFlats()
        let night = try PresetSharing.file(for: try entry("Night flats", sender), map: sender.map)
        var settings = try withNightFlats()
        settings.map.presets[settings.map.presets.count - 1].layers.removeLast()
        let count = settings.map.presets.count
        let received = try presetIn(try readBack(night, settings: settings))
        XCTAssertEqual(
            PresetSharing.add(
                received, as: received.name, replacing: true, presets: try presets(settings),
                in: &settings.map),
            "Night flats")
        XCTAssertEqual(settings.map.presets.count, count)
        XCTAssertEqual(settings.map.presets.last?.layers, sender.map.presets.last?.layers)

        // A built-in preset replaced becomes the user's version of it, which restore removes.
        var renamed = received
        renamed.name = "Inshore Flats"
        XCTAssertEqual(
            PresetSharing.add(
                renamed, as: renamed.name, replacing: true, presets: try presets(settings),
                in: &settings.map),
            "inshore-flats")
        let flats = try entry("inshore-flats", settings)
        XCTAssertTrue(flats.edited)
        XCTAssertEqual(settings.map.presets.first { $0.name == "inshore-flats" }?.layers, sender.map.presets.last?.layers)
        try assertValid(settings)
    }
}
