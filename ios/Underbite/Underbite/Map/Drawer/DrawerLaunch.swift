import Foundation

/// The drawer's launch argument, read in debug builds only (screenshots): `-mapDrawer <group>`
/// opens the drawer with only that group open, `-mapDrawer presets` opens it on the presets.
/// Nothing is saved.
enum DrawerLaunch {
    private static var value: String? { LaunchArguments.value(after: "-mapDrawer") }

    /// Whether the drawer opens with the map.
    static var opens: Bool { value != nil }

    /// The one group to open.
    static var group: String? {
        guard let value, value != presets else { return nil }
        return value
    }

    /// Whether the drawer opens on the presets.
    static var showsPresets: Bool { value == presets }

    private static let presets = "presets"

    private static var shared = false

    /// With `-sharePreset <id>`, the preset whose share sheet opens over the presets, once.
    static func presetToShare() -> String? {
        guard !shared, let id = LaunchArguments.value(after: "-sharePreset") else { return nil }
        shared = true
        return id
    }
}
