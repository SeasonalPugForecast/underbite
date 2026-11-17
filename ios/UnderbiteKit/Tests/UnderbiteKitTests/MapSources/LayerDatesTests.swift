import Foundation
import XCTest

@testable import UnderbiteKit

/// The dates and times of the dated layers: the satellite day and its steps back, the times a
/// grid's axis offers, "last clear day", the radar loop and the over-zoomed frame tiles.
final class LayerDatesTests: XCTestCase {
    /// 2026-11-17 12:00 UTC.
    private let now: Int64 = 1_794_916_800

    private func previous(_ date: String) -> MapDate? { try? previousDate(date: date) }

    private func time(_ t: Int64, _ label: String = "") -> LayerTime {
        LayerTime(time: t, label: label.isEmpty ? String(t) : label)
    }

    // MARK: - Which layer the controls are for

    private func plan(shown: [String], order: [String]) throws -> MapPlan {
        var settings = try Settings.defaults()
        for i in settings.map.layers.indices {
            settings.map.layers[i].shown = shown.contains(settings.map.layers[i].id)
        }
        let moved = order.compactMap { id in settings.map.layers.first { $0.id == id } }
        settings.map.layers = settings.map.layers.filter { !order.contains($0.id) } + moved
        let catalogue = try mapCatalogue(settingsJson: try settings.json())
        return MapPlan.make(catalogue: catalogue, settings: settings.map, battery: false)
    }

    func testTheControlsAreForTheTopmostDatedLayer() throws {
        let all = ["gebco-relief", "sst", "true-color-viirs-n20", "radar", "hf-radar-currents"]
        let radarOnTop = try plan(
            shown: all, order: ["sst", "true-color-viirs-n20", "hf-radar-currents", "radar"])
        XCTAssertEqual(DateControl.topmost(radarOnTop)?.layer.id, "radar")
        XCTAssertEqual(DateControl.topmost(radarOnTop)?.control, .radarFrames)

        let imageryOnTop = try plan(
            shown: all, order: ["radar", "hf-radar-currents", "sst", "true-color-viirs-n20"])
        XCTAssertEqual(DateControl.topmost(imageryOnTop)?.control, .satelliteDays)

        let currentsOnTop = try plan(shown: all, order: ["sst", "hf-radar-currents"])
        XCTAssertEqual(DateControl.topmost(currentsOnTop)?.layer.id, "hf-radar-currents")

        let sst = try plan(shown: ["sst", "gebco-relief"], order: ["sst", "gebco-relief"])
        // GEBCO has no dates: the controls go to the dated layer under it.
        XCTAssertEqual(DateControl.topmost(sst)?.layer.id, "sst")
        XCTAssertEqual(DateControl.topmost(sst)?.control, .gridDays)

        let hours = try plan(shown: ["hf-radar-currents"], order: [])
        XCTAssertEqual(DateControl.topmost(hours)?.control, .gridHours)

        XCTAssertNil(DateControl.topmost(try plan(shown: ["gebco-relief"], order: [])))
    }

    // MARK: - Satellite days

    func testASatelliteLayerOpensOnYesterdayInUTC() {
        let day = gibsDefaultDate(now: now)
        XCTAssertEqual(day.date, "2026-11-16")
        XCTAssertFalse(day.label.isEmpty)
        // A minute past midnight UTC is still "today is partial".
        XCTAssertEqual(gibsDefaultDate(now: 1_794_960_060).date, "2026-11-17")
    }

    func testANoDataDayStepsBackADayAndShowsTheDayServed() throws {
        var day = SatelliteDay(chosen: gibsDefaultDate(now: now))
        // One "no data" tile is not yet a missing day.
        XCTAssertNil(day.update(images: 0, empty: 1, previous: previous))
        let back = try XCTUnwrap(day.update(images: 0, empty: 2, previous: previous))
        XCTAssertEqual(back.date, "2026-11-15")
        XCTAssertEqual(day.asked, back)
        XCTAssertEqual(day.chosen.date, "2026-11-16")
        XCTAssertEqual(day.steps, 1)
        // The day before has pictures: it stays, whatever "no data" holes follow.
        XCTAssertNil(day.update(images: 1, empty: 0, previous: previous))
        XCTAssertTrue(day.served)
        XCTAssertNil(day.update(images: 1, empty: 9, previous: previous))
        XCTAssertEqual(day.asked.date, "2026-11-15")
    }

    func testAPictureAmongNoDataTilesKeepsTheDay() {
        var day = SatelliteDay(chosen: gibsDefaultDate(now: now))
        XCTAssertNil(day.update(images: 1, empty: 5, previous: previous))
        XCTAssertEqual(day.asked.date, "2026-11-16")
        XCTAssertEqual(day.steps, 0)
    }

    func testSteppingBackStopsAfterSevenDays() {
        var day = SatelliteDay(chosen: gibsDefaultDate(now: now), picked: true)
        var asked: [String] = []
        while let back = day.update(images: 0, empty: 3, previous: previous) {
            asked.append(back.date)
        }
        XCTAssertEqual(asked.count, SatelliteDay.maxSteps)
        XCTAssertEqual(asked.first, "2026-11-15")
        XCTAssertEqual(asked.last, "2026-11-09")
        XCTAssertTrue(day.exhausted)
        XCTAssertTrue(day.picked)
        XCTAssertNil(day.update(images: 0, empty: 9, previous: previous))
        XCTAssertEqual(day.asked.date, "2026-11-09")
    }

    func testThePickerOffersTwoWeeksOfDaysNewestFirstWithTheCoresLabels() throws {
        let days = SatelliteDay.days(from: gibsDefaultDate(now: now), previous: previous)
        XCTAssertEqual(days.count, SatelliteDay.pickerDays)
        XCTAssertEqual(days.first?.date, "2026-11-16")
        XCTAssertEqual(days.last?.date, "2026-11-03")
        // Across a month's end, and every label is the core's.
        let march = try XCTUnwrap(previous("2026-03-02"))
        let across = SatelliteDay.days(from: march, count: 3, previous: previous)
        XCTAssertEqual(across.map(\.date), ["2026-03-01", "2026-02-28", "2026-02-27"])
        XCTAssertEqual(across[1].label, "Feb 28")
    }

    // MARK: - Grid times

    func testAnAxisIsKeptADayOrAnHourForAnHourlyLayer() {
        XCTAssertFalse(GridTimes.isStale(fetched: now, now: now + 86_399, dates: .daily))
        XCTAssertTrue(GridTimes.isStale(fetched: now, now: now + 86_400, dates: .daily))
        XCTAssertFalse(GridTimes.isStale(fetched: now, now: now + 3_599, dates: .hourly))
        XCTAssertTrue(GridTimes.isStale(fetched: now, now: now + 3_600, dates: .hourly))
        // A clock set back does not keep an axis from the future.
        XCTAssertTrue(GridTimes.isStale(fetched: now, now: now - 10, dates: .daily))
    }

    func testAnHourlyLayerOffersTheLastDayAndADailyOneTheWholeAxis() {
        let hours = (0..<48).map { time(now - Int64(47 - $0) * 3_600) }
        let offered = GridTimes.offered(hours, dates: .hourly, now: now)
        XCTAssertEqual(offered.count, 25)
        XCTAssertEqual(offered.last?.time, now)
        XCTAssertEqual(offered.first?.time, now - 86_400)
        // An axis whose newest hour is older than a day still offers that hour.
        let old = [time(now - 200_000), time(now - 190_000)]
        XCTAssertEqual(GridTimes.offered(old, dates: .hourly, now: now).map(\.time), [now - 190_000])
        let days = (0..<11).map { time(now - Int64(10 - $0) * 86_400) }
        XCTAssertEqual(GridTimes.offered(days, dates: .daily, now: now), days)
    }

    func testOnlyTimesInTheAxisAreAskedForAndTheNewestAsTheLayerOpens() {
        let axis = [time(100), time(200), time(300)]
        XCTAssertEqual(GridTimes.checked(200, in: axis), 200)
        // The newest is asked for as none: what the layer opens on.
        XCTAssertNil(GridTimes.checked(300, in: axis))
        // Past the end, or revised out of the axis: the newest.
        XCTAssertNil(GridTimes.checked(400, in: axis))
        XCTAssertNil(GridTimes.checked(250, in: axis))
        XCTAssertNil(GridTimes.checked(nil, in: axis))
        XCTAssertNil(GridTimes.checked(200, in: []))
        XCTAssertEqual(GridTimes.shown(nil, in: axis)?.time, 300)
        XCTAssertEqual(GridTimes.shown(100, in: axis)?.time, 100)
        XCTAssertNil(GridTimes.shown(150, in: axis))
    }

    func testStepsMoveAlongTheOfferedTimesAndStopAtTheEnds() {
        let axis = [time(100), time(200), time(300)]
        XCTAssertEqual(GridTimes.step(from: nil, by: -1, in: axis), .some(200))
        XCTAssertEqual(GridTimes.step(from: 200, by: -1, in: axis), .some(100))
        XCTAssertEqual(GridTimes.step(from: 200, by: 1, in: axis), .some(nil))
        // No step past either end.
        XCTAssertTrue(GridTimes.step(from: nil, by: 1, in: axis) == nil)
        XCTAssertTrue(GridTimes.step(from: 100, by: -1, in: axis) == nil)
        XCTAssertTrue(GridTimes.step(from: nil, by: -1, in: []) == nil)
    }

    func testLastClearDayTriesTheNewestDaysNewestFirst() {
        let axis = (1...12).map { time(Int64($0) * 86_400) }
        let tries = GridTimes.clearDayTries(axis)
        XCTAssertEqual(tries.count, SatelliteDay.maxSteps + 1)
        XCTAssertEqual(tries.first, .some(nil))
        XCTAssertEqual(tries[1], 11 * 86_400)
        XCTAssertEqual(tries.last, 5 * 86_400)
        XCTAssertEqual(GridTimes.clearDayTries([time(5)]), [nil])
    }

    func testADayIsClearOnceValuesShowAndCloudyOnceBlocksStopArriving() {
        var clear = ClearDayWatch(revision: 3)
        XCTAssertEqual(clear.check(hasValues: false, revision: 3), .waiting)
        XCTAssertEqual(clear.check(hasValues: true, revision: 4), .clear)

        var cloudy = ClearDayWatch(revision: 3)
        XCTAssertEqual(cloudy.check(hasValues: false, revision: 4), .waiting)
        XCTAssertEqual(cloudy.check(hasValues: false, revision: 5), .waiting)
        for _ in 1..<ClearDayWatch.quiet {
            XCTAssertEqual(cloudy.check(hasValues: false, revision: 5), .waiting)
        }
        XCTAssertEqual(cloudy.check(hasValues: false, revision: 5), .cloudy)

        // The day on screen already: quiet alone makes it cloudy.
        var onScreen = ClearDayWatch(revision: 7, arrived: true)
        for _ in 1..<ClearDayWatch.quiet {
            XCTAssertEqual(onScreen.check(hasValues: false, revision: 7), .waiting)
        }
        XCTAssertEqual(onScreen.check(hasValues: false, revision: 7), .cloudy)

        // Nothing ever arrives: not "cloudy", only unknown in the end.
        var silent = ClearDayWatch(revision: 0)
        for _ in 1..<ClearDayWatch.longest {
            XCTAssertEqual(silent.check(hasValues: false, revision: 0), .waiting)
        }
        XCTAssertEqual(silent.check(hasValues: false, revision: 0), .unknown)
    }

    // MARK: - Radar loop

    func testTheLoopHoldsTheLatestFrameThenStartsOver() {
        var step = 0
        var shown: [Int] = []
        for _ in 0..<8 {
            let next = RadarLoopRules.next(after: step, count: 4)
            step = next.step
            shown.append(next.index)
        }
        XCTAssertEqual(shown, [1, 2, 3, 3, 3, 3, 0, 1])
        XCTAssertEqual(RadarLoopRules.next(after: 5, count: 0).index, 0)
        XCTAssertTrue(RadarLoopRules.isDue(loaded: nil, now: now))
        XCTAssertFalse(RadarLoopRules.isDue(loaded: now - 299, now: now))
        XCTAssertTrue(RadarLoopRules.isDue(loaded: now - 300, now: now))
    }

    func testAFrameTilePastTheRadarsZoomIsCutFromItsAncestor() {
        let same = OverZoomedTile(z: 9, x: 150, y: 192, deepest: 10)
        XCTAssertEqual([same.z, same.x, same.y], [9, 150, 192])
        XCTAssertNil(same.crop)
        let deeper = OverZoomedTile(z: 12, x: 1206, y: 1539, deepest: 10)
        XCTAssertEqual([deeper.z, deeper.x, deeper.y], [10, 301, 384])
        XCTAssertEqual(deeper.crop?.size, 0.25)
        XCTAssertEqual(deeper.crop?.left, 0.5)
        XCTAssertEqual(deeper.crop?.top, 0.75)
    }
}
