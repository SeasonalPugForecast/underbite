import Foundation
import MapKit
import XCTest

@testable import UnderbiteKit

/// The access and stocking marks: the core's kinds and access in the app's marks, which layer
/// each goes to, the grouping at a zoom, which mark a tap meets, and the areas thinned for the
/// zoom they are drawn at.
final class AccessMarksTests: XCTestCase {
    /// Near The Battery.
    private let battery = (latitude: 40.7006, longitude: -74.0142)

    private func access(
        _ id: String, kind: String = "ramp", access: String = "open", lat: Double? = nil,
        lon: Double? = nil, shape: AccessShape = .point
    ) -> AccessPin {
        AccessPin(
            id: id, source: kind == "public_land" ? "padus" : "overpass",
            sourceName: kind == "public_land" ? "USGS PAD-US" : "OpenStreetMap", kind: kind,
            kindLabel: kind, title: id, access: access, accessLabel: access,
            lat: lat ?? battery.latitude, lon: lon ?? battery.longitude, shape: shape,
            accessibility: id)
    }

    private func stocked(_ id: String, lat: Double, lon: Double) -> StockingPin {
        StockingPin(
            id: id, waterbody: id, county: "Westchester", name: id, lat: lat, lon: lon,
            kind: "river", cardJson: nil, label: id)
    }

    /// A mark `points` screen points east of the Battery at `perPoint` map points a point.
    private func east(points: Double, perPoint: Double, id: String, kind: String = "ramp")
        -> AccessMark
    {
        let start = MKMapPoint(
            CLLocationCoordinate2D(latitude: battery.latitude, longitude: battery.longitude))
        let at = MKMapPoint(x: start.x + points * perPoint, y: start.y).coordinate
        return AccessMark(access(id, kind: kind, lat: at.latitude, lon: at.longitude))
    }

    func testTheCoresKindsAndAccessHaveTheirOwnMarks() {
        for kind in ["ramp", "pier", "fishing_spot", "parking", "public_land"] {
            XCTAssertNotEqual(AccessMarkKind(core: kind), .other, kind)
            XCTAssertEqual(AccessMarkKind(core: kind).rawValue, kind)
        }
        XCTAssertEqual(AccessMarkKind(core: "a_kind_from_later"), .other)
        // Stocking is the app's own kind, never an access place's.
        XCTAssertEqual(AccessMarkKind(core: "stocking"), .other)
        let symbols = AccessMarkKind.allCases.map(\.symbol)
        XCTAssertEqual(Set(symbols).count, symbols.count, "each kind has its own symbol")

        for level in ["open", "restricted", "closed", "unknown"] {
            XCTAssertEqual(AccessMarkRole(core: level).rawValue, level)
        }
        XCTAssertEqual(AccessMarkRole(core: "private_later"), .unknown)
        XCTAssertEqual(AccessMarkRole(core: "stocking"), .unknown)
    }

    func testAccessIsNeverColourAlone() {
        let roles = AccessMarkRole.allCases
        XCTAssertEqual(Set(roles.map(\.colour)).count, roles.count)
        XCTAssertEqual(AccessMarkRole.restricted.badge, .caution)
        XCTAssertEqual(AccessMarkRole.closed.badge, .closed)
        XCTAssertEqual(AccessMarkRole.open.badge, .none)
        XCTAssertFalse(AccessMarkRole.unknown.filled, "an unknown access is an outline")
        for role in roles where role.filled {
            XCTAssertGreaterThanOrEqual(
                role.colour.contrast(with: role.glyph), 2.5, "\(role) symbol stands out")
        }
    }

    func testPublicLandAccessPointsAndStockedWatersGoToTheirLayers() {
        let marks = AccessMarks.marks(
            access: [
                access("land", kind: "public_land"), access("ramp"), access("pier", kind: "pier"),
                access("ramp"),
            ],
            stocking: [stocked("croton", lat: 41.2, lon: -73.86)])
        XCTAssertEqual(marks.map(\.id), ["land", "ramp", "pier", "croton"], "each id once")
        XCTAssertEqual(
            marks.map(\.layer),
            [
                AccessLayers.publicLand, AccessLayers.accessPoints, AccessLayers.accessPoints,
                AccessLayers.stockingPoints,
            ])
        XCTAssertEqual(marks.map(\.source), ["padus", "overpass", "overpass", "ny-dec-stocking"])
        XCTAssertEqual(marks[3].kind, .stocking)
        XCTAssertEqual(marks[3].role, .stocking)

        let shown = AccessMarks.shown(
            marks, layers: [AccessLayers.publicLand, AccessLayers.accessPoints])
        XCTAssertEqual(shown.map(\.id), ["ramp", "pier"], "ramps lead; land is drawn as its area")
    }

    func testNearbyMarksGroupAndFarOnesStandAlone() {
        let perPoint = AccessMarks.clusterRadius
        let a = east(points: 0, perPoint: perPoint, id: "a")
        let near = east(points: 10, perPoint: perPoint, id: "near", kind: "pier")
        let far = east(points: 200, perPoint: perPoint, id: "far")
        let groups = AccessMarks.clusters(
            AccessMarks.shown([a, near, far], layers: [AccessLayers.accessPoints]),
            mapPointsPerPoint: perPoint)
        XCTAssertEqual(groups.count, 2)
        XCTAssertEqual(groups[0].marks.map(\.id), ["a", "near"])
        XCTAssertEqual(groups[1].single?.id, "far")
        // The same marks and zoom group the same way.
        XCTAssertEqual(
            AccessMarks.clusters([a, near, far], mapPointsPerPoint: perPoint).map(\.id),
            groups.map(\.id))
        // Zoomed in far enough, every mark stands alone.
        XCTAssertEqual(
            AccessMarks.clusters([a, near, far], mapPointsPerPoint: perPoint / 100).count, 3)
    }

    func testATapMeetsTheNearestMarkWithinReach() {
        let perPoint = 10.0
        let a = east(points: 0, perPoint: perPoint, id: "a")
        let b = east(points: 100, perPoint: perPoint, id: "b")
        let groups = AccessMarks.clusters([a, b], mapPointsPerPoint: perPoint)
        let hit = AccessMarks.hit(
            groups, x: a.x + 15 * perPoint, y: a.y, mapPointsPerPoint: perPoint)
        XCTAssertEqual(hit?.single?.id, "a")
        XCTAssertNil(
            AccessMarks.hit(groups, x: a.x + 50 * perPoint, y: a.y, mapPointsPerPoint: perPoint))
        XCTAssertEqual(
            AccessMarks.hit(groups, x: b.x, y: b.y + 5 * perPoint, mapPointsPerPoint: perPoint)?
                .single?.id, "b")
    }

    func testLayersAreDrawnFromTheirShallowestZoom() {
        XCTAssertTrue(AccessMarks.drawn(minZoom: 11, at: 11))
        XCTAssertTrue(AccessMarks.drawn(minZoom: 11, at: 10.9), "a hair short still shows")
        XCTAssertFalse(AccessMarks.drawn(minZoom: 11, at: 10))
        XCTAssertTrue(AccessMarks.drawn(minZoom: nil, at: 2))
    }

    // MARK: - Areas

    private func square(_ id: String, side: Double = 0.01) -> AccessPin {
        let (lat, lon) = battery
        let ring = [
            MapPoint(lat: lat, lon: lon), MapPoint(lat: lat, lon: lon + side),
            MapPoint(lat: lat + side, lon: lon + side), MapPoint(lat: lat + side, lon: lon),
        ]
        return access(id, kind: "public_land", shape: .polygon(rings: [ring]))
    }

    func testAreasAndLinesBecomeShapesInTheirLayer() throws {
        let land = try XCTUnwrap(AccessShape2D(square("land")))
        XCTAssertEqual(land.layer, AccessLayers.publicLand)
        XCTAssertEqual(land.form, .area)
        XCTAssertEqual(land.rings.first?.count, 4)
        let corner = MKMapPoint(
            CLLocationCoordinate2D(latitude: battery.latitude, longitude: battery.longitude))
        XCTAssertEqual(land.bounds.minX, corner.x, accuracy: 1e-6)
        XCTAssertEqual(land.bounds.maxY, corner.y, accuracy: 1e-6, "north is up: smaller y")
        XCTAssertGreaterThan(land.bounds.width, 0)

        XCTAssertNil(AccessShape2D(access("ramp")), "a point is a mark only")
        let pier = try XCTUnwrap(
            AccessShape2D(
                access(
                    "pier", kind: "pier",
                    shape: .line(points: [
                        MapPoint(lat: 40.70, lon: -74.02), MapPoint(lat: 40.701, lon: -74.021),
                    ]))))
        XCTAssertEqual(pier.form, .line)
        XCTAssertEqual(pier.layer, AccessLayers.accessPoints)
        XCTAssertNil(
            AccessShape2D(access("bad", kind: "public_land", shape: .polygon(rings: [[]]))),
            "an empty area draws nothing")
    }

    func testThinningKeepsTheEndsAndDropsWhatIsUnderAPoint() {
        let ring = (0...100).map { MapXY(x: Double($0), y: 0) }
        let thin = AccessThinning.thin(ring, tolerance: 10, closed: false)
        XCTAssertEqual(thin.first, ring.first)
        XCTAssertEqual(thin.last, ring.last)
        XCTAssertEqual(thin.count, 11)
        for (a, b) in zip(thin, thin.dropFirst().dropLast()) {
            XCTAssertGreaterThanOrEqual(b.x - a.x, 10)
        }
        XCTAssertEqual(AccessThinning.thin(ring, tolerance: 0, closed: false), ring)
        // An area smaller than the tolerance is gone; a line keeps its ends.
        let tiny = [MapXY(x: 0, y: 0), MapXY(x: 1, y: 0), MapXY(x: 1, y: 1), MapXY(x: 0, y: 1)]
        XCTAssertEqual(AccessThinning.thin(tiny, tolerance: 5, closed: true), [])
        XCTAssertEqual(AccessThinning.thin(tiny, tolerance: 5, closed: false).count, 2)
    }

    func testEachZoomBucketThinsUnderHalfAPoint() {
        for perPoint in [0.3, 1, 1.7, 16, 100, 4_096, 65_000] {
            let bucket = AccessThinning.bucket(mapPointsPerPoint: perPoint)
            let tolerance = AccessThinning.tolerance(bucket: bucket)
            XCTAssertLessThanOrEqual(tolerance / perPoint, 0.5 + 1e-9, "\(perPoint)")
            XCTAssertGreaterThanOrEqual(tolerance / perPoint, 0.25 - 1e-9, "\(perPoint)")
        }
        XCTAssertEqual(AccessThinning.bucket(mapPointsPerPoint: 0), 0)
        XCTAssertEqual(AccessThinning.bucket(mapPointsPerPoint: .infinity), 0)
    }

    func testATapInsideAnAreaMeetsTheSmallestAreaThatHoldsIt() throws {
        let big = try XCTUnwrap(AccessShape2D(square("big", side: 0.02)))
        let small = try XCTUnwrap(AccessShape2D(square("small", side: 0.005)))
        let inBoth = MKMapPoint(
            CLLocationCoordinate2D(
                latitude: battery.latitude + 0.002, longitude: battery.longitude + 0.002))
        XCTAssertEqual(AccessThinning.area(at: inBoth.x, inBoth.y, in: [big, small])?.id, "small")
        let inBig = MKMapPoint(
            CLLocationCoordinate2D(
                latitude: battery.latitude + 0.015, longitude: battery.longitude + 0.015))
        XCTAssertEqual(AccessThinning.area(at: inBig.x, inBig.y, in: [big, small])?.id, "big")
        let outside = MKMapPoint(
            CLLocationCoordinate2D(
                latitude: battery.latitude - 0.01, longitude: battery.longitude))
        XCTAssertNil(AccessThinning.area(at: outside.x, outside.y, in: [big, small]))

        // A hole is not the area.
        let (lat, lon) = battery
        func ring(_ s: Double, _ e: Double) -> [MapPoint] {
            [
                MapPoint(lat: lat + s, lon: lon + s), MapPoint(lat: lat + s, lon: lon + e),
                MapPoint(lat: lat + e, lon: lon + e), MapPoint(lat: lat + e, lon: lon + s),
            ]
        }
        let holed = try XCTUnwrap(
            AccessShape2D(
                access(
                    "holed", kind: "public_land",
                    shape: .polygon(rings: [ring(0, 0.02), ring(0.005, 0.015)]))))
        let middle = MKMapPoint(
            CLLocationCoordinate2D(latitude: lat + 0.01, longitude: lon + 0.01))
        XCTAssertNil(AccessThinning.area(at: middle.x, middle.y, in: [holed]))
        XCTAssertEqual(AccessThinning.area(at: inBoth.x, inBoth.y, in: [holed])?.id, "holed")
    }

    func testAreasUnderAPointAcrossAreNotDrawn() throws {
        let land = try XCTUnwrap(AccessShape2D(square("land", side: 0.001)))
        let across = max(land.bounds.width, land.bounds.height)
        XCTAssertTrue(AccessThinning.visible(land, mapPointsPerPoint: across / 10))
        XCTAssertFalse(AccessThinning.visible(land, mapPointsPerPoint: across * 2))
    }
}
