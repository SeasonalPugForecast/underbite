import Foundation
import XCTest

@testable import UnderbiteKit

/// The trip store: trips kept in `trips.json` as the core's JSON, ids and times set on save,
/// deleting, the list's order, and files it cannot read set aside.
@MainActor
final class TripStoreTests: XCTestCase {
    /// A directory of its own for each test (XCTest makes an instance per test).
    private let directory = FileManager.default.temporaryDirectory
        .appending(path: "underbite-trips-\(UUID().uuidString)", directoryHint: .isDirectory)

    override func tearDown() {
        try? FileManager.default.removeItem(at: directory)
    }

    private var files: AppFiles { AppFiles(directory: directory) }

    /// 2027-01-15 08:00 UTC, 03:00 at the test places.
    private static let now: Int64 = 1_800_000_000

    private func store(now: Int64 = TripStoreTests.now) -> TripStore {
        TripStore(files: files, clock: { now })
    }

    private static let spots = [SeedSpots.battery, SeedSpots.croton]

    /// A new trip at the test places, from `first` to `last`.
    private func trip(_ name: String, _ first: String, _ last: String) -> String {
        var draft = TripDraft(first: first, places: Self.spots.map(\.id))
        draft.name = name
        draft.last = last
        return draft.json(spots: Self.spots, areas: [], settingsJSON: "{}")
    }

    private func object(_ json: String) throws -> [String: Any] {
        try XCTUnwrap(JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any])
    }

    func testSavedTripsReadBackInANewStore() throws {
        let s = store()
        let a = try s.save(trip("Hudson weekend", "2027-02-01", "2027-02-03"))
        let b = try s.save(trip("Croton morning", "2027-03-10", "2027-03-10"))
        XCTAssertNotEqual(a, b)
        XCTAssertEqual(s.trips.map(\.id), [a, b])
        XCTAssertNotNil(UUID(uuidString: a), "a new trip's id is a UUID")
        let saved = try object(XCTUnwrap(s.trip(a)).json)
        XCTAssertEqual(saved["id"] as? String, a)
        XCTAssertEqual((saved["modified_at"] as? NSNumber)?.int64Value, Self.now)
        XCTAssertEqual(saved["first"] as? String, "2027-02-01")
        XCTAssertEqual((saved["places"] as? [[String: Any]])?.count, 2)

        let again = store()
        XCTAssertEqual(again.trips, s.trips)
        XCTAssertFalse(again.unreadable)
        let text = try XCTUnwrap(files.text(TripStore.fileName))
        XCTAssertTrue(text.contains(#""latitude":40.7033,"#), "the core's own numbers: \(text)")
        let file = try object(text)
        XCTAssertEqual((file["version"] as? NSNumber)?.intValue, 1)
        XCTAssertEqual((file["trips"] as? [Any])?.count, 2)
    }

    func testSavingAnEditedTripKeepsItsIdAndSetsItsTime() throws {
        let id = try store(now: 100).save(trip("Hudson weekend", "2027-02-01", "2027-02-03"))
        let later = store(now: 200)
        var draft = TripDraft(json: try XCTUnwrap(later.trip(id)).json)
        draft.name = "Hudson long weekend"
        XCTAssertEqual(try later.save(draft.json(spots: Self.spots, areas: [], settingsJSON: "{}")), id)
        XCTAssertEqual(later.trips.count, 1)
        let saved = try object(XCTUnwrap(later.trip(id)).json)
        XCTAssertEqual(saved["name"] as? String, "Hudson long weekend")
        XCTAssertEqual((saved["modified_at"] as? NSNumber)?.int64Value, 200)
        XCTAssertEqual(store().trip(id), later.trip(id))
    }

    func testDeletingATripRemovesItFromTheFile() throws {
        let s = store()
        let a = try s.save(trip("A", "2027-02-01", "2027-02-03"))
        let b = try s.save(trip("B", "2027-02-05", "2027-02-06"))
        let before = s.revision
        s.remove(a)
        XCTAssertEqual(s.trips.map(\.id), [b])
        XCTAssertGreaterThan(s.revision, before)
        XCTAssertEqual(store().trips.map(\.id), [b])
        s.remove("no-such-trip")
        XCTAssertEqual(store().trips.map(\.id), [b])
    }

    func testUpcomingTripsComeSoonestFirstAndPastOnesLatestFirst() async throws {
        let s = store()
        let longAgo = try s.save(trip("Long ago", "2026-12-01", "2026-12-03"))
        let lastWeek = try s.save(trip("Last week", "2027-01-05", "2027-01-07"))
        let underWay = try s.save(trip("Under way", "2027-01-14", "2027-01-16"))
        let later = try s.save(trip("Later", "2027-02-01", "2027-02-03"))
        let soon = try s.save(trip("Soon", "2027-01-20", "2027-01-21"))
        await s.rebuild(settingsJSON: defaultSettingsJson())
        XCTAssertEqual(s.upcoming.map(\.id), [underWay, soon, later])
        XCTAssertEqual(s.past.map(\.id), [lastWeek, longAgo])

        let row = try XCTUnwrap(s.listing(soon))
        XCTAssertEqual(row.title, "Soon")
        XCTAssertFalse(row.dates.isEmpty, "the core words the dates")
        XCTAssertFalse(row.status.isEmpty)
        XCTAssertEqual(row.places.map(\.name), ["The Battery", "Croton River"])
        XCTAssertNil(row.problem)

        s.remove(soon)
        XCTAssertEqual(s.upcoming.map(\.id), [underWay, later])
    }

    func testUnreadableTripsAreSetAsideAndNeverOverwritten() throws {
        files.write(Data("not json".utf8), TripStore.fileName)
        let s = store()
        XCTAssertTrue(s.trips.isEmpty)
        XCTAssertTrue(s.unreadable)
        XCTAssertEqual(s.keptAs, "trips.json.bad")
        XCTAssertEqual(files.text("trips.json.bad"), "not json")
        try s.save(trip("A", "2027-02-01", "2027-02-03"))
        XCTAssertEqual(files.text("trips.json.bad"), "not json", "the set-aside file stays")
        XCTAssertEqual(store().trips.count, 1)
    }

    func testTripsOfALaterVersionAreSetAside() {
        let later = #"{"version": 2, "trips": []}"#
        files.write(Data(later.utf8), TripStore.fileName)
        let s = store()
        XCTAssertTrue(s.unreadable)
        XCTAssertEqual(files.text("trips.json.bad"), later)
    }

    func testATripWithoutAnIdInTheFileGetsOne() throws {
        let json = trip("A", "2027-02-01", "2027-02-03")
        var object = try object(json)
        object.removeValue(forKey: "id")
        let file = #"{"version": 1, "trips": [\#(try TripStore.text(of: object))]}"#
        files.write(Data(file.utf8), TripStore.fileName)
        let s = store()
        XCTAssertEqual(s.trips.count, 1)
        XCTAssertFalse(try XCTUnwrap(s.trips.first).id.isEmpty)
    }

    func testWhatTheCoreCannotReadIsNotSaved() {
        let s = store()
        XCTAssertThrowsError(try s.save(#"{"name": "No dates"}"#))
        XCTAssertThrowsError(try s.save("not json"))
        XCTAssertTrue(s.trips.isEmpty)
        XCTAssertFalse(files.exists(TripStore.fileName))
    }

    func testTheSampleTripIsThreeDaysAtTheTestPlaces() async throws {
        let json = TripSeeds.trip(today: "2027-01-15", spots: Self.spots)
        XCTAssertNil(tripProblem(tripJson: json))
        let s = store()
        XCTAssertEqual(try s.save(json), TripSeeds.id)
        XCTAssertEqual(try s.save(json), TripSeeds.id, "seeding again updates the same trip")
        XCTAssertEqual(s.trips.count, 1)
        let saved = try object(XCTUnwrap(s.trip(TripSeeds.id)).json)
        XCTAssertEqual(saved["first"] as? String, "2027-01-18")
        XCTAssertEqual(saved["last"] as? String, "2027-01-20")
        await s.rebuild(settingsJSON: defaultSettingsJson())
        XCTAssertEqual(s.upcoming.first?.places.map(\.name), ["The Battery", "Croton River"])
    }
}
