import Foundation
import Observation
import UnderbiteKit

/// The chosen colour theme and the custom themes, as the app's views see them: read once from
/// `ThemeStore` and written back on every change, so the app root, the Colours page and the
/// editor redraw together. The widgets read the same store and paint with a change on their
/// next timeline. Custom themes are saved small: only the roles that differ from the base
/// preset. The package tests compile this file too.
@MainActor
@Observable
final class AppTheme {
    /// The one model of the process.
    #if DEBUG
        static let shared = AppTheme(store: launchStore(ProcessInfo.processInfo.arguments))
    #else
        static let shared = AppTheme(store: .shared)
    #endif

    /// The chosen theme's id: a preset's or a custom theme's.
    private(set) var themeID: String
    /// The custom themes, in the reader's order, as saved (without their base's colours).
    private(set) var customThemes: [ColorTheme]

    private let store: ThemeStore

    init(store: ThemeStore) {
        self.store = store
        themeID = store.themeID
        customThemes = store.customThemes
    }

    /// Where the themes are kept: the App Group's defaults, or the app's own when the build has
    /// no App Group (the widgets then keep the default colours).
    var backing: ThemeStore.Backing { store.backing }

    /// The theme to paint with (`ColorPresets.resolve`): in sun mode, the sun preset unless the
    /// chosen theme is based on it.
    func resolved(sunMode: Bool) -> ColorTheme {
        ColorPresets.resolve(id: themeID, custom: customThemes, sunMode: sunMode)
    }

    /// The custom theme `id` as saved, or `nil`.
    func custom(_ id: String) -> ColorTheme? {
        customThemes.first { $0.id == id }
    }

    /// The preset `theme` is based on (the default preset for an unknown base).
    static func base(of theme: ColorTheme) -> ColorTheme {
        ColorPresets.preset(theme.base) ?? ColorPresets.underbite
    }

    /// Chooses the preset or custom theme `id`; an id that is neither is ignored.
    func choose(_ id: String) {
        guard ColorPresets.preset(id) != nil || custom(id) != nil, id != themeID else { return }
        themeID = id
        store.themeID = id
    }

    /// A new custom theme from the preset `id` ("My Underbite"), chosen; `nil` for an unknown
    /// preset.
    @discardableResult
    func customize(preset id: String) -> ColorTheme? {
        guard let preset = ColorPresets.preset(id) else { return nil }
        let theme = CustomThemes.duplicate(preset, existing: customThemes)
        customThemes.append(theme)
        save()
        choose(theme.id)
        return theme
    }

    /// A copy of the custom theme `id` ("Mine 2"), listed after it; `nil` when there is none.
    @discardableResult
    func duplicate(_ id: String) -> ColorTheme? {
        guard let index = customThemes.firstIndex(where: { $0.id == id }) else { return nil }
        let copy = CustomThemes.duplicate(customThemes[index], existing: customThemes)
        customThemes.insert(copy, at: index + 1)
        save()
        return copy
    }

    /// Names the custom theme `id` with `text`, trimmed and numbered on when another theme has
    /// the name; `false` when nothing is left of it or there is no such theme.
    @discardableResult
    func rename(_ id: String, to text: String) -> Bool {
        guard let name = CustomThemes.cleanName(text),
            let index = customThemes.firstIndex(where: { $0.id == id })
        else { return false }
        guard customThemes[index].name != name else { return true }
        let others = customThemes.filter { $0.id != id }
        customThemes[index].name = CustomThemes.uniqueName(name, among: others)
        save()
        return true
    }

    /// Deletes the custom theme `id`; while it was chosen, its base preset is chosen instead.
    func delete(_ id: String) {
        guard let theme = custom(id) else { return }
        customThemes.removeAll { $0.id == id }
        save()
        if themeID == id {
            themeID = Self.base(of: theme).id
            store.themeID = themeID
        }
    }

    /// Paints `role` of the custom theme `id` with `color`; the base's own colour lets the
    /// role follow the base again.
    func set(_ role: ColorRole, to color: ThemeColor, in id: String) {
        update(id) { $0 = $0.setting(role, to: color, base: Self.base(of: $0)) }
    }

    /// Gives `role` of the custom theme `id` back to its base.
    func reset(_ role: ColorRole, in id: String) {
        update(id) { $0.colors[role] = nil }
    }

    /// Gives every role of the custom theme `id` back to its base.
    func restoreDefaults(_ id: String) {
        update(id) { $0.colors = [:] }
    }

    private func update(_ id: String, _ change: (inout ColorTheme) -> Void) {
        guard let index = customThemes.firstIndex(where: { $0.id == id }) else { return }
        var theme = customThemes[index]
        change(&theme)
        guard theme != customThemes[index] else { return }
        customThemes[index] = theme
        save()
    }

    /// Writes the custom themes, each trimmed to what differs from its base.
    private func save() {
        store.customThemes = customThemes.map { $0.trimmed(against: Self.base(of: $0)) }
    }

    /// The store of a launch: the shared one, or in debug builds a launch-only one, emptied
    /// first, for a screenshot that makes a custom theme (`-customTheme <preset>`,
    /// `-editTheme <preset>`), so screenshots never leave themes behind.
    nonisolated static func launchStore(_ arguments: [String]) -> ThemeStore {
        #if DEBUG
            let launchOnly = "dev.savery.underbite.launch-themes"
            if arguments.contains("-customTheme") || arguments.contains("-editTheme"),
                let defaults = UserDefaults(suiteName: launchOnly)
            {
                defaults.removePersistentDomain(forName: launchOnly)
                return ThemeStore(defaults: defaults, backing: .standard)
            }
        #endif
        return ThemeStore.shared
    }
}
