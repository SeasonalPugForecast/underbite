import Foundation
import MapKit
import XCTest

@testable import UnderbiteKit

/// The "Mine" layers' data with the real core: the catches at their exact positions and how
/// their marks group and meet a tap, the tracks as simplified lines, the heat map's squares and
/// colours, and the seeded track of the screenshots.
@MainActor
final class MineMarksTests: XCTestCase {
    /// A directory of its own for each test (XCTest makes an instance per test).
    private let directory = FileManager.default.temporaryDirectory
        .appending(path: "underbite-mine-\(UUID().uuidString)", directoryHint: .isDirectory)

    override func tearDown() {
        try? FileManager.default.removeItem(at: directory)
    }

    private var files: AppFiles { AppFiles(directory: directory) }

    /// A log holding the five sample catches (three at the Battery, two on the Croton).
    private func seeded() -> CatchStore {
        let store = CatchStore(files: files, clock: { 1_800_000_000 })
        CatchSeeds.seed(catches: store, settingsJSON: "{}")
        return store
    }

    private func point(_ id: String, _ latitude: Double, _ longitude: Double) -> CatchPoint {
        CatchPoint(id: id, latitude: latitude, longitude: longitude)
    }

    /// The point `points` screen points east of `from` at `perPoint` map points a point.
    private func east(of from: CatchPoint, points: Double, perPoint: Double, id: String)
        -> CatchPoint
    {
        let at = MKMapPoint(x: from.at.x + points * perPoint, y: from.at.y).coordinate
        return point(id, at.latitude, at.longitude)
    }

    // MARK: - Catches

    func testCatchesAreReadNewestFirstAtTheirExactPositions() {
        let store = seeded()
        let points = CatchMarks.load(book: store.book)
        XCTAssertEqual(points.map(\.id), CatchSeeds.seeds.map(\.id))
        for (point, seed) in zip(points, CatchSeeds.seeds) {
            XCTAssertEqual(point.latitude, seed.spot.latitude, seed.id)
            XCTAssertEqual(point.longitude, seed.spot.longitude, seed.id)
            let at = MKMapPoint(
                CLLocationCoordinate2D(latitude: seed.spot.latitude, longitude: seed.spot.longitude))
            XCTAssertEqual(point.at, MapXY(x: at.x, y: at.y), seed.id)
        }
    }

    func testDeletedCatchesAndUnreadableLogsDrawNothing() {
        let store = seeded()
        store.delete(CatchSeeds.seeds[0].id)
        let points = CatchMarks.load(book: store.book)
        XCTAssertEqual(points.map(\.id), CatchSeeds.seeds.dropFirst().map(\.id))
        XCTAssertEqual(CatchMarks.load(book: logNew()), [])
        XCTAssertEqual(CatchMarks.load(book: Data("not a log".utf8)), [])
    }

    func testCatchesAtOnePlaceShareAMarkAndFarOnesDoNot() {
        let points = CatchMarks.load(book: seeded().book)
        // The harbour and the Croton at a view of both: one mark each, the newest catch leading.
        let wide = CatchMarks.mapPointsPerPoint(zoom: 10)
        let marks = CatchMarks.clusters(points, mapPointsPerPoint: wide)
        XCTAssertEqual(marks.count, 2)
        XCTAssertEqual(marks.map { $0.catches.count }, [2, 3])
        XCTAssertEqual(marks.map { $0.catches[0].id }, [CatchSeeds.seeds[0].id, CatchSeeds.seeds[1].id])
        XCTAssertEqual(marks, CatchMarks.clusters(points, mapPointsPerPoint: wide), "the same again")
        // Zoomed far out, the whole log is one mark.
        let world = CatchMarks.clusters(
            points, mapPointsPerPoint: CatchMarks.mapPointsPerPoint(zoom: 3))
        XCTAssertEqual(world.map { $0.catches.count }, [5])
    }

    func testMarksGroupWithinTheRadiusOnScreen() {
        let perPoint = CatchMarks.mapPointsPerPoint(zoom: 14)
        let a = point("a", 41.2108, -73.8625)
        let near = east(of: a, points: CatchMarks.clusterRadius - 1, perPoint: perPoint, id: "near")
        let far = east(of: a, points: CatchMarks.clusterRadius + 5, perPoint: perPoint, id: "far")
        let marks = CatchMarks.clusters([a, near, far], mapPointsPerPoint: perPoint)
        XCTAssertEqual(marks.map { $0.catches.map(\.id) }, [["a", "near"], ["far"]])
        // Three zoom levels deeper (six points apart become 48) each stands apart.
        let deeper = CatchMarks.clusters([a, near, far], mapPointsPerPoint: perPoint / 8)
        XCTAssertEqual(deeper.count, 3)
    }

    func testATapMeetsTheNearestMarkWithinReach() {
        let perPoint = CatchMarks.mapPointsPerPoint(zoom: 14)
        let a = point("a", 41.2108, -73.8625)
        let b = east(of: a, points: 60, perPoint: perPoint, id: "b")
        let marks = CatchMarks.clusters([a, b], mapPointsPerPoint: perPoint)
        func tap(_ points: Double) -> String? {
            CatchMarks.hit(
                marks, at: MapXY(x: a.at.x + points * perPoint, y: a.at.y),
                mapPointsPerPoint: perPoint)?.catches.first?.id
        }
        XCTAssertEqual(tap(0), "a")
        XCTAssertEqual(tap(CatchMarks.reach - 1), "a")
        XCTAssertNil(tap(CatchMarks.reach + 2), "between the two, out of reach of both")
        XCTAssertEqual(tap(50), "b")
        XCTAssertNil(CatchMarks.hit([], at: a.at, mapPointsPerPoint: perPoint))
    }

    // MARK: - Tracks

    func testTracksBecomeOneLinePerSegmentLeavingOutSinglePoints() {
        let track = SavedTrack(
            id: "t", name: "Walk", file: "walk.gpx", importedAt: 0,
            segments: [
                [
                    .init(latitude: 41.2108, longitude: -73.8625, time: nil),
                    .init(latitude: 41.2112, longitude: -73.8631, time: nil),
                ],
                [.init(latitude: 41.2131, longitude: -73.8652, time: nil)],
                [],
            ])
        let lines = TrackLines.lines([track])
        XCTAssertEqual(lines.count, 1)
        XCTAssertEqual(lines[0].points.count, 2)
        let a = MapXY(latitude: 41.2108, longitude: -73.8625)
        let b = MapXY(latitude: 41.2112, longitude: -73.8631)
        XCTAssertEqual(lines[0].minX, min(a.x, b.x))
        XCTAssertEqual(lines[0].maxY, max(a.y, b.y))
        XCTAssertTrue(lines[0].meets(minX: a.x, minY: a.y, maxX: a.x, maxY: a.y))
        XCTAssertFalse(lines[0].meets(minX: 0, minY: 0, maxX: 1, maxY: 1))
    }

    func testLongStraightTracksSimplifyToTheirEnds() {
        // Ten thousand points along a straight line, with jitter well under the tolerance.
        let points = (0..<10_000).map { i in
            MapXY(x: Double(i) * 10, y: 5_000 + (i.isMultiple(of: 2) ? 0.1 : -0.1))
        }
        let line = TrackLines.simplify(TrackLine(points), tolerance: 1)
        XCTAssertEqual(line.points, [points[0], points[9_999]])
    }

    func testSimplifyingKeepsTheCornersAndEveryPointWithinTolerance() {
        // A zigzag whose corners stand 50 map points off the line between its ends.
        var points: [MapXY] = []
        for i in 0...100 {
            let corner = i.isMultiple(of: 10)
            let y = corner ? (i.isMultiple(of: 20) ? 0.0 : 50.0) : Double.nan
            points.append(MapXY(x: Double(i), y: y))
        }
        // Fill the legs between corners with points on them.
        for i in points.indices where points[i].y.isNaN {
            let a = points[i - i % 10], b = points[i - i % 10 + 10]
            let t = Double(i % 10) / 10
            points[i] = MapXY(x: a.x + (b.x - a.x) * t, y: a.y + (b.y - a.y) * t)
        }
        let tolerance = 0.5
        let simple = TrackLines.simplify(TrackLine(points), tolerance: tolerance)
        XCTAssertEqual(simple.points, stride(from: 0, through: 100, by: 10).map { points[$0] })
        for p in points {
            let nearest = zip(simple.points, simple.points.dropFirst())
                .map { TrackLines.distance(p, from: $0, to: $1) }
                .min() ?? .infinity
            XCTAssertLessThanOrEqual(nearest, tolerance)
        }
        // Short lines are left as they are.
        let two = TrackLine([MapXY(x: 0, y: 0), MapXY(x: 1, y: 1)])
        XCTAssertEqual(TrackLines.simplify(two, tolerance: 10), two)
    }

    func testEachZoomLevelHalvesTheTolerance() {
        XCTAssertEqual(TrackLines.level(zoomScale: 1), 0)
        XCTAssertEqual(TrackLines.level(zoomScale: 1.9), 0)
        XCTAssertEqual(TrackLines.level(zoomScale: 0.25), -2)
        XCTAssertEqual(TrackLines.level(zoomScale: 0.3), -2)
        XCTAssertEqual(TrackLines.level(zoomScale: 0), 0)
        XCTAssertEqual(TrackLines.tolerance(level: 0), TrackLines.tolerancePoints)
        XCTAssertEqual(TrackLines.tolerance(level: -2), TrackLines.tolerancePoints * 4)
        XCTAssertEqual(TrackLines.tolerance(level: 1), TrackLines.tolerancePoints / 2)
        // At any scale of a level, the tolerance stays within one point on screen.
        for scale in [0.25, 0.3, 0.49] {
            let level = TrackLines.level(zoomScale: scale)
            XCTAssertLessThanOrEqual(TrackLines.tolerance(level: level) * scale, 1)
        }
    }

    // MARK: - The heat map

    func testHeatSquaresAreAPowerOfTwoThatReadsAtTheZoom() {
        var last = Double.infinity
        for zoom in stride(from: 2.0, through: 20, by: 0.5) {
            let side = CatchHeat.cellMetres(zoom: zoom, latitude: 41)
            XCTAssertEqual(log2(side), log2(side).rounded(), "\(side) at \(zoom)")
            XCTAssertGreaterThanOrEqual(side, CatchHeat.smallestMetres)
            XCTAssertLessThanOrEqual(side, CatchHeat.largestMetres)
            XCTAssertLessThanOrEqual(side, last, "deeper zooms never have larger squares")
            last = side
            let onScreen =
                side * MKMapPointsPerMeterAtLatitude(41) / CatchMarks.mapPointsPerPoint(zoom: zoom)
            if side > CatchHeat.smallestMetres, side < CatchHeat.largestMetres {
                XCTAssertGreaterThanOrEqual(onScreen, CatchHeat.targetPoints - 1e-6, "\(zoom)")
                XCTAssertLessThanOrEqual(onScreen, 2 * CatchHeat.targetPoints + 1e-6, "\(zoom)")
            }
        }
    }

    func testTheHeatMapIsTheCoresBinsOfTheLog() throws {
        let book = seeded().book
        let side = 256.0
        let cells = CatchHeat.cells(book: book, cellMetres: side)
        XCTAssertEqual(cells.map(\.count).sorted(), [2, 3])
        XCTAssertTrue(cells.allSatisfy { $0.total == 5 })
        let squares = CatchHeat.squares(cells, cellMetres: side)
        XCTAssertEqual(squares.map(\.fraction).sorted(), [2.0 / 3.0, 1])
        let battery = try XCTUnwrap(squares.first { $0.fraction == 1 })
        let spot = MapXY(latitude: SeedSpots.battery.latitude, longitude: SeedSpots.battery.longitude)
        // The square holds the catches' place (the core's grid is on a sphere a little smaller
        // than MapKit's ellipsoid: a thousandth or so apart).
        XCTAssertLessThanOrEqual(abs(battery.at.x - spot.x), battery.side / 2 * 1.01)
        XCTAssertLessThanOrEqual(abs(battery.at.y - spot.y), battery.side / 2 * 1.01)
        XCTAssertEqual(
            battery.side, side * MKMapPointsPerMeterAtLatitude(battery.at.coordinate.latitude),
            accuracy: battery.side * 1e-9)
        XCTAssertEqual(CatchHeat.squares([], cellMetres: side), [])
        XCTAssertEqual(CatchHeat.cells(book: logNew(), cellMetres: side), [])
    }

    func testTheHeatRampDarkensWithMoreCatches() {
        func luminance(_ c: (red: Double, green: Double, blue: Double)) -> Double {
            0.2126 * c.red + 0.7152 * c.green + 0.0722 * c.blue
        }
        var last = Double.infinity
        for step in 0...20 {
            let f = Double(step) / 20
            let l = luminance(CatchHeat.colour(f))
            XCTAssertLessThan(l, last, "\(f)")
            last = l
        }
        let low = CatchHeat.colour(0), high = CatchHeat.colour(1)
        XCTAssertEqual(low.red, Double(0xFC) / 255, accuracy: 1e-9)
        XCTAssertEqual(high.blue, Double(0x81) / 255, accuracy: 1e-9)
        // Out of range is held at the ends.
        XCTAssertEqual(CatchHeat.colour(-1).green, low.green)
        XCTAssertEqual(CatchHeat.colour(.nan).green, low.green)
        XCTAssertEqual(CatchHeat.colour(3).green, high.green)
        XCTAssertLessThan(CatchHeat.alpha(0.1), CatchHeat.alpha(1))
        XCTAssertEqual(CatchHeat.alpha(1), 1, accuracy: 1e-12)
    }

    // MARK: - The screenshots' track

    func testTheSeededTrackIsTheCrotonWalkSavedOnce() throws {
        let tracks = MineSeeds.tracks(settingsJSON: "{}")
        XCTAssertEqual(tracks.map(\.id), [MineSeeds.trackID])
        let walk = try XCTUnwrap(tracks.first)
        XCTAssertEqual(walk.name, "Croton River walk")
        XCTAssertEqual(walk.segments.map(\.count), [3, 2])
        XCTAssertEqual(walk.importedAt, MineSeeds.importedAt)

        let store = TrackStore(files: files)
        MineSeeds.seed(into: store, settingsJSON: "{}")
        MineSeeds.seed(into: store, settingsJSON: "{}")
        XCTAssertEqual(store.tracks.map(\.id), [MineSeeds.trackID])
        XCTAssertEqual(TrackLines.lines(store.tracks).map { $0.points.count }, [3, 2])
    }
}
