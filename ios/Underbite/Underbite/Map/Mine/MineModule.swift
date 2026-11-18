import MapKit
import SwiftUI
@preconcurrency import UnderbiteKit

/// Draws the user's own data in the "Mine" group: the logged catches as marks (grouped when too
/// close to tell apart), the imported tracks as lines and, when asked for, a heat map of the
/// catches. The saved spots of the same group are the stations module's.
///
/// Everything here is on the device and stays there: catches at their exact positions from the
/// catch log, tracks from the imported tracks, and the heat map binned by the core from the log
/// (`catch_cells`); nothing is fetched and nothing is shared. Each layer is one overlay with its
/// own renderer (the heat map under the tracks under the catches); the only annotation view is
/// the ring around the mark tapped, whose catches open in a sheet (a popover on the iPad).
@MainActor
final class MineModule: MapModule {
    private weak var controller: MapController?
    private let catches: CatchStore
    private let tracks: TrackStore

    private let heat = HeatOverlay()
    private let lines = TrackOverlay()
    private let pins = CatchPinOverlay()
    private var heatRenderer: HeatRenderer?
    private var trackRenderer: TrackRenderer?
    private var pinRenderer: CatchPinRenderer?

    /// The module's layers on screen, by id.
    private var shown: [String: LayerInfo] = [:]
    private var region: MapRegion?
    private var mapPointsPerPoint = CatchMarks.mapPointsPerPoint(zoom: 3)

    /// The catches at their positions, and the log's revision they were read at.
    private var points: [CatchPoint] = []
    private var pointsRevision: Int?
    private var pointsTask: Task<Void, Never>?
    private var marks: [CatchMark] = []
    /// The tracks' revision the lines were made at.
    private var linesRevision: Int?
    /// What the heat map was last binned for.
    private var heatKey: HeatKey?
    private var heatTask: Task<Void, Never>?

    /// The mark tapped and its ring, while its sheet is up.
    private var selection: (mark: CatchMark, ring: CatchSelectionAnnotation)?
    /// The ring whose sheet is still to be presented, once the map has its view.
    private var presentPending = false
    private weak var presented: UIViewController?
    private var launchCatchShown = false

    init(controller: MapController, catches: CatchStore = .shared, tracks: TrackStore = .shared) {
        self.controller = controller
        self.catches = catches
        self.tracks = tracks
        #if DEBUG
            if ProcessInfo.processInfo.arguments.contains("-seedTracks") {
                MineSeeds.seed(into: tracks, settingsJSON: "{}")
            }
        #endif
        observeStores()
    }

    var layerIDs: Set<String> {
        Set((controller?.catalogue ?? []).filter { $0.draw == .mine }.map(\.id))
    }

    // MARK: - Drawing

    func overlays(in context: MapModuleContext) -> [any MKOverlay] {
        follow(context)
        // All three while any is on screen, so they keep their order as layers come and go.
        return shown.isEmpty ? [] : [heat, lines, pins]
    }

    func renderer(for overlay: any MKOverlay) -> MKOverlayRenderer? {
        let made: MKOverlayRenderer
        switch overlay {
        case let overlay as HeatOverlay where overlay === heat:
            let renderer = HeatRenderer(overlay)
            heatRenderer = renderer
            made = renderer
        case let overlay as TrackOverlay where overlay === lines:
            let renderer = TrackRenderer(overlay)
            trackRenderer = renderer
            made = renderer
        case let overlay as CatchPinOverlay where overlay === pins:
            let renderer = CatchPinRenderer(overlay)
            pinRenderer = renderer
            made = renderer
        default:
            return nil
        }
        applyOpacities()
        return made
    }

    func annotations(in context: MapModuleContext) -> [any MKAnnotation] {
        selection.map { [$0.ring] } ?? []
    }

    func annotationView(for annotation: any MKAnnotation, on map: MKMapView) -> MKAnnotationView? {
        guard let ring = annotation as? CatchSelectionAnnotation else { return nil }
        let view =
            map.dequeueReusableAnnotationView(withIdentifier: CatchSelectionAnnotation.reuse)
            as? PinSelectionView
            ?? PinSelectionView(annotation: ring, reuseIdentifier: CatchSelectionAnnotation.reuse)
        view.annotation = ring
        if presentPending, let selection, selection.ring === ring {
            presentPending = false
            // After MapKit has placed the view: the sheet points at the mark.
            Task { @MainActor [weak self, weak map] in
                guard let self, let map else { return }
                self.present(selection.mark, ring: ring, on: map)
            }
        }
        return view
    }

    func regionDidSettle(_ context: MapModuleContext) {
        follow(context)
    }

    /// Follows the layers on screen, their opacities and the zoom: what is drawn is brought up
    /// to date and drawn again.
    private func follow(_ context: MapModuleContext) {
        let layers = Dictionary(
            context.layers.map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })
        let changed = Set(layers.keys) != Set(shown.keys)
        shown = layers
        if let settled = context.region { region = settled }
        var regroup = changed
        if let region {
            let perPoint = CatchMarks.mapPointsPerPoint(zoom: region.zoom)
            if perPoint != mapPointsPerPoint {
                mapPointsPerPoint = perPoint
                regroup = true
            }
        }
        applyOpacities()
        refresh(regroup: regroup)
    }

    /// Brings each layer's drawing up to date with the stores: reads the catches when the log
    /// changed, makes the lines when the tracks changed, bins the heat map for the zoom.
    private func refresh(regroup: Bool = false) {
        if shown[CatchMarks.layer] != nil {
            if pointsRevision != catches.revision {
                loadPoints()
            } else if regroup {
                self.regroup()
            }
        } else if !marks.isEmpty {
            self.regroup()
        }

        if shown[TrackLines.layer] != nil {
            if linesRevision != tracks.revision {
                linesRevision = tracks.revision
                lines.set(TrackLines.lines(tracks.tracks))
                trackRenderer?.setNeedsDisplay()
            }
        } else if linesRevision != nil {
            linesRevision = nil
            lines.set([])
            trackRenderer?.setNeedsDisplay()
        }

        if shown[CatchHeat.layer] != nil, let region {
            let key = HeatKey(
                revision: catches.revision,
                cellMetres: CatchHeat.cellMetres(zoom: region.zoom, latitude: region.centerLatitude))
            if key != heatKey { binHeat(key) }
        } else if heatKey != nil {
            heatTask?.cancel()
            heatKey = nil
            heat.set([])
            heatRenderer?.setNeedsDisplay()
        }
    }

    /// Each layer's opacity, from the settings.
    private func applyOpacities() {
        func set(_ renderer: MKOverlayRenderer?, _ id: String) {
            guard let renderer, let layer = shown[id] else { return }
            let alpha = CGFloat(layer.opacity)
            if renderer.alpha != alpha {
                renderer.alpha = alpha
                renderer.setNeedsDisplay()
            }
        }
        set(heatRenderer, CatchHeat.layer)
        set(trackRenderer, TrackLines.layer)
        set(pinRenderer, CatchMarks.layer)
    }

    /// Groups the catches at the zoom shown and draws them again; a mark tapped whose catches
    /// are gone is let go.
    private func regroup() {
        let drawn = shown[CatchMarks.layer] != nil ? points : []
        marks = CatchMarks.clusters(drawn, mapPointsPerPoint: mapPointsPerPoint)
        pins.set(marks)
        pinRenderer?.setNeedsDisplay()
        if let selection {
            let ids = Set(drawn.map(\.id))
            if !selection.mark.catches.allSatisfy({ ids.contains($0.id) }) {
                // Not while the map is asking what to draw: once it is done.
                let ring = selection.ring
                Task { @MainActor [weak self] in
                    guard let self, self.selection?.ring === ring else { return }
                    self.select(nil)
                }
            }
        }
    }

    // MARK: - Loading

    private func observeStores() {
        withObservationTracking {
            _ = catches.revision
            _ = tracks.revision
        } onChange: { [weak self] in
            Task { @MainActor [weak self] in
                self?.refresh()
                self?.observeStores()
            }
        }
    }

    /// Reads where the catches are, off the main actor.
    private func loadPoints() {
        let revision = catches.revision
        pointsRevision = revision
        let book = catches.book
        pointsTask?.cancel()
        pointsTask = Task { [weak self] in
            let found = await Self.points(book)
            guard let self, !Task.isCancelled, self.catches.revision == revision else { return }
            self.points = found
            self.regroup()
            #if DEBUG
                self.selectLaunchCatch()
            #endif
        }
    }

    #if DEBUG
        /// `-mapCatch <id>` (debug builds, screenshots): once the catches are read, the mark of
        /// the catch `id` is picked out as if tapped, once.
        private func selectLaunchCatch() {
            guard !launchCatchShown, let id = LaunchArguments.value(after: "-mapCatch"),
                let mark = marks.first(where: { $0.catches.contains { $0.id == id } })
            else { return }
            launchCatchShown = true
            select(mark)
        }
    #endif

    @concurrent
    private nonisolated static func points(_ book: Data) async -> [CatchPoint] {
        CatchMarks.load(book: book)
    }

    /// Bins the catches for the heat map, off the main actor.
    private func binHeat(_ key: HeatKey) {
        heatKey = key
        let book = catches.book
        heatTask?.cancel()
        heatTask = Task { [weak self] in
            let squares = await Self.squares(book, cellMetres: key.cellMetres)
            guard let self, !Task.isCancelled, self.heatKey == key else { return }
            self.heat.set(squares)
            self.heatRenderer?.setNeedsDisplay()
        }
    }

    @concurrent
    private nonisolated static func squares(_ book: Data, cellMetres: Double) async -> [HeatSquare] {
        CatchHeat.squares(CatchHeat.cells(book: book, cellMetres: cellMetres), cellMetres: cellMetres)
    }

    // MARK: - Taps

    func handleTap(_ tap: MapTap) -> Bool {
        guard shown[CatchMarks.layer] != nil, !marks.isEmpty else { return false }
        if abs(log2(tap.mapPointsPerPoint / mapPointsPerPoint)) > 0.05 {
            // The map moved without settling yet: group at the zoom of the tap.
            mapPointsPerPoint = tap.mapPointsPerPoint
            regroup()
        }
        let at = MKMapPoint(tap.coordinate)
        guard
            let hit = CatchMarks.hit(
                marks, at: MapXY(x: at.x, y: at.y), mapPointsPerPoint: mapPointsPerPoint)
        else { return false }
        select(hit)
        return true
    }

    /// Picks out a mark (its sheet comes up once its ring is on the map), or none (its sheet goes).
    private func select(_ mark: CatchMark?) {
        presented?.dismiss(animated: true)
        presented = nil
        if let mark {
            selection = (mark, CatchSelectionAnnotation(mark))
            presentPending = true
        } else {
            selection = nil
            presentPending = false
        }
        controller?.moduleDidChange()
    }

    private func present(_ mark: CatchMark, ring: CatchSelectionAnnotation, on map: MKMapView) {
        guard selection?.ring === ring else { return }
        let title = shown[CatchMarks.layer]?.title ?? ""
        presented = MineCatchPresenter.present(
            ids: mark.catches.map(\.id), title: title, at: mark.coordinate, on: map,
            catches: catches
        ) { [weak self, weak ring] in
            guard let self, let ring, self.selection?.ring === ring else { return }
            self.presented = nil
            self.select(nil)
        }
        if presented == nil { select(nil) }
    }
}

/// What the heat map was binned for: the log's revision and the squares' size.
private struct HeatKey: Equatable {
    let revision: Int
    let cellMetres: Double
}

/// The ring around the catch mark tapped.
final class CatchSelectionAnnotation: NSObject, MKAnnotation {
    static let reuse = "catch-selection"
    let coordinate: CLLocationCoordinate2D

    init(_ mark: CatchMark) {
        coordinate = mark.coordinate
    }
}
