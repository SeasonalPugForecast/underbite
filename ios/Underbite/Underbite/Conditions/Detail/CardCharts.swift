import UnderbiteKit

/// One chart of a card's full form with what goes with it.
struct CardChartBlock: Identifiable {
    var id: String
    var chart: ViewModel.Chart
    var band: ChartValueBand?
    var notes: [String] = []
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

    private static func single(_ id: String, _ chart: ViewModel.Chart?, notes: [String] = [])
        -> [CardChartBlock]
    {
        chart.map { [CardChartBlock(id: id, chart: $0, notes: notes)] } ?? []
    }

    /// Heights show their datum ("above MLLW"); a modelled curve has no heights and no datum, and
    /// its series carries the view's label of what it is.
    private static func tide(_ full: ViewModel.TideFull) -> [CardChartBlock] {
        var notes: [String] = []
        if full.heightsShown, let datum = full.datumLabel { notes.append(datum) }
        if let moon = full.days.compactMap(\.moon).first {
            notes.append("\(moon.label) \(moon.illumination.text)\(moon.illumination.unit)")
        }
        return single("tide", full.chart, notes: notes)
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
                id: id, chart: chart, band: band, notes: thresholds(percentile))
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
        let all = percentile.thresholds.compactMap { threshold in
            threshold.value.unit == chart.unit ? threshold.value.value : nil
        }
        return ChartValueBand(low: lowValue, high: highValue, label: label, thresholds: all)
    }

    /// "5th percentile 0.6 cfs · 50th percentile 15.0 cfs · …", then where the numbers come from.
    private static func thresholds(_ percentile: ViewModel.Percentile) -> [String] {
        let list = percentile.thresholds
            .map { "\($0.label) \($0.value.text) \($0.value.unit)" }
            .joined(separator: " · ")
        var lines: [String] = []
        if !list.isEmpty { lines.append(list) }
        lines.append("\(percentile.basis) · \(percentile.sample)")
        return lines
    }
}
