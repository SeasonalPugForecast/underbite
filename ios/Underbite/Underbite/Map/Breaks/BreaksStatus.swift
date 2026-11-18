import Foundation
@preconcurrency import UnderbiteKit

/// What a breaks layer says beside or in place of its lines: its grid layer is not on the map,
/// the view is too wide, the grid is still coming, nothing is as sharp as the threshold, or
/// clouds hid most of the day. None while breaks are drawn. The words are the core's
/// (`breaks_words` and the `BreakSet` of `GridStore.breaks`).
nonisolated enum BreaksStatus: Equatable, Sendable {
    case needsSource
    case zoomIn
    case loading
    case noBreaks
    case fewClearCells

    /// How far short of a layer's shallowest zoom a view may be and still look for breaks: the
    /// core says whether the view's blocks are fine enough.
    static let zoomSlack = 0.25

    /// Whether a view at `zoom` is deep enough for a layer drawn from `minZoom` on.
    static func deepEnough(zoom: Double, minZoom: UInt8?) -> Bool {
        guard let minZoom else { return true }
        return zoom >= Double(minZoom) - zoomSlack
    }

    /// The status of `layer` in a view at `zoom` (none before the map has settled) with the
    /// breaks last found for it (none while they are looked for).
    static func of(_ layer: BreaksShown.Layer, zoom: Double?, set: BreakSet?) -> BreaksStatus? {
        guard layer.sourceDrawn else { return .needsSource }
        if let zoom, !deepEnough(zoom: zoom, minZoom: layer.minZoom) { return .zoomIn }
        guard let set else { return .loading }
        switch set.state {
        case .drawn: return nil
        case .noBreaks: return .noBreaks
        case .fewClearCells: return .fewClearCells
        case .zoomIn: return .zoomIn
        case .waiting: return .loading
        }
    }

    /// The core's words for it.
    func text(_ words: BreaksWords, layer: BreakLayerWords, set: BreakSet?) -> String {
        switch self {
        case .needsSource: layer.needsSource
        case .zoomIn: words.zoomIn
        case .loading: words.loading
        case .noBreaks, .fewClearCells: set?.stateText ?? words.loading
        }
    }
}
