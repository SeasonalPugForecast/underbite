import Foundation
import UnderbiteKit

/// A preset as the drawer lists it: a built-in one (with the user's own version when they saved
/// over it) or one of the user's own.
nonisolated struct PresetEntry: Equatable, Sendable, Identifiable {
    /// What the settings store as the active preset: the built-in id or the user's name.
    let id: String
    /// The core's name for a built-in preset; the user's own as typed.
    let name: String
    /// One of the core's presets.
    let builtIn: Bool
    /// A built-in preset the user saved their own layers over.
    let edited: Bool
    /// The layers it shows.
    let layers: [PresetLayerInfo]
}

/// The preset choices of the layer drawer and what they write to the map settings. Built-in
/// presets belong to the core; the user's live in `map.presets` as snapshots of `map.layers`.
/// Saving over a built-in preset stores the user's version under the built-in id (the user's
/// wins on a clash), which "restore" removes again; the user's own presets are never touched by
/// it. A preset may name layers this build does not have (saved by a newer version): they stay
/// in the preset and are left off the map.
nonisolated enum PresetEditing {
    /// The presets in the core's order: the built-in ones, each replaced by the user's version
    /// when there is one, then the user's own.
    static func entries(_ presets: MapPresets) -> [PresetEntry] {
        let builtIn = presets.presets.filter(\.builtIn)
        let builtInIDs = Set(builtIn.map(\.id))
        let own = presets.presets.filter { !$0.builtIn }
        var out = builtIn.map { preset in
            let mine = own.first { $0.id == preset.id }
            return PresetEntry(
                id: preset.id, name: preset.name, builtIn: true, edited: mine != nil,
                layers: mine?.layers ?? preset.layers)
        }
        var seen = Set<String>()
        for preset in own where !builtInIDs.contains(preset.id) && seen.insert(preset.id).inserted {
            out.append(
                PresetEntry(
                    id: preset.id, name: preset.name, builtIn: false, edited: false,
                    layers: preset.layers))
        }
        return out
    }

    /// Whether the user saved over any built-in preset.
    static func hasEdits(_ presets: MapPresets) -> Bool {
        entries(presets).contains(where: \.edited)
    }

    /// Shows `entry` on the map and makes it the active preset.
    ///
    /// The user's version of a preset (their own, or theirs of a built-in one) gives every layer
    /// it knows its saved state and order; layers it does not name go above it, hidden. A
    /// built-in preset shows its layers on top in its order at its opacities and hides the rest.
    /// A built-in preset also chooses its base map; the user's own keep the one on screen.
    ///
    /// - Parameters:
    ///   - presets: the core's presets for the settings (`map_presets`), for the built-in base.
    ///   - known: the catalogue's overlay ids; anything else in a preset stays off the map.
    static func apply(
        _ entry: PresetEntry, presets: MapPresets, known: Set<String>,
        to map: inout Settings.MapSettings
    ) {
        let template = map.layers.first
        if let saved = map.presets.first(where: { $0.name == entry.id }) {
            var seen = Set<String>()
            let states = saved.layers.filter { known.contains($0.id) && seen.insert($0.id).inserted }
            let rest = map.layers.filter { !seen.contains($0.id) }.map { state -> Settings.MapLayer in
                var hidden = state
                hidden.shown = false
                return hidden
            }
            map.layers = states + rest
        } else {
            var shown: [String: PresetLayerInfo] = [:]
            var order: [String] = []
            for layer in entry.layers where known.contains(layer.id) && shown[layer.id] == nil {
                shown[layer.id] = layer
                order.append(layer.id)
            }
            for id in order where !map.layers.contains(where: { $0.id == id }) {
                if let added = DrawerSections.layerState(id: id, like: template) {
                    map.layers.append(added)
                }
            }
            var rest: [Settings.MapLayer] = []
            var top: [String: Settings.MapLayer] = [:]
            for var state in map.layers {
                if let preset = shown[state.id] {
                    state.shown = true
                    state.opacity = preset.opacity.map(storedOpacity)
                    if top[state.id] == nil { top[state.id] = state }
                } else {
                    state.shown = false
                    rest.append(state)
                }
            }
            map.layers = rest + order.compactMap { top[$0] }
        }
        if entry.builtIn,
            let base = presets.presets.first(where: { $0.builtIn && $0.id == entry.id })?.base
        {
            map.base = base
        }
        map.activePreset = entry.id
    }

    /// The id a typed preset name saves under: a built-in preset's id when the name is that
    /// preset's (saving over it), else the name itself, trimmed; none for a blank name.
    static func savedID(for name: String, presets: MapPresets) -> String? {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }
        if let builtIn = presets.presets.first(where: {
            $0.builtIn
                && ($0.name.caseInsensitiveCompare(trimmed) == .orderedSame
                    || $0.id.caseInsensitiveCompare(trimmed) == .orderedSame)
        }) {
            return builtIn.id
        }
        return trimmed
    }

    /// Saves the layers on the map (every layer's state and the order) as the preset `name`,
    /// over the user's preset of that name or the built-in preset it names, and makes it the
    /// active one. False for a blank name.
    @discardableResult
    static func save(name: String, presets: MapPresets, in map: inout Settings.MapSettings) -> Bool
    {
        guard let id = savedID(for: name, presets: presets) else { return false }
        if let i = map.presets.firstIndex(where: { $0.name == id }) {
            map.presets[i].layers = map.layers
        } else {
            guard let preset = newPreset(name: id, layers: map.layers) else { return false }
            map.presets.append(preset)
        }
        map.activePreset = id
        return true
    }

    /// Whether `name` may be given to the user's preset `id`: not blank, and no other preset
    /// (theirs or built in) has it.
    static func canRename(_ id: String, to name: String, presets: MapPresets) -> Bool {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return false }
        if trimmed == id { return true }
        return !presets.presets.contains {
            ($0.id != id || $0.builtIn)
                && ($0.name.caseInsensitiveCompare(trimmed) == .orderedSame
                    || $0.id.caseInsensitiveCompare(trimmed) == .orderedSame)
        }
    }

    /// Renames the user's own preset `id`; the active preset follows. False when the name may not
    /// be used (``canRename(_:to:presets:)``) or `id` is no preset of the user's own.
    @discardableResult
    static func rename(
        _ id: String, to name: String, presets: MapPresets, in map: inout Settings.MapSettings
    ) -> Bool {
        guard !presets.presets.contains(where: { $0.builtIn && $0.id == id }),
            canRename(id, to: name, presets: presets),
            let i = map.presets.firstIndex(where: { $0.name == id })
        else { return false }
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        map.presets[i].name = trimmed
        if map.activePreset == id { map.activePreset = trimmed }
        return true
    }

    /// Deletes the user's preset `id` (their own, or their version of a built-in one, which then
    /// is the core's again). When it was the active preset and is gone for good, `fallback` (the
    /// default settings' active preset) becomes the active one.
    static func delete(
        _ id: String, presets: MapPresets, fallback: String, in map: inout Settings.MapSettings
    ) {
        map.presets.removeAll { $0.name == id }
        let builtIn = presets.presets.contains { $0.builtIn && $0.id == id }
        if map.activePreset == id, !builtIn { map.activePreset = fallback }
    }

    /// Restores the built-in presets: the user's versions of them are removed; the user's own
    /// presets, the layers on the map and the active preset stay.
    static func restoreBuiltIns(presets: MapPresets, in map: inout Settings.MapSettings) {
        let builtIn = Set(presets.presets.filter(\.builtIn).map(\.id))
        map.presets.removeAll { builtIn.contains($0.name) }
    }

    /// A preset's opacity (single precision, from the core) as the settings store it: to the
    /// thousandth, so 0.7 stays 0.7 rather than 0.699999988.
    static func storedOpacity(_ opacity: Float) -> Double {
        (Double(opacity) * 1000).rounded() / 1000
    }

    /// A preset of the settings' type (it has no public initialiser).
    private static func newPreset(name: String, layers: [Settings.MapLayer]) -> Settings.MapPreset? {
        let object: [String: Any] = ["name": name, "layers": [Any]()]
        guard
            let data = try? JSONSerialization.data(withJSONObject: object),
            var preset = try? JSONDecoder().decode(Settings.MapPreset.self, from: data)
        else { return nil }
        preset.layers = layers
        return preset
    }
}
