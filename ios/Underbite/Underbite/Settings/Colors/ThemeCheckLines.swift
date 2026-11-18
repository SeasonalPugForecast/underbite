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

    private static func text(_ key: Key, _ g: Gathered, locale: Locale) -> String {
        let role = key.role
        let appearance =
            g.light && g.dark ? "in light and dark appearance" : g.dark ? "in dark appearance" : "in light appearance"
        let measured = "\(ratio(g.lowest, locale: locale)) \(appearance), below \(minimum(g.minimum, locale: locale))"
        switch key.kind {
        case .contrast:
            let what =
                switch role.kind {
                case .text: "words at"
                case .pin: "the pin's symbol at"
                case .mark, .fill: "marks at"
                }
            return "\(role.title) (\(role.group.title)): \(what) \(measured)"
        case .fillText:
            return "\(role.title) (\(role.group.title)): words on it at \(measured)"
        case .fillVisible:
            return "\(role.title) (\(role.group.title)): hard to see against the background \(appearance)"
        case .apart:
            let other = key.other.map { " and \($0.title)" } ?? ""
            return "\(role.title)\(other) (\(role.group.title)): too close to tell apart with \(visions(g.visions)) vision \(appearance)"
        }
    }

    /// "typical", "deutan", "typical and deutan", "protan, deutan and tritan".
    private static func visions(_ visions: [Deficiency?]) -> String {
        let names = visions.sorted { rank($0) < rank($1) }.map { $0.map(\.rawValue) ?? "typical" }
        guard let last = names.last else { return "typical" }
        return names.count == 1 ? last : names.dropLast().joined(separator: ", ") + " and " + last
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
