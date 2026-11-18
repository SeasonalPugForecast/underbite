import Foundation
import MapKit
import UnderbiteKit

/// The catalogue ids of the access group's layers.
nonisolated enum AccessLayers {
    static let publicLand = "public-land"
    static let accessPoints = "access-points"
    static let stockingPoints = "stocking-points"
    /// The layers a "Find ramps and piers here" press looks up with `plan_access`.
    static let access: Set<String> = [publicLand, accessPoints]
}

/// What a mark on the map stands for. Each kind has its own symbol, so kinds are told apart
/// without colour; the colour carries the access the source records (``AccessMarkRole``).
nonisolated enum AccessMarkKind: String, CaseIterable, Sendable {
    case ramp
    case pier
    case fishingSpot = "fishing_spot"
    case parking
    case publicLand = "public_land"
    /// A stocked water.
    case stocking
    /// A kind this version does not know yet.
    case other

    /// The kind the core names (`AccessPin.kind`).
    init(core kind: String) {
        switch AccessMarkKind(rawValue: kind) {
        case .some(let known) where known != .stocking: self = known
        default: self = .other
        }
    }

    /// Which kind leads a group of marks, lowest first: the places to put in or fish from
    /// before parking and the land around them.
    var precedence: Int {
        switch self {
        case .ramp: 0
        case .pier: 1
        case .fishingSpot: 2
        case .stocking: 3
        case .parking: 4
        case .other: 5
        case .publicLand: 6
        }
    }

    /// The SF Symbol drawn on the mark.
    var symbol: String {
        switch self {
        case .ramp: "sailboat.fill"
        case .pier: "building.columns.fill"
        case .fishingSpot: "figure.fishing"
        case .parking: "parkingsign"
        case .publicLand: "tree.fill"
        case .stocking: "fish.fill"
        case .other: "mappin"
        }
    }
}

/// How a mark's access is shown besides its colour: never by colour alone.
nonisolated enum AccessBadge: Sendable {
    case none
    /// Restricted: a "!" badge.
    case caution
    /// Closed: a cross badge.
    case closed
}

/// The access the source records for a place (`AccessPin.access`), with the app's colour for
/// it from the Okabe-Ito set; a stocked water has its own role. An unknown access is drawn as a
/// dashed outline.
nonisolated enum AccessMarkRole: String, CaseIterable, Sendable {
    case open
    case restricted
    case closed
    case unknown
    case stocking

    /// The role the core names; one this version does not know is shown as unknown.
    init(core access: String) {
        switch AccessMarkRole(rawValue: access) {
        case .some(let known) where known != .stocking: self = known
        default: self = .unknown
        }
    }

    var colour: PinRGB {
        switch self {
        case .open: PinRGB(0x009E73)
        case .restricted: PinRGB(0xE69F00)
        case .closed: PinRGB(0xD55E00)
        case .unknown: PinRGB(0x6B6B6B)
        case .stocking: PinRGB(0x0072B2)
        }
    }

    /// Filled, or an outline only (unknown).
    var filled: Bool { self != .unknown }

    var badge: AccessBadge {
        switch self {
        case .restricted: .caution
        case .closed: .closed
        default: .none
        }
    }

    /// The symbol's colour on a filled mark: white or ink, whichever stands out more.
    var glyph: PinRGB {
        colour.contrast(with: .white) >= colour.contrast(with: .ink) ? .white : .ink
    }
}

/// What a mark was made from.
nonisolated enum AccessItem: Hashable, Sendable {
    case access(AccessPin)
    case stocking(StockingPin)
}

/// One place on the map, in map points.
nonisolated struct AccessMark: Identifiable, Hashable, Sendable {
    /// The core's id.
    let id: String
    /// The catalogue layer that shows it.
    let layer: String
    let kind: AccessMarkKind
    let role: AccessMarkRole
    let latitude: Double
    let longitude: Double
    /// MapKit map points.
    let x: Double
    let y: Double
    let item: AccessItem

    init(
        id: String, layer: String, kind: AccessMarkKind, role: AccessMarkRole, latitude: Double,
        longitude: Double, item: AccessItem
    ) {
        self.id = id
        self.layer = layer
        self.kind = kind
        self.role = role
        self.latitude = latitude
        self.longitude = longitude
        let point = MKMapPoint(CLLocationCoordinate2D(latitude: latitude, longitude: longitude))
        x = point.x
        y = point.y
        self.item = item
    }

    /// A place `plan_access` found: public land in its own layer, the rest in the access points.
    init(_ pin: AccessPin) {
        let kind = AccessMarkKind(core: pin.kind)
        self.init(
            id: pin.id,
            layer: kind == .publicLand ? AccessLayers.publicLand : AccessLayers.accessPoints,
            kind: kind, role: AccessMarkRole(core: pin.access), latitude: pin.lat, longitude: pin.lon,
            item: .access(pin))
    }

    /// A stocked water `plan_stocking` found.
    init(_ pin: StockingPin) {
        self.init(
            id: pin.id, layer: AccessLayers.stockingPoints, kind: .stocking, role: .stocking,
            latitude: pin.lat, longitude: pin.lon, item: .stocking(pin))
    }

    /// The source to credit for it (`padus`, `overpass`, `ny-dec-stocking`).
    var source: String {
        switch item {
        case .access(let pin): pin.source
        case .stocking: "ny-dec-stocking"
        }
    }

    /// The name shown.
    var title: String {
        switch item {
        case .access(let pin): pin.title
        case .stocking(let pin): pin.name
        }
    }

    /// The core's words under the name.
    var subtitle: String {
        switch item {
        case .access(let pin): pin.kindLabel
        case .stocking(let pin): pin.label
        }
    }

    /// What VoiceOver reads.
    var accessibility: String {
        switch item {
        case .access(let pin): pin.accessibility
        case .stocking(let pin): pin.name + ". " + pin.label
        }
    }
}

/// One mark drawn: a place, or places too close to tell apart at the zoom shown.
nonisolated struct AccessCluster: Identifiable, Hashable, Sendable {
    /// The marks, the one the group stands at first.
    let marks: [AccessMark]

    var id: String { marks.first.map { "\($0.id)+\(marks.count)" } ?? "" }
    var x: Double { marks.first?.x ?? 0 }
    var y: Double { marks.first?.y ?? 0 }
    var single: AccessMark? { marks.count == 1 ? marks.first : nil }
    var coordinate: CLLocationCoordinate2D { MKMapPoint(x: x, y: y).coordinate }
}

/// The access and stocking marks: which are shown, how they group at a zoom, which one a tap
/// meets. They are drawn by one overlay renderer, so they are grouped here.
nonisolated enum AccessMarks {
    /// Marks nearer each other than this, in points, share one mark.
    static let clusterRadius = 28.0
    /// How far from a mark's centre a tap still meets it, in points (a 44-point target).
    static let reach = 22.0

    /// The marks of the lookups' results, each once (the first of an id wins).
    static func marks(access: [AccessPin], stocking: [StockingPin]) -> [AccessMark] {
        var seen = Set<String>()
        var out: [AccessMark] = []
        for mark in access.map(AccessMark.init) + stocking.map(AccessMark.init)
        where seen.insert(mark.id).inserted {
            out.append(mark)
        }
        return out
    }

    /// The marks drawn for the layers shown, in the order they lead groups: by
    /// ``AccessMarkKind/precedence``, each kind in the core's order. Public land is drawn as its
    /// areas, without marks: hundreds of parcels would hide the places to fish from.
    static func shown(_ marks: [AccessMark], layers: Set<String>) -> [AccessMark] {
        marks.enumerated()
            .filter { layers.contains($0.element.layer) && $0.element.kind != .publicLand }
            .sorted { a, b in
                (a.element.kind.precedence, a.offset) < (b.element.kind.precedence, b.offset)
            }
            .map(\.element)
    }

    /// Groups `marks` (in priority order) at the zoom where a point is `mapPointsPerPoint` map
    /// points: each joins the nearest group within `radius` points of it, else starts one at its
    /// own position. Deterministic for the same marks and zoom.
    static func clusters(
        _ marks: [AccessMark], mapPointsPerPoint: Double, radius: Double = clusterRadius
    ) -> [AccessCluster] {
        let cell = max(radius * mapPointsPerPoint, .leastNonzeroMagnitude)
        var groups: [[AccessMark]] = []
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
        return groups.map { AccessCluster(marks: $0) }
    }

    /// The group a tap at map point (`x`, `y`) meets: the nearest within `reach` points.
    static func hit(
        _ clusters: [AccessCluster], x: Double, y: Double, mapPointsPerPoint: Double,
        reach: Double = reach
    ) -> AccessCluster? {
        let limit = reach * mapPointsPerPoint
        var best: (cluster: AccessCluster, distance: Double)?
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

    private struct Cell: Hashable {
        let x: Int
        let y: Int
    }
}
