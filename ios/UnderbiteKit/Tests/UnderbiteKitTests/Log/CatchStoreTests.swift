import Foundation
import XCTest

@testable import UnderbiteKit

/// The catch store: the log kept in `catches.json` as the core's text, ids and times set on save,
/// deleting as a tombstone with the photos removed, files it cannot read set aside, the list and
/// detail from the core's views, and the export's temporary file.
@MainActor
final class CatchStoreTests: XCTestCase {
    /// A directory of its own for each test (XCTest makes an instance per test).
    private let directory = FileManager.default.temporaryDirectory
        .appending(path: "underbite-catches-\(UUID().uuidString)", directoryHint: .isDirectory)

    override func tearDown() {
        try? FileManager.default.removeItem(at: directory)
    }

    private var files: AppFiles { AppFiles(directory: directory) }

    /// 2027-01-15 08:00 UTC.
    private static let now: Int64 = 1_800_000_000
    /// 2026-11-08 06:10 at the Battery.
    private static let caught: Int64 = 1_794_136_200

    private static let places = [
        PlaceName(spotId: SeedSpots.battery.id, name: SeedSpots.battery.name),
        PlaceName(spotId: SeedSpots.croton.id, name: SeedSpots.croton.name),
    ]

    private func store(now: Int64 = CatchStoreTests.now) -> CatchStore {
        CatchStore(files: files, clock: { now })
    }

    /// A striped bass at the Battery, 28 in (or 71 cm).
    private func input(id: String = "", length: String? = nil, photos: [String] = [])
        throws -> CatchInput
    {
        let units = try fishUnitsInfo(settingsJson: "{}")
        return CatchInput(
            id: id, caughtAt: Self.caught, timeZone: "America/New_York",
            latitude: SeedSpots.battery.latitude, longitude: SeedSpots.battery.longitude,
            spotId: SeedSpots.battery.id, area: nil, species: "morone-saxatilis",
            speciesText: nil,
            lengthText: length ?? (units.lengthUnit == "inches" ? "28" : "71"), weightText: "",
            ouncesText: "", fate: .released, lure: "Bucktail jig", method: nil, notes: "Ebb tide",
            photos: photos)
    }

    /// The records of the stored file, tombstones too.
    private func entries() throws -> [[String: Any]] {
        let data = try XCTUnwrap(files.data(CatchStore.fileName))
        let file = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        return try XCTUnwrap(file["entries"] as? [[String: Any]])
    }

    func testSavedCatchesReadBackInANewStore() throws {
        let s = store()
        let id = try s.save(input(), settingsJSON: "{}")
        XCTAssertNotNil(UUID(uuidString: id), "a new catch's id is a UUID")
        XCTAssertEqual(s.count, 1)
        XCTAssertEqual(s.revision, 1)

        let saved = try entries()
        XCTAssertEqual(saved.count, 1)
        XCTAssertEqual(saved[0]["id"] as? String, id)
        XCTAssertEqual((saved[0]["created_at"] as? NSNumber)?.int64Value, Self.now)
        XCTAssertEqual((saved[0]["modified_at"] as? NSNumber)?.int64Value, Self.now)
        XCTAssertEqual((saved[0]["caught_at"] as? NSNumber)?.int64Value, Self.caught)

        let again = store()
        XCTAssertEqual(again.book, s.book, "the core's text reads back the same")
        XCTAssertEqual(again.count, 1)
        XCTAssertFalse(again.unreadable)
    }

    func testAnEditKeepsTheIdAndCreationAndMovesTheModificationTime() throws {
        let id = try store().save(input(), settingsJSON: "{}")
        let later = store(now: Self.now + 600)
        XCTAssertEqual(try later.save(input(id: id, length: "30"), settingsJSON: "{}"), id)
        let saved = try entries()
        XCTAssertEqual(saved.count, 1)
        XCTAssertEqual((saved[0]["created_at"] as? NSNumber)?.int64Value, Self.now)
        XCTAssertEqual((saved[0]["modified_at"] as? NSNumber)?.int64Value, Self.now + 600)
    }

    func testDeletingLeavesATombstoneAndRemovesThePhotos() async throws {
        let photo = CatchStore.photosFolder + "/bass.jpg"
        XCTAssertTrue(files.write(Data([0xFF, 0xD8]), photo))
        let s = store()
        let id = try s.save(input(photos: ["bass.jpg"]), settingsJSON: "{}")
        XCTAssertEqual(s.photoURL("bass.jpg"), files.url(photo))

        let later = store(now: Self.now + 60)
        later.delete(id)
        XCTAssertEqual(later.count, 0)
        XCTAssertFalse(files.exists(photo), "the photo goes with the catch")
        XCTAssertNil(later.detail(id, settingsJSON: "{}", places: Self.places))

        let saved = try entries()
        XCTAssertEqual(saved.count, 1, "the record stays, so a sync carries the deletion")
        XCTAssertEqual(saved[0]["id"] as? String, id)
        XCTAssertEqual(saved[0]["deleted"] as? Bool, true)
        XCTAssertEqual((saved[0]["modified_at"] as? NSNumber)?.int64Value, Self.now + 60)
        XCTAssertNil(saved[0]["notes"], "a tombstone keeps no notes")
        XCTAssertNil(saved[0]["photos"], "nor photos")

        let again = store()
        XCTAssertEqual(again.count, 0)
        await again.rebuild(settingsJSON: "{}", places: Self.places)
        let list = try XCTUnwrap(again.list)
        XCTAssertTrue(list.rows.isEmpty)
        XCTAssertNotNil(list.empty, "the core's empty state")

        let revision = again.revision
        again.delete(id)
        XCTAssertEqual(again.revision, revision, "deleting again changes nothing")
    }

    func testAFieldTheCoreRefusesLeavesTheLogAlone() throws {
        let s = store()
        XCTAssertThrowsError(try s.save(input(length: "long"), settingsJSON: "{}")) { error in
            guard case .Field(let field, let message) = error as? LogError else {
                return XCTFail("not a field error: \(error)")
            }
            XCTAssertEqual(field, "length")
            XCTAssertEqual(CatchStore.message(error), message)
            XCTAssertFalse(message.isEmpty)
        }
        XCTAssertEqual(s.count, 0)
        XCTAssertEqual(s.revision, 0)
        XCTAssertFalse(files.exists(CatchStore.fileName), "nothing is written")
    }

    func testAnUnreadableFileIsSetAsideAndNeverWrittenOver() throws {
        XCTAssertTrue(files.write(Data("not a log".utf8), CatchStore.fileName))
        let s = store()
        XCTAssertTrue(s.unreadable)
        XCTAssertEqual(s.keptAs, "catches.json.bad")
        XCTAssertEqual(s.count, 0)
        XCTAssertEqual(files.text("catches.json.bad"), "not a log")
        XCTAssertTrue(s.canSave, "a new log starts beside the kept one")

        try s.save(input(), settingsJSON: "{}")
        XCTAssertEqual(try entries().count, 1, "a new log starts beside the old file")
        XCTAssertEqual(files.text("catches.json.bad"), "not a log")
    }

    func testNoCatchOrPhotoIsSavedWhenTheUnreadableFileCannotBeMoved() async throws {
        XCTAssertTrue(files.write(Data("not a log".utf8), CatchStore.fileName))
        // A `.bad` name for every attempt is taken by a directory, so nothing can move there.
        for name in ["catches.json.bad"] + (2...100).map({ "catches.json.\($0).bad" }) {
            try FileManager.default.createDirectory(
                at: files.url(name), withIntermediateDirectories: true)
        }
        let s = store()
        XCTAssertTrue(s.unreadable)
        XCTAssertNil(s.keptAs)
        XCTAssertFalse(s.canSave)
        XCTAssertThrowsError(try s.save(input(), settingsJSON: "{}")) { error in
            XCTAssertTrue(error is CatchStore.Unsaved, "\(error)")
        }
        XCTAssertEqual(s.count, 0, "not shown, as it would be gone at the next launch")
        XCTAssertEqual(s.revision, 0)
        do {
            _ = try await s.addPhoto(Data([0xFF, 0xD8]))
            XCTFail("a photo was kept")
        } catch {
            XCTAssertTrue(error is CatchStore.Unsaved, "\(error)")
        }
        XCTAssertFalse(files.exists(CatchStore.photosFolder), "no photo is written")
        XCTAssertEqual(files.text(CatchStore.fileName), "not a log", "never saved over")
    }

    func testAFileFromANewerVersionIsSetAside() throws {
        XCTAssertTrue(files.write(Data(#"{"v":999,"entries":[]}"#.utf8), CatchStore.fileName))
        let s = store()
        XCTAssertTrue(s.unreadable)
        XCTAssertEqual(s.keptAs, "catches.json.bad")
    }

    func testTheListAndDetailAreTheCoresWords() async throws {
        let s = store()
        let id = try s.save(input(), settingsJSON: "{}")
        await s.rebuild(settingsJSON: "{}", places: Self.places)
        let row = try XCTUnwrap(s.list?.rows.first)
        XCTAssertEqual(row.id, id)
        XCTAssertEqual(row.place, "The Battery", "the spot's name, never coordinates")
        XCTAssertEqual(row.fate, .released)
        XCTAssertFalse(row.species.isEmpty)
        XCTAssertFalse(row.when.isEmpty)
        XCTAssertNotNil(row.size)
        XCTAssertFalse(row.hasConditions)
        XCTAssertNil(s.list?.empty)

        let detail = try XCTUnwrap(s.detail(id, settingsJSON: "{}", places: Self.places))
        XCTAssertEqual(detail.row, row)
        XCTAssertTrue(detail.details.contains { $0.value == "Bucktail jig" })
        XCTAssertNotNil(detail.noConditions)
        XCTAssertFalse(detail.conditionsTitle.isEmpty)
        let text = "\(detail)"
        XCTAssertFalse(text.contains("40.70"), "no coordinates in any text: \(text)")
    }

    func testTheEmptyLogIsSpokenInThePlainVoice() async throws {
        let s = store()
        await s.rebuild(settingsJSON: "{}", places: [])
        let list = try XCTUnwrap(s.list)
        let empty = try XCTUnwrap(list.empty)
        let spoken = try XCTUnwrap(list.spokenEmpty)
        XCTAssertNotEqual(empty.title, spoken.title, "Luna's title is not what VoiceOver reads")

        let plain = try XCTUnwrap(CatchList.plainVoice("{}"))
        XCTAssertNil(CatchList.plainVoice(plain), "already plain")
        await s.rebuild(settingsJSON: plain, places: [])
        XCTAssertEqual(s.list?.empty, spoken)
        XCTAssertEqual(s.list?.spokenEmpty, spoken)
    }

    func testTheExportIsATemporaryFileThatIsDeletedAfter() throws {
        let s = store()
        try s.save(input(), settingsJSON: "{}")
        for format in CatchStore.ExportFormat.allCases {
            let file = try s.exportFile(format)
            XCTAssertEqual(file.pathExtension, format.rawValue)
            XCTAssertTrue(file.path.hasPrefix(FileManager.default.temporaryDirectory.path))
            let text = try String(contentsOf: file, encoding: .utf8)
            XCTAssertTrue(text.contains("40.7033"), "the export holds the exact position")
            CatchStore.discardExport(file)
            XCTAssertFalse(FileManager.default.fileExists(atPath: file.path))
            XCTAssertFalse(
                FileManager.default.fileExists(atPath: file.deletingLastPathComponent().path))
        }
    }

    func testDebugRunsWithoutALogUseAFolderOfTheirOwn() {
        let standard = AppFiles.standard.directory
        XCTAssertEqual(CatchStore.launchFiles([]).directory, standard)
        XCTAssertEqual(CatchStore.launchFiles(["-seed", "-tab", "log"]).directory, standard)
        for flag in ["-emptyLog", "-sample"] {
            let files = CatchStore.launchFiles([flag])
            XCTAssertNotEqual(files.directory, standard)
            XCTAssertTrue(files.directory.path.hasPrefix(FileManager.default.temporaryDirectory.path))
        }
    }
}

/// The list's filters, on the words of the core's rows.
final class CatchListFilterTests: XCTestCase {
    private func row(_ id: String, _ species: String, _ place: String?, _ fate: CatchFate, _ label: String)
        -> CatchRow
    {
        CatchRow(
            id: id, species: species, size: nil, when: "Nov 8 2026 · 6:10 AM", place: place,
            fate: fate, fateLabel: label, hasConditions: false, summary: species)
    }

    private lazy var luna = [
        row("a", "Striped bass", "The Battery", .released, "Released"),
        row("b", "Bluefish", "The Battery", .kept, "Kept"),
        row("c", "Black sea bass", "The Battery", .keptForSashimi, "Kept for sashimi"),
        row("d", "Brown trout", "Croton River", .released, "Released"),
        row("e", "Striped bass", nil, .kept, "Kept"),
    ]

    func testNoFilterLetsEveryRowThrough() {
        let filter = CatchListFilter()
        XCTAssertFalse(filter.isActive)
        XCTAssertEqual(filter.apply(luna).count, 5)
    }

    func testFiltersNarrowTogether() {
        var filter = CatchListFilter(species: "Striped bass")
        XCTAssertEqual(filter.apply(luna).map(\.id), ["a", "e"])
        filter.place = "The Battery"
        XCTAssertEqual(filter.apply(luna).map(\.id), ["a"])
        filter = CatchListFilter(fate: "Kept")
        XCTAssertEqual(filter.apply(luna).map(\.id), ["b", "e"], "Luna's sashimi tag is its own")
    }

    func testTheChoicesComeFromTheRows() {
        XCTAssertEqual(
            CatchListFilter.species(in: luna),
            ["Striped bass", "Bluefish", "Black sea bass", "Brown trout"])
        XCTAssertEqual(CatchListFilter.places(in: luna), ["The Battery", "Croton River"])
        XCTAssertEqual(CatchListFilter.fates(in: luna), ["Released", "Kept", "Kept for sashimi"])
    }

    func testThePlainVoiceKeepsTheSashimiCatchesWithTheKeptOnes() {
        let plain = [
            row("b", "Bluefish", "The Battery", .kept, "Kept"),
            row("c", "Black sea bass", "The Battery", .keptForSashimi, "Kept"),
        ]
        XCTAssertEqual(CatchListFilter.fates(in: plain), ["Kept"])
        XCTAssertEqual(CatchListFilter(fate: "Kept").apply(plain).map(\.id), ["b", "c"])
    }

    func testAChoiceNoRowHasIsDropped() {
        let filter = CatchListFilter(species: "Bluefish", place: "Croton River", fate: "Kept")
        let left = [row("a", "Striped bass", "The Battery", .released, "Released")]
        XCTAssertEqual(filter.pruned(for: left), CatchListFilter())
        XCTAssertEqual(filter.pruned(for: luna), filter)
    }
}
