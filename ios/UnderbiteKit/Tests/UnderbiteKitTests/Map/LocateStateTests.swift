import XCTest

@testable import UnderbiteKit

/// The locate button asks for access only when tapped, then centres and follows; a second tap
/// stops following; precise location is asked for once.
final class LocateStateTests: XCTestCase {
    func testOpeningTheMapAsksForNothing() {
        let state = LocateState()
        XCTAssertFalse(state.showsPosition)
        XCTAssertFalse(state.following)
        XCTAssertEqual(state.symbol, "location")
    }

    func testTheFirstTapAsksForAccessThenFollowsOnceGiven() {
        var state = LocateState()
        XCTAssertEqual(state.tap(), [.askAccess])
        XCTAssertFalse(state.following)
        XCTAssertTrue(state.accessChanged(to: .allowed, precise: true))
        XCTAssertTrue(state.showsPosition)
        XCTAssertTrue(state.following)
        XCTAssertEqual(state.symbol, "location.fill")
        XCTAssertEqual(state.accessibilityValue, "Following")
    }

    func testAccessGivenWithoutATapDoesNotMoveTheMap() {
        var state = LocateState()
        XCTAssertFalse(state.accessChanged(to: .allowed, precise: true))
        XCTAssertTrue(state.showsPosition)
        XCTAssertFalse(state.following)
    }

    func testRefusingAfterTheTapLeavesTheMapAlone() {
        var state = LocateState()
        _ = state.tap()
        XCTAssertFalse(state.accessChanged(to: .refused, precise: false))
        XCTAssertFalse(state.showsPosition)
        XCTAssertFalse(state.followWhenAllowed)
        XCTAssertEqual(state.tap(), [.explainRefused])
        XCTAssertEqual(state.symbol, "location.slash")
    }

    func testTheSecondTapTogglesFollowing() {
        var state = LocateState()
        _ = state.accessChanged(to: .allowed, precise: true)
        XCTAssertEqual(state.tap(), [.follow])
        XCTAssertTrue(state.following)
        XCTAssertEqual(state.tap(), [.stopFollowing])
        XCTAssertFalse(state.following)
        XCTAssertEqual(state.tap(), [.follow])
    }

    func testMovingTheMapStopsFollowing() {
        var state = LocateState()
        _ = state.accessChanged(to: .allowed, precise: true)
        _ = state.tap()
        state.stoppedFollowing()
        XCTAssertEqual(state.tap(), [.follow])
    }

    func testApproximateAccessAsksForPreciseOnce() {
        var state = LocateState()
        _ = state.accessChanged(to: .allowed, precise: false)
        XCTAssertEqual(state.tap(), [.askPrecise, .follow])
        XCTAssertEqual(state.tap(), [.stopFollowing])
        XCTAssertEqual(state.tap(), [.follow])
    }

    func testLosingAccessStopsFollowing() {
        var state = LocateState()
        _ = state.accessChanged(to: .allowed, precise: true)
        _ = state.tap()
        _ = state.accessChanged(to: .refused, precise: true)
        XCTAssertFalse(state.following)
        XCTAssertFalse(state.showsPosition)
    }
}
