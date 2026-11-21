import XCTest

@testable import UnderbiteKit

/// The time controls' chip and full controls: which events open and fold them.
final class TimeControlsFoldTests: XCTestCase {
    func testStartsCompact() {
        XCTAssertFalse(TimeControlsFold().expanded)
    }

    func testTheChipExpands() {
        var fold = TimeControlsFold()
        fold.apply(.expand)
        XCTAssertTrue(fold.expanded)
    }

    func testAPickFoldsBack() {
        var fold = TimeControlsFold()
        fold.apply(.expand)
        fold.apply(.pick)
        XCTAssertFalse(fold.expanded)
    }

    func testSteppingStaysOpen() {
        var fold = TimeControlsFold()
        fold.apply(.expand)
        fold.apply(.step)
        fold.apply(.step)
        XCTAssertTrue(fold.expanded)
    }

    func testTheCollapseControlFolds() {
        var fold = TimeControlsFold()
        fold.apply(.expand)
        fold.apply(.step)
        fold.apply(.collapse)
        XCTAssertFalse(fold.expanded)
    }

    func testAPanFolds() {
        var fold = TimeControlsFold()
        fold.apply(.expand)
        fold.apply(.pan)
        XCTAssertFalse(fold.expanded)
    }

    func testEventsOnTheChipChangeNothing() {
        var fold = TimeControlsFold()
        for event in [TimeControlsFold.Event.step, .pick, .collapse, .pan] {
            fold.apply(event)
            XCTAssertFalse(fold.expanded)
        }
    }
}
