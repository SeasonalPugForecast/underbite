import Foundation
import XCTest

@testable import UnderbiteKit

/// Runs the work handed to it only when told: the pause after the last answer, by hand.
private final class Pauses: @unchecked Sendable {
    private let lock = NSLock()
    private var pending: [@Sendable () -> Void] = []

    var later: FirstTiles.Later {
        { [self] work in lock.withLock { pending.append(work) } }
    }

    func elapse() {
        let due = lock.withLock { () -> [@Sendable () -> Void] in
            defer { pending = [] }
            return pending
        }
        due.forEach { $0() }
    }
}

/// Counts the times it is called.
private final class Calls: @unchecked Sendable {
    private let lock = NSLock()
    private var n = 0
    var count: Int { lock.withLock { n } }
    func hit() { lock.withLock { n += 1 } }
}

/// A replaced grid overlay comes off the map once the new one's first tiles are answered.
final class FirstTilesTests: XCTestCase {
    func testTheTilesAreAnsweredOnceEveryOneAskedForHasItsReply() {
        let pauses = Pauses()
        let tiles = FirstTiles(later: pauses.later)
        let calls = Calls()
        tiles.onAnswered { calls.hit() }
        tiles.ask()
        tiles.ask()
        tiles.answer()
        pauses.elapse()
        XCTAssertEqual(calls.count, 0, "one tile still has no reply")
        tiles.answer()
        XCTAssertEqual(calls.count, 0, "not before the pause")
        pauses.elapse()
        XCTAssertEqual(calls.count, 1)
        XCTAssertTrue(tiles.isDone)
        // Later tiles change nothing: it is said once.
        tiles.ask()
        tiles.answer()
        pauses.elapse()
        XCTAssertEqual(calls.count, 1)
    }

    func testATileAskedForDuringThePauseIsWaitedFor() {
        let pauses = Pauses()
        let tiles = FirstTiles(later: pauses.later)
        let calls = Calls()
        tiles.onAnswered { calls.hit() }
        tiles.ask()
        tiles.answer()
        tiles.ask()
        pauses.elapse()
        XCTAssertEqual(calls.count, 0, "the second tile was asked for in the pause")
        tiles.answer()
        pauses.elapse()
        XCTAssertEqual(calls.count, 1)
    }

    func testNothingIsSaidBeforeATileIsAskedFor() {
        let pauses = Pauses()
        let tiles = FirstTiles(later: pauses.later)
        let calls = Calls()
        tiles.onAnswered { calls.hit() }
        pauses.elapse()
        XCTAssertEqual(calls.count, 0)
        XCTAssertFalse(tiles.isDone)
    }

    func testAnActionSetAfterwardsRunsAtOnce() {
        let pauses = Pauses()
        let tiles = FirstTiles(later: pauses.later)
        tiles.ask()
        tiles.answer()
        pauses.elapse()
        let calls = Calls()
        tiles.onAnswered { calls.hit() }
        XCTAssertEqual(calls.count, 1)
    }

    func testTheDefaultPauseRunsOnItsOwn() {
        let tiles = FirstTiles()
        let answered = expectation(description: "first tiles answered")
        tiles.onAnswered { answered.fulfill() }
        tiles.ask()
        DispatchQueue.global().async { tiles.answer() }
        wait(for: [answered], timeout: 5)
    }
}

/// The overlays a grid layer replaced wait for the one drawn now.
final class RetiredOverlaysTests: XCTestCase {
    private final class Overlay {}

    func testTheReplacedOverlaysComeOffWhenTheCurrentOneSettles() {
        var retired = RetiredOverlays<Overlay>()
        let (a, b, c) = (Overlay(), Overlay(), Overlay())
        // a is replaced by b, then b by c before b's tiles answered.
        retired.retire(a, key: "sst")
        retired.retire(b, key: "sst")
        XCTAssertEqual(retired.count(for: "sst"), 2)
        // b settles late: it is no longer drawn, so nothing comes off yet.
        XCTAssertTrue(retired.settled(ObjectIdentifier(b), key: "sst", current: c).isEmpty)
        // c settles: both come off, oldest first, and only once.
        let off = retired.settled(ObjectIdentifier(c), key: "sst", current: c)
        XCTAssertEqual(off.count, 2)
        XCTAssertTrue(off[0] === a && off[1] === b)
        XCTAssertTrue(retired.settled(ObjectIdentifier(c), key: "sst", current: c).isEmpty)
    }

    func testLayersAreKeptApartAndALayerLeavingTakesItsOwn() {
        var retired = RetiredOverlays<Overlay>()
        let (sst, chlorophyll, current) = (Overlay(), Overlay(), Overlay())
        retired.retire(sst, key: "sst")
        retired.retire(chlorophyll, key: "chlorophyll")
        XCTAssertTrue(retired.settled(ObjectIdentifier(current), key: "sst", current: nil).isEmpty)
        let gone = retired.drop(key: "sst")
        XCTAssertEqual(gone.count, 1)
        XCTAssertTrue(gone[0] === sst)
        XCTAssertEqual(retired.count(for: "sst"), 0)
        XCTAssertEqual(retired.count(for: "chlorophyll"), 1)
    }
}
