import BackgroundTasks
import Foundation

/// The alert checks iOS runs while the app is in the background, as an app-refresh task. iOS
/// decides when, and whether, a requested refresh runs.
enum AlertsBackground {
    /// The task's identifier, listed in the Info.plist's `BGTaskSchedulerPermittedIdentifiers`.
    static let refreshTask = "dev.savery.underbite.refresh"

    /// The soonest a requested refresh may start after the request.
    static let minimumInterval: TimeInterval = 30 * 60

    /// Asks iOS for the next refresh, replacing a pending request; refused requests (no
    /// background refresh, the simulator) are left for the next time the app goes to the
    /// background.
    static func schedule() {
        let request = BGAppRefreshTaskRequest(identifier: refreshTask)
        request.earliestBeginDate = .now.addingTimeInterval(minimumInterval)
        try? BGTaskScheduler.shared.submit(request)
    }

    /// One background refresh for the spots of `store`: the next one is asked for first.
    static func run(_ store: SpotStore) async {
        schedule()
    }
}
