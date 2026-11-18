import Foundation
@preconcurrency import UnderbiteKit

/// What of the breaks layers is shown: the catalogue's `sst-breaks` and `chlorophyll-breaks` as
/// the map has them on screen, each with its opacity, its shallowest zoom, the grid layer its
/// breaks are found in and whether the map draws that layer now (the breaks need its grid: they
/// ask nothing of their own).
nonisolated struct BreaksShown: Equatable, Sendable {
    /// One breaks layer on screen.
    struct Layer: Equatable, Sendable {
        let id: String
        /// The grid layer the breaks are found in (`sst`).
        let source: String
        var opacity: Double
        var minZoom: UInt8?
        /// Whether the map draws the grid layer now (shown, available, not held back by the
        /// battery-friendly mode).
        var sourceDrawn: Bool
        /// What the grid layer is drawn with: its overlay key (day, dataset, ramp), which
        /// changes when another day is picked.
        var sourceKey: String?
    }

    /// Bottom first, as the map has them.
    var layers: [Layer] = []

    var any: Bool { !layers.isEmpty }

    init() {}

    /// From the module's layers on screen (`layers`), the grid layers the map draws (`drawn`)
    /// and each breaks layer's grid layer (`sources`, by breaks layer id, from the core's
    /// `breaks_words`). Layers that are not breaks layers, or whose grid layer is not known, are
    /// passed over.
    init(_ layers: [LayerInfo], drawn: [PlannedLayer], sources: [String: String]) {
        for layer in layers where layer.draw == .breaks {
            guard let source = sources[layer.id] else { continue }
            let planned = drawn.first { $0.id == source && $0.kind == .gridRaster }
            self.layers.append(
                Layer(
                    id: layer.id, source: source, opacity: Double(layer.opacity),
                    minZoom: layer.minZoom, sourceDrawn: planned != nil,
                    sourceKey: planned?.overlayKey))
        }
    }

    /// The layer with `id`, if shown.
    func layer(_ id: String) -> Layer? {
        layers.first { $0.id == id }
    }

    /// The layers whose breaks can be looked for: their grid layer is drawn.
    var findable: [Layer] { layers.filter(\.sourceDrawn) }
}
