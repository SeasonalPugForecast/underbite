import XCTest

/// XCTest's accessibility audit on each main screen, against the offline sample (`-sample`):
/// contrast, element detection, hit regions, descriptions, Dynamic Type, clipped text and
/// traits. Run on the iPhone and the iPad simulators.
@MainActor
final class AccessibilityAuditTests: XCTestCase {
    override func setUp() async throws {
        continueAfterFailure = true
    }

    // MARK: - Screens

    func testSpotsList() throws {
        let app = launch(["-tab", "spots"])
        XCTAssertTrue(app.staticTexts["The Battery"].firstMatch.waitForExistence(timeout: 15))
        try audit(app, "spots")
    }

    func testConditions() throws {
        let app = openSpot()
        try audit(app, "conditions")
        try auditScrolling(app, "conditions")
    }

    func testConditionsInSunMode() throws {
        let app = openSpot(["-sunMode"])
        try audit(app, "conditions-sun")
        try auditScrolling(app, "conditions-sun")
    }

    /// At the largest size, set for the app with a launch argument: the Dynamic Type check
    /// is left out, since it changes the text size itself and the argument holds it fixed.
    func testConditionsAtTheLargestTextSize() throws {
        let app = openSpot(Self.largestText)
        let checks = Self.checks.filter { $0.1 != .dynamicType }
        try audit(app, "conditions-xxl", checks)
        try auditScrolling(app, "conditions-xxl", checks)
    }

    func testSpotsAtTheLargestTextSize() throws {
        let app = launch(["-tab", "spots"] + Self.largestText)
        XCTAssertTrue(app.staticTexts["The Battery"].firstMatch.waitForExistence(timeout: 15))
        try audit(app, "spots-xxl", Self.checks.filter { $0.1 != .dynamicType })
    }

    func testTideDetail() throws {
        let app = openSpot(["-open-card", "tide"])
        XCTAssertTrue(app.navigationBars["Tide"].waitForExistence(timeout: 15))
        try audit(app, "tide-detail")
        try auditScrolling(app, "tide-detail")
    }

    func testFlowDetail() throws {
        let app = openSpot(["-open-card", "flow"])
        XCTAssertTrue(app.navigationBars["River flow"].waitForExistence(timeout: 15))
        try audit(app, "flow-detail")
        try auditScrolling(app, "flow-detail")
    }

    func testCustomize() throws {
        let app = openSpot(["-customize"])
        XCTAssertTrue(app.navigationBars["Customize"].waitForExistence(timeout: 15))
        try audit(app, "customize")
    }

    func testSettings() throws {
        let app = launch(["-tab", "settings"])
        XCTAssertTrue(app.navigationBars["Settings"].waitForExistence(timeout: 15))
        try audit(app, "settings")
    }

    func testSettingsPages() throws {
        for (page, title) in [
            ("sources", "Sources"), ("keys", "Keys and accounts"), ("attribution", "Attribution"),
            ("about", "About"), ("acknowledgements", "Acknowledgements"),
        ] {
            let app = launch(["-tab", "settings", "-settings", page])
            XCTAssertTrue(
                app.navigationBars[title].waitForExistence(timeout: 15), "\(page) did not open")
            try audit(app, "settings-\(page)")
        }
    }

    func testPreferencePages() throws {
        for (page, title) in [
            ("units", "Units"), ("limits", "Go/no-go limits"), ("rivers", "River ranges"),
            ("fishing", "Trout and fishing windows"), ("freshness", "Freshness and distance"),
            ("refresh", "Refresh"), ("voice", "Voice"), ("privacy", "Privacy"),
            ("display", "Sun mode"),
        ] {
            let app = launch(["-tab", "settings", "-preferences", page])
            XCTAssertTrue(
                app.navigationBars[title].waitForExistence(timeout: 15), "\(page) did not open")
            try audit(app, "preferences-\(page)")
        }
    }

    func testWelcome() throws {
        let app = launch(["-welcome"])
        XCTAssertTrue(app.buttons["Get started"].waitForExistence(timeout: 15))
        try audit(app, "welcome")
    }

    func testOtherSections() throws {
        for tab in ["map", "trips", "log"] {
            let app = launch(["-tab", tab])
            XCTAssertTrue(app.tabBars.firstMatch.waitForExistence(timeout: 15))
            try audit(app, tab)
        }
    }

    // MARK: - Helpers

    /// The largest accessibility text size, set for the app alone through UIKit's own user
    /// default.
    static let largestText = [
        "-UIPreferredContentSizeCategoryName", "UICTContentSizeCategoryAccessibilityXXXL",
    ]

    private func launch(_ arguments: [String]) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = ["-sample"] + arguments
        app.launch()
        return app
    }

    /// The sample spot's conditions, loaded (or the card page `more` opens over them).
    private func openSpot(_ more: [String] = []) -> XCUIApplication {
        let app = launch(["-tab", "spots", "-open-spot", "battery"] + more)
        if !more.contains("-open-card") {
            XCTAssertTrue(app.navigationBars["The Battery"].waitForExistence(timeout: 15))
            XCTAssertTrue(
                app.staticTexts["Tidal river or estuary"].firstMatch.waitForExistence(timeout: 15))
        }
        return app
    }

    /// Every check of the audit on iOS, one at a time: together they can take longer than
    /// the audit allows itself on a busy machine.
    static let checks: [(String, XCUIAccessibilityAuditType)] = [
        ("contrast", .contrast), ("element detection", .elementDetection),
        ("hit region", .hitRegion), ("description", .sufficientElementDescription),
        ("dynamic type", .dynamicType), ("clipped text", .textClipped), ("trait", .trait),
    ]

    /// Audits the screen as it is; every issue is recorded unless `AuditExceptions` names it.
    /// A check that runs out of time is tried once more.
    private func audit(
        _ app: XCUIApplication, _ screen: String,
        _ checks: [(String, XCUIAccessibilityAuditType)] = AccessibilityAuditTests.checks
    ) throws {
        for (name, check) in checks {
            for attempt in 1...2 {
                do {
                    try app.performAccessibilityAudit(for: check) { issue in
                        let excused = AuditExceptions.excuses(
                            issue, in: app, textSizeFixed: app.launchArguments.contains(
                                Self.largestText[0]))
                        let element = issue.element.map { "\($0.elementType.rawValue) '\($0.label)'" }
                        print(
                            "AUDIT \(screen) [\(name)] \(excused ? "excused" : "ISSUE"): "
                                + "\(issue.compactDescription) | \(element ?? "-") | \(issue.detailedDescription)")
                        return excused
                    }
                    break
                } catch where attempt == 1 {
                    print("AUDIT \(screen) [\(name)] retrying: \(error)")
                }
            }
        }
    }

    /// Scrolls the main scroll view or list down twice, auditing each time. The drag holds
    /// still before it lets go, so the page is at rest when the audit looks at it. The Dynamic
    /// Type and clipped-text checks run on the page at its top only: they change the text
    /// size, which re-flows a scrolled page under them, so they would compare a text with
    /// whatever moved into its place.
    private func auditScrolling(
        _ app: XCUIApplication, _ screen: String,
        _ checks: [(String, XCUIAccessibilityAuditType)] = AccessibilityAuditTests.checks
    ) throws {
        let scroller =
            app.scrollViews.firstMatch.exists ? app.scrollViews.firstMatch : app.collectionViews.firstMatch
        guard scroller.exists else { return }
        for step in 1...2 {
            let from = scroller.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.75))
            let to = scroller.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.25))
            from.press(forDuration: 0.1, thenDragTo: to, withVelocity: .slow, thenHoldForDuration: 0.5)
            // A card the drag began on fades back from its pressed look.
            Thread.sleep(forTimeInterval: 1.5)
            try audit(
                app, "\(screen) scrolled \(step)",
                checks.filter {
                    // At a size fixed by the launch, the clipped-text check grows nothing and
                    // reads the page as it is: it runs on every part of it.
                    $0.1 != .dynamicType
                        && ($0.1 != .textClipped || app.launchArguments.contains(Self.largestText[0]))
                })
        }
    }
}
