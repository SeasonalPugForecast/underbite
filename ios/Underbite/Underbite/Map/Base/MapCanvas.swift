import MapKit
import SwiftUI
import UnderbiteKit

/// Apple's map view with what the ``MapController`` plans on it: the base map's configuration,
/// the layers the map draws itself in the settings' order (tiles and rendered grids as tile
/// overlays, arrows as one overlay per layer), each at its opacity, and the modules' overlays and
/// annotations above them. It reports the region once the map settles and taps the map itself
/// does not use (a double tap still zooms).
struct MapCanvas: UIViewRepresentable {
    let controller: MapController

    func makeCoordinator() -> Coordinator { Coordinator(controller: controller) }

    func makeUIView(context: Context) -> MKMapView {
        let map = MKMapView()
        map.delegate = context.coordinator
        map.showsCompass = true
        map.showsScale = true
        let start = controller.start
        map.setRegion(
            MKCoordinateRegion(
                center: CLLocationCoordinate2D(latitude: start.latitude, longitude: start.longitude),
                span: MKCoordinateSpan(latitudeDelta: start.span, longitudeDelta: start.span)),
            animated: false)
        let coordinator = context.coordinator
        // A single tap waits for a double tap to fail, so zooming in never opens the inspector.
        let double = UITapGestureRecognizer(target: nil, action: nil)
        double.numberOfTapsRequired = 2
        double.delegate = coordinator
        let single = UITapGestureRecognizer(
            target: coordinator, action: #selector(Coordinator.tapped(_:)))
        single.delegate = coordinator
        single.require(toFail: double)
        map.addGestureRecognizer(double)
        map.addGestureRecognizer(single)
        coordinator.map = map
        return map
    }

    func updateUIView(_ map: MKMapView, context: Context) {
        // Read here so that the view updates when they change.
        let plan = controller.plan
        let revisions = controller.gridRevisions
        let moduleRevision = controller.moduleRevision
        context.coordinator.sync(
            map, plan: plan, revisions: revisions, moduleRevision: moduleRevision)
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
        /// Per module, its overlays and annotations on the map.
        private var moduleOverlays: [ObjectIdentifier: [ObjectIdentifier: any MKOverlay]] = [:]
        private var moduleAnnotations: [ObjectIdentifier: [ObjectIdentifier: any MKAnnotation]] =
            [:]
        private var askedModules: (revision: Int, plan: MapPlan)?

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
                    (overlay as? GridTileOverlay)?.setRange(controller.drawnRanges[layer.id])
                    (renderer as? MKTileOverlayRenderer)?.reloadData()
                    (renderer as? ArrowRenderer)?.setNeedsDisplay()
                }
            }
            if askedModules?.revision != moduleRevision || askedModules?.plan != plan {
                askedModules = (moduleRevision, plan)
                syncModules(map)
            }
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
                guard let overlay = overlays.removeValue(forKey: key) else { continue }
                map.removeOverlay(overlay)
                keys[ObjectIdentifier(overlay)] = nil
                renderers[ObjectIdentifier(overlay)] = nil
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
                    map.addOverlay(overlay, level: module.overlayLevel)
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
            for module in controller.modules {
                if let renderer = module.renderer(for: overlay) { return renderer }
            }
            return MKOverlayRenderer(overlay: overlay)
        }

        func mapView(_ mapView: MKMapView, viewFor annotation: any MKAnnotation) -> MKAnnotationView? {
            for module in controller.modules {
                if let view = module.annotationView(for: annotation, on: mapView) { return view }
            }
            return nil
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

        func gestureRecognizer(
            _ gestureRecognizer: UIGestureRecognizer,
            shouldRecognizeSimultaneouslyWith other: UIGestureRecognizer
        ) -> Bool {
            true
        }
    }
}
