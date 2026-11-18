import Foundation
import XCTest

/// The trip page's layout: places as columns only on a regular width with room for them, and a
/// place's cells paired into rows with the long ones alone.
final class TripGridLayoutTests: XCTestCase {
    private func form(
        regular: Bool = true, accessibility: Bool = false, width: Double = 800, places: Int = 2,
        column: Double = 220
    ) -> TripGridLayout.Form {
        TripGridLayout.form(
            regular: regular, accessibilitySize: accessibility, width: width, places: places,
            minimumColumn: column, spacing: 12)
    }

    func testARegularWidthWithRoomShowsPlacesAsColumns() {
        XCTAssertEqual(form(), .grid)
        XCTAssertEqual(form(width: 452), .grid, "two columns of 220 and the space between")
        XCTAssertEqual(form(width: 300, places: 1, column: 300), .grid)
    }

    func testACompactWidthStacksTheDays() {
        XCTAssertEqual(form(regular: false), .stacked)
        XCTAssertEqual(form(regular: false, width: 2_000), .stacked)
    }

    func testLargeTextOrManyPlacesStackEvenOnARegularWidth() {
        XCTAssertEqual(form(accessibility: true, width: 2_000), .stacked)
        XCTAssertEqual(form(width: 451), .stacked, "the columns no longer fit")
        XCTAssertEqual(form(places: 4), .stacked)
        XCTAssertEqual(form(column: 440), .stacked, "columns grown with the text size")
    }

    func testNoPlacesOrNoWidthYetStack() {
        XCTAssertEqual(form(places: 0), .stacked)
        XCTAssertEqual(form(width: 0), .stacked)
    }

    private static let kinds = [
        "verdict", "windows", "tides", "flow", "water_temp", "sun", "moon", "solunar",
    ]

    func testCellsPairUpWithTheLongOnesAlone() {
        XCTAssertEqual(
            TripGridLayout.rows(Self.kinds, columns: 2), [[0, 1], [2], [3, 4], [5, 6], [7]])
    }

    func testOneColumnPutsEveryCellOnItsOwnRow() {
        XCTAssertEqual(TripGridLayout.rows(Self.kinds, columns: 1), (0..<8).map { [$0] })
        XCTAssertEqual(TripGridLayout.rows(Self.kinds, columns: 0), (0..<8).map { [$0] })
    }

    func testALongCellEndsTheRowBeforeIt() {
        XCTAssertEqual(TripGridLayout.rows(["flow", "tides", "sun"], columns: 2), [[0], [1], [2]])
        XCTAssertEqual(TripGridLayout.rows(["a", "b", "c"], columns: 2), [[0, 1], [2]])
        XCTAssertEqual(TripGridLayout.rows([], columns: 2), [])
    }

    func testTheTidesShowEveryHighAndLow() {
        let lines = ["High 4:30 PM, 5.6 ft", "Low 11:00 PM, 0.4 ft"]
        XCTAssertEqual(
            TripGridLayout.shownLines(kind: "tides", value: lines[0], lines: lines), lines)
        XCTAssertEqual(TripGridLayout.shownLines(kind: "tides", value: "—", lines: []), ["—"])
        XCTAssertEqual(
            TripGridLayout.shownLines(kind: "flow", value: "706 cfs", lines: ["In your range"]),
            ["706 cfs"])
    }
}
