import Foundation
import MapKit
import UnderbiteKit

/// The map's state, shared by the map view, its controls and the modules: the core's layer
/// catalogue for the settings, the plan of what is drawn, the region on screen, taps, the dates
/// and times the dated layers show, and the grid blocks and tiles as they arrive.
///
/// Layer choices live in the settings' `[map]` section: switching a layer, the base map or an
/// opacity writes the settings (through `writeSettings`), and the map follows the settings it is
/// given in ``apply(settingsJSON:)``. Dates, times and opacity previews are the map's own until a
/// control saves them.
@MainActor
@Observable
final class MapController {
    /// The core's catalogue for the current settings: bases, then overlays in drawer order.
    private(set) var catalogue: [LayerInfo] = []
    /// What is drawn.
    private(set) var plan = MapPlan()
    /// The settings JSON the map draws with.
    private(set) var settingsJSON = ""
    /// The region on screen, once the map has settled.
    private(set) var region: MapRegion?
    /// Where the map opens.
    let start: MapStart
    /// The point inspector's content after a tap that no module took.
    var inspection: MapInspection?
    /// Per tiled layer, the day shown (`YYYY-MM-DD`); none for the source's newest.
    private(set) var layerDates: [String: String] = [:]
    /// Per grid layer, the time shown (Unix seconds); none for the newest.
    private(set) var layerTimes: [String: Int64] = [:]
    /// Per grid layer, the dataset used; none for the primary.
    private(set) var layerDatasets: [String: String] = [:]
    /// Per layer, an opacity being previewed (a slider being dragged).
    private(set) var previewOpacities: [String: Double] = [:]
    /// Per grid layer, raised as blocks arrive: the map draws the layer again.
    private(set) var gridRevisions: [String: Int] = [:]
    /// Per tiled layer and day, what its tiles came back as.
    private(set) var tileReports: [String: TileReport] = [:]
    /// Raised by ``moduleDidChange()``: the map asks the modules again.
    private(set) var moduleRevision = 0

    /// The modules, in drawing order (``MapModules``).
    @ObservationIgnored private(set) var modules: [any MapModule] = []
    /// Writes changed settings JSON (the store's `updateSettings`).
    @ObservationIgnored var writeSettings: ((String) -> Void)?
    @ObservationIgnored let gridStore: GridStore
    @ObservationIgnored let tileLoader: TileLoader
    @ObservationIgnored private let gridLoader: GridLoader
    @ObservationIgnored private let clock: @Sendable () -> Int64
    @ObservationIgnored private let lowPower: () -> Bool
    /// The settings as given, before any launch override.
    @ObservationIgnored private var givenJSON = ""
    @ObservationIgnored private var mapSettings: Settings.MapSettings?
    @ObservationIgnored private var arrowOverlays: [String: ArrowOverlay] = [:]
    @ObservationIgnored private var blocksDue: Set<String> = []

    /// - Parameters:
    ///   - host: the app's fetch host; tiles and grids go through it and nothing else.
    ///   - clock: Unix seconds, the map's one clock read.
    init(
        start: MapStart,
        host: FetchHost = URLSessionHost(storeURL: AppFiles.standard.hostDirectory),
        clock: @escaping @Sendable () -> Int64 = { Int64(Date().timeIntervalSince1970) },
        lowPower: @escaping () -> Bool = { ProcessInfo.processInfo.isLowPowerModeEnabled }
    ) {
        self.start = start
        self.clock = clock
        self.lowPower = lowPower
        let store = GridStore()
        gridStore = store
        tileLoader = TileLoader(host: host)
        gridLoader = GridLoader(host: host, store: store, clock: clock)
        gridLoader.onBlock = { [weak self] layer in self?.blockArrived(layer) }
        gridLoader.onNoData = { [weak self] layer in self?.noData(layer) }
        modules = MapModules.make(controller: self)
        let report: TileLoader.Report = { [weak self] layer, date, outcome in
            Task { @MainActor [weak self] in
                self?.record(layer: layer, date: date, outcome: outcome)
            }
        }
        let loader = tileLoader
        Task { await loader.onReport(report) }
    }

    /// Whether the battery-friendly mode is on: at most two raster layers, no animation.
    var battery: Bool { plan.battery }

    // MARK: - Settings

    /// Draws with `json` (the app's settings): the catalogue and the plan follow it.
    func apply(settingsJSON json: String) {
        guard json != givenJSON else { return }
        givenJSON = json
        var effective = json
        if MapLaunch.overrides, var settings = try? Settings(json: json) {
            MapLaunch.apply(to: &settings.map)
            effective = (try? settings.json()) ?? json
        }
        settingsJSON = effective
        mapSettings = try? Settings(json: effective).map
        if let catalogue = try? mapCatalogue(settingsJson: effective) {
            self.catalogue = catalogue
        }
        replan()
    }

    /// Low Power Mode changed: the battery-friendly mode may follow it.
    func powerStateChanged() {
        replan()
    }

    /// Shows or hides an overlay layer, or, for a base map, chooses it.
    func setShown(_ id: String, _ shown: Bool) {
        if catalogue.contains(where: { $0.id == id && $0.isBase }) {
            if shown { setBase(id) }
            return
        }
        edit { map in
            if let i = map.layers.firstIndex(where: { $0.id == id }) {
                map.layers[i].shown = shown
            } else if let added = Self.layerState(id: id, shown: shown) {
                map.layers.append(added)
            }
        }
    }

    /// Chooses the base map.
    func setBase(_ id: String) {
        edit { $0.base = id }
    }

    /// Previews an opacity while a slider moves; ``setOpacity(_:_:)`` saves it.
    func previewOpacity(_ id: String, _ opacity: Double?) {
        previewOpacities[id] = opacity
        replan()
    }

    /// Saves a layer's opacity (0 to 1) in the settings.
    func setOpacity(_ id: String, _ opacity: Double) {
        previewOpacities[id] = nil
        edit { map in
            if let i = map.layers.firstIndex(where: { $0.id == id }) {
                map.layers[i].opacity = min(max(opacity, 0), 1)
            }
        }
        replan()
    }

    /// The day a tiled layer shows (`YYYY-MM-DD`, a UTC day); none for the newest.
    func setDate(_ date: String?, for id: String) {
        layerDates[id] = date
        replan()
    }

    /// The time a grid layer shows (Unix seconds); none for the newest.
    func setTime(_ time: Int64?, for id: String) {
        layerTimes[id] = time
        replan()
    }

    /// The dataset a grid layer uses; none for the primary.
    func setDataset(_ dataset: String?, for id: String) {
        layerDatasets[id] = dataset
        replan()
    }

    /// Applies `change` to the given settings (not the launch overrides) and writes them.
    private func edit(_ change: (inout Settings.MapSettings) -> Void) {
        guard var settings = try? Settings(json: givenJSON) else { return }
        change(&settings.map)
        guard let json = try? settings.json() else { return }
        writeSettings?(json)
    }

    /// A layer state for an id the settings do not list yet.
    private static func layerState(id: String, shown: Bool) -> Settings.MapLayer? {
        let json = #"{"id":"\#(id)","shown":\#(shown)}"#
        return try? JSONDecoder().decode(Settings.MapLayer.self, from: Data(json.utf8))
    }

    private func replan() {
        guard let map = mapSettings else {
            plan = MapPlan()
            return
        }
        let next = MapPlan.make(
            catalogue: catalogue, settings: map,
            battery: MapPlan.batteryOn(setting: map.batteryMode, lowPower: lowPower()),
            dates: layerDates, times: layerTimes, datasets: layerDatasets,
            opacities: previewOpacities)
        guard next != plan else { return }
        let fetchChanged =
            next.layers.map(\.overlayKey) != plan.layers.map(\.overlayKey)
        plan = next
        if fetchChanged { loadForRegion() }
    }

    // MARK: - The region and taps

    /// The map settled on `region`.
    func regionDidSettle(_ region: MapRegion) {
        self.region = region
        loadForRegion()
    }

    private func loadForRegion() {
        guard let region else { return }
        gridLoader.load(plan.layers, in: region)
        refreshArrows()
        for module in modules {
            module.regionDidSettle(context(for: module))
        }
    }

    /// What `module` gets to draw with.
    func context(for module: any MapModule) -> MapModuleContext {
        MapModuleContext(
            region: region,
            layers: plan.shown.filter { module.layerIDs.contains($0.id) },
            settingsJSON: settingsJSON, battery: plan.battery)
    }

    /// A module has something new to draw.
    func moduleDidChange() {
        moduleRevision += 1
    }

    /// A tap on the map: the topmost module that takes it, else the point inspector with the
    /// values of the grid layers on screen.
    func didTap(_ tap: MapTap) {
        for module in modules.reversed() where module.handleTap(tap) {
            return
        }
        let layers = plan.layers.filter { $0.kind != .tiles }.map(\.id).reversed()
        guard !layers.isEmpty,
            let rows = try? gridStore.inspect(
                lat: tap.coordinate.latitude, lon: tap.coordinate.longitude, layers: Array(layers),
                settingsJson: settingsJSON, timeZone: TimeZone.current.identifier),
            !rows.isEmpty
        else { return }
        inspection = MapInspection(tap: tap, rows: rows)
    }

    // MARK: - Grids and arrows

    /// The one overlay of an arrow layer.
    func arrowOverlay(for id: String) -> ArrowOverlay {
        if let overlay = arrowOverlays[id] { return overlay }
        let overlay = ArrowOverlay(layer: id)
        arrowOverlays[id] = overlay
        return overlay
    }

    /// The arrows of the arrow layers for the region on screen, from the blocks held.
    private func refreshArrows() {
        guard let region else { return }
        for layer in plan.layers where layer.kind == .gridArrows {
            let arrows =
                (try? gridStore.arrows(
                    layer: layer.id, ramp: layer.ramp, low: layer.range?.first,
                    high: layer.range?.last, west: region.west, south: region.south,
                    east: region.east, north: region.north, zoom: region.zoom)) ?? []
            arrowOverlay(for: layer.id).set(arrows)
            gridRevisions[layer.id, default: 0] += 1
        }
    }

    /// Blocks arrive one by one: the layers are drawn again once a moment has passed.
    private func blockArrived(_ layer: String) {
        let first = blocksDue.isEmpty
        blocksDue.insert(layer)
        guard first else { return }
        Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(300))
            self?.flushBlocks()
        }
    }

    private func flushBlocks() {
        let layers = blocksDue
        blocksDue = []
        for layer in layers {
            gridRevisions[layer, default: 0] += 1
        }
        if plan.layers.contains(where: { $0.kind == .gridArrows && layers.contains($0.id) }) {
            refreshArrows()
        }
    }

    /// Every block of a layer had no data: a layer with a fallback dataset switches to it.
    private func noData(_ layer: String) {
        guard layerDatasets[layer] == nil,
            let info = catalogue.first(where: { $0.id == layer }), info.datasets.count > 1
        else { return }
        setDataset(info.datasets[1], for: layer)
    }

    private func record(layer: String, date: String?, outcome: TileOutcome) {
        let key = TileReport.key(layer: layer, date: date)
        var report = tileReports[key] ?? TileReport()
        switch outcome {
        case .image: report.images += 1
        case .noData: report.empty += 1
        default: report.failed += 1
        }
        tileReports[key] = report
    }
}

/// Where the map opens: a centre and the degrees of latitude shown.
struct MapStart: Equatable {
    var latitude: Double
    var longitude: Double
    var span: Double

    /// The lower 48 states, for a first launch without spots.
    static let overview = MapStart(latitude: 39.5, longitude: -98.35, span: 28)
}

/// The point inspector's content.
struct MapInspection: Identifiable {
    let tap: MapTap
    let rows: [InspectorRow]
    var id: UUID { tap.id }
}

/// What a tiled layer's tiles for one day came back as.
struct TileReport: Equatable {
    var images = 0
    /// No data for the day (a GIBS day not yet or never there).
    var empty = 0
    var failed = 0

    static func key(layer: String, date: String?) -> String { "\(layer)|\(date ?? "-")" }
}
