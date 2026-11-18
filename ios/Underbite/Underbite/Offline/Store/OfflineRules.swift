import Foundation

/// A response to keep in the offline store. Times are Unix seconds, as the core's.
nonisolated struct OfflineEntry: Equatable, Sendable {
    /// The response body.
    let data: Data
    /// The response's content type (kept for tiles; checked, not kept, for blobs).
    let contentType: String?
    /// The response's entity tag, for asking the server whether the copy is still current
    /// (kept for tiles).
    let etag: String?
    /// When it was fetched.
    let fetchedAt: Int64
    /// Until when it may be used without asking again; at or before ``fetchedAt`` it is stale
    /// at once and only read when stale copies are allowed.
    let expiresAt: Int64

    init(
        data: Data, contentType: String?, etag: String? = nil, fetchedAt: Int64, expiresAt: Int64
    ) {
        self.data = data
        self.contentType = contentType
        self.etag = etag
        self.fetchedAt = fetchedAt
        self.expiresAt = expiresAt
    }

    /// An entry fresh for `lifetime` seconds (the request's `maxAgeS`) from `fetchedAt`.
    init(
        data: Data, contentType: String?, etag: String? = nil, fetchedAt: Int64, lifetime: UInt64
    ) {
        let lifetime = Int64(clamping: lifetime)
        let (expires, overflow) = fetchedAt.addingReportingOverflow(lifetime)
        self.init(
            data: data, contentType: contentType, etag: etag, fetchedAt: fetchedAt,
            expiresAt: overflow ? .max : expires)
    }
}

/// A copy read from the offline store.
nonisolated struct OfflineRecord: Equatable, Sendable {
    let data: Data
    /// The content type stored with a tile; none for a blob.
    let contentType: String?
    /// The entity tag stored with a tile; none for a blob.
    let etag: String?
    /// When the copy was fetched, Unix seconds: the age to show beside what it draws.
    let fetchedAt: Int64
    let expiresAt: Int64
    /// Whether the copy was past its lifetime when it was read (only read so when stale copies
    /// were allowed: offline, or after a failed fetch).
    let isStale: Bool
}

/// What the store holds, for the storage page and the pack downloader.
nonisolated struct OfflineUsage: Equatable, Sendable {
    /// The bytes of every stored body.
    let bytes: Int64
    /// The bytes of the bodies a pack pins.
    let pinnedBytes: Int64
    let tiles: Int
    let blobs: Int
    /// The bound past which unpinned rows are dropped.
    let limitBytes: Int64
}

/// What one pack holds in the store.
nonisolated struct OfflinePackUsage: Equatable, Sendable {
    /// The rows the pack pins.
    let items: Int
    /// Those of them stored.
    let stored: Int
    /// The bytes of their bodies (shared rows counted in every pack that pins them).
    let bytes: Int64
}

/// The store's rules, apart from SQLite: what may be kept, when a copy is fresh, the order of
/// use and which rows go when the store is over its bound.
nonisolated enum OfflineRules {
    // MARK: What may be kept

    /// Whether `entry` may be kept as a row of `kind`. Only answers the core accepted are stored
    /// (an image `classify_tile` called one, a grid block the grid store took, metadata its
    /// provider read); this check is the last guard against an error kept as data: an empty
    /// body, a tile that is not an image (the chart service answers failures with status 200
    /// and a web page) or a blob that is a web page.
    static func admits(_ entry: OfflineEntry, as kind: OfflineKind) -> Bool {
        guard !entry.data.isEmpty, entry.fetchedAt >= 0 else { return false }
        switch kind {
        case .tile:
            return isImage(entry.contentType) && !isWebPage(entry.data)
        case .blob:
            return !isWebPageType(entry.contentType) && !isWebPage(entry.data)
        }
    }

    /// Whether `contentType` names an image (`image/png`, `image/jpeg; charset=...`).
    static func isImage(_ contentType: String?) -> Bool {
        mediaType(contentType)?.hasPrefix("image/") == true
    }

    static func isWebPageType(_ contentType: String?) -> Bool {
        guard let media = mediaType(contentType) else { return false }
        return media == "text/html" || media == "application/xhtml+xml"
    }

    /// Whether `data` starts, after white space, as an HTML page.
    static func isWebPage(_ data: Data) -> Bool {
        let space: Set<UInt8> = [0x20, 0x09, 0x0A, 0x0D]
        let head = data.prefix(64).drop(while: { space.contains($0) })
        guard let text = String(data: Data(head), encoding: .ascii)?.lowercased() else {
            return false
        }
        return text.hasPrefix("<!doctype html") || text.hasPrefix("<html")
    }

    private static func mediaType(_ contentType: String?) -> String? {
        guard let contentType else { return nil }
        let media = contentType.split(separator: ";", maxSplits: 1).first ?? ""
        let trimmed = media.trimmingCharacters(in: .whitespaces).lowercased()
        return trimmed.isEmpty ? nil : trimmed
    }

    // MARK: Freshness

    /// Whether a copy that expires at `expiresAt` may be used without asking again at `now`.
    static func isFresh(expiresAt: Int64, now: Int64) -> Bool {
        now < expiresAt
    }

    // MARK: Order of use

    /// The use stamp of a row used at `now` (Unix seconds) after the last stamp `last`: the time
    /// in milliseconds, raised past `last` so that every use orders after the one before even
    /// within a millisecond or when the clock steps back.
    static func stamp(now: Int64, after last: Int64) -> Int64 {
        let (millis, overflow) = now.multipliedReportingOverflow(by: 1000)
        let time = overflow ? (now < 0 ? Int64.min : Int64.max) : millis
        return last == .max ? .max : max(time, last + 1)
    }

    // MARK: Bound and eviction

    /// 1,500 MB: `map.offline_max_mb`'s default.
    static let defaultLimitMegabytes = 1500

    /// The bound of `megabytes` (`map.offline_max_mb`) in bytes; a megabyte is 1,000,000 bytes, as
    /// the system's storage figures count. Zero or less keeps nothing unpinned.
    static func limitBytes(megabytes: Int) -> Int64 {
        let (bytes, overflow) = Int64(max(0, megabytes)).multipliedReportingOverflow(by: 1_000_000)
        return overflow ? .max : bytes
    }

    /// Where eviction stops once the store passes `limit`: 95% of it, so that a full store does
    /// not drop a row on every write.
    static func lowWater(_ limit: Int64) -> Int64 {
        max(0, limit - limit / 20)
    }

    /// An unpinned row eviction may drop.
    nonisolated struct Candidate: Equatable, Sendable {
        let kind: OfflineKind
        let rowid: Int64
        let bytes: Int64
        let used: Int64
    }

    /// The candidates of both tables in order of last use (oldest first), from each table's
    /// oldest `batch` candidates. When a table gave a full batch, rows past it may be older than
    /// some of the other table's, so the merge stops at the newest use both lists still cover.
    static func oldestFirst(
        tiles: [Candidate], blobs: [Candidate], batch: Int
    ) -> [Candidate] {
        var cutoff = Int64.max
        if tiles.count >= batch, let last = tiles.last { cutoff = min(cutoff, last.used) }
        if blobs.count >= batch, let last = blobs.last { cutoff = min(cutoff, last.used) }
        return (tiles + blobs).filter { $0.used <= cutoff }.sorted {
            ($0.used, $0.kind.rawValue, $0.rowid) < ($1.used, $1.kind.rawValue, $1.rowid)
        }
    }

    /// The leading `candidates` to drop so that a store of `total` bytes comes down to
    /// `target`: none when it is already there.
    static func victims(_ candidates: [Candidate], total: Int64, target: Int64) -> [Candidate] {
        var total = total
        var dropped: [Candidate] = []
        for candidate in candidates where total > target {
            dropped.append(candidate)
            total -= candidate.bytes
        }
        return dropped
    }
}
