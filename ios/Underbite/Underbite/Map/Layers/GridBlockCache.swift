import Foundation
import UnderbiteKit

/// The grid blocks kept on the device between launches: each block's ERDDAP response body,
/// stored through the fetch host's store (namespace ``namespace``, one value per block under
/// ``GridBlockIndex/name(of:)``, never a `URLSession` or a file of its own), and the
/// ``GridBlockIndex`` beside them, bounded to ``GridBlockIndex/capacity`` with the least recently
/// used blocks dropped first.
///
/// A kept block goes back into the core's `GridStore` only while the core says it may still be
/// used (`grid_block_is_fresh` on its key and fetch time), and with its own fetch time, so the
/// store asks for it again when it should. Overviews are kept like any block.
actor GridBlockCache {
    static let namespace = "grid"
    static let indexKey = "index"

    private let host: FetchHost
    private let capacity: Int
    /// Read from the host on first use, not on the main thread.
    private var loaded: GridBlockIndex?
    /// Uses not yet written back.
    private var dirty = false

    init(host: FetchHost, capacity: Int = GridBlockIndex.capacity) {
        self.host = host
        self.capacity = capacity
    }

    private var index: GridBlockIndex {
        get {
            if let loaded { return loaded }
            let stored = GridBlockIndex.stored(
                host.load(namespace: Self.namespace, key: Self.indexKey))
            loaded = stored
            return stored
        }
        set { loaded = newValue }
    }

    /// The index as it stands (tests).
    var snapshot: GridBlockIndex { index }

    /// Hands each block of `keys` that is kept and still fresh at `now` to `store`, with the time
    /// it was fetched; returns the keys added. A kept body the store refuses is forgotten.
    func restore(_ keys: [String], into store: GridStore, now: Int64) -> [String] {
        var added: [String] = []
        for key in keys {
            guard let entry = index.entry(key), let name = GridBlockIndex.name(of: key) else {
                continue
            }
            guard (try? gridBlockIsFresh(key: key, fetchedAt: entry.fetchedAt, now: now)) == true
            else { continue }
            guard let body = host.load(namespace: Self.namespace, key: name) else {
                index.remove(key)
                dirty = true
                continue
            }
            do {
                try store.addGrid(key: key, response: Self.response(body), now: entry.fetchedAt)
                index.touch(key)
                dirty = true
                added.append(key)
            } catch {
                forget(key, name: name)
            }
        }
        return added
    }

    /// Keeps the body of block `key`, fetched at `fetchedAt` and accepted by the store, and
    /// writes the index at once (a body without an entry would never be dropped).
    func save(_ key: String, body: Data, fetchedAt: Int64) {
        guard let name = GridBlockIndex.name(of: key),
            let text = String(data: body, encoding: .utf8), !text.isEmpty
        else { return }
        var updated = self.index
        let dropped = updated.put(key, bytes: body.count, fetchedAt: fetchedAt, capacity: capacity)
        self.index = updated
        if updated.entry(key) != nil {
            host.store(namespace: Self.namespace, key: name, value: text)
        }
        for old in dropped {
            if let oldName = GridBlockIndex.name(of: old) {
                host.store(namespace: Self.namespace, key: oldName, value: "")
            }
        }
        write()
    }

    /// Writes the index if blocks were used since it was last written.
    func flush() {
        if dirty { write() }
    }

    private func forget(_ key: String, name: String) {
        index.remove(key)
        host.store(namespace: Self.namespace, key: name, value: "")
        dirty = true
    }

    private func write() {
        host.store(namespace: Self.namespace, key: Self.indexKey, value: index.json())
        dirty = false
    }

    /// A kept body as the response the store reads: ERDDAP's JSON with a success status.
    static func response(_ body: String) -> HostResponse {
        HostResponse(
            status: 200, body: Data(body.utf8), contentType: "application/json;charset=UTF-8",
            date: nil, expires: nil, lastModified: nil, etag: nil, retryAfter: nil,
            rateLimitRemaining: nil)
    }
}
