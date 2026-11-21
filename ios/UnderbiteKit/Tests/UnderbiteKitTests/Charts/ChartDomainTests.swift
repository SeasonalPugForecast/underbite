import XCTest

@testable import UnderbiteKit

/// The value axis of a chart: the data, widened to the nearest thresholds when there are any.
final class ChartDomainTests: XCTestCase {
    func testWithoutThresholdsTheDomainIsTheDataPadded() {
        let domain = ChartDomain.y(values: [10, 20], rules: [], thresholds: [])
        XCTAssertEqual(domain.lowerBound, 8.8, accuracy: 1e-9)
        XCTAssertEqual(domain.upperBound, 21.2, accuracy: 1e-9)
    }

    func testTheDomainReachesTheNearestThresholdsAroundTheData() {
        let domain = ChartDomain.y(
            values: [13.4, 13.9], rules: [], thresholds: [2, 8, 15, 30, 80])
        // 8 is the nearest below and 15 the nearest above.
        XCTAssertEqual(domain.lowerBound, 8 - 7 * 0.12, accuracy: 1e-9)
        XCTAssertEqual(domain.upperBound, 15 + 7 * 0.12, accuracy: 1e-9)
    }

    func testDataBeyondEveryThresholdKeepsItsOwnEdge() {
        let domain = ChartDomain.y(values: [1, 100], rules: [], thresholds: [5, 10, 50])
        XCTAssertEqual(domain.lowerBound, 1 - 99 * 0.12, accuracy: 1e-9)
        XCTAssertEqual(domain.upperBound, 100 + 99 * 0.12, accuracy: 1e-9)
    }

    func testRulesAreKeptInside() {
        let domain = ChartDomain.y(values: [10, 11], rules: [20], thresholds: [5, 12])
        XCTAssertGreaterThanOrEqual(domain.upperBound, 20)
        XCTAssertLessThanOrEqual(domain.lowerBound, 5)
    }

    func testABandIsFilledOnlyWhenItLeavesPartOfTheAxisOpen() {
        let domain = ChartDomain.y(values: [13.4, 13.9], rules: [], thresholds: [2, 8, 15, 30, 80])
        // Both edges outside: the whole axis, no fill.
        XCTAssertFalse(ChartDomain.fills(low: 2, high: 80, in: domain))
        // One edge inside, the other beyond: a part of the axis.
        XCTAssertTrue(ChartDomain.fills(low: 14, high: 80, in: domain))
        XCTAssertTrue(ChartDomain.fills(low: 2, high: 9, in: domain))
        // An edge inside but the span still covers nearly all of it: no fill.
        let wide = 0.0...100.0
        XCTAssertFalse(ChartDomain.fills(low: -50, high: 95, in: wide))
        XCTAssertTrue(ChartDomain.fills(low: -50, high: 60, in: wide))
    }

    func testCrowdedRulesKeepOnlyTheTopLabelOfEachCluster() {
        let domain = 0.0...100.0
        // 90 and 88 and 50 and 49: the first of each cluster, from the top, is named.
        let named = ChartDomain.labelled([50, 90, 49, 88], in: domain)
        XCTAssertEqual(named, [1, 0])
        XCTAssertEqual(ChartDomain.labelled([], in: domain), [])
        XCTAssertEqual(ChartDomain.labelled([10, 40, 80], in: domain), [0, 1, 2])
    }
}
