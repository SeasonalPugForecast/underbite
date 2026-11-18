import Foundation

/// One role's colour: for light appearance, and optionally another for dark.
public struct ThemeColor: Hashable, Sendable, Codable {
    public var light: RGBA
    public var dark: RGBA?

    public init(light: RGBA, dark: RGBA? = nil) {
        self.light = light
        self.dark = dark
    }

    /// The colour in light (`dark == false`) or dark appearance.
    public func resolved(dark isDark: Bool) -> RGBA {
        isDark ? (dark ?? light) : light
    }

    private enum Keys: String, CodingKey { case light, dark }

    /// `{"light": "#RRGGBB", "dark": "#RRGGBBAA"}`, or just `"#RRGGBB"` for both.
    public init(from decoder: Decoder) throws {
        func color(_ text: String, _ path: [CodingKey]) throws -> RGBA {
            guard let c = RGBA(hex: text) else {
                throw DecodingError.dataCorrupted(.init(codingPath: path, debugDescription: "not a colour: \(text)"))
            }
            return c
        }
        if let text = try? decoder.singleValueContainer().decode(String.self) {
            self.init(light: try color(text, decoder.codingPath))
            return
        }
        let c = try decoder.container(keyedBy: Keys.self)
        let dark = try c.decodeIfPresent(String.self, forKey: .dark)
        self.init(
            light: try color(c.decode(String.self, forKey: .light), c.codingPath),
            dark: try dark.map { try color($0, c.codingPath) })
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: Keys.self)
        try c.encode(light.hex, forKey: .light)
        try c.encodeIfPresent(dark?.hex, forKey: .dark)
    }
}

/// A full set of colours: a preset (`ColorPresets`) or one of the reader's own, based on a
/// preset.
public struct ColorTheme: Identifiable, Hashable, Sendable, Codable {
    /// A preset's id (`underbite`, `sun`) or a custom theme's (`custom-…`).
    public var id: String
    public var name: String
    /// The preset that fills the roles this theme lacks; a preset's base is itself.
    public var base: String
    public var colors: [ColorRole: ThemeColor]

    public init(id: String, name: String, base: String? = nil, colors: [ColorRole: ThemeColor]) {
        self.id = id
        self.name = name
        self.base = base ?? id
        self.colors = colors
    }

    /// The colour for `role`. Saved themes are completed when loaded (`completed(from:)`), so
    /// the default preset's colour only stands in for a theme that was not.
    public subscript(role: ColorRole) -> ThemeColor {
        colors[role] ?? ColorPresets.underbite.colors[role] ?? ThemeColor(light: .black)
    }

    /// The roles this theme has no colour for.
    public var missing: [ColorRole] { ColorRole.allCases.filter { colors[$0] == nil } }

    /// This theme with every role it lacks taken from `base`: a role added in a later version
    /// never breaks a saved theme.
    public func completed(from base: ColorTheme) -> ColorTheme {
        var out = self
        out.colors.merge(base.colors) { mine, _ in mine }
        return out
    }

    private enum Keys: String, CodingKey { case id, name, base, colors }

    private struct RoleKey: CodingKey {
        var stringValue: String
        var intValue: Int? { nil }
        init(stringValue: String) { self.stringValue = stringValue }
        init?(intValue: Int) { nil }
    }

    /// Unknown roles and colours that do not parse are dropped, and the base fills them.
    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: Keys.self)
        let id = try c.decode(String.self, forKey: .id)
        var colors: [ColorRole: ThemeColor] = [:]
        if c.contains(.colors) {
            let roles = try c.nestedContainer(keyedBy: RoleKey.self, forKey: .colors)
            for key in roles.allKeys {
                guard let role = ColorRole(rawValue: key.stringValue),
                    let color = try? roles.decode(ThemeColor.self, forKey: key)
                else { continue }
                colors[role] = color
            }
        }
        self.init(
            id: id, name: try c.decodeIfPresent(String.self, forKey: .name) ?? id,
            base: try c.decodeIfPresent(String.self, forKey: .base) ?? ColorPresets.defaultID,
            colors: colors)
    }

    /// Roles by their raw values (the store also sorts keys, so a theme always writes the same
    /// bytes).
    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: Keys.self)
        try c.encode(id, forKey: .id)
        try c.encode(name, forKey: .name)
        try c.encode(base, forKey: .base)
        var roles = c.nestedContainer(keyedBy: RoleKey.self, forKey: .colors)
        for (role, color) in colors.sorted(by: { $0.key.rawValue < $1.key.rawValue }) {
            try roles.encode(color, forKey: RoleKey(stringValue: role.rawValue))
        }
    }
}
