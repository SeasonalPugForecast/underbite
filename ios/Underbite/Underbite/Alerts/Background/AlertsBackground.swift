import BackgroundTasks
import Foundation
import OSLog
@preconcurrency import UnderbiteKit
@preconcurrency import UserNotifications
import WidgetKit

/// The alert checks and widget updates iOS runs while the app is in the background, as an
/// app-refresh task. iOS decides when, and whether, a requested refresh runs, and gives it up to
/// about 30 seconds; when the time is up it cancels the task (``BackgroundRefresh`` says what
/// that leaves).
enum AlertsBackground {
    /// The task's identifier, listed in the Info.plist's `BGTaskSchedulerPermittedIdentifiers`.
    static let refreshTask = "dev.savery.underbite.refresh"

    /// The soonest a requested refresh may start after the request.
    static let minimumInterval: TimeInterval = 30 * 60

    /// How long after its start a refresh still starts a fetch, and how long it waits for one
    /// under way, out of the system's 30 seconds: the rest is for the evaluation, the
    /// notifications and the widget payloads.
    static let mayStartFor: Duration = .seconds(18)
    static let waitFor: Duration = .seconds(24)

    private static let log = Logger(subsystem: "dev.savery.underbite", category: "alerts")

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
        await refresh(store)
    }

    /// Checks the alert rules, posts what is due, and updates the widgets, from what `store`
    /// and `trips` hold now. `postAfter` holds the notifications until then (debug runs).
    @discardableResult
    static func refresh(
        _ store: SpotStore, trips: TripStore = .shared,
        postAfter: ContinuousClock.Instant? = nil
    ) async -> BackgroundRefresh.Outcome {
        let files = AppFiles.standard
        let host = URLSessionHost(storeURL: files.hostDirectory)
        let keys: @Sendable () -> Keys = { Keychain.keys() }
                let live = AlertCheck.live(host: host, keys: keys)
        let refresh = BackgroundRefresh(
            alertFetch: live.spotFetch,
            spotFetch: { spot, settings, now in
                try await fetchSpot(
                    spot: spot, settingsJson: settings, keys: keys(),
                    options: FetchOptions(allowMetered: true, inferWaterType: false), now: now,
                    cachedMetadata: [], host: host)
            },
            gridSend: live.gridSend,
            post: { notices in
                if let postAfter { try? await Task.sleep(until: postAfter) }
                await AlertNotifications.post(notices)
            },
            payloads: .shared,
            reloadWidgets: { await MainActor.run { WidgetCenter.shared.reloadAllTimelines() } },
            files: files,
            budget: FetchBudget(mayStartFor: mayStartFor, waitFor: waitFor))
        let input = BackgroundRefresh.Input(
            settingsJSON: store.settingsJSON, spots: store.arranged, appSpotID: store.activeID,
            trips: trips.trips, widgets: await installedWidgets(),
            timeZone: TimeZone.current.identifier, now: Int64(Date().timeIntervalSince1970))
        let outcome = await refresh.run(input)
        log.info(
            """
            refresh: \(outcome.notices.count, privacy: .public) due, \
            \(outcome.fetched.count, privacy: .public) fetched \
            (\(outcome.fetchedWhole.count, privacy: .public) whole), \
            \(outcome.payloads.count, privacy: .public) payloads, \
            cancelled \(outcome.cancelled, privacy: .public)
            """)
        return outcome
    }

    /// The widgets on the Home and Lock Screens now. Which spot each shows is the widget's own
    /// setting, which only its configuration type reads; until it is read here, each follows
    /// the app's spot.
    static func installedWidgets() async -> [WidgetNeeds.Installed] {
        let widgets = (try? await WidgetCenter.shared.currentConfigurations()) ?? []
        return widgets.map { WidgetNeeds.Installed(kind: WidgetKind(rawValue: $0.kind)) }
    }

    #if DEBUG
        private static var ranOnce = false

        /// `-seedWaterAlert`: a rule on The Battery's water temperature alone (under 20 °C),
        /// which its tide station reports every few minutes without a key, so a screenshot run
        /// sees a notification. The `-seedAlerts` rule also needs a water temperature at the
        /// Croton River, whose gauge reports none.
        static func seedWaterAlert(into store: SpotStore) {
            if store.spot(SeedSpots.battery.id) == nil { store.add(SeedSpots.battery) }
            let rule = AlertRuleDocument(
                id: "5EED0000-0000-4000-8000-0000000000A2", name: "Cool water at the Battery",
                target: .spot(SeedSpots.battery.id),
                allOf: [
                    AlertRuleDocument.Condition(
                        kind: "measurement", metric: "water_temperature", below: 20)
                ])
            guard let json = try? rule.json(),
                let settings = try? putAlertRule(settingsJson: store.settingsJSON, ruleJson: json)
            else { return }
            try? store.updateSettings(settings)
        }
    #endif

    /// Debug builds: `-runBackgroundRefresh` runs one refresh in the foreground, once, as iOS
    /// would in the background, with the rules of `-seedAlerts` and `-seedWaterAlert` saved
    /// first and after the refresh of opening the app (iOS runs one only after the app was
    /// used). A screenshot run cannot answer the permission prompt, so when notifications were
    /// never asked for they are allowed provisionally, which delivers them to Notification
    /// Center without a banner. A number after the flag holds the notifications until that many
    /// seconds after the start, so a screenshot taken then shows the banner where notifications
    /// are allowed. The log (subsystem `dev.savery.underbite`, category `alerts`) lists the
    /// rules found due and the notifications delivered. Release builds do nothing.
    static func runIfAsked(_ store: SpotStore) async {
        #if DEBUG
            let arguments = ProcessInfo.processInfo.arguments
            guard let index = arguments.firstIndex(of: "-runBackgroundRefresh"), !ranOnce else {
                return
            }
            ranOnce = true
            let start = ContinuousClock.now
            let hold = arguments.indices.contains(index + 1) ? Double(arguments[index + 1]) : nil
            if AlertSeeds.wanted { AlertSeeds.seed(into: store) }
            if arguments.contains("-seedWaterAlert") { seedWaterAlert(into: store) }
            if await AlertNotifications.permission() == .notAsked {
                _ = try? await UNUserNotificationCenter.current().requestAuthorization(
                    options: [.alert, .sound, .provisional])
            }
            await store.refreshOnOpen()
            let outcome = await refresh(store, postAfter: hold.map { start + .seconds($0) })
            let delivered = await UNUserNotificationCenter.current().deliveredNotifications()
            let ids = delivered.map(\.request.identifier).sorted().joined(separator: " ")
            let rules = outcome.notices.map(\.ruleId).joined(separator: " ")
            log.info(
                """
                debug refresh after \(ContinuousClock.now - start, privacy: .public): \
                due [\(rules, privacy: .public)], \
                delivered [\(ids, privacy: .public)], \
                problem \(outcome.checkProblem ?? "none")
                """)
        #endif
    }
}
