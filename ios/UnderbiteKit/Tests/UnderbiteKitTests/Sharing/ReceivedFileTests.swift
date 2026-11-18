import Foundation
import XCTest

@testable import UnderbiteKit

/// Received `.underbite` files with the real core: the preview of a trip or a spot, the core's
/// text for a file this build cannot use, what the preview offers, and adding, copying and
/// replacing a trip without ever making a place more precise than this device holds it.
@MainActor
final class ReceivedFileTests: XCTestCase {
    /// A directory of its own for each test (XCTest makes an instance per test).
    private let directory = FileManager.default.temporaryDirectory
        .appending(path: "underbite-sharing-\(UUID().uuidString)", directoryHint: .isDirectory)

    override func tearDown() {
        try? FileManager.default.removeItem(at: directory)
    }

    /// 2027-01-15 08:00 UTC.
    nonisolated private static let now: Int64 = 1_800_000_000
    private static let spots = [SeedSpots.battery, SeedSpots.croton]
    private let settings = defaultSettingsJson()

    /// Three days at the test places.
    private func tripJSON(id: String = "shared-trip", name: String = "Hudson stripers") -> String {
        var draft = TripDraft(first: "2027-02-01", days: 3, places: Self.spots.map(\.id))
        draft.id = id
        draft.name = name
        draft.notes = "Incoming tide at the Battery."
        return draft.json(spots: Self.spots, areas: [], settingsJSON: settings)
    }

    private func object(_ json: String) throws -> [String: Any] {
        try XCTUnwrap(JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any])
    }

    private func places(_ json: String) throws -> [[String: Any]] {
        try XCTUnwrap(object(json)["places"] as? [[String: Any]])
    }

    private func words() throws -> ShareWords {
        try XCTUnwrap(ShareText.words(settingsJSON: settings))
    }

    // MARK: - Reading

    func testTheReceivedTripKeepsTheSendersPrecision() throws {
        let rounded = try UnderbiteFile.trip(tripJSON(), fuzz: .rounded1Km)
        let read = ReceivedFile.read(rounded.data, settingsJSON: settings, now: Self.now)
        guard case .trip(let json, _) = read.content else { return XCTFail("\(read)") }
        XCTAssertEqual(try places(json).first?["latitude"] as? Double, 40.7)

        let area = try UnderbiteFile.trip(tripJSON(), fuzz: .areaOnly)
        let readArea = ReceivedFile.read(area.data, settingsJSON: settings, now: Self.now)
        guard case .trip(let areaJSON, _) = readArea.content else { return XCTFail("\(readArea)") }
        XCTAssertNil(try places(areaJSON).first?["latitude"])
        XCTAssertEqual(readArea.approximate, "Approximate location")
    }

    func testAReceivedTripPreviewsWithTheApproximateLocationNote() throws {
        let file = try UnderbiteFile.trip(tripJSON(), fuzz: .rounded1Km)
        let read = ReceivedFile.read(file.data, settingsJSON: settings, now: Self.now)
        XCTAssertEqual(read.heading, try words().receiveTrip)
        XCTAssertEqual(read.approximate, "Approximate location")
        guard case .trip(_, let listing) = read.content else { return XCTFail("\(read)") }
        XCTAssertEqual(listing.title, "Hudson stripers")
        XCTAssertFalse(listing.dates.isEmpty, "the core words the dates")
        XCTAssertEqual(listing.places.map(\.name), ["The Battery", "Croton River"])
        XCTAssertEqual(listing.notes, "Incoming tide at the Battery.")
    }

    func testAnExactFileIsNotApproximate() throws {
        let file = try UnderbiteFile.trip(tripJSON(), fuzz: .exact)
        let read = ReceivedFile.read(file.data, settingsJSON: settings, now: Self.now)
        XCTAssertNil(read.approximate)
    }

    func testAReceivedSpotPreviewsAndAddsWithItsCoordinatesUnlessSharedByArea() throws {
        let battery = SeedSpots.battery
        let exact = try spotFile(
            spot: battery.info, area: "Lower Hudson", region: "US-NY", notes: "Flood tide.",
            fuzz: .exact)
        let read = ReceivedFile.read(Data(exact.utf8), settingsJSON: settings, now: Self.now)
        XCTAssertEqual(read.heading, try words().receiveSpot)
        XCTAssertNil(read.approximate)
        guard case .spot(let place) = read.content else { return XCTFail("\(read)") }
        XCTAssertEqual(place.name, "The Battery")
        XCTAssertEqual(place.area, "Lower Hudson")
        XCTAssertEqual(place.notes, "Flood tide.")
        XCTAssertFalse(place.waterTypeLabel.isEmpty)
        XCTAssertNotEqual(place.waterTypeLabel, battery.waterType, "the core's name, not the id")
        XCTAssertEqual(read.spotToAdd(.add)?.latitude, battery.latitude)
        XCTAssertEqual(read.spotToAdd(.add)?.id, battery.id)
        XCTAssertEqual(read.spotToAdd(.addCopy) { "copy" }?.id, "copy")

        let area = try spotFile(
            spot: battery.info, area: "Lower Hudson", region: nil, notes: "", fuzz: .areaOnly)
        let readArea = ReceivedFile.read(Data(area.utf8), settingsJSON: settings, now: Self.now)
        XCTAssertEqual(readArea.approximate, "Approximate location")
        XCTAssertNil(readArea.spotToAdd(.add))
        XCTAssertEqual(readArea.choices(trips: [], spots: []), [])
    }

    func testFilesThisBuildCannotUseShowTheCoresText() {
        // A preset without its layers is a damaged file, not one of a kind this build lacks.
        let layers = Data(#"{"format":"underbite","kind":"layers","v":1}"#.utf8)
        let newer = Data(#"{"format":"underbite","kind":"trip","v":99}"#.utf8)
        let junk = Data("not a file".utf8)
        for (data, expected) in [(layers, "not a file"), (newer, "newer version"), (junk, "")] {
            let read = ReceivedFile.read(data, settingsJSON: settings, now: Self.now)
            XCTAssertNil(read.heading)
            guard case .problem(let message) = read.content else {
                XCTFail("\(read)")
                continue
            }
            XCTAssertFalse(message.isEmpty)
            XCTAssertFalse(message.contains("ShareError"), message)
            if !expected.isEmpty { XCTAssertTrue(message.contains(expected), message) }
            XCTAssertEqual(read.choices(trips: [], spots: []), [])
        }
    }

    func testAFileTooLargeIsReadOnlyAsFarAsTheCoresLimit() throws {
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let url = directory.appending(path: "big.underbite")
        try Data(repeating: 0x20, count: 3_000_000).write(to: url)
        XCTAssertEqual(try ReceivedFile.read(url).count, ReceivedFile.maxBytes + 1)
        let read = ReceivedFile.open(url, settingsJSON: settings, now: Self.now, inbox: nil)
        guard case .problem(let message) = read.content else { return XCTFail("\(read)") }
        XCTAssertFalse(message.isEmpty)
        XCTAssertTrue(FileManager.default.fileExists(atPath: url.path), "kept outside the inbox")
    }

    func testTheInboxCopyIsRemovedOnceRead() throws {
        let inbox = directory.appending(path: "Inbox", directoryHint: .isDirectory)
        try FileManager.default.createDirectory(at: inbox, withIntermediateDirectories: true)
        let url = try UnderbiteFile.trip(tripJSON(), fuzz: .exact).write(in: inbox)
        let read = ReceivedFile.open(url, settingsJSON: settings, now: Self.now, inbox: inbox)
        guard case .trip = read.content else { return XCTFail("\(read)") }
        XCTAssertFalse(FileManager.default.fileExists(atPath: url.path))
    }

    // MARK: - Adding

    private func tripStore() -> TripStore {
        TripStore(files: AppFiles(directory: directory), clock: { Self.now })
    }

    func testANewTripIsAddedUnderItsOwnId() throws {
        let trips = tripStore()
        let file = try UnderbiteFile.trip(tripJSON(), fuzz: .rounded1Km)
        let read = ReceivedFile.read(file.data, settingsJSON: settings, now: Self.now)
        XCTAssertEqual(read.choices(trips: trips.trips, spots: []), [.add])
        XCTAssertFalse(read.exists(trips: trips.trips, spots: []))
        let id = try read.acceptTrip(.add, trips: trips)
        XCTAssertEqual(id, "shared-trip")
        XCTAssertEqual(try places(XCTUnwrap(trips.trip("shared-trip")).json).first?["fuzz"] as? String, "rounded_1km")
    }

    func testATripAlreadyHereIsAddedAsACopyOrReplacesMineKeepingMyExactPlaces() throws {
        let trips = tripStore()
        try trips.save(tripJSON())
        let file = try UnderbiteFile.trip(tripJSON(name: "Hudson stripers, again"), fuzz: .rounded1Km)
        let read = ReceivedFile.read(file.data, settingsJSON: settings, now: Self.now)
        XCTAssertEqual(read.choices(trips: trips.trips, spots: []), [.addCopy, .replace])
        XCTAssertTrue(read.exists(trips: trips.trips, spots: []))

        let copy = try XCTUnwrap(try read.acceptTrip(.addCopy, trips: trips))
        XCTAssertNotEqual(copy, "shared-trip")
        XCTAssertEqual(trips.trips.count, 2)

        XCTAssertEqual(try read.acceptTrip(.replace, trips: trips), "shared-trip")
        XCTAssertEqual(trips.trips.count, 2)
        let mine = try object(XCTUnwrap(trips.trip("shared-trip")).json)
        XCTAssertEqual(mine["name"] as? String, "Hudson stripers, again")
        let place = try XCTUnwrap((mine["places"] as? [[String: Any]])?.first)
        XCTAssertEqual(place["latitude"] as? Double, 40.7033, "a rounded copy never moves mine")
        XCTAssertNotEqual(place["fuzz"] as? String, "rounded_1km")
    }

    func testASpotAlreadyHereIsOnlyAddedAsACopy() throws {
        let file = try spotFile(
            spot: SeedSpots.croton.info, area: nil, region: nil, notes: "", fuzz: .rounded1Km)
        let read = ReceivedFile.read(Data(file.utf8), settingsJSON: settings, now: Self.now)
        XCTAssertEqual(read.choices(trips: [], spots: []), [.add])
        XCTAssertEqual(read.choices(trips: [], spots: [SeedSpots.croton]), [.addCopy])
        XCTAssertTrue(read.exists(trips: [], spots: [SeedSpots.croton]))
    }

    func testTheSampleOfScreenshotsIsAnApproximateTrip() {
        let read = ReceivedFile.sample(today: "2027-01-15", settingsJSON: settings, now: Self.now)
        XCTAssertEqual(read.approximate, "Approximate location")
        guard case .trip(_, let listing) = read.content else { return XCTFail("\(read)") }
        XCTAssertEqual(listing.places.count, 2)
    }
}
