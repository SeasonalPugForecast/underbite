import Foundation
import XCTest

@testable import UnderbiteKit

/// The species screens' reading of the core: the list and its search, a profile page with its
/// sources and licences, the seasons, the editor's units, and what a picker returns.
final class SpeciesCatalogueTests: XCTestCase {
    private static let bass = "morone-saxatilis"
    private let settings = defaultSettingsJson()

    private var rows: [SpeciesListRow] { SpeciesCatalogue.rows(edits: "", settingsJSON: settings) }

    func testTheListIsTheCataloguesByNameWithIds() throws {
        let rows = rows
        XCTAssertGreaterThanOrEqual(rows.count, 25)
        let bass = try XCTUnwrap(rows.first { $0.id == Self.bass })
        XCTAssertEqual(bass.name, "Striped bass")
        XCTAssertEqual(bass.scientificName, "Morone saxatilis")
        XCTAssertNotNil(bass.waterLabel)
        XCTAssertFalse(bass.edited)
        XCTAssertEqual(SpeciesCatalogue.rows(edits: "", settingsJSON: "not json"), [])
    }

    func testSearchReadsCommonAndScientificNamesIgnoringCaseAndAccents() {
        let rows = rows
        func ids(_ query: String) -> [String] { SpeciesSearch.matching(query, in: rows).map(\.id) }
        XCTAssertEqual(SpeciesSearch.matching("  ", in: rows), rows)
        XCTAssertEqual(ids("STRIPED"), [Self.bass])
        XCTAssertEqual(ids("morone SAX"), [Self.bass])
        XCTAssertEqual(ids("Mórone"), [Self.bass], "accents are ignored")
        XCTAssertEqual(ids("strïped bäss"), [Self.bass])
        XCTAssertTrue(ids("bass").contains("micropterus-dolomieu"))
        XCTAssertTrue(ids("SALMO").contains("salmo-trutta"))
        XCTAssertEqual(ids("no such fish anywhere"), [])
    }

    func testTheStripedBassPageNamesEachSourceAndTheFishBaseLicence() throws {
        let page = try SpeciesCatalogue.profile(id: Self.bass, edits: "", settingsJSON: settings)
        XCTAssertEqual(page.id, Self.bass)
        XCTAssertEqual(page.name, "Striped bass")
        XCTAssertFalse(page.edited)
        let sourced = page.fields.filter { $0.source != nil }
        XCTAssertFalse(sourced.isEmpty)
        let fishBase = sourced.filter { $0.source?.licence == "cc_by_nc4" }
        XCTAssertFalse(fishBase.isEmpty, "FishBase-derived fields")
        for field in fishBase {
            XCTAssertEqual(field.source?.licenceLabel, "CC BY-NC 4.0", field.label)
            XCTAssertEqual(field.source?.name, "FishBase", field.label)
        }
        XCTAssertFalse(page.notices.isEmpty, "the CC BY-NC notice")
        XCTAssertTrue(page.credits.contains { $0.licence == "CC BY-NC 4.0" })
        XCTAssertTrue(page.fields.contains { $0.field == "spawn_temp" })
        XCTAssertTrue(page.fields.contains { $0.field == nil }, "the scientific name is not edited")
        XCTAssertThrowsError(
            try SpeciesCatalogue.profile(id: "no-such-fish", edits: "", settingsJSON: settings))
    }

    func testTheSeasonsKeepTheLegalSectionToLinksAndTheDisclaimer() throws {
        let ny = try SpeciesCatalogue.seasons(
            id: Self.bass, edits: "", snapshotJSON: nil, region: " us-ny ", settingsJSON: settings,
            now: 1_800_000_000)
        XCTAssertEqual(ny.id, Self.bass)
        XCTAssertEqual(ny.typical.months.map(\.month), Array(1...12))
        XCTAssertEqual(ny.legal.region, "US-NY")
        XCTAssertFalse(ny.legal.links.isEmpty)
        for link in ny.legal.links {
            XCTAssertTrue(link.link.hasPrefix("https://"), link.link)
        }
        XCTAssertFalse(ny.legal.disclaimer.isEmpty)
        XCTAssertFalse(ny.now.stageLabel.isEmpty)

        let nowhere = try SpeciesCatalogue.seasons(
            id: Self.bass, edits: "", snapshotJSON: nil, region: "", settingsJSON: settings,
            now: 1_800_000_000)
        XCTAssertNil(nowhere.legal.region)
        XCTAssertTrue(nowhere.legal.links.isEmpty)
        XCTAssertNotNil(nowhere.legal.empty)
        XCTAssertEqual(nowhere.legal.disclaimer, ny.legal.disclaimer, "always shown")
    }

    func testTheEditorsKindsChoicesAndUnits() throws {
        XCTAssertEqual(SpeciesEditKind(field: "lures"), .texts)
        XCTAssertEqual(SpeciesEditKind(field: "water"), .one)
        XCTAssertEqual(SpeciesEditKind(field: "light"), .many)
        XCTAssertEqual(SpeciesEditKind(field: "spawn_temp"), .temperature)
        XCTAssertEqual(SpeciesEditKind(field: "depth"), .depth)
        XCTAssertEqual(
            SpeciesCatalogue.choices(field: "water", settingsJSON: settings).map(\.id),
            ["fresh", "salt", "both"])
        XCTAssertEqual(SpeciesCatalogue.choices(field: "lures", settingsJSON: settings), [])

        var metric = try Settings(json: settings)
        metric.units.waterTemp = "C"
        let metricJSON = try metric.json()
        XCTAssertEqual(SpeciesCatalogue.unitLabel(field: "spawn_temp", settingsJSON: metricJSON), "°C")
        XCTAssertFalse(SpeciesCatalogue.unitLabel(field: "depth", settingsJSON: settings).isEmpty)
        XCTAssertEqual(SpeciesCatalogue.unitLabel(field: "lures", settingsJSON: settings), "")
    }

    func testAPickerReturnsIdsAndKeepsOnesTheCatalogueLacks() {
        let rows = rows
        var trip = SpeciesSelection(ids: ["some-newer-fish"], allowsMany: true)
        trip.choose(Self.bass)
        trip.choose("pomatomus-saltatrix")
        XCTAssertEqual(trip.ids, ["some-newer-fish", Self.bass, "pomatomus-saltatrix"])
        trip.choose(Self.bass)
        XCTAssertEqual(trip.ids, ["some-newer-fish", "pomatomus-saltatrix"])
        XCTAssertEqual(trip.unknown(in: rows), ["some-newer-fish"])
        XCTAssertEqual(trip.chosenRows(in: rows).map(\.name), ["Bluefish"])

        var one = SpeciesSelection(ids: [], allowsMany: false)
        one.choose(Self.bass)
        one.choose("pomatomus-saltatrix")
        XCTAssertEqual(one.ids, ["pomatomus-saltatrix"])
    }
}
