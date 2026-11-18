import Foundation
import XCTest

@testable import UnderbiteKit

/// A theme as JSON: what a saved custom theme looks like and how an old or hand-edited one
/// loads.
final class ColorThemeTests: XCTestCase {
    private func decode(_ json: String) throws -> ColorTheme {
        try JSONDecoder().decode(ColorTheme.self, from: Data(json.utf8))
    }

    func testACustomThemeRoundTrips() throws {
        let theme = ColorTheme(
            id: "custom-1", name: "Dawn patrol", base: "sun",
            colors: [
                .tideCurve: ThemeColor(light: RGBA(rgb: 0x004C99), dark: RGBA(rgb: 0x7FC4FF)),
                .timelineWindow: ThemeColor(light: RGBA(rgba: 0xFFB30066)),
                .verdictGo: ThemeColor(light: RGBA(rgb: 0x005964)),
            ])
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        let data = try encoder.encode(theme)
        XCTAssertEqual(try JSONDecoder().decode(ColorTheme.self, from: data), theme)
        XCTAssertEqual(
            String(decoding: data, as: UTF8.self),
            ##"{"base":"sun","colors":{"tide.curve":{"dark":"#7FC4FF","light":"#004C99"},"##
                + ##""timeline.window":{"light":"#FFB30066"},"verdict.go":{"light":"#005964"}},"##
                + ##""id":"custom-1","name":"Dawn patrol"}"##)
    }

    func testUnknownRoleKeysAreIgnored() throws {
        let theme = try decode(
            #"""
            {"id": "custom-1", "name": "Old", "base": "underbite", "colors": {
              "tide.curve": "#123456",
              "sky.clear_day.top": "#2F6FD0",
              "tide.someday": {"light": "#000000"}
            }}
            """#)
        XCTAssertEqual(theme.colors, [.tideCurve: ThemeColor(light: RGBA(rgb: 0x123456))])
    }

    func testColoursThatDoNotParseAreDroppedAndTheRestKept() throws {
        let theme = try decode(
            #"""
            {"id": "custom-1", "colors": {
              "accent": "blue", "chart.now": {"dark": "#FF0000"}, "spot.pinned": 7,
              "tide.low": {"light": "#00808F", "dark": "#40D0D8"}
            }}
            """#)
        XCTAssertEqual(theme.colors, [.tideLow: ThemeColor(light: RGBA(rgb: 0x00808F), dark: RGBA(rgb: 0x40D0D8))])
        // A theme without a name or a base is named by its id and based on the default.
        XCTAssertEqual(theme.name, "custom-1")
        XCTAssertEqual(theme.base, ColorPresets.defaultID)
    }

    func testARoleMissingFromASavedThemeFallsBackToItsBase() throws {
        let saved = try decode(##"{"id": "custom-1", "name": "Mine", "base": "sun", "colors": {"accent": "#002B80"}}"##)
        XCTAssertTrue(saved.missing.contains(.chartNow))
        let loaded = ColorPresets.completed(saved)
        XCTAssertEqual(loaded.missing, [])
        XCTAssertEqual(loaded[.accent], ThemeColor(light: RGBA(rgb: 0x002B80)))
        XCTAssertEqual(loaded[.chartNow], ColorPresets.sun[.chartNow])
        // An unknown base falls back to the default preset.
        var orphan = saved
        orphan.base = "aurora"
        XCTAssertEqual(ColorPresets.completed(orphan)[.chartNow], ColorPresets.underbite[.chartNow])
        // Even a theme never completed paints every role.
        XCTAssertEqual(saved[.tideFill], ColorPresets.underbite[.tideFill])
    }

    func testAThemeWithoutAnIdDoesNotDecode() {
        XCTAssertThrowsError(try decode(#"{"name": "No id", "colors": {}}"#))
    }

    func testAColourResolvesForEachAppearance() {
        let both = ThemeColor(light: RGBA(rgb: 0x0066D9), dark: RGBA(rgb: 0x0A84FF))
        XCTAssertEqual(both.resolved(dark: false), RGBA(rgb: 0x0066D9))
        XCTAssertEqual(both.resolved(dark: true), RGBA(rgb: 0x0A84FF))
        let one = ThemeColor(light: RGBA(rgb: 0x34435E))
        XCTAssertEqual(one.resolved(dark: true), RGBA(rgb: 0x34435E))
    }
}
