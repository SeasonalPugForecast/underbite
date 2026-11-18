import Foundation
import MapKit
@preconcurrency import UnderbiteKit

/// One species record or forecast beach on the map, at the position its source gives.
nonisolated struct RecordItem: Identifiable, Hashable, Sendable {
    enum Kind: Hashable, Sendable {
        /// A GBIF or iNaturalist record at a stated position.
        case record
        /// A USGS Nonindigenous Aquatic Species record: a species introduced where it was seen.
        case invasive
        /// A beach of the Gulf respiratory irritation forecast.
        case beach
    }

    /// The pin's id from the core.
    let id: String
    let kind: Kind
    /// MapKit map points.
    let at: MapXY
    /// The forecast level of a beach (`very_low` ... `high`, `unknown`); none for a record.
    let level: String?

    init(id: String, kind: Kind, latitude: Double, longitude: Double, level: String? = nil) {
        self.id = id
        self.kind = kind
        at = MapXY(latitude: latitude, longitude: longitude)
        self.level = level
    }
}

/// A record whose source blurred its position on purpose: drawn as the circle it lies in,
/// never as a point, which would mislead by up to the circle's radius.
nonisolated struct RecordArea: Identifiable, Hashable, Sendable {
    /// The pin's id from the core.
    let id: String
    /// The circle's centre, map points.
    let at: MapXY
    /// The circle's radius, metres, as the source states it.
    let radiusMetres: Double
    /// The radius in map points at the circle's latitude.
    let radius: Double
    let invasive: Bool

    init(id: String, latitude: Double, longitude: Double, radiusMetres: Double, invasive: Bool) {
        self.id = id
        at = MapXY(latitude: latitude, longitude: longitude)
        self.radiusMetres = radiusMetres
        radius = radiusMetres * MKMapPointsPerMeterAtLatitude(min(max(latitude, -85), 85))
        self.invasive = invasive
    }

    var coordinate: CLLocationCoordinate2D { at.coordinate }
}

/// One mark on the map: an item, or items of the same kind too close to tell apart at the zoom
/// shown.
nonisolated struct RecordMark: Identifiable, Hashable, Sendable {
    /// The items, the one the mark stands at first.
    let items: [RecordItem]

    var id: String { items.first.map { "\($0.id)+\(items.count)" } ?? "" }
    var kind: RecordItem.Kind { items.first?.kind ?? .record }
    var at: MapXY { items.first?.at ?? MapXY(x: 0, y: 0) }
    var coordinate: CLLocationCoordinate2D { at.coordinate }

    /// For beaches, the highest level among them, which the mark is coloured by.
    var level: String? {
        items.compactMap(\.level).max { RecordMarks.rank($0) < RecordMarks.rank($1) }
    }
}

/// The species records and forecast beaches on the map: which records are points and which are
/// areas, how the points group at a zoom, and which mark or area a tap meets. Marks are drawn
/// by one overlay renderer per layer, so they are grouped here (MapKit groups annotation views
/// only).
nonisolated enum RecordMarks {
    /// The catalogue id of the species records' layer.
    static let recordsLayer = "species-records"
    /// The catalogue id of the beach forecasts' layer.
    static let beachesLayer = "hab-beaches"
    /// The source id of the USGS Nonindigenous Aquatic Species database.
    static let invasiveSource = "usgs-nas"
    /// Items nearer each other than this, in points, share one mark.
    static let clusterRadius = 30.0
    /// How far from a mark's centre, or from an area's edge, a tap still meets it, in points
    /// (a 44-point target).
    static let reach = 22.0
    /// An area up to this radius on screen, in points, is met by a tap anywhere inside it; a
    /// larger one only near its edge, so a blurred record covering the view does not take every
    /// tap from the point inspector.
    static let wholeAreaRadius = 120.0

    /// Map points per screen point at Web Mercator zoom `zoom` (``MapRegion/zoom``).
    static func mapPointsPerPoint(zoom: Double) -> Double {
        MapRegion.worldMapPoints / 256 / pow(2, zoom)
    }

    /// Whether a layer is drawn at Web Mercator zoom `zoom`: from its shallowest zoom on.
    static func drawn(minZoom: UInt8?, zoom: Double) -> Bool {
        zoom + 0.01 >= Double(minZoom ?? 0)
    }

    /// The records split into points and areas. A record its source blurred is an area of the
    /// radius stated; one without a usable radius is left out rather than drawn as a point.
    static func split(_ pins: [RecordPin]) -> (points: [RecordItem], areas: [RecordArea]) {
        var points: [RecordItem] = []
        var areas: [RecordArea] = []
        for pin in pins where valid(pin.lat, pin.lon) {
            let invasive = pin.source == invasiveSource
            if pin.obscured {
                guard let radius = pin.radiusM, radius.isFinite, radius > 0 else { continue }
                areas.append(
                    RecordArea(
                        id: pin.id, latitude: pin.lat, longitude: pin.lon, radiusMetres: radius,
                        invasive: invasive))
            } else {
                points.append(
                    RecordItem(
                        id: pin.id, kind: invasive ? .invasive : .record, latitude: pin.lat,
                        longitude: pin.lon))
            }
        }
        // The largest circles first, so the smaller ones are drawn on top of them.
        areas.sort { $0.radius > $1.radius || ($0.radius == $1.radius && $0.id < $1.id) }
        return (points, areas)
    }

    /// The forecast beaches as items.
    static func beaches(_ pins: [BloomPin]) -> [RecordItem] {
        pins.filter { valid($0.lat, $0.lon) }.map {
            RecordItem(
                id: $0.id, kind: .beach, latitude: $0.lat, longitude: $0.lon, level: $0.level)
        }
    }

    private static func valid(_ latitude: Double, _ longitude: Double) -> Bool {
        latitude.isFinite && longitude.isFinite && (-90...90).contains(latitude)
            && (-180...180).contains(longitude)
    }

    /// Groups `items` at the zoom where a point is `mapPointsPerPoint` map points: each item
    /// joins the nearest mark of its own kind within `radius` points of it, else starts one at
    /// its own position. Deterministic for the same items and zoom.
    static func clusters(
        _ items: [RecordItem], mapPointsPerPoint: Double, radius: Double = clusterRadius
    ) -> [RecordMark] {
        let cell = max(radius * mapPointsPerPoint, .leastNonzeroMagnitude)
        var groups: [[RecordItem]] = []
        var grid: [Cell: [Int]] = [:]
        for item in items {
            let home = Cell(
                kind: item.kind, x: Int((item.at.x / cell).rounded(.down)),
                y: Int((item.at.y / cell).rounded(.down)))
            var best: (index: Int, distance: Double)?
            for dx in -1...1 {
                for dy in -1...1 {
                    let near = Cell(kind: item.kind, x: home.x + dx, y: home.y + dy)
                    for index in grid[near] ?? [] {
                        let seed = groups[index][0].at
                        let distance = hypot(seed.x - item.at.x, seed.y - item.at.y)
                        if distance <= cell, distance < (best?.distance ?? .infinity) {
                            best = (index, distance)
                        }
                    }
                }
            }
            if let best {
                groups[best.index].append(item)
            } else {
                grid[home, default: []].append(groups.count)
                groups.append([item])
            }
        }
        return groups.map { RecordMark(items: $0) }
    }

    /// The mark a tap at map point `at` meets: the nearest within `reach` points.
    static func hit(
        _ marks: [RecordMark], at: MapXY, mapPointsPerPoint: Double, reach: Double = reach
    ) -> RecordMark? {
        let limit = reach * mapPointsPerPoint
        var best: (mark: RecordMark, distance: Double)?
        for mark in marks {
            let distance = hypot(mark.at.x - at.x, mark.at.y - at.y)
            if distance <= limit, distance < (best?.distance ?? .infinity) {
                best = (mark, distance)
            }
        }
        return best?.mark
    }

    /// The areas a tap at map point `at` meets, the smallest first: those it falls inside when
    /// they are at most ``wholeAreaRadius`` points across on screen, else those whose edge it
    /// falls within `reach` points of.
    static func hitAreas(
        _ areas: [RecordArea], at: MapXY, mapPointsPerPoint: Double, reach: Double = reach
    ) -> [RecordArea] {
        let near = reach * mapPointsPerPoint
        let whole = wholeAreaRadius * mapPointsPerPoint
        return areas.filter { area in
            let distance = hypot(area.at.x - at.x, area.at.y - at.y)
            if area.radius <= whole { return distance <= area.radius + near }
            return abs(distance - area.radius) <= near
        }
        .sorted { $0.radius < $1.radius || ($0.radius == $1.radius && $0.id < $1.id) }
    }

    /// The order of the forecast levels, `unknown` lowest.
    static func rank(_ level: String) -> Int {
        switch level {
        case "very_low": 1
        case "low": 2
        case "moderate": 3
        case "high": 4
        default: 0
        }
    }

    /// The records of `next` added to `kept` (a next page): those already there are not
    /// repeated, and the order is kept.
    static func merge(_ kept: [RecordPin], _ next: [RecordPin]) -> [RecordPin] {
        var seen = Set(kept.map(\.id))
        var out = kept
        for pin in next where seen.insert(pin.id).inserted {
            out.append(pin)
        }
        return out
    }

    private struct Cell: Hashable {
        let kind: RecordItem.Kind
        let x: Int
        let y: Int
    }
}
