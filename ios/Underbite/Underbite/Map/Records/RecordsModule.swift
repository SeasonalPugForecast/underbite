import MapKit
import SwiftUI
@preconcurrency import UnderbiteKit

/// Draws the species records and the beach bloom forecasts of the "Water" group, looked up only
/// when the user presses a button in the module's drawer section: nothing is asked when a layer
/// is switched on or the map moves.
///
/// - **Species records** (`species-records`): one page of GBIF and iNaturalist records of the
///   species chosen in the view, and the USGS Nonindigenous Aquatic Species records of the state
///   the species pages name (`speciesRegion`, kept to the view by the core). A record whose
///   source blurred its position is drawn as a circle of the stated radius, never as a point;
///   a nonindigenous record is an orange diamond. Pressing again in the same view asks the next
///   page while the core says there is one. The lookup state the core keeps (iNaturalist's pace
///   and daily count, the services' pauses) is kept in ``RecordsStateFile``.
/// - **Beach bloom forecasts** (`hab-beaches`): the beaches of the Gulf respiratory irritation
///   forecast in the view, coloured by level, with NOAA NCCOS's credit and the plain safety note.
///
/// Each layer is one overlay with its own renderer; the only annotation view is the ring around
/// the mark tapped (or an empty point at the tap on a circle), whose records open in a sheet (a
/// popover on the iPad) with each record's licence, page and credit, and a photo only when the
/// core gives one (openly licensed), loaded through the app's fetch host. No observer is named.
@MainActor
@Observable
final class RecordsModule: MapModule {
    /// The species to look up (catalogue id), kept on this device.
    private(set) var speciesID: String?
    private(set) var findingRecords = false
    private(set) var findingBeaches = false
    /// What the last records lookup said ("Zoom in to look for species records", a pause).
    private(set) var recordsNotices: [String] = []
    /// The standing notes of the records shown (what records are, the USGS disclaimer).
    private(set) var recordsNotes: [String] = []
    /// The sources of the records shown, for the key under the button.
    private(set) var recordSources: [RecordSource] = []
    /// What the last beach lookup said (the Gulf-only notice, no beaches in the view).
    private(set) var beachNotices: [String] = []

    @ObservationIgnored private weak var controller: MapController?
    @ObservationIgnored let host: URLSessionHost
    @ObservationIgnored private let stateFile: RecordsStateFile
    @ObservationIgnored private let defaults: UserDefaults
    @ObservationIgnored private let clock: @Sendable () -> Int64

    @ObservationIgnored private let recordsOverlay = RecordsOverlay()
    @ObservationIgnored private let beachesOverlay = BeachesOverlay()
    @ObservationIgnored private var recordsRenderer: RecordsRenderer?
    @ObservationIgnored private var beachesRenderer: BeachesRenderer?

    /// The module's layers on screen, by id.
    @ObservationIgnored private var shown: [String: LayerInfo] = [:]
    @ObservationIgnored private var region: MapRegion?
    @ObservationIgnored private var mapPointsPerPoint = RecordMarks.mapPointsPerPoint(zoom: 3)

    @ObservationIgnored private var recordPins: [RecordPin] = []
    @ObservationIgnored private var recordByID: [String: RecordPin] = [:]
    @ObservationIgnored private var beachPins: [BloomPin] = []
    @ObservationIgnored private var beachByID: [String: BloomPin] = [:]
    @ObservationIgnored private var beachCredit: NearbyCredit?
    @ObservationIgnored private var beachNotes: [String] = []
    @ObservationIgnored private var recordMarks: [RecordMark] = []
    @ObservationIgnored private var beachMarks: [RecordMark] = []
    @ObservationIgnored private var areas: [RecordArea] = []
    /// The last records lookup: what was asked, and whether the core has a next page.
    @ObservationIgnored private var lastRecords: LastRecords?
    @ObservationIgnored private var recordsTask: Task<Void, Never>?
    @ObservationIgnored private var beachesTask: Task<Void, Never>?

    /// What is picked out and its ring, while its sheet is up.
    @ObservationIgnored private var selection:
        (picked: RecordPick, ring: RecordSelectionAnnotation)?
    @ObservationIgnored private var presentPending = false
    @ObservationIgnored private weak var presented: UIViewController?
    /// What a screenshot launch asks for (debug builds only).
    @ObservationIgnored private var launch = RecordsLaunch()

    /// Where the species chosen is kept.
    static let speciesKey = "mapRecordsSpecies"
    /// The region the species pages keep (ISO 3166-2, `US-NY`): its state is the one the USGS
    /// Nonindigenous Aquatic Species database is asked about.
    static let regionKey = "speciesRegion"

    init(
        controller: MapController,
        host: URLSessionHost = URLSessionHost(storeURL: AppFiles.standard.hostDirectory),
        stateFile: RecordsStateFile = RecordsStateFile(),
        defaults: UserDefaults = .standard,
        clock: @escaping @Sendable () -> Int64 = { Int64(Date().timeIntervalSince1970) }
    ) {
        self.controller = controller
        self.host = host
        self.stateFile = stateFile
        self.defaults = defaults
        self.clock = clock
        speciesID = launch.species ?? defaults.string(forKey: Self.speciesKey)
    }

    var layerIDs: Set<String> {
        let ids: Set<String> = [RecordMarks.recordsLayer, RecordMarks.beachesLayer]
        return Set((controller?.catalogue ?? []).map(\.id).filter { ids.contains($0) })
    }

    // MARK: - The drawer

    func drawerSection() -> AnyView? {
        guard let controller else { return nil }
        return AnyView(RecordsDrawerSection(module: self, controller: controller))
    }

    /// Chooses the species to look up; the records of another species are taken off the map.
    func setSpecies(_ id: String?) {
        guard id != speciesID else { return }
        speciesID = id
        defaults.set(id, forKey: Self.speciesKey)
        recordsTask?.cancel()
        recordsTask = nil
        findingRecords = false
        lastRecords = nil
        setRecords([], notices: [], notes: [])
    }

    /// The catalogue's species with the user's edits, in the settings' words.
    func speciesRows() -> [SpeciesListRow] {
        SpeciesCatalogue.rows(
            edits: SpeciesEditsStore.shared.edits, settingsJSON: controller?.settingsJSON ?? "{}")
    }

    /// Whether a layer is drawn, and so may be looked up, at the zoom on screen.
    func canLookUp(_ id: String) -> Bool {
        guard let region = controller?.region,
            let layer = controller?.catalogue.first(where: { $0.id == id })
        else { return false }
        return RecordMarks.drawn(minZoom: layer.minZoom, zoom: region.zoom)
    }

    /// Looks up the chosen species' records in the view: the first page, or the next one when
    /// the last lookup was of the same species in the same view and the core has more.
    func findRecords() {
        guard !findingRecords, let controller, let region = controller.region,
            let id = speciesID, let species = speciesRows().first(where: { $0.id == id })
        else { return }
        let area = RecordsLookup.area(region)
        var page: UInt32 = 1
        if let last = lastRecords, last.species == id, last.area == area, last.more {
            page = last.page + 1
        }
        let query = RecordsLookup.query(
            scientificName: species.scientificName, page: page,
            region: defaults.string(forKey: Self.regionKey) ?? "")
        let plan: NearbyPlan
        do {
            plan = try planRecords(
                area: area, query: query, stateJson: stateFile.read(),
                settingsJson: controller.settingsJSON, now: clock())
        } catch NearbyError.ZoomIn(let message) {
            recordsNotices = [message]
            return
        } catch {
            return
        }
        findingRecords = true
        let host = host
        let clock = clock
        recordsTask = Task { [weak self] in
            let results = await RecordsLookup.run(
                plan, send: { try await host.send(request: $0) }, clock: clock)
            guard let self else { return }
            // What was sent counts against the daily cap even when the answer is not wanted.
            self.stateFile.write(results.stateJson)
            guard !Task.isCancelled, self.speciesID == id else { return }
            self.findingRecords = false
            self.lastRecords = LastRecords(species: id, area: area, page: page, more: results.more)
            let pins =
                page == 1 ? results.records : RecordMarks.merge(self.recordPins, results.records)
            self.setRecords(pins, notices: results.notices, notes: results.notes)
            self.openLaunchRecord()
        }
    }

    /// Looks up the forecast beaches in the view.
    func findBeaches() {
        guard !findingBeaches, let controller, let region = controller.region else { return }
        let plan: NearbyPlan
        do {
            plan = try planBlooms(
                area: RecordsLookup.area(region), settingsJson: controller.settingsJSON,
                now: clock())
        } catch NearbyError.ZoomIn(let message) {
            beachNotices = [message]
            return
        } catch {
            return
        }
        findingBeaches = true
        let host = host
        let clock = clock
        beachesTask = Task { [weak self] in
            let results = await RecordsLookup.run(
                plan, send: { try await host.send(request: $0) }, clock: clock)
            guard let self, !Task.isCancelled else { return }
            self.findingBeaches = false
            self.beachPins = results.blooms
            self.beachByID = Dictionary(
                results.blooms.map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })
            self.beachCredit = results.credits.first { $0.source == "noaa-hab" }
            self.beachNotes = results.notes
            self.beachNotices = results.notices
            self.redraw()
        }
    }

    private func setRecords(_ pins: [RecordPin], notices: [String], notes: [String]) {
        recordPins = pins
        recordByID = Dictionary(pins.map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })
        recordsNotices = notices
        recordsNotes = notes
        var sources: [RecordSource] = []
        for pin in pins where !sources.contains(where: { $0.id == pin.source }) {
            sources.append(
                RecordSource(
                    id: pin.source, name: pin.sourceName,
                    kind: pin.source == RecordMarks.invasiveSource ? .invasive : .record))
        }
        recordSources = sources
        redraw()
    }

    // MARK: - Drawing

    func overlays(in context: MapModuleContext) -> [any MKOverlay] {
        follow(context)
        // Both while either is on screen, so they keep their order as layers come and go.
        return shown.isEmpty ? [] : [beachesOverlay, recordsOverlay]
    }

    func renderer(for overlay: any MKOverlay) -> MKOverlayRenderer? {
        let made: MKOverlayRenderer
        switch overlay {
        case let overlay as RecordsOverlay where overlay === recordsOverlay:
            let renderer = RecordsRenderer(overlay)
            recordsRenderer = renderer
            made = renderer
        case let overlay as BeachesOverlay where overlay === beachesOverlay:
            let renderer = BeachesRenderer(overlay)
            beachesRenderer = renderer
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
        guard let ring = annotation as? RecordSelectionAnnotation else { return nil }
        let view: MKAnnotationView
        if ring.onMark {
            view =
                map.dequeueReusableAnnotationView(withIdentifier: RecordSelectionAnnotation.ring)
                as? PinSelectionView
                ?? PinSelectionView(
                    annotation: ring, reuseIdentifier: RecordSelectionAnnotation.ring)
        } else {
            view =
                map.dequeueReusableAnnotationView(withIdentifier: RecordSelectionAnnotation.point)
                ?? AreaPointView(annotation: ring, reuseIdentifier: RecordSelectionAnnotation.point)
        }
        view.annotation = ring
        if presentPending, let selection, selection.ring === ring {
            presentPending = false
            // After MapKit has placed the view: the sheet points at it.
            Task { @MainActor [weak self, weak map] in
                guard let self, let map else { return }
                self.present(selection.picked, ring: ring, on: map)
            }
        }
        return view
    }

    func regionDidSettle(_ context: MapModuleContext) {
        follow(context)
    }

    /// Follows the layers on screen, their opacities and the zoom.
    private func follow(_ context: MapModuleContext) {
        let layers = Dictionary(
            context.layers.map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })
        var changed = Set(layers.keys) != Set(shown.keys)
        shown = layers
        if let settled = context.region {
            if settled.zoom != region?.zoom { changed = true }
            region = settled
            mapPointsPerPoint = RecordMarks.mapPointsPerPoint(zoom: settled.zoom)
        }
        applyOpacities()
        if changed { redraw() }
        if launch.wantsFind, shown[RecordMarks.recordsLayer] != nil, region != nil {
            launch.wantsFind = false
            // Not while the map is asking what to draw: once it is done.
            Task { @MainActor [weak self] in self?.findRecords() }
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
        set(recordsRenderer, RecordMarks.recordsLayer)
        set(beachesRenderer, RecordMarks.beachesLayer)
    }

    /// Whether the layer `id` is on screen and drawn at the zoom shown.
    private func drawing(_ id: String) -> Bool {
        guard let layer = shown[id], let region else { return false }
        return RecordMarks.drawn(minZoom: layer.minZoom, zoom: region.zoom)
    }

    /// Splits and groups what was found at the zoom shown and draws it again; a pick whose
    /// items are no longer drawn is let go.
    private func redraw() {
        if drawing(RecordMarks.recordsLayer) {
            let split = RecordMarks.split(recordPins)
            areas = split.areas
            recordMarks = RecordMarks.clusters(split.points, mapPointsPerPoint: mapPointsPerPoint)
        } else {
            areas = []
            recordMarks = []
        }
        beachMarks =
            drawing(RecordMarks.beachesLayer)
            ? RecordMarks.clusters(
                RecordMarks.beaches(beachPins), mapPointsPerPoint: mapPointsPerPoint) : []
        recordsOverlay.set(areas: areas, marks: recordMarks, picked: pickedAreas)
        beachesOverlay.set(beachMarks)
        recordsRenderer?.setNeedsDisplay()
        beachesRenderer?.setNeedsDisplay()
        if let selection, !stillDrawn(selection.picked) {
            let ring = selection.ring
            Task { @MainActor [weak self] in
                guard let self, self.selection?.ring === ring else { return }
                self.select(nil)
            }
        }
    }

    private var pickedAreas: Set<String> {
        guard case .areas(let ids, _)? = selection?.picked else { return [] }
        return Set(ids)
    }

    private func stillDrawn(_ picked: RecordPick) -> Bool {
        switch picked {
        case .mark(let mark):
            let drawn = Set((recordMarks + beachMarks).flatMap(\.items).map(\.id))
            return mark.items.allSatisfy { drawn.contains($0.id) }
        case .areas(let ids, _):
            let drawn = Set(areas.map(\.id))
            return ids.allSatisfy { drawn.contains($0) }
        }
    }

    // MARK: - Taps

    func handleTap(_ tap: MapTap) -> Bool {
        guard !recordMarks.isEmpty || !beachMarks.isEmpty || !areas.isEmpty else { return false }
        if abs(log2(tap.mapPointsPerPoint / mapPointsPerPoint)) > 0.05 {
            // The map moved without settling yet: group at the zoom of the tap.
            mapPointsPerPoint = tap.mapPointsPerPoint
            redraw()
        }
        let point = MKMapPoint(tap.coordinate)
        let at = MapXY(x: point.x, y: point.y)
        if let mark = RecordMarks.hit(recordMarks, at: at, mapPointsPerPoint: mapPointsPerPoint)
            ?? RecordMarks.hit(beachMarks, at: at, mapPointsPerPoint: mapPointsPerPoint)
        {
            select(.mark(mark))
            return true
        }
        let hit = RecordMarks.hitAreas(areas, at: at, mapPointsPerPoint: mapPointsPerPoint)
        guard !hit.isEmpty else { return false }
        select(.areas(hit.map(\.id), at: tap.coordinate))
        return true
    }

    /// Picks something out (its sheet comes up once its ring is on the map), or nothing (its
    /// sheet goes).
    private func select(_ picked: RecordPick?) {
        presented?.dismiss(animated: true)
        presented = nil
        if let picked {
            selection = (picked, RecordSelectionAnnotation(picked))
            presentPending = true
        } else {
            selection = nil
            presentPending = false
        }
        recordsOverlay.set(areas: areas, marks: recordMarks, picked: pickedAreas)
        recordsRenderer?.setNeedsDisplay()
        controller?.moduleDidChange()
    }

    private func present(_ picked: RecordPick, ring: RecordSelectionAnnotation, on map: MKMapView) {
        guard selection?.ring === ring else { return }
        let content: RecordsSheetContent
        let layer: String
        switch picked {
        case .mark(let mark) where mark.kind == .beach:
            layer = RecordMarks.beachesLayer
            content = .beaches(
                mark.items.compactMap { beachByID[$0.id] }, credit: beachCredit, notes: beachNotes)
        case .mark(let mark):
            layer = RecordMarks.recordsLayer
            content = .records(mark.items.compactMap { recordByID[$0.id] }, notes: recordsNotes)
        case .areas(let ids, _):
            layer = RecordMarks.recordsLayer
            content = .records(ids.compactMap { recordByID[$0] }, notes: recordsNotes)
        }
        presented = RecordsPresenter.present(
            content, title: shown[layer]?.title ?? "", at: ring.coordinate, on: map, host: host
        ) { [weak self, weak ring] in
            guard let self, let ring, self.selection?.ring === ring else { return }
            self.presented = nil
            self.select(nil)
        }
        if presented == nil { select(nil) }
    }

    /// `-mapRecordsOpen` (debug builds, screenshots): once the records are found, the first
    /// iNaturalist record drawn as a point with a photo, alone on its mark if one is, is picked
    /// out as if tapped (else the first point, else the smallest circle), once.
    private func openLaunchRecord() {
        guard launch.wantsOpen else { return }
        launch.wantsOpen = false
        // A record alone on its mark first, so its sheet shows the record in full.
        let alone = recordMarks.filter { $0.items.count == 1 }.flatMap(\.items)
        let points = (alone + recordMarks.flatMap(\.items)).compactMap { recordByID[$0.id] }
        let chosen = points.first { $0.source == "inaturalist" && $0.photo != nil } ?? points.first
        if let chosen,
            let mark = recordMarks.first(where: { $0.items.contains { $0.id == chosen.id } })
        {
            select(.mark(mark))
        } else if let area = areas.last {
            select(.areas([area.id], at: area.coordinate))
        }
    }
}

/// A source of the records shown, for the drawer's key.
struct RecordSource: Identifiable, Equatable {
    let id: String
    let name: String
    let kind: RecordItem.Kind
}

/// What the last records lookup asked, and whether the core has a next page.
private struct LastRecords {
    let species: String
    let area: NearbyArea
    let page: UInt32
    let more: Bool
}

/// What a tap picked out: a mark, or the circles of blurred records it fell in.
enum RecordPick {
    case mark(RecordMark)
    case areas([String], at: CLLocationCoordinate2D)
}

/// Where the sheet of what was picked out points: the mark's ring, or the point tapped on a
/// circle.
final class RecordSelectionAnnotation: NSObject, MKAnnotation {
    static let ring = "record-selection"
    static let point = "record-area-point"
    let coordinate: CLLocationCoordinate2D
    /// A mark's ring, rather than a point on a circle.
    let onMark: Bool

    init(_ picked: RecordPick) {
        switch picked {
        case .mark(let mark):
            coordinate = mark.coordinate
            onMark = true
        case .areas(_, let at):
            coordinate = at
            onMark = false
        }
    }
}

/// An empty point on a circle tapped: the sheet points at it, and the circle itself shows the
/// pick. It takes no touches.
final class AreaPointView: MKAnnotationView {
    override init(annotation: (any MKAnnotation)?, reuseIdentifier: String?) {
        super.init(annotation: annotation, reuseIdentifier: reuseIdentifier)
        frame = CGRect(x: 0, y: 0, width: 1, height: 1)
        isUserInteractionEnabled = false
        canShowCallout = false
        displayPriority = .required
        isAccessibilityElement = false
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { nil }
}
