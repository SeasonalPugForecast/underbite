import XCTest

@testable import UnderbiteKit

/// The colour range stays put for small changes and follows large ones.
final class RangeFitTests: XCTestCase {
    private let held = ValueRange(low: 10, high: 20)

    func testTheFirstRangeIsTaken() {
        let new = ValueRange(low: 12, high: 18)
        XCTAssertEqual(RangeFit.settle(new: new, held: nil), new)
    }

    func testNothingInViewKeepsTheLastRange() {
        XCTAssertEqual(RangeFit.settle(new: nil, held: held), held)
        XCTAssertNil(RangeFit.settle(new: nil, held: nil))
    }

    func testASmallShiftKeepsTheRangeDrawn() {
        let near = ValueRange(low: 10.4, high: 19.5)
        XCTAssertEqual(RangeFit.settle(new: near, held: held), held)
    }

    func testALargeShiftReplacesIt() {
        let far = ValueRange(low: 12, high: 20)
        XCTAssertEqual(RangeFit.settle(new: far, held: held), far)
    }
}
