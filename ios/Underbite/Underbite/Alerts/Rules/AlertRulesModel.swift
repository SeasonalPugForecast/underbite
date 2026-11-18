import Foundation
import Observation
@preconcurrency import UnderbiteKit

/// The alert rules screen's state: the rules as the core lists them (words and last state),
/// the screen's fixed words, the notification permission, and "check now". The rules live in
/// the settings; what the last check found lives in ``AlertStateFile``.
@MainActor
@Observable
final class AlertRulesModel {
    /// The rules in the settings' order, with their words and states.
    private(set) var rows: [AlertRuleRow] = []
    /// Why the rules could not be listed, in the core's words.
    private(set) var listProblem: String?
    /// The screen's fixed words in the settings' voice.
    private(set) var texts: AlertTexts?
    /// The system's answer; `nil` until asked.
    private(set) var permission: NotificationPermission?
    /// A check is under way.
    private(set) var checking = false
    /// What the last "check now" found, since the screen opened.
    private(set) var lastCheck: CheckResult?

    /// What a check found: the notifications due, or why it could not run.
    struct CheckResult: Equatable {
        var notices: [AlertNotice]
        var error: String?
    }

    @ObservationIgnored let files: AppFiles
    @ObservationIgnored let stateFile: AlertStateFile
    @ObservationIgnored private let check: AlertCheck
    @ObservationIgnored private let clock: @Sendable () -> Int64

    init(
        files: AppFiles = .standard,
        clock: @escaping @Sendable () -> Int64 = { Int64(Date().timeIntervalSince1970) }
    ) {
        self.files = files
        stateFile = AlertStateFile(files: files)
        check = .live(
            host: URLSessionHost(storeURL: files.hostDirectory), keys: { Keychain.keys() })
        self.clock = clock
    }

    /// The saved spots by id and name, for the core's words.
    static func spotNames(_ spots: [Spot]) -> [NamedPlace] {
        spots.map { NamedPlace(id: $0.id, name: $0.name) }
    }

    /// The saved trips by id and name: the list's name, else the one in the trip's JSON.
    static func tripNames(_ trips: TripStore) -> [NamedPlace] {
        trips.trips.map { trip in
            let name = trips.listing(trip.id)?.title ?? Self.name(inTrip: trip.json) ?? ""
            return NamedPlace(id: trip.id, name: name)
        }
    }

    private static func name(inTrip json: String) -> String? {
        let object = try? JSONSerialization.jsonObject(with: Data(json.utf8))
        return (object as? [String: Any])?["name"] as? String
    }

    /// Lists the rules again for `settingsJSON` (after an edit, a check, or a spot or trip
    /// added or deleted).
    func reload(settingsJSON: String, spots: [Spot], trips: TripStore) {
        texts = try? alertTexts(settingsJson: settingsJSON)
        let spotNames = Self.spotNames(spots), tripNames = Self.tripNames(trips)
        let now = clock()
        do {
            rows = try stateFile.using { state in
                try alertRules(
                    settingsJson: settingsJSON, spots: spotNames, trips: tripNames, state: state,
                    timeZone: TimeZone.current.identifier, now: now)
            }
            listProblem = nil
        } catch {
            rows = []
            listProblem = AlertWords.message(of: error)
        }
    }

    /// Reads the notification permission again (the screen opened, or the app came back from
    /// Settings).
    func refreshPermission() async {
        permission = await AlertNotifications.permission()
    }

    /// Asks for permission when it has never been asked.
    func askIfNeeded() async {
        if permission == nil { await refreshPermission() }
        guard permission == .notAsked else { return }
        permission = await AlertNotifications.ask()
    }

    /// Checks the enabled rules now: fetches what they read, evaluates them against the kept
    /// state, keeps the new state, posts what is due (when allowed) and lists the rules again.
    func checkNow(store: SpotStore, trips: TripStore) async {
        guard !checking else { return }
        checking = true
        defer { checking = false }
        await askIfNeeded()
        let settings = store.settingsJSON
        let files = self.files
        do {
            let outcome = try await check.run(
                settingsJSON: settings, spots: store.arranged, trips: trips.trips.map(\.json),
                lastSnapshot: { files.lastSnapshot($0) }, state: stateFile,
                timeZone: TimeZone.current.identifier, now: clock())
            lastCheck = CheckResult(notices: outcome.run.notifications, error: nil)
            await AlertNotifications.post(outcome.run.notifications)
        } catch {
            lastCheck = CheckResult(notices: [], error: AlertWords.message(of: error))
        }
        reload(settingsJSON: store.settingsJSON, spots: store.arranged, trips: trips)
    }
}
