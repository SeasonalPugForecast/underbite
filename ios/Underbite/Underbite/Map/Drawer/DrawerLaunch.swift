import Foundation

/// The drawer's launch argument, read in debug builds only (screenshots): `-mapDrawer <group>`
/// opens the drawer with that group open as well, `-mapDrawer presets` opens it on the presets.
/// Nothing is saved.
enum DrawerLaunch {
    private static var value: String? { LaunchArguments.value(after: "-mapDrawer") }

    /// Whether the drawer opens with the map.
    static var opens: Bool { value != nil }

    /// The group to open besides those with a layer on.
    static var group: String? {
        guard let value, value != presets else { return nil }
        return value
    }

    /// Whether the drawer opens on the presets.
    static var showsPresets: Bool { value == presets }

    private static let presets = "presets"
}
