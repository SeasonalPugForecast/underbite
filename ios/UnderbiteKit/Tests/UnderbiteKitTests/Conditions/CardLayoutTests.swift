import Foundation
import XCTest

@testable import UnderbiteKit

/// Where a card edit is written, against the core's own resolution of a water type's cards.
final class CardLayoutTests: XCTestCase {
    private let tidal = "tidal_river_estuary"
    private let river = "river_stream"

    private func defaults() throws -> Settings { try Settings.defaults() }

    private func shownIDs(_ settings: Settings, _ waterType: String) throws -> [String] {
        try cardCatalogue(waterType: waterType, settingsJson: settings.json())
            .filter(\.shown).map(\.id)
    }

    private func assertValid(_ settings: Settings, file: StaticString = #filePath, line: UInt = #line)
        throws
    {
        let issues = validateSettings(json: try settings.json())
        XCTAssertTrue(issues.isEmpty, "\(issues)", file: file, line: line)
    }

    // MARK: - Reading

    func testTheChoicesAreTheCoresCatalogueForTheWaterType() throws {
        let settings = try defaults()
        let choices = try CardLayoutEditing.choices(settings: settings, waterType: tidal)
        XCTAssertEqual(choices.all.filter(\.shown).map(\.id), try shownIDs(settings, tidal))
        XCTAssertEqual(choices.all.first?.id, "now")
        for id in ["windows", "go_no_go"] {
            let choice = try XCTUnwrap(choices.all.first { $0.id == id })
            XCTAssertTrue(choice.available, id)
            XCTAssertTrue(choices.editable.contains { $0.id == id }, id)
        }
    }

    func testTheBaseChoicesFollowTheBaseOrderAndHiddenList() throws {
        var settings = try defaults()
        settings.cards.hidden = ["waves"]
        let choices = try CardLayoutEditing.choices(settings: settings, waterType: nil)
        XCTAssertEqual(
            choices.all.filter(\.shown).map(\.id),
            settings.cards.order.filter { $0 != "waves" })
        XCTAssertEqual(choices.all.first { $0.id == "waves" }?.shown, false)
    }

    func testOnlyRiversPondsAndIceShowOtherCardsThanTheDefaultsAtFirst() throws {
        let settings = try defaults()
        XCTAssertTrue(try CardLayoutEditing.usesOwnLayout(settings: settings, waterType: river))
        XCTAssertFalse(try CardLayoutEditing.usesOwnLayout(settings: settings, waterType: tidal))
        XCTAssertFalse(try CardLayoutEditing.usesOwnLayout(settings: settings, waterType: "offshore"))
    }

    // MARK: - Writing

    func testAnEditWithoutAnOwnLayoutWritesTheDefaultList() throws {
        var settings = try defaults()
        var choices = try CardLayoutEditing.choices(settings: settings, waterType: tidal)
        var edited = choices.editable
        edited.insert(edited.remove(at: 1), at: 0)  // alerts first
        edited[edited.firstIndex { $0.id == "wind" }!].shown = false
        choices.setEditable(edited)
        CardLayoutEditing.apply(choices, to: &settings, waterType: tidal, ownLayout: false)

        XCTAssertEqual(settings.cards.order.first, "alerts")
        XCTAssertTrue(settings.cards.hidden.contains("wind"))
        XCTAssertNil(settings.cards.tidal)
        let shown = try shownIDs(settings, tidal)
        XCTAssertEqual(shown.first, "alerts")
        XCTAssertFalse(shown.contains("wind"))
        // Another water type with no layout of its own follows the default list.
        XCTAssertFalse(try shownIDs(settings, "offshore").contains("wind"))
        try assertValid(settings)
    }

    func testAnEditWithAnOwnLayoutWritesOnlyThatWaterType() throws {
        var settings = try defaults()
        let base = settings.cards
        var choices = try CardLayoutEditing.choices(settings: settings, waterType: tidal)
        try CardLayoutEditing.setOwnLayout(true, choices: choices, in: &settings, waterType: tidal)
        XCTAssertNotNil(settings.cards.tidal)

        var edited = choices.editable
        edited[edited.firstIndex { $0.id == "tide" }!].shown = false
        choices.setEditable(edited)
        CardLayoutEditing.apply(choices, to: &settings, waterType: tidal, ownLayout: true)

        XCTAssertEqual(settings.cards.order, base.order)
        XCTAssertEqual(settings.cards.hidden, base.hidden)
        XCTAssertEqual(settings.cards.tidal?.hidden.contains("tide"), true)
        XCTAssertFalse(try shownIDs(settings, tidal).contains("tide"))
        XCTAssertTrue(try shownIDs(settings, "offshore").contains("tide"))
        try assertValid(settings)
    }

    func testAnOwnLayoutKeepsAHiddenCardHiddenAndTheOrderTheEditorLeft() throws {
        var settings = try defaults()
        var choices = try CardLayoutEditing.choices(settings: settings, waterType: tidal)
        var edited = choices.editable
        edited.insert(edited.remove(at: 4), at: 0)
        choices.setEditable(edited)
        CardLayoutEditing.apply(choices, to: &settings, waterType: tidal, ownLayout: true)
        let ids = try shownIDs(settings, tidal)
        XCTAssertEqual(ids.first, edited[0].id)
        let again = try CardLayoutEditing.choices(settings: settings, waterType: tidal)
        XCTAssertEqual(again.editable.map(\.id), edited.map(\.id))
        XCTAssertEqual(again.editable.map(\.shown), edited.map(\.shown))
    }

    func testCardsThisVersionDoesNotBuildKeepTheirPlace() throws {
        var settings = try defaults()
        var choices = try CardLayoutEditing.choices(settings: settings, waterType: tidal)
        var edited = choices.editable
        edited.reverse()
        choices.setEditable(edited)
        CardLayoutEditing.apply(choices, to: &settings, waterType: tidal, ownLayout: false)
        XCTAssertTrue(settings.cards.order.contains("windows"))
        XCTAssertFalse(settings.cards.hidden.contains("windows"))
        // A card nobody ordered and this version does not build is left out of both lists, so
        // that it stays off until its card exists.
        XCTAssertFalse(settings.cards.order.contains("species"))
        XCTAssertFalse(settings.cards.hidden.contains("species"))
        try assertValid(settings)
    }

    func testEditingARiverWritesItsOwnLayoutAndLeavesTheDefaultListAlone() throws {
        var settings = try defaults()
        let base = settings.cards
        var choices = try CardLayoutEditing.choices(settings: settings, waterType: river)
        let own = try CardLayoutEditing.usesOwnLayout(settings: settings, waterType: river)
        XCTAssertTrue(own)
        var edited = choices.editable
        edited[edited.firstIndex { $0.id == "flow" }!].shown = false
        choices.setEditable(edited)
        CardLayoutEditing.apply(choices, to: &settings, waterType: river, ownLayout: own)
        XCTAssertEqual(settings.cards.order, base.order)
        XCTAssertEqual(settings.cards.hidden, base.hidden)
        XCTAssertFalse(try shownIDs(settings, river).contains("flow"))
        XCTAssertTrue(try shownIDs(settings, tidal).contains("flow"))
        try assertValid(settings)
    }

    func testTakingARiverBackToTheDefaultListMakesItFollowTheDefaults() throws {
        var settings = try defaults()
        let choices = try CardLayoutEditing.choices(settings: settings, waterType: river)
        try CardLayoutEditing.setOwnLayout(false, choices: choices, in: &settings, waterType: river)
        XCTAssertEqual(try shownIDs(settings, river), settings.cards.order)
        XCTAssertFalse(try CardLayoutEditing.usesOwnLayout(settings: settings, waterType: river))

        // An edit now goes to the default list and the river follows it.
        var again = try CardLayoutEditing.choices(settings: settings, waterType: river)
        var edited = again.editable
        edited[edited.firstIndex { $0.id == "wind" }!].shown = false
        again.setEditable(edited)
        CardLayoutEditing.apply(again, to: &settings, waterType: river, ownLayout: false)
        XCTAssertTrue(settings.cards.hidden.contains("wind"))
        XCTAssertFalse(try shownIDs(settings, river).contains("wind"))
        XCTAssertFalse(try shownIDs(settings, tidal).contains("wind"))
        try assertValid(settings)
    }

    func testTakingATidalLayoutBackRemovesIt() throws {
        var settings = try defaults()
        let choices = try CardLayoutEditing.choices(settings: settings, waterType: tidal)
        try CardLayoutEditing.setOwnLayout(true, choices: choices, in: &settings, waterType: tidal)
        try CardLayoutEditing.setOwnLayout(false, choices: choices, in: &settings, waterType: tidal)
        XCTAssertNil(settings.cards.tidal)
    }

    func testTheQuickToggleHidesAndShowsACardWhereItsEditsGo() throws {
        let settings = try defaults()
        let hidden = try CardLayoutEditing.setting(
            "wind", shown: false, in: settings, waterType: tidal)
        XCTAssertFalse(try shownIDs(hidden, tidal).contains("wind"))
        XCTAssertNil(hidden.cards.tidal)
        let back = try CardLayoutEditing.setting("wind", shown: true, in: hidden, waterType: tidal)
        XCTAssertTrue(try shownIDs(back, tidal).contains("wind"))

        let river = try CardLayoutEditing.setting("flow", shown: false, in: settings, waterType: river)
        XCTAssertFalse(try shownIDs(river, self.river).contains("flow"))
        XCTAssertTrue(try shownIDs(river, tidal).contains("flow"))
    }

    func testRestoringTheDefaultsPutsTheShippedLayoutBack() throws {
        var settings = try defaults()
        let shipped = settings
        let choices = try CardLayoutEditing.choices(settings: settings, waterType: nil)
        var edited = choices.editable
        edited.reverse()
        var changed = choices
        changed.setEditable(edited)
        CardLayoutEditing.apply(changed, to: &settings, waterType: nil, ownLayout: false)
        XCTAssertNotEqual(settings.cards.order, shipped.cards.order)
        try CardLayoutEditing.restoreDefaults(in: &settings, waterType: nil, ownLayout: false)
        XCTAssertEqual(settings.cards, shipped.cards)

        try CardLayoutEditing.setOwnLayout(
            true, choices: try CardLayoutEditing.choices(settings: settings, waterType: tidal),
            in: &settings, waterType: tidal)
        try CardLayoutEditing.restoreDefaults(in: &settings, waterType: tidal, ownLayout: true)
        XCTAssertNil(settings.cards.tidal)
    }

    func testCardsOfAnUnknownWaterTypeAreRefused() {
        XCTAssertThrowsError(
            try CardLayoutEditing.choices(settings: try! Settings.defaults(), waterType: "swamp"))
    }
}
