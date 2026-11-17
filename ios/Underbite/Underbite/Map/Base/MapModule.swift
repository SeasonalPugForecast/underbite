import MapKit
import SwiftUI
import UnderbiteKit

/// A part of the map that draws catalogue layers of its own (stations, spots, depth, access, ...)
/// on top of what the map view draws itself (base maps, tiles, rendered grids, arrows).
/// ``MapModules`` lists them; the ``MapController`` hands each the region and the layers on
/// screen, and ``MapCanvas`` puts what they return on the map.
///
/// Layer data is drawn in bulk: one overlay with its own renderer for many points (pins, with
/// their own hit-testing in ``handleTap(_:)``), one multi-polyline per contour level. Annotation
/// views are only for the few items picked out (the tapped pin, the active spot), never one per
/// item of a layer.
@MainActor
protocol MapModule: AnyObject {
    /// The catalogue ids of the layers this module draws.
    var layerIDs: Set<String> { get }

    /// The level its overlays sit at; above the labels unless the module says otherwise.
    var overlayLevel: MKOverlayLevel { get }

    /// The overlays to show now: the same objects as before stay on the map, new ones are added
    /// and missing ones removed. Called again after ``MapController/moduleDidChange()``, when the
    /// region settles and when the layers change.
    func overlays(in context: MapModuleContext) -> [any MKOverlay]

    /// The renderer of one of its overlays; none for an overlay it does not know.
    func renderer(for overlay: any MKOverlay) -> MKOverlayRenderer?

    /// The annotations to show now (the few items picked out), as for ``overlays(in:)``.
    func annotations(in context: MapModuleContext) -> [any MKAnnotation]

    /// The view of one of its annotations; none for an annotation it does not know.
    func annotationView(for annotation: any MKAnnotation, on map: MKMapView) -> MKAnnotationView?

    /// The map settled on a region (`context.region`): load what it needs, then call
    /// ``MapController/moduleDidChange()``.
    func regionDidSettle(_ context: MapModuleContext)

    /// A tap on the map; true when the module took it (a pin), so no other module and not the
    /// point inspector gets it. Modules are asked topmost first.
    func handleTap(_ tap: MapTap) -> Bool

    /// The module's own section in the layer drawer, if any.
    func drawerSection() -> AnyView?
}

extension MapModule {
    var overlayLevel: MKOverlayLevel { .aboveLabels }
    func overlays(in context: MapModuleContext) -> [any MKOverlay] { [] }
    func renderer(for overlay: any MKOverlay) -> MKOverlayRenderer? { nil }
    func annotations(in context: MapModuleContext) -> [any MKAnnotation] { [] }
    func annotationView(for annotation: any MKAnnotation, on map: MKMapView) -> MKAnnotationView? {
        nil
    }
    func regionDidSettle(_ context: MapModuleContext) {}
    func handleTap(_ tap: MapTap) -> Bool { false }
    func drawerSection() -> AnyView? { nil }
}

/// What a module gets to decide what to draw.
struct MapModuleContext {
    /// The region on screen, once the map has settled.
    let region: MapRegion?
    /// The module's layers on screen (shown, available, not held back), bottom first.
    let layers: [LayerInfo]
    /// The settings JSON the map draws with.
    let settingsJSON: String
    /// Whether the battery-friendly mode is on (no animation).
    let battery: Bool
}

/// A tap on the map.
struct MapTap: Equatable, Identifiable {
    let id = UUID()
    let coordinate: CLLocationCoordinate2D
    /// Where on the map view, in points.
    let point: CGPoint
    /// Map points per screen point at the zoom of the tap, for hit-testing at a size on screen.
    let mapPointsPerPoint: Double

    static func == (a: MapTap, b: MapTap) -> Bool { a.id == b.id }
}
