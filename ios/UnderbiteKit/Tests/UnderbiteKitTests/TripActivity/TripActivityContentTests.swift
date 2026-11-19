import Foundation
import XCTest

@testable import UnderbiteKit

/// The trip-day Live Activity's content: the core's, from the places' last snapshots, on the
/// trip's days only.
final class TripActivityContentTests: XCTestCase {
    /// A directory of its own for each test (XCTest makes an instance per test).
    private let directory = FileManager.default.temporaryDirectory
        .appending(path: "underbite-trip-activity-\(UUID().uuidString)", directoryHint: .isDirectory)

    override func tearDown() {
        try? FileManager.default.removeItem(at: directory)
    }

    private var files: AppFiles { AppFiles(directory: directory) }

    /// Just after the sample snapshot of The Battery was fetched, on 2026-11-06 in New York.
    private let now = SampleSnapshot.fetchedAt + 60

    private func trip(first: String, days: Int = 1) -> SavedTrip {
        var draft = TripDraft(
            first: first, days: days, places: [SeedSpots.battery.id, SeedSpots.croton.id])
        draft.id = "trip-activity-test"
        draft.name = "Hudson stripers"
        return SavedTrip(
            id: draft.id,
            json: draft.json(
                spots: [SeedSpots.battery, SeedSpots.croton], areas: [], settingsJSON: "{}"))
    }

    /// The Battery's sample snapshot; the Croton River has none yet.
    private func snapshot(_ key: String) -> String? {
        key == SeedSpots.battery.id ? SampleSnapshot.json : nil
    }

    private func current(_ trip: SavedTrip, at now: Int64) -> ViewModel.TripDayActivity? {
        TripActivityContent.current(
            trip, settingsJSON: defaultSettingsJson(), snapshot: snapshot, files: files, now: now)
    }

    func testOnATripDayTheContentIsTheCoresForThePlaceWithATide() throws {
        let trip = trip(first: "2026-11-06")
        XCTAssertTrue(TripActivityContent.isTripDay(trip, settingsJSON: defaultSettingsJson(), now: now))
        let content = try XCTUnwrap(current(trip, at: now))
        XCTAssertEqual(content.title, "Hudson stripers")
        XCTAssertEqual(content.place, "The Battery")
        XCTAssertEqual(content.generatedAt, now)
        // The sample's next high, ten minutes after it was fetched.
        let tide = try XCTUnwrap(content.nextTideChange)
        XCTAssertEqual(tide.at, SampleSnapshot.fetchedAt + 600)
        XCTAssertFalse(tide.label.isEmpty)
        XCTAssertFalse(tide.text.isEmpty)
        XCTAssertEqual(content.staleAt, tide.at)
        XCTAssertGreaterThan(content.endsAt, now)
        XCTAssertTrue(content.credit.contains("NOAA CO-OPS"), content.credit)
        XCTAssertTrue(content.notices.contains { $0.kind == "not_for_navigation" })
        XCTAssertTrue(TripActivityMatch.shows(content, at: Date(timeIntervalSince1970: TimeInterval(now))))
    }

    func testOnAnyDayOfTheTripTheContentIsBuilt() throws {
        let trip = trip(first: "2026-11-05", days: 3)
        XCTAssertNotNil(current(trip, at: now))
    }

    func testBeforeTheTripThereIsNoContent() {
        let trip = trip(first: "2026-11-09", days: 2)
        XCTAssertFalse(TripActivityContent.isTripDay(trip, settingsJSON: defaultSettingsJson(), now: now))
        XCTAssertNil(current(trip, at: now))
        // The content itself can still be built (the debug preview's).
        XCTAssertNotNil(
            TripActivityContent.activity(
                trip, settingsJSON: defaultSettingsJson(), snapshot: snapshot, files: files, now: now))
    }

    func testAfterTheTripThereIsNoContent() {
        let trip = trip(first: "2026-11-03", days: 2)
        XCTAssertNil(current(trip, at: now))
    }

    func testTheTripsViewSaysWhetherTodayIsATripDay() throws {
        let trip = trip(first: "2026-11-06")
        let today = try ViewModel.buildTrip(
            tripJSON: trip.json, places: [], cachedMetadata: [], settingsJSON: defaultSettingsJson(),
            now: Date(timeIntervalSince1970: TimeInterval(now)))
        XCTAssertTrue(TripActivityContent.isTripDay(today))
        // A view given is taken at its word: built three days before, today is not a trip day.
        let before = try ViewModel.buildTrip(
            tripJSON: trip.json, places: [], cachedMetadata: [], settingsJSON: defaultSettingsJson(),
            now: Date(timeIntervalSince1970: TimeInterval(now - 3 * 86_400)))
        XCTAssertFalse(TripActivityContent.isTripDay(before))
        XCTAssertNil(
            TripActivityContent.current(
                trip, view: before, settingsJSON: defaultSettingsJson(), snapshot: snapshot,
                files: files, now: now))
    }

    func testWithoutSnapshotsThereIsNothingToCountDownTo() throws {
        let content = try XCTUnwrap(
            TripActivityContent.current(
                trip(first: "2026-11-06"), settingsJSON: defaultSettingsJson(), snapshot: { _ in nil },
                files: files, now: now))
        XCTAssertTrue(content.targets.isEmpty)
        XCTAssertFalse(TripActivityMatch.shows(content, at: Date(timeIntervalSince1970: TimeInterval(now))))
    }
}
