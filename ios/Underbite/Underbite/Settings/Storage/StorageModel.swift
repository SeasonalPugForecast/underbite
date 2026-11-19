import Foundation
import Observation
@preconcurrency import UnderbiteKit

/// One pack of the storage page: its name and what it holds, in the core's words.
nonisolated struct StoragePackRow: Equatable, Identifiable, Sendable {
    let id: String
    let name: String
    /// "48.3 MB".
    let sizeText: String
    /// "Refreshed Nov 17", "Not downloaded yet".
    let refreshedText: String
}

/// A choice of the offline limit: megabytes as `map.offline_max_mb` stores them, and the core's
/// label.
nonisolated struct StorageLimitChoice: Equatable, Identifiable, Sendable {
    let megabytes: Int
    let label: String
    var id: Int { megabytes }
}

/// What the storage page shows, every word and number from the core: the store's total against
/// the limit, the packs' share and the map cache's, each pack's size and the limit's choices.
nonisolated struct StorageFigures: Equatable, Sendable {
    let words: StorageView
    /// How full the store is, 0 to 1 (none without a limit).
    let fill: Double?
    let packs: [StoragePackRow]
    let limits: [StorageLimitChoice]
    /// The limit the settings hold, megabytes.
    let limitMegabytes: Int

    /// The figures of `usage` under the settings `settingsJSON`, with the packs (`id`, its JSON
    /// and what the store holds of it) at Unix time `now` in `timeZone`.
    static func make(
        usage: OfflineUsage, packs: [(id: String, json: String, bytes: Int64)],
        settingsJSON: String, now: Int64, timeZone: String
    ) -> StorageFigures {
        let limit = max(0, (try? Settings(json: settingsJSON))?.map.offlineMaxMb ?? 0)
        let total = UInt64(max(0, usage.bytes))
        let pinned = UInt64(max(0, min(usage.pinnedBytes, usage.bytes)))
        // What the packs hold: the sum of their own rows (a row two packs share counts for
        // both, so it is bounded by what is pinned). The spots' kept metadata is pinned too, but
        // it is no pack's, and clearing the map cache does not free it.
        let held = packs.reduce(into: UInt64(0)) { $0 += UInt64(max(0, $1.bytes)) }
        let inPacks = min(held, pinned)
        // "Packs" is the packs' share; "Map cache" is what is not pinned at all, which is
        // what clearing frees. Both phrases are the core's.
        let packWords = storageView(
            totalBytes: total, pinnedBytes: inPacks, limitMb: UInt32(clamping: limit),
            settingsJson: settingsJSON)
        let cacheWords = storageView(
            totalBytes: total, pinnedBytes: pinned, limitMb: UInt32(clamping: limit),
            settingsJson: settingsJSON)
        let words = StorageView(
            totalText: packWords.totalText, packsText: packWords.packsText,
            cacheText: cacheWords.cacheText, limitText: packWords.limitText)
        let limitBytes = OfflineRules.limitBytes(megabytes: limit)
        let fill = limitBytes > 0 ? min(1, Double(total) / Double(limitBytes)) : nil
        let rows = packs.compactMap { pack -> StoragePackRow? in
            guard let info = PackInfo(json: pack.json),
                let row = try? packRow(
                    packJson: pack.json, storedBytes: UInt64(max(0, pack.bytes)), now: now,
                    settingsJson: settingsJSON, timeZone: timeZone)
            else { return nil }
            return StoragePackRow(
                id: pack.id, name: info.name, sizeText: row.sizeText,
                refreshedText: row.refreshedText)
        }
        return StorageFigures(
            words: words, fill: fill, packs: rows,
            limits: limitChoices(settingsJSON: settingsJSON), limitMegabytes: limit)
    }

    /// The core's choices of the limit, and "no map cache" (0) first: its words say what 0 does.
    static func limitChoices(settingsJSON: String) -> [StorageLimitChoice] {
        let off = storageView(totalBytes: 0, pinnedBytes: 0, limitMb: 0, settingsJson: settingsJSON)
        let stops = offlineLimitStops(settingsJson: settingsJSON).map {
            StorageLimitChoice(megabytes: Int($0.value.rounded()), label: $0.label)
        }
        return [StorageLimitChoice(megabytes: 0, label: off.limitText)]
            + stops.filter { $0.megabytes > 0 }
    }
}

/// The storage page's state: reads the app's one offline store and the packs, clears the map
/// cache and bounds the store to a new limit. Clearing drops only unpinned rows: what the packs
/// hold and the spots' kept station metadata are pinned, and the spots' last conditions are not
/// in this store at all.
@MainActor
@Observable
final class StorageModel {
    private(set) var figures: StorageFigures?
    /// Whether the map cache is being cleared.
    private(set) var clearing = false

    @ObservationIgnored private let store: OfflineStore
    @ObservationIgnored private let prepare: @Sendable () async -> Void
    @ObservationIgnored private let clock: @Sendable () -> Int64

    init(
        store: OfflineStore = MapOffline.store,
        prepare: @escaping @Sendable () async -> Void = { await MapOffline.prepare() },
        clock: @escaping @Sendable () -> Int64 = { Int64(Date().timeIntervalSince1970) }
    ) {
        self.store = store
        self.prepare = prepare
        self.clock = clock
    }

    /// Reads the store and words it for `settingsJSON`, with the packs `packs` (id and JSON).
    func load(settingsJSON: String, packs: [(id: String, json: String)]) async {
        await prepare()
        guard let usage = try? await store.usage() else { return }
        var held: [(id: String, json: String, bytes: Int64)] = []
        for pack in packs {
            let bytes = (try? await store.usage(of: pack.id))?.bytes ?? 0
            held.append((pack.id, pack.json, bytes))
        }
        let now = clock()
        let zone = TimeZone.current.identifier
        figures = await Task.detached(priority: .userInitiated) {
            StorageFigures.make(
                usage: usage, packs: held, settingsJSON: settingsJSON, now: now, timeZone: zone)
        }.value
    }

    /// Drops every unpinned row (the map cache), then reads the store again.
    func clearCache(settingsJSON: String, packs: [(id: String, json: String)]) async {
        clearing = true
        defer { clearing = false }
        await prepare()
        _ = try? await store.removeUnpinned()
        await load(settingsJSON: settingsJSON, packs: packs)
    }

    /// Bounds the store to `megabytes` at once (the settings were changed to it), then reads it
    /// again.
    func bound(to megabytes: Int, settingsJSON: String, packs: [(id: String, json: String)]) async
    {
        await prepare()
        try? await store.setLimit(bytes: OfflineRules.limitBytes(megabytes: megabytes))
        await load(settingsJSON: settingsJSON, packs: packs)
    }
}
