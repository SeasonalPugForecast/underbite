import Foundation
@preconcurrency import UnderbiteKit

/// What the depth layers say in place of depth: the view is too far out (shallower than the
/// layers' zoom, or wider than one elevation area), the elevation is being fetched, or it could
/// not be. None while depth is drawn. The words are the core's (`depth_words`).
nonisolated enum DepthStatus: Equatable, Sendable {
    case zoomIn
    case loading
    case failed

    /// How far short of a layer's shallowest zoom a view may be and still draw it: a view a
    /// little wider than the layer's first zoom step still counts as at it.
    static let zoomSlack = 0.25

    /// Whether a view at `zoom` is deep enough for layers drawn from `minZoom` on.
    static func deepEnough(zoom: Double, minZoom: UInt8?) -> Bool {
        guard let minZoom else { return true }
        return zoom >= Double(minZoom) - zoomSlack
    }

    /// The status for a view at `zoom` (none before the map has settled), the layers'
    /// shallowest zoom and where the elevation stands.
    static func of(zoom: Double?, minZoom: UInt8?, loader: DepthLoader.State) -> DepthStatus? {
        if let zoom, !deepEnough(zoom: zoom, minZoom: minZoom) { return .zoomIn }
        switch loader {
        case .tooWide: return .zoomIn
        case .loading: return .loading
        case .failed: return .failed
        case .idle, .ready: return nil
        }
    }

    /// The core's words for it.
    func text(_ words: DepthWords) -> String {
        switch self {
        case .zoomIn: words.zoomIn
        case .loading: words.loading
        case .failed: words.failed
        }
    }
}
