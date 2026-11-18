import Foundation

/// The colour presets. Presets are data: each is a full `ColorTheme`, every role defined,
/// light and dark where they differ. `ColorPresetsTests` checks every one against
/// `ThemeChecks`: the default at the standard level, the sun preset at the high one.
public enum ColorPresets {
    /// The preset everyone starts with.
    public static let defaultID = "underbite"
    /// The high-contrast preset sun mode paints with.
    public static let sunID = "sun"

    /// The gallery's presets, in order.
    public static let all: [ColorTheme] = [underbite, sun]

    /// Every preset's id, in the gallery's order.
    public static var ids: [String] { all.map(\.id) }

    /// The preset with `id`, or `nil`.
    public static func preset(_ id: String) -> ColorTheme? {
        all.first { $0.id == id }
    }

    /// `theme` with every role it lacks taken from its base preset (the default when the base
    /// is unknown): how a saved custom theme is loaded.
    public static func completed(_ theme: ColorTheme) -> ColorTheme {
        theme.completed(from: preset(theme.base) ?? underbite)
    }

    /// The theme the app and the widgets paint with: the preset or custom theme `id` (the
    /// default when it is neither), completed from its base. In sun mode, the sun preset,
    /// unless the theme is the sun preset or a custom theme based on it.
    public static func resolve(id: String, custom: [ColorTheme] = [], sunMode: Bool = false) -> ColorTheme {
        let theme =
            preset(id)
            ?? custom.first(where: { $0.id == id }).map(completed)
            ?? underbite
        if sunMode && theme.base != sunID { return sun }
        return theme
    }

    private static func c(_ light: UInt32, _ dark: UInt32? = nil) -> ThemeColor {
        ThemeColor(light: RGBA(rgb: light), dark: dark.map { RGBA(rgb: $0) })
    }

    /// A translucent span: `0xRRGGBBAA` in light and in dark appearance.
    private static func translucent(_ light: UInt32, _ dark: UInt32) -> ThemeColor {
        ThemeColor(light: RGBA(rgba: light), dark: RGBA(rgba: dark))
    }

    // MARK: - Presets

    /// The default, safe for colour-blind eyes: each ordered set steps in lightness as well as
    /// hue, and the colours drawn together stay apart under protanopia, deuteranopia and
    /// tritanopia. Flow runs brown (low) through grey (normal) to blue (high); go is teal and
    /// no-go crimson, never green against red. Station pins are the Okabe–Ito set.
    public static let underbite = ColorTheme(
        id: "underbite", name: "Underbite",
        colors: [
            // Text and accent
            .accent: c(0x0066D9, 0x0A84FF), .statusWarning: c(0xA84D00, 0xFF9F0A),
            .statusError: c(0xBF0000, 0xFF453A), .statusSuccess: c(0x007326, 0x30D158),
            // Card states
            .cardFresh: c(0x007326, 0x30D158), .cardStale: c(0xA84D00, 0xFF9F0A),
            .cardFar: c(0xA84D00, 0xFF9F0A), .cardUnavailable: c(0x636366, 0xAEAEB2),
            // Go / no-go
            .verdictGo: c(0x006E7A, 0x4FD1DB), .verdictMarginal: c(0x975A00, 0xFFC247),
            .verdictNoGo: c(0xB8004F, 0xFF7AA8), .verdictUnknown: c(0x2C2C2E, 0xE5E5EA),
            // Alerts
            .alertSevere: c(0xB8004F, 0xFF7AA8), .alertModerate: c(0xA84D00, 0xFF9F0A),
            // Catch log
            .fateReleased: c(0x007326, 0x30D158), .fateKept: c(0x0066D9, 0x5AA9FF),
            .fateKeptForSashimi: c(0xB8336A, 0xFF8FB8),
            // River flow
            .flowRecordLow: c(0x6B2E00, 0xE8A06A), .flowMuchBelowNormal: c(0x9C4A00, 0xF2B880),
            .flowBelowNormal: c(0xA3741A, 0xF5D9A0), .flowNormal: c(0x6B7268, 0xD8DDD5),
            .flowAboveNormal: c(0x3F7FB8, 0xA8D2F5), .flowMuchAboveNormal: c(0x1F5AA6, 0x79B4F0),
            .flowRecordHigh: c(0x1A2F78, 0x8E9CF5),
            // Flood stages
            .floodAction: c(0x6E5A00, 0xF0E442), .floodMinor: c(0xD35C00, 0xFFA94D),
            .floodModerate: c(0xB0001E, 0xFF6B6B), .floodMajor: c(0x7B1FA2, 0xD59BF6),
            // Station pins
            .stationNone: c(0x34435E), .stationUnknown: c(0x6B6B6B), .stationNoData: c(0x4A4A4A),
            .stationReporting: c(0x0072B2), .stationLow: c(0xE69F00), .stationNormal: c(0x009E73),
            .stationElevated: c(0xF0E442), .stationFloodMinor: c(0xD55E00), .stationFloodModerate: c(0xCC79A7),
            .stationFloodMajor: c(0x6A1B9A),
            // Tide
            .tideCurve: c(0x007AFF, 0x0A84FF), .tideFill: translucent(0x007AFF4D, 0x0A84FF4D),
            .tideHigh: c(0x0055C4, 0x64B5FF), .tideLow: c(0x00808F, 0x40D0D8),
            .tideSlack: translucent(0x8E8E934D, 0x8E8E9366),
            // Timeline
            .timelineNight: translucent(0x5856D638, 0x5E5CE666), .timelineSun: c(0xC25400, 0xFF9F0A),
            .timelineSolunarMajor: translucent(0x34C75959, 0x30D15866),
            .timelineSolunarMinor: translucent(0x34C7592E, 0x30D15833),
            .timelineWindow: translucent(0xFFCC004D, 0xFFD60A4D),
            // Charts
            .chartPredicted: c(0x007AFF, 0x0A84FF), .chartObserved: c(0xC25400, 0xFF9F0A),
            .chartForecast: c(0x00808F, 0x40D0D8), .chartModelled: c(0x6E6E73, 0x98989D),
            .chartFloodCategory: c(0xC2002F, 0xFF6961), .chartUserRange: c(0x0055C4, 0x64B5FF),
            .chartThreshold: c(0x3A3A3C, 0xD1D1D6), .chartValueBand: translucent(0x00C7BE29, 0x63E6E233),
            .chartNow: c(0xD70015, 0xFF453A),
            // Spots
            .spotFavourite: c(0xA87B00, 0xFFD60A), .spotPinned: c(0xC25400, 0xFF9F0A),
        ])

    /// Sun mode: the default's colours pushed to the high level for glare (words at 7:1, marks
    /// and pin symbols at 4.5:1, words on a shaded span at 7:1) and stronger spans, still apart
    /// for colour-blind eyes.
    public static let sun = ColorTheme(
        id: "sun", name: "Sun",
        colors: [
            // Text and accent
            .accent: c(0x0049B4, 0x68ABFF), .statusWarning: c(0x8C3300, 0xFF9F0A),
            .statusError: c(0xA60000, 0xFE887A), .statusSuccess: c(0x005E14, 0x30D158),
            // Card states
            .cardFresh: c(0x005E14, 0x30D158), .cardStale: c(0x8C3300, 0xFF9F0A),
            .cardFar: c(0x8C3300, 0xFF9F0A), .cardUnavailable: c(0x505052, 0xAEAEB2),
            // Go / no-go
            .verdictGo: c(0x005964, 0x4FD1DB), .verdictMarginal: c(0x7B4000, 0xFFC247),
            .verdictNoGo: c(0xA10445, 0xFF80AB), .verdictUnknown: c(0x2C2C2E, 0xE5E5EA),
            // Alerts
            .alertSevere: c(0xA10445, 0xFF80AB), .alertModerate: c(0x8C3300, 0xFF9F0A),
            // Catch log
            .fateReleased: c(0x005E14, 0x30D158), .fateKept: c(0x0049B4, 0x5DACFF),
            .fateKeptForSashimi: c(0x9B0F52, 0xFF8FB8),
            // River flow
            .flowRecordLow: c(0x4F1C00, 0xE8A06A), .flowMuchBelowNormal: c(0x763000, 0xF2B880),
            .flowBelowNormal: c(0x875D00, 0xF5D9A0), .flowNormal: c(0x666F64, 0xD8DDD5),
            .flowAboveNormal: c(0x1F689D, 0xA8D2F5), .flowMuchAboveNormal: c(0x14488F, 0x79B4F0),
            .flowRecordHigh: c(0x1B276C, 0x8E9CF5),
            // Flood stages
            .floodAction: c(0x4D3F00, 0xF0E442), .floodMinor: c(0xBB4600, 0xFFA94D),
            .floodModerate: c(0xB0001E, 0xFF6B6B), .floodMajor: c(0x7B1FA2, 0xD59BF6),
            // Station pins
            .stationNone: c(0x34435E), .stationUnknown: c(0x6B6B6B), .stationNoData: c(0x4A4A4A),
            .stationReporting: c(0x0072B2), .stationLow: c(0xE69F00), .stationNormal: c(0x009E73),
            .stationElevated: c(0xF0E442), .stationFloodMinor: c(0xDA630D), .stationFloodModerate: c(0xCC79A7),
            .stationFloodMajor: c(0x6A1B9A),
            // Tide
            .tideCurve: c(0x0065E8, 0x0A84FF), .tideFill: translucent(0x007AFF8B, 0x0A84FF80),
            .tideHigh: c(0x0055C4, 0x64B5FF), .tideLow: c(0x007786, 0x40D0D8),
            .tideSlack: translucent(0x8E8E938B, 0x8E8E9373),
            // Timeline
            .timelineNight: translucent(0x5856D665, 0x5E5CE699), .timelineSun: c(0xB84B00, 0xFF9F0A),
            .timelineSolunarMajor: translucent(0x34C759A0, 0x30D15861),
            .timelineSolunarMinor: translucent(0x34C75953, 0x30D15833),
            .timelineWindow: translucent(0xFFCC008B, 0xFFD60A4D),
            // Charts
            .chartPredicted: c(0x0065E8, 0x0A84FF), .chartObserved: c(0xB84B00, 0xFF9F0A),
            .chartForecast: c(0x007786, 0x40D0D8), .chartModelled: c(0x48484A, 0x98989D),
            .chartFloodCategory: c(0xC2002F, 0xFF6961), .chartUserRange: c(0x0055C4, 0x64B5FF),
            .chartThreshold: c(0x3A3A3C, 0xD1D1D6), .chartValueBand: translucent(0x00C7BE4A, 0x63E6E24D),
            .chartNow: c(0xD70015, 0xFF453A),
            // Spots
            .spotFavourite: c(0x916500, 0xFFD60A), .spotPinned: c(0xB84B00, 0xFF9F0A),
        ])
}
