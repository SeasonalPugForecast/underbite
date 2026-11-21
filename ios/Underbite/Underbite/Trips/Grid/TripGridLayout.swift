import Foundation

/// How the trip page lays out the core's grid: the choice between places as columns and days
/// stacked as cards, and how a place's cells pair up into rows. Foundation only; it reads the
/// kinds of the cells, never their values.
nonisolated enum TripGridLayout {
    /// The two forms of the page.
    enum Form: Equatable {
        /// Days as rows, places as columns.
        case grid
        /// One card per day, its places one under the other.
        case stacked
    }

    /// The form for a page `width` points wide holding `places` columns of at least
    /// `minimumColumn` points (already scaled with the text size) `spacing` apart. Only a
    /// regular width shows the grid, and only when its columns fit: at large text sizes, or
    /// with many places, the stacked form takes over even on an iPad. The accessibility sizes
    /// always stack.
    static func form(
        regular: Bool, accessibilitySize: Bool, width: Double, places: Int,
        minimumColumn: Double, spacing: Double
    ) -> Form {
        guard regular, !accessibilitySize, places > 0, width > 0 else { return .stacked }
        let needed = Double(places) * minimumColumn + Double(places - 1) * spacing
        return needed <= width ? .grid : .stacked
    }

    /// The narrowest width of a place's cells, in points, that still pairs two cells side by
    /// side: about 340 pt for the tile, less its 24 pt of padding. A narrower column (a
    /// place column on an iPad, a small phone) shows one cell per row.
    static let minimumPairWidth: Double = 316

    /// How many cells a row holds in a column `width` points wide (0 while it is not measured
    /// yet, which keeps the pair). The accessibility sizes always show one.
    static func cellColumns(accessibilitySize: Bool, width: Double) -> Int {
        if accessibilitySize { return 1 }
        return width > 0 && width < minimumPairWidth ? 1 : 2
    }

    /// The kinds whose value is long or comes as several lines (every high and low, every
    /// solunar period): they take a row of their own.
    static let wideKinds: Set<String> = ["tides", "solunar"]

    /// The cells of a place's day as rows of indices into `kinds`, in order: up to `columns`
    /// cells side by side, a wide kind alone on its row.
    static func rows(_ kinds: [String], columns: Int) -> [[Int]] {
        let columns = max(columns, 1)
        var rows: [[Int]] = []
        var row: [Int] = []
        for (index, kind) in kinds.enumerated() {
            if wideKinds.contains(kind) || columns == 1 {
                if !row.isEmpty { rows.append(row) }
                rows.append([index])
                row = []
                continue
            }
            row.append(index)
            if row.count == columns {
                rows.append(row)
                row = []
            }
        }
        if !row.isEmpty { rows.append(row) }
        return rows
    }

    /// The kinds whose every line shows in the grid: each high and low of the day.
    static let allLinesKinds: Set<String> = ["tides"]

    /// The lines a cell shows under its label: every line of a kind that lists its events,
    /// else the value alone. The other lines are on the day's page.
    static func shownLines(kind: String, value: String, lines: [String]) -> [String] {
        allLinesKinds.contains(kind) && !lines.isEmpty ? lines : [value]
    }

    /// The lines of a verdict cell on the day's page without the reasons, which the page lists
    /// itself under the verdict: no reason is said twice.
    static func linesWithoutReasons(_ lines: [String], reasons: [String]) -> [String] {
        lines.filter { !reasons.contains($0) }
    }
}
