import Foundation
import Network
import UnderbiteKit

/// The rules of a pack download, apart from the network: how many requests go at once, when a
/// failed one is asked again, and whether the path lets a download run.
nonisolated enum PackRules {
    /// Requests in flight to one host at most: public services publish no limits, so a pack
    /// stays well under what a browser opens.
    static let perHost = 4
    /// Requests in flight in all.
    static let total = 8
    /// Items read from the plan at a time; where a download stopped is kept per page.
    static let pageSize: UInt32 = 200
    /// Tries of one item before it counts as failed.
    static let attempts: UInt32 = 4
    /// The longest wait between tries.
    static let longestBackOff: UInt32 = 60
    /// How often a download waiting for Wi-Fi looks at the path again, seconds.
    static let routeCheck: UInt32 = 5

    /// The wait before try `attempt + 1` after try `attempt` (from 1) failed at the transport or
    /// the server: 2, 4, 8 ... seconds, at most ``longestBackOff``.
    static func backOff(after attempt: UInt32) -> UInt32 {
        let shift = min(max(attempt, 1), 16)
        return min(UInt32(1) << shift, longestBackOff)
    }

    /// The wait a server asked for, kept within ``longestBackOff`` so that a download keeps
    /// moving (it asks again then, and the core bounds the tries).
    static func serverWait(_ seconds: UInt32) -> UInt32 {
        min(max(seconds, 1), longestBackOff)
    }

    /// Whether a download may send on `route`: never without one, and with "Wi-Fi only" only
    /// on a path that is neither expensive (cellular, a personal hotspot) nor constrained (Low
    /// Data Mode).
    static func maySend(on route: PackRoute, wifiOnly: Bool) -> Bool {
        switch route {
        case .none: false
        case .metered: !wifiOnly
        case .unmetered: true
        }
    }

    /// The store's pin of a pack's download: the pack's id, or a separate pin while a refresh
    /// replaces what the pack holds.
    static func pin(_ packID: String, staging: Bool) -> String {
        staging ? packID + "+refresh" : packID
    }

    /// The store row of a pack item.
    static func storeKey(_ request: PackRequest) -> OfflineKey? {
        let kind: OfflinePackItemKind =
            switch request.kind {
            case .tile: .tile
            case .grid: .grid
            case .dem: .dem
            }
        return OfflineKey(pack: kind, key: request.key)
    }
}

/// A choice read from any thread: "Wi-Fi only", which a running download follows.
nonisolated final class PackFlag: @unchecked Sendable {
    private let lock = NSLock()
    private var on: Bool

    init(_ on: Bool) { self.on = on }

    var value: Bool {
        get { lock.withLock { on } }
        set { lock.withLock { on = newValue } }
    }
}

/// The path to the network as a pack download sees it.
nonisolated enum PackRoute: Equatable, Sendable {
    /// No route.
    case none
    /// Expensive (cellular, a personal hotspot) or constrained (Low Data Mode).
    case metered
    /// Wi-Fi or wired, without Low Data Mode.
    case unmetered

    /// The route of a path: `isExpensive` and `isConstrained` are `NWPath`'s own flags.
    static func of(satisfied: Bool, expensive: Bool, constrained: Bool) -> PackRoute {
        guard satisfied else { return .none }
        return expensive || constrained ? .metered : .unmetered
    }
}

/// The device's path, from `NWPathMonitor`. A debug build launched with `-offline` has none.
nonisolated final class PackPath: @unchecked Sendable {
    static let shared = PackPath()

    private let lock = NSLock()
    private var route: PackRoute = .unmetered
    private let monitor = NWPathMonitor()

    init() {
        monitor.pathUpdateHandler = { [weak self] path in
            let route = PackRoute.of(
                satisfied: path.status == .satisfied, expensive: path.isExpensive,
                constrained: path.isConstrained)
            self?.lock.withLock { self?.route = route }
        }
        monitor.start(queue: DispatchQueue(label: "underbite.pack-path", qos: .utility))
    }

    deinit {
        monitor.cancel()
    }

    var current: PackRoute {
        NetworkRoute.launchOffline ? .none : lock.withLock { route }
    }
}

/// Requests in flight per host, within ``PackRules/perHost`` and ``PackRules/total``.
nonisolated struct PackSlots: Equatable, Sendable {
    private(set) var inFlight: [String: Int] = [:]
    private(set) var running = 0
    let perHost: Int
    let total: Int

    init(perHost: Int = PackRules.perHost, total: Int = PackRules.total) {
        self.perHost = perHost
        self.total = total
    }

    func hasRoom(for host: String) -> Bool {
        running < total && inFlight[host, default: 0] < perHost
    }

    /// Takes a slot for `host`; false when there is none.
    mutating func take(_ host: String) -> Bool {
        guard hasRoom(for: host) else { return false }
        inFlight[host, default: 0] += 1
        running += 1
        return true
    }

    mutating func release(_ host: String) {
        guard let n = inFlight[host], n > 0 else { return }
        inFlight[host] = n == 1 ? nil : n - 1
        running -= 1
    }

    /// The first of `hosts` (in order) with a free slot.
    func firstWithRoom(_ hosts: [String]) -> Int? {
        hosts.firstIndex { hasRoom(for: $0) }
    }
}
