import Foundation

/// The reader's own colour themes: made from a preset or from another custom theme, and saved
/// small, with only the roles whose colour differs from the base preset. The base fills the
/// rest when the theme is resolved (`ColorPresets.completed`), so a role added later reaches
/// custom themes too.
public enum CustomThemes {
    /// Custom theme ids start with this, so they never meet a preset's.
    public static let idPrefix = "custom-"

    /// A new custom theme from `theme`: "My <Preset>" for a preset, "<Name> 2" for a custom
    /// theme (or the next free number), with the same base and colours.
    public static func duplicate(
        _ theme: ColorTheme, existing: [ColorTheme], id: String = idPrefix + UUID().uuidString.lowercased()
    ) -> ColorTheme {
        let isCustom = existing.contains { $0.id == theme.id }
        let name = isCustom ? theme.name : "My \(theme.name)"
        return ColorTheme(
            id: id, name: uniqueName(name, among: existing), base: theme.base,
            colors: isCustom ? theme.colors : [:])
    }

    /// `name`, or with the first free number after it ("My Sun 2") when another custom theme
    /// has it already. A number it ends with is counted on from.
    public static func uniqueName(_ name: String, among existing: [ColorTheme]) -> String {
        let taken = Set(existing.map(\.name))
        guard taken.contains(name) else { return name }
        var stem = name
        var n = 2
        if let space = name.lastIndex(of: " "), let number = Int(name[name.index(after: space)...]) {
            stem = String(name[..<space])
            n = number + 1
        }
        while taken.contains("\(stem) \(n)") { n += 1 }
        return "\(stem) \(n)"
    }

    /// A name typed for a theme, trimmed; `nil` when nothing is left.
    public static func cleanName(_ text: String) -> String? {
        let name = text.trimmingCharacters(in: .whitespacesAndNewlines)
        return name.isEmpty ? nil : name
    }

    /// Saved custom themes as loaded: each once (the first of an id), never one without an id
    /// or with a preset's id (the preset would win).
    public static func loaded(_ themes: [ColorTheme]) -> [ColorTheme] {
        var seen = Set<String>()
        return themes.filter { theme in
            !theme.id.isEmpty && ColorPresets.preset(theme.id) == nil && seen.insert(theme.id).inserted
        }
    }
}

extension ColorTheme {
    /// Only the roles whose colour differs from `base`'s: what a custom theme saves.
    public func trimmed(against base: ColorTheme) -> ColorTheme {
        var out = self
        out.colors = colors.filter { role, color in base.colors[role] != color }
        return out
    }

    /// This custom theme with `role` painted `color`; a colour equal to the base's is dropped,
    /// so the role follows the base again.
    public func setting(_ role: ColorRole, to color: ThemeColor, base: ColorTheme) -> ColorTheme {
        var out = self
        out.colors[role] = base.colors[role] == color ? nil : color
        return out
    }

    /// Whether this custom theme paints `role` differently from `base`.
    public func differs(_ role: ColorRole, from base: ColorTheme) -> Bool {
        colors[role].map { $0 != base.colors[role] } ?? false
    }

    /// This theme as a reader with `deficiency` sees it, every colour passed through the
    /// simulation: the editor's "Preview as".
    public func simulated(as deficiency: Deficiency) -> ColorTheme {
        var out = self
        out.colors = colors.mapValues { color in
            ThemeColor(
                light: ColorMath.simulate(color.light, deficiency),
                dark: color.dark.map { ColorMath.simulate($0, deficiency) })
        }
        return out
    }
}
