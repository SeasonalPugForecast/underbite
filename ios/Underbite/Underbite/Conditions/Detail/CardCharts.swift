import UnderbiteKit

/// One chart of a card's full form with what goes with it.
struct CardChartBlock: Identifiable {
    var id: String
    var chart: ViewModel.Chart
    var band: ChartValueBand?
    var notes: [String] = []
    /// Labelled values under the chart (the moon's illumination, the percentile thresholds).
    var facts: [ChartFact] = []
}

/// Which charts each card carries, and the words that sit under them. Only the view model's text
/// is used: the datum a tide height is measured from, the moon phase, the percentile
/// thresholds with their basis.
enum CardCharts {
    /// `settingsJSON` gives the core's words their voice; the shipped defaults when it is not at hand.
    static func blocks(for card: ViewModel.Card, settingsJSON: String = defaultSettingsJson())
        -> [CardChartBlock]
    {
        switch card {
        case .tide(let c): tide(c.full)
        case .current(let c): single("current", c.full.chart)
        case .flow(let c): flow(c.full, settingsJSON: settingsJSON)
        case .waterTemp(let c): single("water_temp", c.full.chart)
        case .clarity(let c): single("clarity", c.full.chart)
        case .waves(let c):
            single("wave_height", c.full.heightChart) + single("wave_period", c.full.periodChart)
        case .wind(let c): single("wind", c.full.chart)
        case .pressure(let c): single("pressure", c.full.chart)
        default: []
        }
    }

    private static func single(
        _ id: String, _ chart: ViewModel.Chart?, notes: [String] = [], facts: [ChartFact] = []
    ) -> [CardChartBlock] {
        chart.map { [CardChartBlock(id: id, chart: $0, notes: notes, facts: facts)] } ?? []
    }

    /// Heights show their datum ("above MLLW"); a modelled curve has no heights and no datum, and
    /// its series carries the view's label of what it is.
    private static func tide(_ full: ViewModel.TideFull) -> [CardChartBlock] {
        var notes: [String] = []
        if full.heightsShown, let datum = full.datumLabel { notes.append(datum) }
        var facts: [ChartFact] = []
        if let moon = full.days.compactMap(\.moon).first {
            facts.append(ChartFact(label: moon.label, value: moon.illumination))
        }
        return single("tide", full.chart, notes: notes, facts: facts)
    }

    /// The percentile band for the date goes on the chart whose unit it is in (discharge, or the
    /// level of a lake); the flood categories are the rules the view put on the stage chart.
    private static func flow(_ full: ViewModel.FlowFull, settingsJSON: String) -> [CardChartBlock] {
        let percentile = full.percentile
        func block(_ id: String, _ chart: ViewModel.Chart) -> CardChartBlock {
            guard let percentile,
                let band = band(percentile, in: chart, settingsJSON: settingsJSON)
            else {
                return CardChartBlock(id: id, chart: chart)
            }
            return CardChartBlock(
                id: id, chart: chart, band: band, notes: [percentile.basis, percentile.sample],
                facts: thresholds(percentile))
        }
        return [("flow", full.chart), ("stage", full.stageChart)].compactMap { id, chart in
            chart.map { block(id, $0) }
        }
    }

    /// The widest span of thresholds the view gives, when it is in the chart's unit.
    private static func band(
        _ percentile: ViewModel.Percentile, in chart: ViewModel.Chart, settingsJSON: String
    ) -> ChartValueBand? {
        guard let low = percentile.thresholds.first, let high = percentile.thresholds.last,
            let lowValue = low.value.value, let highValue = high.value.value,
            lowValue < highValue, low.value.unit == chart.unit, high.value.unit == chart.unit
        else { return nil }
        // The shaded span is the widest the view gives (5th to 95th percentile), not the
        // "normal" class of the headline, so it carries the core's own label for it.
        let label =
            (try? percentileBandLabel(
                low: low.label, high: high.label, settingsJson: settingsJSON)) ?? ""
        let all = percentile.thresholds.compactMap { threshold -> ChartThreshold? in
            guard threshold.value.unit == chart.unit, let value = threshold.value.value
            else { return nil }
            return ChartThreshold(value: value, label: "\(threshold.label) \(threshold.value.text)")
        }
        return ChartValueBand(low: lowValue, high: highValue, label: label, thresholds: all)
    }

    /// One row per threshold ("5th percentile", "0.6 cfs"), in the view's order.
    private static func thresholds(_ percentile: ViewModel.Percentile) -> [ChartFact] {
        percentile.thresholds.map { ChartFact(label: $0.label, value: $0.value) }
    }
}
