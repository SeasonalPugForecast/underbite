import Foundation
import Observation
@preconcurrency import UnderbiteKit

/// What the store keeps in `state.json`.
nonisolated private struct StoredState: Codable {
    var version = 1
    var spots: [Spot] = []
    var areas: [Area] = []
    var activeID: String?
    var refreshMinutes = RefreshPolicy.defaultMinutes
}

/// One row of the spot list and the page header, as `view_model` returns it: the core wrote
/// every word, the store only decodes the ones the shell shows until the view-model types land.
nonisolated struct SpotListing: Decodable, Identifiable, Equatable, Sendable {
    var id: String
    var name: String
    var waterTypeLabel: String
    var status: Status

    struct Status: Decodable, Equatable, Sendable {
        var lastSuccess: Int64?
        /// "updated 6 min ago", written by the core.
        var updated: String?
        var error: String?
        var refreshing: Bool
    }
}

/// The part of the page JSON the store reads itself.
nonisolated private struct PageHead: Decodable {
    var spots: [SpotListing]
}

/// The spots, their cached data and the refresh, shared by every window of the process. The
/// store holds no formatted value: the list's words and the page come from the core's
/// `view_model`. Exact coordinates stay here; the core rounds what a fetch sends.
@MainActor
@Observable
final class SpotStore {
    /// The one store of the process: every scene, every size class and every iPad window shares it.
    static let shared = SpotStore()

    /// Runs one spot's fetch (the core's `fetch_spot`; tests may stand another in).
    typealias Fetcher =
        @Sendable (SpotInfo, String, Keys, FetchOptions, Int64, FetchHost) async throws
        -> SpotFetchOutcome

    private(set) var spots: [Spot] = []
    private(set) var areas: [Area] = []
    private(set) var activeID: String?
    /// How often data refreshes, in minutes (one of `RefreshPolicy.choices`).
    private(set) var refreshMinutes = RefreshPolicy.defaultMinutes
    /// The settings, current version, as JSON.
    private(set) var settingsJSON: String
    /// What migrating the saved settings changed or found wrong.
    private(set) var settingsIssues: [SettingsIssue] = []
    /// The ids being fetched now.
    private(set) var refreshing: Set<String> = []
    /// The list rows of the last page build, in list order.
    private(set) var listing: [SpotListing] = []
    /// The page JSON of the active spot (`view_model`), for the views to decode.
    private(set) var pageJSON = ""
    /// Where the phone is, when it was asked; never saved unless the user saves it as a spot.
    private(set) var currentPlace: CurrentPlace?

    /// A fix of the phone.
    struct CurrentPlace: Equatable {
        var latitude: Double
        var longitude: Double
    }

    @ObservationIgnored private let files: AppFiles
    @ObservationIgnored private let host: FetchHost
    @ObservationIgnored private let keys: @Sendable () -> Keys
    @ObservationIgnored private let clock: @Sendable () -> Int64
    @ObservationIgnored private let lowPower: @Sendable () -> Bool
    @ObservationIgnored private let fetcher: Fetcher
    @ObservationIgnored private var caches: [String: SpotCache] = [:]
    @ObservationIgnored private var queue = RefreshQueue()
    @ObservationIgnored private var running = false
    @ObservationIgnored private var waiters: [CheckedContinuation<Void, Never>] = []
    @ObservationIgnored private var generation = 0
    @ObservationIgnored private var pendingSettingsSave: Task<Void, Never>?

    /// - Parameters:
    ///   - clock: Unix seconds; the app's one clock read, passed on to the core as `now`.
    ///   - keys: read at fetch time from the Keychain, never kept.
    init(
        files: AppFiles = .standard,
        host: FetchHost? = nil,
        keys: @escaping @Sendable () -> Keys = { Keychain.keys() },
        clock: @escaping @Sendable () -> Int64 = { Int64(Date().timeIntervalSince1970) },
        lowPower: @escaping @Sendable () -> Bool = { ProcessInfo.processInfo.isLowPowerModeEnabled },
        fetcher: @escaping Fetcher = { spot, settings, keys, options, now, host in
            try await fetchSpot(
                spot: spot, settingsJson: settings, keys: keys, options: options, now: now,
                host: host)
        }
    ) {
        self.files = files
        self.host = host ?? URLSessionHost(storeURL: files.hostDirectory)
        self.keys = keys
        self.clock = clock
        self.lowPower = lowPower
        self.fetcher = fetcher
        settingsJSON = defaultSettingsJson()
        load()
    }

    // MARK: - Loading

    /// Before the first frame: the state, the settings and the cache of the shown spot only.
    private func load() {
        if let state = files.read(StoredState.self, "state.json") {
            spots = state.spots
            areas = state.areas
            activeID = state.activeID
            refreshMinutes =
                RefreshPolicy.choices.contains(state.refreshMinutes)
                ? state.refreshMinutes : RefreshPolicy.defaultMinutes
        }
        loadSettings()
        if activeID == nil || !spots.contains(where: { $0.id == activeID }) {
            activeID = arranged.first?.id
        }
        if let activeID { _ = cache(activeID) }
    }

    /// The saved settings, migrated on every load; unreadable ones leave the defaults.
    private func loadSettings() {
        guard let saved = files.text("settings.json") else { return }
        do {
            let migrated = try migrateSettings(json: saved)
            settingsJSON = migrated.json
            settingsIssues = migrated.issues
            if migrated.json != saved { saveSettingsNow() }
        } catch {
            settingsIssues = []
        }
    }

    /// The cache of `id`, read from its file the first time.
    func cache(_ id: String) -> SpotCache {
        if let held = caches[id] { return held }
        let read = files.read(SpotCache.self, files.cacheName(id)) ?? SpotCache()
        caches[id] = read
        return read
    }

    // MARK: - Spots

    /// The spots in list order.
    var arranged: [Spot] { Spot.arranged(spots) }

    var activeSpot: Spot? { spots.first { $0.id == activeID } }

    func spot(_ id: String) -> Spot? { spots.first { $0.id == id } }

    func listing(for id: String) -> SpotListing? { listing.first { $0.id == id } }

    /// Adds `spot` at the end of the user's order.
    func add(_ spot: Spot) {
        guard self.spot(spot.id) == nil else { return }
        var spot = spot
        spot.order = (spots.map(\.order).max() ?? -1) + 1
        spots.append(spot)
        if activeID == nil { activeID = spot.id }
        saveState()
    }

    func update(_ spot: Spot) {
        guard let index = spots.firstIndex(where: { $0.id == spot.id }) else { return }
        spots[index] = spot
        saveState()
    }

    func remove(_ id: String) {
        spots.removeAll { $0.id == id }
        caches[id] = nil
        files.remove(files.cacheName(id))
        if activeID == id { activeID = arranged.first?.id }
        saveState()
    }

    /// Puts the spots in the user's order: `ids` first, in that order.
    func reorder(_ ids: [String]) {
        for (position, id) in ids.enumerated() {
            if let index = spots.firstIndex(where: { $0.id == id }) { spots[index].order = position }
        }
        saveState()
    }

    func setPinned(_ id: String, _ pinned: Bool) {
        change(id) { $0.pinned = pinned }
    }

    func setFavourite(_ id: String, _ favourite: Bool) {
        change(id) { $0.favourite = favourite }
    }

    func assign(_ id: String, toArea areaID: String?) {
        change(id) { $0.areaID = areaID }
    }

    private func change(_ id: String, _ edit: (inout Spot) -> Void) {
        guard let index = spots.firstIndex(where: { $0.id == id }) else { return }
        edit(&spots[index])
        saveState()
    }

    func addArea(named name: String) -> Area {
        let area = Area(name: name, order: (areas.map(\.order).max() ?? -1) + 1)
        areas.append(area)
        saveState()
        return area
    }

    func removeArea(_ id: String) {
        areas.removeAll { $0.id == id }
        for index in spots.indices where spots[index].areaID == id { spots[index].areaID = nil }
        saveState()
    }

    func setRefreshMinutes(_ minutes: Int) {
        guard RefreshPolicy.choices.contains(minutes) else { return }
        refreshMinutes = minutes
        saveState()
    }

    /// Makes `id` the shown spot and builds its page.
    func show(_ id: String) async {
        setActive(id)
        await rebuildPage()
    }

    /// Makes `id` the shown spot; its cache is read now, the others stay on disk.
    func setActive(_ id: String) {
        guard spot(id) != nil else { return }
        activeID = id
        _ = cache(id)
        saveState()
    }

    // MARK: - Current location

    /// Records a fix (about 100 m): returns whether the phone moved 1 km or more from the last
    /// one the store holds (or there was none). Held in memory only.
    @discardableResult
    func noteFix(latitude: Double, longitude: Double, accuracy: Double = 0) -> Bool {
        let moved =
            currentPlace.map {
                RefreshPolicy.shouldMove(
                    from: ($0.latitude, $0.longitude), to: (latitude, longitude), accuracy: accuracy)
            } ?? true
        if moved { currentPlace = CurrentPlace(latitude: latitude, longitude: longitude) }
        return moved
    }

    /// Saves where the phone is as a spot; the only way the location is ever stored.
    @discardableResult
    func saveCurrentPlace(named name: String, waterType: String, timeZone: String) -> Spot? {
        guard let here = currentPlace else { return nil }
        let spot = Spot(
            name: name, latitude: here.latitude, longitude: here.longitude, waterType: waterType,
            waterTypeChosen: false, timeZone: timeZone)
        add(spot)
        return spot
    }

    // MARK: - Settings

    /// Replaces the settings with `json` (migrated first); saved a second after the last change.
    func updateSettings(_ json: String) throws {
        let migrated = try migrateSettings(json: json)
        settingsJSON = migrated.json
        settingsIssues = migrated.issues
        pendingSettingsSave?.cancel()
        pendingSettingsSave = Task { [weak self] in
            try? await Task.sleep(for: .seconds(1))
            guard !Task.isCancelled else { return }
            self?.saveSettingsNow()
        }
    }

    /// Writes a pending settings change now (the app is going to the background).
    func saveSettingsNow() {
        pendingSettingsSave?.cancel()
        pendingSettingsSave = nil
        files.write(Data(settingsJSON.utf8), "settings.json")
    }

    // MARK: - Refresh

    /// Fetches the spots whose data is older than the refresh interval allows (all of them
    /// the first time), the shown spot first.
    func refreshOnOpen() async {
        let now = clock()
        let after = RefreshPolicy.dueAfter(minutes: refreshMinutes, lowPower: lowPower())
        let due = RefreshPolicy.due(
            spots.map { ($0.id, cache($0.id).lastSuccess) }, now: now, after: after)
        guard !due.isEmpty else {
            await rebuildPage()
            return
        }
        await refresh(only: due)
    }

    /// Fetches `ids` (every spot when `nil`), one at a time: each fetch loads the call ledger
    /// and saves its own copy, so side-by-side fetches would under-count the metered calls.
    /// A refresh asked for while one runs joins it and returns when it ends. Cancelling the
    /// task drops the spots still waiting.
    func refresh(only ids: Set<String>? = nil) async {
        let wanted = arranged.map(\.id).filter { ids?.contains($0) ?? true }
        let first = wanted.filter { $0 == activeID } + wanted.filter { $0 != activeID }
        queue.add(first)
        if running {
            await withCheckedContinuation { waiters.append($0) }
            return
        }
        running = true
        repeat {
            while let id = queue.next() {
                if Task.isCancelled {
                    queue.clear()
                    break
                }
                await fetchOne(id)
            }
            await rebuildPage()
            // A refresh that joined during the build has its spots waiting.
        } while !queue.isEmpty
        running = false
        let done = waiters
        waiters = []
        for waiter in done { waiter.resume() }
    }

    private func fetchOne(_ id: String) async {
        guard let spot = spot(id) else { return }
        var held = cache(id)
        refreshing.insert(id)
        defer { refreshing.remove(id) }
        await rebuildPage()
        let now = clock()
        let options = FetchOptions(
            allowMetered: true, inferWaterType: !spot.waterTypeChosen && held.suggestion == nil)
        do {
            let outcome = try await fetcher(
                spot.info, settingsJSON, keys(), options, now, host)
            if let hint = outcome.suggestion {
                held.suggestion = WaterTypeHint(waterType: hint.waterType, reasons: hint.reasons)
            }
            // Every source unreachable: the phone is offline; the older snapshot stays.
            if !outcome.errors.isEmpty, outcome.errors.allSatisfy({ $0.kind == "transport" }) {
                held.error = outcome.errors[0].message
            } else {
                held.snapshotJSON = outcome.snapshotJson
                held.lastSuccess = now
                held.error = nil
            }
        } catch {
            held.error = String(describing: error)
        }
        caches[id] = held
        files.write(held, files.cacheName(id))
    }

    /// The stations around the spot, ranked per measurement, kept with its cache.
    func discoverStations(for id: String) async {
        guard let spot = spot(id) else { return }
        var held = cache(id)
        let found = try? await UnderbiteKit.discoverStations(
            spot: spot.info, settingsJson: settingsJSON, keys: keys(), allowMetered: true,
            now: clock(), host: host)
        guard let found else { return }
        held.stationsJSON = found.stationsJson
        caches[id] = held
        files.write(held, files.cacheName(id))
    }

    // MARK: - The page

    /// Builds the list rows and the active spot's page with `view_model`, off the main thread
    /// and from the cache, and publishes them unless a newer build has started.
    func rebuildPage() async {
        generation += 1
        let mine = generation
        let order = arranged
        let active = order.firstIndex { $0.id == activeID } ?? 0
        let activeKey = activeID
        let activeCache = activeKey.map { cache($0) }
        let others = Dictionary(
            uniqueKeysWithValues: order.map { ($0.id, SpotCache.statusOnly(cache($0.id))) })
        let busy = refreshing
        let settings = settingsJSON
        let now = clock()
        let built = await Task.detached(priority: .userInitiated) {
            () -> (String, [SpotListing]) in
            let entries = order.map { spot -> SpotEntry in
                let held = spot.id == activeKey ? activeCache : nil
                let status = others[spot.id]
                return SpotEntry(
                    spot: spot.info, snapshotJson: held?.snapshotJSON,
                    lastSuccess: status?.lastSuccess, error: status?.error,
                    refreshing: busy.contains(spot.id))
            }
            guard
                let json = try? viewModel(
                    spots: entries, active: UInt32(active), settingsJson: settings, now: now),
                let head = try? Self.decoder.decode(PageHead.self, from: Data(json.utf8))
            else { return ("", []) }
            return (json, head.spots)
        }.value
        guard mine == generation else { return }
        pageJSON = built.0
        listing = built.1
    }

    private nonisolated static let decoder: JSONDecoder = {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return decoder
    }()

    // MARK: - Saving

    private func saveState() {
        files.write(
            StoredState(
                spots: spots, areas: areas, activeID: activeID, refreshMinutes: refreshMinutes),
            "state.json")
    }
}

extension SpotCache {
    /// The cache without its snapshot: what the list needs of every spot.
    fileprivate nonisolated static func statusOnly(_ cache: SpotCache) -> SpotCache {
        var copy = cache
        copy.snapshotJSON = nil
        return copy
    }
}
