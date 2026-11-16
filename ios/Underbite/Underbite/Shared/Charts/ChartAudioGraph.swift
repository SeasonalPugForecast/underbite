import Accessibility
import SwiftUI
import UnderbiteKit

/// A chart for VoiceOver's audio graph and its chart description: every series with its points
/// and the view model's own labels. A time reads as the view's nearest tick label and a value
/// as the label of the nearest point, so nothing is formatted here.
nonisolated struct ChartAudioGraph: AXChartDescriptorRepresentable {
    let chart: ViewModel.Chart
    let yDomain: ClosedRange<Double>

    func makeChartDescriptor() -> AXChartDescriptor {
        let ticks = chart.xTicks.sorted { $0.t < $1.t }
        let points = chart.series.flatMap(\.points)
        let times = points.map { Double($0.t) } + ticks.map { Double($0.t) }
        let start = times.min() ?? 0
        let end = max(times.max() ?? start + 1, start + 1)
        let xAxis = AXNumericDataAxisDescriptor(
            title: chart.title, range: start...end, gridlinePositions: ticks.map { Double($0.t) }
        ) { t in
            ticks.min(by: { abs(Double($0.t) - t) < abs(Double($1.t) - t) })?.label ?? ""
        }
        let yAxis = AXNumericDataAxisDescriptor(
            title: chart.unit.isEmpty ? chart.title : chart.unit, range: yDomain,
            gridlinePositions: []
        ) { value in
            points.min(by: { abs($0.value - value) < abs($1.value - value) })?.label ?? ""
        }
        let series = chart.series.map { series in
            AXDataSeriesDescriptor(
                name: series.label,
                isContinuous: series.style == "line" || series.style == "area",
                dataPoints: series.points.map {
                    AXDataPoint(x: Double($0.t), y: $0.value, additionalValues: [], label: $0.label)
                })
        }
        return AXChartDescriptor(
            title: chart.title, summary: chart.accessibility, xAxis: xAxis, yAxis: yAxis,
            additionalAxes: [], series: series)
    }

    func updateChartDescriptor(_ descriptor: AXChartDescriptor) {}
}
