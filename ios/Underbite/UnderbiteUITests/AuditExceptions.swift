import XCTest

/// Where things are on the screen being audited, read once per audit.
struct AuditScreen {
    var window: CGRect
    /// The navigation bars and tab bars.
    var bars: [CGRect]
    /// The lists and scroll views.
    var lists: [CGRect]
    /// The lowest edge of the bars along the top: the iPad's tab bar, which floats over the
    /// page there, is not always listed as a tab bar, so the status bar and it count at least.
    var topChrome: CGFloat
    /// The launch fixed the text size, so the size checks change nothing on the page.
    var textSizeFixed: Bool

    @MainActor
    init(_ app: XCUIApplication, textSizeFixed: Bool) {
        window = app.windows.firstMatch.frame
        bars = (app.navigationBars.allElementsBoundByIndex + app.tabBars.allElementsBoundByIndex)
            .map(\.frame).filter { !$0.isEmpty }
        lists = (app.collectionViews.allElementsBoundByIndex + app.scrollViews.allElementsBoundByIndex)
            .map(\.frame).filter { !$0.isEmpty }
        let screen = window
        let top = bars.filter { $0.midY < screen.midY }.map(\.maxY)
        topChrome = max(top.max() ?? 0, screen.minY + 110)
        self.textSizeFixed = textSizeFixed
    }
}

/// The audit issues the tests let pass, each with its reason. Kept short on purpose: an issue
/// is fixed in the app unless the audit cannot judge what it sees.
@MainActor
enum AuditExceptions {
    static func excuses(_ issue: XCUIAccessibilityAuditIssue, on screen: AuditScreen) -> Bool {
        guard let element = issue.element else {
            // Text with no element of its own is drawn inside a chart (axis, event and band
            // labels), by MapKit, or belongs to the page dimmed behind a sheet. Each chart is
            // one element that reads its description and carries the audio graph of its
            // values, and its words stop growing at the largest standard size so the plot
            // keeps its room; the map's labels are MapKit's; a page behind a sheet is out of
            // reach until the sheet closes.
            return [.elementDetection, .dynamicType, .textClipped].contains(issue.auditType)
        }
        let frame = element.frame
        let type = issue.auditType
        // A button of a navigation bar or toolbar: the system draws it on Liquid Glass in its
        // own colours and, rather than growing it with Dynamic Type, shows its label in the
        // large content viewer on a long press.
        if element.elementType == .button,
            screen.bars.contains(where: { $0.insetBy(dx: 0, dy: -8).contains(frame) })
        {
            return type == .dynamicType || type == .contrast
        }
        // Text a scroll left under a navigation bar or a tab bar, or in the fade the bars lay
        // over the content next to them: the audit reads it through the glass and finds it
        // faint or cut. Scrolled into view it is audited whole.
        if screen.bars.contains(where: { $0.insetBy(dx: 0, dy: -44).intersects(frame) })
            || frame.minY < screen.topChrome + 44
        {
            return true
        }
        // Text a scroll took off the screen altogether (a card above the top), or partly out
        // of the list or sheet that holds it or in the fade at its edge: the audit still lists
        // it, and measures what is behind or over the list's edge.
        if !screen.window.contains(frame) { return true }
        // The innermost page-sized list it overlaps holds it (a page behind a sheet does not;
        // a strip scrolled sideways within a page is not a page).
        let holder = screen.lists
            .filter { $0.intersects(frame) && $0.height >= screen.window.height * 0.4 }
            .min { $0.width * $0.height < $1.width * $1.height }
        if let holder, !holder.insetBy(dx: 0, dy: 44).contains(frame) { return true }
        // The Dynamic Type and clipped-text checks grow the text up to the largest sizes,
        // about three times as tall, which pushes everything below the top third of the screen
        // off it or under the tab bar, where they cannot read it. The same text styles are
        // checked at the top of every page, the clipped-text check runs on whole pages at the
        // largest size (the -xxl tests), and the -xxl screenshots show them.
        if !screen.textSizeFixed, type == .dynamicType || type == .textClipped,
            frame.maxY > screen.window.height / 3
        {
            return true
        }
        // The iPad's list column, on the launch the audit looks at first: every row but the
        // chosen one comes back as failing contrast, though the rows are the label colour on
        // the column's own background, pass on the iPhone and pass when the same screen is
        // launched again in the same test (the window still taking focus draws them in its
        // inactive look). Their text styles are the same as the detail column's, which is
        // audited.
        if type == .contrast, screen.window.width > 700, frame.maxX <= 320 { return true }
        // A control switched off until there is something to save: its dimmed look tells it
        // apart, and contrast rules leave inactive controls out.
        if type == .contrast, !element.isEnabled { return true }
        // MapKit's own legal link: drawn, sized and coloured by MapKit.
        if type == .hitRegion, element.label == "Legal" { return true }
        return false
    }
}
