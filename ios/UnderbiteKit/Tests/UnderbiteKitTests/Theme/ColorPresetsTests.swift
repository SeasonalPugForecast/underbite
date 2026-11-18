import Foundation
import XCTest

@testable import UnderbiteKit

/// The presets: complete, and each meeting the contrast and separation `ThemeChecks` measures
/// (the rules are stated there): the default at the standard level, the sun preset at the high
/// one.
final class ColorPresetsTests: XCTestCase {
    private func describe(_ failures: [ThemeChecks.Finding]) -> String {
        failures.map { f in
            "\(f.role.rawValue) \(f.check) \(f.dark ? "dark" : "light"): \(f.value) < \(f.minimum)"
        }.joined(separator: "\n")
    }

    func testEveryPresetDefinesEveryRole() {
        for preset in ColorPresets.all {
            XCTAssertEqual(preset.missing, [], preset.id)
            XCTAssertEqual(preset.base, preset.id)
        }
        XCTAssertEqual(Set(ColorPresets.ids).count, ColorPresets.all.count)
        XCTAssertEqual(ColorPresets.ids.first, ColorPresets.defaultID)
        XCTAssertTrue(ColorPresets.ids.contains(ColorPresets.sunID))
    }

    func testEveryPresetMeetsTheStandardLevel() {
        for preset in ColorPresets.all {
            let failures = ThemeChecks.failures(preset, level: .standard)
            XCTAssertTrue(failures.isEmpty, "\(preset.id):\n\(describe(failures))")
        }
    }

    func testTheSunPresetMeetsTheHighLevel() {
        XCTAssertEqual(ThemeChecks.level(for: ColorPresets.sun), .high)
        let failures = ThemeChecks.failures(ColorPresets.sun)
        XCTAssertTrue(failures.isEmpty, describe(failures))
    }

    func testTheChecksMeasureEveryRoleInBothAppearances() {
        let findings = ThemeChecks.findings(ColorPresets.underbite, level: .standard)
        for dark in [false, true] {
            let measured = Set(findings.filter { $0.dark == dark }.map(\.role))
            XCTAssertEqual(measured, Set(ColorRole.allCases))
        }
        // Each apart pair four ways (typical vision and three deficiencies), in both.
        let apart = findings.filter { if case .apart = $0.check { return true }; return false }
        XCTAssertEqual(apart.count, ThemeChecks.apartPairs.count * 4 * 2)
    }

    func testTheChecksCatchAThemeThatFallsShort() {
        var theme = ColorPresets.underbite
        // Pale yellow words on white, a fill too strong for words, a pin with no room for
        // its symbol, and go painted as marginal.
        theme.colors[.statusWarning] = ThemeColor(light: RGBA(rgb: 0xFFE680), dark: RGBA(rgb: 0xFF9F0A))
        theme.colors[.timelineWindow] = ThemeColor(light: RGBA(rgb: 0x000000))
        theme.colors[.stationLow] = ThemeColor(light: RGBA(rgb: 0x6E6E6E), dark: RGBA(rgb: 0x6E6E6E))
        theme.colors[.verdictGo] = theme.colors[.verdictMarginal]
        let failures = ThemeChecks.failures(theme, level: .standard)
        let failed = Set(failures.map { "\($0.role.rawValue) \($0.dark)" })
        XCTAssertTrue(failed.contains("status.warning false"), describe(failures))
        XCTAssertFalse(failed.contains("status.warning true"), describe(failures))
        XCTAssertTrue(failures.contains { $0.role == .timelineWindow && $0.check == .fillText })
        XCTAssertTrue(failures.contains { $0.role == .verdictGo && $0.check == .apart(.verdictMarginal, deficiency: nil) })
        // #6E6E6E: 5.1:1 with white, so a pin passes; the check takes the better symbol.
        XCTAssertFalse(failures.contains { $0.role == .stationLow && $0.check == .contrast })
        theme.colors[.stationLow] = ThemeColor(light: RGBA(rgb: 0x7C7C7C))
        let pin = ThemeChecks.findings(theme, level: .high).first { $0.role == .stationLow && $0.check == .contrast }
        XCTAssertEqual(pin?.passes, false)
    }

    func testResolvePicksThePresetTheCustomThemeOrTheDefault() {
        XCTAssertEqual(ColorPresets.resolve(id: "sun"), ColorPresets.sun)
        XCTAssertEqual(ColorPresets.resolve(id: "no-such-theme"), ColorPresets.underbite)

        let custom = ColorTheme(
            id: "custom-a", name: "Harbour", base: "underbite",
            colors: [.tideCurve: ThemeColor(light: RGBA(rgb: 0x123456))])
        let resolved = ColorPresets.resolve(id: "custom-a", custom: [custom])
        XCTAssertEqual(resolved.id, "custom-a")
        XCTAssertEqual(resolved.missing, [])
        XCTAssertEqual(resolved[.tideCurve].light, RGBA(rgb: 0x123456))
        XCTAssertEqual(resolved[.chartNow], ColorPresets.underbite[.chartNow])
    }

    func testSunModePaintsWithTheSunPresetUnlessTheThemeIsBasedOnIt() {
        XCTAssertEqual(ColorPresets.resolve(id: "underbite", sunMode: true), ColorPresets.sun)
        let custom = ColorTheme(id: "custom-a", name: "Mine", base: "underbite", colors: [:])
        XCTAssertEqual(ColorPresets.resolve(id: "custom-a", custom: [custom], sunMode: true), ColorPresets.sun)

        let bright = ColorTheme(
            id: "custom-b", name: "Brighter", base: "sun",
            colors: [.accent: ThemeColor(light: RGBA(rgb: 0x002B80))])
        let resolved = ColorPresets.resolve(id: "custom-b", custom: [bright], sunMode: true)
        XCTAssertEqual(resolved.id, "custom-b")
        XCTAssertEqual(resolved[.accent].light, RGBA(rgb: 0x002B80))
        XCTAssertEqual(resolved[.chartNow], ColorPresets.sun[.chartNow])
        XCTAssertEqual(ThemeChecks.level(for: resolved), .high)
    }
}
