import UnderbiteKit

/// Which x-axis ticks to label. Ticks are thinned by position only, never relabelled.
enum ChartTickThinning {
    /// A rough width of one label character at the axis font, and the least gap between labels.
    static let characterWidth: Double = 7.5
    static let gap: Double = 10

    /// A tick that names a day: major, with a label that starts with a word, not an hour number.
    static func isDayMark(_ tick: ViewModel.ChartTick) -> Bool {
        tick.major && tick.label.first.map { !$0.isNumber } == true
    }

    /// The ticks to label, in time order. Day marks go first, then the other major ticks, then
    /// the minor ones, each kept only where its label clears every label already kept.
    static func thinned(
        _ ticks: [ViewModel.ChartTick], domain: ClosedRange<Double>, plotWidth: Double
    ) -> [ViewModel.ChartTick] {
        let span = max(domain.upperBound - domain.lowerBound, 1)
        func interval(_ tick: ViewModel.ChartTick) -> ClosedRange<Double> {
            let centre = (Double(tick.t) - domain.lowerBound) / span * plotWidth
            let half = Double(tick.label.count) * characterWidth / 2 + gap / 2
            return (centre - half)...(centre + half)
        }
        var kept: [(tick: ViewModel.ChartTick, box: ClosedRange<Double>)] = []
        let passes: [(ViewModel.ChartTick) -> Bool] = [
            { isDayMark($0) }, { $0.major && !isDayMark($0) }, { !$0.major },
        ]
        for pass in passes {
            for tick in ticks where pass(tick) {
                let box = interval(tick)
                if !kept.contains(where: { $0.box.overlaps(box) }) { kept.append((tick, box)) }
            }
        }
        return kept.map(\.tick).sorted { $0.t < $1.t }
    }
}
