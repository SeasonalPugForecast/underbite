import Foundation
import Network
import Observation
import UnderbiteKit

/// The map's side of the offline store: the one store the app opens, whether the device has a
/// route to the network, and the rules that decide, per tile or grid block, between the copy kept
/// on the device and the network.
nonisolated enum MapOffline {
    /// The app's offline store (`Application Support/offline`): one actor owns its one
    /// connection, so the map, the packs and the storage page all use this one.
    static let store = OfflineStore()

    /// Opens the store, starting it afresh when its file was written by another version of the
    /// store (a cache: everything in it can be fetched again).
    static func prepare(_ store: OfflineStore = MapOffline.store) async {
        do {
            try await store.open()
        } catch OfflineStoreError.schema {
            try? await store.reset()
        } catch {}
    }

    /// The layer whose tiles are never written to the store: a radar picture is good for
    /// minutes, and an old one would pass for the weather now.
    static let memoryOnlyLayers: Set<String> = ["radar"]

    /// Whether the response to `request` for `layer` may be written to the store: not when the
    /// core marks it `no_store` or gives it no lifetime (a satellite layer's current day, which
    /// changes during the day), nor for a ``memoryOnlyLayers`` layer.
    static func persists(layer: String, _ request: HostRequest) -> Bool {
        guard !request.noStore, let lifetime = request.maxAgeS, lifetime > 0 else { return false }
        return !memoryOnlyLayers.contains(layer)
    }

    /// How long a tile the store never sees is kept fresh in memory: the satellite layers' current
    /// day fills in as passes arrive, so it is asked for again within the hour.
    static let memoryOnlyLifetime: TimeInterval = 3_600
}

// MARK: - The route to the network

/// Whether the device has a route to the network, from `NWPathMonitor`: no route is offline, and
/// the map then reads the store without asking. A debug build launched with `-offline` is offline
/// whatever the path (screenshots of the offline map).
nonisolated final class NetworkRoute: @unchecked Sendable {
    static let shared = NetworkRoute()

    private let lock = NSLock()
    private var satisfied = true
    private let forced: Bool
    private let monitor = NWPathMonitor()

    init(forcedOffline: Bool = NetworkRoute.launchOffline) {
        forced = forcedOffline
        monitor.pathUpdateHandler = { [weak self] path in
            self?.update(path.status == .satisfied)
        }
        monitor.start(queue: DispatchQueue(label: "underbite.network-route", qos: .utility))
    }

    deinit {
        monitor.cancel()
    }

    /// No route to the network (or `-offline` in a debug build).
    var isOffline: Bool {
        forced || lock.withLock { !satisfied }
    }

    private func update(_ satisfied: Bool) {
        lock.withLock { self.satisfied = satisfied }
    }

    /// Whether the launch asked for the offline map: `-offline`, in debug builds only.
    static var launchOffline: Bool {
        #if DEBUG
            ProcessInfo.processInfo.arguments.contains("-offline")
        #else
            false
        #endif
    }
}

// MARK: - Read-through

/// What the map does for one tile or block, from the copy kept on the device and the route: the
/// store first, the network for what is missing or stale, the stored copy again when the network
/// fails.
nonisolated enum ReadThrough: Equatable, Sendable {
    /// Draw the kept copy without asking: it is fresh, or the device is offline.
    case serve
    /// Ask the network; on failure draw the kept copy if there is one.
    case fetch
    /// Offline with nothing kept: nothing to draw, and the layer says so.
    case missing

    /// The step for a kept copy (`kept`: none; else whether it is stale) while `offline`.
    static func step(kept: Bool?, offline: Bool) -> ReadThrough {
        switch (kept, offline) {
        case (.some(_), true): .serve
        case (.some(false), false): .serve
        case (.some(true), false): .fetch
        case (.none, true): .missing
        case (.none, false): .fetch
        }
    }

    /// Whether a fetch that ended without something to draw falls back to the kept copy: after a
    /// transport failure, a server error or a refusal; not when the source said it has nothing
    /// there (`noData`).
    static func fallsBack(after outcome: TileOutcome?) -> Bool {
        switch outcome {
        case .none, .failed, .retry: true
        case .image, .noData, .tooDeep: false
        }
    }
}

// MARK: - What each layer drew from

/// Where one tile or block a layer drew came from.
nonisolated enum PieceSource: Equatable, Sendable {
    /// The network, or a copy kept while it is fresh and the device online.
    case current
    /// A copy kept on the device, fetched at this time (Unix seconds): offline, or after a
    /// failed fetch.
    case stored(fetchedAt: Int64)
    /// Nothing to draw: offline (or the request failed at the transport) with nothing kept.
    case missing
}

/// What a layer's badge says.
nonisolated enum LayerBadge: Equatable, Sendable {
    /// "cached <date>": some of what it draws is a kept copy, the oldest fetched then.
    case cached(fetchedAt: Int64)
    /// "not available offline": offline, nothing of it is kept.
    case notAvailable
}

/// The tiles or blocks of one layer and day, each with where it last came from. The layer is
/// badged once, never per tile: "cached" with the oldest kept copy drawn while any is, else "not
/// available offline" while pieces are missing.
nonisolated struct LayerPieces: Equatable, Sendable {
    /// The most pieces remembered per layer and day; past it, those drawn current go first.
    static let capacity = 2_048

    private(set) var pieces: [String: PieceSource] = [:]

    mutating func record(_ piece: String, _ source: PieceSource) {
        pieces[piece] = source
        guard pieces.count > Self.capacity else { return }
        pieces = pieces.filter { $0.value != .current }
        if pieces.count > Self.capacity { pieces = [piece: source] }
    }

    var badge: LayerBadge? {
        var oldest: Int64?
        var missing = false
        for source in pieces.values {
            switch source {
            case .current: continue
            case .stored(let at): oldest = min(oldest ?? at, at)
            case .missing: missing = true
            }
        }
        if let oldest { return .cached(fetchedAt: oldest) }
        return missing ? .notAvailable : nil
    }
}

/// Per layer and day, what its tiles and blocks were drawn from, for the badges over the map.
/// The loaders report from their own threads; the record is kept on the main thread.
@MainActor
@Observable
final class MapOfflineStatus {
    nonisolated static let shared = MapOfflineStatus()

    private(set) var layers: [String: LayerPieces] = [:]

    nonisolated init() {}

    /// The key of a tiled layer's day (`date`: none for the newest) or a grid layer's time
    /// (Unix seconds as text).
    nonisolated static func key(layer: String, date: String?) -> String {
        "\(layer)|\(date ?? "-")"
    }

    /// Reports from any thread that `piece` of `layer` on `date` came from `source`.
    nonisolated func report(layer: String, date: String?, piece: String, _ source: PieceSource) {
        let key = Self.key(layer: layer, date: date)
        Task { @MainActor in self.record(key, piece: piece, source) }
    }

    func record(_ key: String, piece: String, _ source: PieceSource) {
        var pieces = layers[key] ?? LayerPieces()
        pieces.record(piece, source)
        if layers[key] != pieces { layers[key] = pieces }
    }

    /// The badge of `layer` on `date`, if any.
    func badge(layer: String, date: String?) -> LayerBadge? {
        layers[Self.key(layer: layer, date: date)]?.badge
    }
}
