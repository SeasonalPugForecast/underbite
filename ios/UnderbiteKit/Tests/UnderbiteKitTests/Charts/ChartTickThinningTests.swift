import XCTest

@testable import UnderbiteKit

/// Which x-axis ticks get a label: the day marks and the major hours stay, no two labels meet.
final class ChartTickThinningTests: XCTestCase {
    /// Hourly ticks from midnight, major every third hour, with the weekday on each midnight,
    /// as the view model supplies them.
    private func hourly(_ hours: Int) -> [ViewModel.ChartTick] {
        (0...hours).map { h in
            let hour = h % 24
            let twelve = hour % 12 == 0 ? 12 : hour % 12
            let label = hour == 0 ? ["Sat", "Sun", "Mon"][h / 24 % 3] : "\(twelve) \(hour < 12 ? "AM" : "PM")"
            return ViewModel.ChartTick(t: Int64(h) * 3_600, label: label, major: hour % 3 == 0)
        }
    }

    private func thin(_ ticks: [ViewModel.ChartTick], width: Double) -> [ViewModel.ChartTick] {
        let last = Double(ticks.last?.t ?? 1)
        return ChartTickThinning.thinned(ticks, domain: 0...last, plotWidth: width)
    }

    func testTheWeekdayTicksSurviveOnANarrowChart() {
        let kept = thin(hourly(48), width: 300)
        XCTAssertEqual(kept.filter { ["Sat", "Sun", "Mon"].contains($0.label) }.count, 3)
    }

    func testNoTwoLabelsOverlap() {
        let ticks = hourly(48)
        let kept = thin(ticks, width: 300)
        let perSecond = 300.0 / Double(ticks.last?.t ?? 1)
        for (a, b) in zip(kept, kept.dropFirst()) {
            let apart = Double(b.t - a.t) * perSecond
            let need =
                Double(a.label.count + b.label.count) * ChartTickThinning.characterWidth / 2
                + ChartTickThinning.gap
            XCTAssertGreaterThanOrEqual(apart, need - 1e-9, "\(a.label) and \(b.label)")
        }
    }

    func testMinorTicksFillOnlyWhatTheMajorOnesLeaveFree() {
        let kept = thin(hourly(12), width: 600)
        XCTAssertGreaterThanOrEqual(kept.filter(\.major).count, 4)
        XCTAssertTrue(kept.contains { !$0.major })
        XCTAssertEqual(kept.map(\.t), kept.map(\.t).sorted())
    }

    func testAnEmptyListStaysEmpty() {
        XCTAssertTrue(ChartTickThinning.thinned([], domain: 0...1, plotWidth: 300).isEmpty)
    }
}
