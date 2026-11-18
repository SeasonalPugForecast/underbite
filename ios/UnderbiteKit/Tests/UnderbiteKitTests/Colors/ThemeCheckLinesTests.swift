import Foundation
import XCTest

@testable import UnderbiteKit

/// The editor's warnings: none for the presets, one line per role and check for an edit that
/// falls short, with appearances and colour visions said once.
final class ThemeCheckLinesTests: XCTestCase {
    private let english = Locale(identifier: "en_US_POSIX")

    func testThePresetsRaiseNoWarning() {
        for preset in ColorPresets.all {
            XCTAssertEqual(ThemeCheckLines.lines(preset, locale: english), [], preset.id)
        }
    }

    func testAnEditThatFallsShortIsSaidOncePerRoleAndCheck() {
        let base = ColorPresets.underbite
        let theme = ColorTheme(id: "custom-1", name: "Mine", base: "underbite", colors: [:])
            .setting(.accent, to: ThemeColor(light: .white, dark: base[.accent].dark), base: base)
            .setting(.verdictMarginal, to: base[.verdictGo], base: base)
            .completed(from: base)
        let lines = ThemeCheckLines.lines(theme, locale: english)
        XCTAssertEqual(
            lines.map(\.text),
            [
                "Accent (Text and accent): words at 1.0:1 in light appearance, below 4.5:1",
                "Go and Marginal (Go / no-go): too close to tell apart with typical, protan, deutan and tritan vision in light and dark appearance",
            ])
        XCTAssertEqual(lines.map(\.roles), [[.accent], [.verdictGo, .verdictMarginal]])
        XCTAssertEqual(ThemeCheckLines.roles(lines), [.accent, .verdictGo, .verdictMarginal])
        XCTAssertEqual(Set(lines.map(\.id)).count, lines.count)
    }

    func testEachKindOfCheckHasItsWords() {
        func finding(_ role: ColorRole, _ check: ThemeChecks.Finding.Check, dark: Bool, value: Double, minimum: Double)
            -> ThemeChecks.Finding
        {
            ThemeChecks.Finding(role: role, check: check, dark: dark, value: value, minimum: minimum)
        }
        let failures = [
            finding(.stationLow, .contrast, dark: false, value: 2.04, minimum: 3),
            finding(.stationLow, .contrast, dark: true, value: 2.98, minimum: 3),
            finding(.flowNormal, .contrast, dark: true, value: 2.95, minimum: 3),
            finding(.timelineWindow, .fillText, dark: true, value: 4.49, minimum: 4.5),
            finding(.timelineWindow, .fillVisible, dark: false, value: 0.01, minimum: 0.03),
            finding(.flowNormal, .apart(.flowAboveNormal, deficiency: .tritan), dark: false, value: 0.01, minimum: 0.04),
            finding(.flowNormal, .apart(.flowAboveNormal, deficiency: .deutan), dark: false, value: 0.02, minimum: 0.04),
            // A passing measurement is never a warning.
            finding(.accent, .contrast, dark: false, value: 5, minimum: 4.5),
        ]
        XCTAssertEqual(
            ThemeCheckLines.lines(failures, locale: english).map(\.text),
            [
                "Normal (River flow): marks at 2.9:1 in dark appearance, below 3:1",
                "Normal and Above normal (River flow): too close to tell apart with deutan and tritan vision in light appearance",
                "Low water (Station pins): the pin's symbol at 2.0:1 in light and dark appearance, below 3:1",
                "Snack times (Timeline): words on it at 4.4:1 in dark appearance, below 4.5:1",
                "Snack times (Timeline): hard to see against the background in light appearance",
            ])
    }

    func testRatiosFollowTheLocale() {
        XCTAssertEqual(ThemeCheckLines.ratio(4.49, locale: english), "4.4:1")
        XCTAssertEqual(ThemeCheckLines.ratio(4.49, locale: Locale(identifier: "de_DE")), "4,4:1")
    }
}
