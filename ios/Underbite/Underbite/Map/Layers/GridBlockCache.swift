import Foundation
import UnderbiteKit

/// The grid blocks kept on the device between launches: each block's ERDDAP response body as a
/// blob of the ``OfflineStore`` under the core's block key (`grid_requests` names it; an offline
/// pack's grid items use the same key, so the map draws a pack's blocks and a pack finds the
/// map's). The store bounds them with everything else it keeps.
///
/// A kept block goes back into the core's `GridStore` with its own fetch time, so the store asks
/// for it again when it should. Online, only while the core says it may still be used
/// (`grid_block_is_fresh`: MUR's newest week is revised daily, other days keep); offline, or
/// after a failed fetch, at any age.
///
/// Blocks kept by earlier versions in the fetch host's store (namespace ``legacyNamespace``) are
/// moved into the store on first use and deleted there.
actor GridBlockCache {
    /// One block handed back to the core's store.
    nonisolated struct Restored: Equatable, Sendable {
        let key: String
        /// When it was fetched, Unix seconds.
        let fetchedAt: Int64
        /// Whether the core counts it fresh at the time it was restored.
        let fresh: Bool
    }

    /// Where earlier versions kept the blocks: the fetch host's store, with an index.
    static let legacyNamespace = "grid"
    static let legacyIndexKey = "index"

    private let store: OfflineStore
    private let host: FetchHost?
    private var migrated = false

    /// - Parameters:
    ///   - store: the device's store.
    ///   - host: the fetch host whose store may still hold blocks of an earlier version.
    init(store: OfflineStore = MapOffline.store, host: FetchHost? = nil) {
        self.store = store
        self.host = host
    }

    /// Hands each block of `keys` that is kept to `grids`, with the time it was fetched: those
    /// the core counts fresh at `now`, and the stale ones too when `allowingStale` (offline, or
    /// after a failed fetch). Returns the blocks added. A kept body the core cannot read is
    /// skipped.
    func restore(
        _ keys: [String], into grids: GridStore, now: Int64, allowingStale: Bool = false
    ) async -> [Restored] {
        await migrate(now: now)
        var added: [Restored] = []
        for key in keys {
            guard
                let record = try? await store.get(.blob(key), now: now, allowingStale: true)
            else { continue }
            let fresh =
                (try? gridBlockIsFresh(key: key, fetchedAt: record.fetchedAt, now: now)) == true
            guard fresh || allowingStale else { continue }
            do {
                try grids.addGrid(
                    key: key, response: Self.response(record.data), now: record.fetchedAt)
                added.append(Restored(key: key, fetchedAt: record.fetchedAt, fresh: fresh))
            } catch {
                continue
            }
        }
        return added
    }

    /// Keeps the body of block `key`, fetched at `fetchedAt` and accepted by the core's store,
    /// with `lifetime` (the request's; whether it may be drawn is still the core's rule).
    func save(_ key: String, body: Data, contentType: String?, fetchedAt: Int64, lifetime: UInt64)
        async
    {
        let entry = OfflineEntry(
            data: body, contentType: contentType, fetchedAt: fetchedAt, lifetime: lifetime)
        _ = try? await store.put(.blob(key), entry, now: fetchedAt)
    }

    /// Moves the blocks an earlier version kept in the fetch host's store into the device's
    /// store, then deletes them there; once per launch, and nothing to do after the first.
    private func migrate(now: Int64) async {
        guard !migrated else { return }
        migrated = true
        guard let host,
            let json = host.load(namespace: Self.legacyNamespace, key: Self.legacyIndexKey)
        else { return }
        let entries = LegacyIndex.entries(json)
        var items: [(key: OfflineKey, entry: OfflineEntry)] = []
        for (key, fetchedAt) in entries {
            guard let name = LegacyIndex.name(of: key) else { continue }
            if let body = host.load(namespace: Self.legacyNamespace, key: name), !body.isEmpty {
                items.append(
                    (
                        .blob(key),
                        OfflineEntry(
                            data: Data(body.utf8), contentType: "application/json",
                            fetchedAt: fetchedAt, expiresAt: fetchedAt)
                    ))
            }
            host.store(namespace: Self.legacyNamespace, key: name, value: "")
        }
        if !items.isEmpty { _ = try? await store.put(items, now: now) }
        host.store(namespace: Self.legacyNamespace, key: Self.legacyIndexKey, value: "")
    }

    /// A kept body as the response the core's store reads: ERDDAP's JSON with a success status.
    static func response(_ body: Data) -> HostResponse {
        HostResponse(
            status: 200, body: body, contentType: "application/json;charset=UTF-8",
            date: nil, expires: nil, lastModified: nil, etag: nil, retryAfter: nil,
            rateLimitRemaining: nil)
    }
}

/// The index earlier versions kept beside the blocks in the fetch host's store, read only to move
/// them into the device's store.
nonisolated enum LegacyIndex {
    private struct Stored: Decodable {
        struct Entry: Decodable {
            var fetchedAt: Int64
        }
        var entries: [String: Entry]
    }

    /// Each block key with its fetch time; none for an index that cannot be read.
    static func entries(_ json: String) -> [(key: String, fetchedAt: Int64)] {
        guard let stored = try? JSONDecoder().decode(Stored.self, from: Data(json.utf8)) else {
            return []
        }
        return stored.entries.map { ($0.key, $0.value.fetchedAt) }.sorted { $0.key < $1.key }
    }

    /// The name a block's body was stored under: the key with its `/` as `.`; none for a key
    /// that could not have been stored.
    static func name(of key: String) -> String? {
        guard !key.isEmpty, !key.contains("."), !key.hasPrefix("/") else { return nil }
        let name = key.replacingOccurrences(of: "/", with: ".")
        let allowed = name.unicodeScalars.allSatisfy {
            ("a"..."z").contains($0) || ("A"..."Z").contains($0) || ("0"..."9").contains($0)
                || $0 == "-" || $0 == "_" || $0 == "."
        }
        return allowed ? name : nil
    }
}
