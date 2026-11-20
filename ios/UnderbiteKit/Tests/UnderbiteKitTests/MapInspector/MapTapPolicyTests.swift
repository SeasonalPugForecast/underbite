import XCTest

@testable import UnderbiteKit

/// What a tap on the map does: the marker drops on a long press, or on a tap in pin mode.
final class MapTapPolicyTests: XCTestCase {
    func testAPlainTapOnEmptyMapDoesNothingWithPinModeOff() {
        XCTAssertEqual(MapTapPolicy.action(measuring: false, pinMode: false), .ignore)
    }

    func testATapOnEmptyMapDropsTheMarkerWithPinModeOn() {
        XCTAssertEqual(MapTapPolicy.action(measuring: false, pinMode: true), .dropMarker)
    }

    func testATapWhileMeasuringAddsAPointInEitherMode() {
        XCTAssertEqual(MapTapPolicy.action(measuring: true, pinMode: false), .extendLine)
        XCTAssertEqual(MapTapPolicy.action(measuring: true, pinMode: true), .extendLine)
    }

    func testAPinTapReachesItsModuleBeforeTheInspector() {
        // Topmost first: stations, then the inspector at the bottom.
        let modules = ["stations", "catches", "inspector"]
        let taker = MapTapPolicy.deliver((), to: modules) { module, _ in
            module == "catches" || module == "inspector"
        }
        XCTAssertEqual(taker, "catches")
    }

    func testATapNoPinTookReachesTheInspectorWhichMayIgnoreIt() {
        let modules = ["stations", "inspector"]
        let pinMode = false
        let taker = MapTapPolicy.deliver((), to: modules) { module, _ in
            module == "inspector"
                && MapTapPolicy.action(measuring: false, pinMode: pinMode) != .ignore
        }
        XCTAssertNil(taker)
    }
}
