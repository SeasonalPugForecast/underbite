import XCTest

/// The audit issues the tests let pass, each with its reason. Kept short on purpose: an issue
/// is fixed in the app unless the audit cannot judge what it sees.
@MainActor
enum AuditExceptions {
    /// - Parameter textSizeFixed: the launch fixed the text size, so the size checks change
    ///   nothing on the page.
    static func excuses(
        _ issue: XCUIAccessibilityAuditIssue, in app: XCUIApplication, textSizeFixed: Bool
    ) -> Bool {
        guard let element = issue.element else {
            // Text with no element of its own is drawn inside a chart (axis, event and band
            // labels) or by MapKit. Each chart is one element that reads its description and
            // carries the audio graph of its values, and its words stop growing at the largest
            // standard size so the plot keeps its room; the map's labels are MapKit's.
            return [.elementDetection, .dynamicType, .textClipped].contains(issue.auditType)
        }
        // Text a scroll left under the navigation bar or the tab bar, or in the fade the bars
        // lay over the content next to them: the audit reads it through the glass and finds it
        // faint or cut. Scrolled into view it is audited whole.
        if underBars(element, in: app) { return true }
        // Text a scroll took off the screen altogether (a card above the top): the audit
        // still lists it, but finds nothing of it to measure.
        if !app.windows.firstMatch.frame.contains(element.frame) { return true }
        // The Dynamic Type and clipped-text checks grow the text up to the largest sizes,
        // about three times as tall, which pushes everything below the top third of the screen
        // off it or under the tab bar, where they cannot read it. The same text styles are
        // checked at the top of every page, the clipped-text check runs on whole pages at the
        // largest size (the -xxl tests), and the -xxl screenshots show them.
        if !textSizeFixed, issue.auditType == .dynamicType || issue.auditType == .textClipped,
            element.frame.maxY > app.windows.firstMatch.frame.height / 3
        {
            return true
        }
        // A control switched off until there is something to save: its dimmed look tells it
        // apart, and contrast rules leave inactive controls out.
        if issue.auditType == .contrast, !element.isEnabled { return true }
        // MapKit's own legal link: drawn, sized and coloured by MapKit.
        if issue.auditType == .hitRegion, element.label == "Legal" { return true }
        return false
    }

    /// Whether `element` is page content reaching under a navigation bar or a tab bar (the
    /// bars' own buttons and titles are audited as they are).
    private static func underBars(_ element: XCUIElement, in app: XCUIApplication) -> Bool {
        let frame = element.frame
        let bars = app.navigationBars.allElementsBoundByIndex + app.tabBars.allElementsBoundByIndex
        return bars.contains { bar in
            // The bar and the scroll edge fade beside it.
            let covered = bar.frame.insetBy(dx: 0, dy: -44)
            guard !covered.isEmpty, covered.intersects(frame) else { return false }
            let own = bar.descendants(matching: .any)
                .matching(NSPredicate(format: "label == %@", element.label))
            return own.count == 0
        }
    }
}
