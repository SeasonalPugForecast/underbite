import Foundation
import XCTest

@testable import UnderbiteKit

/// The trip checklist: ticks, items and regions written onto the trip's JSON and saved through
/// the store, the official pages the core links once a place has a region, the trip's pack as
/// the core words it, a new pack started from the trip, and the trip-to-pack links file.
@MainActor
final class TripChecklistTests: XCTestCase {
    /// A directory of its own for each test (XCTest makes an instance per test).
    private let directory = FileManager.default.temporaryDirectory
        .appending(path: "underbite-trip-checklist-\(UUID().uuidString)", directoryHint: .isDirectory)

    override func tearDown() {
        try? FileManager.default.removeItem(at: directory)
    }

    private var files: AppFiles { AppFiles(directory: directory) }

    /// 2026-11-15T12:00:00Z.
    nonisolated private static let now: Int64 = 1_794_744_000
    nonisolated private static let spots = [SeedSpots.battery, SeedSpots.croton]

    /// Three days at the test places, kept by a store.
    private func kept() throws -> (TripStore, String) {
        var draft = TripDraft(first: "2026-11-20", days: 3, places: Self.spots.map(\.id))
        draft.name = "Hudson stripers"
        draft.method = "boat"
        let trips = TripStore(files: files, clock: { Self.now })
        let id = try trips.save(draft.json(spots: Self.spots, areas: [], settingsJSON: "{}"))
        return (trips, id)
    }

    private func json(_ trips: TripStore, _ id: String) throws -> String {
        try XCTUnwrap(trips.trip(id)).json
    }

    private func view(
        _ json: String,
        app: TripAppStatus = TripAppStatus(pack: .notDownloaded, sharing: .notShared)
    ) throws -> ViewModel.TripView {
        try ViewModel.buildTrip(
            tripJSON: json, places: [], cachedMetadata: [], app: app,
            settingsJSON: defaultSettingsJson(),
            now: Date(timeIntervalSince1970: TimeInterval(Self.now)))
    }

    private func rows(_ view: ViewModel.TripView, _ kind: String) -> [ViewModel.ChecklistRow] {
        view.checklist.filter { $0.kind == kind }
    }

    // MARK: - Items

    func testATickIsSavedToTheTripAndTheCoreShowsIt() throws {
        let (trips, id) = try kept()
        let added = try XCTUnwrap(
            TripChecklistEdit.adding(try json(trips, id), item: "  Buy bunker ", id: "bunker"))
        try trips.save(added)
        var item = try XCTUnwrap(rows(try view(try json(trips, id)), "item").first)
        XCTAssertEqual(item.id, "bunker")
        XCTAssertEqual(item.text, "Buy bunker", "trimmed")
        XCTAssertEqual(item.done, false)

        let ticked = try XCTUnwrap(
            TripChecklistEdit.ticking(try json(trips, id), item: "bunker", done: true))
        try trips.save(ticked)
        item = try XCTUnwrap(rows(try view(try json(trips, id)), "item").first)
        XCTAssertEqual(item.done, true)

        // What was saved is the trip the core reads, back from the file.
        let again = TripStore(files: files, clock: { Self.now })
        let reread = try XCTUnwrap(rows(try view(try json(again, id)), "item").first)
        XCTAssertEqual(reread.done, true)
        XCTAssertEqual(try view(try json(again, id)).title, "Hudson stripers", "the rest is kept")
        XCTAssertNil(TripChecklistEdit.ticking(try json(trips, id), item: "nothing", done: true))
    }

    func testItemsAreAddedRenamedAndRemoved() throws {
        let (trips, id) = try kept()
        var trip = try json(trips, id)
        trip = try XCTUnwrap(TripChecklistEdit.adding(trip, item: "Ice", id: "a"))
        trip = try XCTUnwrap(TripChecklistEdit.adding(trip, item: "Bait", id: "b"))
        XCTAssertNil(TripChecklistEdit.adding(trip, item: "Again", id: "a"), "ids are unique")
        XCTAssertNil(TripChecklistEdit.adding(trip, item: "   ", id: "c"), "no blank items")
        trip = try XCTUnwrap(TripChecklistEdit.renaming(trip, item: "a", to: "Ice, two bags"))
        XCTAssertNil(TripChecklistEdit.renaming(trip, item: "a", to: " "))
        try trips.save(trip)
        XCTAssertEqual(
            rows(try view(try json(trips, id)), "item").map(\.text), ["Ice, two bags", "Bait"])

        trip = try XCTUnwrap(TripChecklistEdit.removing(try json(trips, id), item: "a"))
        XCTAssertNil(TripChecklistEdit.removing(trip, item: "a"))
        try trips.save(trip)
        XCTAssertEqual(rows(try view(try json(trips, id)), "item").map(\.id), ["b"])
    }

    func testTheMethodsGearAndTheSharingRowComeFromTheCore() throws {
        let (trips, id) = try kept()
        let view = try view(try json(trips, id))
        let gear = rows(view, "gear")
        XCTAssertFalse(gear.isEmpty, "a boat's gear")
        XCTAssertTrue(gear.allSatisfy { $0.done == nil && !$0.text.isEmpty })
        XCTAssertEqual(rows(view, "sharing").count, 1)
        XCTAssertEqual(view.checklist.last?.kind, "sharing")
    }

    // MARK: - Regions and the official pages

    func testARegionBringsTheOfficialPagesWithTheDisclaimer() throws {
        let (trips, id) = try kept()
        let before = try view(try json(trips, id))
        XCTAssertTrue(rows(before, "licence").isEmpty, "no region, no pages")
        XCTAssertTrue(rows(before, "disclaimer").isEmpty)
        XCTAssertEqual(
            TripChecklistEdit.places(try json(trips, id)).map(\.region), ["", ""])

        var trip = try json(trips, id)
        for place in TripChecklistEdit.places(trip) {
            trip = try XCTUnwrap(TripChecklistEdit.settingRegion(trip, place: place.id, to: " us-ny "))
        }
        XCTAssertNil(
            TripChecklistEdit.settingRegion(trip, place: SeedSpots.battery.id, to: "US-NY"),
            "nothing changes")
        XCTAssertNil(TripChecklistEdit.settingRegion(trip, place: "elsewhere", to: "US-NY"))
        try trips.save(trip)
        XCTAssertEqual(TripChecklistEdit.places(try json(trips, id)).map(\.region), ["US-NY", "US-NY"])

        let view = try view(try json(trips, id))
        let kinds = view.checklist.map(\.kind)
        let licence = try XCTUnwrap(rows(view, "licence").first)
        XCTAssertEqual(rows(view, "licence").count, 1, "one licence page for both places")
        XCTAssertFalse(rows(view, "regulations").isEmpty)
        for row in rows(view, "licence") + rows(view, "regulations") {
            XCTAssertTrue(row.link?.hasPrefix("https://") ?? false, row.link ?? "")
            XCTAssertNotNil(row.detail, "who publishes it and when the link was checked")
        }
        XCTAssertTrue(licence.link?.hasPrefix("https://dec.ny.gov/") ?? false, "New York's own")
        XCTAssertTrue(
            rows(view, "regulations").contains { $0.link?.hasPrefix("https://dec.ny.gov/") ?? false })
        XCTAssertFalse(licence.text.isEmpty)
        let disclaimer = try XCTUnwrap(kinds.firstIndex(of: "disclaimer"))
        XCTAssertTrue(
            kinds[..<disclaimer].allSatisfy { $0 == "licence" || $0 == "regulations" },
            "the disclaimer follows the links: \(kinds)")
        XCTAssertFalse(view.checklist[disclaimer].text.isEmpty)

        let cleared = try XCTUnwrap(
            TripChecklistEdit.settingRegion(
                try json(trips, id), place: SeedSpots.croton.id, to: "  "))
        XCTAssertEqual(TripChecklistEdit.places(cleared).map(\.region), ["US-NY", ""])
    }

    // MARK: - The pack

    func testThePackRowFollowsTheTripsPack() throws {
        let (trips, id) = try kept()
        let settings = defaultSettingsJson()
        let pack = PackRecord(json: try batteryChartPack(settings: settings))

        XCTAssertEqual(
            TripPackLink.appStatus(nil, storedBytes: nil).pack, .notDownloaded)
        XCTAssertEqual(
            TripPackLink.appStatus(pack, storedBytes: 0).pack, .notDownloaded,
            "a pack never downloaded")
        let none = try XCTUnwrap(rows(try view(try json(trips, id)), "offline_pack").first)
        XCTAssertEqual(none.done, false)

        var downloaded = pack
        downloaded.json = try packMarkRefreshed(packJson: pack.json, now: Self.now - 3_600)
        downloaded.status = .done
        let status = TripPackLink.appStatus(downloaded, storedBytes: 2_500_000)
        XCTAssertEqual(status.pack, .ready(bytes: 2_500_000, savedAt: Self.now - 3_600))
        XCTAssertEqual(
            TripPackLink.appStatus(downloaded, storedBytes: -1).pack,
            .ready(bytes: 0, savedAt: Self.now - 3_600))
        let ready = try XCTUnwrap(rows(try view(try json(trips, id), app: status), "offline_pack").first)
        XCTAssertEqual(ready.done, true)
        XCTAssertNotEqual(ready.text, none.text, "the core words its size and day")
    }

    func testANewPackStartsFromTheTripsPlacesDaysAndLayers() throws {
        let (trips, id) = try kept()
        let trip = try json(trips, id)
        let places = try tripSpots(tripJson: trip, now: Self.now)
        let days = try view(trip).days.map(\.date)
        XCTAssertEqual(days, ["2026-11-20", "2026-11-21", "2026-11-22"])
        let layers = TripPackLink.layers(try mapCatalogue(settingsJson: defaultSettingsJson()))
        XCTAssertEqual(layers.first, PackDraft.chart)
        XCTAssertEqual(Set(layers).count, layers.count)

        let prefill = TripPackLink.prefill(
            name: "Hudson stripers", places: places, saved: [SeedSpots.battery.id], days: days,
            layers: layers, maxDays: 31)
        XCTAssertEqual(prefill.name, "Hudson stripers")
        XCTAssertEqual(prefill.layers, layers)
        XCTAssertEqual(prefill.dates, .days(dates: days))
        XCTAssertEqual(prefill.spotIDs, [SeedSpots.battery.id], "only the saved spots")
        let area = try XCTUnwrap(prefill.area)
        let m = PackDraft.spotMargin
        XCTAssertEqual(area.west, SeedSpots.battery.longitude - m, accuracy: 1e-9)
        XCTAssertEqual(area.south, SeedSpots.battery.latitude - m, accuracy: 1e-9)
        XCTAssertEqual(area.east, SeedSpots.croton.longitude + m, accuracy: 1e-9)
        XCTAssertEqual(area.north, SeedSpots.croton.latitude + m, accuracy: 1e-9)
        for spot in Self.spots {
            XCTAssertTrue(area.contains(latitude: spot.latitude, longitude: spot.longitude))
        }

        // The draft the new-pack screen makes of it keeps all of it.
        let draft = PackDraft(prefill: prefill, defaultZoom: 1, spots: [])
        XCTAssertEqual(draft.area, area)
        XCTAssertEqual(draft.dates, .days(dates: days))
        XCTAssertEqual(draft.name, "Hudson stripers")

        let short = TripPackLink.prefill(
            name: "", places: places, saved: [], days: days, layers: layers, maxDays: 2)
        XCTAssertEqual(short.dates, .days(dates: Array(days.prefix(2))), "the first days")
        XCTAssertNil(short.spotIDs, "none saved: the screen picks the spots inside")

        let nowhere = TripPackLink.prefill(
            name: "", places: [TripSpot(key: "x", spot: nil, forecast: false)], saved: [],
            days: [], layers: layers, maxDays: 31)
        XCTAssertNil(nowhere.area, "no coordinates: the screen's own area")
        XCTAssertNil(nowhere.dates)
    }

    func testTheTripRemembersOnlyItsPacksID() throws {
        let settings = defaultSettingsJson()
        let a = PackRecord(json: try batteryChartPack(id: "pack-a", settings: settings))
        let b = PackRecord(json: try batteryChartPack(id: "pack-b", settings: settings))
        var links = TripPackLinks.load(files)
        XCTAssertEqual(links, TripPackLinks())
        links.packs = ["trip-1": "pack-a", "trip-2": "pack-gone", "trip-gone": "pack-b"]
        XCTAssertTrue(links.save(files))

        let again = TripPackLinks.load(files)
        XCTAssertEqual(again, links)
        XCTAssertEqual(again.pack(of: "trip-1", in: [a, b])?.id, "pack-a")
        XCTAssertNil(again.pack(of: "trip-2", in: [a, b]), "a deleted pack is no pack")
        XCTAssertNil(again.pack(of: "trip-3", in: [a, b]))
        XCTAssertEqual(
            again.pruned(trips: ["trip-1", "trip-2"], packs: [a, b]).packs, ["trip-1": "pack-a"])

        XCTAssertTrue(files.write(Data("not json".utf8), TripPackLinks.name))
        XCTAssertEqual(TripPackLinks.load(files), TripPackLinks())
        XCTAssertTrue(files.exists(TripPackLinks.name + ".bad"), "set aside, never written over")
    }
}
