import Charts
import SwiftUI
import UnderbiteKit

/// A span the view asks to be shaded on the value axis, with its words (the percentile band of
/// the day of the year on a flow chart). Values are in the chart's own display units.
struct ChartValueBand: Hashable {
    var low: Double
    var high: Double
    var label: String
    /// The thresholds the view gives for the day, in the same units: the value axis reaches the
    /// nearest of them below and above the data, so a quiet series is not stretched into noise,
    /// and each one inside the axis is drawn as a thin rule with the view's own words.
    var thresholds: [ChartThreshold] = []
}

/// One percentile threshold: its value on the axis and the view's label for it.
struct ChartThreshold: Hashable {
    var value: Double
    var label: String
}

/// What a chart's marks look like. Colours carry the series' role; the line style and weight
/// carry it too, so no meaning rests on colour alone. The pairs drawn together (predicted and
/// observed, a flood stage and the user's range) are blue against orange or red, which
/// colour-blind eyes tell apart.
enum ChartPalette {
    static func color(role: String) -> Color {
        switch role {
        case "observed": .orange
        case "forecast": .teal
        case "modelled": .gray
        default: .blue
        }
    }

    static func dash(role: String) -> [CGFloat] {
        switch role {
        case "forecast", "modelled": [6, 4]
        default: []
        }
    }

    static func width(role: String) -> CGFloat { role == "observed" ? 3 : 2 }

    /// A shaded span's fill; `strength` is 1, or more in sun mode.
    static func band(kind: String, strength: Double = 1) -> Color {
        switch kind {
        case "night": Color.indigo.opacity(0.14 * strength)
        case "slack": Color.green.opacity(0.18 * strength)
        case "solunar_major": Color.purple.opacity(0.18 * strength)
        case "solunar_minor": Color.purple.opacity(0.09 * strength)
        default: Color.yellow.opacity(0.18 * strength)
        }
    }

    static func rule(kind: String) -> Color {
        switch kind {
        case "flood_category": .red
        case "user_range": .blue
        default: .orange
        }
    }

    /// Rules differ by dash too: a flood stage long dashes, the user's range short ones.
    static func ruleDash(kind: String) -> [CGFloat] {
        kind == "user_range" ? [2, 3] : [5, 3]
    }

    static func symbol(marker kind: String) -> String? {
        switch kind {
        case "sunrise": "sunrise.fill"
        case "sunset": "sunset.fill"
        default: nil
        }
    }
}

/// The geometry of one chart, worked out once: the axes' domains, which ticks and which value
/// labels to draw. Nothing here writes a number or a time: every label is looked up in the view
/// model's ticks, points, rules and markers.
struct ChartGeometry {
    var xDomain: ClosedRange<Double>
    var yDomain: ClosedRange<Double>
    var xTicks: [ViewModel.ChartTick]
    /// The value-axis labels: the lowest and the highest point of the first series, in the
    /// view's own point labels.
    var yLabels: [Double: String]
    /// The rules near enough to the data to be drawn; the key lists every rule.
    var rules: [ViewModel.ChartRule]
    var band: ChartValueBand?
    /// The thresholds that fall inside the axis, drawn as rules, and the indices of those that
    /// have room for their words.
    var thresholdRules: [ChartThreshold]
    var thresholdLabelled: Set<Int>
    var drawsZero: Bool

    init(chart: ViewModel.Chart, compact: Bool, band: ChartValueBand?) {
        let points = chart.series.flatMap(\.points)
        let times =
            points.map { Double($0.t) } + chart.xTicks.map { Double($0.t) }
            + chart.bands.flatMap { [Double($0.startT), Double($0.endT)] }
        let t0 = times.min() ?? 0
        let t1 = times.max() ?? t0 + 1
        xDomain = t0...(t1 > t0 ? t1 : t0 + 1)
        // The view's ticks, thinned by position only (never relabelled) so no two labels collide:
        // the day marks and the major ticks always survive, the minor ones fill the gaps.
        xTicks = ChartTickThinning.thinned(
            chart.xTicks, domain: xDomain, plotWidth: compact ? 300 : 640)

        let values = points.map(\.value)
        let lo = values.min() ?? 0
        let hi = values.max() ?? lo + 1
        let span = max(hi - lo, abs(hi) * 0.25, 1e-6)
        // A rule far from the data would flatten it: it stays in the key below the chart.
        let near = chart.rules.filter { $0.value >= lo - span && $0.value <= hi + span }
        rules = near
        let domain = ChartDomain.y(
            values: values, rules: near.map(\.value), thresholds: (band?.thresholds ?? []).map(\.value))
        yDomain = domain
        drawsZero = domain.contains(0) && lo < 0
        let inside = (band?.thresholds ?? []).filter { domain.contains($0.value) }
        thresholdRules = inside
        thresholdLabelled = ChartDomain.labelled(inside.map(\.value), in: domain)

        var labels: [Double: String] = [:]
        if let first = chart.series.first,
            let least = first.points.min(by: { $0.value < $1.value }),
            let most = first.points.max(by: { $0.value < $1.value })
        {
            labels[least.value] = least.label
            labels[most.value] = most.label
        }
        yLabels = labels
        if let band, ChartDomain.fills(low: band.low, high: band.high, in: yDomain) {
            self.band = ChartValueBand(
                low: max(band.low, yDomain.lowerBound),
                high: min(band.high, yDomain.upperBound),
                label: band.label, thresholds: band.thresholds)
        } else {
            self.band = nil
        }
    }
}

/// One chart of the view model drawn with Swift Charts: the series, the shaded spans, the
/// rules, the marked events and now. Swift Charts only lays out; ticks and labels are the view
/// model's, supplied explicitly. A narrow layout takes the major ticks, a wide one all.
struct ChartPlot: View {
    let chart: ViewModel.Chart
    var valueBand: ChartValueBand?
    @Environment(\.horizontalSizeClass) private var sizeClass
    @Environment(\.sunMode) private var sunMode

    private var stroke: CGFloat { Palette.strokeScale(sunMode) }

    var body: some View {
        let geometry = ChartGeometry(
            chart: chart, compact: sizeClass != .regular, band: valueBand)
        let tickLabels = Dictionary(
            geometry.xTicks.map { (Double($0.t), $0.label) }, uniquingKeysWith: { first, _ in first })
        Chart {
            marks(geometry)
        }
        .chartXScale(domain: geometry.xDomain)
        .chartYScale(domain: geometry.yDomain)
        .chartXAxis {
            AxisMarks(values: geometry.xTicks.map { Double($0.t) }) { value in
                AxisGridLine()
                AxisTick()
                AxisValueLabel {
                    if let t = value.as(Double.self), let label = tickLabels[t] {
                        Text(label).font(.caption)
                    }
                }
            }
        }
        .chartYAxis {
            AxisMarks(position: .leading, values: Array(geometry.yLabels.keys)) { value in
                AxisGridLine()
                AxisValueLabel {
                    if let v = value.as(Double.self), let label = geometry.yLabels[v] {
                        Text(label).font(.caption)
                    }
                }
            }
        }
        // The plot's words stop growing at the largest standard size, so the curve keeps its
        // room; the key under it and the audio graph carry every value at any size.
        .dynamicTypeSize(...DynamicTypeSize.xxxLarge)
        .frame(height: sizeClass == .regular ? 340 : 240)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(Text(chart.accessibility))
        .accessibilityChartDescriptor(ChartAudioGraph(chart: chart, yDomain: geometry.yDomain))
    }

    @ChartContentBuilder private func marks(_ geometry: ChartGeometry) -> some ChartContent {
        ForEach(Array(chart.bands.enumerated()), id: \.offset) { _, band in
            RectangleMark(
                xStart: .value("From", Double(band.startT)),
                xEnd: .value("To", Double(band.endT))
            )
            .foregroundStyle(
                ChartPalette.band(kind: band.kind, strength: Palette.fillScale(sunMode)))
        }
        if let band = geometry.band {
            RectangleMark(
                xStart: .value("From", geometry.xDomain.lowerBound),
                xEnd: .value("To", geometry.xDomain.upperBound),
                yStart: .value("Low", band.low),
                yEnd: .value("High", band.high)
            )
            .foregroundStyle(Color.mint.opacity(0.16 * Palette.fillScale(sunMode)))
        }
        ForEach(Array(geometry.thresholdRules.enumerated()), id: \.offset) { index, threshold in
            RuleMark(y: .value("Threshold", threshold.value))
                .foregroundStyle(Color.mint.opacity(0.9))
                .lineStyle(StrokeStyle(lineWidth: 0.75 * stroke))
                .annotation(position: .top, alignment: .trailing, spacing: 0) {
                    if geometry.thresholdLabelled.contains(index) {
                        Text(threshold.label).font(.caption2).foregroundStyle(.readableSecondary)
                            .padding(.horizontal, 3)
                            .background(
                                Color(.secondarySystemGroupedBackground).opacity(0.9),
                                in: RoundedRectangle(cornerRadius: 3))
                    }
                }
        }
        if geometry.drawsZero {
            RuleMark(y: .value("Zero", 0.0)).foregroundStyle(.secondary.opacity(0.6))
        }
        ForEach(Array(geometry.rules.enumerated()), id: \.offset) { _, rule in
            RuleMark(y: .value("Rule", rule.value))
                .foregroundStyle(ChartPalette.rule(kind: rule.kind))
                .lineStyle(
                    StrokeStyle(lineWidth: 1.5 * stroke, dash: ChartPalette.ruleDash(kind: rule.kind)))
                .annotation(position: .top, alignment: .leading, spacing: 1) {
                    Text(rule.label).font(.caption).foregroundStyle(.readableSecondary)
                        .padding(.horizontal, 3)
                        .background(
                            Color(.secondarySystemGroupedBackground).opacity(0.9),
                            in: RoundedRectangle(cornerRadius: 3))
                }
        }
        // Now goes under the curves and the event labels, which sit on a backing of their own
        // so the line never runs through their words.
        if let now = chart.nowT {
            RuleMark(x: .value("Now", Double(now)))
                .foregroundStyle(Color.red.opacity(0.8))
                .lineStyle(StrokeStyle(lineWidth: 1.5 * stroke))
        }
        ForEach(Array(chart.series.enumerated()), id: \.offset) { index, series in
            seriesMarks(series, index: index)
        }
        ForEach(Array(chart.markers.enumerated()), id: \.offset) { _, marker in
            markerMark(marker)
        }
    }

    @ChartContentBuilder private func seriesMarks(
        _ series: ViewModel.ChartSeries, index: Int
    ) -> some ChartContent {
        let color = ChartPalette.color(role: series.role)
        let name = "\(index)"
        ForEach(Array(series.points.enumerated()), id: \.offset) { _, point in
            switch series.style {
            case "area":
                AreaMark(
                    x: .value("Time", Double(point.t)), y: .value("Value", point.value),
                    series: .value("Series", name)
                )
                .foregroundStyle(color.opacity(0.3))
            case "bars":
                BarMark(
                    x: .value("Time", Double(point.t)), y: .value("Value", point.value),
                    width: .fixed(4)
                )
                .foregroundStyle(color)
            case "dots":
                PointMark(x: .value("Time", Double(point.t)), y: .value("Value", point.value))
                    .symbolSize(30)
                    .foregroundStyle(color)
            default:
                LineMark(
                    x: .value("Time", Double(point.t)), y: .value("Value", point.value),
                    series: .value("Series", name)
                )
                .foregroundStyle(color)
                .lineStyle(
                    StrokeStyle(
                        lineWidth: ChartPalette.width(role: series.role) * stroke,
                        dash: ChartPalette.dash(role: series.role)))
            }
        }
    }

    @ChartContentBuilder private func markerMark(_ marker: ViewModel.ChartMarker) -> some ChartContent {
        if let value = marker.value {
            PointMark(x: .value("Time", Double(marker.t)), y: .value("Value", value))
                .symbolSize(50)
                .foregroundStyle(marker.kind == "low" ? Color.teal : Color.blue)
                .annotation(
                    position: marker.kind == "low" ? .bottom : .top,
                    spacing: 4,
                    overflowResolution: .init(x: .fit(to: .chart), y: .fit(to: .chart))
                ) {
                    Text(marker.label)
                        .font(.caption.weight(.medium))
                        .padding(.horizontal, 3)
                        .background(
                            Color(.secondarySystemGroupedBackground).opacity(0.9),
                            in: RoundedRectangle(cornerRadius: 3))
                }
        } else if let symbol = ChartPalette.symbol(marker: marker.kind) {
            RuleMark(x: .value("Time", Double(marker.t)))
                .foregroundStyle(Color.orange.opacity(0.7))
                .lineStyle(StrokeStyle(lineWidth: 1 * stroke, dash: [2, 3]))
                .annotation(
                    position: .top, spacing: 0,
                    overflowResolution: .init(x: .fit(to: .chart), y: .disabled)
                ) {
                    Image(systemName: symbol).font(.caption2).foregroundStyle(.orange)
                        .accessibilityHidden(true)
                }
        } else {
            RuleMark(x: .value("Time", Double(marker.t)))
                .foregroundStyle(Color.green.opacity(0.7))
                .lineStyle(StrokeStyle(lineWidth: 1 * stroke, dash: [2, 3]))
        }
    }
}

/// A chart with its title and unit above and its key below: what each line and band is, in the
/// view's words, and every rule, drawn or too far from the data to be.
struct ChartBlock: View {
    let chart: ViewModel.Chart
    var valueBand: ChartValueBand?
    /// Further lines of words under the key (the datum, the percentile thresholds).
    var notes: [String] = []
    /// Labelled values under the key, one row each (the moon's illumination, the thresholds).
    var facts: [ChartFact] = []

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(alignment: .firstTextBaseline) {
                Text(chart.title).font(.headline)
                if !chart.unit.isEmpty {
                    Text(chart.unit).font(.subheadline).foregroundStyle(.readableSecondary)
                }
            }
            .accessibilityHidden(true)
            ChartPlot(chart: chart, valueBand: valueBand)
            ChartKey(chart: chart, valueBand: valueBand, notes: notes, facts: facts)
        }
        .padding(.vertical, 4)
    }
}

/// A label and its value, as the view model gives them: shown as a row, the value and its unit
/// in separate texts.
struct ChartFact: Hashable {
    var label: String
    var value: ViewModel.Quantity
}

struct ChartKey: View {
    let chart: ViewModel.Chart
    var valueBand: ChartValueBand?
    var notes: [String] = []
    var facts: [ChartFact] = []

    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            ForEach(Array(chart.series.enumerated()), id: \.offset) { _, series in
                HStack(spacing: 6) {
                    Capsule()
                        .fill(ChartPalette.color(role: series.role))
                        .frame(width: 18, height: ChartPalette.width(role: series.role))
                    Text(series.label)
                }
            }
            ForEach(Array(uniqueBands.enumerated()), id: \.offset) { _, band in
                HStack(spacing: 6) {
                    RoundedRectangle(cornerRadius: 2)
                        .fill(ChartPalette.band(kind: band.kind))
                        .overlay(RoundedRectangle(cornerRadius: 2).stroke(.secondary.opacity(0.4)))
                        .frame(width: 18, height: 10)
                    Text(band.label)
                }
            }
            ForEach(Array(chart.rules.enumerated()), id: \.offset) { _, rule in
                HStack(spacing: 6) {
                    Capsule()
                        .fill(ChartPalette.rule(kind: rule.kind))
                        .frame(width: 18, height: 2)
                    Text(rule.label)
                }
            }
            if let band = valueBand {
                HStack(spacing: 6) {
                    RoundedRectangle(cornerRadius: 2).fill(Color.mint.opacity(0.35))
                        .frame(width: 18, height: 10)
                    Text(band.label)
                }
            }
            if !timeMarkers.isEmpty {
                // Each marker's own words in a wrapping layout: nothing is joined here.
                WrappingLayout(spacing: 12, lineSpacing: 2) {
                    ForEach(timeMarkers, id: \.offset) { Text($0.label) }
                }
            }
            if !facts.isEmpty {
                Grid(alignment: .leadingFirstTextBaseline, horizontalSpacing: 12, verticalSpacing: 2) {
                    ForEach(Array(facts.enumerated()), id: \.offset) { _, fact in
                        GridRow {
                            Text(fact.label)
                            HStack(alignment: .firstTextBaseline, spacing: 4) {
                                Text(fact.value.text)
                                if !fact.value.unit.isEmpty { Text(fact.value.unit) }
                            }
                        }
                    }
                }
            }
            ForEach(Array(notes.enumerated()), id: \.offset) { _, note in Text(note) }
        }
        .font(.caption)
        .foregroundStyle(.readableSecondary)
        .fixedSize(horizontal: false, vertical: true)
        .accessibilityHidden(true)
    }

    /// One entry per kind and label: a chart with a band for every night names it once.
    private var uniqueBands: [ViewModel.ChartBand] {
        var seen = Set<String>()
        return chart.bands.filter { !$0.label.isEmpty && seen.insert($0.kind + $0.label).inserted }
    }

    /// The sunrise and sunset markers, in time order.
    private var timeMarkers: [(offset: Int, label: String)] {
        chart.markers.filter { ChartPalette.symbol(marker: $0.kind) != nil }
            .enumerated().map { (offset: $0.offset, label: $0.element.label) }
    }
}

/// Lays its children out left to right and wraps them onto further lines when the row is full.
struct WrappingLayout: Layout {
    var spacing: CGFloat = 8
    var lineSpacing: CGFloat = 4

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let placed = arrange(width: proposal.width ?? .infinity, subviews: subviews)
        return CGSize(width: placed.width, height: placed.height)
    }

    func placeSubviews(
        in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()
    ) {
        let placed = arrange(width: bounds.width, subviews: subviews)
        for (subview, origin) in zip(subviews, placed.origins) {
            subview.place(
                at: CGPoint(x: bounds.minX + origin.x, y: bounds.minY + origin.y),
                proposal: .unspecified)
        }
    }

    private func arrange(width: CGFloat, subviews: Subviews)
        -> (origins: [CGPoint], width: CGFloat, height: CGFloat)
    {
        var origins: [CGPoint] = []
        var x: CGFloat = 0
        var y: CGFloat = 0
        var lineHeight: CGFloat = 0
        var widest: CGFloat = 0
        for subview in subviews {
            let size = subview.sizeThatFits(.unspecified)
            if x > 0, x + size.width > width {
                y += lineHeight + lineSpacing
                x = 0
                lineHeight = 0
            }
            origins.append(CGPoint(x: x, y: y))
            x += size.width + spacing
            widest = max(widest, x - spacing)
            lineHeight = max(lineHeight, size.height)
        }
        return (origins, widest, y + lineHeight)
    }
}
