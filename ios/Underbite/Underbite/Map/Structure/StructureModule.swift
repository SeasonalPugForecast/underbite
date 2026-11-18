import MapKit
import SwiftUI
@preconcurrency import UnderbiteKit

/// Structure on the map: the charted wrecks, obstructions and rocks from NOAA's ENC Direct to GIS
/// (`enc-structure-points`), New York's artificial reef sites (`artificial-reefs-ny`) and the
/// lake contours states publish (`lake-contours-in`, `lake-contours-mn`). The ENC picture layers
/// of the same groups (seabed, special areas, depths) are tiles the map draws itself.
///
/// What is drawn is fetched by ``StructureLoader`` for the box around the view once the map has
/// settled and stayed still for a moment, and kept by box, so panning inside it asks nothing.
/// The points and reefs are one overlay with one renderer, grouped here for the zoom; the
/// contours another, under them. Only the mark picked out has an annotation view: the card about
/// it points at it, a popover on the iPad and a sheet on the iPhone. A tap on a mark or inside a
/// reef area is taken before the point inspector.
///
/// No charted depth is shown: the service states neither its unit nor its datum, and the card
/// says so in the core's words. Reef depths are the state's list with its own reference, and lake
/// contour labels are depths below the lake surface, the drawer saying so; none is put beside
/// the elevation model's approximate depth.
@MainActor
@Observable
final class StructureModule: MapModule {
    /// The module's layers switched on, for the drawer.
    private(set) var layers: [LayerInfo] = []
    /// Something on screen is being fetched.
    private(set) var busy = false

    @ObservationIgnored private weak var controller: MapController?
    @ObservationIgnored private let loader: StructureLoader
    @ObservationIgnored private let pinOverlay = StructurePinOverlay()
    @ObservationIgnored private let contourOverlay = LakeContourOverlay()
    @ObservationIgnored private var pinRenderer: StructurePinRenderer?
    @ObservationIgnored private var contourRenderer: LakeContourRenderer?
    @ObservationIgnored private var drawn = StructureDrawn()
    @ObservationIgnored private var region: MapRegion?
    @ObservationIgnored private var settingsJSON = ""
    @ObservationIgnored private var marks: [StructureMark] = []
    @ObservationIgnored private var areas: [StructureArea] = []
    @ObservationIgnored private var clusters: [StructureCluster] = []
    @ObservationIgnored private var mapPointsPerPoint = StructureMarks.mapPointsPerPoint(zoom: 3)
    /// Raised each time the loader has something new.
    @ObservationIgnored private var loaded = 0
    /// What the contour overlay was last given.
    @ObservationIgnored private var placed: (loaded: Int, layers: [String], opacity: Double)?
    @ObservationIgnored private var settle: Task<Void, Never>?
    @ObservationIgnored private var selection: StructureSelection?
    @ObservationIgnored private var selectionMark: StructureSelectionAnnotation?
    @ObservationIgnored private var card: UIViewController?
    @ObservationIgnored private let cardWatcher = StructureCardWatcher()
    @ObservationIgnored private var launchOpen = StructureLaunch.open

    /// How long the map stays still before the view is asked for.
    static let settleDelay: Duration = .milliseconds(600)

    init(
        controller: MapController,
        host: FetchHost = URLSessionHost(storeURL: AppFiles.standard.hostDirectory),
        clock: @escaping @Sendable () -> Int64 = { Int64(Date().timeIntervalSince1970) }
    ) {
        self.controller = controller
        loader = StructureLoader(host: host, clock: clock)
        loader.onChange = { [weak self] in self?.loaderChanged() }
        cardWatcher.dismissed = { [weak self] in self?.cardClosed() }
    }

    /// The catalogue's charted point and state feature layers.
    var layerIDs: Set<String> {
        Set((controller?.catalogue ?? []).filter(StructureLayerIDs.drawsLayer).map(\.id))
    }

    // MARK: - Drawing

    func overlays(in context: MapModuleContext) -> [any MKOverlay] {
        // Asked while the map view updates: what follows from a change waits for it to end.
        let next = StructureDrawn(context.layers, zoom: (context.region ?? region)?.zoom)
        if next != drawn || context.settingsJSON != settingsJSON || context.layers != layers {
            Task { @MainActor [weak self] in
                guard let self, self.follow(context) else { return }
                self.scheduleLoad()
            }
        }
        // Both or neither, so the contours always stay under the marks.
        return drawn.any ? [contourOverlay, pinOverlay] : []
    }

    func renderer(for overlay: any MKOverlay) -> MKOverlayRenderer? {
        if overlay === pinOverlay {
            let made = StructurePinRenderer(pinOverlay)
            pinRenderer = made
            return made
        }
        if overlay === contourOverlay {
            let made = LakeContourRenderer(contourOverlay)
            contourRenderer = made
            return made
        }
        return nil
    }

    func annotations(in context: MapModuleContext) -> [any MKAnnotation] {
        selectionMark.map { [$0] } ?? []
    }

    func annotationView(for annotation: any MKAnnotation, on map: MKMapView) -> MKAnnotationView? {
        guard let mark = annotation as? StructureSelectionAnnotation else { return nil }
        let view = StructureSelectionView(annotation: mark, reuseIdentifier: nil)
        let id = mark.selection
        view.onWindow = { [weak self] anchor in self?.showCard(for: id, from: anchor) }
        return view
    }

    func regionDidSettle(_ context: MapModuleContext) {
        follow(context)
        scheduleLoad()
    }

    /// Follows the layers on screen, their opacity, the zoom and the settings; true when what
    /// is to be fetched may have changed.
    @discardableResult
    private func follow(_ context: MapModuleContext) -> Bool {
        if let region = context.region { self.region = region }
        if layers != context.layers { layers = context.layers }
        let next = StructureDrawn(context.layers, zoom: (context.region ?? region)?.zoom)
        var load = next.wants != drawn.wants || context.settingsJSON != settingsJSON
        settingsJSON = context.settingsJSON
        let rebuilt = next != drawn
        drawn = next
        if let region {
            let perPoint = StructureMarks.mapPointsPerPoint(zoom: region.zoom)
            if perPoint != mapPointsPerPoint {
                mapPointsPerPoint = perPoint
                if !rebuilt { redraw() }
                load = true
            }
        }
        if rebuilt { rebuild() }
        return load
    }

    /// Asks the loader for the view once the map has stayed still a moment; at once when
    /// nothing is wanted any more (what was drawn goes).
    private func scheduleLoad() {
        settle?.cancel()
        guard let region else { return }
        let (wants, json) = (drawn.wants, settingsJSON)
        guard wants.any else {
            loader.load(region, wants: wants, settingsJSON: json)
            return
        }
        settle = Task { [weak self] in
            try? await Task.sleep(for: Self.settleDelay)
            guard !Task.isCancelled, let self else { return }
            self.loader.load(region, wants: wants, settingsJSON: json)
        }
    }

    private func loaderChanged() {
        loaded += 1
        if busy != loader.busy { busy = loader.busy }
        rebuild()
    }

    /// The marks, areas and lines of the layers drawn, from what the loader holds.
    private func rebuild() {
        marks = StructureMarks.marks(
            points: loader.points, pointsLayer: drawn.pointsLayer, reefs: loader.reefs,
            reefsLayer: drawn.reefsLayer)
        areas = StructureMarks.areas(reefs: loader.reefs, reefsLayer: drawn.reefsLayer)
        let wanted = (loaded, drawn.contourLayers, drawn.contourOpacity)
        if placed.map({ ($0.loaded, $0.layers, $0.opacity) != wanted }) ?? true {
            placed = wanted
            let lines = drawn.contourLayers.flatMap { loader.contours[$0] ?? [] }
            contourOverlay.set(LakeContours.place(lines), opacity: drawn.contourOpacity)
            contourRenderer?.setNeedsDisplay()
        }
        redraw()
        controller?.moduleDidChange()
        openAtLaunch()
    }

    /// Groups the marks at the zoom shown and draws them again.
    private func redraw() {
        clusters = StructureMarks.clusters(marks, mapPointsPerPoint: mapPointsPerPoint)
        pinOverlay.set(
            clusters, areas: areas, opacity: drawn.opacity, picked: selection?.area)
        pinRenderer?.setNeedsDisplay()
    }

    // MARK: - Taps and the card

    func handleTap(_ tap: MapTap) -> Bool {
        guard !marks.isEmpty || !areas.isEmpty else { return false }
        if abs(log2(tap.mapPointsPerPoint / mapPointsPerPoint)) > 0.05 {
            // The map moved without settling yet: group at the zoom of the tap.
            mapPointsPerPoint = tap.mapPointsPerPoint
            redraw()
        }
        let at = MKMapPoint(tap.coordinate)
        if let hit = StructureMarks.hit(
            clusters, x: at.x, y: at.y, mapPointsPerPoint: mapPointsPerPoint)
        {
            select(hit, at: hit.coordinate, area: nil)
            return true
        }
        // A reef's area: a tap inside it opens the reef.
        guard let area = areas.last(where: { $0.contains(x: at.x, y: at.y) }),
            let mark = marks.first(where: { $0.id == area.id })
        else { return false }
        select(StructureCluster(marks: [mark]), at: tap.coordinate, area: area.id)
        return true
    }

    /// Picks out a mark or a reef area: a ring goes round it (at `coordinate`) and, once the
    /// ring is on screen, its card comes up pointing at it.
    private func select(
        _ cluster: StructureCluster, at coordinate: CLLocationCoordinate2D, area: String?
    ) {
        card?.dismiss(animated: false)
        card = nil
        let chosen = StructureSelection(cluster: cluster, coordinate: coordinate, area: area)
        selection = chosen
        selectionMark = StructureSelectionAnnotation(chosen)
        redraw()
        controller?.moduleDidChange()
    }

    /// The card of the mark picked out, from its ring.
    private func showCard(for id: UUID, from anchor: UIView) {
        guard let selection, selection.id == id, card == nil, let window = anchor.window,
            var top = window.rootViewController
        else { return }
        while let next = top.presentedViewController, !next.isBeingDismissed { top = next }
        let layers = Dictionary(
            (controller?.catalogue ?? []).map { ($0.id, $0) }, uniquingKeysWith: { a, _ in a })
        let view = StructureCard(cluster: selection.cluster, layers: layers) { [weak self] in
            self?.closeCard()
        }
        let made = UIHostingController(rootView: view)
        made.modalPresentationStyle = .popover
        made.preferredContentSize = CGSize(width: 380, height: 440)
        if let popover = made.popoverPresentationController {
            popover.sourceView = anchor
            popover.sourceRect = anchor.bounds
            popover.delegate = cardWatcher
            let sheet = popover.adaptiveSheetPresentationController
            sheet.detents = [.medium(), .large()]
            sheet.prefersGrabberVisible = true
        }
        card = made
        top.present(made, animated: true)
    }

    /// "Done" in the card.
    private func closeCard() {
        guard let card else { return }
        card.dismiss(animated: true) { [weak self] in self?.cardClosed() }
    }

    /// The card went: the ring goes too.
    private func cardClosed() {
        card = nil
        selection = nil
        guard selectionMark != nil else { return }
        selectionMark = nil
        redraw()
        controller?.moduleDidChange()
    }

    /// `-mapStructureOpen` (debug builds, screenshots): the first time marks are drawn, the
    /// one nearest the middle of the view is picked out as if tapped.
    private func openAtLaunch() {
        guard launchOpen, let region, !clusters.isEmpty else { return }
        launchOpen = false
        let middle = MKMapPoint(
            CLLocationCoordinate2D(
                latitude: region.centerLatitude, longitude: region.centerLongitude))
        // A mark alone first, so its card shows the point in full.
        let alone = clusters.filter { $0.single != nil }
        let pool = alone.isEmpty ? clusters : alone
        guard
            let nearest = pool.min(by: {
                hypot($0.x - middle.x, $0.y - middle.y) < hypot($1.x - middle.x, $1.y - middle.y)
            })
        else { return }
        Task { @MainActor [weak self] in
            self?.select(nearest, at: nearest.coordinate, area: nil)
        }
    }

    // MARK: - Drawer

    func drawerSection() -> AnyView? {
        guard !layers.isEmpty else { return nil }
        return AnyView(StructureSection(module: self))
    }
}

/// What of the module's layers is drawn at the zoom on screen, and at what opacity.
nonisolated struct StructureDrawn: Equatable, Sendable {
    /// Per layer drawn, its opacity.
    var opacity: [String: Double] = [:]
    var pointsLayer: String?
    var reefsLayer: String?
    var contourLayers: [String] = []
    var contourOpacity = 1.0

    var any: Bool { !opacity.isEmpty }

    /// What the loader is to fetch for them.
    var wants: StructureLoader.Wants {
        StructureLoader.Wants(
            points: pointsLayer != nil, reefs: reefsLayer != nil, contours: contourLayers)
    }

    init() {}

    /// Which of `layers` are drawn at `zoom`; none before the map has settled.
    init(_ layers: [LayerInfo], zoom: Double?) {
        guard let zoom else { return }
        var contourOpacity: Double?
        for layer in layers where StructureMarks.drawn(minZoom: layer.minZoom, at: zoom) {
            opacity[layer.id] = Double(layer.opacity)
            if layer.draw == .encPoints {
                pointsLayer = pointsLayer ?? layer.id
            } else if StructureLayerIDs.isMarks(layer) {
                reefsLayer = layer.id
            } else if StructureLayerIDs.isContours(layer) {
                contourLayers.append(layer.id)
                contourOpacity = contourOpacity ?? Double(layer.opacity)
            }
        }
        self.contourOpacity = contourOpacity ?? 1
    }
}

/// The mark picked out.
struct StructureSelection: Identifiable {
    let id = UUID()
    let cluster: StructureCluster
    /// Where its ring goes: the mark, or the tap inside a reef area.
    let coordinate: CLLocationCoordinate2D
    /// The reef area tapped, drawn stronger while its card is up.
    let area: String?
}

/// Hears the card being swiped or tapped away.
private final class StructureCardWatcher: NSObject, UIPopoverPresentationControllerDelegate {
    var dismissed: (@MainActor () -> Void)?

    func presentationControllerDidDismiss(_ presentationController: UIPresentationController) {
        dismissed?()
    }
}

/// The ring around the mark picked out.
final class StructureSelectionAnnotation: NSObject, MKAnnotation {
    let coordinate: CLLocationCoordinate2D
    let selection: UUID

    init(_ chosen: StructureSelection) {
        coordinate = chosen.coordinate
        selection = chosen.id
    }
}

/// A ring in the accent colour, a little larger than a mark; it takes no touches. Once on
/// screen it tells the module, which points the card at it.
final class StructureSelectionView: MKAnnotationView {
    var onWindow: (@MainActor (UIView) -> Void)?
    private let ring = CAShapeLayer()

    override init(annotation: (any MKAnnotation)?, reuseIdentifier: String?) {
        super.init(annotation: annotation, reuseIdentifier: reuseIdentifier)
        let side = 46.0
        frame = CGRect(x: 0, y: 0, width: side, height: side)
        ring.path = CGPath(
            ellipseIn: CGRect(x: 3, y: 3, width: side - 6, height: side - 6), transform: nil)
        ring.fillColor = nil
        ring.lineWidth = 4
        ring.shadowColor = UIColor.black.cgColor
        ring.shadowOpacity = 0.4
        ring.shadowRadius = 2
        ring.shadowOffset = .zero
        layer.addSublayer(ring)
        isUserInteractionEnabled = false
        canShowCallout = false
        displayPriority = .required
        zPriority = .max
        updateColours()
        registerForTraitChanges([UITraitUserInterfaceStyle.self]) {
            (view: StructureSelectionView, _: UITraitCollection) in
            view.updateColours()
        }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { nil }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        guard window != nil, let onWindow else { return }
        self.onWindow = nil
        // After the map has placed the view.
        Task { @MainActor [weak self] in
            guard let self, self.window != nil else { return }
            onWindow(self)
        }
    }

    private func updateColours() {
        ring.strokeColor = UIColor(Palette.accent).resolvedColor(with: traitCollection).cgColor
    }
}

/// The structure module's launch argument, read in debug builds only (screenshots):
/// `-mapStructureOpen` picks out the mark nearest the middle of the view once marks are drawn.
enum StructureLaunch {
    static var open: Bool {
        #if DEBUG
            ProcessInfo.processInfo.arguments.contains("-mapStructureOpen")
        #else
            false
        #endif
    }
}
