import Foundation
import XCTest

@testable import UnderbiteKit

/// A species profile sent as a `.underbite` file: the received-file preview reads it through
/// the species calls, and importing it changes only the user's edits, keeping the user's own
/// edit on a conflict unless asked.
@MainActor
final class SpeciesShareTests: XCTestCase {
    private let directory = FileManager.default.temporaryDirectory
        .appending(path: "underbite-species-share-\(UUID().uuidString)", directoryHint: .isDirectory)

    override func tearDown() {
        try? FileManager.default.removeItem(at: directory)
    }

    private let settings = defaultSettingsJson()
    private static let bass = "morone-saxatilis"

    private func store(_ name: String) -> SpeciesEditsStore {
        SpeciesEditsStore(
            files: AppFiles(directory: directory.appending(path: name, directoryHint: .isDirectory)))
    }

    private func lures(_ edits: String) throws -> [String] {
        let page = try SpeciesCatalogue.profile(id: Self.bass, edits: edits, settingsJSON: settings)
        return try XCTUnwrap(page.fields.first { $0.field == "lures" }).edit
    }

    func testASharedProfileIsPreviewedThenImportedIntoTheEdits() throws {
        let sender = store("sender")
        try sender.edit(id: Self.bass, field: "lures", values: ["Bucktail"], settingsJSON: settings)
        let text = try speciesShareFile(id: Self.bass, edits: sender.edits)

        let read = ReceivedFile.read(Data(text.utf8), settingsJSON: settings, now: 0, edits: "")
        guard case .profile(let profile) = read.content else { return XCTFail("\(read)") }
        XCTAssertEqual(read.heading, "Striped bass")
        XCTAssertEqual(profile.shared.id, Self.bass)
        XCTAssertTrue(profile.shared.plan.known)
        XCTAssertEqual(profile.shared.plan.added.map(\.field), ["lures"])
        XCTAssertEqual(read.choices(trips: [], spots: []), [], "a profile is imported, not added")
        let page = try XCTUnwrap(profile.page)
        XCTAssertTrue(page.fields.contains { $0.source?.licenceLabel == "CC BY-NC 4.0" })
        XCTAssertTrue(page.fields.contains { $0.edited }, "the sender's edit is marked")

        let other = store("other")
        try other.edit(id: Self.bass, field: "lures", values: ["Eel"], settingsJSON: settings)
        let again = ReceivedFile.read(
            Data(text.utf8), settingsJSON: settings, now: 0, edits: other.edits)
        guard case .profile(let conflicting) = again.content else { return XCTFail("\(again)") }
        XCTAssertEqual(conflicting.shared.plan.conflicts.map(\.field), ["lures"])
        XCTAssertEqual(conflicting.shared.plan.added, [])

        try other.importProfile(conflicting.data, takeTheirs: false, settingsJSON: settings)
        XCTAssertEqual(try lures(other.edits), ["Eel"], "the user's edit stays")
        try other.importProfile(conflicting.data, takeTheirs: true, settingsJSON: settings)
        XCTAssertEqual(try lures(other.edits), ["Bucktail"])
        XCTAssertEqual(store("other").edits, other.edits, "kept in the file")
    }

    func testAFileOfAnotherKindIsNotReadAsAProfile() {
        let layers = Data(
            #"{"format":"underbite","kind":"layers","v":1,"layers":{"name":"Surf","layers":[]}}"#
                .utf8)
        let read = ReceivedFile.read(layers, settingsJSON: settings, now: 0, edits: "")
        guard case .preset(let preset) = read.content else { return XCTFail("\(read)") }
        XCTAssertEqual(preset.name, "Surf")
    }

    func testABrokenProfileFileSaysWhyInTheCoresWords() {
        let broken = Data(#"{"format":"underbite","kind":"profile","v":1}"#.utf8)
        let read = ReceivedFile.read(broken, settingsJSON: settings, now: 0, edits: "")
        XCTAssertNil(read.heading)
        guard case .problem(let message) = read.content else { return XCTFail("\(read)") }
        XCTAssertFalse(message.isEmpty)
        XCTAssertFalse(message.contains("ShareError"), message)
    }
}
