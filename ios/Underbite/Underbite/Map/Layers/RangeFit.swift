import Foundation
import UnderbiteKit

/// Keeps the colour range of a gridded layer steady while the map is panned: a new fitted range
/// replaces the one drawn only when it differs enough to see, so that the tiles are not drawn
/// again for a change of a few percent.
nonisolated enum RangeFit {
    /// The share of the held range's span that either end may move before the range is replaced.
    static let tolerance = 0.1

    /// The fitted range to draw: `new` unless it is within the tolerance of `held`; `held` when
    /// there is no new one (nothing of the layer is in view yet, so the last range stays).
    static func settle(new: ValueRange?, held: ValueRange?) -> ValueRange? {
        guard let new else { return held }
        guard let held else { return new }
        let span = max(held.high - held.low, Double.leastNormalMagnitude)
        let steady =
            abs(new.low - held.low) <= tolerance * span
            && abs(new.high - held.high) <= tolerance * span
        return steady ? held : new
    }
}

/// What a raster layer's legend shows: the colour bar for the range drawn and, when that range is
/// the one fitted to the view, the core's words for it.
nonisolated struct MapLegendState: Equatable, Sendable {
    var stops: [LegendStop]
    var fittedCaption: String?
}
