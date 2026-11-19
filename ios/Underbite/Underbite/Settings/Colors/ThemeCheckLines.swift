import Foundation
import UnderbiteKit

/// One check a theme falls short of, in words: the role (or the pair of roles) and what is
/// wrong, with light and dark appearance, and the colour visions, said once.
nonisolated struct ThemeCheckLine: Identifiable, Equatable {
    let id: String
    /// The roles it is about: one, or the pair that is too close.
    let roles: [ColorRole]
    let text: String
}

/// The editor's warnings: `ThemeChecks.failures` for a theme as lines. They warn and never
/// block. The contrast ratios here measure the theme's own colours, not a reading, so they
/// are written here and never pass through the core. The package tests compile this file too.
nonisolated enum ThemeCheckLines {
    /// What `theme` falls short of at its level (high for the sun preset and themes based on
    /// it), in the order of the roles.
    static func lines(_ theme: ColorTheme, locale: Locale = .current) -> [ThemeCheckLine] {
        lines(ThemeChecks.failures(theme), locale: locale)
    }

    /// The roles `lines` are about.
    static func roles(_ lines: [ThemeCheckLine]) -> Set<ColorRole> {
        Set(lines.flatMap(\.roles))
    }

    /// What a check measures, without the vision an apart check simulates.
    private enum Kind: Int, Comparable {
        case contrast, fillText, fillVisible, apart

        static func < (a: Kind, b: Kind) -> Bool { a.rawValue < b.rawValue }
    }

    /// One role (and the other of a pair) under one kind of check.
    private struct Key: Hashable {
        let role: ColorRole
        let kind: Kind
        let other: ColorRole?
    }

    /// The failures under one key: the appearances, the visions (apart only, `nil` for
    /// typical), the lowest value and the minimum.
    private struct Gathered {
        var light = false
        var dark = false
        var visions: [Deficiency?] = []
        var lowest = Double.infinity
        var minimum = 0.0
    }

    /// `failures` as lines, one per role and check.
    static func lines(_ failures: [ThemeChecks.Finding], locale: Locale = .current) -> [ThemeCheckLine] {
        var gathered: [Key: Gathered] = [:]
        for finding in failures where !finding.passes {
            let key: Key
            var vision: Deficiency?
            switch finding.check {
            case .contrast: key = Key(role: finding.role, kind: .contrast, other: nil)
            case .fillText: key = Key(role: finding.role, kind: .fillText, other: nil)
            case .fillVisible: key = Key(role: finding.role, kind: .fillVisible, other: nil)
            case .apart(let other, let deficiency):
                key = Key(role: finding.role, kind: .apart, other: other)
                vision = deficiency
            }
            var g = gathered[key] ?? Gathered()
            if finding.dark { g.dark = true } else { g.light = true }
            if key.kind == .apart, !g.visions.contains(vision) { g.visions.append(vision) }
            g.lowest = min(g.lowest, finding.value)
            g.minimum = finding.minimum
            gathered[key] = g
        }
        let order = Dictionary(uniqueKeysWithValues: ColorRole.allCases.enumerated().map { ($1, $0) })
        let keys = gathered.keys.sorted { a, b in
            let ia = order[a.role] ?? 0, ib = order[b.role] ?? 0
            if ia != ib { return ia < ib }
            if a.kind != b.kind { return a.kind < b.kind }
            return (a.other.flatMap { order[$0] } ?? -1) < (b.other.flatMap { order[$0] } ?? -1)
        }
        return keys.compactMap { key in
            guard let g = gathered[key] else { return nil }
            return ThemeCheckLine(
                id: [key.role.rawValue, "\(key.kind.rawValue)", key.other?.rawValue ?? ""].joined(separator: "|"),
                roles: [key.role] + (key.other.map { [$0] } ?? []),
                text: text(key, g, locale: locale))
        }
    }

    /// One whole sentence per kind of check and appearance, so a language can order the words
    /// as it needs: keys with positional arguments, formatted here.
    private static func text(_ key: Key, _ g: Gathered, locale: Locale) -> String {
        enum Appearance { case light, dark, both }
        let appearance: Appearance = g.light && g.dark ? .both : g.dark ? .dark : .light
        let role = key.role
        let measured = [ratio(g.lowest, locale: locale), minimum(g.minimum, locale: locale)]
        let args: [CVarArg]
        switch key.kind {
        case .contrast, .fillText: args = [role.title, role.group.title] + measured
        case .fillVisible: args = [role.title, role.group.title]
        case .apart:
            let pair = key.other.map { joined(role.title, $0.title) } ?? role.title
            args = [pair, role.group.title, visions(g.visions)]
        }
        switch key.kind {
        case .contrast:
            switch role.kind {
            case .text:
                switch appearance {
                case .light:
                    return String(
                        format: NSLocalizedString(
                            "%1$@ (%2$@): words at %3$@ in light appearance, below %4$@", bundle: .main,
                            comment: "Colour check warning: the contrast of text, in light appearance. %1$@ is the colour role, %2$@ its group. %3$@ is the lowest contrast ratio measured and %4$@ the ratio it should reach."),
                        arguments: args)
                case .dark:
                    return String(
                        format: NSLocalizedString(
                            "%1$@ (%2$@): words at %3$@ in dark appearance, below %4$@", bundle: .main,
                            comment: "Colour check warning: the contrast of text, in dark appearance. %1$@ is the colour role, %2$@ its group. %3$@ is the lowest contrast ratio measured and %4$@ the ratio it should reach."),
                        arguments: args)
                case .both:
                    return String(
                        format: NSLocalizedString(
                            "%1$@ (%2$@): words at %3$@ in light and dark appearance, below %4$@", bundle: .main,
                            comment: "Colour check warning: the contrast of text, in light and dark appearance. %1$@ is the colour role, %2$@ its group. %3$@ is the lowest contrast ratio measured and %4$@ the ratio it should reach."),
                        arguments: args)
                }
            case .pin:
                switch appearance {
                case .light:
                    return String(
                        format: NSLocalizedString(
                            "%1$@ (%2$@): the pin's symbol at %3$@ in light appearance, below %4$@", bundle: .main,
                            comment: "Colour check warning: the contrast of a map pin's symbol, in light appearance. %1$@ is the colour role, %2$@ its group. %3$@ is the lowest contrast ratio measured and %4$@ the ratio it should reach."),
                        arguments: args)
                case .dark:
                    return String(
                        format: NSLocalizedString(
                            "%1$@ (%2$@): the pin's symbol at %3$@ in dark appearance, below %4$@", bundle: .main,
                            comment: "Colour check warning: the contrast of a map pin's symbol, in dark appearance. %1$@ is the colour role, %2$@ its group. %3$@ is the lowest contrast ratio measured and %4$@ the ratio it should reach."),
                        arguments: args)
                case .both:
                    return String(
                        format: NSLocalizedString(
                            "%1$@ (%2$@): the pin's symbol at %3$@ in light and dark appearance, below %4$@", bundle: .main,
                            comment: "Colour check warning: the contrast of a map pin's symbol, in light and dark appearance. %1$@ is the colour role, %2$@ its group. %3$@ is the lowest contrast ratio measured and %4$@ the ratio it should reach."),
                        arguments: args)
                }
            case .mark, .fill:
                switch appearance {
                case .light:
                    return String(
                        format: NSLocalizedString(
                            "%1$@ (%2$@): marks at %3$@ in light appearance, below %4$@", bundle: .main,
                            comment: "Colour check warning: the contrast of marks, in light appearance. %1$@ is the colour role, %2$@ its group. %3$@ is the lowest contrast ratio measured and %4$@ the ratio it should reach."),
                        arguments: args)
                case .dark:
                    return String(
                        format: NSLocalizedString(
                            "%1$@ (%2$@): marks at %3$@ in dark appearance, below %4$@", bundle: .main,
                            comment: "Colour check warning: the contrast of marks, in dark appearance. %1$@ is the colour role, %2$@ its group. %3$@ is the lowest contrast ratio measured and %4$@ the ratio it should reach."),
                        arguments: args)
                case .both:
                    return String(
                        format: NSLocalizedString(
                            "%1$@ (%2$@): marks at %3$@ in light and dark appearance, below %4$@", bundle: .main,
                            comment: "Colour check warning: the contrast of marks, in light and dark appearance. %1$@ is the colour role, %2$@ its group. %3$@ is the lowest contrast ratio measured and %4$@ the ratio it should reach."),
                        arguments: args)
                }
            }
        case .fillText:
            switch appearance {
            case .light:
                return String(
                    format: NSLocalizedString(
                        "%1$@ (%2$@): words on it at %3$@ in light appearance, below %4$@", bundle: .main,
                        comment: "Colour check warning: the contrast of text drawn on a colour, in light appearance. %1$@ is the colour role, %2$@ its group. %3$@ is the lowest contrast ratio measured and %4$@ the ratio it should reach."),
                    arguments: args)
            case .dark:
                return String(
                    format: NSLocalizedString(
                        "%1$@ (%2$@): words on it at %3$@ in dark appearance, below %4$@", bundle: .main,
                        comment: "Colour check warning: the contrast of text drawn on a colour, in dark appearance. %1$@ is the colour role, %2$@ its group. %3$@ is the lowest contrast ratio measured and %4$@ the ratio it should reach."),
                    arguments: args)
            case .both:
                return String(
                    format: NSLocalizedString(
                        "%1$@ (%2$@): words on it at %3$@ in light and dark appearance, below %4$@", bundle: .main,
                        comment: "Colour check warning: the contrast of text drawn on a colour, in light and dark appearance. %1$@ is the colour role, %2$@ its group. %3$@ is the lowest contrast ratio measured and %4$@ the ratio it should reach."),
                    arguments: args)
            }
        case .fillVisible:
            switch appearance {
            case .light:
                return String(
                    format: NSLocalizedString(
                        "%1$@ (%2$@): hard to see against the background in light appearance", bundle: .main,
                        comment: "Colour check warning: a colour that is hard to see against the background, in light appearance. %1$@ is the colour role, %2$@ its group."),
                    arguments: args)
            case .dark:
                return String(
                    format: NSLocalizedString(
                        "%1$@ (%2$@): hard to see against the background in dark appearance", bundle: .main,
                        comment: "Colour check warning: a colour that is hard to see against the background, in dark appearance. %1$@ is the colour role, %2$@ its group."),
                    arguments: args)
            case .both:
                return String(
                    format: NSLocalizedString(
                        "%1$@ (%2$@): hard to see against the background in light and dark appearance", bundle: .main,
                        comment: "Colour check warning: a colour that is hard to see against the background, in light and dark appearance. %1$@ is the colour role, %2$@ its group."),
                    arguments: args)
            }
        case .apart:
            switch appearance {
            case .light:
                return String(
                    format: NSLocalizedString(
                        "%1$@ (%2$@): too close to tell apart with %3$@ vision in light appearance", bundle: .main,
                        comment: "Colour check warning: colours too close to tell apart, in light appearance. %1$@ is the colour role, %2$@ its group. %1$@ may name two roles; %2$@ is the group of the first and %3$@ the kinds of colour vision."),
                    arguments: args)
            case .dark:
                return String(
                    format: NSLocalizedString(
                        "%1$@ (%2$@): too close to tell apart with %3$@ vision in dark appearance", bundle: .main,
                        comment: "Colour check warning: colours too close to tell apart, in dark appearance. %1$@ is the colour role, %2$@ its group. %1$@ may name two roles; %2$@ is the group of the first and %3$@ the kinds of colour vision."),
                    arguments: args)
            case .both:
                return String(
                    format: NSLocalizedString(
                        "%1$@ (%2$@): too close to tell apart with %3$@ vision in light and dark appearance", bundle: .main,
                        comment: "Colour check warning: colours too close to tell apart, in light and dark appearance. %1$@ is the colour role, %2$@ its group. %1$@ may name two roles; %2$@ is the group of the first and %3$@ the kinds of colour vision."),
                    arguments: args)
            }
        }
    }

    /// Two names joined with and: two colour roles, or the last two kinds of colour vision.
    private static func joined(_ first: String, _ second: String) -> String {
        String(
            format: NSLocalizedString(
                "%1$@ and %2$@", bundle: .main,
                comment: "Joins two nouns of one kind with and: the names of two colour roles, or of two kinds of colour vision"),
            first, second)
    }

    /// "typical", "deutan", "typical and deutan", "protan, deutan and tritan".
    private static func visions(_ visions: [Deficiency?]) -> String {
        let typical = String(localized: "typical", comment: "Name of typical colour vision in a colour check warning")
        let names = visions.sorted { rank($0) < rank($1) }.map { $0.map(name) ?? typical }
        guard let last = names.last else { return typical }
        guard names.count > 1 else { return last }
        let head = names.dropLast().dropFirst().reduce(names[0]) {
            String(
                format: NSLocalizedString(
                    "%1$@, %2$@", bundle: .main,
                    comment: "Joins two names in a list of kinds of colour vision"),
                $0, $1)
        }
        return joined(head, last)
    }

    private static func name(_ vision: Deficiency) -> String {
        switch vision {
        case .protan: String(localized: "protan", comment: "Protanopia, a kind of colour blindness, in a colour check warning")
        case .deutan: String(localized: "deutan", comment: "Deuteranopia, a kind of colour blindness, in a colour check warning")
        case .tritan: String(localized: "tritan", comment: "Tritanopia, a kind of colour blindness, in a colour check warning")
        case .achromat: String(localized: "achromat", comment: "Seeing no colour, in a colour check warning")
        }
    }

    private static func rank(_ vision: Deficiency?) -> Int {
        guard let vision else { return -1 }
        return Deficiency.allCases.firstIndex(of: vision) ?? 0
    }

    /// A measured contrast ratio, one decimal rounded down ("4.4:1", never "4.5:1" for a
    /// ratio that falls short of 4.5).
    static func ratio(_ value: Double, locale: Locale) -> String {
        value.formatted(.number.precision(.fractionLength(1)).rounded(rule: .down).locale(locale)) + ":1"
    }

    /// A threshold: "3:1", "4.5:1".
    private static func minimum(_ value: Double, locale: Locale) -> String {
        value.formatted(.number.precision(.fractionLength(0...1)).locale(locale)) + ":1"
    }
}
