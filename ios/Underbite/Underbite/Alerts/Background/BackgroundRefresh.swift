import Foundation
@preconcurrency import UnderbiteKit

/// One background refresh, in the order that matters most when it is cut short:
///
/// 1. the alert check (``AlertCheck``): the core's plan, a fetch of only what the enabled rules
///    read (a spot a widget also shows is fetched whole instead, once), the evaluation against
///    the kept state, the new state kept;
/// 2. the notifications the check found due, posted when the system allows;
/// 3. a whole fetch of each spot an installed widget shows that the check did not fetch;
/// 4. the widget payloads of every saved spot and of the next trip, from the freshest snapshot
///    of each, the list of spots for the widgets' settings, and one reload of the widgets.
///
/// Requests carry the spots' coordinates rounded by the core. The ``FetchBudget`` keeps the
/// refresh inside the system's time: no fetch starts once it is spent, and a fetch still under
/// way at its end is no longer waited for. The check then evaluates with the last snapshot of a
/// spot it did not fetch, and the payloads are written in time.
///
/// When the task is cancelled (the system's time is up) the refresh stops waiting for the fetch
/// under way and returns at once. Every file is written whole, so what it leaves is consistent:
/// cancelled before the evaluation, the rule state is the previous check's and nothing was
/// posted; after it, the new state is kept and its notifications posted; the payloads stay as
/// the last refresh wrote them.
nonisolated struct BackgroundRefresh: Sendable {
    /// Fetches a whole spot (the core's `fetch_spot`) with the settings at Unix time `now`.
    typealias SpotFetch = @Sendable (SpotInfo, String, Int64) async throws -> SpotFetchOutcome

    /// Fetches what a spot's rules read (the core's `fetch_alert_spot`).
    let alertFetch: AlertCheck.SpotFetch
    let spotFetch: SpotFetch
    let gridSend: AlertCheck.GridSend
    /// Posts the notifications due (``AlertNotifications/post(_:)``).
    let post: @Sendable ([AlertNotice]) async -> Void
    let payloads: WidgetPayloadStore
    /// Asks WidgetKit to read the payloads again: once per refresh, as it limits reloads.
    let reloadWidgets: @Sendable () async -> Void
    /// The app's files: the rule state, the spots' last snapshots, the trips' kept data.
    let files: AppFiles
    let budget: FetchBudget

    /// What the app holds when the refresh starts.
    struct Input: Sendable {
        var settingsJSON: String
        /// The saved spots, in the list's order.
        var spots: [Spot]
        /// The spot the app shows, which "the app's" widgets follow.
        var appSpotID: String?
        var trips: [SavedTrip]
        /// The widgets installed now.
        var widgets: [WidgetNeeds.Installed]
        /// The phone's IANA zone.
        var timeZone: String
        /// Unix seconds.
        var now: Int64
    }

    /// What a refresh did.
    struct Outcome: Equatable, Sendable {
        /// The notifications the check found due (posted when allowed).
        var notices: [AlertNotice] = []
        /// Why the check could not run, in the core's words.
        var checkProblem: String?
        /// The spots fetched, in order, and whether whole (for a widget) or for rules only.
        var fetched: [String] = []
        var fetchedWhole: [String] = []
        /// The spots whose payload was written.
        var payloads: [String] = []
        /// The trip whose payload was written as the next trip's.
        var nextTrip: String?
        var reloaded = false
        /// The task was cancelled before the refresh was done.
        var cancelled = false
    }

    @concurrent
    func run(_ input: Input) async -> Outcome {
        var outcome = Outcome()
        let saved = Set(input.spots.map(\.id))
        let nextTripID = UnderbiteKit.nextTrip(trips: input.trips.map(\.json), now: input.now)
        let nextTripJSON = input.trips.first { $0.id == nextTripID }?.json
        let whole = Set(widgetSpots(input, nextTrip: nextTripJSON, saved: saved))
        let log = FetchLog()

        // 1, 2: the rules, then their notifications.
        let alertFetch = self.alertFetch, spotFetch = self.spotFetch, gridSend = self.gridSend
        let budget = self.budget
        let check = AlertCheck(
            spotFetch: { fetch, settings, now in
                let id = fetch.spot.id
                let isWhole = whole.contains(id)
                return try await Self.fetching(id, whole: isWhole, log: log, budget: budget) {
                    isWhole
                        ? try await spotFetch(fetch.spot, settings, now)
                        : try await alertFetch(fetch, settings, now)
                }
            },
            gridSend: { request in
                guard budget.mayStart else { throw FetchBudget.Spent() }
                return try await Abandoning.run(until: budget.waitUntil) {
                    try await gridSend(request)
                }
            })
        do {
            let checked = try await check.run(
                settingsJSON: input.settingsJSON, spots: input.spots,
                trips: input.trips.map(\.json), lastSnapshot: { [files] in files.lastSnapshot($0) },
                state: AlertStateFile(files: files), timeZone: input.timeZone, now: input.now)
            outcome.notices = checked.run.notifications
        } catch is CancellationError {
            log.report(into: &outcome)
            outcome.cancelled = true
            return outcome
        } catch {
            outcome.checkProblem = AlertWords.message(of: error)
        }
        if !outcome.notices.isEmpty { await post(outcome.notices) }

        // 3: the widgets' spots the rules did not fetch.
        for spot in input.spots where whole.contains(spot.id) && !log.has(spot.id) {
            guard !Task.isCancelled else { break }
            _ = try? await Self.fetching(spot.id, whole: true, log: log, budget: budget) {
                try await spotFetch(spot.info, input.settingsJSON, input.now)
            }
        }
        log.report(into: &outcome)
        if Task.isCancelled {
            outcome.cancelled = true
            return outcome
        }

        // 4: the payloads, then one reload.
        let fresh = log.snapshots
        let snapshot: (String) -> String? = { [files] id in
            fresh[id] ?? files.lastSnapshot(id)
        }
        for spot in input.spots {
            guard let json = snapshot(spot.id),
                let payload = try? spotWidget(
                    spot: spot.info, snapshotJson: json, settingsJson: input.settingsJSON,
                    now: input.now),
                (try? payloads.saveSpot(payload)) != nil
            else { continue }
            outcome.payloads.append(spot.id)
        }
        outcome.nextTrip = writeNextTrip(
            nextTripID, json: nextTripJSON, input: input,
            snapshot: { saved.contains($0) ? snapshot($0) : nil })
        try? payloads.saveSpots(
            WidgetSpots(
                spots: input.spots.map { WidgetSpot(id: $0.id, name: $0.name) },
                appSpotID: input.appSpotID))
        try? payloads.keepSpots(input.spots.map(\.id))
        await reloadWidgets()
        outcome.reloaded = true
        return outcome
    }

    /// The spots the installed widgets show, fetched whole.
    private func widgetSpots(_ input: Input, nextTrip: String?, saved: Set<String>) -> [String] {
        let tripPlaces = nextTrip.map {
            WidgetNeeds.tripPlaces($0, saved: saved, now: input.now)
        }
        return WidgetNeeds.spotIDs(
            installed: input.widgets,
            spots: WidgetSpots(
                spots: input.spots.map { WidgetSpot(id: $0.id, name: $0.name) },
                appSpotID: input.appSpotID),
            nextTripPlaces: tripPlaces ?? [])
    }

    /// Writes the next trip's payload, or removes it when there is no next trip or its payload
    /// cannot be built; returns the trip written.
    private func writeNextTrip(
        _ id: String?, json: String?, input: Input, snapshot: (String) -> String?
    ) -> String? {
        guard let id, let json else {
            try? payloads.removeNextTrip()
            return nil
        }
        let keys = ((try? tripSpots(tripJson: json, now: input.now)) ?? []).map(\.key)
        let places = TripData.read(id, files: files).places(keys, snapshot: snapshot)
        guard
            let payload = try? tripWidget(
                tripJson: json, places: places, settingsJson: input.settingsJSON, now: input.now),
            (try? payloads.saveNextTrip(payload)) != nil
        else {
            try? payloads.removeNextTrip()
            return nil
        }
        return id
    }

    /// Runs one fetch of spot `id` unless the budget is spent, without waiting past its end or
    /// a cancellation, and logs its snapshot.
    private static func fetching(
        _ id: String, whole: Bool, log: FetchLog, budget: FetchBudget,
        _ body: @escaping @Sendable () async throws -> SpotFetchOutcome
    ) async throws -> SpotFetchOutcome {
        guard budget.mayStart else { throw FetchBudget.Spent() }
        let outcome = try await Abandoning.run(until: budget.waitUntil, body)
        log.keep(id, whole: whole, outcome)
        return outcome
    }
}

/// The spots a refresh fetched and their snapshots, from the fetches running beside it.
nonisolated private final class FetchLog: @unchecked Sendable {
    private let lock = NSLock()
    private var order: [(id: String, whole: Bool)] = []
    private var kept: [String: String] = [:]

    func keep(_ id: String, whole: Bool, _ outcome: SpotFetchOutcome) {
        lock.withLock {
            order.append((id, whole))
            // Nothing answered (the core says so): the phone is offline, and the older snapshot stays.
            if !outcome.unanswered { kept[id] = outcome.snapshotJson }
        }
    }

    func has(_ id: String) -> Bool { lock.withLock { order.contains { $0.id == id } } }

    var snapshots: [String: String] { lock.withLock { kept } }

    func report(into outcome: inout BackgroundRefresh.Outcome) {
        let order = lock.withLock { self.order }
        outcome.fetched = order.map(\.id)
        outcome.fetchedWhole = order.filter(\.whole).map(\.id)
    }
}

/// Waits for work that does not stop when its task is cancelled (a fetch through the core,
/// whose requests run in tasks of their own) only until the task is cancelled, or until
/// `deadline`: then it throws `CancellationError` (or ``FetchBudget/Spent``) at once and the
/// work's result is dropped.
nonisolated enum Abandoning {
    static func run<T: Sendable>(
        until deadline: ContinuousClock.Instant? = nil,
        _ operation: @escaping @Sendable () async throws -> T
    ) async throws -> T {
        try Task.checkCancellation()
        let gate = Gate<T>()
        return try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { continuation in
                // Cancelled already: the work is not started.
                guard gate.wait(continuation) else { return }
                gate.start(
                    Task {
                        do {
                            gate.finish(.success(try await operation()))
                        } catch {
                            gate.finish(.failure(error))
                        }
                    })
                if let deadline {
                    gate.start(
                        Task {
                            guard (try? await Task.sleep(until: deadline)) != nil else { return }
                            gate.finish(.failure(FetchBudget.Spent()))
                        })
                }
            }
        } onCancel: {
            gate.finish(.failure(CancellationError()))
        }
    }

    /// The continuation, resumed once by whichever ends first: the work, the deadline or the
    /// cancellation. The others are then cancelled.
    nonisolated private final class Gate<T: Sendable>: @unchecked Sendable {
        private let lock = NSLock()
        private var continuation: CheckedContinuation<T, any Error>?
        private var early: Result<T, any Error>?
        private var done = false
        private var tasks: [Task<Void, Never>] = []

        /// Keeps `continuation` for the result; resumes it at once, and returns `false`, when the
        /// task was cancelled before.
        func wait(_ continuation: CheckedContinuation<T, any Error>) -> Bool {
            let ready: Result<T, any Error>? = lock.withLock {
                if let early {
                    done = true
                    return early
                }
                self.continuation = continuation
                return nil
            }
            guard let ready else { return true }
            continuation.resume(with: ready)
            return false
        }

        func finish(_ result: Result<T, any Error>) {
            let (waiting, others): (CheckedContinuation<T, any Error>?, [Task<Void, Never>]) =
                lock.withLock {
                    guard !done else { return (nil, []) }
                    guard let continuation else {
                        if early == nil { early = result }
                        return (nil, [])
                    }
                    done = true
                    self.continuation = nil
                    return (continuation, tasks)
                }
            waiting?.resume(with: result)
            for task in others { task.cancel() }
        }

        /// Keeps `task` to cancel once the wait is over; cancels it at once when it is.
        func start(_ task: Task<Void, Never>) {
            let over = lock.withLock {
                tasks.append(task)
                return done
            }
            if over { task.cancel() }
        }
    }
}

/// How long a background refresh fetches, out of the system's time: a fetch may start until
/// `mayStartFor` after the refresh began, and one under way is waited for until `waitFor`.
nonisolated struct FetchBudget: Sendable {
    /// Why a fetch was not started, or no longer waited for.
    struct Spent: Error {}

    let startBy: ContinuousClock.Instant
    let waitUntil: ContinuousClock.Instant

    init(mayStartFor: Duration, waitFor: Duration, from start: ContinuousClock.Instant = .now) {
        startBy = start + mayStartFor
        waitUntil = start + max(mayStartFor, waitFor)
    }

    /// Whether a fetch may still start.
    var mayStart: Bool { ContinuousClock.now < startBy }
}
