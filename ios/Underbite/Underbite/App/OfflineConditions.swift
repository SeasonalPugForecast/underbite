import Foundation
import Network
@preconcurrency import UnderbiteKit

/// How a spot's fetch went, read from its outcome: the core decides what counts as having no
/// signal and words it; the app only reads its note.
nonisolated enum FetchStanding: Equatable, Sendable {
    /// Something answered: the snapshot is new.
    case online
    /// Nothing answered, and the core merged the last snapshot (and any tide it computed from
    /// kept constants) with its note: "offline: computed from harmonic constants, other values
    /// from Nov 18, 2:40 PM".
    case offline(note: String)
    /// Nothing answered and there was nothing to fall back on: the source's own message.
    case unreachable(message: String)

    /// The source the core files its offline note under.
    static let offlineSource = "offline"

    init(_ errors: [SourceIssue]) {
        if let note = errors.first(where: { $0.source == Self.offlineSource }) {
            self = .offline(note: note.message)
        } else if let first = errors.first, errors.allSatisfy({ $0.kind == "transport" }) {
            self = .unreachable(message: first.message)
        } else {
            self = .online
        }
    }
}

/// The station metadata (harmonic constants, datums, offsets, station lists) each spot's fetches
/// keep in the app's offline store, so a fetch without signal still computes the tide. Rows are
/// blobs under the core's cache keys. A station's own files (a few kilobytes each) are pinned to
/// the spot (``pin(spot:)``): neither the store's bound nor clearing the cache drops them, and
/// deleting the spot lets them go unless another spot or a pack holds them. The station lists
/// (megabytes; the rankings kept with the spot name its stations already) stay in the cache.
nonisolated struct SpotMetadata: Sendable {
    let store: OfflineStore

    /// How long a kept copy counts as fresh. The core asks again for constants a week old, and
    /// the fetch passes back every copy whatever its age (the core judges how old is too old).
    static let lifetime: UInt64 = 7 * 86_400

    /// The pin a spot's metadata is kept under: never a pack's id (those are UUIDs or `seed-`
    /// names), and the spot id is the app's own.
    static func pin(spot id: String) -> String { "spot/" + id }

    /// Whether `key` is one station's file (`.../stations/8518750/harcon.json`) rather than a
    /// list of stations (`.../stations.json`, NDBC's `activestations.xml`).
    static func isStationFile(_ key: String) -> Bool {
        key.split(separator: "?", maxSplits: 1).first?.contains("/stations/") ?? false
    }

    /// The copies kept under `keys`, at any age, as the core takes them back.
    func cached(_ keys: [String], now: Int64) async -> [CachedResponse] {
        var out: [CachedResponse] = []
        for key in keys {
            guard let record = try? await store.get(.blob(key), now: now, allowingStale: true)
            else { continue }
            out.append(CachedResponse(key: key, body: record.data, fetchedAt: record.fetchedAt))
        }
        return out
    }

    /// Keeps the responses a fetch of spot `id` offered, pinned to the spot, and returns the
    /// keys to pass back next time: those kept before and the new ones. The core offers only what
    /// it asked for again, so every offered copy replaces the one under its key.
    func keep(_ offered: [CachedResponse], spot id: String, kept: [String], now: Int64) async
        -> [String]
    {
        var keys = kept
        for entry in offered where !keys.contains(entry.key) { keys.append(entry.key) }
        guard !offered.isEmpty else { return keys }
        // Pinned before written, so the bound cannot drop a new row on the way in.
        let pinned = offered.map(\.key).filter(Self.isStationFile)
        try? await store.pin(Self.pin(spot: id), pinned.map { .blob($0) })
        let rows: [(key: OfflineKey, entry: OfflineEntry)] = offered.map { entry in
            (
                .blob(entry.key),
                OfflineEntry(
                    data: entry.body, contentType: nil, fetchedAt: entry.fetchedAt,
                    lifetime: Self.lifetime)
            )
        }
        _ = try? await store.put(rows, now: now)
        return keys
    }

    /// Lets go of what spot `id` kept; rows no other spot or pack pins are deleted.
    func forget(spot id: String) async {
        _ = try? await store.unpin(Self.pin(spot: id))
    }
}

/// A host that fails every request as a phone without signal does, and keeps values and the
/// call ledger through `base`. Debug builds launched with `-offline` fetch through it.
nonisolated final class UnreachableHost: FetchHost, @unchecked Sendable {
    private let base: FetchHost

    init(_ base: FetchHost) {
        self.base = base
    }

    func send(request: HostRequest) async throws -> HostResponse {
        throw HostError.Failed(message: URLError(.notConnectedToInternet).localizedDescription)
    }

    func load(namespace: String, key: String) -> String? {
        base.load(namespace: namespace, key: key)
    }

    func store(namespace: String, key: String, value: String) {
        base.store(namespace: namespace, key: key, value: value)
    }

    func saveUsage(ledgerJson: String) throws {
        try base.saveUsage(ledgerJson: ledgerJson)
    }
}

/// Calls back once the network route has come back and held for ``settle``: a brief loss and
/// return, or a route that flaps, calls nothing until it is steady. The banner itself only ever
/// follows fetch outcomes; this only asks for a fetch when there is reason to expect an answer.
nonisolated final class RouteReturn: @unchecked Sendable {
    /// How long the route must hold before it counts as back.
    static let settle: Duration = .seconds(4)

    private let lock = NSLock()
    private var lost = false
    private var pending: Task<Void, Never>?
    /// Raised with every wait started or cancelled, so only the latest wait calls back.
    private var generation = 0
    private let sleep: @Sendable (Duration) async -> Void
    private let settle: Duration
    private let onReturn: @Sendable () async -> Void

    init(
        settle: Duration = RouteReturn.settle,
        sleep: @escaping @Sendable (Duration) async -> Void = { try? await Task.sleep(for: $0) },
        onReturn: @escaping @Sendable () async -> Void
    ) {
        self.settle = settle
        self.sleep = sleep
        self.onReturn = onReturn
    }

    /// A route update: `satisfied` when the device has a route. Returns the task that calls
    /// back if this update starts one (tests wait on it).
    @discardableResult
    func update(satisfied: Bool) -> Task<Void, Never>? {
        lock.withLock {
            guard satisfied else {
                lost = true
                generation += 1
                pending?.cancel()
                pending = nil
                return nil
            }
            guard lost, pending == nil else { return nil }
            generation += 1
            let mine = generation
            let task = Task { [weak self, settle, sleep] in
                await sleep(settle)
                guard let self, self.settled(mine) else { return }
                await self.onReturn()
            }
            pending = task
            return task
        }
    }

    /// The wait `mine` ended with the route still up and no newer wait: it counts as back.
    private func settled(_ mine: Int) -> Bool {
        lock.withLock {
            guard mine == generation, pending != nil else { return false }
            lost = false
            pending = nil
            return true
        }
    }

    // MARK: The system's path

    private var monitor: NWPathMonitor?

    /// Follows the system's path from now on.
    func start() {
        let monitor = NWPathMonitor()
        monitor.pathUpdateHandler = { [weak self] path in
            self?.update(satisfied: path.status == .satisfied)
        }
        monitor.start(queue: DispatchQueue(label: "underbite.route-return", qos: .utility))
        lock.withLock { self.monitor = monitor }
    }

    deinit {
        monitor?.cancel()
        pending?.cancel()
    }
}
