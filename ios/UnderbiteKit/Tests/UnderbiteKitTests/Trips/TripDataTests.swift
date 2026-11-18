import Foundation
import XCTest

@testable import UnderbiteKit

/// What a trip's page keeps: histories by place, station metadata by key, the places as the
/// core takes them, and the file they live in.
final class TripDataTests: XCTestCase {
    private let directory = FileManager.default.temporaryDirectory
        .appending(path: "underbite-trip-data-\(UUID().uuidString)", directoryHint: .isDirectory)

    override func tearDown() {
        try? FileManager.default.removeItem(at: directory)
    }

    private func results(
        histories: [(String, String)], cacheable: [(String, String, Int64)]
    ) -> TripResults {
        TripResults(
            histories: histories.map { TripPlaceHistory(key: $0.0, historyJson: $0.1) },
            cacheable: cacheable.map {
                CachedResponse(key: $0.0, body: Data($0.1.utf8), fetchedAt: $0.2)
            },
            errors: [])
    }

    func testANewHistoryReplacesTheOldAndMetadataIsKeptByKey() {
        var data = TripData()
        data.merge(results(histories: [("a", "{\"v\":1}")], cacheable: [("harcon", "x", 1)]))
        data.merge(
            results(
                histories: [("a", "{\"v\":2}"), ("b", "{}")],
                cacheable: [("harcon", "y", 2), ("datums", "z", 2)]))
        XCTAssertEqual(data.histories, ["a": "{\"v\":2}", "b": "{}"])
        XCTAssertEqual(data.metadata.map(\.key), ["harcon", "datums"])
        XCTAssertEqual(data.cachedMetadata.first?.body, Data("y".utf8))
        XCTAssertEqual(data.cachedMetadata.first?.fetchedAt, 2)
    }

    func testThePlacesCarryTheirSnapshotAndKeptHistory() {
        var data = TripData()
        data.histories = ["a": "{\"h\":1}", "b": "{\"h\":2}"]
        let places = data.places(["a", "b", "c"], snapshot: { $0 == "a" ? "{\"s\":1}" : nil })
        XCTAssertEqual(places.map(\.key), ["a", "b", "c"])
        XCTAssertEqual(places.map(\.snapshotJson), ["{\"s\":1}", nil, nil])
        XCTAssertEqual(places.map(\.historyJson), ["{\"h\":1}", "{\"h\":2}", nil])
        let again = data.places(["a", "b"], snapshot: { _ in nil }, fresh: ["b"])
        XCTAssertEqual(again.map(\.historyJson), ["{\"h\":1}", nil], "b is planned again whole")
    }

    func testOnlyHistoryRequestsAskForAPlaceAgain() {
        func request(_ place: String, _ source: String) -> TripRequest {
            TripRequest(
                id: place + source, place: place, source: source, metered: source != "coops",
                request: HostRequest(
                    url: "https://example.invalid/", headers: [], method: .get, body: nil,
                    fresh: false, noStore: false, maxAgeS: nil))
        }
        XCTAssertEqual(
            TripData.historyPlaces([
                request("a", "coops"), request("b", "usgs"), request("c", "open-meteo-marine"),
            ]),
            ["b", "c"])
        XCTAssertEqual(TripData.historyPlaces([]), [])
    }

    func testWhatIsKeptReadsBackAndAMissingOrBadFileIsEmpty() throws {
        let files = AppFiles(directory: directory)
        XCTAssertEqual(TripData.read("t/1", files: files), TripData())
        var data = TripData()
        data.merge(results(histories: [("a", "{}")], cacheable: [("harcon", "x", 1)]))
        data.write("t/1", files: files)
        XCTAssertEqual(TripData.fileName("t/1"), "trip-data/t%2F1.json", "one file name")
        XCTAssertEqual(TripData.read("t/1", files: files), data)
        files.write(Data("not json".utf8), TripData.fileName("t/1"))
        XCTAssertEqual(TripData.read("t/1", files: files), TripData())
    }
}
