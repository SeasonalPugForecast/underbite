import Foundation
import XCTest

/// A picked day is the trip's date as a calendar day, in the phone's zone, never an instant.
final class TripDatesTests: XCTestCase {
    private func zone(_ id: String) throws -> TimeZone { try XCTUnwrap(TimeZone(identifier: id)) }

    func testTheDayIsTheOneThePhoneShows() throws {
        // 2027-07-12 02:00 UTC: still the 11th in New York, already the 12th in Tokyo.
        let instant = Date(timeIntervalSince1970: 1_815_357_600)
        XCTAssertEqual(TripDates.day(of: instant, in: try zone("America/New_York")), "2027-07-11")
        XCTAssertEqual(TripDates.day(of: instant, in: try zone("Asia/Tokyo")), "2027-07-12")
        XCTAssertEqual(TripDates.day(of: instant, in: try zone("UTC")), "2027-07-12")
    }

    func testADayComesBackAsTheSameDayInEveryZone() throws {
        for id in ["America/New_York", "Pacific/Kiritimati", "Pacific/Pago_Pago", "Europe/London"] {
            let z = try zone(id)
            // The days the clocks change, and the turn of the year.
            for day in ["2027-03-14", "2027-11-07", "2027-03-28", "2027-12-31", "2028-02-29"] {
                let date = try XCTUnwrap(TripDates.date(of: day, in: z), "\(day) in \(id)")
                XCTAssertEqual(TripDates.day(of: date, in: z), day, "\(day) in \(id)")
            }
        }
    }

    func testDaysAddAndCountAsCalendarDays() {
        XCTAssertEqual(TripDates.adding(2, to: "2027-07-12"), "2027-07-14")
        XCTAssertEqual(TripDates.adding(1, to: "2027-02-28"), "2027-03-01")
        XCTAssertEqual(TripDates.adding(1, to: "2028-02-28"), "2028-02-29")
        XCTAssertEqual(TripDates.adding(-1, to: "2027-01-01"), "2026-12-31")
        XCTAssertEqual(TripDates.adding(30, to: "2027-03-01"), "2027-03-31")
        XCTAssertEqual(TripDates.days(from: "2027-03-13", to: "2027-03-15"), 2)
        XCTAssertEqual(TripDates.days(from: "2027-07-12", to: "2027-07-11"), -1)
        XCTAssertNil(TripDates.days(from: "", to: "2027-07-11"))
    }

    func testOnlyRealDatesAreRead() {
        XCTAssertNil(TripDates.parts(of: "2027-02-30"))
        XCTAssertNil(TripDates.parts(of: "2027-7-12"))
        XCTAssertNil(TripDates.parts(of: "12/07/2027"))
        XCTAssertNil(TripDates.date(of: "tomorrow"))
        XCTAssertEqual(TripDates.parts(of: "2028-02-29")?.day, 29)
    }
}
