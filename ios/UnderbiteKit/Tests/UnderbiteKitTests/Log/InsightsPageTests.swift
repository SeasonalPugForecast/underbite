import Foundation
import XCTest

@testable import UnderbiteKit

/// What worked: the core's page read for the seeded catches, the filters turned into the core's,
/// and the choices the filters offer.
@MainActor
final class InsightsPageTests: XCTestCase {
    private let directory = FileManager.default.temporaryDirectory
        .appending(path: "underbite-insights-\(UUID().uuidString)", directoryHint: .isDirectory)

    override func tearDown() {
        try? FileManager.default.removeItem(at: directory)
    }

    private static let places = [
        PlaceName(spotId: SeedSpots.battery.id, name: SeedSpots.battery.name),
        PlaceName(spotId: SeedSpots.croton.id, name: SeedSpots.croton.name),
    ]

    private func store(_ seeds: [CatchSeeds.Seed]) -> CatchStore {
        let s = CatchStore(files: AppFiles(directory: directory), clock: { 1_800_000_000 })
        CatchSeeds.seed(catches: s, settingsJSON: "{}", chosen: seeds)
        return s
    }

    private static let bass = CatchFilter(
        species: "morone-saxatilis", spotId: nil, area: nil, from: nil, until: nil, method: nil)

    func testTheSeasonOfStripedBassShowsTideAndWaterTemperatureBars() throws {
        let s = store(CatchSeeds.seeds + CatchSeeds.more)
        let page = try XCTUnwrap(
            InsightsPage.build(book: s.book, filter: Self.bass, settingsJSON: "{}"))
        XCTAssertEqual(page.total, CatchSeeds.more.count + 1)
        XCTAssertNil(page.empty)
        XCTAssertNil(page.tooFew)
        XCTAssertFalse(page.notice.isEmpty)

        let tide = try XCTUnwrap(page.sections.first { $0.kind == "tide_direction" })
        XCTAssertEqual(tide.bars.map(\.key), ["rising", "falling"])
        XCTAssertEqual(tide.bars.filter(\.peak).map(\.key), ["falling"])
        XCTAssertEqual(tide.bars.map(\.count).reduce(0, +), page.total)
        XCTAssertTrue(tide.bars.allSatisfy { (0...1).contains($0.fraction) })
        XCTAssertEqual(tide.bars.first { $0.peak }?.fraction, 1)
        let sentence = try XCTUnwrap(tide.sentence)
        XCTAssertTrue(sentence.contains("triped bass"), sentence)

        let water = try XCTUnwrap(page.sections.first { $0.kind == "water_temperature" })
        XCTAssertGreaterThanOrEqual(water.bars.count, 3)
        XCTAssertEqual(water.bars.filter(\.peak).map(\.key), ["from-12"])
        for bar in tide.bars + water.bars {
            XCTAssertFalse(bar.label.isEmpty)
            XCTAssertTrue(bar.spoken.contains(bar.label), "\(bar.spoken) names \(bar.label)")
        }

        // Charted sections come first; those without enough catches follow in the core's order.
        let arranged = page.arranged
        XCTAssertEqual(Set(arranged.map(\.kind)), Set(page.sections.map(\.kind)))
        let firstUncharted = arranged.firstIndex { !$0.charted } ?? arranged.count
        XCTAssertTrue(arranged[firstUncharted...].allSatisfy { !$0.charted && $0.tooFew != nil })
        XCTAssertEqual(arranged.first?.kind, "tide_direction")
        XCTAssertFalse(
            try XCTUnwrap(page.sections.first { $0.kind == "flow" }).charted,
            "no striped bass has a flow")
    }

    func testFiveCatchesAreTooFewToTell() throws {
        let s = store(CatchSeeds.seeds)
        let all = CatchFilter(
            species: nil, spotId: nil, area: nil, from: nil, until: nil, method: nil)
        let page = try XCTUnwrap(InsightsPage.build(book: s.book, filter: all, settingsJSON: "{}"))
        XCTAssertEqual(page.total, 5)
        XCTAssertNotNil(page.tooFew)
        XCTAssertTrue(page.sections.isEmpty)

        let empty = CatchStore(
            files: AppFiles(directory: directory.appending(path: "empty")), clock: { 0 })
        let none = try XCTUnwrap(
            InsightsPage.build(book: empty.book, filter: all, settingsJSON: "{}"))
        XCTAssertEqual(none.total, 0)
        XCTAssertNotNil(none.empty)
    }

    func testThePlaceFilterCountsOnlyItsCatches() throws {
        let s = store(CatchSeeds.seeds + CatchSeeds.more)
        var filter = InsightsFilter()
        filter.place = .spot(SeedSpots.croton.id)
        let calendar = Calendar(identifier: .gregorian)
        let croton = try XCTUnwrap(
            InsightsPage.build(
                book: s.book, filter: filter.catchFilter(calendar: calendar), settingsJSON: "{}"))
        XCTAssertEqual(croton.total, 2)
        filter.place = .spot(SeedSpots.battery.id)
        let battery = try XCTUnwrap(
            InsightsPage.build(
                book: s.book, filter: filter.catchFilter(calendar: calendar), settingsJSON: "{}"))
        XCTAssertEqual(battery.total, CatchSeeds.seeds.count + CatchSeeds.more.count - 2)
    }

    func testDaysRunFromTheStartOfTheFirstToTheEndOfTheLast() throws {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = try XCTUnwrap(TimeZone(identifier: "America/New_York"))
        // 2026-05-02 06:20 and 2026-05-09 19:40 at the Battery.
        let filter = InsightsFilter(
            species: "morone-saxatilis", place: .area("Pier A"),
            from: Date(timeIntervalSince1970: 1_777_717_200),
            until: Date(timeIntervalSince1970: 1_778_370_000))
        let core = filter.catchFilter(calendar: calendar)
        XCTAssertEqual(core.species, "morone-saxatilis")
        XCTAssertNil(core.spotId)
        XCTAssertEqual(core.area, "Pier A")
        XCTAssertEqual(core.from, 1_777_694_400, "2026-05-02 00:00 EDT")
        XCTAssertEqual(core.until, 1_778_385_600, "2026-05-10 00:00 EDT")
        XCTAssertNil(InsightsFilter().catchFilter(calendar: calendar).from)
        XCTAssertFalse(InsightsFilter().isActive)
        XCTAssertTrue(filter.isActive)
    }

    func testTheChoicesAreTheLogsSpeciesAndPlaces() async throws {
        let s = store(CatchSeeds.seeds + CatchSeeds.more)
        await s.rebuild(settingsJSON: "{}", places: Self.places)
        let rows = try XCTUnwrap(s.list?.rows)
        let catalogue = SpeciesChoice.catalogue(settingsJSON: "{}")
        XCTAssertFalse(catalogue.isEmpty)
        let typed = CatchRow(
            id: "typed", species: "Mystery fish", size: nil, when: "", place: "Pier A",
            fate: .released, fateLabel: "Released", hasConditions: false, summary: "")
        let choices = InsightsChoices(rows: rows + [typed], catalogue: catalogue, spots: Self.places)
        XCTAssertEqual(
            choices.species.map(\.id),
            [
                "salmo-trutta", "morone-saxatilis", "pomatomus-saltatrix", "centropristis-striata",
                "micropterus-dolomieu",
            ], "newest first, each once; a typed species has no id to count by")
        XCTAssertEqual(
            choices.places,
            [
                .init(place: .spot(SeedSpots.croton.id), name: SeedSpots.croton.name),
                .init(place: .spot(SeedSpots.battery.id), name: SeedSpots.battery.name),
                .init(place: .area("Pier A"), name: "Pier A"),
            ])

        var filter = InsightsFilter(species: "esox-lucius", place: .area("Gone"))
        filter = filter.pruned(for: choices)
        XCTAssertEqual(filter, InsightsFilter())
        let kept = InsightsFilter(species: "morone-saxatilis", place: .area("Pier A"))
        XCTAssertEqual(kept.pruned(for: choices), kept)
    }

    func testRowsHoldTheirColumns() {
        let sections = (0..<5).map {
            InsightsPage.Section(
                kind: "k\($0)", title: "", bars: [], sentence: nil, tooFew: nil, unknown: nil)
        }
        XCTAssertEqual(
            InsightsPage.rows(sections, columns: 2).map { $0.map(\.kind) },
            [["k0", "k1"], ["k2", "k3"], ["k4"]])
        XCTAssertEqual(InsightsPage.rows(sections, columns: 1).count, 5)
        XCTAssertEqual(InsightsPage.rows(sections, columns: 0).count, 5)
        XCTAssertTrue(InsightsPage.rows([], columns: 2).isEmpty)
    }
}
