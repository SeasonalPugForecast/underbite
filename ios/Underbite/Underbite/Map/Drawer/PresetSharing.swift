import Foundation
@preconcurrency import UnderbiteKit

/// A map layer preset someone sent, as the core read it: the layers this version does not have
/// are left out and the Copernicus Marine ones are off unless the user opted in to that source.
/// The package tests compile this file too.
nonisolated struct ReceivedPreset: Equatable, Sendable {
    /// The name the sender gave it.
    var name: String
    /// Its layer states as the settings keep them in a preset (the core's JSON), in drawing
    /// order.
    var layersJSON: String
    /// What the core says of the layers it left out or switched off.
    var notes: [String]

    init(name: String, layersJSON: String, notes: [String]) {
        self.name = name
        self.layersJSON = layersJSON
        self.notes = notes
    }

    init(_ shared: SharedPreset) {
        self.init(name: shared.name, layersJSON: shared.layersJson, notes: shared.notes)
    }

    /// The ids of the layers it shows, in drawing order.
    var shownIDs: [String] { PresetSharing.shownIDs(layersJSON: layersJSON) }
}

/// Sharing map layer presets as `.underbite` files and adding received ones to the user's
/// presets. The core writes and reads the file; presets are read and written in the settings'
/// one stored form (`map.presets`), as the drawer saves them. A file holds the preset's name and
/// layer states only: no places, keys or dates.
nonisolated enum PresetSharing {
    /// The core's words in the settings' voice (the default settings' when those cannot be
    /// read).
    static func words(settingsJSON: String) -> PresetShareWords? {
        (try? presetShareWords(settingsJson: settingsJSON))
            ?? (try? presetShareWords(settingsJson: defaultSettingsJson()))
    }

    /// A preset's layer states as the settings keep them, as JSON. The user's version (their
    /// own preset, or theirs of a built-in one) is shared as saved: every layer's state and the
    /// order. A built-in preset is shared as the layers it shows, on, at its opacities.
    static func layersJSON(for entry: PresetEntry, map: Settings.MapSettings) -> String {
        if let saved = map.presets.first(where: { $0.name == entry.id }),
            let data = try? JSONEncoder().encode(saved.layers)
        {
            return String(decoding: data, as: UTF8.self)
        }
        let layers: [[String: Any]] = entry.layers.map { layer in
            var state: [String: Any] = ["id": layer.id, "shown": true]
            if let opacity = layer.opacity {
                state["opacity"] = PresetEditing.storedOpacity(opacity)
            }
            return state
        }
        guard let data = try? JSONSerialization.data(withJSONObject: layers) else { return "[]" }
        return String(decoding: data, as: UTF8.self)
    }

    /// The `.underbite` file of `entry`, written by the core and named after the preset.
    static func file(for entry: PresetEntry, map: Settings.MapSettings) throws -> UnderbiteFile {
        let text = try encodeLayerPreset(
            name: entry.name, layersJson: layersJSON(for: entry, map: map))
        return UnderbiteFile(
            fileName: UnderbiteFile.fileName(for: entry.name), data: Data(text.utf8))
    }

    /// Reads a preset file with the core, for its preview.
    static func read(_ data: Data, settingsJSON: String) throws -> ReceivedPreset {
        ReceivedPreset(try decodeLayerPreset(bytes: data, settingsJson: settingsJSON))
    }

    /// The ids of the layers a preset's JSON shows, in drawing order.
    static func shownIDs(layersJSON: String) -> [String] {
        layers(layersJSON).filter(\.shown).map(\.id)
    }

    /// The id of the preset already called `name`: a built-in one by its name or id, or one of
    /// the user's own, ignoring case and surrounding spaces. None when the name is free or blank.
    static func existing(named name: String, presets: MapPresets) -> String? {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }
        return presets.presets.first {
            $0.name.caseInsensitiveCompare(trimmed) == .orderedSame
                || $0.id.caseInsensitiveCompare(trimmed) == .orderedSame
        }?.id
    }

    /// Whether a received preset can be added as `name`: not blank, and no preset has it.
    static func canAdd(as name: String, presets: MapPresets) -> Bool {
        !name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && existing(named: name, presets: presets) == nil
    }

    /// Adds `preset` to the user's presets as `name`. A name no preset has adds a preset of the
    /// user's own. A name taken by another preset is refused unless `replacing`, when the
    /// received layers take the place of the user's preset of that name, or become their
    /// version of the built-in one (which "restore" puts back). The layers on the map and the
    /// active preset stay. Returns the id it was saved under; none when nothing was added.
    @discardableResult
    static func add(
        _ preset: ReceivedPreset, as name: String, replacing: Bool, presets: MapPresets,
        in map: inout Settings.MapSettings
    ) -> String? {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }
        var id = trimmed
        if let existing = existing(named: trimmed, presets: presets) {
            guard replacing else { return nil }
            id = existing
        }
        guard let made = makePreset(name: id, layersJSON: preset.layersJSON) else { return nil }
        if let i = map.presets.firstIndex(where: { $0.name == id }) {
            map.presets[i].layers = made.layers
        } else {
            map.presets.append(made)
        }
        return id
    }

    private static func layers(_ json: String) -> [Settings.MapLayer] {
        (try? JSONDecoder().decode([Settings.MapLayer].self, from: Data(json.utf8))) ?? []
    }

    /// A preset of the settings' type (it has no public initialiser) from the core's JSON.
    private static func makePreset(name: String, layersJSON: String) -> Settings.MapPreset? {
        guard
            let layers = try? JSONSerialization.jsonObject(with: Data(layersJSON.utf8)) as? [Any],
            let data = try? JSONSerialization.data(withJSONObject: [
                "name": name, "layers": layers,
            ] as [String: Any])
        else { return nil }
        return try? JSONDecoder().decode(Settings.MapPreset.self, from: data)
    }
}
