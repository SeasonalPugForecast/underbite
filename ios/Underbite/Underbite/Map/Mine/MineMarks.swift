import Foundation
import MapKit
@preconcurrency import UnderbiteKit

/// A point in MapKit map points, as the map's own data keeps it.
nonisolated struct MapXY: Equatable, Hashable, Sendable {
    var x: Double
    var y: Double

    init(x: Double, y: Double) {
        self.x = x
        self.y = y
    }

    init(latitude: Double, longitude: Double) {
        let point = MKMapPoint(CLLocationCoordinate2D(latitude: latitude, longitude: longitude))
        x = point.x
        y = point.y
    }

    var coordinate: CLLocationCoordinate2D { MKMapPoint(x: x, y: y).coordinate }
}

// MARK: - Catches

/// One logged catch on the map, at its exact position: it is drawn on the device and never sent.
nonisolated struct CatchPoint: Identifiable, Hashable, Sendable {
    /// The catch's id in the log.
    let id: String
    let latitude: Double
    let longitude: Double
    /// MapKit map points.
    let at: MapXY

    init(id: String, latitude: Double, longitude: Double) {
        self.id = id
        self.latitude = latitude
        self.longitude = longitude
        at = MapXY(latitude: latitude, longitude: longitude)
    }
}

/// One mark on the map: a single catch, or catches too close to tell apart at the zoom shown.
nonisolated struct CatchMark: Identifiable, Hashable, Sendable {
    /// The catches, the one the mark stands at first.
    let catches: [CatchPoint]

    var id: String { catches.first.map { "\($0.id)+\(catches.count)" } ?? "" }
    var at: MapXY { catches.first?.at ?? MapXY(x: 0, y: 0) }
    var coordinate: CLLocationCoordinate2D { at.coordinate }
}

/// The catches on the map: where each one is, how they group at a zoom, and which mark a tap
/// meets. The marks are drawn by one overlay renderer, so they are grouped here (MapKit groups
/// annotation views only).
nonisolated enum CatchMarks {
    /// The catalogue id of the catches' layer.
    static let layer = "catches"
    /// Catches nearer each other than this, in points, share one mark.
    static let clusterRadius = 30.0
    /// How far from a mark's centre a tap still meets it, in points (a 44-point target).
    static let reach = 22.0

    /// Map points per screen point at Web Mercator zoom `zoom` (``MapRegion/zoom``).
    static func mapPointsPerPoint(zoom: Double) -> Double {
        MapRegion.worldMapPoints / 256 / pow(2, zoom)
    }

    /// The catches of the log `book` that are not deleted, newest first, at their exact
    /// positions, as the core keeps them. Nothing when the log cannot be read.
    static func load(book: Data) -> [CatchPoint] {
        // Only the ids and positions are read: no word of the settings is needed.
        guard
            let json = try? catchListView(book: book, settingsJson: "{}", spots: []),
            let list = try? CatchJSON.decoder.decode(CatchList.self, from: Data(json.utf8))
        else { return [] }
        return list.rows.compactMap { row in
            guard let fields = try? logEditor(book: book, id: row.id, settingsJson: "{}"),
                fields.latitude.isFinite, fields.longitude.isFinite,
                (-90...90).contains(fields.latitude), (-180...180).contains(fields.longitude)
            else { return nil }
            return CatchPoint(id: row.id, latitude: fields.latitude, longitude: fields.longitude)
        }
    }

    /// Groups `catches` (newest first) at the zoom where a point is `mapPointsPerPoint` map
    /// points: each catch joins the nearest mark within `radius` points of it, else starts one at
    /// its own position. Deterministic for the same catches and zoom.
    static func clusters(
        _ catches: [CatchPoint], mapPointsPerPoint: Double, radius: Double = clusterRadius
    ) -> [CatchMark] {
        let cell = max(radius * mapPointsPerPoint, .leastNonzeroMagnitude)
        var groups: [[CatchPoint]] = []
        var grid: [Cell: [Int]] = [:]
        for point in catches {
            let home = Cell(
                x: Int((point.at.x / cell).rounded(.down)), y: Int((point.at.y / cell).rounded(.down)))
            var best: (index: Int, distance: Double)?
            for dx in -1...1 {
                for dy in -1...1 {
                    for index in grid[Cell(x: home.x + dx, y: home.y + dy)] ?? [] {
                        let seed = groups[index][0].at
                        let distance = hypot(seed.x - point.at.x, seed.y - point.at.y)
                        if distance <= cell, distance < (best?.distance ?? .infinity) {
                            best = (index, distance)
                        }
                    }
                }
            }
            if let best {
                groups[best.index].append(point)
            } else {
                grid[home, default: []].append(groups.count)
                groups.append([point])
            }
        }
        return groups.map { CatchMark(catches: $0) }
    }

    /// The mark a tap at map point `at` meets: the nearest within `reach` points.
    static func hit(
        _ marks: [CatchMark], at: MapXY, mapPointsPerPoint: Double, reach: Double = reach
    ) -> CatchMark? {
        let limit = reach * mapPointsPerPoint
        var best: (mark: CatchMark, distance: Double)?
        for mark in marks {
            let distance = hypot(mark.at.x - at.x, mark.at.y - at.y)
            if distance <= limit, distance < (best?.distance ?? .infinity) {
                best = (mark, distance)
            }
        }
        return best?.mark
    }

    private struct Cell: Hashable {
        let x: Int
        let y: Int
    }
}

// MARK: - Tracks

/// One segment of an imported track in map points, with the box around it.
nonisolated struct TrackLine: Equatable, Sendable {
    let points: [MapXY]
    let minX: Double
    let minY: Double
    let maxX: Double
    let maxY: Double

    init(_ points: [MapXY]) {
        self.points = points
        minX = points.map(\.x).min() ?? 0
        minY = points.map(\.y).min() ?? 0
        maxX = points.map(\.x).max() ?? 0
        maxY = points.map(\.y).max() ?? 0
    }

    /// Whether the line's box meets the box from (`minX`, `minY`) to (`maxX`, `maxY`).
    func meets(minX: Double, minY: Double, maxX: Double, maxY: Double) -> Bool {
        self.minX <= maxX && self.maxX >= minX && self.minY <= maxY && self.maxY >= minY
    }
}

/// The imported tracks as lines on the map. A long track has thousands of points, far more than
/// the screen can show apart: each zoom level draws it simplified to what half a point shows.
nonisolated enum TrackLines {
    /// The catalogue id of the tracks' layer.
    static let layer = "tracks"
    /// How far, in screen points, a simplified line may stray from the track.
    static let tolerancePoints = 0.5

    /// Every segment of `tracks` with two points or more, in map points, in import order.
    static func lines(_ tracks: [SavedTrack]) -> [TrackLine] {
        tracks.flatMap { track in
            track.segments.compactMap { segment -> TrackLine? in
                let points = segment
                    .filter { $0.latitude.isFinite && $0.longitude.isFinite }
                    .map { MapXY(latitude: $0.latitude, longitude: $0.longitude) }
                return points.count >= 2 ? TrackLine(points) : nil
            }
        }
    }

    /// The zoom level a renderer's `zoomScale` (screen points per map point) draws at: the whole
    /// power of two at or below it.
    static func level(zoomScale: Double) -> Int {
        guard zoomScale > 0, zoomScale.isFinite else { return 0 }
        return Int(log2(zoomScale).rounded(.down))
    }

    /// The tolerance of a level, in map points: ``tolerancePoints`` at the level's own scale, so
    /// never more than twice that on screen at any scale the level draws.
    static func tolerance(level: Int) -> Double {
        tolerancePoints / pow(2, Double(level))
    }

    /// `line` simplified (Ramer–Douglas–Peucker): the fewest of its points that keep every
    /// dropped point within `tolerance` map points of the line drawn. Its ends are always kept.
    static func simplify(_ line: TrackLine, tolerance: Double) -> TrackLine {
        let points = line.points
        guard points.count > 2, tolerance > 0 else { return line }
        var keep = [Bool](repeating: false, count: points.count)
        keep[0] = true
        keep[points.count - 1] = true
        var stack = [(0, points.count - 1)]
        while let (first, last) = stack.popLast() {
            guard last > first + 1 else { continue }
            let a = points[first], b = points[last]
            var farthest = first
            var distance = 0.0
            for i in (first + 1)..<last {
                let d = Self.distance(points[i], from: a, to: b)
                if d > distance {
                    distance = d
                    farthest = i
                }
            }
            if distance > tolerance {
                keep[farthest] = true
                stack.append((first, farthest))
                stack.append((farthest, last))
            }
        }
        return TrackLine(points.indices.filter { keep[$0] }.map { points[$0] })
    }

    /// The distance from `p` to the segment from `a` to `b`.
    static func distance(_ p: MapXY, from a: MapXY, to b: MapXY) -> Double {
        let dx = b.x - a.x, dy = b.y - a.y
        let length = dx * dx + dy * dy
        guard length > 0 else { return hypot(p.x - a.x, p.y - a.y) }
        let t = min(max(((p.x - a.x) * dx + (p.y - a.y) * dy) / length, 0), 1)
        return hypot(p.x - (a.x + t * dx), p.y - (a.y + t * dy))
    }
}

// MARK: - The heat map

/// One square of the catch heat map, in map points.
nonisolated struct HeatSquare: Equatable, Sendable {
    /// Its centre.
    let at: MapXY
    /// Its side.
    let side: Double
    /// Its catches against the fullest square's, 0 to 1.
    let fraction: Double
}

/// The catch heat map: the core bins the catches on a grid fixed to the Earth
/// (`catch_cells`); the grid's squares are chosen a size that reads at the zoom shown. It is
/// drawn on the device and never shared.
nonisolated enum CatchHeat {
    /// The catalogue id of the heat map's layer.
    static let layer = "catch-heat"
    /// The least size of a square on screen, in points; it is at most twice that.
    static let targetPoints = 40.0
    /// The smallest square, in metres: finer than this says more than a logged position does.
    static let smallestMetres = 64.0
    /// The largest square, in metres.
    static let largestMetres = 131_072.0

    /// The side of the squares, in metres, at Web Mercator zoom `zoom` around `latitude`: a power
    /// of two, so the same few sizes come back as the map zooms.
    static func cellMetres(zoom: Double, latitude: Double) -> Double {
        let perMetre = MKMapPointsPerMeterAtLatitude(min(max(latitude, -85), 85))
        let metresPerPoint = CatchMarks.mapPointsPerPoint(zoom: zoom) / perMetre
        let wanted = targetPoints * metresPerPoint
        guard wanted.isFinite, wanted > 0 else { return smallestMetres }
        let side = pow(2, log2(wanted).rounded(.up))
        return min(max(side, smallestMetres), largestMetres)
    }

    /// The core's bins of every catch in `book` on squares of `cellMetres`.
    static func cells(book: Data, cellMetres: Double) -> [CatchCell] {
        let all = CatchFilter(
            species: nil, spotId: nil, area: nil, from: nil, until: nil, method: nil)
        return (try? catchCells(book: book, filter: all, cellM: cellMetres)) ?? []
    }

    /// The squares to draw for `cells` of `cellMetres`, each against the fullest one.
    static func squares(_ cells: [CatchCell], cellMetres: Double) -> [HeatSquare] {
        let most = Double(cells.map(\.count).max() ?? 0)
        guard most > 0 else { return [] }
        return cells.filter { $0.count > 0 }.map { cell in
            HeatSquare(
                at: MapXY(latitude: cell.latitude, longitude: cell.longitude),
                side: cellMetres * MKMapPointsPerMeterAtLatitude(cell.latitude),
                fraction: Double(cell.count) / most)
        }
    }

    /// The colours from few catches to the most, a light yellow to a deep magenta (the "magma"
    /// ramp, which reads in order without telling colours apart).
    static let ramp: [(fraction: Double, rgb: UInt32)] = [
        (0, 0xFCFDBF), (0.35, 0xFEA16E), (0.7, 0xE3466D), (1, 0x8C2981),
    ]

    /// The colour of `fraction` along ``ramp``: red, green and blue from 0 to 1.
    static func colour(_ fraction: Double) -> (red: Double, green: Double, blue: Double) {
        let f = min(max(fraction.isFinite ? fraction : 0, 0), 1)
        func channels(_ rgb: UInt32) -> (Double, Double, Double) {
            (
                Double((rgb >> 16) & 0xFF) / 255, Double((rgb >> 8) & 0xFF) / 255,
                Double(rgb & 0xFF) / 255
            )
        }
        var lower = ramp[0]
        for stop in ramp {
            if stop.fraction >= f {
                let (r0, g0, b0) = channels(lower.rgb)
                let (r1, g1, b1) = channels(stop.rgb)
                let span = stop.fraction - lower.fraction
                let t = span > 0 ? (f - lower.fraction) / span : 1
                return (r0 + (r1 - r0) * t, g0 + (g1 - g0) * t, b0 + (b1 - b0) * t)
            }
            lower = stop
        }
        let (r, g, b) = channels(lower.rgb)
        return (r, g, b)
    }

    /// How opaque a square of `fraction` is drawn, before the layer's own opacity: the few-catch
    /// squares lighter, so the busy places stand out.
    static func alpha(_ fraction: Double) -> Double {
        0.45 + 0.55 * min(max(fraction, 0), 1)
    }
}
