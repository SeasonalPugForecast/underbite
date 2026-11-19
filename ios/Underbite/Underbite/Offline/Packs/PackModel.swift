import Foundation
import UnderbiteKit

/// The few fields of a stored pack the screens and the downloader read, decoded from the core's
/// pack JSON. The JSON itself is only ever written by the core (`pack_new`,
/// `pack_mark_refreshed`); this is a read-only view of it.
nonisolated struct PackInfo: Equatable, Sendable {
    let id: String
    let name: String
    let area: PackBox
    let minZoom: UInt8
    let maxZoom: UInt8
    let layers: [String]
    let spots: [String]
    /// When the last download completed, Unix seconds; none before the first.
    let refreshedAt: Int64?
    /// Whether it holds the newest data only (no days chosen).
    let isLatest: Bool

    private struct Stored: Decodable {
        struct Box: Decodable {
            let west: Double
            let south: Double
            let east: Double
            let north: Double
        }
        let id: String
        let name: String
        let bbox: Box
        let min_zoom: UInt8
        let max_zoom: UInt8
        let layers: [String]
        let spots: [String]
        let refreshed_at: Int64?
        let dates: Dates
    }

    /// The day policy, read only as far as "the newest only" (the core writes it as a bare
    /// string; the other policies are objects).
    private enum Dates: Decodable {
        case latest
        case other

        init(from decoder: Decoder) throws {
            let c = try decoder.singleValueContainer()
            self = (try? c.decode(String.self)) == "latest" ? .latest : .other
        }
    }

    /// The fields of `json`, or none when it is not a pack the core wrote.
    init?(json: String) {
        guard let s = try? JSONDecoder().decode(Stored.self, from: Data(json.utf8)) else {
            return nil
        }
        id = s.id
        name = s.name
        area = PackBox(west: s.bbox.west, south: s.bbox.south, east: s.bbox.east, north: s.bbox.north)
        minZoom = s.min_zoom
        maxZoom = s.max_zoom
        layers = s.layers
        spots = s.spots
        refreshedAt = s.refreshed_at
        isLatest = s.dates == .latest
    }
}

/// Where a pack's download stands.
nonisolated enum PackStatus: String, Codable, Equatable, Sendable {
    case notDownloaded
    case downloading
    case waitingForWifi
    case paused
    case done
    case failed
    case cancelled

    /// Whether a download is under way (sending, or waiting for Wi-Fi).
    var isRunning: Bool { self == .downloading || self == .waitingForWifi }

    /// The core's word for it.
    func text(_ words: PackStatusWords) -> String {
        switch self {
        case .notDownloaded: words.notDownloaded
        case .downloading: words.downloading
        case .waitingForWifi: words.waitingForWifi
        case .paused: words.paused
        case .done: words.done
        case .failed: words.failed
        case .cancelled: words.cancelled
        }
    }
}

/// Where a download stopped, kept so that it resumes there: the plan is made again with the same
/// time and axes (so it lists the same items in the same order), the pages before `nextIndex`
/// are done, and in the page at `nextIndex` an item already stored since `startedAt` is not
/// asked for again.
nonisolated struct PackProgress: Codable, Equatable, Sendable {
    /// When this download started, Unix seconds: a copy stored since then is this download's.
    var startedAt: Int64
    /// The time the pack was planned at (a `Latest` pack's newest day follows it).
    var plannedAt: Int64
    /// The times of the pack's gridded layers it was planned with, by layer id.
    var axes: [String: [Int64]] = [:]
    /// Whether the download refreshes a pack already downloaded: its items are pinned apart
    /// until it completes, so the copies the pack holds stay until the new ones are in.
    var staging: Bool = false
    /// The items of the pages done.
    var nextIndex: UInt64 = 0
    var total: UInt64 = 0
    var done: UInt64 = 0
    var failed: UInt64 = 0
    var bytes: UInt64 = 0
    /// Answers asking to slow down (HTTP 429 or 503, or a busy ERDDAP server).
    var throttled: UInt64 = 0
    /// Bytes and items stored per layer, for the next estimate.
    var layerBytes: [String: UInt64] = [:]
    var layerItems: [String: UInt64] = [:]

    init(startedAt: Int64, plannedAt: Int64, axes: [String: [Int64]] = [:], staging: Bool = false) {
        self.startedAt = startedAt
        self.plannedAt = plannedAt
        self.axes = axes
        self.staging = staging
    }

    /// The share done, 0 to 1.
    var fraction: Double {
        total == 0 ? 0 : min(1, Double(done) / Double(total))
    }
}

/// A stored pack: the core's JSON, where its download stands and where it stopped.
nonisolated struct PackRecord: Codable, Equatable, Sendable, Identifiable {
    var json: String
    var status: PackStatus = .notDownloaded
    /// The download under way or stopped; none once it completed or was cancelled.
    var progress: PackProgress?
    /// The last completed download's counts, for its row.
    var lastDone: UInt64?
    var lastFailed: UInt64?
    /// Why the last download failed, in the core's words.
    var problem: String?
    /// Paused by a tap: it waits for one to resume (a download the app was stopped in the
    /// middle of resumes by itself).
    var userPaused = false

    var info: PackInfo? { PackInfo(json: json) }
    var id: String { info?.id ?? "" }
}

/// The packs file: the pack definitions, where their downloads stand, the Wi-Fi choice and the
/// sizes measured. It is the app's own JSON beside its other files, never part of the settings,
/// so a settings export never carries a pack.
nonisolated struct PacksFile: Codable, Equatable, Sendable {
    static let currentVersion = 1
    static let name = "packs.json"

    var version = PacksFile.currentVersion
    /// Downloads wait while the path is expensive or constrained.
    var wifiOnly = true
    var packs: [PackRecord] = []
    /// Average bytes per item measured by downloads, by layer id (`depth-dem` for elevation
    /// areas): the estimates use them over the catalogue's seeds.
    var measured: [String: UInt64] = [:]

    init() {}

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        version = try c.decodeIfPresent(Int.self, forKey: .version) ?? Self.currentVersion
        wifiOnly = try c.decodeIfPresent(Bool.self, forKey: .wifiOnly) ?? true
        packs = try c.decodeIfPresent([PackRecord].self, forKey: .packs) ?? []
        measured = try c.decodeIfPresent([String: UInt64].self, forKey: .measured) ?? [:]
    }

    /// The file in `files`; a new one when there is none or it cannot be read (set aside then).
    static func load(_ files: AppFiles) -> PacksFile {
        do {
            return try files.decode(PacksFile.self, name) ?? PacksFile()
        } catch {
            _ = files.setAside(name)
            return PacksFile()
        }
    }

    @discardableResult
    func save(_ files: AppFiles) -> Bool {
        files.write(self, Self.name)
    }

    /// The averages of `progress`'s stored items merged into `measured`.
    mutating func measure(_ progress: PackProgress) {
        for (layer, items) in progress.layerItems where items > 0 {
            let bytes = progress.layerBytes[layer] ?? 0
            measured[layer] = bytes / items
        }
    }
}

/// The words of an error from the core or the store.
nonisolated enum PackErrors {
    /// The core's own message for its errors, the description of anything else.
    static func message(of error: Error) -> String {
        switch error {
        case MapError.Invalid(let message): message
        default: String(describing: error)
        }
    }
}

extension PackBox {
    /// The box around a map region.
    nonisolated init(region: MapRegion) {
        self.init(west: region.west, south: region.south, east: region.east, north: region.north)
    }

    /// Whether the point is inside (a box crossing the antimeridian has `west` > `east`).
    nonisolated func contains(latitude: Double, longitude: Double) -> Bool {
        guard latitude >= south, latitude <= north else { return false }
        return west <= east
            ? longitude >= west && longitude <= east
            : longitude >= west || longitude <= east
    }
}
