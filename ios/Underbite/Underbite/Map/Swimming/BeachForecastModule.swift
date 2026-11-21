import MapKit
import SwiftUI
@preconcurrency import UnderbiteKit

/// The beach forecast layer of the "Water" group: the shore zones of the National Weather
/// Service's marine beach forecast in the view, filled by the rip current risk the forecast office
/// issued (a zone with nothing issued is outlined, never drawn as a low risk). Nothing is asked
/// when the layer is switched on or the map moves: the drawer's "Show beach forecasts here" looks
/// the zones up for the view on screen through the core's `plan_beach_forecast` and the app's
/// fetch host, and is the only way the services are asked. What the lookup said (zoom in, no
/// zones, a failure) shows under the button with the key of the colours, the layer's words and
/// the credit.
///
/// The zones are one overlay with one renderer. A tap inside a zone picks it out (its edge is
/// heavier) and opens its card, a popover on the iPad and a sheet on the iPhone. Every word and
/// colour name is the core's: Swift draws and lays out.
@MainActor
@Observable
final class BeachForecastModule: MapModule {
    /// A lookup is running.
    private(set) var busy = false
    /// What the last lookup said: the core's notices, and its "zoom in" words.
    private(set) var notices: [String] = []
    /// The standing notes to show with the zones.
    private(set) var notes: [String] = []
    /// The sources of the zones shown.
    private(set) var credits: [NearbyCredit] = []
    /// What the colours stand for, in words.
    private(set) var legend: [BeachLegendItem] = []

    @ObservationIgnored private weak var controller: MapController?
    @ObservationIgnored private let host: FetchHost
    @ObservationIgnored private let clock: @Sendable () -> Int64
    @ObservationIgnored private let state: NearbyStateStore
    @ObservationIgnored private let overlay = BeachOverlay()
    @ObservationIgnored private var renderer: BeachRenderer?
    @ObservationIgnored private var pins: [BeachZonePin] = []
    @ObservationIgnored private var shapes: [BeachShape] = []
    /// The layer's opacity while it is drawn (shown and at a zoom it is asked at).
    @ObservationIgnored private var drawn: Double?
    @ObservationIgnored private var lookup: Task<Void, Never>?
    @ObservationIgnored private var selection: (zone: String, mark: BeachSelectionAnnotation)?
    @ObservationIgnored private var presentPending = false
    @ObservationIgnored private weak var presented: UIViewController?
    @ObservationIgnored private var launchPressed = false
    @ObservationIgnored private var launchOpen = BeachLaunch.open

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
    }

    var layerIDs: Set<String> {
        Set((controller?.catalogue ?? []).map(\.id).filter { $0 == BeachZones.layer })
    }

    /// Whether the lookup may be asked at the zoom on screen.
    var canLookUp: Bool {
        guard let controller, let region = controller.region,
            let layer = controller.catalogue.first(where: { $0.id == BeachZones.layer })
        else { return false }
        return AccessMarks.drawn(minZoom: layer.minZoom, at: region.zoom)
    }

    // MARK: - Drawing

    func overlays(in context: MapModuleContext) -> [any MKOverlay] {
        follow(context)
        return drawn == nil ? [] : [overlay]
    }

    func renderer(for overlay: any MKOverlay) -> MKOverlayRenderer? {
        guard overlay === self.overlay else { return nil }
        let made = BeachRenderer(self.overlay)
        made.alpha = CGFloat(drawn ?? 1)
        renderer = made
        return made
    }

    func annotations(in context: MapModuleContext) -> [any MKAnnotation] {
        selection.map { [$0.mark] } ?? []
    }

    func annotationView(for annotation: any MKAnnotation, on map: MKMapView) -> MKAnnotationView? {
        guard let mark = annotation as? BeachSelectionAnnotation else { return nil }
        let view =
            map.dequeueReusableAnnotationView(withIdentifier: BeachSelectionAnnotation.identifier)
            ?? AreaPointView(
                annotation: mark, reuseIdentifier: BeachSelectionAnnotation.identifier)
        view.annotation = mark
        if presentPending, let selection, selection.mark === mark {
            presentPending = false
            // After MapKit has placed the view: the sheet points at it.
            Task { @MainActor [weak self, weak map] in
                guard let self, let map else { return }
                self.present(mark, on: map)
            }
        }
        return view
    }

    func regionDidSettle(_ context: MapModuleContext) {
        follow(context)
        #if DEBUG
            pressAtLaunch()
        #endif
    }

    /// Follows the layer on screen, its opacity and the zoom.
    private func follow(_ context: MapModuleContext) {
        var now: Double?
        if let layer = context.layers.first(where: { $0.id == BeachZones.layer }),
            context.region.map({ AccessMarks.drawn(minZoom: layer.minZoom, at: $0.zoom) }) ?? true
        {
            now = Double(layer.opacity)
        }
        guard now != drawn else { return }
        drawn = now
        if let now, let renderer, renderer.alpha != CGFloat(now) {
            renderer.alpha = CGFloat(now)
            renderer.setNeedsDisplay()
        }
    }

    #if DEBUG
        /// `-mapFindBeaches` (debug builds, screenshots): the button is pressed once, when the
        /// map has first settled with the layer on screen.
        private func pressAtLaunch() {
            guard !launchPressed, BeachLaunch.find, drawn != nil, controller?.region != nil
            else { return }
            launchPressed = true
            // Not while the map is asking what to draw: once it is done.
            Task { @MainActor [weak self] in self?.find() }
        }
    #endif

    // MARK: - Looking up

    /// "Show beach forecasts here": looks the zones up for the view on screen. With the layer
    /// off, it turns it on first.
    func find() {
        guard !busy, let controller, let region = controller.region else { return }
        if drawn == nil, layerIDs.contains(BeachZones.layer) {
            controller.setShown(BeachZones.layer, true)
        }
        let area = RecordsLookup.area(region)
        let json = controller.settingsJSON
        let sample = BeachLaunch.sample
        let clock: @Sendable () -> Int64
        if let sample {
            let at = sample.now
            clock = { at }
        } else {
            clock = self.clock
        }
        let made: (plan: NearbyPlan, base: String)
        do {
            if sample != nil {
                // A hand-made forecast leaves the kept state alone.
                made = (
                    try planBeachForecast(
                        area: area, stateJson: "", settingsJson: json, now: clock()), ""
                )
            } else {
                made = try state.plan { kept in
                    try planBeachForecast(
                        area: area, stateJson: kept, settingsJson: json, now: clock())
                }
            }
        } catch NearbyError.ZoomIn(let message) {
            notices = [message]
            return
        } catch {
            return
        }
        busy = true
        notices = []
        let host = self.host
        let send: RecordsLookup.Send = { request in
            if let sample { return sample.answer(request) }
            return try await host.send(request: request)
        }
        lookup = Task { [weak self] in
            let results = await RecordsLookup.run(made.plan, send: send, clock: clock)
            guard let self else { return }
            if sample == nil, let kept = results.stateJson { self.state.save(kept, base: made.base) }
            self.busy = false
            self.show(results)
        }
    }

    /// The zones, words and credits of the last results.
    private func show(_ results: NearbyResults) {
        pins = results.beachZones
        shapes = BeachZones.shapes(results.beachZones)
        notices = results.notices
        notes = results.notes
        credits = results.credits
        legend = results.beachLegend
        if let selection, !shapes.contains(where: { $0.id == selection.zone }) { select(nil) }
        overlay.set(shapes, picked: selection?.zone)
        renderer?.setNeedsDisplay()
        controller?.moduleDidChange()
        openAtLaunch()
    }

    /// `-mapBeachOpen` (debug builds, screenshots): the first zone is picked out as if tapped.
    private func openAtLaunch() {
        guard launchOpen, let pin = pins.first else { return }
        launchOpen = false
        select(pin.id, at: CLLocationCoordinate2D(latitude: pin.lat, longitude: pin.lon))
    }

    // MARK: - Taps and the card

    func handleTap(_ tap: MapTap) -> Bool {
        guard drawn != nil, !shapes.isEmpty else { return false }
        let at = MKMapPoint(tap.coordinate)
        guard let zone = BeachZones.hit(shapes, x: at.x, y: at.y) else { return false }
        select(zone.id, at: tap.coordinate)
        return true
    }

    /// Picks a zone out (its card comes up once the point it points at is on the map), or none.
    private func select(_ zone: String?, at coordinate: CLLocationCoordinate2D? = nil) {
        presented?.dismiss(animated: true)
        presented = nil
        if let zone, let coordinate {
            selection = (zone, BeachSelectionAnnotation(coordinate: coordinate, zone: zone))
            presentPending = true
        } else {
            selection = nil
            presentPending = false
        }
        overlay.set(shapes, picked: selection?.zone)
        renderer?.setNeedsDisplay()
        controller?.moduleDidChange()
    }

    private func present(_ mark: BeachSelectionAnnotation, on map: MKMapView) {
        guard selection?.mark === mark, let zone = pins.first(where: { $0.id == mark.zone }) else {
            return
        }
        presented = BeachPresenter.present(
            zone, credits: credits, at: mark.coordinate, on: map
        ) { [weak self, weak mark] in
            guard let self, let mark, self.selection?.mark === mark else { return }
            self.presented = nil
            self.select(nil)
        }
        if presented == nil { select(nil) }
    }
}

/// Where the card of the zone picked out points: the point tapped.
final class BeachSelectionAnnotation: NSObject, MKAnnotation {
    static let identifier = "beach-selection"
    let coordinate: CLLocationCoordinate2D
    let zone: String

    init(coordinate: CLLocationCoordinate2D, zone: String) {
        self.coordinate = coordinate
        self.zone = zone
    }
}

/// Presents a zone's card pointing at a point of the map: a popover where the width is regular
/// (the iPad), a sheet at medium height otherwise.
@MainActor
enum BeachPresenter {
    static let popoverSize = CGSize(width: 380, height: 520)

    @discardableResult
    static func present(
        _ zone: BeachZonePin, credits: [NearbyCredit], at coordinate: CLLocationCoordinate2D,
        on map: MKMapView, closed: @escaping () -> Void
    ) -> UIViewController? {
        guard var top = map.window?.rootViewController else { return nil }
        while let next = top.presentedViewController, !next.isBeingDismissed { top = next }
        weak var presented: UIViewController?
        let card = BeachCard(
            zone: zone, credits: credits.filter { $0.source == "nws-beach-forecast" },
            done: { presented?.dismiss(animated: true) }
        )
        // Gone by a swipe, a tap outside the popover or "Done": the zone is let go.
        .onDisappear(perform: closed)
        let hosting = UIHostingController(rootView: card)
        presented = hosting
        hosting.modalPresentationStyle = .popover
        hosting.preferredContentSize = popoverSize
        if let popover = hosting.popoverPresentationController {
            popover.sourceView = map
            let at = map.convert(coordinate, toPointTo: map)
            popover.sourceRect = CGRect(x: at.x - 22, y: at.y - 22, width: 44, height: 44)
            let adapted = popover.adaptiveSheetPresentationController
            adapted.detents = [.medium(), .large()]
            adapted.prefersGrabberVisible = true
        }
        top.present(hosting, animated: true)
        return hosting
    }
}

extension BeachForecastModule {
    func drawerSection() -> AnyView? {
        guard let controller else { return nil }
        return AnyView(BeachSection(module: self, controller: controller))
    }
}
