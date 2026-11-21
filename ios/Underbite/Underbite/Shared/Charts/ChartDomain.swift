import Foundation

/// The value axis of a chart worked out from its numbers. Foundation only, so the package tests
/// cover it; it writes no words.
nonisolated enum ChartDomain {
    /// The value-axis domain: the data (and the rules near it), widened to the nearest threshold
    /// below and the nearest above when there are any, then padded by 12 %. Without thresholds it
    /// is the data alone, padded.
    static func y(values: [Double], rules: [Double], thresholds: [Double]) -> ClosedRange<Double> {
        let all = values + rules
        var low = all.min() ?? 0
        var high = all.max() ?? low + 1
        if let below = thresholds.filter({ $0 <= low }).max() { low = below }
        if let above = thresholds.filter({ $0 >= high }).min() { high = above }
        let pad = max(high - low, 1e-6) * 0.12
        return (low - pad)...(high + pad)
    }

    /// Whether a shaded span reaches into the domain by one of its own edges. A span that covers
    /// the whole axis says nothing and is not drawn.
    static func bandShows(low: Double, high: Double, in domain: ClosedRange<Double>) -> Bool {
        domain.contains(low) || domain.contains(high)
    }
}
