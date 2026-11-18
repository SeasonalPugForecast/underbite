import Foundation
import XCTest

@testable import UnderbiteKit

/// The imported tracks in `tracks.json`: kept across launches, the same track kept once, and a
/// file that cannot be read set aside and never written over.
@MainActor
final class TrackStoreTests: XCTestCase {
    /// A directory of its own for each test (XCTest makes an instance per test).
    private let directory = FileManager.default.temporaryDirectory
        .appending(path: "underbite-tracks-\(UUID().uuidString)", directoryHint: .isDirectory)

    override func tearDown() {
        try? FileManager.default.removeItem(at: directory)
    }

    private var files: AppFiles { AppFiles(directory: directory) }

    private func track(_ id: String, name: String? = "Croton River walk", shift: Double = 0)
        -> SavedTrack
    {
        SavedTrack(
            id: id, name: name, file: "walk.gpx", importedAt: 1_800_000_000,
            segments: [
                [
                    .init(latitude: 41.2108 + shift, longitude: -73.8625, time: 1_746_882_000),
                    .init(latitude: 41.2112 + shift, longitude: -73.8631, time: nil),
                ]
            ])
    }

    func testTracksAreKeptAcrossLaunches() throws {
        let store = TrackStore(files: files)
        XCTAssertTrue(store.tracks.isEmpty)
        XCTAssertFalse(files.exists(TrackStore.fileName), "nothing is written before a track")
        XCTAssertEqual(try store.add([track("a"), track("b", name: nil, shift: 0.01)]), ["a", "b"])
        XCTAssertEqual(store.revision, 1)

        let again = TrackStore(files: files)
        XCTAssertEqual(again.tracks, store.tracks)
        XCTAssertFalse(again.unreadable)

        again.remove("a")
        XCTAssertEqual(TrackStore(files: files).tracks.map(\.id), ["b"])
    }

    func testTheFileHoldsItsVersionAndTheTracks() throws {
        let store = TrackStore(files: files)
        try store.add([track("a")])
        let data = try XCTUnwrap(files.data(TrackStore.fileName))
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        XCTAssertEqual(object["version"] as? Int, 1)
        let tracks = try XCTUnwrap(object["tracks"] as? [[String: Any]])
        XCTAssertEqual(tracks.first?["name"] as? String, "Croton River walk")
        XCTAssertEqual(tracks.first?["file"] as? String, "walk.gpx")
    }

    func testTheSameTrackIsKeptOnce() throws {
        let store = TrackStore(files: files)
        try store.add([track("a")])
        XCTAssertEqual(try store.add([track("b")]), [], "same name and points")
        XCTAssertEqual(try store.add([track("c", name: "Other")]), ["c"])
        XCTAssertEqual(store.tracks.map(\.id), ["a", "c"])
    }

    func testAnUnreadableFileIsSetAsideAndNeverWrittenOver() throws {
        files.write(Data("{ not json".utf8), TrackStore.fileName)
        let store = TrackStore(files: files)
        XCTAssertTrue(store.unreadable)
        XCTAssertEqual(store.keptAs, "tracks.json.bad")
        XCTAssertTrue(store.canSave)
        XCTAssertEqual(files.text("tracks.json.bad"), "{ not json")

        try store.add([track("a")])
        XCTAssertEqual(TrackStore(files: files).tracks.map(\.id), ["a"])
        XCTAssertEqual(files.text("tracks.json.bad"), "{ not json")
    }

    func testALaterVersionIsNotRead() throws {
        files.write(Data(#"{"tracks":[],"version":2}"#.utf8), TrackStore.fileName)
        let store = TrackStore(files: files)
        XCTAssertTrue(store.unreadable)
        XCTAssertEqual(store.keptAs, "tracks.json.bad")
    }

    func testNothingIsSavedWhenTheUnreadableFileCannotBeMoved() throws {
        files.write(Data("{ not json".utf8), TrackStore.fileName)
        // Every name it could be kept under is taken.
        files.write(Data(), "tracks.json.bad")
        for attempt in 2...100 { files.write(Data(), "tracks.json.\(attempt).bad") }
        let store = TrackStore(files: files)
        XCTAssertTrue(store.unreadable)
        XCTAssertNil(store.keptAs)
        XCTAssertFalse(store.canSave)
        XCTAssertThrowsError(try store.add([track("a")]))
        XCTAssertEqual(files.text(TrackStore.fileName), "{ not json")
    }
}
