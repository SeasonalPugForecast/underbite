import XCTest

/// What VoiceOver reads and the safety notices a screen must show, against the offline sample
/// (`-sample`). Run on the iPhone and the iPad simulators.
@MainActor
final class ReadingTests: XCTestCase {
    private func launch(_ arguments: [String]) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = ["-sample"] + arguments
        app.launch()
        app.activate()
        return app
    }

    /// The alert banner is one element that reads every line it shows: the National Weather
    /// Service's headline, when it is in effect, how severe it is, and the credit.
    func testTheAlertBannerReadsEveryLineItShows() throws {
        let app = launch(["-tab", "spots", "-open-spot", "battery"])
        let banner = app.descendants(matching: .any)["alerts-banner"].firstMatch
        XCTAssertTrue(banner.waitForExistence(timeout: 15))
        let label = banner.label
        for line in [
            "Small Craft Advisory in effect", "Nov 6, 4:20 PM – Nov 7, 2:20 AM", "Minor",
            "Marine alerts: NOAA National Weather Service",
        ] {
            XCTAssertTrue(label.contains(line), "\(line) is not read: \(label)")
        }
    }

    func testTheMapSaysItIsNotForNavigation() {
        let app = launch(["-tab", "map"])
        XCTAssertTrue(app.maps.firstMatch.waitForExistence(timeout: 15))
        let notice = app.descendants(matching: .any)["map-notice"].firstMatch
        XCTAssertTrue(notice.waitForExistence(timeout: 5))
        XCTAssertEqual(notice.label, "Not for navigation.")
    }
}
