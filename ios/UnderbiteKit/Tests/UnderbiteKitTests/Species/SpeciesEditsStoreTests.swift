import Foundation
import XCTest

@testable import UnderbiteKit

/// The species edits: kept in `species-edits.json` as the core's text, changed only through the
/// core, read back by a new store, and a file that cannot be read set aside.
@MainActor
final class SpeciesEditsStoreTests: XCTestCase {
    /// A directory of its own for each test (XCTest makes an instance per test).
    private let directory = FileManager.default.temporaryDirectory
        .appending(path: "underbite-species-\(UUID().uuidString)", directoryHint: .isDirectory)

    override func tearDown() {
        try? FileManager.default.removeItem(at: directory)
    }

    private var files: AppFiles { AppFiles(directory: directory) }
    private let settings = defaultSettingsJson()
    private static let bass = "morone-saxatilis"

    private func store() -> SpeciesEditsStore { SpeciesEditsStore(files: files) }

    private func lures(_ edits: String) throws -> SpeciesFieldRow {
        let page = try SpeciesCatalogue.profile(id: Self.bass, edits: edits, settingsJSON: settings)
        return try XCTUnwrap(page.fields.first { $0.field == "lures" })
    }

    func testAnEditIsSavedAndReadBackInANewStore() throws {
        let s = store()
        XCTAssertEqual(s.edits, "")
        XCTAssertFalse(s.unreadable)
        try s.edit(
            id: Self.bass, field: "lures", values: ["Bucktail", " ", "Pencil popper"],
            settingsJSON: settings)
        XCTAssertEqual(s.revision, 1)
        let field = try lures(s.edits)
        XCTAssertTrue(field.edited)
        XCTAssertEqual(field.edit, ["Bucktail", "Pencil popper"])
        XCTAssertEqual(field.source?.licence, "user_edited")
        XCTAssertEqual(files.text(SpeciesEditsStore.fileName), s.edits, "the core's own text")
        XCTAssertTrue(
            SpeciesCatalogue.rows(edits: s.edits, settingsJSON: settings)
                .first { $0.id == Self.bass }?.edited ?? false)

        let again = store()
        XCTAssertEqual(again.edits, s.edits)
        XCTAssertFalse(again.unreadable)
    }

    func testARefusedValueChangesNothingAndSaysWhyInTheCoresWords() throws {
        let s = store()
        XCTAssertThrowsError(
            try s.edit(id: Self.bass, field: "spawn_temp", values: ["warm", "60"], settingsJSON: settings)
        ) { error in
            guard case SpeciesError.Field(let field, _) = error else {
                return XCTFail("\(error)")
            }
            XCTAssertEqual(field, "spawn_temp")
            XCTAssertFalse(SpeciesCatalogue.message(of: error).isEmpty)
        }
        XCTAssertEqual(s.edits, "")
        XCTAssertEqual(s.revision, 0)
        XCTAssertFalse(files.exists(SpeciesEditsStore.fileName))
    }

    func testResettingAFieldOrTheWholeProfile() throws {
        let s = store()
        try s.edit(id: Self.bass, field: "lures", values: ["Bucktail"], settingsJSON: settings)
        try s.edit(id: Self.bass, field: "water", values: ["salt"], settingsJSON: settings)
        try s.reset(id: Self.bass, field: "lures")
        XCTAssertFalse(try lures(s.edits).edited)
        var page = try SpeciesCatalogue.profile(id: Self.bass, edits: s.edits, settingsJSON: settings)
        XCTAssertTrue(page.edited, "the water is still edited")
        try s.reset(id: Self.bass, field: nil)
        page = try SpeciesCatalogue.profile(id: Self.bass, edits: s.edits, settingsJSON: settings)
        XCTAssertFalse(page.edited)
        XCTAssertEqual(store().edits, s.edits)
    }

    func testUnreadableEditsAreSetAsideAndNeverOverwritten() throws {
        files.write(Data(#"{"species": 7}"#.utf8), SpeciesEditsStore.fileName)
        let s = store()
        XCTAssertTrue(s.unreadable)
        XCTAssertEqual(s.edits, "")
        XCTAssertEqual(s.keptAs, "species-edits.json.bad")
        XCTAssertTrue(s.canSave)
        try s.edit(id: Self.bass, field: "lures", values: ["Bucktail"], settingsJSON: settings)
        XCTAssertEqual(files.text("species-edits.json.bad"), #"{"species": 7}"#)
        XCTAssertFalse(store().unreadable)
    }

    func testNoEditIsSavedWhenTheUnreadableFileCannotBeMoved() throws {
        files.write(Data("not json".utf8), SpeciesEditsStore.fileName)
        let taken = ["species-edits.json.bad"] + (2...100).map { "species-edits.json.\($0).bad" }
        for name in taken {
            try FileManager.default.createDirectory(
                at: files.url(name), withIntermediateDirectories: true)
        }
        let s = store()
        XCTAssertTrue(s.unreadable)
        XCTAssertNil(s.keptAs)
        XCTAssertFalse(s.canSave)
        XCTAssertThrowsError(
            try s.edit(id: Self.bass, field: "lures", values: ["Bucktail"], settingsJSON: settings)
        ) { error in
            XCTAssertTrue(error is SpeciesEditsStore.Unsaved, "\(error)")
        }
        XCTAssertEqual(files.text(SpeciesEditsStore.fileName), "not json", "never saved over")
    }
}
