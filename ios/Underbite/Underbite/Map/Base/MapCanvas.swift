import MapKit
import SwiftUI
import UnderbiteKit

/// Apple's map view with what the ``MapController`` plans on it: the base map's configuration,
/// the layers the map draws itself in the settings' order (tiles and rendered grids as tile
/// overlays, arrows as one overlay per layer), each at its opacity, and the modules' overlays and
/// annotations above them. It reports the region once the map settles and taps the map itself
/// does not use (a double tap still zooms). With a ``MapLocator`` it shows the phone's position
/// (MapKit's own dot, drawn on this device) once location access is given, and follows it when
/// asked.
struct MapCanvas: UIViewRepresentable {
    let controller: MapController
    var locator: MapLocator?
    /// Told of the map view once, for controls that belong to it (the compass).
    var onMap: ((MKMapView) -> Void)?

    func makeCoordinator() -> Coordinator { Coordinator(controller: controller) }

    func makeUIView(context: Context) -> MKMapView {
        let map = MKMapView()
        map.delegate = context.coordinator
        // The compass is an MKCompassButton in the button column: the system one would sit
        // under it.
        map.showsCompass = false
        map.showsScale = true
        let start = controller.start
        map.setRegion(
            MKCoordinateRegion(
                center: CLLocationCoordinate2D(latitude: start.latitude, longitude: start.longitude),
                span: MKCoordinateSpan(latitudeDelta: start.span, longitudeDelta: start.span)),
            animated: false)
        let coordinator = context.coordinator
        // A single tap waits for a double tap to fail, so zooming in never drops a marker.
        let double = UITapGestureRecognizer(target: nil, action: nil)
        double.numberOfTapsRequired = 2
        double.delegate = coordinator
        let single = UITapGestureRecognizer(
            target: coordinator, action: #selector(Coordinator.tapped(_:)))
        single.delegate = coordinator
        single.require(toFail: double)
        // A long press drops the marker; a tap waits for it to fail. UIKit's allowable movement
        // (10 points) keeps a pan from counting as a press.
        let long = UILongPressGestureRecognizer(
            target: coordinator, action: #selector(Coordinator.pressed(_:)))
        long.minimumPressDuration = 0.5
        long.delegate = coordinator
        single.require(toFail: long)
        map.addGestureRecognizer(double)
        map.addGestureRecognizer(single)
        map.addGestureRecognizer(long)
        #if DEBUG
            // `-mapHeading <degrees>` turns the map for a screenshot.
            if let heading = MapLaunch.heading {
                map.setCamera(
                    MKMapCamera(
                        lookingAtCenter: CLLocationCoordinate2D(
                            latitude: start.latitude, longitude: start.longitude),
                        fromDistance: start.span * 111_000 * 2, pitch: 0, heading: heading),
                    animated: false)
            }
        #endif
        coordinator.map = map
        if let onMap {
            Task { @MainActor in onMap(map) }
        }
        return map
    }

    func updateUIView(_ map: MKMapView, context: Context) {
        // Read here so that the view updates when they change.
        let plan = controller.plan
        let revisions = controller.gridRevisions
        let moduleRevision = controller.moduleRevision
        context.coordinator.sync(
            map, plan: plan, revisions: revisions, moduleRevision: moduleRevision)
        context.coordinator.syncPosition(map, locator: locator, state: locator?.state)
    }

    final class Coordinator: NSObject, MKMapViewDelegate, UIGestureRecognizerDelegate {
        let controller: MapController
        weak var map: MKMapView?
        private var baseKey: String?
        /// The layers' overlays on the map by overlay key, bottom first in `order`.
        private var overlays: [String: any MKOverlay] = [:]
        private var order: [String] = []
        private var planned: [String: PlannedLayer] = [:]
        private var keys: [ObjectIdentifier: String] = [:]
        private var renderers: [ObjectIdentifier: MKOverlayRenderer] = [:]
        private var drawnRevisions: [String: Int] = [:]
        /// Grid overlays replaced by newer ones, on the map until those have their first tiles.
        private var retired = RetiredOverlays<GridTileOverlay>()
        /// One replacement per grid layer at a time; revisions in between are drawn together.
        private var gate = ReplacementGate()
        /// Per grid layer, the replacing overlay whose first tiles the gate waits for.
        private var replacing: [String: ObjectIdentifier] = [:]
        /// Per module, its overlays and annotations on the map.
        private var moduleOverlays: [ObjectIdentifier: [ObjectIdentifier: any MKOverlay]] = [:]
        private var moduleAnnotations: [ObjectIdentifier: [ObjectIdentifier: any MKAnnotation]] =
            [:]
        private var askedModules: (revision: Int, plan: MapPlan)?
        private weak var locator: MapLocator?
        #if DEBUG
            private var simulated: SimulatedPosition?
        #endif

        init(controller: MapController) {
            self.controller = controller
        }

        func sync(_ map: MKMapView, plan: MapPlan, revisions: [String: Int], moduleRevision: Int) {
            syncBase(map, plan.base)
            syncLayers(map, plan.layers)
            for layer in plan.layers {
                guard let overlay = overlays[layer.overlayKey] else { continue }
                let renderer = renderers[ObjectIdentifier(overlay)]
                renderer?.alpha = CGFloat(layer.opacity)
                let revision = revisions[layer.id] ?? 0
                if drawnRevisions[layer.overlayKey] != revision {
                    drawnRevisions[layer.overlayKey] = revision
                    if let grid = overlay as? GridTileOverlay {
                        // While a replacement draws, the newest revision waits for it.
                        if gate.request(layer.overlayKey) { replace(grid, with: layer, on: map) }
                    } else {
                        (renderer as? ArrowRenderer)?.setNeedsDisplay()
                    }
                }
            }
            if askedModules?.revision != moduleRevision || askedModules?.plan != plan {
                askedModules = (moduleRevision, plan)
                syncModules(map)
            }
        }

        // MARK: - The phone's position

        /// Shows the position while access is given and follows it while the locator asks.
        func syncPosition(_ map: MKMapView, locator: MapLocator?, state: LocateState?) {
            self.locator = locator
            guard let state else { return }
            #if DEBUG
                if let at = locator?.simulated {
                    showSimulated(at, following: state.following, on: map)
                    return
                }
            #endif
            if map.showsUserLocation != state.showsPosition {
                map.showsUserLocation = state.showsPosition
            }
            let mode: MKUserTrackingMode = state.showsPosition && state.following ? .follow : .none
            if map.userTrackingMode != mode {
                map.setUserTrackingMode(mode, animated: true)
            }
        }

        #if DEBUG
            /// The fixed position of `-fakeLocation`: a dot like MapKit's in its circle; following
            /// centres the map on it once.
            private func showSimulated(
                _ at: CLLocationCoordinate2D, following: Bool, on map: MKMapView
            ) {
                if simulated == nil {
                    let position = SimulatedPosition(at)
                    simulated = position
                    map.addOverlay(position.accuracy, level: .aboveLabels)
                    map.addAnnotation(position)
                }
                guard following else { return }
                map.setCenter(at, animated: true)
                // After this view update, not during it.
                let locator = self.locator
                Task { @MainActor in locator?.mapStoppedFollowing() }
            }
        #endif

        func mapView(_ mapView: MKMapView, didChange mode: MKUserTrackingMode, animated: Bool) {
            // Moving the map by hand stops following.
            if mode == .none { locator?.mapStoppedFollowing() }
        }

        // MARK: - Base map

        private func syncBase(_ map: MKMapView, _ base: LayerInfo?) {
            let key = base?.id ?? ""
            guard key != baseKey else { return }
            baseKey = key
            map.preferredConfiguration = Self.configuration(base?.draw)
        }

        /// Apple's map for a base: the standard map, imagery, or imagery with labels; under the
        /// NOAA chart, the muted standard map without points of interest (it shows only beyond
        /// the chart's coverage).
        static func configuration(_ draw: LayerDraw?) -> MKMapConfiguration {
            // Places anglers look for.
            let places = MKPointOfInterestFilter(including: [.marina, .beach, .park, .nationalPark])
            switch draw {
            case .appleSatellite:
                return MKImageryMapConfiguration(elevationStyle: .flat)
            case .appleHybrid:
                let c = MKHybridMapConfiguration(elevationStyle: .flat)
                c.pointOfInterestFilter = places
                return c
            case .tiles:
                let c = MKStandardMapConfiguration(elevationStyle: .flat, emphasisStyle: .muted)
                c.pointOfInterestFilter = .excludingAll
                return c
            default:
                let c = MKStandardMapConfiguration(elevationStyle: .flat, emphasisStyle: .default)
                c.pointOfInterestFilter = places
                return c
            }
        }

        // MARK: - The layers the map draws itself

        private func syncLayers(_ map: MKMapView, _ layers: [PlannedLayer]) {
            planned = Dictionary(
                layers.map { ($0.overlayKey, $0) }, uniquingKeysWith: { first, _ in first })
            let wanted = layers.map(\.overlayKey)
            guard wanted != order else { return }
            let steps = OverlayOrder.steps(current: order, wanted: wanted)
            for key in steps.remove {
                for old in retired.drop(key: key) {
                    take(old, off: map)
                }
                gate.drop(key)
                replacing[key] = nil
                guard let overlay = overlays.removeValue(forKey: key) else { continue }
                take(overlay, off: map)
                drawnRevisions[key] = nil
            }
            for (key, above) in steps.insert {
                guard let layer = planned[key] else { continue }
                let overlay = makeOverlay(layer)
                overlays[key] = overlay
                keys[ObjectIdentifier(overlay)] = key
                drawnRevisions[key] = controller.gridRevisions[layer.id] ?? 0
                if let above, let below = overlays[above] {
                    map.insertOverlay(overlay, above: below)
                } else {
                    // The bottom of its level: under the modules' overlays.
                    map.insertOverlay(
                        overlay, at: 0, level: layer.aboveLabels ? .aboveLabels : .aboveRoads)
                }
            }
            order = wanted
        }

        private func take(_ overlay: any MKOverlay, off map: MKMapView) {
            map.removeOverlay(overlay)
            keys[ObjectIdentifier(overlay)] = nil
            renderers[ObjectIdentifier(overlay)] = nil
        }

        /// How long a replaced grid overlay stays at most when the new one's tiles do not answer
        /// (none asked for, such as below the layer's shallowest zoom).
        static let retireAfter: Duration = .seconds(2)

        /// How long after a replacement settles an owed one starts, so that blocks landing in a
        /// burst are drawn a few at a time rather than one replacement each.
        static let replaceSpacing: Duration = .milliseconds(400)

        /// Draws a grid layer again with the blocks held and the range now drawn: a new overlay
        /// goes right above the old one, which comes off once the new one's first tiles are
        /// answered, so the layer does not blink. (MapKit asks a tile overlay for each tile once:
        /// reloading its renderer drops the tiles it has and does not ask for them again.)
        private func replace(_ old: GridTileOverlay, with layer: PlannedLayer, on map: MKMapView) {
            let key = layer.overlayKey
            let fresh = GridTileOverlay(
                layer, store: controller.gridStore, range: controller.drawnRanges[layer.id])
            overlays[key] = fresh
            keys[ObjectIdentifier(fresh)] = key
            retired.retire(old, key: key)
            map.insertOverlay(fresh, above: old)
            let id = ObjectIdentifier(fresh)
            replacing[key] = id
            fresh.firstTiles.onAnswered { [weak self] in
                Task { @MainActor in self?.settled(id, key: key) }
            }
            Task { @MainActor [weak self] in
                try? await Task.sleep(for: Self.retireAfter)
                self?.settled(id, key: key)
            }
        }

        /// The grid overlay `id` drew its first tiles (or waited long enough): the overlays it
        /// replaced come off the map.
        private func settled(_ id: ObjectIdentifier, key: String) {
            guard let map else { return }
            let current = overlays[key] as? GridTileOverlay
            for old in retired.settled(id, key: key, current: current) {
                take(old, off: map)
            }
            // The first word from the replacement the gate waits for (its tiles, or the timer).
            guard replacing[key] == id else { return }
            replacing[key] = nil
            guard gate.settled(key) else { return }
            Task { @MainActor [weak self] in
                try? await Task.sleep(for: Self.replaceSpacing)
                self?.replaceOwed(key)
            }
        }

        /// Draws the revisions that came while the last replacement of `key` was drawing.
        private func replaceOwed(_ key: String) {
            guard let map, let layer = planned[key], let grid = overlays[key] as? GridTileOverlay
            else {
                gate.drop(key)
                return
            }
            replace(grid, with: layer, on: map)
        }

        private func makeOverlay(_ layer: PlannedLayer) -> any MKOverlay {
            switch layer.kind {
            case .tiles:
                CoreTileOverlay(layer: layer.info, date: layer.date, loader: controller.tileLoader)
            case .gridRaster:
                GridTileOverlay(
                    layer, store: controller.gridStore, range: controller.drawnRanges[layer.id])
            case .gridArrows:
                controller.arrowOverlay(for: layer.id)
            }
        }

        // MARK: - Modules

        /// The lowest overlay on the map, at `level`, of the modules listed after `module`.
        private func firstOverlay(above module: any MapModule, at level: MKOverlayLevel)
            -> (any MKOverlay)?
        {
            let modules = controller.modules
            guard let index = modules.firstIndex(where: { $0 === module }) else { return nil }
            for later in modules[(index + 1)...] where later.overlayLevel == level {
                let held = moduleOverlays[ObjectIdentifier(later)] ?? [:]
                if let overlay = held.values.first { return overlay }
            }
            return nil
        }

        private func syncModules(_ map: MKMapView) {
            for module in controller.modules {
                let id = ObjectIdentifier(module)
                let context = controller.context(for: module)
                var held = moduleOverlays[id] ?? [:]
                let wanted = module.overlays(in: context)
                let wantedIDs = Set(wanted.map { ObjectIdentifier($0) })
                for (key, overlay) in held where !wantedIDs.contains(key) {
                    map.removeOverlay(overlay)
                    held[key] = nil
                }
                for overlay in wanted where held[ObjectIdentifier(overlay)] == nil {
                    // Modules are listed bottom first: an overlay that comes late (a forecast
                    // that has loaded) goes under those of the modules above it, so the pins
                    // stay on top.
                    if let above = firstOverlay(above: module, at: module.overlayLevel) {
                        map.insertOverlay(overlay, below: above)
                    } else {
                        map.addOverlay(overlay, level: module.overlayLevel)
                    }
                    held[ObjectIdentifier(overlay)] = overlay
                }
                moduleOverlays[id] = held

                var pinned = moduleAnnotations[id] ?? [:]
                let shown = module.annotations(in: context)
                let shownIDs = Set(shown.map { ObjectIdentifier($0) })
                for (key, annotation) in pinned where !shownIDs.contains(key) {
                    map.removeAnnotation(annotation)
                    pinned[key] = nil
                }
                for annotation in shown where pinned[ObjectIdentifier(annotation)] == nil {
                    map.addAnnotation(annotation)
                    pinned[ObjectIdentifier(annotation)] = annotation
                }
                moduleAnnotations[id] = pinned
            }
        }

        // MARK: - MKMapViewDelegate

        func mapView(_ mapView: MKMapView, rendererFor overlay: any MKOverlay) -> MKOverlayRenderer {
            if let key = keys[ObjectIdentifier(overlay)], let layer = planned[key] {
                let renderer: MKOverlayRenderer =
                    if let arrows = overlay as? ArrowOverlay {
                        ArrowRenderer(arrows)
                    } else if let tiles = overlay as? MKTileOverlay {
                        MKTileOverlayRenderer(tileOverlay: tiles)
                    } else {
                        MKOverlayRenderer(overlay: overlay)
                    }
                renderer.alpha = CGFloat(layer.opacity)
                renderers[ObjectIdentifier(overlay)] = renderer
                return renderer
            }
            #if DEBUG
                if let circle = overlay as? MKCircle, circle === simulated?.accuracy {
                    return SimulatedPosition.renderer(for: circle)
                }
            #endif
            for module in controller.modules {
                if let renderer = module.renderer(for: overlay) { return renderer }
            }
            return MKOverlayRenderer(overlay: overlay)
        }

        func mapView(_ mapView: MKMapView, viewFor annotation: any MKAnnotation) -> MKAnnotationView? {
            // MapKit's own view for the phone's position.
            if annotation is MKUserLocation { return nil }
            #if DEBUG
                if annotation is SimulatedPosition {
                    return SimulatedPositionView(
                        annotation: annotation, reuseIdentifier: SimulatedPosition.reuse)
                }
            #endif
            for module in controller.modules {
                if let view = module.annotationView(for: annotation, on: mapView) { return view }
            }
            return nil
        }

        func mapViewDidFinishLoadingMap(_ mapView: MKMapView) {
            MapLoad.shared.mark(.baseMap)
        }

        func mapViewDidFinishRenderingMap(_ mapView: MKMapView, fullyRendered: Bool) {
            if fullyRendered { MapLoad.shared.mark(.baseRendered) }
        }

        func mapView(_ mapView: MKMapView, regionDidChangeAnimated animated: Bool) {
            // Before the first layout the map has no size and no zoom yet.
            guard mapView.bounds.width > 0 else { return }
            controller.regionDidSettle(Self.region(of: mapView))
        }

        /// The region a map view shows, clipped to one world.
        static func region(of map: MKMapView) -> MapRegion {
            let rect = map.visibleMapRect
            let northWest = MKMapPoint(x: rect.minX, y: rect.minY).coordinate
            let southEast = MKMapPoint(x: rect.maxX, y: rect.maxY).coordinate
            var west = northWest.longitude, east = southEast.longitude
            if rect.width >= MKMapSize.world.width || east < west {
                (west, east) = (-180, 180)
            }
            return MapRegion(
                west: west, south: max(southEast.latitude, -90), east: east,
                north: min(northWest.latitude, 90),
                zoom: MapRegion.zoom(viewPoints: Double(map.bounds.width), mapPoints: rect.width))
        }

        // MARK: - Taps

        @objc func tapped(_ recognizer: UITapGestureRecognizer) {
            guard recognizer.state == .ended, let map, map.bounds.width > 0 else { return }
            let point = recognizer.location(in: map)
            controller.didTap(
                MapTap(
                    coordinate: map.convert(point, toCoordinateFrom: map), point: point,
                    mapPointsPerPoint: map.visibleMapRect.width / Double(map.bounds.width)))
        }

        @objc func pressed(_ recognizer: UILongPressGestureRecognizer) {
            guard recognizer.state == .began, let map, map.bounds.width > 0 else { return }
            let point = recognizer.location(in: map)
            let press = MapTap(
                coordinate: map.convert(point, toCoordinateFrom: map), point: point,
                mapPointsPerPoint: map.visibleMapRect.width / Double(map.bounds.width))
            controller.modules.lazy.compactMap { $0 as? InspectorModule }.first?.longPress(press)
        }

        func gestureRecognizer(
            _ gestureRecognizer: UIGestureRecognizer,
            shouldRecognizeSimultaneouslyWith other: UIGestureRecognizer
        ) -> Bool {
            true
        }
    }
}
