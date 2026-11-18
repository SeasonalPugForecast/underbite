import Foundation
import XCTest

@testable import UnderbiteKit

/// Where the shared folder is: in the App Group's container when the app is entitled to it,
/// else in the temporary directory, saying so.
final class SharedContainerTests: XCTestCase {
    private let group = URL(filePath: "/containers/group.dev.savery.underbite", directoryHint: .isDirectory)
    private let temporary = URL(filePath: "/tmp/test", directoryHint: .isDirectory)

    func testTheAppGroupsContainerWhenThereIsOne() {
        let container = SharedContainer.resolve(
            groupContainer: group, temporaryDirectory: temporary, containerExists: { _ in true })
        XCTAssertEqual(container.backing, .appGroup)
        XCTAssertTrue(container.isShared)
        XCTAssertEqual(container.url.path(percentEncoded: false), "/containers/group.dev.savery.underbite/widgets/")
        XCTAssertTrue(container.description.contains("group.dev.savery.underbite"), container.description)
        XCTAssertEqual(SharedContainer.appGroupID, ThemeStore.appGroupID)
    }

    func testAMissingContainerFallsBackToTheTemporaryDirectoryAndSaysSo() {
        let container = SharedContainer.resolve(
            groupContainer: nil, temporaryDirectory: temporary, containerExists: { _ in true })
        XCTAssertEqual(container.backing, .temporary)
        XCTAssertFalse(container.isShared)
        XCTAssertEqual(container.url.path(percentEncoded: false), "/tmp/test/underbite-shared/widgets/")
        XCTAssertTrue(container.description.contains("no App Group"), container.description)
        XCTAssertTrue(container.description.contains("do not see"), container.description)
    }

    /// macOS answers a container URL for any group; it is only the App Group's when it exists
    /// (the theme store's rule). iOS answers `nil` without the entitlement.
    func testAContainerThatIsNotThereIsMissing() {
        let container = SharedContainer.resolve(
            groupContainer: group, temporaryDirectory: temporary, containerExists: { _ in false })
        #if os(macOS)
            XCTAssertEqual(container.backing, .temporary)
        #else
            XCTAssertEqual(container.backing, .appGroup)
        #endif
    }

    func testTheFallbackFolderCanBeMade() throws {
        // The package tests are unsigned: whichever the shared folder is, it says.
        XCTAssertFalse(SharedContainer.shared.description.isEmpty)
        let base = FileManager.default.temporaryDirectory
            .appending(path: "underbite-container-\(UUID().uuidString)", directoryHint: .isDirectory)
        defer { try? FileManager.default.removeItem(at: base) }
        let fallback = SharedContainer.resolve(
            groupContainer: nil, temporaryDirectory: base, containerExists: { _ in false })
        let made = try fallback.directory()
        XCTAssertEqual(made, fallback.url)
        var isDirectory: ObjCBool = false
        XCTAssertTrue(FileManager.default.fileExists(atPath: made.path(percentEncoded: false), isDirectory: &isDirectory))
        XCTAssertTrue(isDirectory.boolValue)
    }
}
