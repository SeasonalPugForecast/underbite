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
    /// Sun mode; absent from files written before it existed.
    var sunMode: Bool?
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
    /// Looks up the stations around one spot (the core's `discover_stations`; tests may stand
    /// another in).
    typealias Discoverer =
        @Sendable (SpotInfo, String, Keys, Int64, FetchHost) async throws -> DiscoveryOutcome
    /// Builds the page JSON from the spots, the active index, the settings and now (the core's
    /// `view_model`; tests and the sample mode may stand another in).
    typealias PageBuilder =
        @Sendable ([SpotEntry], UInt32, String, Int64) throws -> String

    private(set) var spots: [Spot] = []
    private(set) var areas: [Area] = []
    private(set) var activeID: String?
    /// How often data refreshes, in minutes (one of `RefreshPolicy.choices`).
    private(set) var refreshMinutes = RefreshPolicy.defaultMinutes
    /// Sun mode: high contrast and heavier strokes for reading in bright light. A display
    /// preference of this device, like the refresh interval; it changes no value or word.
    private(set) var sunMode = false
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
    /// The ids whose stations are being looked up now.
    private(set) var discovering: Set<String> = []
    /// Per spot, what the last station lookup could not read, or why it failed.
    private(set) var stationProblems: [String: String] = [:]
    /// Raised whenever a spot's stations change, so the views that list them follow.
    private(set) var stationsRevision = 0

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
    @ObservationIgnored private let discoverer: Discoverer
    @ObservationIgnored private let pageBuilder: PageBuilder
    @ObservationIgnored private var caches: [String: SpotCache] = [:]
    @ObservationIgnored private var queue = RefreshQueue()
    /// The spots whose stations wait to be looked up, before any fetch still waiting.
    @ObservationIgnored private var lookups = RefreshQueue()
    @ObservationIgnored private var running = false
    /// A refresh asked for the page to be built once the calls are done.
    @ObservationIgnored private var pageDue = false
    @ObservationIgnored private var waiters: [CheckedContinuation<Void, Never>] = []
    @ObservationIgnored private var lookupWaiters: [String: [CheckedContinuation<Void, Never>]] =
        [:]
    /// The station rows last built per spot, with the stations revision and settings they are of.
    @ObservationIgnored private var rowsBuilt:
        [String: (revision: Int, settings: String, rows: [StationGroup])] = [:]
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
        },
        discoverer: @escaping Discoverer = { spot, settings, keys, now, host in
            try await UnderbiteKit.discoverStations(
                spot: spot, settingsJson: settings, keys: keys, allowMetered: true, now: now,
                host: host)
        },
        pageBuilder: @escaping PageBuilder = { spots, active, settings, now in
            try viewModel(spots: spots, active: active, settingsJson: settings, now: now)
        }
    ) {
        self.files = files
        self.host = host ?? URLSessionHost(storeURL: files.hostDirectory)
        self.keys = keys
        self.clock = clock
        self.lowPower = lowPower
        self.fetcher = fetcher
        self.discoverer = discoverer
        self.pageBuilder = pageBuilder
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
            sunMode = state.sunMode ?? false
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
        stationProblems[id] = nil
        rowsBuilt[id] = nil
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

    /// Renames an area; a blank name leaves it as it was.
    func renameArea(_ id: String, to name: String) {
        let name = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !name.isEmpty, let index = areas.firstIndex(where: { $0.id == id }) else { return }
        areas[index].name = name
        saveState()
    }

    /// The areas in the user's order.
    var arrangedAreas: [Area] { SpotGrouping.arrangedAreas(areas) }

    /// Puts the areas in the user's order: `ids` first, in that order, the others after them.
    func reorderAreas(_ ids: [String]) {
        let rest = arrangedAreas.map(\.id).filter { !ids.contains($0) }
        for (position, id) in (ids + rest).enumerated() {
            if let index = areas.firstIndex(where: { $0.id == id }) { areas[index].order = position }
        }
        saveState()
    }

    func setRefreshMinutes(_ minutes: Int) {
        guard RefreshPolicy.choices.contains(minutes) else { return }
        refreshMinutes = minutes
        saveState()
    }

    func setSunMode(_ on: Bool) {
        guard on != sunMode else { return }
        sunMode = on
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

    /// Every kind of water with the core's name for it, in the order a picker lists them.
    var waterTypeOptions: [WaterTypeOption] {
        (try? UnderbiteKit.waterTypeOptions(settingsJson: settingsJSON)) ?? []
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
        pageDue = true
        if running {
            await withCheckedContinuation { waiters.append($0) }
            return
        }
        running = true
        await drain()
    }

    /// Runs the waiting station lookups and fetches one at a time, lookups first (someone is
    /// looking at the list), until none is left. Every call loads the call ledger and saves its
    /// own copy, so one at a time is what keeps every metered call counted. Cancelling the task
    /// drops the fetches still waiting. Called with `running` set.
    private func drain() async {
        repeat {
            while true {
                if let id = lookups.next() {
                    await discoverOne(id)
                    continue
                }
                if Task.isCancelled {
                    queue.clear()
                    break
                }
                guard let id = queue.next() else { break }
                await fetchOne(id)
            }
            if pageDue {
                pageDue = false
                await rebuildPage()
            }
            // A refresh or a lookup that joined during the build has its spots waiting.
        } while !queue.isEmpty || !lookups.isEmpty
        running = false
        let done = waiters + lookupWaiters.values.flatMap { $0 }
        waiters = []
        lookupWaiters = [:]
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
        var ranked: String?
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
                // The stations the fetch ranked, so the list shows them before any lookup.
                ranked = storedStations(spotId: id, host: host)
            }
        } catch {
            held.error = String(describing: error)
        }
        // Else the stations a list kept while the fetch ran stay.
        if let ranked {
            held.stationsJSON = ranked
            stationsRevision += 1
        } else {
            held.stationsJSON = cache(id).stationsJSON
        }
        caches[id] = held
        files.write(held, files.cacheName(id))
    }

    // MARK: - Stations

    /// Looks up the stations around spot `id` again, through the store's own host and in turn
    /// with the fetches (never beside one, so the call ledger counts both), and keeps what it
    /// found with the spot's cache. Returns once that lookup is done; the list shows the kept
    /// one meanwhile. Cancelling the caller does not stop it.
    func discoverStations(for id: String) async {
        guard spot(id) != nil else { return }
        keepStoredStations(id)
        lookups.add([id])
        if !running {
            running = true
            // The calls run on their own: a closed list cancels its task, which must not drop
            // the fetches a refresh adds meanwhile.
            Task { await self.drain() }
        }
        await withCheckedContinuation { lookupWaiters[id, default: []].append($0) }
    }

    /// Keeps the stations a fetch or lookup of `id` last stored on the host, read without a
    /// request: a fetch ranks them first and stores them long before it ends, so the list need
    /// not wait for it.
    private func keepStoredStations(_ id: String) {
        var held = cache(id)
        guard let stored = storedStations(spotId: id, host: host), stored != held.stationsJSON
        else { return }
        held.stationsJSON = stored
        caches[id] = held
        files.write(held, files.cacheName(id))
        stationsRevision += 1
    }

    private func discoverOne(_ id: String) async {
        // Those asking now are answered by this lookup; one asking while it runs waits for the
        // next.
        let asking = lookupWaiters.removeValue(forKey: id) ?? []
        defer { for waiter in asking { waiter.resume() } }
        guard let spot = spot(id) else { return }
        discovering.insert(id)
        defer { discovering.remove(id) }
        let settings = settingsJSON
        do {
            let found = try await discoverer(spot.info, settings, keys(), clock(), host)
            // A spot deleted meanwhile keeps no stations.
            guard self.spot(id) != nil else { return }
            var held = cache(id)
            // Every list unreachable: the phone is offline; the older stations stay.
            let offline =
                !found.errors.isEmpty && found.errors.allSatisfy { $0.kind == "transport" }
            if !offline || held.stationsJSON == nil {
                held.stationsJSON = found.stationsJson
                caches[id] = held
                files.write(held, files.cacheName(id))
                stationsRevision += 1
            }
            stationProblems[id] = found.errors.first?.message
        } catch {
            stationProblems[id] = String(describing: error)
        }
    }

    /// The stations around spot `id` as the core words them, from the last lookup kept with
    /// the spot, so they show at once after a launch.
    func stationRows(for id: String) -> [StationGroup] {
        let revision = stationsRevision
        if let held = rowsBuilt[id], held.revision == revision, held.settings == settingsJSON {
            return held.rows
        }
        guard let json = cache(id).stationsJSON else { return [] }
        let rows =
            (try? UnderbiteKit.stationRows(
                stationsJson: json, settingsJson: settingsJSON, now: clock())) ?? []
        rowsBuilt[id] = (revision, settingsJSON, rows)
        return rows
    }

    /// Remembers `choice` for the station `key` (`coops:8518750`) of spot `id` in the store's
    /// own station store, which the next fetch reads, then looks the stations up again so the
    /// list shows it. The page follows with the spot's next fetch.
    func chooseStation(_ choice: StationChoice, key: String, for id: String) async throws {
        let now = clock()
        switch choice {
        case .pin: try pinStation(spotId: id, key: key, now: now, host: host)
        case .exclude: try excludeStation(spotId: id, key: key, now: now, host: host)
        case .clear: try clearStation(spotId: id, key: key, now: now, host: host)
        }
        await discoverStations(for: id)
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
        let build = pageBuilder
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
                let json = try? build(entries, UInt32(active), settings, now),
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
                spots: spots, areas: areas, activeID: activeID, refreshMinutes: refreshMinutes,
                sunMode: sunMode),
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
