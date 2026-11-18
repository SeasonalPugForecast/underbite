import Foundation
import Observation
@preconcurrency import UnderbiteKit

/// The data behind one trip's page: the core's trip view built from the spots' snapshots, the
/// places' histories and the tide stations' metadata, and the refresh that fetches them. The
/// spots are fetched by the spot store, as for their own pages; the requests the core plans for
/// the trip (past years' values, tide constants) go out through a fetch host of their own, as
/// the map's do. Every word on the page is the core's.
@MainActor
@Observable
final class TripPage {
    /// Sends one of the core's requests (the fetch host's `send`; tests stand another in).
    typealias Sender = @Sendable (HostRequest) async throws -> HostResponse

    let tripID: String
    /// The trip as the core words it; `nil` until it is first built.
    private(set) var view: ViewModel.TripView?
    /// Why the view could not be built, in the core's words.
    private(set) var problem: String?
    /// A refresh is under way.
    private(set) var refreshing = false

    @ObservationIgnored private let files: AppFiles
    @ObservationIgnored private let send: Sender
    @ObservationIgnored private let keys: @Sendable () -> Keys
    @ObservationIgnored private let clock: @Sendable () -> Int64
    @ObservationIgnored private let lowPower: @Sendable () -> Bool
    @ObservationIgnored private var kept: TripData?
    @ObservationIgnored private var generation = 0

    /// - Parameters:
    ///   - send: defaults to a fetch host on the app's host directory.
    ///   - clock: Unix seconds, passed on to the core as `now`.
    ///   - keys: read at fetch time from the Keychain, never kept.
    init(
        tripID: String,
        files: AppFiles = .standard,
        send: Sender? = nil,
        keys: @escaping @Sendable () -> Keys = { Keychain.keys() },
        clock: @escaping @Sendable () -> Int64 = { Int64(Date().timeIntervalSince1970) },
        lowPower: @escaping @Sendable () -> Bool = { ProcessInfo.processInfo.isLowPowerModeEnabled }
    ) {
        self.tripID = tripID
        self.files = files
        if let send {
            self.send = send
        } else {
            let host = URLSessionHost(storeURL: files.hostDirectory)
            self.send = { try await host.send(request: $0) }
        }
        self.keys = keys
        self.clock = clock
        self.lowPower = lowPower
    }

    /// What is kept for the trip, read from its file the first time.
    private var data: TripData {
        get {
            if let kept { return kept }
            let read = TripData.read(tripID, files: files)
            kept = read
            return read
        }
        set {
            kept = newValue
            newValue.write(tripID, files: files)
        }
    }

    /// The keys of the trip's places, in its order: each is the spot id of its column.
    nonisolated static func placeKeys(_ trip: SavedTrip, now: Int64) -> [String] {
        ((try? tripSpots(tripJson: trip.json, now: now)) ?? []).map(\.key)
    }

    /// The snapshot of each place that is one of the saved spots.
    private func snapshots(_ keys: [String], spots: SpotStore) -> [String: String] {
        var out: [String: String] = [:]
        for key in keys where spots.spot(key) != nil {
            if let json = spots.cache(key).snapshotJSON { out[key] = json }
        }
        return out
    }

    // MARK: - Building

    /// Builds the view of `trip` from what is held now, off the main actor, and publishes it
    /// unless a newer build has started.
    func build(_ trip: SavedTrip, spots: SpotStore) async {
        generation += 1
        let mine = generation
        let now = clock()
        let keys = Self.placeKeys(trip, now: now)
        let snapshots = snapshots(keys, spots: spots)
        let data = self.data
        let settings = spots.settingsJSON
        let built = await Task.detached(priority: .userInitiated) {
            () -> Result<ViewModel.TripView, any Error> in
            Result {
                try ViewModel.buildTrip(
                    tripJSON: trip.json, places: data.places(keys) { snapshots[$0] },
                    cachedMetadata: data.cachedMetadata, settingsJSON: settings,
                    now: Date(timeIntervalSince1970: TimeInterval(now)))
            }
        }.value
        guard mine == generation else { return }
        switch built {
        case .success(let view):
            self.view = view
            problem = nil
        case .failure(let error):
            problem = SpotStore.message(of: error)
        }
    }

    // MARK: - Refreshing

    /// Fetches what the trip needs, then builds it again. The spots of its places are fetched
    /// through the spot store: every one when `all` (pulling to refresh), else those whose
    /// data is older than the refresh interval allows. Then the core plans the rest from their
    /// snapshots, and the page sends its requests one at a time.
    func refresh(_ trip: SavedTrip, spots: SpotStore, all: Bool) async {
        refreshing = true
        defer { refreshing = false }
        let saved = Self.placeKeys(trip, now: clock()).filter { spots.spot($0) != nil }
        let due: Set<String>
        if all {
            due = Set(saved)
        } else {
            let after = RefreshPolicy.dueAfter(minutes: spots.refreshMinutes, lowPower: lowPower())
            due = RefreshPolicy.due(
                saved.map { ($0, spots.cache($0).lastSuccess) }, now: clock(), after: after)
        }
        if !due.isEmpty { await spots.refresh(only: due) }
        await fetchPlanned(trip, spots: spots)
        await build(trip, spots: spots)
    }

    /// Sends the requests the core plans for `trip` and keeps what they gave.
    private func fetchPlanned(_ trip: SavedTrip, spots: SpotStore) async {
        let now = clock()
        let keys = Self.placeKeys(trip, now: now)
        let snapshots = snapshots(keys, spots: spots)
        let settings = spots.settingsJSON
        let secrets = self.keys()
        let send = self.send
        let held = data
        let fetched = await Task.detached(priority: .userInitiated) { () -> TripData? in
            func makePlan(fresh: Set<String>) throws -> TripPlan {
                try planTrip(
                    tripJson: trip.json,
                    places: held.places(keys, snapshot: { snapshots[$0] }, fresh: fresh),
                    cachedMetadata: held.cachedMetadata, settingsJson: settings, keys: secrets,
                    allowMetered: true, now: now)
            }
            guard var plan = try? makePlan(fresh: []) else { return nil }
            // A place missing part of its history is asked for all of it, so the history that
            // comes back replaces the kept one whole.
            let again = TripData.historyPlaces(plan.requests())
            if !again.isEmpty, let whole = try? makePlan(fresh: again) { plan = whole }
            let requests = plan.requests()
            guard !requests.isEmpty else { return nil }
            var responses: [TripResponse] = []
            for request in requests {
                if Task.isCancelled { break }
                // A request that fails has no response: the core counts it as failed and the
                // cells it would have filled say "unavailable".
                if let response = try? await send(request.request) {
                    responses.append(TripResponse(id: request.id, response: response))
                }
            }
            var next = held
            next.merge(plan.results(responses: responses))
            return next
        }.value
        if let fetched, fetched != held { data = fetched }
    }
}
