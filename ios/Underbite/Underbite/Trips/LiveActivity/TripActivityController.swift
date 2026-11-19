import ActivityKit
import Foundation
import OSLog
@preconcurrency import UnderbiteKit

/// Starts a trip's Live Activity when asked from its page on one of its days (iOS lets the app
/// start one only while it is open), keeps it current after each refresh, and ends it after
/// sunset (else at the end of the day), when the trip is gone or no longer has a day today, or
/// when asked. One activity per trip (``TripActivityMatch``). Where the system does not let the
/// app start Live Activities (turned off in Settings, or not shown on the device),
/// ``enabled`` is false and nothing is offered.
nonisolated enum TripActivityController {
    private static let log = Logger(subsystem: "dev.savery.underbite", category: "live-activity")

    /// Whether the app may start a Live Activity now.
    static var enabled: Bool { ActivityAuthorizationInfo().areActivitiesEnabled }

    /// The activities still shown and updatable.
    private static var live: [Activity<TripDayAttributes>] {
        Activity<TripDayAttributes>.activities.filter {
            $0.activityState == .active || $0.activityState == .stale
        }
    }

    private static var running: [TripActivityMatch.Running] {
        live.map { TripActivityMatch.Running(id: $0.id, tripID: $0.attributes.tripID) }
    }

    /// Whether trip `tripID` has an activity running.
    static func isRunning(_ tripID: String) -> Bool {
        running.contains { $0.tripID == tripID }
    }

    /// Shows `content` for trip `tripID`: its running activity is updated (any repeat ended), or
    /// one is started. Returns whether one runs now.
    @discardableResult
    static func show(_ tripID: String, content: ViewModel.TripDayActivity) async -> Bool {
        let plan = TripActivityMatch.plan(running: running, tripID: tripID)
        await apply(plan.end.map { .end(id: $0) } + (plan.keep.map { [.update(id: $0, content: content)] } ?? []))
        guard plan.keep == nil else { return true }
        do {
            _ = try Activity.request(
                attributes: TripDayAttributes(tripID: tripID), content: TripDayAttributes.content(content))
            return true
        } catch {
            log.error("could not start: \(String(describing: error), privacy: .public)")
            return false
        }
    }

    /// Ends trip `tripID`'s activity and takes it off the Lock Screen.
    static func stop(_ tripID: String) async {
        await apply(running.filter { $0.tripID == tripID }.map { .end(id: $0.id) })
    }

    /// After a refresh: each running activity updated with its trip's content from the spots'
    /// last snapshots in `store`, or ended (``TripActivityMatch/refresh(running:content:now:)``).
    /// Only the trips with an activity are built.
    @MainActor
    static func refresh(
        store: SpotStore, trips: TripStore = .shared, files: AppFiles = .standard, now: Date = .now
    ) async {
        let running = self.running
        guard !running.isEmpty else { return }
        let t = Int64(now.timeIntervalSince1970)
        let settings = store.settingsJSON
        let steps = TripActivityMatch.refresh(
            running: running,
            content: { id in
                guard let trip = trips.trip(id) else { return nil }
                let held = snapshots(of: trip, spots: store, now: t)
                return TripActivityContent.current(
                    trip, settingsJSON: settings, snapshot: { held[$0] }, files: files, now: t)
            }, now: now)
        await apply(steps)
    }

    /// The last snapshot of each of `trip`'s places that is a saved spot, by place key.
    @MainActor
    static func snapshots(of trip: SavedTrip, spots: SpotStore, now: Int64) -> [String: String] {
        var out: [String: String] = [:]
        for key in TripPage.placeKeys(trip, now: now) where spots.spot(key) != nil {
            if let json = spots.cache(key).snapshotJSON { out[key] = json }
        }
        return out
    }

    /// Updates and ends the activities `steps` name.
    private static func apply(_ steps: [TripActivityMatch.Step]) async {
        guard !steps.isEmpty else { return }
        for activity in Activity<TripDayAttributes>.activities {
            for step in steps {
                switch step {
                case .update(let id, let content) where id == activity.id:
                    await activity.update(TripDayAttributes.content(content))
                case .end(let id) where id == activity.id:
                    await activity.end(nil, dismissalPolicy: .immediate)
                default:
                    continue
                }
            }
        }
    }
}
