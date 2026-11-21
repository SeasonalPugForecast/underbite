import Foundation
import MapKit
import UnderbiteKit

/// What a pin on the map stands for. Each kind has its own shape and symbol, so kinds are told
/// apart without colour; the colour carries the station's state (``PinRole``).
nonisolated enum PinKind: String, CaseIterable, Sendable {
    case tide
    case current
    case buoy
    case riverGauge = "river_gauge"
    case forecastGauge = "forecast_gauge"
    /// A saved spot.
    case spot
    /// A station of a kind this version does not know yet.
    case other

    /// The kind the core names (`StationPin.kind`).
    init(core kind: String) {
        switch PinKind(rawValue: kind) {
        case .some(let known) where known != .spot: self = known
        default: self = .other
        }
    }

    /// The outline drawn for the kind.
    var shape: PinShape {
        switch self {
        case .tide: .circle
        case .current: .diamond
        case .buoy: .square
        case .riverGauge: .hexagon
        case .forecastGauge: .shield
        case .spot: .drop
        case .other: .circle
        }
    }

    /// Which kind leads a group of pins, lowest first: the few stations a place is known by
    /// before the many gauges.
    var precedence: Int {
        switch self {
        case .spot: 0
        case .tide: 1
        case .current: 2
        case .buoy: 3
        case .forecastGauge: 4
        case .riverGauge: 5
        case .other: 6
        }
    }

    /// The SF Symbol drawn inside the outline.
    var symbol: String {
        switch self {
        case .tide: "water.waves"
        case .current: "arrow.left.and.right"
        case .buoy: "lifepreserver"
        case .riverGauge: "drop.fill"
        case .forecastGauge: "chart.line.uptrend.xyaxis"
        case .spot: "fish.fill"
        case .other: "mappin"
        }
    }
}

/// The outline of a pin.
nonisolated enum PinShape: Sendable {
    case circle, diamond, square, hexagon, shield, drop
}

/// A colour as red, green and blue from 0 to 255.
nonisolated struct PinRGB: Equatable, Hashable, Sendable {
    let red: UInt8
    let green: UInt8
    let blue: UInt8

    init(_ hex: UInt32) {
        red = UInt8((hex >> 16) & 0xFF)
        green = UInt8((hex >> 8) & 0xFF)
        blue = UInt8(hex & 0xFF)
    }

    /// The WCAG relative luminance, 0 (black) to 1 (white).
    var luminance: Double {
        func channel(_ value: UInt8) -> Double {
            let c = Double(value) / 255
            return c <= 0.040_45 ? c / 12.92 : pow((c + 0.055) / 1.055, 2.4)
        }
        return 0.2126 * channel(red) + 0.7152 * channel(green) + 0.0722 * channel(blue)
    }

    /// The WCAG contrast ratio against `other`, 1 to 21.
    func contrast(with other: PinRGB) -> Double {
        let (a, b) = (luminance, other.luminance)
        return (max(a, b) + 0.05) / (min(a, b) + 0.05)
    }

    static let white = PinRGB(0xFFFFFF)
    static let ink = PinRGB(0x1B1B1F)
}

/// How a pin's state is marked besides its colour: never by colour alone.
nonisolated enum PinMark: Sendable {
    /// The state is not known: an outline only, dashed.
    case unknown
    /// No recent value: an outline only.
    case hollow
    /// A value (or a kind with no state): filled.
    case solid
    /// At or above a stage that asks for attention: filled, with a "!" badge.
    case alert
}

/// The state of a station as the core names its colour role (`StationPin.state_role`), with the
/// app's colour for it. The colours are the Okabe-Ito set, which colour-blind eyes tell apart;
/// the state is also marked by ``PinMark`` and worded in the pin's label.
nonisolated enum PinRole: String, CaseIterable, Sendable {
    case none
    case unknown
    case noData = "no_data"
    case reporting
    case low
    case normal
    case elevated
    case floodMinor = "flood_minor"
    case floodModerate = "flood_moderate"
    case floodMajor = "flood_major"

    /// The role the core names; one this version does not know is shown as unknown.
    init(core role: String) {
        self = PinRole(rawValue: role) ?? .unknown
    }

    /// The fill, or the outline of a hollow pin.
    var colour: PinRGB {
        switch self {
        case .none: PinRGB(0x34435E)
        case .unknown: PinRGB(0x6B6B6B)
        case .noData: PinRGB(0x4A4A4A)
        case .reporting: PinRGB(0x0072B2)
        case .low: PinRGB(0xE69F00)
        case .normal: PinRGB(0x009E73)
        case .elevated: PinRGB(0xF0E442)
        case .floodMinor: PinRGB(0xD55E00)
        case .floodModerate: PinRGB(0xCC79A7)
        case .floodMajor: PinRGB(0x6A1B9A)
        }
    }

    var mark: PinMark {
        switch self {
        case .unknown: .unknown
        case .noData: .hollow
        case .none, .reporting, .low, .normal: .solid
        case .elevated, .floodMinor, .floodModerate, .floodMajor: .alert
        }
    }

    /// The symbol's colour on a filled pin: white or ink, whichever stands out more.
    var glyph: PinRGB {
        colour.contrast(with: .white) >= colour.contrast(with: .ink) ? .white : .ink
    }
}

/// One station or saved spot on the map, in map points.
nonisolated struct MapPin: Identifiable, Hashable, Sendable {
    /// The core's station id (`coops:8518750`), or `spot:<id>` for a saved spot.
    let id: String
    let kind: PinKind
    /// The catalogue layer that shows it.
    let layer: String
    let name: String
    /// The core's words for the pin ("Tide station", "River gauge · 412 ft³/s, 25 min ago");
    /// empty for a saved spot.
    let label: String
    let role: PinRole
    let latitude: Double
    let longitude: Double
    /// MapKit map points.
    let x: Double
    let y: Double
    /// The saved spot's id, for a spot.
    let spotID: String?
    /// The station reports live observations, not only predictions.
    let observed: Bool
    /// A harmonic (reference) station.
    let reference: Bool

    init(
        id: String, kind: PinKind, layer: String, name: String, label: String, role: PinRole,
        latitude: Double, longitude: Double, spotID: String? = nil, observed: Bool = false,
        reference: Bool = false
    ) {
        self.id = id
        self.kind = kind
        self.layer = layer
        self.name = name
        self.label = label
        self.role = role
        self.latitude = latitude
        self.longitude = longitude
        let point = MKMapPoint(CLLocationCoordinate2D(latitude: latitude, longitude: longitude))
        x = point.x
        y = point.y
        self.spotID = spotID
        self.observed = observed
        self.reference = reference
    }

    /// A station pin as `map_stations` gives it.
    init(_ pin: StationPin) {
        self.init(
            id: pin.id, kind: PinKind(core: pin.kind), layer: pin.layer, name: pin.name,
            label: pin.label, role: PinRole(core: pin.stateRole), latitude: pin.lat,
            longitude: pin.lon, observed: pin.observed, reference: pin.reference)
    }

    /// A saved spot's pin, at its exact position (it never leaves the device from here).
    static func spot(id: String, name: String, latitude: Double, longitude: Double) -> MapPin {
        MapPin(
            id: "spot:" + id, kind: .spot, layer: StationPins.spotsLayer, name: name, label: "",
            role: .none, latitude: latitude, longitude: longitude, spotID: id)
    }
}

/// One mark on the map: a single pin, or pins too close to tell apart at the zoom shown.
nonisolated struct PinCluster: Identifiable, Hashable, Sendable {
    /// The pins, the one the mark stands at first.
    let pins: [MapPin]
    /// Drawn by the overlay; false for a pin an annotation view shows (the active spot).
    let drawn: Bool

    var id: String { pins.first.map { "\($0.id)+\(pins.count)" } ?? "" }
    var x: Double { pins.first?.x ?? 0 }
    var y: Double { pins.first?.y ?? 0 }
    var single: MapPin? { pins.count == 1 ? pins.first : nil }
    var coordinate: CLLocationCoordinate2D { MKMapPoint(x: x, y: y).coordinate }
}

/// The station and spot pins on screen: which are drawn, how they group at a zoom, which one a
/// tap meets. MapKit's clustering is for annotation views only; these pins are drawn by one
/// overlay renderer, so they are grouped here.
nonisolated enum StationPins {
    /// The catalogue id of the saved spots' layer.
    static let spotsLayer = "spots"
    /// One mark at most in a square of this side, in points; the pins in it share the mark (the
    /// core's `PIN_CELL_POINTS`).
    static let cellPoints = 56.0
    /// How far from a mark's centre a tap still meets it, in points (a 44-point target).
    static let reach = 22.0
    /// How near a saved spot must be to a station, in metres, to count as at the station.
    static let atStationMetres = 30.0

    /// Map points per screen point at Web Mercator zoom `zoom` (``MapRegion/zoom``).
    static func mapPointsPerPoint(zoom: Double) -> Double {
        MapRegion.worldMapPoints / 256 / pow(2, zoom)
    }

    /// How a station's mark is drawn while a layer that fills areas (a raster, the beach zones) is
    /// on: smaller and translucent, so the cells and zones under it show. Spots keep their full
    /// mark. The tap target is the pins' own and does not change.
    static let dimmedScale = 0.75
    static let dimmedAlpha = 0.6

    /// Whether the station marks are dimmed: some layer shown is one that fills areas.
    static func dimsStations(shown: Set<String>, areaFilling: Set<String>) -> Bool {
        !shown.isDisjoint(with: areaFilling)
    }

    /// The pins of the layers shown, in the order they lead groups: saved spots, then the
    /// stations by ``PinKind/precedence``, each kind in the core's order.
    static func shown(stations: [MapPin], spots: [MapPin], layers: Set<String>) -> [MapPin] {
        (spots + stations).enumerated()
            .filter { layers.contains($0.element.layer) }
            .sorted { a, b in
                (a.element.kind.precedence, a.offset) < (b.element.kind.precedence, b.offset)
            }
            .map(\.element)
    }

    /// Thins `pins` to one mark per screen cell of ``cellPoints`` at the zoom where a point is
    /// `mapPointsPerPoint` map points, and folds the rest into the mark's count. The core decides
    /// which pin leads a cell (`thinPins`): saved spots and stations at one first, then stations
    /// with observations, then harmonic ones, then the nearest the cell's centre. The pin `apart`
    /// (the active spot) is never grouped and not drawn. Deterministic for the same pins and zoom.
    static func clusters(
        _ pins: [MapPin], mapPointsPerPoint: Double, apart: String? = nil,
        cellPoints: Double = cellPoints
    ) -> [PinCluster] {
        let zoom = log2(
            MapRegion.worldMapPoints / 256 / max(mapPointsPerPoint, .leastNonzeroMagnitude))
        let spots = pins.filter { $0.kind == .spot }
        var byID: [String: MapPin] = [:]
        var sites: [PinSite] = []
        var separate: [PinCluster] = []
        for pin in pins where byID[pin.id] == nil {
            byID[pin.id] = pin
            if pin.id == apart {
                separate.append(PinCluster(pins: [pin], drawn: false))
                continue
            }
            let atSpot =
                pin.kind == .spot
                || spots.contains {
                    MKMapPoint(x: $0.x, y: $0.y).distance(to: MKMapPoint(x: pin.x, y: pin.y))
                        <= atStationMetres
                }
            sites.append(
                PinSite(
                    id: pin.id, lat: pin.latitude, lon: pin.longitude, observed: pin.observed,
                    reference: pin.reference, always: atSpot))
        }
        let marks = thinPins(sites: sites, zoom: zoom, cellPoints: cellPoints).compactMap {
            group -> PinCluster? in
            let members = group.members.compactMap { byID[$0] }
            return members.isEmpty ? nil : PinCluster(pins: members, drawn: true)
        }
        return marks + separate
    }

    /// The pins to list for a screen reader: every pin of the marks drawn that lies in `region`,
    /// saved spots first, then the nearest the region's centre; at most `limit`.
    static func onScreen(
        _ clusters: [PinCluster], in region: MapRegion, limit: Int = 100
    ) -> [MapPin] {
        func inside(_ pin: MapPin) -> Bool {
            let lonIn =
                region.west <= region.east
                ? (region.west...region.east).contains(pin.longitude)
                : (pin.longitude >= region.west || pin.longitude <= region.east)
            return lonIn && (region.south...region.north).contains(pin.latitude)
        }
        let centre = MKMapPoint(
            CLLocationCoordinate2D(
                latitude: region.centerLatitude, longitude: region.centerLongitude))
        func distance(_ pin: MapPin) -> Double { hypot(pin.x - centre.x, pin.y - centre.y) }
        let found = clusters.filter(\.drawn).flatMap(\.pins).filter(inside)
        let sorted = found.sorted { a, b in
            if (a.kind == .spot) != (b.kind == .spot) { return a.kind == .spot }
            let (da, db) = (distance(a), distance(b))
            return da != db ? da < db : a.id < b.id
        }
        return Array(sorted.prefix(limit))
    }

    /// What a screen reader says of a pin: the station's name, then the core's words for it
    /// (its kind and state); a saved spot has its name only.
    static func spoken(_ pin: MapPin) -> String {
        pin.label.isEmpty ? pin.name : "\(pin.name), \(pin.label)"
    }

    /// The mark a tap at map point (`x`, `y`) meets: the nearest within ``reach`` points.
    static func hit(
        _ clusters: [PinCluster], x: Double, y: Double, mapPointsPerPoint: Double,
        reach: Double = reach
    ) -> PinCluster? {
        let limit = reach * mapPointsPerPoint
        var best: (cluster: PinCluster, distance: Double)?
        for cluster in clusters {
            let distance = hypot(cluster.x - x, cluster.y - y)
            if distance <= limit, distance < (best?.distance ?? .infinity) {
                best = (cluster, distance)
            }
        }
        return best?.cluster
    }

    /// The first of `spots` (id, latitude, longitude) at the station `pin`, if any.
    static func spot(
        at pin: MapPin, among spots: [(id: String, latitude: Double, longitude: Double)]
    ) -> String? {
        let station = MKMapPoint(CLLocationCoordinate2D(latitude: pin.latitude, longitude: pin.longitude))
        return spots.first { spot in
            let point = MKMapPoint(CLLocationCoordinate2D(latitude: spot.latitude, longitude: spot.longitude))
            return station.distance(to: point) <= atStationMetres
        }?.id
    }
}

/// What the pins were last asked for: the same region, layers and settings are not asked again.
nonisolated struct PinQuery: Equatable, Sendable {
    let region: MapRegion
    /// The station layers shown, bottom first.
    let layers: [String]
    let settingsJSON: String
}
