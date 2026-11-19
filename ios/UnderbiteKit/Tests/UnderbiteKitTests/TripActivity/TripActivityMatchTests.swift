import Foundation
import XCTest

@testable import UnderbiteKit

/// One trip-day Live Activity per trip: which to keep, update and end, and what a trip's page
/// offers.
final class TripActivityMatchTests: XCTestCase {
    private typealias Running = TripActivityMatch.Running

    private func sample() throws -> ViewModel.TripDayActivity {
        try ViewModel.decodeTripActivity(Fixtures.text("sample-trip-activity"))
    }

    private func at(_ t: Int64) -> Date { Date(timeIntervalSince1970: TimeInterval(t)) }

    // MARK: - Starting

    func testATripWithoutAnActivityKeepsNoneAndEndsNone() {
        let plan = TripActivityMatch.plan(
            running: [Running(id: "a", tripID: "other")], tripID: "trip")
        XCTAssertNil(plan.keep)
        XCTAssertEqual(plan.end, [])
    }

    func testTheFirstActivityOfATripIsKeptAndItsRepeatsEnded() {
        let running = [
            Running(id: "a", tripID: "other"), Running(id: "b", tripID: "trip"),
            Running(id: "c", tripID: "trip"), Running(id: "d", tripID: "trip"),
        ]
        let plan = TripActivityMatch.plan(running: running, tripID: "trip")
        XCTAssertEqual(plan.keep, "b")
        // Another trip's activity is not this trip's to end.
        XCTAssertEqual(plan.end, ["c", "d"])
    }

    // MARK: - Refreshing

    func testARunningActivityIsUpdatedWithItsTripsContent() throws {
        let content = try sample()
        var asked: [String] = []
        let steps = TripActivityMatch.refresh(
            running: [Running(id: "a", tripID: "trip")],
            content: {
                asked.append($0)
                return content
            }, now: at(content.generatedAt))
        XCTAssertEqual(steps, [.update(id: "a", content: content)])
        XCTAssertEqual(asked, ["trip"])
    }

    func testAnActivityWhoseTripHasNoContentIsEnded() throws {
        // The trip was deleted, or today is none of its days.
        let content = try sample()
        let steps = TripActivityMatch.refresh(
            running: [Running(id: "a", tripID: "gone"), Running(id: "b", tripID: "trip")],
            content: { $0 == "trip" ? content : nil }, now: at(content.generatedAt))
        XCTAssertEqual(steps, [.end(id: "a"), .update(id: "b", content: content)])
    }

    func testAnActivityIsEndedOnceItsDayIsOver() throws {
        let content = try sample()
        let running = [Running(id: "a", tripID: "trip")]
        XCTAssertEqual(
            TripActivityMatch.refresh(running: running, content: { _ in content }, now: content.endDate - 1),
            [.update(id: "a", content: content)])
        XCTAssertEqual(
            TripActivityMatch.refresh(running: running, content: { _ in content }, now: content.endDate),
            [.end(id: "a")])
    }

    func testAnActivityWithNothingToCountDownToIsEnded() throws {
        var content = try sample()
        content.nextTideChange = nil
        content.slack = nil
        content.sunset = nil
        XCTAssertEqual(
            TripActivityMatch.refresh(
                running: [Running(id: "a", tripID: "trip")], content: { _ in content },
                now: at(content.generatedAt)),
            [.end(id: "a")])
    }

    func testARepeatOfATripsActivityIsEndedAndItsContentBuiltOnce() throws {
        let content = try sample()
        var asked: [String] = []
        let steps = TripActivityMatch.refresh(
            running: [
                Running(id: "a", tripID: "trip"), Running(id: "b", tripID: "other"),
                Running(id: "c", tripID: "trip"),
            ],
            content: {
                asked.append($0)
                return content
            }, now: at(content.generatedAt))
        XCTAssertEqual(
            steps,
            [.update(id: "a", content: content), .update(id: "b", content: content), .end(id: "c")])
        XCTAssertEqual(asked, ["trip", "other"])
    }

    // MARK: - The page's offer

    func testTheOfferFollowsTheSystemTheDayAndTheActivity() throws {
        let content = try sample()
        let now = at(content.generatedAt)
        XCTAssertEqual(
            TripActivityMatch.offer(enabled: true, content: content, running: false, now: now), .offer)
        XCTAssertEqual(
            TripActivityMatch.offer(enabled: true, content: content, running: true, now: now), .running)
        // Live Activities off, or not shown on this device: nothing is said.
        XCTAssertEqual(
            TripActivityMatch.offer(enabled: false, content: content, running: false, now: now), .hidden)
        XCTAssertEqual(
            TripActivityMatch.offer(enabled: false, content: content, running: true, now: now), .hidden)
        // Not a trip day.
        XCTAssertEqual(
            TripActivityMatch.offer(enabled: true, content: nil, running: false, now: now), .hidden)
        // After sunset.
        XCTAssertEqual(
            TripActivityMatch.offer(enabled: true, content: content, running: false, now: content.endDate),
            .hidden)
    }

    func testNothingToCountDownToOffersNothing() throws {
        var content = try sample()
        content.nextTideChange = nil
        content.slack = nil
        content.sunset = nil
        XCTAssertEqual(
            TripActivityMatch.offer(
                enabled: true, content: content, running: false, now: at(content.generatedAt)),
            .hidden)
    }
}
