import Foundation
import XCTest

@testable import UnderbiteKit

/// Importing waypoints and tracks with the real core: the preview of a file, the core's words
/// for a file it cannot read, waypoints repeating a saved spot flagged and left unchosen, files
/// read once (an unpacked KMZ folder through its KML), and the chosen waypoints added as spots
/// and tracks saved.
@MainActor
final class ImportPreviewTests: XCTestCase {
    /// A directory of its own for each test (XCTest makes an instance per test).
    private let directory = FileManager.default.temporaryDirectory
        .appending(path: "underbite-import-\(UUID().uuidString)", directoryHint: .isDirectory)

    override func setUp() {
        try? FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        StubProtocol.answer(.failure(URLError(.notConnectedToInternet)))
    }

    override func tearDown() {
        try? FileManager.default.removeItem(at: directory)
    }

    private let settings = defaultSettingsJson()

    private func sample(spots: [Spot] = [SeedSpots.battery]) -> ImportPreview {
        ImportPreview.read(
            ImportSample.gpx, fileName: ImportSample.fileName, spots: spots, settingsJSON: settings)
    }

    private func write(_ text: String, _ name: String) throws -> URL {
        let url = directory.appending(path: name, directoryHint: .notDirectory)
        try FileManager.default.createDirectory(
            at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        try Data(text.utf8).write(to: url)
        return url
    }

    private func waypoint(_ name: String?, _ latitude: Double, _ longitude: Double)
        -> ImportedWaypoint
    {
        ImportedWaypoint(name: name, latitude: latitude, longitude: longitude, time: nil, notes: nil)
    }

    // MARK: - Reading

    func testTheSampleShowsItsWaypointsAndTrackWithTheCoresSummary() throws {
        let preview = sample(spots: [])
        XCTAssertNil(preview.problem)
        XCTAssertEqual(preview.summary, "2 waypoints, 1 track (5 points)")
        XCTAssertEqual(preview.waypoints.map(\.point.name), ["The Battery", "Croton mouth"])
        XCTAssertEqual(preview.waypoints.first?.point.notes, "Stripers on the push")
        XCTAssertEqual(preview.tracks.map(\.track.name), ["Croton River walk"])
        XCTAssertEqual(preview.tracks.first?.track.segments.map(\.count), [3, 2])
        XCTAssertEqual(preview.chosenWaypoints, [0, 1])
        XCTAssertEqual(preview.chosenTracks, [0])
        XCTAssertTrue(preview.hasChoice)
    }

    func testAFileTheCoreCannotReadShowsItsWords() {
        let unknown = ImportPreview.read(
            Data("hello".utf8), fileName: "notes.txt", spots: [], settingsJSON: settings)
        XCTAssertEqual(unknown.problem, "the file is not GPX, KML, KMZ or CSV")
        XCTAssertTrue(unknown.isEmpty)
        XCTAssertFalse(unknown.hasChoice)

        let broken = ImportPreview.read(
            Data("<gpx><wpt lat=\"91\" lon=\"0\"></wpt></gpx>".utf8), fileName: "bad.gpx",
            spots: [], settingsJSON: settings)
        let problem = try? XCTUnwrap(broken.problem)
        XCTAssertTrue(problem?.contains("line") == true, problem ?? "no problem")
        XCTAssertEqual(broken.summary, "")
    }

    func testAFileOverTheLimitIsTooLargeInTheCoresWords() throws {
        let url = directory.appending(path: "huge.gpx", directoryHint: .notDirectory)
        FileManager.default.createFile(atPath: url.path, contents: nil)
        let handle = try FileHandle(forWritingTo: url)
        try handle.truncate(atOffset: UInt64(ImportPreview.maxBytes + 10))
        try handle.close()
        let preview = ImportPreview.open(url, spots: [], settingsJSON: settings, inbox: nil)
        XCTAssertEqual(
            preview.problem,
            "the file is \(ImportPreview.maxBytes + 1) bytes; the limit is \(ImportPreview.maxBytes) bytes")
    }

    // MARK: - Duplicates

    func testAWaypointNamedLikeASpotWithin10MetresIsFlaggedAndLeftUnchosen() {
        let preview = sample()
        XCTAssertEqual(preview.waypoints.map(\.duplicateOf), [SeedSpots.battery.id, nil])
        XCTAssertEqual(preview.chosenWaypoints, [1])
    }

    func testOnlyTheSameNameWithin10MetresIsADuplicate() {
        let battery = SeedSpots.battery
        let lat = battery.latitude
        let lon = battery.longitude
        let find = { (point: ImportedWaypoint) in
            ImportPreview.duplicate(of: point, in: [battery], settingsJSON: self.settings)?.id
        }
        // About 5.6 m north, another case and spaces around it.
        XCTAssertEqual(find(self.waypoint("  the battery ", lat + 0.00005, lon)), battery.id)
        // About 22 m north.
        XCTAssertNil(find(self.waypoint("The Battery", lat + 0.0002, lon)))
        XCTAssertNil(find(self.waypoint("Battery wall", lat, lon)))
        XCTAssertNil(find(self.waypoint(nil, lat, lon)))
    }

    func testAnUnnamedWaypointIsNamedByItsPositionInTheCoresWords() {
        let point = waypoint(nil, 41.192, -73.883)
        let words = try? pointWords(point: MapPoint(lat: 41.192, lon: -73.883), settingsJson: settings)
        XCTAssertEqual(ImportPreview.name(of: point, settingsJSON: settings), words?.point)
        XCTAssertFalse(ImportPreview.name(of: point, settingsJSON: settings).isEmpty)
        XCTAssertEqual(ImportPreview.name(of: waypoint("  ", 0, 0), settingsJSON: settings),
            ImportPreview.pointText(latitude: 0, longitude: 0, settingsJSON: settings))
    }

    func testDistancesAreGreatCircleMetres() {
        // One minute of latitude is about 1852 m.
        XCTAssertEqual(ImportPreview.metres(40, -74, 40 + 1.0 / 60, -74), 1853, accuracy: 3)
        XCTAssertEqual(ImportPreview.metres(40.7033, -74.017, 40.7033, -74.017), 0)
    }

    // MARK: - Files

    func testAFileIsReadOnceAndItsInboxCopyRemoved() throws {
        let inbox = directory.appending(path: "Inbox", directoryHint: .isDirectory)
        let text = String(decoding: ImportSample.gpx, as: UTF8.self)
        let url = try write(text, "Inbox/walk.gpx")
        let preview = ImportPreview.open(url, spots: [], settingsJSON: settings, inbox: inbox)
        XCTAssertEqual(preview.fileName, "walk.gpx")
        XCTAssertEqual(preview.waypoints.count, 2)
        XCTAssertFalse(FileManager.default.fileExists(atPath: url.path))

        let kept = try write(text, "Elsewhere/walk.gpx")
        _ = ImportPreview.open(kept, spots: [], settingsJSON: settings, inbox: inbox)
        XCTAssertTrue(FileManager.default.fileExists(atPath: kept.path))
    }

    func testAnUnpackedKMZFolderIsReadThroughItsKML() throws {
        let kml = """
            <?xml version="1.0" encoding="UTF-8"?>
            <kml xmlns="http://www.opengis.net/kml/2.2"><Document>
              <Placemark><name>The Battery</name>
                <Point><coordinates>-74.0170,40.7033</coordinates></Point></Placemark>
            </Document></kml>
            """
        _ = try write(kml, "Harbor.kmz/doc.kml")
        _ = try write("not a placemark", "Harbor.kmz/files/readme.txt")
        let folder = directory.appending(path: "Harbor.kmz", directoryHint: .isDirectory)
        let preview = ImportPreview.open(folder, spots: [], settingsJSON: settings, inbox: nil)
        XCTAssertNil(preview.problem)
        XCTAssertEqual(preview.fileName, "Harbor.kmz")
        XCTAssertEqual(preview.waypoints.map(\.point.name), ["The Battery"])
    }

    func testAFolderWithoutKMLSaysWhatTheCoreMakesOfIt() throws {
        _ = try write("nothing", "Empty.kmz/readme.txt")
        let folder = directory.appending(path: "Empty.kmz", directoryHint: .isDirectory)
        let preview = ImportPreview.open(folder, spots: [], settingsJSON: settings, inbox: nil)
        XCTAssertNotNil(preview.problem)
        XCTAssertTrue(preview.isEmpty)
    }

    // MARK: - Choosing and adding

    func testTheChosenWaypointsBecomeSpotsWhoseWaterTypeIsSuggested() {
        var preview = sample()
        preview.toggleWaypoint(0)
        preview.toggleWaypoint(1)
        XCTAssertEqual(preview.chosenWaypoints, [0])
        var count = 0
        let spots = preview.spotsToAdd(settingsJSON: settings) {
            count += 1
            return "new-\(count)"
        }
        XCTAssertEqual(spots.map(\.id), ["new-1"])
        XCTAssertEqual(spots.first?.name, "The Battery")
        XCTAssertEqual(spots.first?.latitude, 40.7033)
        XCTAssertEqual(spots.first?.longitude, -74.017)
        XCTAssertEqual(spots.first?.waterType, Spot.unknownWaterType)
        XCTAssertEqual(spots.first?.waterTypeChosen, false)
    }

    func testTheChosenTracksAreSavedWithTheirFileAndTime() {
        var preview = sample()
        let saved = preview.tracksToSave(now: 1_800_000_000) { "track-1" }
        XCTAssertEqual(saved.count, 1)
        XCTAssertEqual(saved.first?.id, "track-1")
        XCTAssertEqual(saved.first?.name, "Croton River walk")
        XCTAssertEqual(saved.first?.file, ImportSample.fileName)
        XCTAssertEqual(saved.first?.importedAt, 1_800_000_000)
        XCTAssertEqual(saved.first?.segments.first?.first?.latitude, 41.2108)
        XCTAssertEqual(saved.first?.segments.first?.first?.time, 1_746_882_000)
        XCTAssertNil(saved.first?.segments.last?.last?.time)
        preview.toggleTrack(0)
        XCTAssertTrue(preview.tracksToSave(now: 0).isEmpty)
    }

    func testAddingPutsTheSpotsInTheStoreAndTheTracksInTracksJSON() throws {
        let files = AppFiles(directory: directory)
        let host = URLSessionHost(session: StubProtocol.session(), storeURL: files.hostDirectory)
        let spots = SpotStore(files: files, host: host)
        spots.add(SeedSpots.battery)
        let tracks = TrackStore(files: files)
        let preview = ImportPreview.read(
            ImportSample.gpx, fileName: ImportSample.fileName, spots: spots.spots,
            settingsJSON: spots.settingsJSON)

        let ids = preview.add(to: spots, tracks: tracks, now: 1_800_000_000)
        XCTAssertEqual(ids.count, 1, "the flagged Battery waypoint is not added again")
        XCTAssertEqual(spots.spots.map(\.name), ["The Battery", "Croton mouth"])
        XCTAssertEqual(tracks.tracks.map(\.name), ["Croton River walk"])
        XCTAssertEqual(TrackStore(files: files).tracks.map(\.name), ["Croton River walk"])

        // The same file again: nothing new but what the user chooses.
        let again = ImportPreview.read(
            ImportSample.gpx, fileName: ImportSample.fileName, spots: spots.spots,
            settingsJSON: spots.settingsJSON)
        XCTAssertEqual(again.chosenWaypoints, [])
        XCTAssertEqual(again.add(to: spots, tracks: tracks, now: 1_800_000_100), [])
        XCTAssertEqual(tracks.tracks.count, 1, "the same track is kept once")
    }

    // MARK: - The map

    func testALongSegmentIsThinnedKeepingItsEnds() {
        let segment = Array(0..<10_001)
        let thinned = ImportPreview.thinned(segment, limit: 1_000)
        XCTAssertEqual(thinned.count, 1_000)
        XCTAssertEqual(thinned.first, 0)
        XCTAssertEqual(thinned.last, 10_000)
        XCTAssertEqual(thinned, thinned.sorted())
        XCTAssertEqual(ImportPreview.thinned([1, 2, 3], limit: 1_000), [1, 2, 3])
    }
}
