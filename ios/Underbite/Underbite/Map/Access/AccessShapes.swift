import Foundation
import MapKit
import UnderbiteKit

/// One area or line of a lookup (public land, an OpenStreetMap way drawn whole), in map points,
/// with what it takes to draw it at any zoom: its bounds and its rings thinned for the zoom.
nonisolated struct AccessShape2D: Sendable {
    enum Form: Sendable {
        /// Rings, the first the outer boundary; filled even-odd, so holes stay open.
        case area
        case line
    }

    let id: String
    /// The catalogue layer that shows it.
    let layer: String
    let role: AccessMarkRole
    let form: Form
    let rings: [[MapXY]]
    /// The bounds of every ring, in map points.
    let bounds: MKMapRect

    init(id: String, layer: String, role: AccessMarkRole, form: Form, rings: [[MapXY]]) {
        self.id = id
        self.layer = layer
        self.role = role
        self.form = form
        self.rings = rings
        var minX = Double.infinity, minY = Double.infinity
        var maxX = -Double.infinity, maxY = -Double.infinity
        for point in rings.joined() {
            minX = min(minX, point.x)
            minY = min(minY, point.y)
            maxX = max(maxX, point.x)
            maxY = max(maxY, point.y)
        }
        bounds =
            minX.isFinite
            ? MKMapRect(x: minX, y: minY, width: maxX - minX, height: maxY - minY) : .null
    }

    /// The shape of a place `plan_access` found; none for a point.
    init?(_ pin: AccessPin) {
        let kind = AccessMarkKind(core: pin.kind)
        let layer = kind == .publicLand ? AccessLayers.publicLand : AccessLayers.accessPoints
        let role = AccessMarkRole(core: pin.access)
        func xy(_ p: MapPoint) -> MapXY {
            let m = MKMapPoint(CLLocationCoordinate2D(latitude: p.lat, longitude: p.lon))
            return MapXY(x: m.x, y: m.y)
        }
        switch pin.shape {
        case .point:
            return nil
        case .line(let points):
            guard points.count > 1 else { return nil }
            self.init(id: pin.id, layer: layer, role: role, form: .line, rings: [points.map(xy)])
        case .polygon(let rings):
            let kept = rings.filter { $0.count > 2 }.map { $0.map(xy) }
            guard !kept.isEmpty else { return nil }
            self.init(id: pin.id, layer: layer, role: role, form: .area, rings: kept)
        }
    }
}

/// Thinning shapes for the zoom they are drawn at, so a large public area costs about as much
/// to draw as it has points on screen.
nonisolated enum AccessThinning {
    /// The zoom bucket of a scale where a point is `mapPointsPerPoint` map points: one per
    /// halving, so a bucket's thinning stays under a point on screen at every zoom inside it.
    static func bucket(mapPointsPerPoint: Double) -> Int {
        guard mapPointsPerPoint > 0, mapPointsPerPoint.isFinite else { return 0 }
        return Int(log2(mapPointsPerPoint).rounded(.down))
    }

    /// The distance, map points, under which a vertex is dropped in `bucket`: half a point at
    /// the bucket's finest scale, less at the others.
    static func tolerance(bucket: Int) -> Double {
        pow(2, Double(bucket)) / 2
    }

    /// `ring` without the vertices nearer than `tolerance` to the last one kept; the first and
    /// last vertices always stay. A ring of an area with fewer than three vertices left is
    /// dropped (it is under a point or two on screen).
    static func thin(_ ring: [MapXY], tolerance: Double, closed: Bool) -> [MapXY] {
        guard ring.count > 2, tolerance > 0 else { return ring }
        var out: [MapXY] = [ring[0]]
        out.reserveCapacity(ring.count)
        let squared = tolerance * tolerance
        for point in ring.dropFirst().dropLast() {
            let last = out[out.count - 1]
            let dx = point.x - last.x, dy = point.y - last.y
            if dx * dx + dy * dy >= squared { out.append(point) }
        }
        if let end = ring.last { out.append(end) }
        if closed, out.count < 3 { return [] }
        return out
    }

    /// The area under map point (`x`, `y`) among `shapes`: the smallest whose rings hold it
    /// (even-odd, so a hole does not), none for a line.
    static func area(at x: Double, _ y: Double, in shapes: [AccessShape2D]) -> AccessShape2D? {
        let point = MKMapPoint(x: x, y: y)
        var best: (shape: AccessShape2D, size: Double)?
        for shape in shapes where shape.form == .area && shape.bounds.contains(point) {
            var inside = false
            for ring in shape.rings {
                var j = ring.count - 1
                for i in ring.indices {
                    let a = ring[i], b = ring[j]
                    if (a.y > y) != (b.y > y), x < (b.x - a.x) * (y - a.y) / (b.y - a.y) + a.x {
                        inside.toggle()
                    }
                    j = i
                }
            }
            let size = shape.bounds.width * shape.bounds.height
            if inside, size < (best?.size ?? .infinity) { best = (shape, size) }
        }
        return best?.shape
    }

    /// Whether `shape` is worth drawing at the scale: a box at least a point across.
    static func visible(_ shape: AccessShape2D, mapPointsPerPoint: Double) -> Bool {
        !shape.bounds.isNull
            && max(shape.bounds.width, shape.bounds.height) >= mapPointsPerPoint
    }
}
