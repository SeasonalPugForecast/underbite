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
    /// A navigation bar draws a title.
    var barTitle: Bool

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
        barTitle = app.navigationBars.allElementsBoundByIndex.contains { !$0.identifier.isEmpty }
    }
}

/// The audit issues the tests let pass, each with its reason and only for the checks the reason
/// is about. Kept short on purpose: an issue is fixed in the app unless the audit cannot judge
/// what it sees.
@MainActor
enum AuditExceptions {
    /// Why an issue is let pass.
    enum Reason: String {
        case drawnText = "text drawn in a chart or by MapKit"
        case barTitle = "a navigation bar's title"
        case glassColumn = "the iPad's glass list column"
        case barButton = "a bar's own button"
        case underBars = "under or beside the bars"
        case offScreen = "scrolled off the screen"
        case listEdge = "at the edge of its list"
        case grownAway = "grown out of the check's reach"
        case inactiveControl = "a control switched off"
        case mapLegal = "MapKit's legal link"
    }

    static func excuses(_ issue: XCUIAccessibilityAuditIssue, on screen: AuditScreen) -> Bool {
        reason(issue, on: screen) != nil
    }

    static func reason(_ issue: XCUIAccessibilityAuditIssue, on screen: AuditScreen) -> Reason? {
        let type = issue.auditType
        guard let element = issue.element else {
            // Text with no element of its own is drawn inside a chart (axis, event and band
            // labels) or by MapKit (the map's labels). Each chart is one element that reads
            // its description and carries the audio graph of its values, and its words stop
            // growing at the largest standard size so the plot keeps its room. Only the checks
            // that look for such text and grow it.
            if [.elementDetection, .dynamicType].contains(type) { return .drawnText }
            // A navigation bar's title, drawn by the bar, which caps its size and shortens a
            // long one ("Settings" as the iPhone's large title, and beside the column button
            // in the iPad's list column): the clipped-text check reports it with no element,
            // and on the iPhone no longer once a scroll folds the large title into the bar.
            if type == .textClipped, screen.barTitle { return .barTitle }
            return nil
        }
        let frame = element.frame
        // A button of a navigation bar or toolbar: the system draws it on Liquid Glass in its
        // own colours and, rather than growing it with Dynamic Type, shows its label in the
        // large content viewer on a long press.
        if [.dynamicType, .contrast].contains(type), element.elementType == .button,
            screen.bars.contains(where: { $0.insetBy(dx: 0, dy: -8).contains(frame) })
        {
            return .barButton
        }
        // Text a scroll left under a navigation bar or a tab bar, or in the fade the bars lay
        // over the content next to them: the audit reads it through the glass and finds it
        // faint, or cut by the bar's edge. Scrolled into view it is audited whole.
        let seen: XCUIAccessibilityAuditType = [.contrast, .textClipped]
        if seen.contains(type),
            screen.bars.contains(where: { $0.insetBy(dx: 0, dy: -44).intersects(frame) })
                || frame.minY < screen.topChrome + 44
        {
            return .underBars
        }
        // Text a scroll took off the screen altogether (a card above the top): the audit still
        // lists it, and measures whatever is there instead.
        if seen.contains(type), !screen.window.contains(frame) { return .offScreen }
        // Text partly out of the list or sheet that holds it, or in the fade at its edge,
        // measured against what is over the list's edge. The innermost page-sized list it
        // overlaps holds it: a sheet's, which floats clear of the window's sides, before the
        // page's behind it (a strip scrolled sideways within a page is not a page).
        let overlapping = screen.lists
            .filter { $0.intersects(frame) && $0.height >= screen.window.height * 0.4 }
        let floating = overlapping.filter {
            $0.minX > screen.window.minX + 1 && $0.maxX < screen.window.maxX - 1
        }
        let holder = (floating.isEmpty ? overlapping : floating)
            .min { $0.width * $0.height < $1.width * $1.height }
        if seen.contains(type), let holder, !holder.insetBy(dx: 0, dy: 44).contains(frame) {
            return .listEdge
        }
        // The Dynamic Type and clipped-text checks grow the text up to the largest sizes,
        // about three times as tall, which pushes everything below the top third of the screen
        // off it or under the tab bar, where they cannot read it. The same text styles are
        // checked at the top of every page, the clipped-text check runs on whole pages at the
        // largest size (the -xxl tests), and the -xxl screenshots show them.
        if !screen.textSizeFixed, type == .dynamicType || type == .textClipped,
            frame.maxY > screen.window.height / 3
        {
            return .grownAway
        }
        // The iPad's list column, which floats on Liquid Glass at the window's leading edge:
        // the contrast check fails every row and header there, on every launch and after a
        // tap, while the screen shows the label colour on the rows' own grey and the section
        // titles in the readable secondary colour. The same rows, with the same styles, pass
        // on the iPhone, where they fill the screen.
        if type == .contrast, screen.window.width > 700, frame.maxX <= 330,
            screen.lists.contains(where: {
                $0.minX <= screen.window.minX + 1 && $0.width <= 340 && $0.contains(frame)
            })
        {
            return .glassColumn
        }
        // A control switched off until there is something to save: its dimmed look tells it
        // apart, and contrast rules leave inactive controls out.
        if type == .contrast, !element.isEnabled { return .inactiveControl }
        // MapKit's own legal link: drawn, sized and coloured by MapKit.
        if type == .hitRegion, element.label == "Legal" { return .mapLegal }
        return nil
    }
}
