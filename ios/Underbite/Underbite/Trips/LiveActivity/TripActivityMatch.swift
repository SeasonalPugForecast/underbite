import Foundation
@preconcurrency import UnderbiteKit

/// Which trip-day Live Activities to keep, update or end, and what a trip's page offers. Each
/// activity is keyed by its trip's id, so a renamed trip keeps its activity and two trips never
/// share one; a second activity of the same trip (started twice, or by an older run) is ended
/// rather than left to pile up. Pure logic, apart from ActivityKit, for the tests.
nonisolated enum TripActivityMatch {
    /// What the controller knows of a running activity.
    struct Running: Equatable, Sendable {
        /// ActivityKit's id for the activity.
        var id: String
        /// The trip it was started for (its attributes cannot change once started).
        var tripID: String
    }

    /// The activity of `tripID` to keep updating (the first one) and the other activities of
    /// that trip to end. Other trips' activities are left as they are.
    static func plan(running: [Running], tripID: String) -> (keep: String?, end: [String]) {
        let mine = running.filter { $0.tripID == tripID }
        return (mine.first?.id, mine.dropFirst().map(\.id))
    }

    /// What to do with one running activity.
    enum Step: Equatable, Sendable {
        /// Show `content` from now on.
        case update(id: String, content: ViewModel.TripDayActivity)
        /// End it and take it off the Lock Screen.
        case end(id: String)
    }

    /// After a refresh: each running activity updated with its trip's content at `now`
    /// (`content`, by trip id, `nil` when the trip is gone or today is none of its days), or
    /// ended when there is none, when it has nothing left to count down to, when it is over
    /// (after sunset, else at the end of the day), or when it repeats an activity of the same
    /// trip. `content` is asked once per trip.
    static func refresh(
        running: [Running], content: (String) -> ViewModel.TripDayActivity?, now: Date
    ) -> [Step] {
        var seen = Set<String>()
        return running.map { activity in
            guard seen.insert(activity.tripID).inserted,
                let built = content(activity.tripID), shows(built, at: now)
            else { return .end(id: activity.id) }
            return .update(id: activity.id, content: built)
        }
    }

    /// What a trip's page shows about the Live Activity.
    enum Offer: Equatable, Sendable {
        /// Nothing: Live Activities are off or unsupported, today is not a trip day, or the
        /// day's activity is over.
        case hidden
        /// The button that starts one.
        case offer
        /// It runs: the button that stops it.
        case running
    }

    /// The page's offer, from whether the system lets the app start activities (`enabled`),
    /// today's content (`nil` when today is not one of the trip's days) and whether the trip's
    /// activity runs.
    static func offer(
        enabled: Bool, content: ViewModel.TripDayActivity?, running: Bool, now: Date
    ) -> Offer {
        guard enabled, let content, shows(content, at: now) else { return .hidden }
        return running ? .running : .offer
    }

    /// Whether `content` is worth showing at `now`: something to count down to, and not over.
    static func shows(_ content: ViewModel.TripDayActivity, at now: Date) -> Bool {
        !content.targets.isEmpty && !content.isOver(at: now)
    }
}
