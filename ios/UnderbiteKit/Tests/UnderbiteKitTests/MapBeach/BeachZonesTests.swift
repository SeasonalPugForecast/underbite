import MapKit
import XCTest

@testable import UnderbiteKit

/// The beach forecast zones as the map keeps them: the colour roles of the core's risk names,
/// the shapes of the zones the core found, and which zone a tap falls in. No network.
final class BeachZonesTests: XCTestCase {
    private func card(role: String) -> BeachCardInfo {
        BeachCardInfo(
            title: "Zone", role: role, risk: "Moderate", rows: [], period: nil, issued: "",
            age: nil, safety: "", accessibility: "")
    }

    /// A square of `side` degrees with its south-west corner at (`lat`, `lon`).
    private func square(_ lat: Double, _ lon: Double, _ side: Double) -> [MapPoint] {
        [
            MapPoint(lat: lat, lon: lon), MapPoint(lat: lat + side, lon: lon),
            MapPoint(lat: lat + side, lon: lon + side), MapPoint(lat: lat, lon: lon + side),
        ]
    }

    private func pin(
        _ id: String, role: String = "moderate", polygons: [[[MapPoint]]]
    ) -> BeachZonePin {
        BeachZonePin(
            id: id, name: id, role: role, lat: 40.58, lon: -73.82,
            polygons: polygons.map { BeachPolygon(rings: $0) }, card: card(role: role))
    }

    private func xy(_ lat: Double, _ lon: Double) -> (x: Double, y: Double) {
        let at = MapXY(latitude: lat, longitude: lon)
        return (at.x, at.y)
    }

    func testTheCoresNamesAreTheRoles() {
        XCTAssertEqual(BeachRole(core: "low"), .low)
        XCTAssertEqual(BeachRole(core: "moderate"), .moderate)
        XCTAssertEqual(BeachRole(core: "high"), .high)
        XCTAssertEqual(BeachRole(core: "none"), .none)
        // A name this build does not know is a zone with nothing to show, never a category.
        XCTAssertEqual(BeachRole(core: "extreme"), .none)
        XCTAssertEqual(BeachRole(core: ""), .none)
    }

    func testOnlyAZoneWithNoForecastIsLeftUnfilledAndNoColourIsGreen() {
        XCTAssertEqual(BeachRole.allCases.filter { !$0.filled }, [.none])
        XCTAssertEqual(BeachRole.none.fillAlpha, 0)
        for role in BeachRole.allCases {
            let r = (role.rgb >> 16) & 0xFF, g = (role.rgb >> 8) & 0xFF, b = role.rgb & 0xFF
            XCTAssertFalse(g > r && g > b, "\(role) is greenish")
        }
        // The scale rises: the fill grows with the category.
        XCTAssertLessThan(BeachRole.low.fillAlpha, BeachRole.moderate.fillAlpha)
        XCTAssertLessThan(BeachRole.moderate.fillAlpha, BeachRole.high.fillAlpha)
        XCTAssertEqual(Set(BeachRole.allCases.map(\.rgb)).count, 4)
    }

    func testAZoneBecomesAShapeWithItsRoleAndBounds() throws {
        let shape = try XCTUnwrap(
            BeachShape(pin("okx:nyz178", role: "high", polygons: [[square(40.5, -73.9, 0.1)]])))
        XCTAssertEqual(shape.id, "okx:nyz178")
        XCTAssertEqual(shape.role, .high)
        XCTAssertEqual(shape.polygons.count, 1)
        XCTAssertFalse(shape.bounds.isNull)
        let corner = MKMapPoint(CLLocationCoordinate2D(latitude: 40.5, longitude: -73.9))
        XCTAssertEqual(shape.bounds.minX, corner.x, accuracy: 1)
    }

    func testAZoneWithNoUsableOutlineIsLeftOut() {
        let two = [MapPoint(lat: 40.5, lon: -73.9), MapPoint(lat: 40.6, lon: -73.9)]
        XCTAssertNil(BeachShape(pin("a", polygons: [])))
        XCTAssertNil(BeachShape(pin("b", polygons: [[two]])))
        let shapes = BeachZones.shapes([
            pin("a", polygons: []), pin("b", polygons: [[square(40.5, -73.9, 0.1)]]),
        ])
        XCTAssertEqual(shapes.map(\.id), ["b"])
    }

    func testATapInsideAZoneFindsItAndOneOutsideOrInAHoleDoesNot() throws {
        let outer = square(40.5, -73.9, 0.2)
        let hole = square(40.55, -73.85, 0.05)
        let zone = try XCTUnwrap(BeachShape(pin("ring", polygons: [[outer, hole]])))
        let inside = xy(40.52, -73.88)
        XCTAssertTrue(zone.contains(x: inside.x, y: inside.y))
        let inHole = xy(40.575, -73.825)
        XCTAssertFalse(zone.contains(x: inHole.x, y: inHole.y))
        let outside = xy(40.8, -73.5)
        XCTAssertFalse(zone.contains(x: outside.x, y: outside.y))
        XCTAssertEqual(BeachZones.hit([zone], x: inside.x, y: inside.y)?.id, "ring")
        XCTAssertNil(BeachZones.hit([zone], x: inHole.x, y: inHole.y))
        XCTAssertNil(BeachZones.hit([zone], x: outside.x, y: outside.y))
    }

    func testAZoneWithTwoOutlinesIsHitInEither() throws {
        let zone = try XCTUnwrap(
            BeachShape(
                pin(
                    "islands",
                    polygons: [[square(40.5, -73.9, 0.05)], [square(40.7, -73.6, 0.05)]])))
        for (lat, lon) in [(40.52, -73.88), (40.72, -73.58)] {
            let at = xy(lat, lon)
            XCTAssertNotNil(BeachZones.hit([zone], x: at.x, y: at.y), "\(lat),\(lon)")
        }
        let between = xy(40.6, -73.75)
        XCTAssertNil(BeachZones.hit([zone], x: between.x, y: between.y))
    }

    func testTheSmallestZoneUnderATapWins() throws {
        let big = try XCTUnwrap(BeachShape(pin("big", polygons: [[square(40.4, -74.0, 0.5)]])))
        let small = try XCTUnwrap(
            BeachShape(pin("small", polygons: [[square(40.5, -73.9, 0.05)]])))
        let at = xy(40.52, -73.88)
        XCTAssertEqual(BeachZones.hit([big, small], x: at.x, y: at.y)?.id, "small")
        XCTAssertEqual(BeachZones.hit([small, big], x: at.x, y: at.y)?.id, "small")
        let elsewhere = xy(40.8, -73.7)
        XCTAssertEqual(BeachZones.hit([big, small], x: elsewhere.x, y: elsewhere.y)?.id, "big")
    }

    func testTheLayerIsTheCatalogues() {
        XCTAssertEqual(BeachZones.layer, "beach-forecast")
    }
}
