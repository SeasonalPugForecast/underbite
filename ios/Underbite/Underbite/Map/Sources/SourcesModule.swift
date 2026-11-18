import MapKit
import Observation
import SwiftUI
import UnderbiteKit

/// The dates and times of the dated layers, and the radar loop.
///
/// - Satellite layers (GIBS) open on the core's `gibs_default_date` and step back a day while the
///   day asked for answers "no data" (at most ``SatelliteDay/maxSteps`` days), from the tile
///   reports the map keeps per layer and day; the day shown is the day served.
/// - Grid layers (ERDDAP) read their dataset's time axis (``TimeAxes``), so the controls offer
///   only times that exist, and a time the axis no longer holds goes back to the newest. "Last
///   clear day" asks for the newest days in turn until the view has values.
/// - The radar loop draws its frames as tile overlays of its own, only the frame shown at full
///   opacity, while the map's own radar overlay (the latest frame) is held at none; it plays
///   only outside the battery-friendly mode.
///
/// ``TimeControls`` shows the controls of the topmost dated layer.
@MainActor
@Observable
final class SourcesModule: MapModule {
    /// How "last clear day" ended.
    enum ClearSearch: Equatable {
        case looking
        case found
        /// No day with values in view among those tried.
        case none
    }

    @ObservationIgnored private weak var controller: MapController?
    @ObservationIgnored private let axes: TimeAxes
    @ObservationIgnored private let radar: RadarFrames
    @ObservationIgnored private let clock: @Sendable () -> Int64

    /// Per satellite layer, its day.
    private(set) var days: [String: SatelliteDay] = [:]
    /// Per grid layer and dataset, its times, labelled for the settings.
    private(set) var times: [TimeAxes.Key: [LayerTime]] = [:]
    /// Per grid layer, how its last "last clear day" went.
    private(set) var clearSearches: [String: ClearSearch] = [:]
    /// The radar loop's frames, oldest first.
    private(set) var frames: [RadarFrame] = []
    /// The frame shown.
    private(set) var frameIndex = 0
    /// Whether the loop plays.
    private(set) var playing = false
    /// Whether the loop's frames are on the map (once a frame was asked for).
    private(set) var loopShown = false

    @ObservationIgnored private var asked: [TimeAxes.Key: (json: String, at: Int64)] = [:]
    @ObservationIgnored private var asking: Set<TimeAxes.Key> = []
    @ObservationIgnored private var searchGeneration = 0
    @ObservationIgnored private var framesAsked: (json: String, at: Int64)?
    @ObservationIgnored private var loadingFrames = false
    @ObservationIgnored private var frameOverlays: [RadarFrameOverlay] = []
    @ObservationIgnored private var frameRenderers: [ObjectIdentifier: MKTileOverlayRenderer] = [:]
    @ObservationIgnored private var playTask: Task<Void, Never>?
    @ObservationIgnored private var loopStep = 0
    @ObservationIgnored private var holdsRadar = false
    @ObservationIgnored private var syncDue = false
    @ObservationIgnored private var pickerDays: (newest: MapDate, days: [MapDate])?
    /// Screenshots (debug builds): `-mapDates clear` looks for the last clear day of the topmost
    /// grid layer once its time axis is in.
    @ObservationIgnored private var launchClearDay =
        LaunchArguments.value(after: "-mapDates") == "clear"

    /// How often the time axes are looked at again (the axes decide whether to ask the server).
    private static let axisCheck: Int64 = 600

    init(controller: MapController) {
        self.controller = controller
        let host = URLSessionHost(storeURL: AppFiles.standard.hostDirectory)
        let clock: @Sendable () -> Int64 = { Int64(Date().timeIntervalSince1970) }
        self.clock = clock
        axes = TimeAxes(host: host, clock: clock)
        radar = RadarFrames(host: host, cache: controller.tileLoader.cache)
        scheduleSync()
        watchReports()
    }

    /// Unix seconds, the module's one clock read.
    var now: Int64 { clock() }

    // MARK: - MapModule

    var layerIDs: Set<String> { [RadarFrames.layer] }

    /// The frames sit where the map's radar overlay does: over the chart's labels on the chart,
    /// else between the roads and the labels.
    var overlayLevel: MKOverlayLevel {
        let radar = controller?.plan.layers.first { $0.id == RadarFrames.layer }
        return radar?.aboveLabels == false ? .aboveRoads : .aboveLabels
    }

    func overlays(in context: MapModuleContext) -> [any MKOverlay] {
        // Asked again whenever what the map draws changes.
        scheduleSync()
        return loopShown ? frameOverlays : []
    }

    func renderer(for overlay: any MKOverlay) -> MKOverlayRenderer? {
        guard let frame = overlay as? RadarFrameOverlay,
            let index = frameOverlays.firstIndex(where: { $0 === frame })
        else { return nil }
        let renderer = MKTileOverlayRenderer(tileOverlay: frame)
        renderer.alpha = index == frameIndex ? radarOpacity : 0
        frameRenderers[ObjectIdentifier(frame)] = renderer
        return renderer
    }

    func regionDidSettle(_ context: MapModuleContext) {
        scheduleSync()
    }

    // MARK: - Keeping up with the map

    /// Looks at the layers on the map once the current change is done.
    private func scheduleSync() {
        guard !syncDue else { return }
        syncDue = true
        Task { [weak self] in self?.sync() }
    }

    private func sync() {
        syncDue = false
        guard let controller else { return }
        let now = clock()
        // Every satellite layer opens on the default day, shown or not, so a layer switched on
        // asks for that day from its first tile. A new UTC day moves a day not picked.
        let opening = gibsDefaultDate(now: now)
        for layer in controller.catalogue where layer.draw == .tiles && layer.dates == .daily {
            if let day = days[layer.id], day.picked || day.chosen == opening { continue }
            days[layer.id] = SatelliteDay(chosen: opening)
            controller.setDate(opening.date, for: layer.id)
        }
        checkReports()
        for layer in controller.plan.layers {
            guard let control = DateControl(layer), control == .gridDays || control == .gridHours
            else { continue }
            let key = TimeAxes.Key(layer: layer.id, dataset: layer.dataset)
            askTimes(key, dates: layer.info.dates, now: now)
            // Only a time the axis holds is asked for.
            if let axis = times[key], GridTimes.checked(layer.time, in: axis) != layer.time {
                controller.setTime(GridTimes.checked(layer.time, in: axis), for: layer.id)
            }
        }
        if launchClearDay, let top = DateControl.topmost(controller.plan),
            top.control == .gridDays, canFindClearDay(top.layer)
        {
            launchClearDay = false
            findClearDay(top.layer)
        }
        syncRadar(now: now)
    }

    /// Follows the tile reports: a satellite day without pictures steps back.
    private func watchReports() {
        withObservationTracking {
            _ = controller?.tileReports
        } onChange: { [weak self] in
            Task { @MainActor [weak self] in
                self?.checkReports()
                self?.watchReports()
            }
        }
    }

    private func checkReports() {
        guard let controller else { return }
        for (id, before) in days {
            guard
                let report = controller.tileReports[
                    TileReport.key(layer: id, date: before.asked.date)]
            else { continue }
            var day = before
            let back = day.update(
                images: report.images, empty: report.empty,
                previous: { try? previousDate(date: $0) })
            if day != before { days[id] = day }
            if let back { controller.setDate(back.date, for: id) }
        }
    }

    // MARK: - Satellite days

    /// The days a satellite layer's picker offers, newest first.
    func satelliteDays() -> [MapDate] {
        let newest = gibsDefaultDate(now: clock())
        if let held = pickerDays, held.newest == newest { return held.days }
        let list = SatelliteDay.days(from: newest, previous: { try? previousDate(date: $0) })
        pickerDays = (newest, list)
        return list
    }

    /// Shows `day` on a satellite layer (it still steps back past a day without pictures).
    func pick(_ day: MapDate, for id: String) {
        days[id] = SatelliteDay(chosen: day, picked: true)
        controller?.setDate(day.date, for: id)
        checkReports()
    }

    // MARK: - Grid times

    /// The times of the layer's dataset, oldest first; none until its axis is in.
    func times(of layer: PlannedLayer) -> [LayerTime]? {
        times[TimeAxes.Key(layer: layer.id, dataset: layer.dataset)]
    }

    private func askTimes(_ key: TimeAxes.Key, dates: LayerDates, now: Int64) {
        guard let controller, !asking.contains(key) else { return }
        let json = controller.settingsJSON
        // A layer still without its axis asks again sooner.
        let every = times[key] == nil ? TimeAxes.retryAfter : Self.axisCheck
        if let last = asked[key], last.json == json, now >= last.at, now - last.at < every {
            return
        }
        asking.insert(key)
        let zone = TimeZone.current.identifier
        let axes = axes
        Task { [weak self] in
            let found = await axes.times(key, dates: dates, settingsJSON: json, timeZone: zone)
            guard let self else { return }
            self.asking.remove(key)
            self.asked[key] = (json, self.clock())
            if let found, self.times[key] != found { self.times[key] = found }
            self.scheduleSync()
            // No axis yet (offline, or the server busy): look again once the pause is over,
            // without waiting for the map to move.
            if found == nil { self.syncLater(after: TimeAxes.retryAfter) }
        }
    }

    /// Looks at the layers again in `seconds`, if the module is still there.
    private func syncLater(after seconds: Int64) {
        Task { [weak self] in
            try? await Task.sleep(for: .seconds(Int(seconds)))
            self?.scheduleSync()
        }
    }

    /// Shows the time `time` (none: the newest) on a grid layer.
    func pick(_ time: Int64?, for id: String) {
        searchGeneration += 1
        clearSearches[id] = nil
        controller?.setTime(time, for: id)
    }

    /// Whether "last clear day" can look on `layer` now: a daily raster with its axis in, drawn
    /// at this zoom.
    func canFindClearDay(_ layer: PlannedLayer) -> Bool {
        guard layer.kind == .gridRaster, layer.info.dates == .daily, times(of: layer) != nil,
            let region = controller?.region
        else { return false }
        return region.zoom >= Double(layer.info.minZoom ?? 0)
    }

    /// Asks for the newest days of `layer` in turn until the view has values: the last clear
    /// day there.
    func findClearDay(_ layer: PlannedLayer) {
        guard canFindClearDay(layer), let axis = times(of: layer) else { return }
        searchGeneration += 1
        let generation = searchGeneration
        let id = layer.id
        clearSearches[id] = .looking
        Task { [weak self] in
            for time in GridTimes.clearDayTries(axis) {
                guard let self, self.searchGeneration == generation,
                    let controller = self.controller,
                    let current = controller.plan.layers.first(where: { $0.id == id })
                else { return }
                let onScreen = current.time == time
                controller.setTime(time, for: id)
                var watch = ClearDayWatch(
                    revision: controller.gridRevisions[id] ?? 0, arrived: onScreen)
                var verdict = ClearDayWatch.Verdict.waiting
                while verdict == .waiting {
                    try? await Task.sleep(for: ClearDayWatch.interval)
                    guard self.searchGeneration == generation else { return }
                    let values = await self.hasValues(id)
                    verdict = watch.check(
                        hasValues: values, revision: controller.gridRevisions[id] ?? 0)
                }
                switch verdict {
                case .clear:
                    self.clearSearches[id] = .found
                    return
                case .cloudy:
                    continue
                case .waiting, .unknown:
                    self.clearSearches[id] = ClearSearch.none
                    return
                }
            }
            guard let self, self.searchGeneration == generation else { return }
            self.clearSearches[id] = ClearSearch.none
        }
    }

    /// Whether the layer's blocks for the time asked for have values in view.
    private func hasValues(_ id: String) async -> Bool {
        guard let controller, let region = controller.region else { return false }
        let store = controller.gridStore
        // The store scans its blocks outside the main thread.
        return await Task.detached(priority: .userInitiated) {
            let range = try? store.visibleRange(
                layer: id, west: region.west, south: region.south, east: region.east,
                north: region.north, zoom: region.zoom)
            return range != nil
        }.value
    }

    // MARK: - The radar loop

    /// Whether the loop may play: not in the battery-friendly mode.
    var canPlay: Bool { !(controller?.battery ?? true) && frames.count > 1 }

    private var radarInfo: LayerInfo? {
        controller?.catalogue.first { $0.id == RadarFrames.layer }
    }

    /// The radar's opacity as the settings have it.
    private var radarOpacity: CGFloat {
        CGFloat(radarInfo?.opacity ?? 0.75)
    }

    private func syncRadar(now: Int64) {
        guard let controller else { return }
        guard controller.plan.layers.contains(where: { $0.id == RadarFrames.layer }) else {
            stopLoop()
            return
        }
        if controller.battery { pause() }
        let json = controller.settingsJSON
        let due =
            framesAsked.map { $0.json != json || RadarLoopRules.isDue(loaded: $0.at, now: now) }
            ?? true
        if due, !loadingFrames { loadFrames(json: json, now: now) }
        // The drawer saving an opacity lets the map's radar overlay show again: hold it back.
        if loopShown, controller.previewOpacities[RadarFrames.layer] == nil {
            controller.previewOpacity(RadarFrames.layer, 0)
            holdsRadar = true
        }
        updateAlphas()
    }

    private func loadFrames(json: String, now: Int64) {
        loadingFrames = true
        framesAsked = (json, now)
        let zone = TimeZone.current.identifier
        let radar = radar
        Task { [weak self] in
            let found = await radar.frames(settingsJSON: json, timeZone: zone)
            guard let self else { return }
            self.loadingFrames = false
            guard let found, found != self.frames else { return }
            let sameTimes = found.map(\.valid) == self.frames.map(\.valid)
            self.frames = found
            guard !sameTimes else { return }
            // A newer composite: the loop starts over, on the latest frame.
            self.frameIndex = found.count - 1
            if self.loopShown { self.makeFrameOverlays() }
        }
    }

    /// Shows frame `index` of the loop, putting the frames on the map first.
    func showFrame(_ index: Int) {
        guard !frames.isEmpty else { return }
        frameIndex = min(max(index, 0), frames.count - 1)
        if !loopShown {
            loopShown = true
            makeFrameOverlays()
        }
        updateAlphas()
    }

    func play() {
        guard canPlay else { return }
        if !loopShown { showFrame(frameIndex) }
        playing = true
        // From the latest frame the loop starts at the oldest; elsewhere it goes on from there.
        loopStep =
            frameIndex == frames.count - 1 ? frames.count + RadarLoopRules.hold - 1 : frameIndex
        playTask?.cancel()
        playTask = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: RadarLoopRules.tick)
                guard !Task.isCancelled, let self, self.playing else { return }
                let next = RadarLoopRules.next(after: self.loopStep, count: self.frames.count)
                self.loopStep = next.step
                self.frameIndex = next.index
                self.updateAlphas()
            }
        }
    }

    func pause() {
        playing = false
        playTask?.cancel()
        playTask = nil
    }

    private func makeFrameOverlays() {
        guard let controller, let info = radarInfo else { return }
        frameOverlays = frames.map {
            RadarFrameOverlay(
                valid: $0.valid, minZoom: Int(info.minZoom ?? 0),
                deepest: Int(info.maxZoom ?? UInt8(MapPlan.deepestZoom)), frames: radar)
        }
        frameRenderers = [:]
        if !holdsRadar {
            controller.previewOpacity(RadarFrames.layer, 0)
            holdsRadar = true
        }
        controller.moduleDidChange()
    }

    private func stopLoop() {
        pause()
        if loopShown {
            loopShown = false
            frameOverlays = []
            frameRenderers = [:]
            controller?.moduleDidChange()
        }
        if holdsRadar {
            holdsRadar = false
            controller?.previewOpacity(RadarFrames.layer, nil)
        }
    }

    private func updateAlphas() {
        let opacity = radarOpacity
        for (i, overlay) in frameOverlays.enumerated() {
            frameRenderers[ObjectIdentifier(overlay)]?.alpha = i == frameIndex ? opacity : 0
        }
    }
}

/// One frame of the radar loop as a MapKit tile overlay: its tiles come from ``RadarFrames``,
/// cut from the radar's deepest tiles past its zoom (MapKit asks for nothing deeper than
/// `maximumZ`, so the overlay over-zooms by itself).
nonisolated final class RadarFrameOverlay: MKTileOverlay {
    let valid: Int64
    private let deepest: Int
    private let frames: RadarFrames

    init(valid: Int64, minZoom: Int, deepest: Int, frames: RadarFrames) {
        self.valid = valid
        self.deepest = deepest
        self.frames = frames
        super.init(urlTemplate: nil)
        tileSize = CGSize(width: Int(mapTileSize()), height: Int(mapTileSize()))
        minimumZ = minZoom
        maximumZ = MapPlan.deepestZoom
        canReplaceMapContent = false
    }

    override func loadTile(
        at path: MKTileOverlayPath, result: @escaping (Data?, (any Error)?) -> Void
    ) {
        let reply = TileReply(result: result)
        let (valid, deepest, frames) = (valid, deepest, frames)
        let (z, x, y) = (path.z, path.x, path.y)
        Task.detached(priority: .userInitiated) {
            reply.send(await frames.tile(valid: valid, z: z, x: x, y: y, deepest: deepest))
        }
    }
}
