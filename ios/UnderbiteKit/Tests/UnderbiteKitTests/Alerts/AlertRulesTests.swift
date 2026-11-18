import Foundation
import XCTest

@testable import UnderbiteKit

/// The editor's rule as the core reads, words and refuses it: thresholds typed in the user's
/// units, the fields its refusals point at, and the box drawn on the map.
final class AlertRulesTests: XCTestCase {
    private let settings = defaultSettingsJson()

    private func savedRow(_ rule: AlertRuleDocument) throws -> AlertRuleRow {
        let json = try putAlertRule(settingsJson: settings, ruleJson: try rule.json())
        let rows = try alertRules(
            settingsJson: json, spots: [NamedPlace(id: SeedSpots.croton.id, name: "Croton River")],
            trips: [], state: nil, timeZone: "America/New_York", now: 1_800_000_000)
        return try XCTUnwrap(rows.first)
    }

    func testFieldsWithoutAValueAreLeftOut() throws {
        let rule = AlertRuleDocument(
            id: "a", name: "Go", target: .trip("t"),
            allOf: [AlertRuleDocument.Condition(kind: "trip_go")])
        let json = try rule.json()
        XCTAssertTrue(json.contains(#"{"kind":"trip_go"}"#), json)
        XCTAssertFalse(json.contains("quiet_hours"))
        XCTAssertFalse(json.contains("repeat_after_hours"))
        XCTAssertTrue(json.contains(#""show_name":true"#))
    }

    func testTheSampleRuleComesBackFromTheCoreWithItsWords() throws {
        let row = try savedRow(AlertSeeds.rule)
        XCTAssertEqual(row.id, AlertSeeds.id)
        XCTAssertEqual(try AlertRuleDocument(json: row.ruleJson), AlertSeeds.rule)
        XCTAssertEqual(row.state, .notChecked)
        XCTAssertEqual(row.words.target, "Croton River")
        XCTAssertTrue(row.words.summary.contains("800 cfs"), row.words.summary)
        XCTAssertTrue(row.words.summary.contains("66"), row.words.summary)
        XCTAssertNotNil(row.words.quiet)
        let saved = try putAlertRule(settingsJson: settings, ruleJson: try AlertSeeds.rule.json())
        XCTAssertEqual(AlertRuleDocument.find(AlertSeeds.id, inSettings: saved), AlertSeeds.rule)
        XCTAssertNil(AlertRuleDocument.find("other", inSettings: saved))
    }

    func testThresholdsAreTypedInTheUsersUnits() throws {
        let draft = AlertDraft(document: AlertSeeds.rule, settingsJSON: settings)
        XCTAssertEqual(draft.conditions.map(\.below), ["800", "66"])
        XCTAssertEqual(draft.conditions.map(\.above), ["", ""])
        XCTAssertEqual(draft.quietStart, 22 * 60)
        XCTAssertEqual(draft.quietEnd, 6 * 60)
        let (document, problems) = draft.document(settingsJSON: settings)
        XCTAssertEqual(problems, [:])
        let flow = try XCTUnwrap(document?.allOf.first?.below)
        // The core keeps a typed threshold to the precision it was typed in.
        XCTAssertEqual(flow, AlertSeeds.flow, accuracy: 1e-3)
        let temperature = try XCTUnwrap(document?.allOf.last?.below)
        XCTAssertEqual(temperature, AlertSeeds.temperature, accuracy: 1e-3)
    }

    func testAThresholdTheCoreCannotReadIsShownOnItsField() {
        var draft = AlertDraft(document: AlertSeeds.rule, settingsJSON: settings)
        draft.conditions[1].above = "warm"
        let (document, problems) = draft.document(settingsJSON: settings)
        XCTAssertNil(document)
        let message = problems[.above(draft.conditions[1].id)]
        XCTAssertNotNil(message)
        XCTAssertFalse(message?.isEmpty ?? true)
        guard case .failure(let refused) = draft.save(into: settings) else {
            return XCTFail("saved a rule with an unreadable threshold")
        }
        XCTAssertEqual(refused.byField, problems)
    }

    func testANamelessRuleIsRefusedOnItsName() {
        var draft = AlertDraft.new(id: "n", spotID: SeedSpots.croton.id)
        draft.conditions[0].below = "500"
        guard case .failure(let refused) = draft.save(into: settings) else {
            return XCTFail("saved a rule without a name")
        }
        XCTAssertNotNil(refused.byField[.name])
        draft.name = "Low water"
        guard case .success(let json) = draft.save(into: settings) else {
            return XCTFail("refused a rule with a name")
        }
        XCTAssertEqual(AlertRuleDocument.find("n", inSettings: json)?.name, "Low water")
    }

    func testABoxOverOneDegreeIsRefusedInTheCoresWords() {
        var draft = AlertDraft.new(id: "b", spotID: nil)
        draft.name = "Warm water"
        draft.targetKind = .box
        draft.conditions[0].metric = "sea_surface_temp"
        draft.conditions[0].above = "68"
        let large = AlertDraft.Box(south: 40, west: -74, north: 41.5, east: -73.5)
        draft.box = large
        guard case .failure(let refused) = draft.save(into: settings) else {
            return XCTFail("saved a box over 1°")
        }
        let message = refused.byField[.target]
        XCTAssertNotNil(message)
        XCTAssertTrue(message?.contains("1°") ?? false, message ?? "")
        XCTAssertEqual(AlertBoxFrame.problem(large, settingsJSON: settings), message)
        let small = AlertDraft.Box(south: 40, west: -74, north: 40.5, east: -73.5)
        XCTAssertNil(AlertBoxFrame.problem(small, settingsJSON: settings))
        draft.box = small
        guard case .success = draft.save(into: settings) else {
            return XCTFail("refused a box under 1°")
        }
    }

    func testABoxRuleRefusesAValueNotGridded() {
        var draft = AlertDraft.new(id: "g", spotID: nil)
        draft.name = "Flow in a box"
        draft.targetKind = .box
        draft.box = AlertDraft.Box(south: 40, west: -74, north: 40.5, east: -73.5)
        draft.conditions[0].below = "500"
        guard case .failure(let refused) = draft.save(into: settings) else {
            return XCTFail("saved a box rule on river flow")
        }
        XCTAssertNotNil(refused.byField[.condition(draft.conditions[0].id)])
        XCTAssertEqual(AlertDraft.kinds(for: .box), [.measurement])
        XCTAssertTrue(AlertDraft.kinds(for: .trip).contains(.tripGo))
        XCTAssertFalse(AlertDraft.kinds(for: .spot).contains(.tripGo))
    }

    func testRefusedPathsPointAtTheirFields() {
        var draft = AlertDraft.new(id: "p", spotID: "s")
        draft.conditions.append(AlertDraft.ConditionDraft(kind: .stormAhead))
        let second = draft.conditions[1].id
        XCTAssertEqual(draft.field(of: "alerts[3].name"), .name)
        XCTAssertEqual(draft.field(of: "alerts[3].target.north"), .target)
        XCTAssertEqual(draft.field(of: "alerts[3].all_of"), .conditions)
        XCTAssertEqual(draft.field(of: "alerts[3].all_of[1]"), .condition(second))
        XCTAssertEqual(draft.field(of: "alerts[3].all_of[1].above"), .above(second))
        XCTAssertEqual(draft.field(of: "alerts[3].all_of[1].below"), .below(second))
        XCTAssertEqual(draft.field(of: "alerts[3].all_of[7]"), .conditions)
        XCTAssertEqual(draft.field(of: "alerts[3].quiet_hours.end_minute"), .quietHours)
        XCTAssertEqual(draft.field(of: "alerts[3].repeat_after_hours"), .repeatHours)
        XCTAssertEqual(draft.field(of: "rule"), .other)
    }

    func testTheFrameHoldsTheMiddleOfTheMap() {
        let box = AlertBoxFrame.box(west: -1, south: -1, east: 1, north: 1)
        XCTAssertEqual(box.west, -0.8, accuracy: 1e-9)
        XCTAssertEqual(box.east, 0.8, accuracy: 1e-9)
        XCTAssertEqual(box.south, -0.8, accuracy: 1e-3)
        XCTAssertEqual(box.north, 0.8, accuracy: 1e-3)
        // Mercator: far north, a degree takes more of the map at the top than at the bottom,
        // so the frame's top edge is fewer degrees in from the map's than its bottom edge.
        let north = AlertBoxFrame.box(west: -20, south: 60, east: 0, north: 70)
        XCTAssertLessThan(70 - north.north, north.south - 60)
        XCTAssertEqual(north.west, -18, accuracy: 1e-9)
    }

    func testAMapOpenedForABoxHoldsItInTheFrame() {
        let box = AlertDraft.Box(south: 40.2, west: -74, north: 40.7, east: -73.4)
        let start = AlertBoxFrame.start(for: box)
        XCTAssertEqual(start.longitude, -73.7, accuracy: 1e-9)
        XCTAssertGreaterThan(start.span, 0.5 / (1 - 2 * AlertBoxFrame.inset) - 0.01)
        XCTAssertLessThan(start.span, 0.5 / (1 - 2 * AlertBoxFrame.inset) + 0.01)
    }
}
