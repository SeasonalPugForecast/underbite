import Foundation
import XCTest

@testable import UnderbiteKit

/// Making, naming and editing the reader's own themes.
final class CustomThemesTests: XCTestCase {
    private let blue = ThemeColor(light: RGBA(rgb: 0x123456))

    func testDuplicatingAPresetStartsEmptyOnItsBase() {
        let copy = CustomThemes.duplicate(ColorPresets.sun, existing: [], id: "custom-1")
        XCTAssertEqual(copy.id, "custom-1")
        XCTAssertEqual(copy.name, "My Sun")
        XCTAssertEqual(copy.base, "sun")
        XCTAssertEqual(copy.colors, [:])
        XCTAssertTrue(CustomThemes.duplicate(ColorPresets.sun, existing: []).id.hasPrefix(CustomThemes.idPrefix))
    }

    func testDuplicatingACustomThemeKeepsItsColoursUnderTheNextName() {
        let mine = ColorTheme(id: "custom-1", name: "My Sun", base: "sun", colors: [.accent: blue])
        let copy = CustomThemes.duplicate(mine, existing: [mine], id: "custom-2")
        XCTAssertEqual(copy.name, "My Sun 2")
        XCTAssertEqual(copy.colors, [.accent: blue])
        let again = CustomThemes.duplicate(copy, existing: [mine, copy], id: "custom-3")
        XCTAssertEqual(again.name, "My Sun 3")
    }

    func testNamesAreTrimmedAndKeptUnique() {
        XCTAssertEqual(CustomThemes.cleanName("  Night shift \n"), "Night shift")
        XCTAssertNil(CustomThemes.cleanName(" \n "))
        let taken = [
            ColorTheme(id: "custom-1", name: "Mine", colors: [:]),
            ColorTheme(id: "custom-2", name: "Mine 2", colors: [:]),
        ]
        XCTAssertEqual(CustomThemes.uniqueName("Mine", among: taken), "Mine 3")
        XCTAssertEqual(CustomThemes.uniqueName("Yours", among: taken), "Yours")
    }

    func testLoadingDropsRepeatsBlankIdsAndPresetIds() {
        let themes = [
            ColorTheme(id: "custom-1", name: "First", colors: [:]),
            ColorTheme(id: "custom-1", name: "Second", colors: [:]),
            ColorTheme(id: "", name: "Blank", colors: [:]),
            ColorTheme(id: "sun", name: "Not the sun", base: "underbite", colors: [:]),
        ]
        XCTAssertEqual(CustomThemes.loaded(themes).map(\.name), ["First"])
    }

    func testEditingKeepsOnlyWhatDiffersFromTheBase() {
        let base = ColorPresets.underbite
        var theme = ColorTheme(id: "custom-1", name: "Mine", base: "underbite", colors: [:])
        theme = theme.setting(.accent, to: blue, base: base)
        XCTAssertTrue(theme.differs(.accent, from: base))
        XCTAssertFalse(theme.differs(.chartNow, from: base))
        theme = theme.setting(.accent, to: base[.accent], base: base)
        XCTAssertEqual(theme.colors, [:])

        let whole = ColorTheme(id: "custom-1", name: "Mine", base: "underbite", colors: base.colors)
            .setting(.tideLow, to: blue, base: base)
        XCTAssertEqual(whole.colors.count, ColorRole.allCases.count)
        XCTAssertEqual(whole.trimmed(against: base).colors, [.tideLow: blue])
    }

    func testPreviewSimulatesEveryColour() {
        let preview = ColorPresets.underbite.simulated(as: .achromat)
        XCTAssertEqual(preview.colors.count, ColorRole.allCases.count)
        for color in preview.colors.values {
            XCTAssertEqual(ColorMath.oklab(color.light).chroma, 0, accuracy: 1e-6)
        }
        XCTAssertEqual(preview[.tideFill].light.alpha, ColorPresets.underbite[.tideFill].light.alpha)
    }
}
