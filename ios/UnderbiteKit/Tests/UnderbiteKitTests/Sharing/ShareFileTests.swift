import Foundation
import XCTest

@testable import UnderbiteKit

/// Writing a trip as a `.underbite` file with the real core: the location choice, the file's
/// name and folder, and each level of precision.
final class ShareFileTests: XCTestCase {
    /// A directory of its own for each test (XCTest makes an instance per test).
    private let directory = FileManager.default.temporaryDirectory
        .appending(path: "underbite-share-\(UUID().uuidString)", directoryHint: .isDirectory)

    override func tearDown() {
        try? FileManager.default.removeItem(at: directory)
    }

    private static let spots = [SeedSpots.battery, SeedSpots.croton]
    private let settings = defaultSettingsJson()

    /// Three days at the test places.
    private var tripJSON: String {
        var draft = TripDraft(first: "2027-02-01", days: 3, places: Self.spots.map(\.id))
        draft.id = "shared-trip"
        draft.name = "Hudson stripers"
        return draft.json(spots: Self.spots, areas: [], settingsJSON: settings)
    }

    private func object(_ data: Data) throws -> [String: Any] {
        try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
    }

    /// The first place of a file.
    private func firstPlace(_ file: UnderbiteFile) throws -> [String: Any] {
        let trip = try XCTUnwrap(object(file.data)["trip"] as? [String: Any])
        return try XCTUnwrap((trip["places"] as? [[String: Any]])?.first)
    }

    func testTheLocationChoiceIsTheCoresWithAbout1KmFirstOffered() throws {
        let words = try XCTUnwrap(ShareText.words(settingsJSON: settings))
        XCTAssertEqual(words.fuzz.map(\.fuzz), [.exact, .rounded1Km, .areaOnly])
        XCTAssertEqual(words.defaultFuzz, .rounded1Km)
        XCTAssertFalse(words.fuzzTitle.isEmpty)
        XCTAssertTrue(words.fuzz.allSatisfy { !$0.label.isEmpty && !$0.detail.isEmpty })
        XCTAssertNotNil(ShareText.words(settingsJSON: "not settings"), "the default voice's")
    }

    func testATripFileIsNamedAfterTheTripAndWrittenInAFolderOfItsOwn() throws {
        let file = try UnderbiteFile.trip(tripJSON, fuzz: .rounded1Km)
        XCTAssertEqual(file.fileName, "Hudson stripers.underbite")
        let a = try file.write(in: directory)
        let b = try file.write(in: directory)
        XCTAssertNotEqual(a.deletingLastPathComponent(), b.deletingLastPathComponent())
        XCTAssertEqual(a.lastPathComponent, file.fileName)
        XCTAssertEqual(try Data(contentsOf: a), file.data)
        let text = try object(file.data)
        XCTAssertEqual(text["format"] as? String, "underbite")
        XCTAssertEqual(text["kind"] as? String, "trip")
    }

    func testFilesSentBeforeAreSweptAwayAndExportsAtLaunch() throws {
        let file = try UnderbiteFile.trip(tripJSON, fuzz: .exact)
        let a = try file.write(in: directory)
        let b = try file.write(in: directory)
        let manager = FileManager.default
        let export = directory.appending(
            path: CatchStore.exportPrefix + "left", directoryHint: .isDirectory)
        let other = directory.appending(path: "other", directoryHint: .isDirectory)
        for folder in [export, other] {
            try manager.createDirectory(at: folder, withIntermediateDirectories: true)
        }

        UnderbiteFile.sweep(in: directory)
        XCTAssertFalse(manager.fileExists(atPath: a.deletingLastPathComponent().path))
        XCTAssertFalse(manager.fileExists(atPath: b.deletingLastPathComponent().path))
        XCTAssertTrue(manager.fileExists(atPath: export.path), "an export may still be shown")
        XCTAssertTrue(manager.fileExists(atPath: other.path), "not the app's")

        UnderbiteFile.sweep(in: directory, exports: true)
        XCTAssertFalse(manager.fileExists(atPath: export.path))
        XCTAssertTrue(manager.fileExists(atPath: other.path))
    }

    func testFileNamesHoldNoCharacterAFileNameCannot() {
        XCTAssertEqual(
            UnderbiteFile.fileName(for: "Bay/Harbor: day 1"), "Bay-Harbor- day 1.underbite")
        XCTAssertEqual(UnderbiteFile.fileName(for: "..hidden"), "hidden.underbite")
        XCTAssertEqual(UnderbiteFile.fileName(for: "  "), "Underbite.underbite")
        XCTAssertEqual(
            UnderbiteFile.fileName(for: String(repeating: "a", count: 200)).count, 90)
    }

    func testEachLevelOfPrecisionIsTheCores() throws {
        let exact = try firstPlace(UnderbiteFile.trip(tripJSON, fuzz: .exact))
        XCTAssertEqual(exact["latitude"] as? Double, 40.7033)
        let rounded = try firstPlace(UnderbiteFile.trip(tripJSON, fuzz: .rounded1Km))
        XCTAssertEqual(rounded["latitude"] as? Double, 40.7)
        XCTAssertEqual(rounded["fuzz"] as? String, "rounded_1km")
        let area = try firstPlace(UnderbiteFile.trip(tripJSON, fuzz: .areaOnly))
        XCTAssertNil(area["latitude"])
        XCTAssertNil(area["longitude"])
        XCTAssertEqual(area["name"] as? String, "The Battery")
    }

    func testATripTheCoreCannotWriteSaysWhyInItsWords() {
        XCTAssertThrowsError(try UnderbiteFile.trip("{}", fuzz: .exact)) { error in
            XCTAssertFalse(ShareText.message(of: error).isEmpty)
        }
        XCTAssertFalse(ShareText.message(of: CocoaError(.fileNoSuchFile)).isEmpty)
    }
}
