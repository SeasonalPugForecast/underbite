import Foundation

/// When spots are fetched, and in what order. Foundation only.
nonisolated enum RefreshPolicy {
    /// How often data refreshes, in minutes.
    static let choices = [15, 30, 60]
    static let defaultMinutes = 15

    /// How old a spot's data may be on opening the app before it is fetched again: two thirds
    /// of the refresh interval, four times that in Low Power Mode.
    static func dueAfter(minutes: Int, lowPower: Bool) -> TimeInterval {
        Double(minutes) * 60 * 2 / 3 * (lowPower ? 4 : 1)
    }

    /// The ids whose data is older than `after` seconds at `now` (Unix seconds), or missing.
    static func due(
        _ spots: [(id: String, lastSuccess: Int64?)], now: Int64, after: TimeInterval
    ) -> Set<String> {
        Set(spots.filter { spot in spot.lastSuccess.map { Double(now - $0) > after } ?? true }.map(\.id))
    }

    /// Whether a followed place at `old` moves to `new`, a fix good to `accuracy` metres: the
    /// phone is 1 km or more away however far off the fix is. A negative accuracy never moves.
    static func shouldMove(
        from old: (lat: Double, lon: Double), to new: (lat: Double, lon: Double), accuracy: Double = 0
    ) -> Bool {
        guard accuracy >= 0 else { return false }
        return distance(old, new) - accuracy >= 1_000
    }

    /// Great-circle distance in metres (haversine, mean Earth radius).
    static func distance(_ a: (lat: Double, lon: Double), _ b: (lat: Double, lon: Double)) -> Double {
        let r = 6_371_000.0
        let p1 = a.lat * .pi / 180
        let p2 = b.lat * .pi / 180
        let dp = p2 - p1
        let dl = (b.lon - a.lon) * .pi / 180
        let h = sin(dp / 2) * sin(dp / 2) + cos(p1) * cos(p2) * sin(dl / 2) * sin(dl / 2)
        return 2 * r * asin(min(1, sqrt(h)))
    }

    /// How long a location fix answers for a new one.
    static let fixLasts: TimeInterval = 10 * 60
}

/// What a place search may send of where the phone is. Foundation only.
nonisolated enum SearchArea {
    /// The centre of the area to search around the phone at `latitude`, `longitude`: rounded to
    /// one decimal (about 11 km), so the map service learns only an approximate area.
    static func centre(latitude: Double, longitude: Double) -> (latitude: Double, longitude: Double) {
        ((latitude * 10).rounded() / 10, (longitude * 10).rounded() / 10)
    }

    /// The span of the area searched, metres.
    static let span: Double = 200_000
}

/// The spots a running refresh still has to fetch, in order. A refresh asked for while another
/// runs joins it: ids not already waiting are added.
nonisolated struct RefreshQueue: Equatable {
    private(set) var waiting: [String] = []

    var isEmpty: Bool { waiting.isEmpty }

    /// Adds `ids` that are not waiting yet.
    mutating func add(_ ids: [String]) {
        for id in ids where !waiting.contains(id) { waiting.append(id) }
    }

    /// The next spot to fetch.
    mutating func next() -> String? {
        waiting.isEmpty ? nil : waiting.removeFirst()
    }

    /// Forgets what is waiting (a cancelled refresh).
    mutating func clear() { waiting = [] }
}
