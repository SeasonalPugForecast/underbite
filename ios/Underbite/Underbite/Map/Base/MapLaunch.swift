import Foundation
import UnderbiteKit

/// The map's launch arguments, read in debug builds only (screenshots): `-mapBase <id>` draws that
/// base map, `-mapPreset <id>` applies that built-in preset (its base map and layers),
/// `-mapLayers <id,id,...>` shows those layers on top of the settings' (in that order, topmost
/// last), and `-mapAt <lat>,<lon>,<degrees of latitude>` opens the map there; `-mapHeading
/// <degrees>` turns it. They change what the map draws for that launch only; nothing is saved.
enum MapLaunch {
    static var base: String? { LaunchArguments.value(after: "-mapBase") }

    static var layers: [String] {
        guard let list = LaunchArguments.value(after: "-mapLayers") else { return [] }
        return list.split(separator: ",").map { String($0) }
    }

    static var start: MapStart? {
        guard let text = LaunchArguments.value(after: "-mapAt") else { return nil }
        let parts = text.split(separator: ",").compactMap { Double($0) }
        guard parts.count == 3 else { return nil }
        return MapStart(latitude: parts[0], longitude: parts[1], span: parts[2])
    }

    /// `-mapHeading <degrees>` turns the map clockwise from north.
    static var heading: Double? {
        LaunchArguments.value(after: "-mapHeading").flatMap { Double($0) }
    }

    static var preset: String? { LaunchArguments.value(after: "-mapPreset") }

    /// Whether any argument changes the map's settings.
    static var overrides: Bool { base != nil || !layers.isEmpty || preset != nil }

    /// The map settings with the launch's base and layers.
    static func apply(to map: inout Settings.MapSettings) {
        if let id = preset, let json = try? Settings.defaults().json(),
            let presets = try? mapPresets(settingsJson: json),
            let entry = PresetEditing.entries(presets).first(where: { $0.id == id })
        {
            // The built-in preset as it comes, over the default settings' layers.
            PresetEditing.apply(
                entry, presets: presets, known: Set(entry.layers.map(\.id)), to: &map)
        }
        if let base { map.base = base }
        let shown = layers
        guard !shown.isEmpty else { return }
        // Named layers move to the top, in the order named, and show.
        var moved = map.layers.filter { shown.contains($0.id) }
        moved.sort { (shown.firstIndex(of: $0.id) ?? 0) < (shown.firstIndex(of: $1.id) ?? 0) }
        for i in moved.indices { moved[i].shown = true }
        map.layers = map.layers.filter { !shown.contains($0.id) } + moved
    }
}
