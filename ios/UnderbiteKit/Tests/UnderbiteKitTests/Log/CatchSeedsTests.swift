import Foundation
import XCTest

@testable import UnderbiteKit

/// The sample catches of screenshots: five at the test places, each with the conditions of its
/// spot, typed in the user's units.
@MainActor
final class CatchSeedsTests: XCTestCase {
    private let directory = FileManager.default.temporaryDirectory
        .appending(path: "underbite-seeds-\(UUID().uuidString)", directoryHint: .isDirectory)

    override func tearDown() {
        try? FileManager.default.removeItem(at: directory)
    }

    private static let places = [
        PlaceName(spotId: SeedSpots.battery.id, name: SeedSpots.battery.name),
        PlaceName(spotId: SeedSpots.croton.id, name: SeedSpots.croton.name),
    ]

    private func store() -> CatchStore {
        CatchStore(files: AppFiles(directory: directory), clock: { 1_800_000_000 })
    }

    func testTheSeedsAreFiveCatchesWithConditionsAndSeedingAgainKeepsFive() async throws {
        let s = store()
        CatchSeeds.seed(catches: s, settingsJSON: "{}")
        XCTAssertEqual(s.count, 5)
        CatchSeeds.seed(catches: s, settingsJSON: "{}")
        XCTAssertEqual(s.count, 5, "seeding again updates the same catches")

        await s.rebuild(settingsJSON: "{}", places: Self.places)
        let rows = try XCTUnwrap(s.list?.rows)
        XCTAssertEqual(rows.map(\.id), CatchSeeds.seeds.map(\.id), "newest first")
        XCTAssertTrue(rows.allSatisfy(\.hasConditions), "\(rows.map(\.summary))")
        XCTAssertEqual(Set(rows.compactMap(\.place)), ["The Battery", "Croton River"])

        let bass = try XCTUnwrap(
            s.detail(CatchSeeds.seeds[1].id, settingsJSON: "{}", places: Self.places))
        let kinds = Set(bass.conditions.map(\.kind))
        for kind in ["tide", "water_temperature", "pressure", "pressure_trend", "wind"] {
            XCTAssertTrue(kinds.contains(kind), "\(kind) in \(kinds)")
        }
        XCTAssertTrue(
            bass.conditions.allSatisfy { $0.footnote.contains("The Battery") },
            "each condition names its station: \(bass.conditions.map(\.footnote))")

        let trout = try XCTUnwrap(
            s.detail(CatchSeeds.seeds[0].id, settingsJSON: "{}", places: Self.places))
        XCTAssertTrue(trout.conditions.contains { $0.kind == "flow" })
        XCTAssertFalse(trout.conditions.contains { $0.kind == "tide" })
    }

    func testTheSeedsTypeTheirSizesInTheUsersUnits() {
        let metric = FishUnitsInfo(
            lengthUnit: "centimetres", lengthLabel: "cm", weightUnit: "kilograms",
            weightLabel: "kg", ouncesLabel: "")
        let input = CatchSeeds.input(CatchSeeds.seeds[1], units: metric)
        XCTAssertEqual(input.lengthText, CatchSeeds.seeds[1].centimetres)
        XCTAssertEqual(input.weightText, CatchSeeds.seeds[1].kilograms)
        XCTAssertEqual(input.spotId, SeedSpots.battery.id)
    }

    func testManyAddsASeasonOfStripedBassWithTheirTides() async throws {
        XCTAssertEqual(CatchSeeds.seeds(for: ["app", "-seedCatches"]).count, 5)
        XCTAssertEqual(CatchSeeds.seeds(for: ["app", "-seedCatches", "-tab", "log"]).count, 5)
        let many = CatchSeeds.seeds(for: ["app", "-seedCatches", "many", "-tab", "log"])
        XCTAssertEqual(many.count, CatchSeeds.seeds.count + CatchSeeds.more.count)
        XCTAssertEqual(Set(many.map(\.id)).count, many.count, "every id is its own")
        XCTAssertTrue(many.allSatisfy { UUID(uuidString: $0.id) != nil })

        let s = store()
        CatchSeeds.seed(catches: s, settingsJSON: "{}", chosen: many)
        XCTAssertEqual(s.count, many.count)
        await s.rebuild(settingsJSON: "{}", places: Self.places)
        let rows = try XCTUnwrap(s.list?.rows)
        XCTAssertTrue(rows.allSatisfy(\.hasConditions), "\(rows.map(\.summary))")

        // A catch past the low has a rising tide; the five keep their falling one.
        let rising = try XCTUnwrap(CatchSeeds.more.first { $0.sinceHigh > 22_500 })
        let detail = try XCTUnwrap(s.detail(rising.id, settingsJSON: "{}", places: Self.places))
        let tide = try XCTUnwrap(detail.conditions.first { $0.kind == "tide" })
        XCTAssertTrue(tide.value.localizedCaseInsensitiveContains("rising"), tide.value)
        let first = try XCTUnwrap(
            s.detail(CatchSeeds.seeds[1].id, settingsJSON: "{}", places: Self.places))
        let ebb = try XCTUnwrap(first.conditions.first { $0.kind == "tide" })
        XCTAssertTrue(ebb.value.localizedCaseInsensitiveContains("falling"), ebb.value)
    }
}
