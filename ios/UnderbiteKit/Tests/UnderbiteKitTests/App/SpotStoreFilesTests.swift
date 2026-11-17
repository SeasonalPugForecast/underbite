import Foundation
import XCTest

@testable import UnderbiteKit

/// The files the store reads at launch: keys missing from them take their defaults, and a file
/// it cannot read is set aside, reported and never written over.
@MainActor
final class SpotStoreFilesTests: XCTestCase {
    /// A directory of its own for each test (XCTest makes an instance per test).
    private let directory = FileManager.default.temporaryDirectory
        .appending(path: "underbite-files-\(UUID().uuidString)", directoryHint: .isDirectory)

    override func setUp() {
        StubProtocol.answer(.failure(URLError(.notConnectedToInternet)))
    }

    override func tearDown() {
        try? FileManager.default.removeItem(at: directory)
    }

    nonisolated private static let noKeys = Keys(
        usgs: nil, copernicusUser: nil, copernicusPassword: nil)

    private var files: AppFiles { AppFiles(directory: directory) }

    private func store() -> SpotStore {
        let host = URLSessionHost(session: StubProtocol.session(), storeURL: files.hostDirectory)
        return SpotStore(
            files: files, host: host, keys: { Self.noKeys }, clock: { 1_800_000_000 },
            lowPower: { false },
            fetcher: { _, _, _, _, _, _ in
                SpotFetchOutcome(snapshotJson: "{}", errors: [], suggestion: nil)
            })
    }

    private func spotJSON(_ spot: Spot) throws -> String {
        try XCTUnwrap(String(data: JSONEncoder().encode(spot), encoding: .utf8))
    }

    func testSpotsSavedWithoutSomeKeysLoadWithTheirDefaults() throws {
        let spot = Spot(id: "a", name: "A", latitude: 40.7033, longitude: -74.017, waterType: "pond")
        // No areas, refresh interval or sun mode; no version either.
        files.write(Data(#"{"spots": [\#(try spotJSON(spot))]}"#.utf8), "state.json")
        let s = store()
        XCTAssertEqual(s.spots, [spot])
        XCTAssertEqual(s.areas, [])
        XCTAssertEqual(s.refreshMinutes, RefreshPolicy.defaultMinutes)
        XCTAssertFalse(s.sunMode)
        XCTAssertEqual(s.activeID, "a")
        XCTAssertFalse(FileManager.default.fileExists(atPath: files.url("state.json.bad").path))
    }

    func testUnreadableSpotsAreSetAsideAndNeverOverwritten() {
        files.write(Data("not json".utf8), "state.json")
        let s = store()
        XCTAssertTrue(s.spots.isEmpty)
        XCTAssertEqual(files.text("state.json.bad"), "not json")
        s.add(Spot(id: "a", name: "A", latitude: 1, longitude: 2, waterType: "pond"))
        XCTAssertEqual(files.text("state.json.bad"), "not json", "the set-aside file stays")
        XCTAssertTrue(files.text("state.json")?.contains("\"a\"") ?? false)

        // Unreadable again: a second file aside, the first one kept.
        files.write(Data("garbage".utf8), "state.json")
        _ = store()
        XCTAssertEqual(files.text("state.json.bad"), "not json")
        XCTAssertEqual(files.text("state.json.2.bad"), "garbage")
    }

    func testSpotsOfALaterVersionAreSetAside() throws {
        let spot = Spot(id: "a", name: "A", latitude: 1, longitude: 2, waterType: "pond")
        let later = #"{"version": 2, "spots": [\#(try spotJSON(spot))]}"#
        files.write(Data(later.utf8), "state.json")
        let s = store()
        XCTAssertTrue(s.spots.isEmpty)
        XCTAssertEqual(files.text("state.json.bad"), later)
    }

    func testUnreadableSettingsAreSetAsideAndNeverOverwritten() {
        files.write(Data("not json".utf8), "settings.json")
        let s = store()
        XCTAssertEqual(s.settingsJSON, defaultSettingsJson())
        XCTAssertEqual(files.text("settings.json.bad"), "not json")
        s.saveSettingsNow()
        XCTAssertEqual(files.text("settings.json.bad"), "not json")
        XCTAssertEqual(files.text("settings.json"), defaultSettingsJson())

        files.write(Data([0xFF, 0xFE, 0x00]), "settings.json")
        _ = store()
        XCTAssertEqual(try? Data(contentsOf: files.url("settings.json.2.bad")), Data([0xFF, 0xFE, 0x00]))
    }

    func testUnreadableFilesAreReportedWithWhereTheyAreKept() {
        files.write(Data("not json".utf8), "state.json")
        files.write(Data("not json".utf8), "settings.json")
        let s = store()
        XCTAssertEqual(
            s.fileProblems,
            [
                FileProblem(file: .spots, keptAs: "state.json.bad"),
                FileProblem(file: .settings, keptAs: "settings.json.bad"),
            ])
        XCTAssertTrue(store().fileProblems.isEmpty, "set aside, they are not read again")
    }

    func testAFileThatCannotBeMovedAsideIsNeverWrittenOver() throws {
        files.write(Data("not json".utf8), "state.json")
        // A `.bad` name for every attempt is taken by a directory, so nothing can move there.
        for name in ["state.json.bad"] + (2...100).map({ "state.json.\($0).bad" }) {
            try FileManager.default.createDirectory(
                at: files.url(name), withIntermediateDirectories: true)
        }
        let s = store()
        XCTAssertEqual(s.fileProblems, [FileProblem(file: .spots, keptAs: nil)])
        s.add(Spot(id: "a", name: "A", latitude: 1, longitude: 2, waterType: "pond"))
        XCTAssertEqual(files.text("state.json"), "not json")
    }
}
