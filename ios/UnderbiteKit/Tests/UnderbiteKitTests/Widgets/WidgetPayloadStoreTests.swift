import Foundation
import XCTest

@testable import UnderbiteKit

/// The payload store over a folder of its own: payloads kept and read back, files per spot and
/// for the next trip, and nothing but payloads let in.
final class WidgetPayloadStoreTests: XCTestCase {
    private var folder: URL!
    private var store: WidgetPayloadStore!

    override func setUpWithError() throws {
        folder = FileManager.default.temporaryDirectory
            .appending(path: "underbite-widget-store-\(UUID().uuidString)", directoryHint: .isDirectory)
        store = WidgetPayloadStore(container: SharedContainer(url: folder, backing: .temporary))
    }

    override func tearDownWithError() throws {
        try? FileManager.default.removeItem(at: folder)
    }

    func testASpotsPayloadRoundTrips() throws {
        let json = try Fixtures.text("sample-widget-battery")
        XCTAssertNil(store.spot(id: "battery"))
        let saved = try store.saveSpot(json)
        XCTAssertEqual(saved, try ViewModel.decodeWidget(json))
        XCTAssertEqual(store.spot(id: "battery"), saved)
        // Kept as the core wrote it.
        try Fixtures.assertSameJSON(
            String(contentsOf: store.spotURL(id: "battery"), encoding: .utf8), json)
        // Another store over the same folder (the widget's) reads it too.
        let widget = WidgetPayloadStore(container: store.container)
        XCTAssertEqual(widget.spot(id: "battery"), saved)
        XCTAssertNil(widget.spot(id: "croton"))
    }

    func testEachSpotHasItsOwnFileAndANewPayloadReplacesTheOld() throws {
        try store.saveSpot(Fixtures.text("sample-widget-battery"))
        try store.saveSpot(Fixtures.text("sample-widget-croton"))
        XCTAssertEqual(store.spotIDs(), ["battery", "croton"])
        XCTAssertEqual(store.spot(id: "croton")?.places.first?.id, "croton")

        // A later payload for the Croton replaces the earlier.
        var later = try XCTUnwrap(Fixtures.object(Fixtures.text("sample-widget-croton")) as? [String: Any])
        later["generated_at"] = 1_794_142_800
        try store.saveSpot(Fixtures.string(later))
        XCTAssertEqual(store.spot(id: "croton")?.generatedAt, 1_794_142_800)
        XCTAssertEqual(store.spotIDs(), ["battery", "croton"])

        try store.removeSpot(id: "battery")
        try store.removeSpot(id: "battery")
        XCTAssertEqual(store.spotIDs(), ["croton"])
        XCTAssertNil(store.spot(id: "battery"))
    }

    func testOnlyTheSavedSpotsPayloadsAreKept() throws {
        try store.saveSpot(Fixtures.text("sample-widget-battery"))
        try store.saveSpot(Fixtures.text("sample-widget-croton"))
        try store.keepSpots(["croton", "elsewhere"])
        XCTAssertEqual(store.spotIDs(), ["croton"])
        try store.keepSpots([String]())
        XCTAssertEqual(store.spotIDs(), [])
    }

    func testAnIdOfAnyShapeNamesOneFileInTheFolder() throws {
        var odd = try XCTUnwrap(Fixtures.object(Fixtures.text("sample-widget-croton")) as? [String: Any])
        let id = "../Croton River/é.1"
        odd["id"] = id
        try store.saveSpot(Fixtures.string(odd))
        XCTAssertEqual(store.spot(id: id)?.id, id)
        XCTAssertEqual(store.spotIDs(), [id])
        let file = store.spotURL(id: id)
        XCTAssertEqual(file.deletingLastPathComponent().lastPathComponent, "spots")
        XCTAssertFalse(file.lastPathComponent.contains("/"))
    }

    func testTheNextTripRoundTripsAndIsForgotten() throws {
        XCTAssertNil(store.nextTrip())
        let json = try Fixtures.text("sample-widget-trip")
        let saved = try store.saveNextTrip(json)
        XCTAssertEqual(saved.id, "harbor")
        XCTAssertEqual(store.nextTrip(), saved)
        // The trip is not a spot.
        XCTAssertEqual(store.spotIDs(), [])
        try store.removeNextTrip()
        try store.removeNextTrip()
        XCTAssertNil(store.nextTrip())
    }

    func testOnlyPayloadsGetIn() throws {
        // A snapshot is not a payload, and carries the spot's coordinates.
        XCTAssertThrowsError(try store.saveSpot(SampleSnapshot.json)) { error in
            guard case WidgetStoreError.notAPayload = error else {
                return XCTFail("\(error)")
            }
        }
        XCTAssertThrowsError(try store.saveSpot("{")) { error in
            guard case WidgetStoreError.notAPayload = error else {
                return XCTFail("\(error)")
            }
        }
        // A trip where a spot belongs, and the other way round.
        XCTAssertThrowsError(try store.saveSpot(Fixtures.text("sample-widget-trip"))) { error in
            XCTAssertEqual(error as? WidgetStoreError, .wrongSubject(expected: "spot", found: "trip"))
        }
        XCTAssertThrowsError(try store.saveNextTrip(Fixtures.text("sample-widget-battery"))) { error in
            XCTAssertEqual(error as? WidgetStoreError, .wrongSubject(expected: "trip", found: "spot"))
        }
        // A payload that would carry where a place is.
        var placed = try XCTUnwrap(Fixtures.object(Fixtures.text("sample-widget-battery")) as? [String: Any])
        var places = try XCTUnwrap(placed["places"] as? [[String: Any]])
        places[0]["latitude"] = 40.7033
        placed["places"] = places
        XCTAssertThrowsError(try store.saveSpot(Fixtures.string(placed))) { error in
            XCTAssertEqual(error as? WidgetStoreError, .carriesCoordinates(key: "latitude"))
        }
        var nameless = try XCTUnwrap(Fixtures.object(Fixtures.text("sample-widget-battery")) as? [String: Any])
        nameless["id"] = ""
        XCTAssertThrowsError(try store.saveSpot(Fixtures.string(nameless))) { error in
            XCTAssertEqual(error as? WidgetStoreError, .noID)
        }
        // Nothing was written.
        XCTAssertEqual(store.spotIDs(), [])
        XCTAssertNil(store.nextTrip())
    }

    func testAFileThatDoesNotReadIsNoPayload() throws {
        try store.saveSpot(Fixtures.text("sample-widget-battery"))
        try Data("{".utf8).write(to: store.spotURL(id: "battery"))
        XCTAssertNil(store.spot(id: "battery"))
    }

    func testTheSpotListRoundTrips() throws {
        XCTAssertEqual(store.spots(), WidgetSpots())
        let spots = WidgetSpots(
            spots: [WidgetSpot(id: "battery", name: "The Battery"), WidgetSpot(id: "croton", name: "Croton River")],
            appSpotID: "croton")
        try store.saveSpots(spots)
        XCTAssertEqual(WidgetPayloadStore(container: store.container).spots(), spots)
        // Names and ids only.
        let text = try String(contentsOf: folder.appending(path: "spot-list.json"), encoding: .utf8)
        XCTAssertNil(WidgetPayloadStore.coordinateKey(in: try Fixtures.object(text)))
    }

    /// Without the App Group the store keeps its payloads in the temporary directory, and says
    /// the widgets do not see them; it works the same.
    func testWithoutTheAppGroupTheStoreFallsBackAndStillWorks() throws {
        let temporary = folder.appending(path: "tmp", directoryHint: .isDirectory)
        let container = SharedContainer.resolve(
            groupContainer: nil, temporaryDirectory: temporary, containerExists: { _ in true })
        XCTAssertEqual(container.backing, .temporary)
        XCTAssertFalse(container.isShared)
        XCTAssertTrue(container.url.path(percentEncoded: false).hasPrefix(temporary.path(percentEncoded: false)))
        let fallback = WidgetPayloadStore(container: container)
        let saved = try fallback.saveSpot(Fixtures.text("sample-widget-battery"))
        XCTAssertEqual(fallback.spot(id: "battery"), saved)
        try fallback.saveNextTrip(Fixtures.text("sample-widget-trip"))
        XCTAssertEqual(fallback.nextTrip()?.id, "harbor")
    }
}
