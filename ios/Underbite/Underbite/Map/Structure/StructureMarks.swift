import Foundation
import MapKit
@preconcurrency import UnderbiteKit

/// The catalogue layers the structure module draws, told apart by how the catalogue draws them:
/// the charted points (`EncPoints`), New York's artificial reefs, and every other state feature
/// layer, which is a lake contour layer.
nonisolated enum StructureLayerIDs {
    static let reefs = "artificial-reefs-ny"

    /// Whether the module draws `layer`.
    static func drawsLayer(_ layer: LayerInfo) -> Bool {
        layer.draw == .encPoints || layer.draw == .features
    }

    /// Whether `layer` is drawn as marks (points or reefs) rather than lines.
    static func isMarks(_ layer: LayerInfo) -> Bool {
        layer.draw == .encPoints || layer.id == reefs
    }

    /// Whether `layer` is a state's lake contours.
    static func isContours(_ layer: LayerInfo) -> Bool {
        layer.draw == .features && layer.id != reefs
    }
}

/// What a mark stands for. Each kind has its own outline and symbol, so kinds are told apart
/// without colour.
nonisolated enum StructureMarkKind: Int, CaseIterable, Sendable {
    case reef
    case wreck
    case fishHaven
    case obstruction
    case rock

    /// The kind of a charted point.
    init(_ point: StructurePoint) {
        if point.fishHaven {
            self = .fishHaven
            return
        }
        switch point.kind {
        case .wreck: self = .wreck
        case .obstruction: self = .obstruction
        case .rock: self = .rock
        }
    }

    /// Which kind leads a group of marks, lowest first.
    var precedence: Int { rawValue }

    /// The SF Symbol drawn on the mark.
    var symbol: String {
        switch self {
        case .reef: "square.stack.3d.up.fill"
        case .wreck: "ferry.fill"
        case .fishHaven: "fish.fill"
        case .obstruction: "exclamationmark"
        case .rock: "mountain.2.fill"
        }
    }

    /// The mark's outline: a square for a reef, a circle for a fish haven, the chart's diamond
    /// for the other charted points.
    var outline: StructureOutline {
        switch self {
        case .reef: .square
        case .fishHaven: .circle
        case .wreck, .obstruction, .rock: .diamond
        }
    }

    /// The fill, from the Okabe-Ito set: the reefs a state lists apart from what the chart shows.
    var colour: PinRGB {
        switch self {
        case .reef: PinRGB(0x009E73)
        case .fishHaven: PinRGB(0x56B4E9)
        case .wreck, .obstruction, .rock: PinRGB(0xCC79A7)
        }
    }

    /// The symbol's colour: white or ink, whichever stands out more on the fill.
    var glyph: PinRGB {
        colour.contrast(with: .white) >= colour.contrast(with: .ink) ? .white : .ink
    }
}

/// The outline of a mark.
nonisolated enum StructureOutline: Sendable {
    case square
    case circle
    case diamond
}

/// What a mark was made from.
nonisolated enum StructureItem: Sendable {
    case point(StructurePoint)
    case reef(ReefInfo)
}

/// One charted point or reef site on the map, in map points.
nonisolated struct StructureMark: Identifiable, Hashable, Sendable {
    /// Unique among the marks drawn: the core's id of a point, the name of a reef.
    let id: String
    /// The catalogue layer that shows it.
    let layer: String
    let kind: StructureMarkKind
    let latitude: Double
    let longitude: Double
    /// MapKit map points.
    let x: Double
    let y: Double
    let item: StructureItem

    init(
        id: String, layer: String, kind: StructureMarkKind, latitude: Double, longitude: Double,
        item: StructureItem
    ) {
        self.id = id
        self.layer = layer
        self.kind = kind
        self.latitude = latitude
        self.longitude = longitude
        let point = MKMapPoint(CLLocationCoordinate2D(latitude: latitude, longitude: longitude))
        x = point.x
        y = point.y
        self.item = item
    }

    /// A charted point of `layer`.
    init(_ point: StructurePoint, layer: String) {
        self.init(
            id: point.id, layer: layer, kind: StructureMarkKind(point), latitude: point.lat,
            longitude: point.lon, item: .point(point))
    }

    /// A reef site of `layer`.
    init(_ reef: ReefInfo, layer: String) {
        self.init(
            id: "reef/" + reef.name, layer: layer, kind: .reef, latitude: reef.lat,
            longitude: reef.lon, item: .reef(reef))
    }

    static func == (a: StructureMark, b: StructureMark) -> Bool {
        a.id == b.id && a.layer == b.layer
    }

    func hash(into hasher: inout Hasher) {
        hasher.combine(id)
        hasher.combine(layer)
    }

    /// The core's name for it: the kind of a charted point ("Wreck"), the name of a reef.
    var title: String {
        switch item {
        case .point(let point): point.title
        case .reef(let reef): reef.name
        }
    }

    /// The core's words under the title: a point's charted category or name, a reef's body of
    /// water.
    var subtitle: String? {
        switch item {
        case .point(let point): point.category ?? point.name
        case .reef(let reef): reef.bodyOfWater
        }
    }

    /// What VoiceOver reads: the core's words, one after the other.
    var accessibility: String {
        [title, subtitle].compactMap { $0 }.joined(separator: ". ")
    }

    var coordinate: CLLocationCoordinate2D {
        CLLocationCoordinate2D(latitude: latitude, longitude: longitude)
    }
}

/// One mark drawn: a point, or points too close to tell apart at the zoom shown.
nonisolated struct StructureCluster: Identifiable, Hashable, Sendable {
    /// The marks, the one the group stands at first.
    let marks: [StructureMark]

    var id: String { marks.first.map { "\($0.id)+\(marks.count)" } ?? "" }
    var x: Double { marks.first?.x ?? 0 }
    var y: Double { marks.first?.y ?? 0 }
    var single: StructureMark? { marks.count == 1 ? marks.first : nil }
    var coordinate: CLLocationCoordinate2D { MKMapPoint(x: x, y: y).coordinate }
}

/// A reef site's area (the corners the state lists), in map points.
nonisolated struct StructureArea: Sendable {
    /// The id of its reef's mark.
    let id: String
    let layer: String
    let corners: [MKMapPoint]
    let bounds: MKMapRect

    init?(_ reef: ReefInfo, layer: String) {
        guard reef.corners.count >= 3 else { return nil }
        id = "reef/" + reef.name
        self.layer = layer
        corners = reef.corners.map {
            MKMapPoint(CLLocationCoordinate2D(latitude: $0.lat, longitude: $0.lon))
        }
        let xs = corners.map(\.x), ys = corners.map(\.y)
        // INVARIANT: three corners or more, so the minima and maxima exist.
        let minX = xs.min() ?? 0, minY = ys.min() ?? 0
        bounds = MKMapRect(
            x: minX, y: minY, width: max((xs.max() ?? 0) - minX, 1),
            height: max((ys.max() ?? 0) - minY, 1))
    }

    /// Whether map point (`x`, `y`) lies inside the area (even-odd rule).
    func contains(x: Double, y: Double) -> Bool {
        guard bounds.contains(MKMapPoint(x: x, y: y)) else { return false }
        var inside = false
        var j = corners.count - 1
        for i in corners.indices {
            let (a, b) = (corners[i], corners[j])
            if (a.y > y) != (b.y > y), x < (b.x - a.x) * (y - a.y) / (b.y - a.y) + a.x {
                inside.toggle()
            }
            j = i
        }
        return inside
    }
}

/// The structure marks: which are made, how they group at a zoom, which one a tap meets. They
/// are drawn by one overlay renderer, so they are grouped here.
nonisolated enum StructureMarks {
    /// Marks nearer each other than this, in points, share one mark.
    static let clusterRadius = 26.0
    /// How far from a mark's centre a tap still meets it, in points (a 44-point target).
    static let reach = 22.0

    /// The marks of the charted points (`pointsLayer`) and the reefs (`reefsLayer`), none for a
    /// layer not given, each once, in the order they lead groups: by
    /// ``StructureMarkKind/precedence``, each kind in the core's order.
    static func marks(
        points: [StructurePoint], pointsLayer: String?, reefs: [ReefInfo], reefsLayer: String?
    ) -> [StructureMark] {
        var made: [StructureMark] = []
        if let reefsLayer { made += reefs.map { StructureMark($0, layer: reefsLayer) } }
        if let pointsLayer { made += points.map { StructureMark($0, layer: pointsLayer) } }
        var seen = Set<String>()
        return made.enumerated()
            .filter { seen.insert($0.element.id).inserted }
            .sorted { a, b in
                (a.element.kind.precedence, a.offset) < (b.element.kind.precedence, b.offset)
            }
            .map(\.element)
    }

    /// The reef areas of `reefsLayer`, none when it is not given.
    static func areas(reefs: [ReefInfo], reefsLayer: String?) -> [StructureArea] {
        guard let reefsLayer else { return [] }
        return reefs.compactMap { StructureArea($0, layer: reefsLayer) }
    }

    /// Groups `marks` (in priority order) at the zoom where a point is `mapPointsPerPoint` map
    /// points: each joins the nearest group within `radius` points of it, else starts one at its
    /// own position. Deterministic for the same marks and zoom.
    static func clusters(
        _ marks: [StructureMark], mapPointsPerPoint: Double, radius: Double = clusterRadius
    ) -> [StructureCluster] {
        let cell = max(radius * mapPointsPerPoint, .leastNonzeroMagnitude)
        var groups: [[StructureMark]] = []
        var grid: [Cell: [Int]] = [:]
        for mark in marks {
            let home = Cell(
                x: Int((mark.x / cell).rounded(.down)), y: Int((mark.y / cell).rounded(.down)))
            var best: (index: Int, distance: Double)?
            for dx in -1...1 {
                for dy in -1...1 {
                    for index in grid[Cell(x: home.x + dx, y: home.y + dy)] ?? [] {
                        let seed = groups[index][0]
                        let distance = hypot(seed.x - mark.x, seed.y - mark.y)
                        if distance <= cell, distance < (best?.distance ?? .infinity) {
                            best = (index, distance)
                        }
                    }
                }
            }
            if let best {
                groups[best.index].append(mark)
            } else {
                grid[home, default: []].append(groups.count)
                groups.append([mark])
            }
        }
        return groups.map { StructureCluster(marks: $0) }
    }

    /// The group a tap at map point (`x`, `y`) meets: the nearest within `reach` points.
    static func hit(
        _ clusters: [StructureCluster], x: Double, y: Double, mapPointsPerPoint: Double,
        reach: Double = reach
    ) -> StructureCluster? {
        let limit = reach * mapPointsPerPoint
        var best: (cluster: StructureCluster, distance: Double)?
        for cluster in clusters {
            let distance = hypot(cluster.x - x, cluster.y - y)
            if distance <= limit, distance < (best?.distance ?? .infinity) {
                best = (cluster, distance)
            }
        }
        return best?.cluster
    }

    /// Whether a layer is drawn at Web Mercator zoom `zoom`: not above its shallowest zoom (a
    /// little slack, so a view the map settles a hair short of it still shows).
    static func drawn(minZoom: UInt8?, at zoom: Double) -> Bool {
        guard let minZoom else { return true }
        return zoom >= Double(minZoom) - 0.25
    }

    /// The whole zoom the core is asked at for a view at `zoom`, with the same slack.
    static func zoomStep(_ zoom: Double) -> UInt8 {
        UInt8(clamping: Int(max(0, min(22, zoom + 0.25)).rounded(.down)))
    }

    /// Map points per screen point at Web Mercator zoom `zoom` (``MapRegion/zoom``).
    static func mapPointsPerPoint(zoom: Double) -> Double {
        MapRegion.worldMapPoints / 256 / pow(2, zoom)
    }

    private struct Cell: Hashable {
        let x: Int
        let y: Int
    }
}
