import XCTest

@testable import UnderbiteKit

/// The map's load timeline: the first of each milestone, in milliseconds from the launch, and
/// the recorder that logs them.
final class MapLoadTimelineTests: XCTestCase {
    func testAMilestoneIsMillisecondsFromTheLaunch() {
        var timeline = MapLoadTimeline(start: 100)
        let found = timeline.note(.baseMap, at: 100.25)
        XCTAssertEqual(found?.ms, 250)
        XCTAssertEqual(timeline.milestone(.baseMap)?.ms, 250)
    }

    func testOnlyTheFirstOfAPhaseAndLayerIsKept() {
        var timeline = MapLoadTimeline(start: 0)
        XCTAssertNotNil(timeline.note(.tile, layer: "a", source: "network", at: 1))
        XCTAssertNil(timeline.note(.tile, layer: "a", source: "memory", at: 2))
        XCTAssertNotNil(timeline.note(.tile, layer: "b", source: "store", at: 3))
        XCTAssertEqual(timeline.milestone(.tile, layer: "a")?.source, "network")
        XCTAssertEqual(timeline.milestone(.tile, layer: "a")?.ms, 1000)
        XCTAssertEqual(timeline.milestones.count, 2)
    }

    func testATimeBeforeTheLaunchIsZero() {
        var timeline = MapLoadTimeline(start: 10)
        XCTAssertEqual(timeline.note(.regionSettled, at: 9)?.ms, 0)
    }

    func testTheAxisWaitIsServedMinusAsked() {
        var timeline = MapLoadTimeline(start: 0)
        XCTAssertNil(timeline.axisWait(layer: "hfradar"))
        timeline.note(.axisAsked, layer: "hfradar", at: 0.5)
        XCTAssertNil(timeline.axisWait(layer: "hfradar"))
        timeline.note(.axis, layer: "hfradar", source: "network", at: 3.5)
        XCTAssertEqual(timeline.axisWait(layer: "hfradar"), 3000)
    }

    func testTheSummaryListsTheMilestonesInTheOrderReached() {
        var timeline = MapLoadTimeline(start: 0)
        XCTAssertEqual(timeline.summary(), "")
        timeline.note(.baseMap, at: 0.4)
        timeline.note(.axis, layer: "sst", source: "store", at: 0.5)
        timeline.note(.block, layer: "sst", source: "cache", at: 0.9)
        XCTAssertEqual(timeline.summary(), "base-map@400 axis[sst]=store@500 block[sst]=cache@900")
    }

    // MARK: - The recorder

    private final class Lines: @unchecked Sendable {
        private let lock = NSLock()
        private var all: [String] = []
        var value: [String] { lock.withLock { all } }
        func add(_ line: String) { lock.withLock { all.append(line) } }
    }

    private final class Clock: @unchecked Sendable {
        private let lock = NSLock()
        private var now = 0.0
        func set(_ value: Double) { lock.withLock { now = value } }
        func read() -> Double { lock.withLock { now } }
    }

    func testTheRecorderLogsEachMilestoneOnceWithoutCoordinatesOrURLs() {
        let lines = Lines()
        let clock = Clock()
        let load = MapLoad(clock: { clock.read() }, emit: { lines.add($0) })
        clock.set(0.125)
        load.mark(.axis, layer: "hfradar", source: "network")
        load.mark(.axis, layer: "hfradar", source: "memory")
        load.mark(.baseMap)
        XCTAssertEqual(
            lines.value,
            [
                "milestone phase=axis layer=hfradar source=network ms=125",
                "milestone phase=base-map ms=125",
            ])
    }

    func testTheSummaryIsLoggedOnceAndEndsTheRecording() {
        let lines = Lines()
        let clock = Clock()
        let load = MapLoad(clock: { clock.read() }, emit: { lines.add($0) })
        clock.set(1)
        load.mark(.regionSettled)
        load.finish()
        load.finish()
        load.mark(.baseMap)
        XCTAssertEqual(
            lines.value, ["milestone phase=region ms=1000", "summary region@1000"])
    }
}
