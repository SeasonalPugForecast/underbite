import Foundation

/// A trip's days are calendar dates, `YYYY-MM-DD`, that mean the same day at each of its places;
/// a date picker works with instants in the phone's zone and calendar. These turn the day a
/// picker shows into that date and back, as a day and never as an instant. The text is the
/// trip's data, never shown: the core words the dates.
nonisolated enum TripDates {
    /// The date of the day `date` falls on for the phone (`zone`), in the Gregorian calendar
    /// the trip is written in, whatever calendar the picker shows it in.
    static func day(of date: Date, in zone: TimeZone = .current) -> String {
        let parts = gregorian(zone).dateComponents([.year, .month, .day], from: date)
        return text(year: parts.year ?? 1970, month: parts.month ?? 1, day: parts.day ?? 1)
    }

    /// Noon of the day `day` (`YYYY-MM-DD`) for the phone (`zone`), the instant a picker shows
    /// as that day; `nil` when `day` is not a date.
    static func date(of day: String, in zone: TimeZone = .current) -> Date? {
        guard let parts = parts(of: day) else { return nil }
        return gregorian(zone).date(
            from: DateComponents(year: parts.year, month: parts.month, day: parts.day, hour: 12))
    }

    /// `days` days after `day` (before it when negative); `nil` when `day` is not a date.
    static func adding(_ days: Int, to day: String) -> String? {
        let utc = TimeZone(identifier: "UTC") ?? .gmt
        guard let start = date(of: day, in: utc),
            let moved = gregorian(utc).date(byAdding: .day, value: days, to: start)
        else { return nil }
        return self.day(of: moved, in: utc)
    }

    /// How many days `last` is after `first` (negative when before); `nil` unless both are dates.
    static func days(from first: String, to last: String) -> Int? {
        let utc = TimeZone(identifier: "UTC") ?? .gmt
        guard let a = date(of: first, in: utc), let b = date(of: last, in: utc) else { return nil }
        return gregorian(utc).dateComponents([.day], from: a, to: b).day
    }

    /// The year, month and day of `YYYY-MM-DD`, when it is a date that exists.
    static func parts(of day: String) -> (year: Int, month: Int, day: Int)? {
        let fields = day.split(separator: "-", omittingEmptySubsequences: false)
        guard fields.count == 3, fields[0].count == 4, fields[1].count == 2, fields[2].count == 2,
            let year = Int(fields[0]), let month = Int(fields[1]), let date = Int(fields[2])
        else { return nil }
        let utc = TimeZone(identifier: "UTC") ?? .gmt
        let calendar = gregorian(utc)
        let parts = DateComponents(year: year, month: month, day: date, hour: 12)
        guard parts.isValidDate(in: calendar) else { return nil }
        return (year, month, date)
    }

    private static func text(year: Int, month: Int, day: Int) -> String {
        String(format: "%04d-%02d-%02d", year, month, day)
    }

    private static func gregorian(_ zone: TimeZone) -> Calendar {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = zone
        return calendar
    }
}
