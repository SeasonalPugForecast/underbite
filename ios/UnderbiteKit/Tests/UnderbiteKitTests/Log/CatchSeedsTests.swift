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
}
