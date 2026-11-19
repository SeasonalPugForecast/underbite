import Foundation
import XCTest

@testable import UnderbiteKit

/// The rules of a pack download apart from the network: Wi-Fi gating, back-off, slots per
/// host, the store's rows and pins, and the packs file.
final class PackRulesTests: XCTestCase {
    func testWifiOnlySendsOnlyOnAPathThatIsNeitherExpensiveNorConstrained() {
        XCTAssertEqual(PackRoute.of(satisfied: false, expensive: false, constrained: false), .none)
        XCTAssertEqual(PackRoute.of(satisfied: true, expensive: true, constrained: false), .metered)
        XCTAssertEqual(PackRoute.of(satisfied: true, expensive: false, constrained: true), .metered)
        XCTAssertEqual(
            PackRoute.of(satisfied: true, expensive: false, constrained: false), .unmetered)

        XCTAssertTrue(PackRules.maySend(on: .unmetered, wifiOnly: true))
        XCTAssertFalse(PackRules.maySend(on: .metered, wifiOnly: true))
        XCTAssertFalse(PackRules.maySend(on: .none, wifiOnly: true))
        XCTAssertTrue(PackRules.maySend(on: .metered, wifiOnly: false))
        XCTAssertFalse(PackRules.maySend(on: .none, wifiOnly: false))
    }

    func testBackOffDoublesUpToAMinute() {
        XCTAssertEqual((1...7).map(PackRules.backOff(after:)), [2, 4, 8, 16, 32, 60, 60])
        XCTAssertEqual(PackRules.backOff(after: 0), 2)
        XCTAssertEqual(PackRules.serverWait(0), 1)
        XCTAssertEqual(PackRules.serverWait(30), 30)
        XCTAssertEqual(PackRules.serverWait(3_600), 60)
    }

    func testSlotsHoldFourPerHostAndEightInAll() {
        var slots = PackSlots()
        for _ in 0..<4 { XCTAssertTrue(slots.take("a")) }
        XCTAssertFalse(slots.take("a"))
        XCTAssertEqual(slots.firstWithRoom(["a", "a", "b"]), 2)
        for _ in 0..<4 { XCTAssertTrue(slots.take("b")) }
        XCTAssertFalse(slots.take("c"), "eight in all")
        slots.release("a")
        XCTAssertTrue(slots.take("c"))
        XCTAssertFalse(slots.hasRoom(for: "a"))
        slots.release("zzz")
        XCTAssertEqual(slots.running, 8)
    }

    func testItemsMapToTheStoresRowsAndRefreshesPinApart() throws {
        let tile = PackRequest(
            key: "noaa-chart/latest/12/1205/1539", kind: .tile, layer: "noaa-chart",
            host: "x", request: nil, error: nil)
        XCTAssertEqual(
            PackRules.storeKey(tile),
            .tile(try XCTUnwrap(OfflineTileKey(layer: "noaa-chart", date: nil, z: 12, x: 1205, y: 1539))))
        let grid = PackRequest(
            key: "sst/jplMURSST41/last/40/-74/1", kind: .grid, layer: "sst", host: "pfeg",
            request: nil, error: nil)
        XCTAssertEqual(PackRules.storeKey(grid), .blob("sst/jplMURSST41/last/40/-74/1"))
        XCTAssertEqual(PackRules.pin("p1", staging: false), "p1")
        XCTAssertNotEqual(PackRules.pin("p1", staging: true), "p1")
    }

    func testTheCoresPackIsReadNotWritten() throws {
        let settings = try packSettings()
        let json = try batteryChartPack(settings: settings)
        let info = try XCTUnwrap(PackInfo(json: json))
        XCTAssertEqual(info.id, "battery-chart")
        XCTAssertEqual(info.name, "The Battery")
        XCTAssertEqual(info.minZoom, 12)
        XCTAssertEqual(info.maxZoom, 13)
        XCTAssertEqual(info.layers, ["noaa-chart"])
        XCTAssertEqual(info.spots, ["seed-battery"])
        XCTAssertTrue(info.isLatest)
        XCTAssertNil(info.refreshedAt)
        let refreshed = try XCTUnwrap(PackInfo(json: try packMarkRefreshed(packJson: json, now: packNow + 5)))
        XCTAssertEqual(refreshed.refreshedAt, packNow + 5)
        let days = try packNew(
            id: "d", name: "Days", area: info.area, minZoom: 12, maxZoom: 13,
            layers: ["noaa-chart"], dates: .lastDays(days: 3), spotIds: [],
            settingsJson: settings, now: packNow)
        XCTAssertEqual(PackInfo(json: days)?.isLatest, false)
        XCTAssertNil(PackInfo(json: "{}"))
    }

    func testThePacksFileStartsWifiOnlyAndKeepsItsPacks() throws {
        let directory = FileManager.default.temporaryDirectory.appending(
            path: "underbite-packs-\(UUID().uuidString)", directoryHint: .isDirectory)
        defer { try? FileManager.default.removeItem(at: directory) }
        let files = AppFiles(directory: directory)
        var file = PacksFile.load(files)
        XCTAssertTrue(file.wifiOnly)
        XCTAssertTrue(file.packs.isEmpty)
        file.packs.append(PackRecord(json: try batteryChartPack(settings: try packSettings())))
        var progress = PackProgress(startedAt: 1, plannedAt: 1)
        progress.layerBytes = ["noaa-chart": 9_000]
        progress.layerItems = ["noaa-chart": 3]
        file.measure(progress)
        XCTAssertEqual(file.measured, ["noaa-chart": 3_000])
        file.wifiOnly = false
        XCTAssertTrue(file.save(files))
        XCTAssertEqual(PacksFile.load(files), file)
        // An unreadable file is set aside, never written over.
        files.write(Data("{".utf8), PacksFile.name)
        XCTAssertEqual(PacksFile.load(files), PacksFile())
        XCTAssertTrue(files.exists(PacksFile.name + ".bad"))
    }

    func testADraftStartsWithTheChartTheNewestDataAndTheSpotsInside() throws {
        let settings = try packSettings()
        let words = try packWords(settingsJson: settings)
        let spots = [
            (id: "battery", latitude: 40.7033, longitude: -74.017),
            (id: "croton", latitude: 41.2, longitude: -73.85),
        ]
        let draft = PackDraft(
            prefill: PackPrefill(), defaultZoom: Int(words.defaultZoomPreset), spots: spots,
            around: (40.7033, -74.017))
        XCTAssertEqual(draft.layers, [PackDraft.chart])
        XCTAssertEqual(draft.dates, .latest)
        XCTAssertEqual(draft.spotIDs, ["battery"])
        let area = try XCTUnwrap(draft.area)
        XCTAssertTrue(area.contains(latitude: 40.7033, longitude: -74.017))

        // Unnamed: planned for the estimate under a stand-in, refused for keeping in the
        // core's words.
        guard case .success(let json) = draft.json(
            id: "draft", presets: words.zoomPresets, settingsJSON: settings, now: packNow,
            forEstimate: true)
        else { return XCTFail("the estimate's draft is made") }
        XCTAssertNotNil(PackInfo(json: json))
        guard case .failure(let problem) = draft.json(
            id: "p", presets: words.zoomPresets, settingsJSON: settings, now: packNow)
        else { return XCTFail("an unnamed pack is not kept") }
        XCTAssertFalse(problem.message.isEmpty)

        // A trip's area, days and spots are kept as given.
        let trip = PackDraft(
            prefill: PackPrefill(
                name: "Trip", area: PackBox(west: -74.1, south: 40.6, east: -73.9, north: 40.8),
                dates: .days(dates: ["2026-12-01"]), spotIDs: ["croton"]),
            defaultZoom: 0, spots: spots)
        XCTAssertEqual(trip.spotIDs, ["croton"])
        XCTAssertEqual(trip.dates, .days(dates: ["2026-12-01"]))
        XCTAssertEqual(trip.area?.west, -74.1)
    }

    func testABoxAcrossTheAntimeridianHoldsBothSides() {
        let box = PackBox(west: 170, south: -10, east: -170, north: 10)
        XCTAssertTrue(box.contains(latitude: 0, longitude: 175))
        XCTAssertTrue(box.contains(latitude: 0, longitude: -175))
        XCTAssertFalse(box.contains(latitude: 0, longitude: 0))
    }
}
