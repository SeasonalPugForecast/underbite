import MapKit
import SwiftUI
import UnderbiteKit

/// What is shown about a tapped pin: a sheet on the iPhone, a popover pointing at the pin on the
/// iPad. It also hands the saved spots to the ``StationsModule``.
struct StationLayer: View {
    let controller: MapController
    @Environment(SpotStore.self) private var store

    init(controller: MapController) {
        self.controller = controller
    }

    private var module: StationsModule? {
        controller.modules.lazy.compactMap { $0 as? StationsModule }.first
    }

    var body: some View {
        if let module {
            PinPopoverAnchor(module: module, catalogue: controller.catalogue)
                .onAppear { module.setSpots(store.spots, active: store.activeID) }
                .onChange(of: store.spots) { _, spots in
                    module.setSpots(spots, active: store.activeID)
                }
                .onChange(of: store.activeID) { _, active in
                    module.setSpots(store.spots, active: active)
                }
        }
    }
}

/// An empty point at the tapped pin that the popover points at; it takes no touches.
private struct PinPopoverAnchor: View {
    let module: StationsModule
    let catalogue: [LayerInfo]

    var body: some View {
        let selection = Binding(get: { module.selection }, set: { module.select($0) })
        let layers = Dictionary(
            catalogue.map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })
        GeometryReader { _ in
            Color.clear
                .frame(width: 1, height: 1)
                .position(module.selection?.point ?? .zero)
                .popover(item: selection, attachmentAnchor: .point(.center)) { chosen in
                    PinSheet(cluster: chosen.cluster, layers: layers)
                        .frame(idealWidth: 380, idealHeight: 420)
                        .presentationDetents([.medium, .large])
                        .presentationDragIndicator(.visible)
                        .presentationCompactAdaptation(.sheet)
                }
        }
        // The map view under it ignores the safe area too: the same points.
        .ignoresSafeArea()
        .accessibilityHidden(true)
    }
}

/// The pin tapped and where it is on screen.
struct PinSelection: Identifiable, Equatable {
    let id = UUID()
    let cluster: PinCluster
    /// The pin's centre on the map view, in points, when it was tapped.
    let point: CGPoint
}

/// Draws the station and spot layers: tide and current stations, buoys, river and forecast
/// gauges from `map_stations` for the region on screen, and the saved spots ("Mine") at their
/// exact positions. All of them are one overlay with its own renderer and hit-testing; pins too
/// close to tell apart at the zoom shown are grouped here (MapKit groups annotation views only).
/// Annotation views are only for the pin picked out and the active spot.
///
/// The pins are asked for once the map has settled, a moment after the last move, and not again
/// for the same region, layers and settings; the core reads stored station lists and asks for
/// the state of at most 25 visible gauges per call.
@MainActor
@Observable
final class StationsModule: MapModule {
    /// The pin tapped, while its sheet is up.
    private(set) var selection: PinSelection?

    @ObservationIgnored private weak var controller: MapController?
    @ObservationIgnored private let host: FetchHost
    @ObservationIgnored private let keys: @Sendable () -> Keys
    @ObservationIgnored private let clock: @Sendable () -> Int64
    @ObservationIgnored private let overlay = StationPinOverlay()
    @ObservationIgnored private var renderer: StationPinRenderer?
    @ObservationIgnored private var stationPins: [MapPin] = []
    @ObservationIgnored private var spotPins: [MapPin] = []
    @ObservationIgnored private var activeSpot: (pin: MapPin, mark: ActiveSpotAnnotation)?
    @ObservationIgnored private var selectionMark: PinSelectionAnnotation?
    @ObservationIgnored private var shownLayers: Set<String> = []
    /// Whether a layer that fills areas is on, so the station marks are drawn lighter.
    @ObservationIgnored private var dimsStations = false
    @ObservationIgnored private var clusters: [PinCluster] = []
    @ObservationIgnored private var mapPointsPerPoint = StationPins.mapPointsPerPoint(zoom: 3)
    @ObservationIgnored private var asked: PinQuery?
    @ObservationIgnored private var pending: Task<Void, Never>?
    @ObservationIgnored private var generation = 0
    @ObservationIgnored private var launchPinShown = false
    @ObservationIgnored private var region: MapRegion?

    /// How long the map must rest before the pins are asked for.
    static let settle: Duration = .milliseconds(450)

    init(
        controller: MapController,
        host: FetchHost = URLSessionHost(storeURL: AppFiles.standard.hostDirectory),
        keys: @escaping @Sendable () -> Keys = { Keychain.keys() },
        clock: @escaping @Sendable () -> Int64 = { Int64(Date().timeIntervalSince1970) }
    ) {
        self.controller = controller
        self.host = host
        self.keys = keys
        self.clock = clock
    }

    var layerIDs: Set<String> {
        Set(
            (controller?.catalogue ?? [])
                .filter { $0.draw == .stations || $0.draw == .spots }
                .map(\.id))
    }

    // MARK: - Drawing

    func overlays(in context: MapModuleContext) -> [any MKOverlay] {
        follow(context)
        return shownLayers.isEmpty ? [] : [overlay]
    }

    func renderer(for overlay: any MKOverlay) -> MKOverlayRenderer? {
        guard overlay === self.overlay else { return nil }
        let made = StationPinRenderer(self.overlay)
        renderer = made
        return made
    }

    func annotations(in context: MapModuleContext) -> [any MKAnnotation] {
        var shown: [any MKAnnotation] = []
        if let activeSpot, shownLayers.contains(StationPins.spotsLayer) {
            shown.append(activeSpot.mark)
        }
        if let selectionMark { shown.append(selectionMark) }
        return shown
    }

    func annotationView(for annotation: any MKAnnotation, on map: MKMapView) -> MKAnnotationView? {
        switch annotation {
        case let spot as ActiveSpotAnnotation:
            let view =
                map.dequeueReusableAnnotationView(withIdentifier: ActiveSpotAnnotation.reuse)
                as? MKMarkerAnnotationView
                ?? MKMarkerAnnotationView(annotation: spot, reuseIdentifier: ActiveSpotAnnotation.reuse)
            view.annotation = spot
            view.markerTintColor = PinDrawing.uiColor(PinRGB(0x005BB5))
            view.glyphImage = UIImage(systemName: PinKind.spot.symbol)
            view.titleVisibility = .visible
            view.displayPriority = .required
            view.canShowCallout = false
            // Taps go to the map, whose hit-testing knows this spot.
            view.isUserInteractionEnabled = false
            return view
        case let mark as PinSelectionAnnotation:
            let view =
                map.dequeueReusableAnnotationView(withIdentifier: PinSelectionAnnotation.reuse)
                as? PinSelectionView
                ?? PinSelectionView(annotation: mark, reuseIdentifier: PinSelectionAnnotation.reuse)
            view.annotation = mark
            return view
        default:
            return nil
        }
    }

    /// Follows the layers on screen and the zoom: the marks are grouped again.
    private func follow(_ context: MapModuleContext) {
        let layers = Set(context.layers.map(\.id))
        var changed = layers != shownLayers
        shownLayers = layers
        // Every layer on screen, not only this module's: the area layers are other modules'.
        let shownAll = controller?.plan.shown ?? []
        let dims = StationPins.dimsStations(
            shown: Set(shownAll.map(\.id)),
            areaFilling: Set(
                shownAll.filter { $0.draw == .gridRaster || $0.id == BeachZones.layer }
                    .map(\.id)))
        if dims != dimsStations {
            dimsStations = dims
            changed = true
        }
        if let region = context.region {
            self.region = region
            let perPoint = StationPins.mapPointsPerPoint(zoom: region.zoom)
            if perPoint != mapPointsPerPoint {
                mapPointsPerPoint = perPoint
                changed = true
            }
        }
        if changed { regroup() }
    }

    /// Groups the pins of the shown layers at the zoom shown and draws them again.
    private func regroup() {
        let pins = StationPins.shown(stations: stationPins, spots: spotPins, layers: shownLayers)
        clusters = StationPins.clusters(
            pins, mapPointsPerPoint: mapPointsPerPoint, apart: activeSpot?.pin.id)
        overlay.set(clusters, dimmed: dimsStations)
        renderer?.setNeedsDisplay()
    }

    // MARK: - Loading

    func regionDidSettle(_ context: MapModuleContext) {
        follow(context)
        let stationLayers = context.layers.filter { $0.draw == .stations }.map(\.id)
        guard let region = context.region, !stationLayers.isEmpty else {
            pending?.cancel()
            asked = nil
            generation += 1
            if !stationPins.isEmpty {
                stationPins = []
                regroup()
                controller?.moduleDidChange()
            }
            return
        }
        let query = PinQuery(
            region: region, layers: stationLayers, settingsJSON: context.settingsJSON)
        guard query != asked else { return }
        pending?.cancel()
        pending = Task { [weak self] in
            try? await Task.sleep(for: Self.settle)
            guard !Task.isCancelled else { return }
            await self?.load(query)
        }
    }

    private func load(_ query: PinQuery) async {
        asked = query
        generation += 1
        let mine = generation
        let found = await Self.fetch(query, keys: keys(), now: clock(), host: host)
        guard mine == generation else { return }
        guard let found else {
            // Asked again on the next settle.
            asked = nil
            return
        }
        stationPins = found
        regroup()
        controller?.moduleDidChange()
        #if DEBUG
            selectLaunchPin(in: query.region)
        #endif
    }

    #if DEBUG
        /// `-mapPin <id>` (debug builds, screenshots): once the pins have come, the pin `id`
        /// (`coops:8518750`, `spot:<id>`) is picked out as if tapped, once.
        private func selectLaunchPin(in region: MapRegion) {
            guard !launchPinShown, let id = LaunchArguments.value(after: "-mapPin"),
                let hit = clusters.first(where: { $0.pins.contains { $0.id == id } })
            else { return }
            launchPinShown = true
            // The map view's top left is the region's north-west corner.
            let corner = MKMapPoint(
                CLLocationCoordinate2D(latitude: region.north, longitude: region.west))
            select(
                PinSelection(
                    cluster: hit,
                    point: CGPoint(
                        x: (hit.x - corner.x) / mapPointsPerPoint,
                        y: (hit.y - corner.y) / mapPointsPerPoint)))
        }
    #endif

    /// The pins `map_stations` gives for `query`; nil when the call failed (lists and states
    /// that failed leave their pins out or without state, and are not a failure).
    @concurrent
    private nonisolated static func fetch(
        _ query: PinQuery, keys: Keys, now: Int64, host: FetchHost
    ) async -> [MapPin]? {
        let region = query.region
        guard
            let found = try? await mapStations(
                west: region.west, south: region.south, east: region.east, north: region.north,
                zoom: region.zoom, settingsJson: query.settingsJSON, keys: keys, now: now,
                host: host)
        else { return nil }
        return found.pins.map(MapPin.init)
    }

    // MARK: - Spots

    /// The saved spots and the active one, from the app's store.
    func setSpots(_ spots: [Spot], active: String?) {
        spotPins = spots.map {
            MapPin.spot(id: $0.id, name: $0.name, latitude: $0.latitude, longitude: $0.longitude)
        }
        if let spot = spotPins.first(where: { $0.spotID == active && active != nil }) {
            if activeSpot?.pin != spot {
                activeSpot = (spot, ActiveSpotAnnotation(spot))
            }
        } else {
            activeSpot = nil
        }
        regroup()
        controller?.moduleDidChange()
    }

    // MARK: - Taps

    func handleTap(_ tap: MapTap) -> Bool {
        guard !shownLayers.isEmpty, !clusters.isEmpty else { return false }
        if abs(log2(tap.mapPointsPerPoint / mapPointsPerPoint)) > 0.05 {
            // The map moved without settling yet: group at the zoom of the tap.
            mapPointsPerPoint = tap.mapPointsPerPoint
            regroup()
        }
        let at = MKMapPoint(tap.coordinate)
        guard
            let hit = StationPins.hit(
                clusters, x: at.x, y: at.y, mapPointsPerPoint: mapPointsPerPoint)
        else { return false }
        let point = CGPoint(
            x: tap.point.x + (hit.x - at.x) / mapPointsPerPoint,
            y: tap.point.y + (hit.y - at.y) / mapPointsPerPoint)
        select(PinSelection(cluster: hit, point: point))
        return true
    }

    // MARK: - Screen readers

    /// The pins on screen for a screen reader's list: what the drawn marks stand for, saved
    /// spots first, then the nearest the middle of the map.
    func pinsOnScreen() -> [MapPin] {
        guard let region else { return [] }
        return StationPins.onScreen(clusters, in: region)
    }

    /// The module's drawer section: the list of the pins on screen.
    func drawerSection() -> AnyView? {
        guard !shownLayers.isEmpty else { return nil }
        return AnyView(StationListSection(module: self))
    }

    /// Picks out the pin `pin` as if it were tapped: its sheet comes up.
    func open(_ pin: MapPin) {
        guard let region,
            let hit = clusters.first(where: { $0.pins.contains { $0.id == pin.id } })
        else { return }
        let corner = MKMapPoint(
            CLLocationCoordinate2D(latitude: region.north, longitude: region.west))
        select(
            PinSelection(
                cluster: PinCluster(pins: [pin], drawn: hit.drawn),
                point: CGPoint(
                    x: (hit.x - corner.x) / mapPointsPerPoint,
                    y: (hit.y - corner.y) / mapPointsPerPoint)))
    }

    /// Picks out a pin (its sheet comes up), or none.
    func select(_ chosen: PinSelection?) {
        guard chosen != selection else { return }
        selection = chosen
        selectionMark = chosen.map { PinSelectionAnnotation($0.cluster) }
        controller?.moduleDidChange()
    }
}

/// The active spot, shown as a marker with its name.
final class ActiveSpotAnnotation: NSObject, MKAnnotation {
    static let reuse = "active-spot"
    let coordinate: CLLocationCoordinate2D
    let title: String?

    init(_ pin: MapPin) {
        coordinate = CLLocationCoordinate2D(latitude: pin.latitude, longitude: pin.longitude)
        title = pin.name
    }
}

/// The ring around the pin picked out.
final class PinSelectionAnnotation: NSObject, MKAnnotation {
    static let reuse = "pin-selection"
    let coordinate: CLLocationCoordinate2D

    init(_ cluster: PinCluster) {
        coordinate = cluster.coordinate
    }
}

/// A ring in the accent colour, a little larger than a pin; it takes no touches.
final class PinSelectionView: MKAnnotationView {
    private let ring = CAShapeLayer()

    override init(annotation: (any MKAnnotation)?, reuseIdentifier: String?) {
        super.init(annotation: annotation, reuseIdentifier: reuseIdentifier)
        let side = 42.0
        frame = CGRect(x: 0, y: 0, width: side, height: side)
        ring.path = CGPath(
            ellipseIn: CGRect(x: 3, y: 3, width: side - 6, height: side - 6), transform: nil)
        ring.fillColor = nil
        ring.lineWidth = 4
        layer.addSublayer(ring)
        isUserInteractionEnabled = false
        canShowCallout = false
        displayPriority = .required
        // Drawn under the pins' overlay would hide it: annotation views are always above.
        zPriority = .max
        updateColours()
        registerForTraitChanges([UITraitUserInterfaceStyle.self]) {
            (view: PinSelectionView, _: UITraitCollection) in
            view.updateColours()
        }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { nil }

    private func updateColours() {
        ring.strokeColor = UIColor(Palette.accent).resolvedColor(with: traitCollection).cgColor
        ring.shadowColor = UIColor.black.cgColor
        ring.shadowOpacity = 0.4
        ring.shadowRadius = 2
        ring.shadowOffset = .zero
    }
}
