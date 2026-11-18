import Foundation
import MapKit
import XCTest

@testable import UnderbiteKit

/// The depth band over the core's stops, and the contours placed on the map.
final class DepthBandTests: XCTestCase {
    private func settings(metres: Bool = false) throws -> Settings {
        var settings = try Settings.defaults()
        if metres {
            var object = try XCTUnwrap(
                try JSONSerialization.jsonObject(with: Data(try settings.json().utf8))
                    as? [String: Any])
            var units = try XCTUnwrap(object["units"] as? [String: Any])
            units["height"] = "m"
            object["units"] = units
            let data = try JSONSerialization.data(withJSONObject: object)
            settings = try Settings(json: String(decoding: data, as: UTF8.self))
        }
        return settings
    }

    private func stops(_ settings: Settings) throws -> [DisplayStop] {
        DepthBand.stops(settingsJSON: try settings.json())
    }

    // MARK: - The band

    func testTheStopsAreTheCoresInTheUsersDepthUnitFromTheSurface() throws {
        let feet = try stops(try settings())
        XCTAssertGreaterThanOrEqual(feet.count, 3)
        XCTAssertEqual(feet.first?.value, 0)
        XCTAssertTrue(feet.allSatisfy { $0.label.hasSuffix(" ft") }, "\(feet.map(\.label))")
        XCTAssertEqual(feet.map(\.value), feet.map(\.value).sorted())
        let metres = try stops(try settings(metres: true))
        XCTAssertTrue(metres.allSatisfy { $0.label.hasSuffix(" m") }, "\(metres.map(\.label))")
    }

    func testABandIsStoredAsTheStopsValuesAndReadBackAtTheSameStops() throws {
        var settings = try settings()
        let stops = try stops(settings)
        XCTAssertNil(DepthBand.positions(of: settings.map.depth.bandM, in: stops))
        DepthBand.set(1...2, stops: stops, in: &settings.map)
        XCTAssertEqual(settings.map.depth.bandM, [stops[1].value, stops[2].value])
        XCTAssertEqual(DepthBand.positions(of: settings.map.depth.bandM, in: stops), 1...2)
        // The core accepts what is stored.
        let json = try settings.json()
        XCTAssertEqual(try Settings(json: json).map.depth.bandM, settings.map.depth.bandM)
        DepthBand.set(nil, stops: stops, in: &settings.map)
        XCTAssertNil(settings.map.depth.bandM)
        DepthBand.set(2...2, stops: stops, in: &settings.map)
        XCTAssertNil(settings.map.depth.bandM, "ends on one stop are no band")
    }

    func testABandBetweenStopsIsReadAtTheNearestAndKeptAStopApart() {
        let stops = [0.0, 10, 20, 30].map { DisplayStop(value: $0, label: "\(Int($0))") }
        XCTAssertEqual(DepthBand.positions(of: [11, 29], in: stops), 1...3)
        XCTAssertEqual(DepthBand.positions(of: [29, 11], in: stops), 1...3, "either order")
        XCTAssertEqual(DepthBand.positions(of: [12, 13], in: stops), 1...2)
        XCTAssertEqual(DepthBand.positions(of: [40, 50], in: stops), 2...3)
        XCTAssertNil(DepthBand.positions(of: [1], in: stops))
        XCTAssertNil(DepthBand.positions(of: [1, 2], in: Array(stops.prefix(1))))
    }

    func testDraggingAnEndSnapsToAStopAndNeverPassesTheOther() {
        XCTAssertEqual(DepthBand.position(at: 0, count: 4), 0)
        XCTAssertEqual(DepthBand.position(at: 0.4, count: 4), 1)
        XCTAssertEqual(DepthBand.position(at: 0.6, count: 4), 2)
        XCTAssertEqual(DepthBand.position(at: 1.7, count: 4), 3)
        XCTAssertEqual(DepthBand.position(at: -1, count: 4), 0)
        XCTAssertEqual(DepthBand.moved(1...2, shallow: true, to: 0, count: 4), 0...2)
        XCTAssertEqual(DepthBand.moved(1...2, shallow: true, to: 3, count: 4), 1...2)
        XCTAssertEqual(DepthBand.moved(1...2, shallow: false, to: 3, count: 4), 1...3)
        XCTAssertEqual(DepthBand.moved(1...2, shallow: false, to: 0, count: 4), 1...2)
        XCTAssertEqual(DepthBand.moved(0...1, shallow: false, to: 9, count: 4), 0...3)
    }

    // MARK: - Contours on the map

    func testALabelTurnsWithItsLineAndReadsWestToEast() {
        XCTAssertEqual(ContourSet.rotation(angleDeg: 90), 0, accuracy: 1e-12)
        XCTAssertEqual(ContourSet.rotation(angleDeg: 0), -.pi / 2, accuracy: 1e-12)
        XCTAssertEqual(ContourSet.rotation(angleDeg: 180), .pi / 2, accuracy: 1e-12)
        XCTAssertEqual(ContourSet.rotation(angleDeg: 45), -.pi / 4, accuracy: 1e-12)
    }

    func testLinesArePlacedInMapPointsAndPickedOutPerTile() {
        let level = ContourLine(
            points: [MapPoint(lat: 40.5, lon: -74.1), MapPoint(lat: 40.5, lon: -74.0)],
            label: "30 ft", labelAt: LabelAt(lat: 40.5, lon: -74.05, angleDeg: 90), major: true)
        let far = ContourLine(
            points: [MapPoint(lat: 10, lon: 10), MapPoint(lat: 10.1, lon: 10.1)],
            label: "60 ft", labelAt: nil, major: false)
        let single = ContourLine(
            points: [MapPoint(lat: 40.5, lon: -74.1)], label: "90 ft", labelAt: nil, major: false)
        let set = ContourSet([level, far, single])
        XCTAssertEqual(set.lines.count, 2, "a single point is no line")
        XCTAssertEqual(set.vertexCount, 4)
        let first = set.lines[0]
        XCTAssertTrue(first.major)
        XCTAssertEqual(first.label?.text, "30 ft")
        XCTAssertEqual(first.label?.rotation ?? 1, 0, accuracy: 1e-12)
        let west = MKMapPoint(CLLocationCoordinate2D(latitude: 40.5, longitude: -74.1))
        XCTAssertEqual(first.points[0].x, west.x, accuracy: 1e-6)
        XCTAssertGreaterThan(first.bounds.height, 0, "a level line still has a box")
        let tile = MKMapRect(
            origin: MKMapPoint(CLLocationCoordinate2D(latitude: 40.6, longitude: -74.2)),
            size: MKMapSize(width: 200_000, height: 200_000))
        XCTAssertEqual(set.lines(in: tile, margin: 0).count, 1)
        XCTAssertEqual(set.labels(near: tile, reach: 0).map(\.text), ["30 ft"])
        let beside = MKMapRect(
            x: tile.maxX + 50, y: tile.minY, width: 1_000, height: 1_000)
        XCTAssertTrue(set.lines(in: beside, margin: 0).isEmpty)
        XCTAssertTrue(set.labels(near: beside, reach: 10).isEmpty)
        XCTAssertEqual(set.labels(near: beside, reach: 1_000_000).count, 1)
    }
}
