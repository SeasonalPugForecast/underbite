import MapKit
import SwiftUI
@preconcurrency import UnderbiteKit

/// Sea surface temperature and chlorophyll breaks: lines where the water changes sharply, found
/// by the core in the grid the `sst` or `chlorophyll` layer already shows (the same day, dataset
/// and blocks in the map's `GridStore`, so no request of their own) through `GridStore.breaks`,
/// and drawn heavier the stronger they are (the core's `emphasis`), one overlay with one
/// renderer per breaks layer (``BreakOverlay``).
///
/// The breaks are found again when the view settles (a new zoom may change the stride of the
/// blocks they are found in), when blocks of the grid layer arrive, when another day of it is
/// picked and when the threshold or the units change. They follow the catalogue's `sst-breaks`
/// and `chlorophyll-breaks` as the drawer, the presets and the settings show them, at their
/// opacity; while their grid layer is not on the map, nothing is drawn and the drawer says to
/// show it.
///
/// Every value and word is the core's: the threshold's stops, the lines' strengths, the legend,
/// the grid and day the breaks came from, the states, the credit and "Not for navigation."
/// (`breaks_words`, `break_threshold_stops`). The drawer's section holds the state, the
/// threshold control and the legend of each layer shown.
@MainActor
@Observable
final class BreaksModule: MapModule {
    /// The words of the controls, states and legends for the settings drawn with.
    private(set) var words: BreaksWords?
    /// What of the module is shown, for the drawer.
    private(set) var visible = BreaksShown()
    /// Per breaks layer, the breaks found for the view.
    private(set) var sets: [String: BreakSet] = [:]
    /// The zoom of the view, once the map has settled.
    private(set) var zoom: Double?
    /// The settings the module draws with: the map's, and the threshold a launch names.
    private(set) var settingsJSON = ""

    @ObservationIgnored private weak var controller: MapController?
    @ObservationIgnored private var shown = BreaksShown()
    @ObservationIgnored private var region: MapRegion?
    /// The map's settings as last given, before the launch's threshold.
    @ObservationIgnored private var givenJSON = ""
    /// Per breaks layer, its overlay and the renderer drawing it.
    @ObservationIgnored private var lineOverlays: [String: BreakOverlay] = [:]
    @ObservationIgnored private var renderers: [String: BreakRenderer] = [:]
    /// Per breaks layer, what its breaks were last looked for with.
    @ObservationIgnored private var asked: [String: BreaksQuery] = [:]
    @ObservationIgnored private var findTask: Task<Void, Never>?
    /// Each look is numbered; what an older look found never replaces a newer one's.
    @ObservationIgnored private var looks = 0
    @ObservationIgnored private var shownLook = 0

    /// How long the map stays quiet before the breaks are looked for again: blocks arrive in
    /// bursts.
    static let settle: Duration = .milliseconds(250)

    init(controller: MapController) {
        self.controller = controller
        watch()
    }

    /// The catalogue's breaks layers.
    var layerIDs: Set<String> {
        Set((controller?.catalogue ?? []).filter { $0.draw == .breaks }.map(\.id))
    }

    // MARK: - Drawing

    func overlays(in context: MapModuleContext) -> [any MKOverlay] {
        // Asked while the map view updates: what follows from a change waits for it to end.
        if shownNow(context.layers) != shown {
            Task { @MainActor [weak self] in self?.refresh() }
        }
        return shown.findable.compactMap { lineOverlays[$0.id] }
    }

    func renderer(for overlay: any MKOverlay) -> MKOverlayRenderer? {
        guard let lines = overlay as? BreakOverlay else { return nil }
        let made = BreakRenderer(lines)
        made.alpha = CGFloat(shown.layer(lines.layer)?.opacity ?? 1)
        if lineOverlays[lines.layer] === lines { renderers[lines.layer] = made }
        return made
    }

    func regionDidSettle(_ context: MapModuleContext) {
        if let region = context.region {
            self.region = region
            zoom = region.zoom
        }
        refresh()
    }

    // MARK: - Following the map

    /// Hears the settings, the plan and the grid layers' blocks change.
    private func watch() {
        guard let controller else { return }
        withObservationTracking {
            _ = controller.settingsJSON
            _ = controller.plan
            _ = controller.gridRevisions
        } onChange: { [weak self] in
            Task { @MainActor [weak self] in
                self?.refresh()
                self?.watch()
            }
        }
    }

    /// The breaks layers on screen now, with their grid layers as the map draws them.
    private func shownNow(_ layers: [LayerInfo]) -> BreaksShown {
        let sources = Dictionary(
            (words?.layers ?? []).map { ($0.id, $0.source) }, uniquingKeysWith: { a, _ in a })
        return BreaksShown(layers, drawn: controller?.plan.layers ?? [], sources: sources)
    }

    /// Follows what is shown and the settings, then looks for the breaks again once the map has
    /// been quiet for a moment.
    private func refresh() {
        guard let controller else { return }
        if controller.settingsJSON != givenJSON || words == nil {
            givenJSON = controller.settingsJSON
            let json = BreaksLaunch.applied(to: givenJSON)
            settingsJSON = json
            let next = try? breaksWords(settingsJson: json)
            if next != words { words = next }
        }
        let context = controller.context(for: self)
        let now = shownNow(context.layers)
        if now != shown {
            shown = now
            visible = now
            for layer in now.layers {
                renderers[layer.id]?.alpha = CGFloat(layer.opacity)
            }
        }
        // Layers no longer shown, or whose grid layer is off, lose their lines.
        let findable = Set(now.findable.map(\.id))
        var dropped = false
        for id in Set(lineOverlays.keys).subtracting(findable) {
            lineOverlays[id] = nil
            renderers[id] = nil
            dropped = true
        }
        for id in Set(sets.keys).subtracting(findable) {
            sets[id] = nil
            asked[id] = nil
        }
        if dropped { controller.moduleDidChange() }
        scheduleFind()
    }

    private func scheduleFind() {
        findTask?.cancel()
        guard shown.any, region != nil else { return }
        findTask = Task { [weak self] in
            try? await Task.sleep(for: Self.settle)
            guard !Task.isCancelled else { return }
            await self?.find()
        }
    }

    /// Looks for the breaks of each layer whose grid layer is drawn, unless nothing they depend
    /// on changed since the last look; the core works outside the main thread.
    private func find() async {
        guard let controller, let region else { return }
        let json = settingsJSON
        var queries: [BreaksQuery] = []
        for layer in shown.findable {
            guard BreaksStatus.deepEnough(zoom: region.zoom, minZoom: layer.minZoom) else {
                clear(layer.id)
                continue
            }
            let query = BreaksQuery(
                layer: layer.id, region: region, settingsJSON: json,
                sourceKey: layer.sourceKey ?? "",
                revision: controller.gridRevisions[layer.source] ?? 0)
            if asked[layer.id] != query { queries.append(query) }
        }
        guard !queries.isEmpty else { return }
        looks += 1
        let look = looks
        let store = controller.gridStore
        let found = await Task.detached(priority: .userInitiated) { () -> [Found] in
            queries.map { query in
                let r = query.region
                let set = try? store.breaks(
                    layer: query.layer, west: r.west, south: r.south, east: r.east,
                    north: r.north, zoom: r.zoom, settingsJson: query.settingsJSON)
                return Found(query: query, set: set, lines: set.map { BreakLines($0.lines) })
            }
        }.value
        // What was found stands until a newer look lands, even if the map moved meanwhile.
        guard look > shownLook else { return }
        shownLook = look
        var changed = false
        for one in found {
            let id = one.query.layer
            // A layer switched off meanwhile stays off.
            guard shown.findable.contains(where: { $0.id == id }) else { continue }
            asked[id] = one.query
            sets[id] = one.set
            lineOverlays[id] = one.lines.map { BreakOverlay(layer: id, lines: $0) }
            renderers[id] = nil
            changed = true
        }
        if changed { controller.moduleDidChange() }
    }

    /// A layer whose view is too wide: no lines, and the drawer says to zoom in.
    private func clear(_ id: String) {
        asked[id] = nil
        sets[id] = nil
        if lineOverlays.removeValue(forKey: id) != nil {
            renderers[id] = nil
            controller?.moduleDidChange()
        }
    }

    // MARK: - Drawer

    func drawerSection() -> AnyView? {
        guard visible.any, words != nil else { return nil }
        return AnyView(BreaksSection(module: self))
    }

    /// The status of a shown layer for the drawer.
    func status(of layer: BreaksShown.Layer) -> BreaksStatus? {
        BreaksStatus.of(layer, zoom: zoom, set: sets[layer.id])
    }
}

/// What a layer's breaks were looked for with: when none of it changes, they are not looked for
/// again.
nonisolated struct BreaksQuery: Equatable, Sendable {
    let layer: String
    let region: MapRegion
    let settingsJSON: String
    /// The grid layer's overlay key: its day, dataset and colours.
    let sourceKey: String
    /// How often blocks of the grid layer arrived.
    let revision: Int
}

/// The breaks found for one query, placed for drawing.
private nonisolated struct Found: Sendable {
    let query: BreaksQuery
    let set: BreakSet?
    let lines: BreakLines?
}
