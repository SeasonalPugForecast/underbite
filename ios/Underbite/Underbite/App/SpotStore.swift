import Foundation
import Observation
@preconcurrency import UnderbiteKit

/// What the store keeps in `state.json`. A key missing from the file takes its default; a file
/// of a later version is not read.
nonisolated private struct StoredState: Codable {
    /// The version this app writes, and the latest it reads.
    static let currentVersion = 1

    var version = StoredState.currentVersion
    var spots: [Spot] = []
    var areas: [Area] = []
    var activeID: String?
    var refreshMinutes = RefreshPolicy.defaultMinutes
    /// Sun mode; absent from files written before it existed.
    var sunMode = false

    private enum CodingKeys: String, CodingKey {
        case version, spots, areas, activeID, refreshMinutes, sunMode
    }

    init(spots: [Spot], areas: [Area], activeID: String?, refreshMinutes: Int, sunMode: Bool) {
        self.spots = spots
        self.areas = areas
        self.activeID = activeID
        self.refreshMinutes = refreshMinutes
        self.sunMode = sunMode
    }

    init(from decoder: any Decoder) throws {
        let file = try decoder.container(keyedBy: CodingKeys.self)
        version = try file.decodeIfPresent(Int.self, forKey: .version) ?? 1
        guard version <= Self.currentVersion else {
            throw DecodingError.dataCorruptedError(
                forKey: .version, in: file,
                debugDescription: "version \(version) is later than \(Self.currentVersion)")
        }
        spots = try file.decodeIfPresent([Spot].self, forKey: .spots) ?? []
        areas = try file.decodeIfPresent([Area].self, forKey: .areas) ?? []
        activeID = try file.decodeIfPresent(String.self, forKey: .activeID)
        refreshMinutes =
            try file.decodeIfPresent(Int.self, forKey: .refreshMinutes)
            ?? RefreshPolicy.defaultMinutes
        sunMode = try file.decodeIfPresent(Bool.self, forKey: .sunMode) ?? false
    }
}

/// A file of the app that could not be read when the store loaded. The store started without
/// it (no spots, or the default settings) and never writes over it.
nonisolated struct FileProblem: Equatable, Sendable {
    enum File: String, Sendable {
        /// The spots and areas, `state.json`.
        case spots = "state.json"
        /// The settings, `settings.json`.
        case settings = "settings.json"
    }

    var file: File
    /// The name the unreadable file was moved to in the app's directory (`state.json.bad`), or
    /// `nil` when it could not be moved: then it stays where it was and is not saved over.
    var keptAs: String?

    /// What the app says of it, in the core's words.
    var text: String {
        fileProblemText(file: file == .spots ? .spots : .settings, keptAs: keptAs)
    }
}

/// What the list needs of a spot's cache: when its last fetch succeeded and why one failed.
nonisolated struct SpotStatus: Equatable, Sendable {
    var lastSuccess: Int64?
    var error: String?
    /// When a fetch last found no signal, until one gets an answer.
    var lastAttempt: Int64?
    /// The core's offline note of the kept snapshot, while the last fetch found no signal: the
    /// merged snapshot lists it among its errors, so it survives a relaunch.
    var offlineNote: String?

    /// The status of `cache`, from what it carries: no snapshot is read.
    init(_ cache: SpotCache) {
        lastSuccess = cache.lastSuccess
        error = cache.error
        lastAttempt = cache.lastAttempt
        offlineNote = cache.lastAttempt == nil ? nil : cache.offlineNote
    }

    /// The status of `cache` as read from its file: one written before the cache carried the
    /// note has it read from the snapshot's errors (decoding the snapshot, so off the main
    /// actor only).
    static func decoding(_ cache: SpotCache) -> SpotStatus {
        var status = SpotStatus(cache)
        if status.lastAttempt != nil, cache.offlineNote == nil {
            status.offlineNote = cache.snapshotJSON.flatMap(Self.offlineNote(in:))
        }
        return status
    }

    /// The snapshot's offline note, if the core merged one in.
    private static func offlineNote(in snapshotJSON: String) -> String? {
        struct Errors: Decodable {
            struct Issue: Decodable {
                var source: String
                var message: String
            }
            var errors: [Issue]?
        }
        let decoded = try? JSONDecoder().decode(Errors.self, from: Data(snapshotJSON.utf8))
        return decoded?.errors?.first { $0.source == FetchStanding.offlineSource }?.message
    }

    /// The time the refresh interval counts from: the last try without signal while there was
    /// none, else the last success.
    var refreshedAt: Int64? { lastAttempt ?? lastSuccess }
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
    /// It keeps the spots' station metadata in the app's one offline store.
    static let shared = SpotStore(
        metadata: SpotMetadata(store: MapOffline.store), simulatesOffline: NetworkRoute.launchOffline)

    /// Runs one spot's fetch (the core's `fetch_spot`; tests may stand another in).
    typealias Fetcher =
        @Sendable (SpotInfo, String, Keys, FetchOptions, Int64, FetchHost) async throws
        -> SpotFetchOutcome
    /// Runs one spot's fetch with the station metadata kept for it (the core's `fetch_spot`).
    typealias MetadataFetcher =
        @Sendable (SpotInfo, String, Keys, FetchOptions, Int64, [CachedResponse], FetchHost)
        async throws -> SpotFetchOutcome
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
    /// Per spot shown, its page JSON (`view_model` with it the active spot), for the views to
    /// decode: each window of an iPad may show a spot of its own.
    private(set) var pages: [String: String] = [:]
    /// Where the phone is, when it was asked; never saved unless the user saves it as a spot.
    private(set) var currentPlace: CurrentPlace?
    /// The ids whose stations are being looked up now.
    private(set) var discovering: Set<String> = []
    /// Per spot, what the last station lookup could not read, or why it failed.
    private(set) var stationProblems: [String: String] = [:]
    /// Raised whenever a spot's stations change, so the views that list them follow.
    private(set) var stationsRevision = 0
    /// The files that could not be read at launch, for the views to say so.
    private(set) var fileProblems: [FileProblem] = []
    /// Per spot, the core's offline note of its last fetch when that found no signal ("offline:
    /// computed from harmonic constants, other values from ..."), until one of its fetches gets
    /// an answer. It follows fetch outcomes only, so a brief loss of the route between fetches
    /// shows nothing.
    private(set) var offlineNotes: [String: String] = [:]

    /// The note the banner shows over the conditions of the shown spot: that spot's own, never
    /// another's.
    var offlineNote: String? {
        activeID.flatMap { offlineNotes[$0] }
    }

    /// The note the banner shows over the sections that are not one spot's (map, trips, log,
    /// settings): none unless every saved spot's last fetch found no signal, which says the
    /// phone has none; then one note, the shown spot's else the first in the list's order. One
    /// spot offline among others that answered says only something about that spot.
    ///
    /// A spot whose last fetch found nothing to merge (no snapshot yet) has no note but is as
    /// offline as the rest, so it counts. The words are the core's: with no note among the
    /// spots, no banner shows (the core has no plain word for it, and none is made up here).
    var generalOfflineNote: String? {
        guard !spots.isEmpty,
            spots.allSatisfy({ offlineNotes[$0.id] != nil || unreachable.contains($0.id) })
        else { return nil }
        return offlineNote ?? arranged.lazy.compactMap { self.offlineNotes[$0.id] }.first
    }

    /// The ids whose last fetch found no signal and had nothing to merge: no note, but not
    /// online either, until one of their fetches gets an answer.
    private(set) var unreachable: Set<String> = []

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
    @ObservationIgnored private let fetcher: MetadataFetcher
    /// Where the spots' station metadata is kept; none keeps nothing (tests, the sample).
    @ObservationIgnored private let metadata: SpotMetadata?
    /// Debug `-offline`: a spot fetched once online (now or before) is then fetched through a
    /// host without signal.
    @ObservationIgnored private let simulatesOffline: Bool
    @ObservationIgnored private var primed: Set<String> = []
    /// The spots waiting for a fetch that someone asked for: their metadata is asked for again.
    @ObservationIgnored private var asked: Set<String> = []
    /// The spots whose position or station choices changed since their metadata was last asked
    /// for: the next fetch asks again, and the spot's pins follow what it needs now.
    @ObservationIgnored private var changed: Set<String> = []
    @ObservationIgnored private let discoverer: Discoverer
    @ObservationIgnored private let pageBuilder: PageBuilder
    /// The whole caches read so far: the shown spots' and those a view asked for. The others
    /// stay on disk.
    @ObservationIgnored private var caches: [String: SpotCache] = [:]
    /// Which spot each page on screen shows, by the page's own token.
    @ObservationIgnored private var viewers: [UUID: String] = [:]
    /// Every spot's status, read from its cache off the main actor.
    @ObservationIgnored private var statuses: [String: SpotStatus] = [:]
    /// Files left in place after they could not be read: never written over.
    @ObservationIgnored private var unwritable: Set<String> = []
    /// The refresh while the app is open.
    @ObservationIgnored private var openLoop: Task<Void, Never>?
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
        fetcher: Fetcher? = nil,
        metadataFetcher: MetadataFetcher? = nil,
        metadata: SpotMetadata? = nil,
        simulatesOffline: Bool = false,
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
        if let metadataFetcher {
            self.fetcher = metadataFetcher
        } else if let fetcher {
            self.fetcher = { @Sendable spot, settings, keys, options, now, _, host in
                try await fetcher(spot, settings, keys, options, now, host)
            }
        } else {
            self.fetcher = { @Sendable spot, settings, keys, options, now, cached, host in
                try await fetchSpot(
                    spot: spot, settingsJson: settings, keys: keys, options: options, now: now,
                    cachedMetadata: cached, host: host)
            }
        }
        self.metadata = metadata
        self.simulatesOffline = simulatesOffline
        self.discoverer = discoverer
        self.pageBuilder = pageBuilder
        settingsJSON = defaultSettingsJson()
        load()
    }

    // MARK: - Loading

    /// Before the first frame: the state, the settings and the cache of the shown spot only.
    private func load() {
        loadState()
        loadSettings()
        if activeID == nil || !spots.contains(where: { $0.id == activeID }) {
            activeID = arranged.first?.id
        }
        if let activeID { _ = cache(activeID) }
        adoptOfflineNotes()
    }

    /// The saved spots and areas; a file that cannot be read is set aside, never overwritten.
    private func loadState() {
        let read: StoredState?
        do {
            read = try files.decode(StoredState.self, FileProblem.File.spots.rawValue)
        } catch {
            setAside(.spots)
            return
        }
        guard let state = read else { return }
        spots = state.spots
        areas = state.areas
        activeID = state.activeID
        refreshMinutes =
            RefreshPolicy.choices.contains(state.refreshMinutes)
            ? state.refreshMinutes : RefreshPolicy.defaultMinutes
        sunMode = state.sunMode
    }

    /// The saved settings, migrated on every load; settings that cannot be read leave the
    /// defaults, and their file is set aside, never overwritten.
    private func loadSettings() {
        let saved: String
        do {
            guard let text = try files.utf8(FileProblem.File.settings.rawValue) else { return }
            saved = text
        } catch {
            setAside(.settings)
            return
        }
        do {
            let migrated = try migrateSettings(json: saved)
            settingsJSON = migrated.json
            settingsIssues = migrated.issues
            if migrated.json != saved { saveSettingsNow() }
        } catch {
            settingsIssues = []
            setAside(.settings)
        }
    }

    /// Moves an unreadable `file` aside and records it; when it cannot be moved, it is left
    /// in place and never saved over.
    private func setAside(_ file: FileProblem.File) {
        let kept = files.setAside(file.rawValue)
        if kept == nil { unwritable.insert(file.rawValue) }
        fileProblems.append(FileProblem(file: file, keptAs: kept))
    }

    /// The cache of `id`, read from its file the first time and held from then on.
    func cache(_ id: String) -> SpotCache {
        if let held = caches[id] { return held }
        let read = files.read(SpotCache.self, files.cacheName(id)) ?? SpotCache()
        caches[id] = read
        statuses[id] = SpotStatus(read)
        return read
    }

    /// The ids whose whole cache is held in memory.
    var heldCaches: Set<String> { Set(caches.keys) }

    /// The cache of `id`: the one held, else its file read off the main actor, and not held.
    private func storedCache(_ id: String) async -> SpotCache {
        if let held = caches[id] { return held }
        let files = self.files
        return await Task.detached(priority: .userInitiated) {
            files.read(SpotCache.self, files.cacheName(id)) ?? SpotCache()
        }.value
    }

    /// Reads the status of every spot not known yet from its cache file, off the main actor;
    /// the snapshots are dropped, not held.
    private func loadStatuses() async {
        adoptOfflineNotes()
        let missing = spots.map(\.id).filter { statuses[$0] == nil }
        guard !missing.isEmpty else { return }
        let files = self.files
        let read = await Task.detached(priority: .userInitiated) {
            missing.map { id in
                (id, SpotStatus.decoding(files.read(SpotCache.self, files.cacheName(id)) ?? SpotCache()))
            }
        }.value
        // A fetch that ended meanwhile wrote a newer one.
        for (id, status) in read where statuses[id] == nil && spot(id) != nil {
            statuses[id] = status
            // A relaunch without signal: the note of the last fetch shows again.
            if let note = status.offlineNote {
                offlineNotes[id] = note
            } else if status.lastAttempt != nil {
                unreachable.insert(id)
            }
        }
    }

    /// A relaunch without signal: the note of a spot's last fetch, read from the status of a
    /// cache read before (the shown spot's), shows again. Never done while a view reads a cache,
    /// which must not change what the views observe.
    private func adoptOfflineNotes() {
        for spot in spots where offlineNotes[spot.id] == nil {
            if let note = statuses[spot.id]?.offlineNote {
                offlineNotes[spot.id] = note
            } else if statuses[spot.id]?.lastAttempt != nil {
                unreachable.insert(spot.id)
            }
        }
    }

    /// Saves `held` as the cache of `id`: its status, its file, and in memory when it is held
    /// there already or `id` is a shown spot.
    private func keep(_ held: SpotCache, for id: String) {
        if caches[id] != nil || shownIDs.contains(id) { caches[id] = held }
        statuses[id] = SpotStatus(held)
        files.write(held, files.cacheName(id))
    }

    // MARK: - Spots

    /// The spots in list order.
    var arranged: [Spot] { Spot.arranged(spots) }

    var activeSpot: Spot? { spots.first { $0.id == activeID } }

    func spot(_ id: String) -> Spot? { spots.first { $0.id == id } }

    /// The page JSON of spot `id`, once a page showing it has been built; empty before.
    func pageJSON(for id: String) -> String { pages[id] ?? "" }

    /// The page JSON of the active spot.
    var pageJSON: String { activeID.map(pageJSON(for:)) ?? "" }

    /// The spots whose pages are built: the active one first, then those shown on screen.
    var shownIDs: [String] {
        var ids = activeID.map { [$0] } ?? []
        for id in viewers.values.sorted() where !ids.contains(id) { ids.append(id) }
        return ids.filter { spot($0) != nil }
    }

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
        let old = spots[index]
        if old.latitude != spot.latitude || old.longitude != spot.longitude {
            changed.insert(spot.id)
        }
        spots[index] = spot
        saveState()
    }

    /// Deletes the spot `id` with everything kept of it: its cache, and on the host its last
    /// snapshot (which holds its exact position), its rankings and its station choices. A fetch
    /// or lookup of it under way forgets them again when it ends.
    func remove(_ id: String) {
        spots.removeAll { $0.id == id }
        caches[id] = nil
        statuses[id] = nil
        pages[id] = nil
        files.remove(files.cacheName(id))
        forgetSpot(spotId: id, host: host)
        if let metadata { Task { await metadata.forget(spot: id) } }
        stationProblems[id] = nil
        offlineNotes[id] = nil
        unreachable.remove(id)
        asked.remove(id)
        changed.remove(id)
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

    /// A page on screen (`viewer`, its own token) shows spot `id`: its page is built, and kept
    /// with every rebuild until the page goes (`hide`). It becomes the active spot, the one a
    /// launch opens. Several pages may show different spots at once, one per iPad window.
    func show(_ id: String, viewer: UUID) async {
        guard spot(id) != nil else { return }
        let previous = viewers.updateValue(id, forKey: viewer)
        _ = cache(id)
        // The spot this page showed before, unless another page shows it, is let go.
        if let previous, previous != id, !viewers.values.contains(previous) {
            caches[previous] = nil
        }
        await show(id)
    }

    /// The page `viewer` has gone: its spot's page and cache are let go unless another page
    /// shows it or it is the active spot.
    func hide(viewer: UUID) {
        guard let id = viewers.removeValue(forKey: viewer) else { return }
        guard !shownIDs.contains(id) else { return }
        pages[id] = nil
        caches[id] = nil
    }

    /// Makes `id` the shown spot; its cache is read now, the others stay on disk.
    func setActive(_ id: String) {
        guard spot(id) != nil else { return }
        activeID = id
        _ = cache(id)
        saveState()
    }

    // MARK: - Preview

    /// A preview of the conditions at a place that is not a saved spot. It shares only what the
    /// store fetches with (the host, the keys, the clock, the settings read now); it adds no
    /// spot and keeps no cache, status, page or station metadata here, so nothing of it reaches
    /// the spot list, the widgets, the alerts, the refresh, the offline packs or iCloud.
    func makePreview(_ prefill: SpotPrefill, place: String) -> ConditionsPreview {
        ConditionsPreview(
            prefill: prefill, place: place, settingsJSON: settingsJSON, host: host, keys: keys,
            clock: clock, fetcher: fetcher, pageBuilder: pageBuilder)
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
        let name = FileProblem.File.settings.rawValue
        guard !unwritable.contains(name) else { return }
        files.write(Data(settingsJSON.utf8), name)
    }

    /// Every kind of water with the core's name for it, in the order a picker lists them.
    var waterTypeOptions: [WaterTypeOption] {
        (try? UnderbiteKit.waterTypeOptions(settingsJson: settingsJSON)) ?? []
    }

    // MARK: - Refresh

    /// Starts the refresh while the app is open (`foreground`), or stops it: in the foreground
    /// the due spots are fetched, and the page built again, every `RefreshPolicy.whileOpen`;
    /// in the background nothing is started (a fetch under way ends first).
    func setForeground(_ foreground: Bool) {
        if foreground {
            guard openLoop == nil else { return }
            openLoop = Task { await self.whileOpen() }
        } else {
            openLoop?.cancel()
            openLoop = nil
        }
    }

    /// Until cancelled, every `tick`: fetches the spots that are due, or builds the page again
    /// when none is, so its ages, the values of now and what has gone stale follow the clock.
    /// Cancelling it cancels no fetch: the refresh runs in a task of its own.
    func whileOpen(every tick: Duration = RefreshPolicy.whileOpen) async {
        while !Task.isCancelled {
            await Task { await self.refreshOnOpen() }.value
            try? await Task.sleep(for: tick)
        }
    }

    /// Fetches the spots whose data is older than the refresh interval allows (all of them
    /// the first time), the shown spot first.
    func refreshOnOpen() async {
        await loadStatuses()
        let now = clock()
        let after = RefreshPolicy.dueAfter(minutes: refreshMinutes, lowPower: lowPower())
        let due = RefreshPolicy.due(
            spots.map { ($0.id, statuses[$0.id]?.refreshedAt) }, now: now, after: after)
        guard !due.isEmpty else {
            await rebuildPage()
            return
        }
        await refresh(only: due, asked: false)
    }

    /// The route came back and held: fetches the spots whose last fetch found no signal.
    func refreshWaiting() async {
        await loadStatuses()
        let waiting = Set(spots.map(\.id).filter { statuses[$0]?.lastAttempt != nil })
        guard !waiting.isEmpty else { return }
        await refresh(only: waiting, asked: false)
    }

    /// Fetches `ids` (every spot when `nil`), one at a time: each fetch loads the call ledger
    /// and saves its own copy, so side-by-side fetches would under-count the metered calls.
    /// A refresh asked for while one runs joins it and returns when it ends; a spot being
    /// fetched then is not fetched again. Cancelling the task drops the spots still waiting.
    /// Someone asked for it (a pull to refresh, a new spot, a pack's download): the station
    /// metadata is asked for again too.
    func refresh(only ids: Set<String>? = nil) async {
        await refresh(only: ids, asked: true)
    }

    private func refresh(only ids: Set<String>?, asked: Bool) async {
        let wanted = arranged.map(\.id).filter { ids?.contains($0) ?? true }
        let first = wanted.filter { $0 == activeID } + wanted.filter { $0 != activeID }
        queue.add(first, except: refreshing)
        // Only the spots that wait for a fetch: one fetching now has its own under way, and a
        // mark on it would outlive that fetch.
        if asked { self.asked.formUnion(wanted.filter { queue.waiting.contains($0) }) }
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
                    asked.subtract(queue.waiting)
                    queue.clear()
                    break
                }
                guard let id = queue.next() else { break }
                // Debug `-offline`: the first fetch went online; the next one, at once and
                // before the other spots, has no signal.
                if await fetchOne(id) { queue.addFirst(id) }
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

    /// Fetches spot `id` with the station metadata kept for it, and keeps what came back: a
    /// snapshot with an answer, the core's merged one without signal (its offline note then
    /// shows, and the time of the last success stays), or the older snapshot when nothing
    /// answered and there was nothing to merge. Returns whether a debug `-offline` launch
    /// should fetch it again, now without signal.
    @discardableResult
    private func fetchOne(_ id: String) async -> Bool {
        guard spot(id) != nil else { return false }
        refreshing.insert(id)
        defer { refreshing.remove(id) }
        let refreshMetadata = asked.remove(id) != nil || changed.contains(id)
        var held = await storedCache(id)
        await rebuildPage()
        guard let spot = spot(id) else { return false }
        let now = clock()
        let options = FetchOptions(
            allowMetered: true, inferWaterType: !spot.waterTypeChosen && held.suggestion == nil,
            refreshMetadata: refreshMetadata)
        let kept = held.metadataKeys ?? []
        let cached = await metadata?.cached(kept, now: now) ?? []
        let withoutSignal =
            simulatesOffline
            && (primed.contains(id) || (held.snapshotJSON != nil && !kept.isEmpty))
        let host: FetchHost = withoutSignal ? UnreachableHost(self.host) : self.host
        let result: Result<SpotFetchOutcome, any Error>
        do {
            result = .success(
                try await fetcher(spot.info, settingsJSON, keys(), options, now, cached, host))
        } catch {
            result = .failure(error)
        }
        // A spot deleted meanwhile keeps nothing: the fetch stored its snapshot on the host.
        guard let current = self.spot(id) else {
            forgetSpot(spotId: id, host: self.host)
            return false
        }
        var ranked: String?
        switch result {
        case .success(let outcome):
            if let hint = outcome.suggestion {
                held.suggestion = WaterTypeHint(waterType: hint.waterType, reasons: hint.reasons)
                // Until the user chooses one, the spot takes the type the core suggests, so
                // the page and the next fetches plan for it.
                if !current.waterTypeChosen, current.waterType != hint.waterType {
                    change(id) { $0.waterType = hint.waterType }
                }
            }
            switch FetchStanding(outcome) {
            case .online:
                held.snapshotJSON = outcome.snapshotJson
                held.lastSuccess = now
                held.lastAttempt = nil
                held.error = nil
                held.offlineNote = nil
                offlineNotes[id] = nil
                unreachable.remove(id)
                if refreshMetadata { changed.remove(id) }
                // The stations the fetch ranked, so the list shows them before any lookup.
                ranked = storedStations(spotId: id, host: self.host)
            case .offline(let note):
                // The core's merge: the last values with their own times, and the tide
                // computed from the kept constants. The last success stays the last online.
                held.snapshotJSON = outcome.snapshotJson
                held.lastAttempt = now
                held.error = nil
                held.offlineNote = note
                offlineNotes[id] = note
                unreachable.remove(id)
            case .unreachable(let message):
                // Nothing answered and nothing to merge: the older snapshot stays.
                held.lastAttempt = now
                held.error = message
                unreachable.insert(id)
            }
            // After an answer to a request for all the metadata, the core names the stations
            // the spot needs now, whether or not their requests got through: the pins of any
            // other station are released. Without that list nothing is.
            let needed =
                refreshMetadata && FetchStanding(outcome) == .online ? outcome.metadataStations : []
            if let metadata, !outcome.cacheable.isEmpty || !needed.isEmpty {
                held.metadataKeys = await metadata.keep(
                    outcome.cacheable, spot: id, kept: kept, now: now, needed: needed)
                // Deleted while the metadata was written: nothing of it stays.
                guard self.spot(id) != nil else {
                    await metadata.forget(spot: id)
                    return false
                }
            }
        case .failure(let error):
            held.error = Self.message(of: error)
        }
        // Else the stations a list kept while the fetch ran stay.
        if let ranked {
            held.stationsJSON = ranked
            stationsRevision += 1
        } else if let kept = caches[id] {
            held.stationsJSON = kept.stationsJSON
        }
        keep(held, for: id)
        guard simulatesOffline, !withoutSignal, case .success = result else { return false }
        return primed.insert(id).inserted
    }

    /// What an error of a fetch, lookup or station choice says: the core's own message for the
    /// FFI's errors.
    nonisolated static func message(of error: any Error) -> String {
        switch error {
        case FetchError.Settings(let message): message
        case FetchError.Invalid(_, let message): message
        default: error.localizedDescription
        }
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
        keep(held, for: id)
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
            // A spot deleted meanwhile keeps no stations, here or on the host.
            guard self.spot(id) != nil else {
                forgetSpot(spotId: id, host: host)
                return
            }
            var held = cache(id)
            // Nothing answered (the core says so, a captive portal's page included): the phone is
            // offline; the older stations stay.
            if !found.unanswered || held.stationsJSON == nil {
                held.stationsJSON = found.stationsJson
                keep(held, for: id)
                stationsRevision += 1
            }
            stationProblems[id] = found.errors.first?.message
        } catch {
            guard self.spot(id) != nil else { return }
            stationProblems[id] = Self.message(of: error)
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
        guard spot(id) != nil else { return }
        let now = clock()
        switch choice {
        case .pin: try pinStation(spotId: id, key: key, now: now, host: host)
        case .exclude: try excludeStation(spotId: id, key: key, now: now, host: host)
        case .clear: try clearStation(spotId: id, key: key, now: now, host: host)
        }
        changed.insert(id)
        await discoverStations(for: id)
    }

    // MARK: - The page

    /// Builds the list rows and the page of every shown spot with `view_model`, off the main
    /// thread and from the cache, and publishes them unless a newer build has started. Each
    /// page holds one spot's view; a page that cannot be built leaves the others and the list.
    func rebuildPage() async {
        await loadStatuses()
        generation += 1
        let mine = generation
        let order = arranged
        let shown = shownIDs.compactMap { id in
            order.firstIndex { $0.id == id }.map { (id: id, index: $0, cache: cache(id)) }
        }
        let others = statuses
        let busy = refreshing
        let settings = settingsJSON
        let now = clock()
        let build = pageBuilder
        let built = await Task.detached(priority: .userInitiated) {
            () -> (pages: [String: String], listing: [SpotListing]?) in
            func entries(snapshot: (Spot) -> String?) -> [SpotEntry] {
                order.map { spot in
                    let status = others[spot.id]
                    return SpotEntry(
                        spot: spot.info, snapshotJson: snapshot(spot),
                        lastSuccess: status?.lastSuccess, error: status?.error,
                        refreshing: busy.contains(spot.id))
                }
            }
            func head(_ json: String) -> [SpotListing]? {
                (try? Self.decoder.decode(PageHead.self, from: Data(json.utf8)))?.spots
            }
            var pages: [String: String] = [:]
            var listing: [SpotListing]?
            for page in shown {
                let all = entries { $0.id == page.id ? page.cache.snapshotJSON : nil }
                guard let json = try? build(all, UInt32(page.index), settings, now) else {
                    continue
                }
                pages[page.id] = json
                if listing == nil { listing = head(json) }
            }
            // No page could be built (or none is shown): the list from the statuses alone.
            if listing == nil,
                let json = try? build(entries { _ in nil }, 0, settings, now)
            {
                listing = head(json)
            }
            return (pages, listing)
        }.value
        guard mine == generation else { return }
        pages = built.pages
        listing = built.listing ?? []
    }

    private nonisolated static let decoder: JSONDecoder = {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return decoder
    }()

    // MARK: - Saving

    private func saveState() {
        guard !unwritable.contains(FileProblem.File.spots.rawValue) else { return }
        files.write(
            StoredState(
                spots: spots, areas: areas, activeID: activeID, refreshMinutes: refreshMinutes,
                sunMode: sunMode),
            FileProblem.File.spots.rawValue)
    }
}
