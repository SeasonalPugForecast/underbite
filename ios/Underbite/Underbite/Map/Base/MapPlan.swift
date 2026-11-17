import Foundation
import UnderbiteKit

/// What the map draws, worked out from the core's layer catalogue and the map settings: the base
/// map, the raster and arrow layers bottom to top, the credits of everything on screen and whether
/// the "Not for navigation." banner shows. Every word in it comes from the catalogue; nothing here
/// touches MapKit, so the order and the battery rule are tested on their own.
nonisolated struct MapPlan: Equatable, Sendable {
    /// The chosen base map.
    var base: LayerInfo?
    /// The layers the map view draws itself (tiles, rendered grids, arrows), bottom first; the
    /// NOAA chart, when it is the base, is the first.
    var layers: [PlannedLayer] = []
    /// Every layer on screen, the base first, then the overlays bottom to top: what the credits
    /// and the modules (stations, spots) go by.
    var shown: [LayerInfo] = []
    /// The raster layers switched on but left off by the battery-friendly mode, bottom first.
    var held: [String] = []
    /// The credit lines of the layers on screen, each once, in drawing order (safety text).
    var credits: [String] = []
    /// "Not for navigation." while a chart, current or depth layer is on screen (safety text).
    var notice: String?
    /// The topmost grid layer on screen with a legend.
    var legend: LayerInfo?
    /// Whether the battery-friendly mode is on: at most ``maxRastersOnBattery`` raster layers and
    /// no animation.
    var battery = false

    /// The most raster layers the battery-friendly mode draws.
    static let maxRastersOnBattery = 2

    /// The deepest zoom MapKit asks tiles for. Tile overlays declare it as their `maximumZ`
    /// whatever their source has: MapKit requests nothing deeper than `maximumZ`, so a source with
    /// fewer levels is over-zoomed by the overlay itself (the core's `tile_request` names the
    /// ancestor and the part of it to draw).
    static let deepestZoom = 22

    /// Whether the battery-friendly mode is on for the settings' `battery_mode` (`auto` follows
    /// Low Power Mode).
    static func batteryOn(setting: String, lowPower: Bool) -> Bool {
        switch setting {
        case "on": true
        case "off": false
        default: lowPower
        }
    }

    /// Whether a layer is a raster for the battery-friendly mode's count.
    static func isRaster(_ draw: LayerDraw) -> Bool {
        draw == .tiles || draw == .gridRaster
    }

    /// The plan for `catalogue` (the core's `map_catalogue`, which knows what is shown and at
    /// what opacity) in the order of `settings.layers` (first at the bottom; catalogue layers the
    /// settings do not list yet go on top in catalogue order).
    ///
    /// - Parameters:
    ///   - dates: per tiled layer, the day to ask for (`YYYY-MM-DD`); none asks for the newest.
    ///   - times: per grid layer, the time to ask for (Unix seconds); none asks for the newest.
    ///   - datasets: per grid layer, the dataset to ask for; none asks for the primary.
    ///   - opacities: per layer, an opacity being previewed, over the settings'.
    static func make(
        catalogue: [LayerInfo], settings: Settings.MapSettings, battery: Bool,
        dates: [String: String] = [:], times: [String: Int64] = [:],
        datasets: [String: String] = [:], opacities: [String: Double] = [:]
    ) -> MapPlan {
        var plan = MapPlan(battery: battery)
        plan.base = catalogue.first { $0.isBase && $0.shown }
        let chartBase = plan.base?.draw == .tiles
        let position = Dictionary(
            settings.layers.enumerated().map { ($0.element.id, $0.offset) },
            uniquingKeysWith: { first, _ in first })
        let overlays = catalogue.enumerated()
            .filter { !$0.element.isBase && $0.element.shown && $0.element.available }
            .sorted {
                let a = position[$0.element.id] ?? settings.layers.count + $0.offset
                let b = position[$1.element.id] ?? settings.layers.count + $1.offset
                return a < b
            }
            .map(\.element)
        // The battery-friendly mode keeps the topmost rasters.
        var kept: [LayerInfo] = []
        var rasters = 0
        for layer in overlays.reversed() {
            if battery, isRaster(layer.draw) {
                rasters += 1
                if rasters > maxRastersOnBattery {
                    plan.held.insert(layer.id, at: 0)
                    continue
                }
            }
            kept.insert(layer, at: 0)
        }
        plan.shown = (plan.base.map { [$0] } ?? []) + kept
        let stored = Dictionary(
            settings.layers.map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })
        for layer in plan.shown {
            guard let kind = PlannedLayer.Kind(layer.draw) else { continue }
            plan.layers.append(
                PlannedLayer(
                    info: layer, kind: kind,
                    opacity: opacities[layer.id] ?? Double(layer.opacity),
                    ramp: layer.ramp,
                    range: stored[layer.id]?.range.flatMap { $0.count == 2 ? $0 : nil },
                    date: kind == .tiles ? dates[layer.id] : nil,
                    time: kind == .tiles ? nil : times[layer.id],
                    dataset: kind == .tiles ? nil : datasets[layer.id],
                    aboveLabels: chartBase))
        }
        for layer in plan.shown where !layer.credit.isEmpty && !plan.credits.contains(layer.credit) {
            plan.credits.append(layer.credit)
        }
        plan.notice = plan.shown.lazy.compactMap(\.notice).first
        plan.legend = kept.last { $0.draw == .gridRaster && !$0.legend.isEmpty }
        return plan
    }
}

/// A layer the map view draws itself.
nonisolated struct PlannedLayer: Equatable, Sendable {
    enum Kind: Equatable, Sendable {
        /// Tiles from a service (`tile_request`).
        case tiles
        /// A grid rendered into tiles on the device (`GridStore.render_tile`).
        case gridRaster
        /// A grid drawn as arrows (`GridStore.arrows`).
        case gridArrows

        init?(_ draw: LayerDraw) {
            switch draw {
            case .tiles: self = .tiles
            case .gridRaster: self = .gridRaster
            case .gridArrows: self = .gridArrows
            default: return nil
            }
        }
    }

    let info: LayerInfo
    let kind: Kind
    /// 0 to 1.
    var opacity: Double
    /// The colour ramp of a grid layer.
    var ramp: String?
    /// The settings' value range, low and high in canonical units; none for the layer's own.
    var range: [Double]?
    var date: String?
    var time: Int64?
    var dataset: String?
    /// Drawn above the base map's labels: over the NOAA chart, which carries its own labels, so
    /// every layer sits above it; otherwise between the roads and the labels.
    var aboveLabels: Bool

    var id: String { info.id }

    /// What identifies the layer's overlay on the map: when it changes, the overlay is replaced.
    /// The opacity is not part of it (the renderer's alpha changes in place).
    var overlayKey: String {
        let parts: [String] = [
            info.id, date ?? "", time.map { String($0) } ?? "", dataset ?? "", ramp ?? "",
            range.map { $0.map { String($0) }.joined(separator: ",") } ?? "",
            aboveLabels ? "labels" : "roads",
        ]
        return parts.joined(separator: "|")
    }
}

/// How to bring the overlays on the map to a wanted order with few changes.
nonisolated enum OverlayOrder {
    /// The overlays to remove from `current` and those to insert, each above the one named (none:
    /// at the bottom of the layers the map view draws), in insertion order, so that the map ends
    /// up with `wanted`, bottom first. Overlays kept in the same relative order stay; a change of
    /// order removes and inserts the kept ones again.
    static func steps(current: [String], wanted: [String]) -> (remove: [String], insert: [(key: String, above: String?)]) {
        let wantedSet = Set(wanted)
        let kept = current.filter { wantedSet.contains($0) }
        let keptSet = Set(kept)
        let inOrder = kept == wanted.filter { keptSet.contains($0) }
        let remove = inOrder ? current.filter { !wantedSet.contains($0) } : current
        let present = inOrder ? keptSet : []
        var insert: [(key: String, above: String?)] = []
        for (i, key) in wanted.enumerated() where !present.contains(key) {
            insert.append((key, i == 0 ? nil : wanted[i - 1]))
        }
        return (remove, insert)
    }
}

/// The part of the world the map shows, in degrees, and its zoom.
nonisolated struct MapRegion: Equatable, Sendable {
    var west: Double
    var south: Double
    var east: Double
    var north: Double
    /// The Web Mercator zoom at which a 256-point tile is drawn 256 points wide.
    var zoom: Double

    var centerLatitude: Double { (south + north) / 2 }
    var centerLongitude: Double { (west + east) / 2 }

    /// MapKit's world width in map points (`MKMapSize.world.width`): 256 × 2²⁰.
    static let worldMapPoints = 268_435_456.0

    /// The zoom of a view `viewPoints` wide that shows `mapPoints` of MapKit's map width.
    static func zoom(viewPoints: Double, mapPoints: Double) -> Double {
        guard viewPoints > 0, mapPoints > 0 else { return 0 }
        return log2(viewPoints / mapPoints * worldMapPoints / 256)
    }
}
