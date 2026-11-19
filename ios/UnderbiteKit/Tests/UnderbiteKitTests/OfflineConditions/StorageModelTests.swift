import Foundation
import XCTest

@testable import UnderbiteKit

/// The storage page: its figures as the core words them, the limit's choices, clearing the map
/// cache and a new limit, on a scratch store.
@MainActor
final class StorageModelTests: XCTestCase {
    private let scratch = ScratchStore()
    /// 2026-11-18T12:00:00Z.
    private let now: Int64 = 1_795_003_200
    nonisolated private static let harcon =
        "https://api.tidesandcurrents.noaa.gov/mdapi/prod/webapi/stations/8518750/harcon.json?units=metric"

    private func settings(limit: Int) throws -> String {
        var settings = try Settings.defaults()
        settings.map.offlineMaxMb = limit
        return try settings.json()
    }

    private func pack(_ id: String, name: String) throws -> String {
        try packNew(
            id: id, name: name, area: PackBox(west: -74.03, south: 40.695, east: -74.0, north: 40.715),
            minZoom: 12, maxZoom: 13, layers: ["noaa-chart"], dates: .latest, spotIds: [],
            settingsJson: defaultSettingsJson(), now: now)
    }

    private func blob(_ bytes: Int) -> OfflineEntry {
        OfflineEntry(
            data: Data(repeating: 0x7B, count: bytes), contentType: nil, fetchedAt: now,
            lifetime: 3_600)
    }

    /// A size as the core writes it.
    private func size(_ bytes: UInt64) -> String {
        storageSizeText(bytes: bytes, settingsJson: defaultSettingsJson())
    }

    private func model() -> StorageModel {
        StorageModel(store: scratch.store, prepare: {}, clock: { [now] in now })
    }

    /// A pack holding 2 MB, the map cache 3 MB and a spot's metadata 1 KB.
    private func fill() async throws -> [(id: String, json: String)] {
        let store = scratch.store
        let json = try pack("p1", name: "Harbour")
        try await store.pin("p1", [.blob("grid/a")])
        try await store.put(.blob("grid/a"), blob(2_000_000), now: now)
        try await store.put(.blob("tile/cache"), blob(3_000_000), now: now)
        _ = await SpotMetadata(store: store).keep(
            [CachedResponse(key: Self.harcon, body: Data(repeating: 0x7B, count: 1_000), fetchedAt: now)],
            spot: "s", kept: [], now: now)
        return [("p1", json)]
    }

    func testTheFiguresAreTheCoresWords() async throws {
        let packs = try await fill()
        let m = model()
        await m.load(settingsJSON: try settings(limit: 1_500), packs: packs)
        let figures = try XCTUnwrap(m.figures)
        // The packs' share is the packs' own rows: the spot's kept metadata is in the total only.
        XCTAssertEqual(figures.words.totalText, "\(size(5_001_000)) of 1.5 GB")
        XCTAssertEqual(figures.words.packsText, "Packs: \(size(2_000_000))")
        XCTAssertEqual(figures.words.cacheText, "Map cache: \(size(3_000_000))")
        XCTAssertEqual(figures.words.limitText, "Limit: 1.5 GB")
        XCTAssertEqual(figures.fill ?? 0, 5_001_000.0 / 1_500_000_000, accuracy: 1e-9)
        XCTAssertEqual(figures.packs.map(\.name), ["Harbour"])
        XCTAssertEqual(figures.packs.first?.sizeText, size(2_000_000))
        XCTAssertEqual(figures.limitMegabytes, 1_500)
    }

    func testAPhoneWithNoPacksReadsPacksAsZero() async throws {
        try await scratch.store.put(.blob("tile/cache"), blob(3_000_000), now: now)
        _ = await SpotMetadata(store: scratch.store).keep(
            [CachedResponse(key: Self.harcon, body: Data(repeating: 0x7B, count: 1_000), fetchedAt: now)],
            spot: "s", kept: [], now: now)
        let m = model()
        await m.load(settingsJSON: try settings(limit: 1_500), packs: [])
        XCTAssertEqual(m.figures?.words.packsText, "Packs: \(size(0))")
        XCTAssertEqual(m.figures?.words.cacheText, "Map cache: \(size(3_000_000))")
        XCTAssertEqual(m.figures?.words.totalText, "\(size(3_001_000)) of 1.5 GB")
    }

    func testTheLimitsOfferNoCacheTheCoresStopsAndTheUsersOwn() throws {
        let choices = StorageFigures.limitChoices(settingsJSON: try settings(limit: 750))
        XCTAssertEqual(choices.map(\.megabytes), [0, 250, 500, 750, 1_000, 1_500, 2_000, 5_000])
        XCTAssertEqual(choices.first?.label, "No map cache: only packs are kept")
        XCTAssertEqual(choices.last?.label, size(5_000_000_000))
    }

    func testClearingTheCacheKeepsPacksAndSpotsMetadata() async throws {
        let packs = try await fill()
        let m = model()
        let json = try settings(limit: 1_500)
        await m.clearCache(settingsJSON: json, packs: packs)
        XCTAssertEqual(m.figures?.words.cacheText, "Map cache: 0 KB")
        XCTAssertEqual(m.figures?.words.totalText, "\(size(2_001_000)) of 1.5 GB")
        let pack = try await scratch.store.usage(of: "p1")
        XCTAssertEqual(pack.stored, 1)
        let kept = await SpotMetadata(store: scratch.store).cached([Self.harcon], now: now)
        XCTAssertEqual(kept.count, 1)
    }

    func testANewLimitBoundsTheStoreAtOnce() async throws {
        let packs = try await fill()
        let m = model()
        await m.bound(to: 4, settingsJSON: try settings(limit: 4), packs: packs)
        // 5 MB under a 4 MB limit: the unpinned map cache goes; packs and metadata stay.
        let usage = try await scratch.store.usage()
        XCTAssertEqual(usage.bytes, 2_001_000)
        XCTAssertEqual(m.figures?.words.limitText, "Limit: \(size(4_000_000))")
        XCTAssertEqual(m.figures?.limitMegabytes, 4)
    }
}
