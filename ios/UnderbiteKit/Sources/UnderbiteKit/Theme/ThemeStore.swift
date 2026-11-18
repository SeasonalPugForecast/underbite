import Foundation

/// The chosen colour theme and the reader's custom themes, in the App Group's defaults so the
/// widgets paint with the same colours. Without the App Group (an unsigned build, the package
/// tests) the app's standard defaults hold them instead, and `backing` says so.
public final class ThemeStore: @unchecked Sendable {
    /// Where the themes are kept.
    public enum Backing: String, Sendable, CustomStringConvertible {
        /// The App Group's defaults, which the widgets read too.
        case appGroup = "app_group"
        /// The app's own defaults: the widgets do not see these.
        case standard

        public var description: String {
            switch self {
            case .appGroup: "App Group defaults (\(ThemeStore.appGroupID)), shared with the widgets"
            case .standard: "standard defaults (no App Group), not shared with the widgets"
            }
        }
    }

    /// The App Group the app and its widgets share.
    public static let appGroupID = "group.dev.savery.underbite"
    /// The chosen theme's id.
    public static let themeKey = "colors.theme"
    /// The custom themes, as a JSON array.
    public static let customThemesKey = "colors.custom_themes"

    /// Where this store keeps the themes.
    public let backing: Backing
    private let defaults: UserDefaults

    /// A store over `defaults` (the tests' own suite, say), which is `backing`.
    public init(defaults: UserDefaults, backing: Backing) {
        self.defaults = defaults
        self.backing = backing
    }

    /// The app's and the widgets' store: the App Group's defaults when the group is there,
    /// else the standard defaults.
    public static let shared: ThemeStore = {
        let fileManager = FileManager.default
        let container = fileManager.containerURL(forSecurityApplicationGroupIdentifier: ThemeStore.appGroupID)
        let found = ThemeStore.backing(groupContainer: container) { url in
            var isDirectory: ObjCBool = false
            return fileManager.fileExists(atPath: url.path(percentEncoded: false), isDirectory: &isDirectory)
                && isDirectory.boolValue
        }
        if found == .appGroup, let group = UserDefaults(suiteName: ThemeStore.appGroupID) {
            return ThemeStore(defaults: group, backing: .appGroup)
        }
        return ThemeStore(defaults: .standard, backing: .standard)
    }()

    /// Whether the App Group is there, from its container: iOS answers `nil` for a group the
    /// app is not entitled to; macOS always answers a URL, which only exists when it is.
    static func backing(groupContainer: URL?, containerExists: (URL) -> Bool) -> Backing {
        guard let groupContainer else { return .standard }
        #if os(macOS)
            return containerExists(groupContainer) ? .appGroup : .standard
        #else
            return .appGroup
        #endif
    }

    /// The chosen theme's id: a preset's or a custom theme's (the default preset until one is
    /// chosen).
    public var themeID: String {
        get { defaults.string(forKey: Self.themeKey) ?? ColorPresets.defaultID }
        set { defaults.set(newValue, forKey: Self.themeKey) }
    }

    /// The saved custom themes, as loaded (`CustomThemes.loaded`): a theme that does not parse
    /// is left out and the others kept; unknown roles are dropped and missing ones filled from
    /// the base when the theme is resolved.
    public var customThemes: [ColorTheme] {
        get {
            guard let data = defaults.data(forKey: Self.customThemesKey),
                let saved = try? JSONDecoder().decode([Lossy].self, from: data)
            else { return [] }
            return CustomThemes.loaded(saved.compactMap(\.theme))
        }
        set {
            let encoder = JSONEncoder()
            encoder.outputFormatting = [.sortedKeys]
            // A theme is strings all the way down, so this does not fail; if it did, the saved
            // themes stay as they were.
            guard let data = try? encoder.encode(newValue) else { return }
            defaults.set(data, forKey: Self.customThemesKey)
        }
    }

    /// Saves `theme` (replacing the custom theme with its id), keeping only the roles whose
    /// colour differs from its base preset.
    public func save(_ theme: ColorTheme) {
        let base = ColorPresets.preset(theme.base) ?? ColorPresets.underbite
        let small = theme.trimmed(against: base)
        var themes = customThemes
        if let index = themes.firstIndex(where: { $0.id == theme.id }) {
            themes[index] = small
        } else {
            themes.append(small)
        }
        customThemes = themes
    }

    /// Deletes the custom theme `id`; when it was the chosen one, its base preset is chosen.
    public func delete(id: String) {
        var themes = customThemes
        guard let index = themes.firstIndex(where: { $0.id == id }) else { return }
        let removed = themes.remove(at: index)
        customThemes = themes
        if themeID == id {
            themeID = ColorPresets.preset(removed.base)?.id ?? ColorPresets.defaultID
        }
    }

    /// The theme to paint with now (`ColorPresets.resolve`).
    public func resolved(sunMode: Bool = false) -> ColorTheme {
        ColorPresets.resolve(id: themeID, custom: customThemes, sunMode: sunMode)
    }

    /// One saved theme, or `nil` when it does not parse, so one bad entry never loses the rest.
    private struct Lossy: Decodable {
        let theme: ColorTheme?

        init(from decoder: Decoder) throws {
            theme = try? ColorTheme(from: decoder)
        }
    }
}
