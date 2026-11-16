import Foundation
import XCTest

@testable import UnderbiteKit

/// `viewModel` in the real core on this machine: the time-zone database and the formatting
/// work outside Linux, and the page decodes into the Swift types.
final class ViewModelCoreTests: XCTestCase {
    private func entry() -> SpotEntry {
        SpotEntry(
            spot: SpotInfo(
                id: "battery", name: "The Battery", latitude: 40.7003, longitude: -74.0142,
                waterType: "tidal_river_estuary", timeZone: "America/New_York"),
            snapshotJson: SampleSnapshot.json, lastSuccess: SampleSnapshot.fetchedAt, error: nil,
            refreshing: false)
    }

    private func tideTimes(_ page: ViewModel.Page) throws -> [String] {
        let view = try XCTUnwrap(page.view)
        let tide = try XCTUnwrap(
            view.cards.compactMap { card -> ViewModel.TideCard? in
                if case .tide(let tide) = card { tide } else { nil }
            }.first)
        return tide.full.next.map(\.time)
    }

    func testViewModelGivesLocalTimeLabelsForNewYork() throws {
        let now = Date(timeIntervalSince1970: TimeInterval(SampleSnapshot.fetchedAt + 360))
        let page = try ViewModel.build(
            spots: [entry()], active: 0, settings: Settings.defaults(), now: now)
        XCTAssertEqual(page.spots.first?.name, "The Battery")
        XCTAssertEqual(page.spots.first?.status.updated, "updated 6 min ago")
        // 1_794_000_600 is 21:30 UTC: 4:30 PM in New York.
        XCTAssertEqual(try tideTimes(page).first, "4:30 PM")
        let kinds = try XCTUnwrap(page.view).cards.compactMap { $0.chrome?.id }
        XCTAssertEqual(kinds.first, "now")
        XCTAssertTrue(kinds.contains("tide"))
    }

    func testTheClockSettingChangesTheLabels() throws {
        var settings = try Settings.defaults()
        settings.presentation.clock = "24h"
        let now = Date(timeIntervalSince1970: TimeInterval(SampleSnapshot.fetchedAt + 360))
        let page = try ViewModel.build(spots: [entry()], active: 0, settings: settings, now: now)
        XCTAssertEqual(try tideTimes(page).first, "16:30")
    }

    func testASpotWithoutASnapshotHasNoView() throws {
        var bare = entry()
        bare.snapshotJson = nil
        let page = try ViewModel.build(
            spots: [bare], active: 0, settings: Settings.defaults(), now: Date(timeIntervalSince1970: 1_794_000_360))
        XCTAssertNil(page.view)
        XCTAssertEqual(page.spots.count, 1)
    }
}
