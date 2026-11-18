import MapKit
import SwiftUI
@preconcurrency import UnderbiteKit

/// The Storms layer: the forecast models' low-pressure tracks as translucent bands (the low that
/// brings the storm near the centre wider, each model in its colour and dash), and NHC's active
/// hurricanes with their official, guidance and observed tracks and NHC's forecast cone, labelled
/// as NHC's cone and never as an impact area. Everything is drawn by one overlay with one
/// renderer (``StormOverlay``); the lows are marked with their pressure at the time step the
/// drawer's slider picks.
///
/// The outlook is the core's for the centre of the view (`fetch_storm_layer`, through the app's
/// fetch host): asked when the layer is on and the map settles, and the core asks the sources only
/// when the centre moved more than one grid cell or a newer run may be out, so panning costs
/// nothing. On opening, the kept outlook is drawn at once (`kept_storm_layer`); a change of units
/// words it again. Hurricanes are absent when none are active. A tapped hurricane opens its card
/// (a popover on the iPad, a sheet on the iPhone) with NHC's advisory link.
///
/// Every value and word is the core's, including the colours; the safety text is whole and in the
/// plain voice.
@MainActor
@Observable
final class StormsModule: MapModule {
    /// What the layer draws and says; none until an outlook is there.
    private(set) var layer: StormLayer?
    /// The words before an outlook is there.
    private(set) var words: StormLayerWords?
    /// The outlook is being asked for.
    private(set) var loading = false
    /// The last ask failed outright (no outlook could be read).
    private(set) var failed = false
    /// The time step whose lows are marked.
    private(set) var step = 0
    /// The layer as the catalogue words it, while it is on.
    private(set) var info: LayerInfo?

    @ObservationIgnored private weak var controller: MapController?
    @ObservationIgnored private let host: FetchHost
    @ObservationIgnored private let clock: @Sendable () -> Int64
    @ObservationIgnored private let overlay = StormOverlay()
    @ObservationIgnored private var renderer: StormRenderer?
    @ObservationIgnored private var opacity = 1.0
    @ObservationIgnored private var region: MapRegion?
    @ObservationIgnored private var givenJSON = ""
    @ObservationIgnored private var fetchTask: Task<Void, Never>?
    /// Each ask is numbered; what an older one brings never replaces a newer one's.
    @ObservationIgnored private var asks = 0
    @ObservationIgnored private var selection: StormHurricane?
    @ObservationIgnored private var selectionMark: StormSelectionAnnotation?
    @ObservationIgnored private var card: UIViewController?
    @ObservationIgnored private let cardWatcher = StormCardWatcher()
    /// The hurricane to frame alone the first time the layer draws it.
    @ObservationIgnored private var focus: String?
    @ObservationIgnored private var frameMark: StormFrameAnnotation?
    @ObservationIgnored private let sample: StormsLaunch.Sample?

    /// How long the map stays quiet before the outlook is asked for again.
    static let settle: Duration = .milliseconds(600)

    init(
        controller: MapController,
        host: FetchHost = URLSessionHost(storeURL: AppFiles.standard.hostDirectory),
        clock: @escaping @Sendable () -> Int64 = { Int64(Date().timeIntervalSince1970) }
    ) {
        self.controller = controller
        self.host = host
        self.clock = clock
        sample = StormsLaunch.sample
        focus = StormsLaunch.focus
        step = StormsLaunch.step ?? 0
        cardWatcher.dismissed = { [weak self] in self?.cardClosed() }
        watch()
    }

    /// The catalogue's storms layer.
    var layerIDs: Set<String> {
        Set((controller?.catalogue ?? []).filter { $0.draw == .storms }.map(\.id))
    }

    /// Frames `id` alone the next time the layer draws it (a hurricane opened from elsewhere).
    func focus(on id: String) {
        focus = id
        frameIfFocused()
    }

    // MARK: - Drawing

    func overlays(in context: MapModuleContext) -> [any MKOverlay] {
        let shown = context.layers.first { $0.draw == .storms }
        // Asked while the map view updates: what follows from a change waits for it to end.
        if shown?.id != info?.id || Double(shown?.opacity ?? 1) != opacity {
            Task { @MainActor [weak self] in self?.follow(shown) }
        }
        guard info != nil, layer != nil else { return [] }
        return [overlay]
    }

    func renderer(for overlay: any MKOverlay) -> MKOverlayRenderer? {
        guard overlay === self.overlay else { return nil }
        let made = StormRenderer(self.overlay)
        made.alpha = CGFloat(opacity)
        renderer = made
        return made
    }

    func annotations(in context: MapModuleContext) -> [any MKAnnotation] {
        guard info != nil else { return [] }
        return [selectionMark, frameMark].compactMap { $0 }
    }

    func annotationView(for annotation: any MKAnnotation, on map: MKMapView) -> MKAnnotationView? {
        if let mark = annotation as? StormSelectionAnnotation {
            let view = StormSelectionView(annotation: mark, reuseIdentifier: nil)
            let id = mark.selection
            view.onWindow = { [weak self] anchor in self?.showCard(for: id, from: anchor) }
            return view
        }
        if let mark = annotation as? StormFrameAnnotation {
            let view = StormFrameView(annotation: mark, reuseIdentifier: nil)
            let rect = mark.rect
            view.onWindow = { [weak self, weak map] in
                guard let map else { return }
                self?.frame(rect, on: map)
            }
            return view
        }
        return nil
    }

    func regionDidSettle(_ context: MapModuleContext) {
        if let region = context.region { self.region = region }
        guard info != nil else { return }
        frameIfFocused()
        scheduleFetch()
    }

    func handleTap(_ tap: MapTap) -> Bool {
        guard info != nil, let layer, !layer.hurricanes.isEmpty else { return false }
        guard
            let hurricane = StormMapRules.hurricane(
                at: MKMapPoint(tap.coordinate), in: layer.hurricanes,
                mapPointsPerPoint: tap.mapPointsPerPoint)
        else { return false }
        select(hurricane)
        return true
    }

    func drawerSection() -> AnyView? {
        guard info != nil else { return nil }
        return AnyView(StormsSection(module: self))
    }

    /// Marks the lows at step `index`.
    func setStep(_ index: Int) {
        guard let layer else { return }
        let next = StormMapRules.startStep(index, count: layer.steps.count)
        guard next != step else { return }
        step = next
        redraw()
    }

    // MARK: - Following the map

    /// Hears the settings change: the layer is worded again in the new units.
    private func watch() {
        guard let controller else { return }
        withObservationTracking {
            _ = controller.settingsJSON
        } onChange: { [weak self] in
            Task { @MainActor [weak self] in
                self?.settingsChanged()
                self?.watch()
            }
        }
    }

    private func settingsChanged() {
        guard let controller, controller.settingsJSON != givenJSON, info != nil else { return }
        givenJSON = controller.settingsJSON
        words = try? stormLayerWords(settingsJson: givenJSON)
        drawKept()
    }

    /// The layer on screen, or none: switched on, it draws what is kept and asks for the
    /// outlook; switched off, it stops asking and closes its card.
    private func follow(_ shown: LayerInfo?) {
        let wasOn = info != nil
        info = shown
        opacity = Double(shown?.opacity ?? 1)
        renderer?.alpha = CGFloat(opacity)
        guard let controller else { return }
        if shown == nil {
            fetchTask?.cancel()
            loading = false
            closeCard()
            if wasOn { controller.moduleDidChange() }
            return
        }
        guard !wasOn else { return }
        givenJSON = controller.settingsJSON
        words = try? stormLayerWords(settingsJson: givenJSON)
        if region == nil { region = controller.region }
        drawKept()
        scheduleFetch()
    }

    /// Draws the kept outlook (or the launch's sample) without asking anything.
    private func drawKept() {
        let json = givenJSON
        let zone = TimeZone.current.identifier
        if let sample {
            let made = try? stormLayerOf(
                outlookJson: sample.outlookJSON, settingsJson: json, timeZone: sample.timeZone,
                now: sample.now)
            show(made)
            return
        }
        if let kept = try? keptStormLayer(
            settingsJson: json, timeZone: zone, now: clock(), host: host)
        {
            show(kept)
        }
    }

    private func scheduleFetch() {
        fetchTask?.cancel()
        guard sample == nil, info != nil, region != nil else { return }
        fetchTask = Task { [weak self] in
            try? await Task.sleep(for: Self.settle)
            guard !Task.isCancelled else { return }
            await self?.fetch()
        }
    }

    /// Asks the core for the layer around the view's centre; it asks the sources only when its
    /// outlook is due.
    private func fetch() async {
        guard let region, info != nil else { return }
        asks += 1
        let ask = asks
        loading = layer == nil
        let (latitude, longitude) = (
            region.centerLatitude, StormMapRules.longitude(region.centerLongitude, near: 0)
        )
        do {
            let outcome = try await fetchStormLayer(
                latitude: latitude, longitude: longitude, settingsJson: givenJSON,
                timeZone: TimeZone.current.identifier, allowMetered: true, now: clock(),
                host: host)
            guard ask == asks, info != nil else { return }
            failed = false
            loading = false
            show(outcome.layer)
        } catch {
            guard ask == asks else { return }
            loading = false
            failed = layer == nil
        }
    }

    /// Shows `next`, keeping the step within its steps, and draws it again if it changed.
    private func show(_ next: StormLayer?) {
        guard let next, next != layer else { return }
        let first = layer == nil
        layer = next
        step = StormMapRules.startStep(step, count: next.steps.count)
        if let selection, !next.hurricanes.contains(where: { $0.id == selection.id }) {
            closeCard()
        }
        redraw()
        if first { controller?.moduleDidChange() }
        frameIfFocused()
    }

    private func redraw() {
        guard let layer else { return }
        overlay.set(StormDrawing.make(layer, step: step))
        renderer?.setNeedsDisplay()
    }

    // MARK: - Framing a hurricane

    /// Frames the focused hurricane once it is drawn: an invisible mark is put in the middle of
    /// the view (MapKit makes views only for marks in view), and once on screen it hands over the
    /// map view to frame.
    private func frameIfFocused() {
        guard let focus, let layer, info != nil, let region = region ?? controller?.region,
            let hurricane = layer.hurricanes.first(where: { $0.id == focus })
        else { return }
        self.focus = nil
        frameMark = StormFrameAnnotation(
            coordinate: CLLocationCoordinate2D(
                latitude: region.centerLatitude, longitude: region.centerLongitude),
            rect: StormMapRules.frame(hurricane))
        controller?.moduleDidChange()
    }

    private func frame(_ rect: MKMapRect, on map: MKMapView) {
        // At least about 800 km across, and a little room around the cone.
        let minimum = MKMapPointsPerMeterAtLatitude(rect.origin.coordinate.latitude) * 800_000
        let grown = rect.insetBy(
            dx: -max(0, minimum - rect.width) / 2 - rect.width * 0.08,
            dy: -max(0, minimum - rect.height) / 2 - rect.height * 0.08)
        let inset = 48.0
        map.setVisibleMapRect(
            grown, edgePadding: UIEdgeInsets(top: inset, left: inset, bottom: inset, right: inset),
            animated: false)
        frameMark = nil
        controller?.moduleDidChange()
    }

    // MARK: - The card

    private func select(_ hurricane: StormHurricane) {
        closeCard()
        selection = hurricane
        selectionMark = StormSelectionAnnotation(hurricane)
        controller?.moduleDidChange()
    }

    private func showCard(for id: String, from anchor: UIView) {
        guard let selection, selection.id == id, let layer, card == nil,
            let window = anchor.window, var top = window.rootViewController
        else { return }
        while let next = top.presentedViewController, !next.isBeingDismissed { top = next }
        let view = StormCard(hurricane: selection, layer: layer) { [weak self] in
            self?.closeCard()
        }
        let made = UIHostingController(rootView: view)
        made.modalPresentationStyle = .popover
        made.preferredContentSize = CGSize(width: 400, height: 520)
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

    private func closeCard() {
        if let card {
            card.dismiss(animated: true) { [weak self] in self?.cardClosed() }
        } else {
            cardClosed()
        }
    }

    private func cardClosed() {
        card = nil
        selection = nil
        guard selectionMark != nil else { return }
        selectionMark = nil
        controller?.moduleDidChange()
    }
}

/// Hears the card being swiped or tapped away.
private final class StormCardWatcher: NSObject, UIPopoverPresentationControllerDelegate {
    var dismissed: (@MainActor () -> Void)?

    func presentationControllerDidDismiss(_ presentationController: UIPresentationController) {
        dismissed?()
    }
}

/// The ring around the hurricane picked out, which the card points at.
final class StormSelectionAnnotation: NSObject, MKAnnotation {
    let coordinate: CLLocationCoordinate2D
    /// NHC's id of the hurricane.
    let selection: String

    init(_ hurricane: StormHurricane) {
        coordinate = CLLocationCoordinate2D(
            latitude: hurricane.latitude, longitude: hurricane.longitude)
        selection = hurricane.id
    }
}

/// A ring in the accent colour around a hurricane's symbol; it takes no touches. Once on screen it
/// tells the module, which points the card at it.
final class StormSelectionView: MKAnnotationView {
    var onWindow: (@MainActor (UIView) -> Void)?
    private let ring = CAShapeLayer()

    override init(annotation: (any MKAnnotation)?, reuseIdentifier: String?) {
        super.init(annotation: annotation, reuseIdentifier: reuseIdentifier)
        let side = StormMapRules.symbolSize + 14
        frame = CGRect(x: 0, y: 0, width: side, height: side)
        ring.path = CGPath(
            ellipseIn: CGRect(x: 2, y: 2, width: side - 4, height: side - 4), transform: nil)
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
            (view: StormSelectionView, _: UITraitCollection) in
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

/// An invisible mark that hands the map view over once on screen, to frame a hurricane.
final class StormFrameAnnotation: NSObject, MKAnnotation {
    let coordinate: CLLocationCoordinate2D
    let rect: MKMapRect

    init(coordinate: CLLocationCoordinate2D, rect: MKMapRect) {
        self.coordinate = coordinate
        self.rect = rect
    }
}

final class StormFrameView: MKAnnotationView {
    var onWindow: (@MainActor () -> Void)?

    override init(annotation: (any MKAnnotation)?, reuseIdentifier: String?) {
        super.init(annotation: annotation, reuseIdentifier: reuseIdentifier)
        frame = CGRect(x: 0, y: 0, width: 1, height: 1)
        isUserInteractionEnabled = false
        isAccessibilityElement = false
        displayPriority = .required
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { nil }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        guard window != nil, let onWindow else { return }
        self.onWindow = nil
        Task { @MainActor in onWindow() }
    }
}
