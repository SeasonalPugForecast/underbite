import Foundation
@preconcurrency import UnderbiteKit

/// One check of the enabled rules, in the core's three steps: `plan_alerts` says what to fetch,
/// most important first; each spot is fetched with `fetch_alert_spot` (only what its rules read)
/// and each box's grid request is sent through the app's host; `evaluate_alerts` then words the
/// notifications due from the latest snapshots and the kept state, and gives the state to keep.
///
/// A spot whose fetch fails is checked against its last snapshot, which the core finds too old
/// or empty and says so; a grid request without an answer is left out.
nonisolated struct AlertCheck: Sendable {
    /// Fetches one spot of the plan at `now` with the settings.
    typealias SpotFetch =
        @Sendable (AlertFetch, String, Int64) async throws -> SpotFetchOutcome
    /// Sends one grid request.
    typealias GridSend = @Sendable (HostRequest) async throws -> HostResponse

    let spotFetch: SpotFetch
    let gridSend: GridSend

    /// Through `host` (the app's `URLSessionHost`), with the keys read when a fetch starts.
    static func live(host: FetchHost, keys: @escaping @Sendable () -> Keys) -> AlertCheck {
        AlertCheck(
            spotFetch: { fetch, settings, now in
                try await fetchAlertSpot(
                    fetch: fetch, settingsJson: settings, keys: keys(), allowMetered: true,
                    now: now, cachedMetadata: [], host: host)
            },
            gridSend: { request in try await host.send(request: request) })
    }

    /// What a check did: the core's run, and the spots it fetched with their snapshots.
    struct Outcome: Sendable {
        var run: AlertRun
        /// The snapshot of each spot fetched, by id.
        var fetched: [String: String]
    }

    /// Checks the rules of `settingsJSON` at Unix time `now`: `spots` are the saved spots,
    /// `trips` the saved trips' JSON, `lastSnapshot` a spot's kept snapshot (the dashboard's),
    /// `timeZone` the phone's. The state is read from and written to `state`.
    func run(
        settingsJSON: String, spots: [Spot], trips: [String],
        lastSnapshot: @Sendable (String) -> String?, state: AlertStateFile, timeZone: String,
        now: Int64
    ) async throws -> Outcome {
        let infos = spots.map(\.info)
        let steps = try planAlerts(settingsJson: settingsJSON, spots: infos, trips: trips, now: now)
        var fetched: [String: String] = [:]
        // Trip places that are not saved spots, fetched for a trip's rules.
        var places: [String: SpotInfo] = [:]
        var grids: [AlertGridResponse] = []
        for step in steps {
            try Task.checkCancellation()
            switch step {
            case .spot(let fetch):
                if let outcome = try? await spotFetch(fetch, settingsJSON, now) {
                    fetched[fetch.spot.id] = outcome.snapshotJson
                }
                if !infos.contains(where: { $0.id == fetch.spot.id }) {
                    places[fetch.spot.id] = fetch.spot
                }
            case .grid(let grid):
                if let response = try? await gridSend(grid.request) {
                    grids.append(AlertGridResponse(key: grid.key, response: response))
                }
            }
        }
        // A check cancelled during its last fetch is not evaluated either: the state stays.
        try Task.checkCancellation()
        var data = infos.map { info in
            AlertSpotData(spot: info, snapshotJson: fetched[info.id] ?? lastSnapshot(info.id))
        }
        data += places.keys.sorted().compactMap { id in
            places[id].map { AlertSpotData(spot: $0, snapshotJson: fetched[id]) }
        }
        let spotData = data
        let gridData = grids
        let run = try state.using { kept in
            try evaluateAlerts(
                settingsJson: settingsJSON, spots: spotData, trips: trips, grids: gridData,
                state: kept, timeZone: timeZone, now: now)
        }
        state.write(run.state)
        return Outcome(run: run, fetched: fetched)
    }
}

extension AppFiles {
    /// The snapshot the dashboard keeps for spot `id`, read from its cache file.
    nonisolated func lastSnapshot(_ id: String) -> String? {
        read(SpotCache.self, cacheName(id))?.snapshotJSON
    }
}
