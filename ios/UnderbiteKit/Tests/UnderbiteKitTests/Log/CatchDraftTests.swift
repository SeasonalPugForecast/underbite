import Foundation
import XCTest

@testable import UnderbiteKit

/// The catch editor's draft: its fields as the core's input, the place and zone of a catch,
/// when a catch takes conditions, the snapshot they come from, the fate picker, the conditions
/// shown before saving, and the species to choose from.
@MainActor
final class CatchDraftTests: XCTestCase {
    private let directory = FileManager.default.temporaryDirectory
        .appending(path: "underbite-draft-\(UUID().uuidString)", directoryHint: .isDirectory)

    override func tearDown() {
        try? FileManager.default.removeItem(at: directory)
    }

    private static let spots = [SeedSpots.battery, SeedSpots.croton]
    private static let phone = TimeZone(identifier: "America/Chicago")!
    /// The striped bass of the seeds: 2026-11-08 06:10 at the Battery.
    private static let bass = CatchSeeds.seeds[1]
    private nonisolated static let now: Int64 = 1_800_000_000

    private func store() -> CatchStore {
        CatchStore(files: AppFiles(directory: directory), clock: { Self.now })
    }

    private static func date(_ seconds: Int64) -> Date {
        Date(timeIntervalSince1970: TimeInterval(seconds))
    }

    // MARK: - A new catch

    func testANewCatchAtASpotTakesTheSpotsPlaceAndZone() throws {
        var draft = CatchDraft(now: Self.date(Self.bass.caughtAt), spot: SeedSpots.battery)
        draft.species = "morone-saxatilis"
        draft.speciesText = "ignored while a catalogue species is chosen"
        draft.lengthText = "28"
        draft.lure = "  Bucktail jig "
        draft.notes = "   "
        draft.area = "ignored at a spot"
        let input = try XCTUnwrap(draft.input(spots: Self.spots, phone: Self.phone))
        XCTAssertEqual(input.id, "", "the store gives a new catch its id")
        XCTAssertEqual(input.caughtAt, Self.bass.caughtAt)
        XCTAssertEqual(input.spotId, SeedSpots.battery.id)
        XCTAssertEqual(input.timeZone, "America/New_York")
        XCTAssertEqual(input.latitude, SeedSpots.battery.latitude)
        XCTAssertEqual(input.longitude, SeedSpots.battery.longitude)
        XCTAssertNil(input.area)
        XCTAssertNil(input.speciesText)
        XCTAssertEqual(input.lure, "Bucktail jig")
        XCTAssertNil(input.notes, "a blank field is no value")
        XCTAssertEqual(input.lengthText, "28", "sizes go to the core as typed")
        XCTAssertTrue(draft.isNew)
        XCTAssertTrue(draft.needsConditions(spots: Self.spots))
    }

    func testANewCatchHereWaitsForTheFixThenTakesThePhonesZoneAndTheArea() throws {
        var draft = CatchDraft(now: Self.date(Self.bass.caughtAt), spot: nil)
        XCTAssertEqual(draft.place, .here)
        XCTAssertNil(draft.input(spots: Self.spots), "no place until the phone's position is known")
        draft.here = .init(latitude: 41.0, longitude: -73.9)
        draft.area = " Tappan Zee "
        draft.speciesText = "Atlantic tomcod"
        let input = try XCTUnwrap(draft.input(spots: Self.spots, phone: Self.phone))
        XCTAssertNil(input.spotId)
        XCTAssertEqual(input.latitude, 41.0)
        XCTAssertEqual(input.timeZone, "America/Chicago")
        XCTAssertEqual(input.area, "Tappan Zee")
        XCTAssertNil(input.species)
        XCTAssertEqual(input.speciesText, "Atlantic tomcod")
    }

    func testTheCatchTimeIsWholeSeconds() {
        let draft = CatchDraft(now: Date(timeIntervalSince1970: 1_794_136_200.75), spot: nil)
        XCTAssertEqual(draft.caughtAtSeconds, 1_794_136_200)
    }

    // MARK: - An edit

    private func savedBass(_ s: CatchStore) throws -> CatchInput {
        let units = try fishUnitsInfo(settingsJson: "{}")
        try s.save(CatchSeeds.input(Self.bass, units: units), settingsJSON: "{}")
        return try logEditor(book: s.book, id: Self.bass.id, settingsJson: "{}")
    }

    func testAnEditGivesBackTheCatchAsSavedAndKeepsItsConditionsUntilTimeOrPlaceChange() throws {
        let saved = try savedBass(store())
        var draft = CatchDraft(editing: saved, spots: Self.spots)
        XCTAssertFalse(draft.isNew)
        XCTAssertEqual(draft.place, .spot(SeedSpots.battery.id))
        XCTAssertEqual(draft.input(spots: Self.spots), saved, "an untouched edit saves the same")
        XCTAssertFalse(draft.needsConditions(spots: Self.spots))

        draft.notes = "Moved to the notes"
        draft.photos = ["a.jpg"]
        XCTAssertFalse(draft.needsConditions(spots: Self.spots), "notes keep the conditions")

        var later = draft
        later.caughtAt = Self.date(Self.bass.caughtAt + 3_600)
        XCTAssertTrue(later.needsConditions(spots: Self.spots))

        var moved = draft
        moved.place = .spot(SeedSpots.croton.id)
        XCTAssertTrue(moved.needsConditions(spots: Self.spots))
        XCTAssertEqual(moved.input(spots: Self.spots)?.timeZone, "America/New_York")
    }

    func testACatchAtASpotNoLongerSavedKeepsItsPositionAndZone() throws {
        let saved = try savedBass(store())
        let draft = CatchDraft(editing: saved, spots: [SeedSpots.croton])
        XCTAssertEqual(draft.place, .stored)
        let input = try XCTUnwrap(draft.input(spots: [SeedSpots.croton], phone: Self.phone))
        XCTAssertNil(input.spotId)
        XCTAssertEqual(input.latitude, saved.latitude)
        XCTAssertEqual(input.longitude, saved.longitude)
        XCTAssertEqual(input.timeZone, "America/New_York", "the zone it was saved with")
        XCTAssertFalse(draft.needsConditions(spots: [SeedSpots.croton]))
    }

    // MARK: - The snapshot

    func testTheSnapshotIsTheSpotsOrThatOfASavedSpotCloseBy() {
        var draft = CatchDraft(now: Date(), spot: SeedSpots.croton)
        XCTAssertEqual(draft.snapshotSpot(spots: Self.spots)?.id, SeedSpots.croton.id)

        draft.place = .here
        XCTAssertNil(draft.snapshotSpot(spots: Self.spots), "no position yet")
        // About 400 m from the Battery.
        draft.here = .init(latitude: 40.7003, longitude: -74.0142)
        XCTAssertEqual(draft.snapshotSpot(spots: Self.spots)?.id, SeedSpots.battery.id)
        // Midtown, 6 km away: no spot close enough.
        draft.here = .init(latitude: 40.7549, longitude: -73.9840)
        XCTAssertNil(draft.snapshotSpot(spots: Self.spots))
    }

    func testDistancesAreGreatCircleMetres() {
        let battery = CatchDraft.Position(latitude: 40.7033, longitude: -74.0170)
        let croton = CatchDraft.Position(
            latitude: SeedSpots.croton.latitude, longitude: SeedSpots.croton.longitude)
        XCTAssertEqual(CatchDraft.distance(battery, battery), 0, accuracy: 0.001)
        XCTAssertEqual(CatchDraft.distance(battery, croton), 59_513, accuracy: 10)
    }

    // MARK: - The fate

    func testACatchKeptForSashimiShowsAsKeptWhereTheVoiceDoesNotOfferItAndStaysSo() {
        var draft = CatchDraft(now: Date(), spot: nil)
        draft.fate = .keptForSashimi
        XCTAssertEqual(draft.shownFate([.released, .kept]), .kept)
        XCTAssertEqual(draft.shownFate([.released, .kept, .keptForSashimi]), .keptForSashimi)
        draft.pick(.kept)
        XCTAssertEqual(draft.fate, .keptForSashimi, "choosing what it shows keeps it")
        draft.pick(.released)
        XCTAssertEqual(draft.fate, .released)
        draft.pick(.kept)
        XCTAssertEqual(draft.fate, .kept)
    }

    func testTheFatePickerOffersTheCoresWords() throws {
        let plain = try fateOptions(settingsJson: #"{"presentation":{"voice":"plain"}}"#)
        XCTAssertEqual(plain.map(\.id), [.released, .kept])
        let luna = try fateOptions(settingsJson: #"{"presentation":{"voice":"luna"}}"#)
        XCTAssertEqual(luna.map(\.id), [.released, .kept, .keptForSashimi])
    }

    // MARK: - The conditions before saving

    func testANewCatchShowsTheSnapshotsConditionsWithTheirSourcesAndTheLogStaysAsItWas() throws {
        let s = store()
        let before = s.book
        var draft = CatchDraft(now: Self.date(Self.bass.caughtAt), spot: SeedSpots.battery)
        draft.lengthText = "not a length"
        let input = try XCTUnwrap(draft.input(spots: Self.spots))
        let preview = try XCTUnwrap(
            CatchConditionsPreview.build(
                book: s.book, input: input, snapshotJSON: CatchSeeds.snapshot(Self.bass),
                attach: true, settingsJSON: "{}", now: Self.now))
        XCTAssertFalse(preview.title.isEmpty)
        XCTAssertNil(preview.none)
        let kinds = Set(preview.conditions.map(\.kind))
        XCTAssertTrue(kinds.isSuperset(of: ["tide", "water_temperature", "wind"]), "\(kinds)")
        // The source and station are stated once, on the first reading of a run.
        XCTAssertTrue(
            preview.conditions.first?.footnote.contains("The Battery") == true,
            "\(preview.conditions.map(\.footnote))")
        XCTAssertTrue(preview.conditions.allSatisfy { !$0.footnote.contains("Croton") })
        XCTAssertEqual(s.book, before, "the conditions are worked out on a copy")
        XCTAssertEqual(s.count, 0)
    }

    func testACatchFarFromTheSnapshotsTimeHasNoConditionsAndTheCoreSaysSo() throws {
        let s = store()
        // Two days after the snapshot.
        let draft = CatchDraft(now: Self.date(Self.bass.caughtAt + 172_800), spot: SeedSpots.battery)
        let input = try XCTUnwrap(draft.input(spots: Self.spots))
        let preview = try XCTUnwrap(
            CatchConditionsPreview.build(
                book: s.book, input: input, snapshotJSON: CatchSeeds.snapshot(Self.bass),
                attach: true, settingsJSON: "{}", now: Self.now))
        XCTAssertEqual(preview.conditions, [])
        XCTAssertNotNil(preview.none)

        let none = try XCTUnwrap(
            CatchConditionsPreview.build(
                book: s.book, input: input, snapshotJSON: nil, attach: true, settingsJSON: "{}",
                now: Self.now))
        XCTAssertEqual(none.conditions, [])
        XCTAssertNotNil(none.none, "no snapshot at all")

        let unreadable = try XCTUnwrap(
            CatchConditionsPreview.build(
                book: s.book, input: input, snapshotJSON: "{}", attach: true, settingsJSON: "{}",
                now: Self.now))
        XCTAssertNotNil(unreadable.none, "a snapshot the core cannot read gives none")
    }

    func testAnEditThatKeepsItsTimeShowsTheConditionsItWasSavedWith() throws {
        let s = store()
        CatchSeeds.seed(catches: s, settingsJSON: "{}")
        let saved = try logEditor(book: s.book, id: Self.bass.id, settingsJson: "{}")
        var draft = CatchDraft(editing: saved, spots: Self.spots)
        draft.notes = "Edited"
        let input = try XCTUnwrap(draft.input(spots: Self.spots))
        let preview = try XCTUnwrap(
            CatchConditionsPreview.build(
                book: s.book, input: input, snapshotJSON: nil,
                attach: draft.needsConditions(spots: Self.spots), settingsJSON: "{}",
                now: Self.now))
        let detail = try XCTUnwrap(
            s.detail(Self.bass.id, settingsJSON: "{}", places: []))
        XCTAssertEqual(preview.conditions, detail.conditions)
        XCTAssertFalse(preview.conditions.isEmpty)
    }

    func testSavingADraftThenAttachingGivesWhatThePreviewShowed() throws {
        let s = store()
        let draft = CatchDraft(now: Self.date(Self.bass.caughtAt), spot: SeedSpots.battery)
        let input = try XCTUnwrap(draft.input(spots: Self.spots))
        let snapshot = CatchSeeds.snapshot(Self.bass)
        let preview = try XCTUnwrap(
            CatchConditionsPreview.build(
                book: s.book, input: input, snapshotJSON: snapshot, attach: true,
                settingsJSON: "{}", now: Self.now))
        let id = try s.save(input, settingsJSON: "{}")
        XCTAssertTrue(try s.attachConditions(to: id, snapshotJSON: snapshot))
        let detail = try XCTUnwrap(s.detail(id, settingsJSON: "{}", places: []))
        XCTAssertEqual(detail.conditions, preview.conditions)
    }

    // MARK: - Species

    func testTheSpeciesComeFromTheCatalogueAndAreFoundByEitherName() throws {
        let all = SpeciesChoice.catalogue(settingsJSON: "{}")
        let bass = try XCTUnwrap(all.first { $0.id == "morone-saxatilis" })
        XCTAssertFalse(bass.name.isEmpty)
        XCTAssertEqual(bass.scientificName, "Morone saxatilis")
        XCTAssertEqual(SpeciesChoice.matching("", in: all), all)
        XCTAssertTrue(SpeciesChoice.matching("SAXATILIS", in: all).contains(bass))
        XCTAssertTrue(SpeciesChoice.matching(" \(bass.name.prefix(5)) ", in: all).contains(bass))
        XCTAssertEqual(SpeciesChoice.matching("no such fish anywhere", in: all), [])
        XCTAssertEqual(SpeciesChoice.catalogue(settingsJSON: "not json"), [])
    }
}
