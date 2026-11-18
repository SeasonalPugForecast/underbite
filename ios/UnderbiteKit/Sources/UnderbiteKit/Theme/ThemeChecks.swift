import Foundation

/// What a theme's colours are measured against, in light and dark appearance. Every preset
/// passes at its level (tested); the editor shows the same measurements for a custom theme as
/// warnings, never as a block.
///
/// The rules, by how a role is painted (`ColorRole.Kind`), on each background of the
/// appearance (light: white and the grouped page `#F2F2F7`; dark: black and the card
/// `#1C1C1E`), the lowest counting:
/// - **text**: WCAG contrast of the colour against the background, at least `Level.text`
///   (4.5:1, WCAG 1.4.3 AA; 7:1 in sun mode, AAA).
/// - **mark**: the same, at least `Level.mark` (3:1, WCAG 1.4.11; 4.5:1 in sun mode).
/// - **fill**: primary text (black in light, white in dark) on the span painted over the
///   background, at least `Level.fillText`; and the span standing off the background by at
///   least `Level.fillVisible` ΔE_OK.
/// - **pin**: a map pin's symbol, white or `#1B1B1F` whichever stands out more, on its fill,
///   at least `Level.mark`.
/// - **apart**: each neighbouring pair of an ordered set (flow classes, flood stages, a
///   gauge's station states) and each pair of a set drawn together (chart lines, chart rules,
///   high and low tide, fate tags, alerts, verdicts, station pins) at least `Level.apart`
///   ΔE_OK with typical vision and `Level.apartColorBlind` as protanopia, deuteranopia and
///   tritanopia simulate them.
public enum ThemeChecks {
    /// The thresholds a theme is held to.
    public struct Level: Equatable, Sendable {
        public var text: Double
        public var mark: Double
        public var fillText: Double
        public var fillVisible: Double
        public var apart: Double
        public var apartColorBlind: Double

        /// Every theme.
        public static let standard = Level(
            text: 4.5, mark: 3, fillText: 4.5, fillVisible: 0.03, apart: 0.06, apartColorBlind: 0.04)
        /// The sun preset and themes based on it: for glare.
        public static let high = Level(
            text: 7, mark: 4.5, fillText: 7, fillVisible: 0.05, apart: 0.06, apartColorBlind: 0.04)
    }

    /// The level `theme` is held to: high for the sun preset and themes based on it.
    public static func level(for theme: ColorTheme) -> Level {
        theme.base == ColorPresets.sunID ? .high : .standard
    }

    /// The backgrounds in light appearance: a card (white) and the grouped page.
    public static let lightBackgrounds = [RGBA(rgb: 0xFFFFFF), RGBA(rgb: 0xF2F2F7)]
    /// The backgrounds in dark appearance: the page (black) and a card.
    public static let darkBackgrounds = [RGBA(rgb: 0x000000), RGBA(rgb: 0x1C1C1E)]
    /// The dark symbol on a light pin.
    public static let pinInk = RGBA(rgb: 0x1B1B1F)
    /// The colour-vision deficiencies the apart check simulates.
    public static let deficiencies: [Deficiency] = [.protan, .deutan, .tritan]

    /// One measurement.
    public struct Finding: Equatable, Sendable {
        public enum Check: Sendable, Equatable {
            /// Text or a mark on the background, or a pin's symbol on the pin.
            case contrast
            /// Primary text on a shaded span.
            case fillText
            /// A shaded span against the background (ΔE_OK).
            case fillVisible
            /// Two roles apart (ΔE_OK), as `deficiency` sees them (typical vision for `nil`).
            case apart(ColorRole, deficiency: Deficiency?)
        }

        public var role: ColorRole
        public var check: Check
        public var dark: Bool
        public var value: Double
        public var minimum: Double
        public var passes: Bool { value >= minimum }
    }

    /// The pairs the apart check measures.
    public static let apartPairs: [(ColorRole, ColorRole)] = {
        var pairs: [(ColorRole, ColorRole)] = []
        for ramp in [ColorRole.flowClasses, ColorRole.floodStages, ColorRole.stationSteps] {
            pairs += zip(ramp, ramp.dropFirst()).map { ($0, $1) }
        }
        let sets: [[ColorRole]] = [
            ColorRole.chartLines, ColorRole.chartRules, [.tideHigh, .tideLow], ColorRole.fates,
            [.alertSevere, .alertModerate], ColorRole.verdicts,
            [.stationNone, .stationReporting] + ColorRole.stationSteps,
        ]
        for set in sets {
            for i in set.indices { for j in set.indices where j > i { pairs.append((set[i], set[j])) } }
        }
        return pairs
    }()

    /// Every measurement of `theme` at `level`, light then dark.
    public static func findings(_ theme: ColorTheme, level: Level) -> [Finding] {
        var out: [Finding] = []
        for dark in [false, true] {
            let backgrounds = dark ? darkBackgrounds : lightBackgrounds
            let primary: RGBA = dark ? .white : .black
            for role in ColorRole.allCases {
                let color = theme[role].resolved(dark: dark)
                switch role.kind {
                case .text, .mark:
                    let ratio = backgrounds.map { ColorMath.contrast(ColorMath.over(color, $0), $0) }.min() ?? 0
                    out.append(
                        Finding(
                            role: role, check: .contrast, dark: dark, value: ratio,
                            minimum: role.kind == .text ? level.text : level.mark))
                case .fill:
                    let onFill = backgrounds.map { ColorMath.contrast(primary, ColorMath.over(color, $0)) }.min() ?? 0
                    let visible = backgrounds.map { ColorMath.deltaE(ColorMath.over(color, $0), $0) }.min() ?? 0
                    out.append(Finding(role: role, check: .fillText, dark: dark, value: onFill, minimum: level.fillText))
                    out.append(
                        Finding(role: role, check: .fillVisible, dark: dark, value: visible, minimum: level.fillVisible))
                case .pin:
                    let fill = ColorMath.over(color, backgrounds[1])
                    let ratio = max(ColorMath.contrast(fill, .white), ColorMath.contrast(fill, pinInk))
                    out.append(Finding(role: role, check: .contrast, dark: dark, value: ratio, minimum: level.mark))
                }
            }
            for (a, b) in apartPairs {
                let x = ColorMath.over(theme[a].resolved(dark: dark), backgrounds[1])
                let y = ColorMath.over(theme[b].resolved(dark: dark), backgrounds[1])
                out.append(
                    Finding(
                        role: a, check: .apart(b, deficiency: nil), dark: dark,
                        value: ColorMath.deltaE(x, y), minimum: level.apart))
                for deficiency in deficiencies {
                    out.append(
                        Finding(
                            role: a, check: .apart(b, deficiency: deficiency), dark: dark,
                            value: ColorMath.deltaE(x, y, as: deficiency), minimum: level.apartColorBlind))
                }
            }
        }
        return out
    }

    /// The measurements of `theme` that fall short of its level (`level(for:)` by default).
    public static func failures(_ theme: ColorTheme, level: Level? = nil) -> [Finding] {
        findings(theme, level: level ?? self.level(for: theme)).filter { !$0.passes }
    }
}
