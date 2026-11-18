import Foundation
@preconcurrency import UnderbiteKit

/// What of the depth layers is shown, at what opacity, and from which zoom on: the catalogue's
/// `depth-shading` and `depth-contours` as the map has them on screen.
nonisolated struct DepthShown: Equatable, Sendable {
    /// The catalogue ids of the shading and the contours.
    static let shadingID = "depth-shading"
    static let contoursID = "depth-contours"
    static let ids: Set<String> = [shadingID, contoursID]

    var shading = false
    var contours = false
    var shadingOpacity = 1.0
    var contourOpacity = 1.0
    /// The shallowest zoom the catalogue draws the shown layers at; none for any zoom.
    var minZoom: UInt8?

    var any: Bool { shading || contours }

    init() {}

    /// From the module's layers on screen; other layers are passed over.
    init(_ layers: [LayerInfo]) {
        for layer in layers {
            switch layer.id {
            case Self.shadingID:
                shading = true
                shadingOpacity = Double(layer.opacity)
            case Self.contoursID:
                contours = true
                contourOpacity = Double(layer.opacity)
            default:
                continue
            }
            if let zoom = layer.minZoom { minZoom = min(minZoom ?? zoom, zoom) }
        }
    }
}
