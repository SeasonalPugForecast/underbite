import Foundation
import XCTest

@testable import UnderbiteKit

/// The theme store over a defaults suite of its own: the chosen theme, custom themes saved and
/// loaded, and which defaults it would use.
final class ThemeStoreTests: XCTestCase {
    private var suite: String!
    private var defaults: UserDefaults!
    private var store: ThemeStore!

    override func setUp() {
        suite = "underbite-theme-tests-\(UUID().uuidString)"
        defaults = UserDefaults(suiteName: suite)
        store = ThemeStore(defaults: defaults, backing: .standard)
    }

    override func tearDown() {
        defaults.removePersistentDomain(forName: suite)
    }

    private func setSaved(_ json: String) {
        defaults.set(Data(json.utf8), forKey: ThemeStore.customThemesKey)
    }

    func testTheDefaultPresetUntilAThemeIsChosen() {
        XCTAssertEqual(store.themeID, ColorPresets.defaultID)
        XCTAssertEqual(store.customThemes, [])
        XCTAssertEqual(store.resolved(), ColorPresets.underbite)
        store.themeID = "sun"
        XCTAssertEqual(ThemeStore(defaults: defaults, backing: .standard).themeID, "sun")
        XCTAssertEqual(store.resolved(), ColorPresets.sun)
    }

    func testACustomThemeRoundTripsThroughTheStore() {
        let base = ColorPresets.underbite
        let theme = ColorTheme(
            id: "custom-1", name: "Night tide", base: "underbite",
            colors: base.colors.merging([
                .tideCurve: ThemeColor(light: RGBA(rgb: 0x004C99), dark: RGBA(rgb: 0x7FC4FF)),
                .timelineNight: ThemeColor(light: RGBA(rgba: 0x1A237E40), dark: RGBA(rgba: 0x3949AB66)),
            ]) { _, new in new })
        store.save(theme)
        store.themeID = theme.id

        // Saved small: only what differs from the base.
        let reread = ThemeStore(defaults: defaults, backing: .standard)
        XCTAssertEqual(reread.customThemes.map(\.id), ["custom-1"])
        XCTAssertEqual(Set(reread.customThemes[0].colors.keys), [.tideCurve, .timelineNight])
        // And whole again when resolved.
        XCTAssertEqual(reread.resolved(), theme)

        // Saving it again replaces it.
        var renamed = theme
        renamed.name = "Night tide, darker"
        reread.save(renamed)
        XCTAssertEqual(store.customThemes.map(\.name), ["Night tide, darker"])
    }

    func testUnknownRoleKeysInASavedThemeAreIgnored() {
        setSaved(
            ##"""
            [{"id": "custom-1", "name": "From later", "base": "underbite", "colors": {
              "tide.curve": "#004C99", "tide.rip": "#FF00FF", "chart.wave_height": {"light": "#123456"}
            }}]
            """##)
        let loaded = store.customThemes
        XCTAssertEqual(loaded.count, 1)
        XCTAssertEqual(loaded[0].colors, [.tideCurve: ThemeColor(light: RGBA(rgb: 0x004C99))])
    }

    func testARoleMissingFromASavedThemeTakesItsDefault() {
        setSaved(##"[{"id": "custom-1", "name": "Old", "base": "sun", "colors": {"accent": "#002B80"}}]"##)
        store.themeID = "custom-1"
        let theme = store.resolved()
        XCTAssertEqual(theme.missing, [])
        XCTAssertEqual(theme[.accent], ThemeColor(light: RGBA(rgb: 0x002B80)))
        for role in ColorRole.allCases where role != .accent {
            XCTAssertEqual(theme[role], ColorPresets.sun[role], role.rawValue)
        }
    }

    func testOneBadThemeNeverLosesTheOthers() {
        setSaved(
            ##"""
            [{"name": "No id"}, 42, {"id": "custom-2", "name": "Fine", "colors": {}},
             {"id": "underbite", "name": "Not the preset"}]
            """##)
        XCTAssertEqual(store.customThemes.map(\.id), ["custom-2"])
        defaults.set(Data("not json".utf8), forKey: ThemeStore.customThemesKey)
        XCTAssertEqual(store.customThemes, [])
        XCTAssertEqual(store.resolved(), ColorPresets.underbite)
    }

    func testDeletingTheChosenThemeChoosesItsBase() {
        store.save(ColorTheme(id: "custom-1", name: "A", base: "sun", colors: [:]))
        store.save(ColorTheme(id: "custom-2", name: "B", base: "underbite", colors: [:]))
        store.themeID = "custom-1"
        store.delete(id: "custom-2")
        XCTAssertEqual(store.themeID, "custom-1")
        store.delete(id: "custom-1")
        XCTAssertEqual(store.themeID, "sun")
        XCTAssertEqual(store.customThemes, [])
        store.delete(id: "custom-9")
        XCTAssertEqual(store.themeID, "sun")
    }

    func testAChosenThemeThatIsGoneResolvesToTheDefault() {
        store.themeID = "custom-gone"
        XCTAssertEqual(store.resolved(), ColorPresets.underbite)
        XCTAssertEqual(store.resolved(sunMode: true), ColorPresets.sun)
    }

    func testTheAppGroupIsUsedOnlyWhenItIsThere() {
        let url = URL(filePath: "/tmp/group.dev.savery.underbite", directoryHint: .isDirectory)
        XCTAssertEqual(ThemeStore.backing(groupContainer: nil) { _ in true }, .standard)
        XCTAssertEqual(ThemeStore.backing(groupContainer: url) { _ in true }, .appGroup)
        #if os(macOS)
            // macOS answers a URL for any group; only an existing container counts.
            XCTAssertEqual(ThemeStore.backing(groupContainer: url) { _ in false }, .standard)
        #endif
        XCTAssertTrue(ThemeStore.Backing.appGroup.description.contains(ThemeStore.appGroupID))
        XCTAssertTrue(ThemeStore.Backing.standard.description.contains("not shared"))
        XCTAssertEqual(store.backing, .standard)
    }
}
