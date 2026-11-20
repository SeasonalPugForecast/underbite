import Foundation
import UnderbiteKit

/// The conditions of a place that is not a saved spot: a fetch and a page for one transient
/// spot, kept in memory for as long as the preview is open.
///
/// It runs the core's `fetch_spot` and `view_model` itself, through the store's fetch host, and
/// never goes through ``SpotStore``'s saved state: no spot is added, no cache or status is
/// written, no station metadata is kept, nothing reaches the spot list, the widgets, the alerts,
/// the background refresh, the offline packs or iCloud. The fetch sends the rounded place exactly
/// as a saved spot's does. Closing cancels the fetch and clears what the host stored for the
/// preview's id (the snapshot holds the exact place).
@MainActor
@Observable
final class ConditionsPreview: Identifiable {
    /// The preview's own id, never a saved spot's.
    nonisolated let id = "preview-" + UUID().uuidString
    /// The place and what is known of the water there; "Save as spot" starts from it.
    nonisolated let prefill: SpotPrefill
    /// What the navigation bar says when the place has no name: the place, as the core wrote it.
    nonisolated let place: String

    /// The page JSON of the preview's spot; empty until the first fetch has answered.
    private(set) var pageJSON = ""
    /// Whether a fetch is running.
    private(set) var refreshing = false
    /// The water type the core suggested, when it was asked to.
    private(set) var suggestedWaterType: String?
    /// Whether the preview was closed; nothing it started changes it after that.
    private(set) var closed = false

    @ObservationIgnored private let host: FetchHost
    @ObservationIgnored private let keys: @Sendable () -> Keys
    @ObservationIgnored private let clock: @Sendable () -> Int64
    @ObservationIgnored private let fetcher: SpotStore.MetadataFetcher
    @ObservationIgnored private let pageBuilder: SpotStore.PageBuilder
    @ObservationIgnored private let settingsJSON: String
    @ObservationIgnored private var snapshotJSON: String?
    @ObservationIgnored private var lastSuccess: Int64?
    @ObservationIgnored private var error: String?
    @ObservationIgnored private var task: Task<Void, Never>?

    init(
        prefill: SpotPrefill, place: String, settingsJSON: String, host: FetchHost,
        keys: @escaping @Sendable () -> Keys, clock: @escaping @Sendable () -> Int64,
        fetcher: @escaping SpotStore.MetadataFetcher, pageBuilder: @escaping SpotStore.PageBuilder
    ) {
        self.prefill = prefill
        self.place = place
        self.settingsJSON = settingsJSON
        self.host = host
        self.keys = keys
        self.clock = clock
        self.fetcher = fetcher
        self.pageBuilder = pageBuilder
    }

    /// Whether the water type is known (the water's own) rather than left to the core.
    nonisolated var typeKnown: Bool { prefill.waterType != nil }

    /// The transient spot: the place with the name and water type known, else the placeholder
    /// type until a fetch suggests one.
    var spot: Spot {
        Spot(
            id: id, name: prefill.name, latitude: prefill.latitude, longitude: prefill.longitude,
            waterType: prefill.waterType ?? suggestedWaterType ?? Spot.unknownWaterType,
            waterTypeChosen: typeKnown)
    }

    /// What "Save as spot" starts the new-spot flow with: the same place and name, and the
    /// water type known or suggested (none, so the core suggests, when neither is).
    var savePrefill: SpotPrefill {
        SpotPrefill(
            latitude: prefill.latitude, longitude: prefill.longitude, name: prefill.name,
            waterType: prefill.waterType ?? suggestedWaterType)
    }

    /// Fetches the conditions and builds the page; returns when that is done or the preview is
    /// closed. A call while one runs waits for it.
    func load() async {
        if let task {
            await task.value
            return
        }
        guard !closed else { return }
        let running = Task { [weak self] in
            guard let self else { return }
            await self.fetch()
        }
        task = running
        await running.value
        task = nil
    }

    /// Cancels the fetch and forgets what the host stored for the preview. Safe to call twice.
    func close() {
        guard !closed else { return }
        closed = true
        task?.cancel()
        task = nil
        snapshotJSON = nil
        pageJSON = ""
        forgetSpot(spotId: id, host: host)
    }

    private func fetch() async {
        refreshing = true
        defer { refreshing = false }
        let now = clock()
        let current = spot
        let options = FetchOptions(
            allowMetered: true, inferWaterType: !typeKnown, refreshMetadata: false)
        let keys = keys()
        let settings = settingsJSON
        let fetcher = fetcher
        let host = host
        do {
            let outcome = try await fetcher(current.info, settings, keys, options, now, [], host)
            // Closed meanwhile: the fetch stored its snapshot on the host; nothing stays.
            guard !closed else {
                forgetSpot(spotId: id, host: host)
                return
            }
            if let hint = outcome.suggestion { suggestedWaterType = hint.waterType }
            switch FetchStanding(outcome) {
            case .online, .offline:
                snapshotJSON = outcome.snapshotJson
                lastSuccess = now
                error = nil
            case .unreachable(let message):
                error = message
            }
        } catch {
            guard !closed else {
                forgetSpot(spotId: id, host: host)
                return
            }
            self.error = SpotStore.message(of: error)
        }
        await build(now: now)
    }

    /// Builds the page of the preview's spot, alone, from what the fetch left in memory.
    private func build(now: Int64) async {
        guard !closed else { return }
        let entry = SpotEntry(
            spot: spot.info, snapshotJson: snapshotJSON, lastSuccess: lastSuccess, error: error,
            refreshing: false)
        let settings = settingsJSON
        let build = pageBuilder
        let json = await Task.detached(priority: .userInitiated) {
            (try? build([entry], 0, settings, now)) ?? ""
        }.value
        guard !closed else { return }
        pageJSON = json
    }
}
