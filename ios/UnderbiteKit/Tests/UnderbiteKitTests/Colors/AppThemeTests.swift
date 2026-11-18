import Foundation
import XCTest

@testable import UnderbiteKit

/// The app's theme model over a defaults suite of its own: choosing, making, editing, naming
/// and deleting custom themes, and what a new model reads back.
@MainActor
final class AppThemeTests: XCTestCase {
    /// A suite of its own for each test (XCTest makes an instance per test).
    private let suite = "underbite-app-theme-tests-\(UUID().uuidString)"

    override func tearDown() {
        UserDefaults(suiteName: suite)?.removePersistentDomain(forName: suite)
    }

    private func model() throws -> AppTheme {
        AppTheme(store: ThemeStore(defaults: try XCTUnwrap(UserDefaults(suiteName: suite)), backing: .standard))
    }

    private let teal = ThemeColor(light: RGBA(rgb: 0x123456), dark: RGBA(rgb: 0x89ABCD))

    func testChoosingAPresetIsSavedAndUnknownIdsAreIgnored() throws {
        let theme = try model()
        XCTAssertEqual(theme.themeID, ColorPresets.defaultID)
        theme.choose("sun")
        XCTAssertEqual(theme.resolved(sunMode: false), ColorPresets.sun)
        theme.choose("custom-missing")
        XCTAssertEqual(theme.themeID, "sun")
        XCTAssertEqual(try model().themeID, "sun")
    }

    func testCustomizingAPresetMakesAChosenThemeOnIt() throws {
        let theme = try model()
        let mine = try XCTUnwrap(theme.customize(preset: "sun"))
        XCTAssertEqual(mine.name, "My Sun")
        XCTAssertEqual(mine.base, "sun")
        XCTAssertEqual(theme.themeID, mine.id)
        XCTAssertNil(theme.customize(preset: "no-such-preset"))

        let again = try model()
        XCTAssertEqual(again.themeID, mine.id)
        XCTAssertEqual(again.customThemes.map(\.name), ["My Sun"])
        XCTAssertEqual(again.resolved(sunMode: false).colors, ColorPresets.sun.colors)
    }

    func testEditsKeepOnlyWhatDiffersAndRoundTrip() throws {
        let theme = try model()
        let id = try XCTUnwrap(theme.customize(preset: "underbite")).id
        theme.set(.verdictGo, to: teal, in: id)
        theme.set(.accent, to: ColorPresets.underbite[.accent], in: id)
        XCTAssertEqual(theme.custom(id)?.colors, [.verdictGo: teal])

        let again = try model()
        XCTAssertEqual(again.custom(id)?.colors, [.verdictGo: teal])
        XCTAssertEqual(again.resolved(sunMode: false)[.verdictGo], teal)
        XCTAssertEqual(again.resolved(sunMode: false)[.verdictNoGo], ColorPresets.underbite[.verdictNoGo])

        again.reset(.verdictGo, in: id)
        XCTAssertEqual(again.custom(id)?.colors, [:])
        again.set(.tideHigh, to: teal, in: id)
        again.set(.tideLow, to: teal, in: id)
        again.restoreDefaults(id)
        XCTAssertEqual(try model().custom(id)?.colors, [:])
    }

    func testDuplicatesFollowTheirOriginalWithTheNextName() throws {
        let theme = try model()
        let first = try XCTUnwrap(theme.customize(preset: "underbite"))
        let other = try XCTUnwrap(theme.customize(preset: "sun"))
        theme.set(.accent, to: teal, in: first.id)
        let copy = try XCTUnwrap(theme.duplicate(first.id))
        XCTAssertEqual(copy.name, "My Underbite 2")
        XCTAssertEqual(copy.colors, [.accent: teal])
        XCTAssertEqual(theme.customThemes.map(\.id), [first.id, copy.id, other.id])
        XCTAssertNil(theme.duplicate("custom-missing"))
        XCTAssertEqual(try model().customThemes.map(\.name), ["My Underbite", "My Underbite 2", "My Sun"])
    }

    func testRenamingTrimsAndKeepsNamesApart() throws {
        let theme = try model()
        let first = try XCTUnwrap(theme.customize(preset: "underbite"))
        let second = try XCTUnwrap(theme.customize(preset: "sun"))
        XCTAssertTrue(theme.rename(first.id, to: "  Night tide \n"))
        XCTAssertEqual(theme.custom(first.id)?.name, "Night tide")
        XCTAssertTrue(theme.rename(second.id, to: "Night tide"))
        XCTAssertEqual(theme.custom(second.id)?.name, "Night tide 2")
        XCTAssertTrue(theme.rename(first.id, to: "Night tide"))
        XCTAssertEqual(theme.custom(first.id)?.name, "Night tide")
        XCTAssertFalse(theme.rename(first.id, to: "   "))
        XCTAssertFalse(theme.rename("custom-missing", to: "Mine"))
        XCTAssertEqual(try model().customThemes.map(\.name), ["Night tide", "Night tide 2"])
    }

    func testDeletingTheChosenThemeChoosesItsBase() throws {
        let theme = try model()
        let mine = try XCTUnwrap(theme.customize(preset: "sun"))
        let kept = try XCTUnwrap(theme.customize(preset: "underbite"))
        theme.choose(mine.id)
        theme.delete(mine.id)
        XCTAssertEqual(theme.themeID, "sun")
        XCTAssertEqual(theme.customThemes.map(\.id), [kept.id])
        theme.delete(kept.id)
        XCTAssertEqual(theme.themeID, "sun")
        let again = try model()
        XCTAssertEqual(again.themeID, "sun")
        XCTAssertEqual(again.customThemes, [])
    }

    func testSunModePaintsWithSunUnlessTheThemeIsBasedOnIt() throws {
        let theme = try model()
        let mine = try XCTUnwrap(theme.customize(preset: "underbite"))
        XCTAssertEqual(theme.resolved(sunMode: true), ColorPresets.sun)
        let sunny = try XCTUnwrap(theme.customize(preset: "sun"))
        theme.set(.accent, to: teal, in: sunny.id)
        XCTAssertEqual(theme.resolved(sunMode: true).id, sunny.id)
        XCTAssertEqual(theme.resolved(sunMode: true)[.accent], teal)
        theme.choose(mine.id)
        XCTAssertEqual(theme.resolved(sunMode: false).id, mine.id)
    }

    func testScreenshotLaunchesUseAnEmptyStoreOfTheirOwn() {
        let launchOnly = "dev.savery.underbite.launch-themes"
        defer { UserDefaults(suiteName: launchOnly)?.removePersistentDomain(forName: launchOnly) }
        let first = AppTheme(store: AppTheme.launchStore(["app", "-customTheme", "underbite"]))
        first.customize(preset: "underbite")
        let second = AppTheme(store: AppTheme.launchStore(["app", "-editTheme", "sun"]))
        XCTAssertEqual(second.customThemes, [])
        XCTAssertEqual(second.themeID, ColorPresets.defaultID)
        XCTAssertEqual(second.backing, .standard)
    }
}
