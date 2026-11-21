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

    func testABandThatCoversTheWholeAxisIsNotDrawn() {
        let domain = ChartDomain.y(values: [13.4, 13.9], rules: [], thresholds: [2, 8, 15, 30, 80])
        XCTAssertFalse(ChartDomain.bandShows(low: 2, high: 80, in: domain))
        XCTAssertTrue(ChartDomain.bandShows(low: 8, high: 80, in: domain))
        XCTAssertTrue(ChartDomain.bandShows(low: 2, high: 15, in: domain))
    }
}
