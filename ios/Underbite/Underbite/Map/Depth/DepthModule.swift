import MapKit
import SwiftUI
import UnderbiteKit

/// Depth from the NOAA NCEI elevation model: the shading of the water by approximate depth with
/// the depth band highlighted, and the depth contours with their labels, major lines heavier.
/// Both draw from one elevation area per view and zoom (``DepthLoader``) in the map's
/// `GridStore`: the shading through the core's `render_depth_tile` as a tile overlay replaced
/// when the area, the band or the ramp change (``DepthTileOverlay``), the contours through
/// `contour_lines` for the area as one overlay with one renderer (``ContourOverlay``), drawn
/// again when the interval or the units change.
///
/// Every value and word is the core's: the band's stops, the contour labels, the legend, the
/// datum note, the credit and "Not for navigation." (`depth_layer_info`). The drawer's section
/// holds the band control and the legend: the side panel on the iPad, the sheet on the iPhone.
@MainActor
@Observable
final class DepthModule: MapModule {
    /// The shading's words, legend and credit for the settings drawn with; none while nothing
    /// of the module is shown.
    private(set) var info: DepthLayerInfo?
    /// Where the elevation for the view stands.
    private(set) var loading: DepthLoader.State = .idle
    /// The settings the module draws with: the map's, and the band a launch names.
    private(set) var settingsJSON = ""

    @ObservationIgnored private weak var controller: MapController?
    @ObservationIgnored private let loader: DepthLoader
    @ObservationIgnored private var shown = DepthShown()
    @ObservationIgnored private var shading: DepthTileOverlay?
    @ObservationIgnored private var shadingRenderer: MKTileOverlayRenderer?
    /// Shading overlays replaced by a newer one, on the map until it has its first tiles.
    @ObservationIgnored private var retired: [DepthTileOverlay] = []
    @ObservationIgnored private var contours: ContourOverlay?
    @ObservationIgnored private var contourRenderer: ContourRenderer?
    @ObservationIgnored private var contourSet = ContourSet.empty
    /// What the overlays show: the area and the settings that matter to them.
    @ObservationIgnored private var drawn: DepthDrawn?
    @ObservationIgnored private var contourTask: Task<Void, Never>?
    @ObservationIgnored private var region: MapRegion?

    init(
        controller: MapController,
        host: FetchHost = URLSessionHost(storeURL: AppFiles.standard.hostDirectory),
        clock: @escaping @Sendable () -> Int64 = { Int64(Date().timeIntervalSince1970) }
    ) {
        self.controller = controller
        loader = DepthLoader(host: host, store: controller.gridStore, clock: clock)
        loader.onChange = { [weak self] state in self?.loaded(state) }
        watchSettings()
    }

    /// The catalogue's depth layers this module draws.
    var layerIDs: Set<String> {
        let known = Set((controller?.catalogue ?? []).map(\.id))
        return DepthShown.ids.intersection(known)
    }

    /// Under the base map's labels, as the map's other shaded layers are.
    var overlayLevel: MKOverlayLevel { .aboveRoads }

    // MARK: - Drawing

    func overlays(in context: MapModuleContext) -> [any MKOverlay] {
        // Asked while the map view updates: what follows from a change waits for it to end.
        if DepthShown(context.layers, launched: DepthLaunch.layers) != shown {
            Task { @MainActor [weak self] in self?.follow(context) }
        }
        var out: [any MKOverlay] = []
        if shown.shading, let shading {
            out += retired
            out.append(shading)
        }
        if shown.contours, let contours { out.append(contours) }
        return out
    }

    func renderer(for overlay: any MKOverlay) -> MKOverlayRenderer? {
        if let tiles = overlay as? DepthTileOverlay {
            let made = MKTileOverlayRenderer(tileOverlay: tiles)
            made.alpha = CGFloat(shown.shadingOpacity)
            if tiles === shading { shadingRenderer = made }
            return made
        }
        if let lines = overlay as? ContourOverlay {
            let made = ContourRenderer(lines)
            made.alpha = CGFloat(shown.contourOpacity)
            if lines === contours { contourRenderer = made }
            return made
        }
        return nil
    }

    func regionDidSettle(_ context: MapModuleContext) {
        follow(context)
        guard let region = context.region else { return }
        self.region = region
        guard shown.any else { return }
        loader.load(region)
    }

    /// Follows what is shown and at what opacity.
    private func follow(_ context: MapModuleContext) {
        let now = DepthShown(context.layers, launched: DepthLaunch.layers)
        guard now != shown else { return }
        let was = shown
        shown = now
        shadingRenderer?.alpha = CGFloat(now.shadingOpacity)
        contourRenderer?.alpha = CGFloat(now.contourOpacity)
        if now.any {
            refreshInfo()
            if !was.any, let region { loader.load(region) }
            if was.contours != now.contours || was.shading != now.shading { redraw() }
        } else {
            info = nil
        }
    }

    // MARK: - Settings

    /// Hears the map's settings change: a new band, ramp, interval or unit draws again.
    private func watchSettings() {
        guard let controller else { return }
        withObservationTracking {
            _ = controller.settingsJSON
        } onChange: { [weak self] in
            Task { @MainActor [weak self] in
                self?.settingsChanged()
                self?.watchSettings()
            }
        }
    }

    private func settingsChanged() {
        guard shown.any else { return }
        refreshInfo()
        redraw()
    }

    /// The settings to draw with, the shading's words and legend.
    private func refreshInfo() {
        let json = DepthLaunch.applied(to: controller?.settingsJSON ?? "")
        if json != settingsJSON { settingsJSON = json }
        let next = try? depthLayerInfo(settingsJson: json)
        if next != info { info = next }
    }

    // MARK: - Elevation

    private func loaded(_ state: DepthLoader.State) {
        loading = state
        redraw()
    }

    /// Draws the overlays again when the area or the settings that matter have changed.
    private func redraw() {
        guard shown.any, case .ready(let area) = loader.state else { return }
        let next = DepthDrawn(area: area, inputs: DepthDrawn.inputs(settingsJSON), shown: shown)
        guard next != drawn else { return }
        let before = drawn
        drawn = next
        // The lines drawn stay until the new ones are worked out.
        if next.area != before?.area || next.inputs.contoursDiffer(from: before?.inputs)
            || next.shown.contours != before?.shown.contours
        {
            loadContours(area)
        }
        if next.area != before?.area || next.inputs.shadingDiffers(from: before?.inputs)
            || next.shown.shading != before?.shown.shading
        {
            replaceShading()
        } else {
            placeContours()
        }
    }

    /// A new shading overlay over the old one, which comes off once the new one has its first
    /// tiles; the contours go back on top of it.
    private func replaceShading() {
        if let old = shading { retired.append(old) }
        guard shown.shading else {
            shading = nil
            retired = []
            placeContours()
            return
        }
        guard let store = controller?.gridStore else { return }
        let fresh = DepthTileOverlay(store: store, settingsJSON: settingsJSON)
        shading = fresh
        shadingRenderer = nil
        let id = ObjectIdentifier(fresh)
        fresh.firstTiles.onAnswered { [weak self] in
            Task { @MainActor in self?.settled(id) }
        }
        Task { @MainActor [weak self] in
            try? await Task.sleep(for: .seconds(2))
            self?.settled(id)
        }
        placeContours()
    }

    /// The shading overlay `id` has its first tiles (or waited long enough).
    private func settled(_ id: ObjectIdentifier) {
        guard let shading, ObjectIdentifier(shading) == id, !retired.isEmpty else { return }
        retired = []
        controller?.moduleDidChange()
    }

    /// The contours as a new overlay, so that the map puts them above the newest shading.
    private func placeContours() {
        contours = shown.contours && !contourSet.lines.isEmpty ? ContourOverlay(contourSet) : nil
        contourRenderer = nil
        controller?.moduleDidChange()
    }

    /// The contours of `area` at the settings' interval, worked out off the main thread.
    private func loadContours(_ area: DepthArea) {
        contourTask?.cancel()
        guard shown.contours, let store = controller?.gridStore else {
            contourSet = .empty
            return
        }
        let json = settingsJSON
        contourTask = Task { [weak self] in
            let set = await Task.detached(priority: .userInitiated) { () -> ContourSet in
                let lines =
                    (try? store.contourLines(
                        west: area.west, south: area.south, east: area.east, north: area.north,
                        zoom: area.zoom, settingsJson: json)) ?? []
                return ContourSet(lines)
            }.value
            guard !Task.isCancelled, let self, self.drawn?.area == area else { return }
            self.contourSet = set
            self.placeContours()
        }
    }

    // MARK: - Drawer

    func drawerSection() -> AnyView? {
        guard shown.any, info != nil else { return nil }
        return AnyView(DepthSection(module: self))
    }
}

/// What of the depth layers is shown, and at what opacity.
nonisolated struct DepthShown: Equatable, Sendable {
    /// The catalogue ids of the shading and the contours.
    static let shadingID = "depth-shading"
    static let contoursID = "depth-contours"
    static let ids: Set<String> = [shadingID, contoursID]

    var shading = false
    var contours = false
    var shadingOpacity = 1.0
    var contourOpacity = 1.0

    var any: Bool { shading || contours }

    init() {}

    /// From the module's layers on screen, and the ids a launch names.
    init(_ layers: [LayerInfo], launched: Set<String> = []) {
        for layer in layers {
            switch layer.id {
            case Self.shadingID:
                shading = true
                shadingOpacity = Double(layer.opacity)
            case Self.contoursID:
                contours = true
                contourOpacity = Double(layer.opacity)
            default:
                break
            }
        }
        if launched.contains(Self.shadingID) { shading = true }
        if launched.contains(Self.contoursID) { contours = true }
    }
}

/// What the module's overlays were drawn for.
nonisolated struct DepthDrawn: Equatable, Sendable {
    /// The settings that matter to the overlays.
    struct Inputs: Equatable, Sendable {
        /// What the shading draws with: the band and the ramp.
        var band: [Double]?
        var ramp: String?
        /// What the contours draw with: the interval and the units of their labels.
        var interval: Double
        var units: Settings.Units?

        /// Whether the shading draws differently with `other`.
        func shadingDiffers(from other: Inputs?) -> Bool {
            band != other?.band || ramp != other?.ramp
        }

        /// Whether the contours draw differently with `other`.
        func contoursDiffer(from other: Inputs?) -> Bool {
            interval != other?.interval || units != other?.units
        }
    }

    let area: DepthArea
    let inputs: Inputs
    let shown: DepthShown

    static func inputs(_ json: String) -> Inputs {
        guard let settings = try? Settings(json: json) else {
            return Inputs(band: nil, ramp: nil, interval: 0, units: nil)
        }
        let depth = settings.map.depth
        return Inputs(
            band: depth.bandM, ramp: depth.ramp, interval: depth.contourIntervalM,
            units: settings.units)
    }
}

/// The depth module's launch arguments, read in debug builds only (screenshots): `-mapDepth
/// <id,id>` shows the shading (`depth-shading`) and the contours (`depth-contours`), and
/// `-mapDepthBand <shallow>,<deep>` (metres) highlights that band. Nothing is saved.
enum DepthLaunch {
    static var layers: Set<String> {
        guard let list = LaunchArguments.value(after: "-mapDepth") else { return [] }
        return Set(list.split(separator: ",").map { String($0) })
    }

    static var band: [Double]? {
        guard let text = LaunchArguments.value(after: "-mapDepthBand") else { return nil }
        let values = text.split(separator: ",").compactMap { Double($0) }
        return values.count == 2 ? values : nil
    }

    /// `json` with the band a launch names.
    static func applied(to json: String) -> String {
        guard let band, var settings = try? Settings(json: json) else { return json }
        settings.map.depth.bandM = band
        return (try? settings.json()) ?? json
    }
}
