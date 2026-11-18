import MapKit
import SwiftUI
import UnderbiteKit

/// The access group's layers: public land from PAD-US as areas, boat ramps, piers and fishing
/// spots from OpenStreetMap, and New York's stocked waters, each as marks. Nothing is asked as
/// the map moves or a layer is switched on: the drawer's "Find ramps and piers here" looks them
/// up for the view on screen, through the core's `plan_access` and `plan_stocking` and the app's
/// fetch host, and is the only way Overpass is ever asked. What the lookup says (busy, kept on
/// the phone, zoom in) shows under the button, with the credits of the sources it used.
///
/// The areas are one overlay with one renderer, thinned for the zoom; the marks another, grouped
/// here. Only the mark picked out has an annotation view: the card about it points at it, a
/// popover on the iPad and a sheet on the iPhone.
@MainActor
@Observable
final class AccessModule: MapModule {
    /// A lookup is running.
    private(set) var busy = false
    /// What the lookups said: the core's notices, and its "zoom in" words.
    private(set) var notices: [String] = []
    /// The standing notes to show with the places (what access means).
    private(set) var notes: [String] = []
    /// The sources of the places shown.
    private(set) var credits: [NearbyCredit] = []

    @ObservationIgnored private weak var controller: MapController?
    @ObservationIgnored private let host: FetchHost
    @ObservationIgnored private let clock: @Sendable () -> Int64
    @ObservationIgnored private let state: NearbyStateStore
    @ObservationIgnored private let shapeOverlay = AccessShapeOverlay()
    @ObservationIgnored private let pinOverlay = AccessPinOverlay()
    @ObservationIgnored private var shapeRenderer: AccessShapeRenderer?
    @ObservationIgnored private var pinRenderer: AccessPinRenderer?
    @ObservationIgnored private var access: NearbyResults?
    @ObservationIgnored private var stocking: NearbyResults?
    @ObservationIgnored private var zoomNotices: [String] = []
    @ObservationIgnored private var marks: [AccessMark] = []
    @ObservationIgnored private var shapes: [AccessShape2D] = []
    /// The layers drawn (shown, at a zoom they are drawn at) with their opacity.
    @ObservationIgnored private var drawn: [String: Double] = [:]
    @ObservationIgnored private var shapesDrawn: (layers: [String: Double], count: Int)?
    @ObservationIgnored private var clusters: [AccessCluster] = []
    @ObservationIgnored private var mapPointsPerPoint = AccessModule.mapPointsPerPoint(zoom: 3)
    @ObservationIgnored private var lookup: Task<Void, Never>?
    @ObservationIgnored private var selection: AccessSelection?
    @ObservationIgnored private var selectionMark: AccessSelectionAnnotation?
    @ObservationIgnored private var card: UIViewController?
    @ObservationIgnored private let cardWatcher = CardWatcher()
    @ObservationIgnored private var launchPressed = false

    init(
        controller: MapController,
        host: FetchHost = URLSessionHost(storeURL: AppFiles.standard.hostDirectory),
        clock: @escaping @Sendable () -> Int64 = { Int64(Date().timeIntervalSince1970) },
        state: NearbyStateStore = .shared
    ) {
        self.controller = controller
        self.host = host
        self.clock = clock
        self.state = state
        cardWatcher.dismissed = { [weak self] in self?.cardClosed() }
    }

    /// The catalogue's lookup layers of the access group.
    var layerIDs: Set<String> {
        Set(
            (controller?.catalogue ?? [])
                .filter { $0.draw == .nearby && $0.group == "access" }
                .map(\.id))
    }

    /// Map points per screen point at Web Mercator zoom `zoom` (``MapRegion/zoom``).
    nonisolated static func mapPointsPerPoint(zoom: Double) -> Double {
        MapRegion.worldMapPoints / 256 / pow(2, zoom)
    }

    // MARK: - Drawing

    func overlays(in context: MapModuleContext) -> [any MKOverlay] {
        follow(context)
        // Both or neither, so the areas always stay under the marks.
        return drawn.isEmpty ? [] : [shapeOverlay, pinOverlay]
    }

    func renderer(for overlay: any MKOverlay) -> MKOverlayRenderer? {
        if overlay === shapeOverlay {
            let made = AccessShapeRenderer(shapeOverlay)
            shapeRenderer = made
            return made
        }
        if overlay === pinOverlay {
            let made = AccessPinRenderer(pinOverlay)
            pinRenderer = made
            return made
        }
        return nil
    }

    func annotations(in context: MapModuleContext) -> [any MKAnnotation] {
        selectionMark.map { [$0] } ?? []
    }

    func annotationView(for annotation: any MKAnnotation, on map: MKMapView) -> MKAnnotationView? {
        guard let mark = annotation as? AccessSelectionAnnotation else { return nil }
        let view = AccessSelectionView(annotation: mark, reuseIdentifier: nil)
        let id = mark.selection
        view.onWindow = { [weak self] anchor in self?.showCard(for: id, from: anchor) }
        return view
    }

    /// Follows the layers on screen, their opacity and the zoom.
    private func follow(_ context: MapModuleContext) {
        var layers: [String: Double] = [:]
        for layer in context.layers
        where context.region.map({ AccessMarks.drawn(minZoom: layer.minZoom, at: $0.zoom) }) ?? true {
            layers[layer.id] = Double(layer.opacity)
        }
        var changed = layers != drawn
        drawn = layers
        if let region = context.region {
            let perPoint = Self.mapPointsPerPoint(zoom: region.zoom)
            if perPoint != mapPointsPerPoint {
                mapPointsPerPoint = perPoint
                changed = true
            }
        }
        if changed { redraw() }
    }

    /// Groups the marks of the layers drawn at the zoom shown and draws them and the areas again.
    private func redraw() {
        let layers = Set(drawn.keys)
        clusters = AccessMarks.clusters(
            AccessMarks.shown(marks, layers: layers), mapPointsPerPoint: mapPointsPerPoint)
        pinOverlay.set(clusters, opacity: drawn)
        pinRenderer?.setNeedsDisplay()
        // The areas keep their thinned paths while only the zoom changes.
        if shapesDrawn?.layers != drawn || shapesDrawn?.count != shapes.count {
            shapesDrawn = (drawn, shapes.count)
            shapeOverlay.set(shapes.filter { layers.contains($0.layer) }, opacity: drawn)
            shapeRenderer?.setNeedsDisplay()
        }
    }

    func regionDidSettle(_ context: MapModuleContext) {
        follow(context)
        #if DEBUG
            pressAtLaunch()
        #endif
    }

    #if DEBUG
        /// `-mapFindAccess` (debug builds, screenshots): the button is pressed once, when the map
        /// has first settled.
        private func pressAtLaunch() {
            guard !launchPressed, ProcessInfo.processInfo.arguments.contains("-mapFindAccess"),
                controller?.region != nil
            else { return }
            launchPressed = true
            find()
        }
    #endif

    // MARK: - Looking up

    /// "Find ramps and piers here": looks up the layers shown for the view on screen: public
    /// land and OpenStreetMap places when either of their layers is on, stocked waters when
    /// theirs is. With none of the group's layers on, it turns on public land and the access
    /// points and looks them up.
    func find() {
        guard !busy, let controller, let region = controller.region else { return }
        var shown = Set(controller.context(for: self).layers.map(\.id))
        if shown.isEmpty {
            for id in [AccessLayers.publicLand, AccessLayers.accessPoints]
            where layerIDs.contains(id) {
                controller.setShown(id, true)
                shown.insert(id)
            }
        }
        let wantsAccess = !shown.isDisjoint(with: AccessLayers.access)
        let wantsStocking = shown.contains(AccessLayers.stockingPoints)
        guard wantsAccess || wantsStocking else { return }
        let area = NearbyArea(
            west: region.west, south: region.south, east: region.east, north: region.north)
        let json = controller.settingsJSON
        busy = true
        zoomNotices = []
        notices = []
        lookup = Task { [weak self] in
            await self?.look(area: area, settingsJSON: json, access: wantsAccess, stocking: wantsStocking)
        }
    }

    private func look(area: NearbyArea, settingsJSON json: String, access: Bool, stocking: Bool)
        async
    {
        let host = self.host
        let clock = self.clock
        let state = self.state
        let send: NearbyLookup.Send = { request in try? await host.send(request: request) }
        let pause: NearbyLookup.Pause = { duration in try? await Task.sleep(for: duration) }
        if access {
            do {
                let (plan, base) = try state.plan { kept in
                    try planAccess(
                        area: area, includeParking: false, stateJson: kept, settingsJson: json,
                        now: clock())
                }
                let results = await NearbyLookup.run(
                    plan, send: send, pause: pause, clock: clock
                ) { [weak self] partial in
                    await self?.show(access: partial, base: base)
                }
                show(access: results, base: base)
            } catch NearbyError.ZoomIn(let message) {
                zoomNotices.append(message)
            } catch {
                // Unreadable settings or a view out of range: nothing to look up.
            }
        }
        if stocking {
            do {
                let plan = try planStocking(area: area, settingsJson: json, now: clock())
                let results = await NearbyLookup.run(
                    plan, send: send, pause: pause, clock: clock
                ) { [weak self] partial in
                    await self?.show(stocking: partial)
                }
                show(stocking: results)
            } catch NearbyError.ZoomIn(let message) {
                zoomNotices.append(message)
            } catch {
                // As above.
            }
        }
        busy = false
        rebuild()
    }

    /// The access results so far; their state is kept over what other lookups keep.
    private func show(access results: NearbyResults, base: String) {
        if let kept = results.stateJson { state.save(kept, base: base) }
        access = results
        rebuild()
    }

    private func show(stocking results: NearbyResults) {
        stocking = results
        rebuild()
    }

    /// The marks, areas, words and credits of the last results.
    private func rebuild() {
        let found = [access, stocking].compactMap { $0 }
        marks = AccessMarks.marks(
            access: access?.access ?? [], stocking: stocking?.stocking ?? [])
        shapes = (access?.access ?? []).compactMap(AccessShape2D.init)
        shapesDrawn = nil
        var words = zoomNotices
        var standing: [String] = []
        var sources: [NearbyCredit] = []
        for results in found {
            for notice in results.notices where !words.contains(notice) { words.append(notice) }
            for note in results.notes where !standing.contains(note) { standing.append(note) }
            for credit in results.credits where !sources.contains(where: { $0.source == credit.source }) {
                sources.append(credit)
            }
        }
        if notices != words { notices = words }
        if notes != standing { notes = standing }
        if credits != sources { credits = sources }
        redraw()
        controller?.moduleDidChange()
    }

    // MARK: - Taps and the card

    func handleTap(_ tap: MapTap) -> Bool {
        guard !drawn.isEmpty else { return false }
        if abs(log2(tap.mapPointsPerPoint / mapPointsPerPoint)) > 0.05 {
            // The map moved without settling yet: group at the zoom of the tap.
            mapPointsPerPoint = tap.mapPointsPerPoint
            redraw()
        }
        let at = MKMapPoint(tap.coordinate)
        if let hit = AccessMarks.hit(
            clusters, x: at.x, y: at.y, mapPointsPerPoint: mapPointsPerPoint)
        {
            select(hit, at: hit.coordinate)
            return true
        }
        // Public land is drawn as areas: a tap inside one opens it.
        guard drawn[AccessLayers.publicLand] != nil,
            let area = AccessThinning.area(
                at: at.x, at.y, in: shapes.filter { $0.layer == AccessLayers.publicLand }),
            let mark = marks.first(where: { $0.id == area.id })
        else { return false }
        select(AccessCluster(marks: [mark]), at: tap.coordinate)
        return true
    }

    /// Picks out a mark or an area: a ring goes round it (at `coordinate`) and, once the ring
    /// is on screen, its card comes up pointing at it.
    private func select(_ cluster: AccessCluster, at coordinate: CLLocationCoordinate2D) {
        card?.dismiss(animated: false)
        card = nil
        let chosen = AccessSelection(cluster: cluster, coordinate: coordinate)
        selection = chosen
        selectionMark = AccessSelectionAnnotation(chosen)
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
        let view = AccessCard(
            cluster: selection.cluster, notes: notes, credits: credits, layers: layers
        ) { [weak self] in self?.closeCard() }
        let made = UIHostingController(rootView: view)
        made.modalPresentationStyle = .popover
        made.preferredContentSize = CGSize(width: 380, height: 460)
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
        controller?.moduleDidChange()
    }

    func drawerSection() -> AnyView? {
        guard let controller else { return nil }
        return AnyView(AccessSection(module: self, settingsJSON: controller.settingsJSON))
    }
}

/// The mark picked out.
struct AccessSelection: Identifiable {
    let id = UUID()
    let cluster: AccessCluster
    /// Where its ring goes: the mark, or the tap inside an area.
    let coordinate: CLLocationCoordinate2D
}

/// Hears the card being swiped or tapped away.
private final class CardWatcher: NSObject, UIPopoverPresentationControllerDelegate {
    var dismissed: (@MainActor () -> Void)?

    func presentationControllerDidDismiss(_ presentationController: UIPresentationController) {
        dismissed?()
    }
}

/// The ring around the mark picked out.
final class AccessSelectionAnnotation: NSObject, MKAnnotation {
    let coordinate: CLLocationCoordinate2D
    let selection: UUID

    init(_ chosen: AccessSelection) {
        coordinate = chosen.coordinate
        selection = chosen.id
    }
}

/// A ring in the accent colour, a little larger than a mark; it takes no touches. Once on
/// screen it tells the module, which points the card at it.
final class AccessSelectionView: MKAnnotationView {
    var onWindow: (@MainActor (UIView) -> Void)?
    private let ring = CAShapeLayer()

    override init(annotation: (any MKAnnotation)?, reuseIdentifier: String?) {
        super.init(annotation: annotation, reuseIdentifier: reuseIdentifier)
        let side = 44.0
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
            (view: AccessSelectionView, _: UITraitCollection) in
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
