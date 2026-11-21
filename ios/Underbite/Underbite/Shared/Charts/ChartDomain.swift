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

    /// The most of the axis a shaded span may cover and still be filled.
    static let widestFill = 0.85

    /// Whether a shaded span is filled: one of its own edges is inside the domain and it does not
    /// cover (nearly) the whole axis, where a fill says nothing.
    static func fills(low: Double, high: Double, in domain: ClosedRange<Double>) -> Bool {
        guard domain.contains(low) || domain.contains(high) else { return false }
        let covered = min(high, domain.upperBound) - max(low, domain.lowerBound)
        return covered < widestFill * (domain.upperBound - domain.lowerBound)
    }

    /// Which rules (by index) get their words: from the top down, a rule is named only when it is
    /// at least `gap` of the axis below the last one named, so labels never print over each other.
    static func labelled(_ values: [Double], in domain: ClosedRange<Double>, gap: Double = 0.07)
        -> Set<Int>
    {
        let span = domain.upperBound - domain.lowerBound
        var named = Set<Int>()
        var last: Double?
        for index in values.indices.sorted(by: { values[$0] > values[$1] }) {
            if let last, last - values[index] < gap * span { continue }
            named.insert(index)
            last = values[index]
        }
        return named
    }
}
