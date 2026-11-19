import Foundation
import XCTest

@testable import UnderbiteKit

/// The packs as the app keeps them: downloads to their end, pause, resume after the app was
/// stopped, cancel and delete, in the packs file (never the settings).
@MainActor
final class PackLibraryTests: XCTestCase {
    private let scratch = ScratchStore()
    private let directory = FileManager.default.temporaryDirectory.appending(
        path: "underbite-pack-library-\(UUID().uuidString)", directoryHint: .isDirectory)
    private var files: AppFiles { AppFiles(directory: directory) }

    override func tearDown() {
        try? FileManager.default.removeItem(at: directory)
        super.tearDown()
    }

    private func library(_ host: PackTestHost, clock: MovingClock = MovingClock(packNow))
        -> PackLibrary
    {
        let store = scratch.store
        return PackLibrary(
            files: files, clock: { clock.seconds },
            context: { packContext(host: host, store: store, clock: clock) })
    }

    func testAPackIsDownloadedMarkedRefreshedAndItsSpotsFetched() async throws {
        let settings = try packSettings()
        let json = try batteryChartPack(settings: settings)
        let host = PackTestHost { _, _ in tileAnswer() }
        let clock = MovingClock(packNow)
        let library = library(host, clock: clock)
        let fetched = PackBoxed<[String]>([])
        library.refreshSpots = { ids in fetched.value = ids }
        let id = try XCTUnwrap(library.add(json))
        XCTAssertNil(library.add(json), "one pack per id")
        XCTAssertEqual(library.record(id)?.status, .notDownloaded)
        XCTAssertTrue(host.urls.isEmpty, "nothing is downloaded before it is asked")

        await library.download(id, settingsJSON: settings)
        XCTAssertTrue(library.isRunning(id))
        await library.wait(id)
        try await Task.sleep(for: .milliseconds(20))
        let record = try XCTUnwrap(library.record(id))
        XCTAssertEqual(record.status, .done)
        XCTAssertNil(record.progress)
        XCTAssertEqual(record.info?.refreshedAt, packNow)
        XCTAssertEqual(record.lastFailed, 0)
        XCTAssertEqual(fetched.value, ["seed-battery"])
        XCTAssertGreaterThan(library.storedBytes[id] ?? 0, 0)
        XCTAssertNotNil(library.file.measured["noaa-chart"])
        // Kept in the packs file, read again by the next launch.
        let again = self.library(host)
        XCTAssertEqual(again.record(id)?.status, .done)
        XCTAssertEqual(again.record(id)?.json, record.json)
        // Fresh until the core calls it stale: the newest data a day after.
        XCTAssertEqual(library.due(settingsJSON: settings, now: packNow + 3_600), [])
        XCTAssertEqual(library.due(settingsJSON: settings, now: packNow + 2 * 86_400), [id])
    }

    func testADownloadTheAppStoppedResumesByItself() async throws {
        let settings = try packSettings()
        let json = try batteryChartPack(settings: settings)
        // The packs file as a stopped app left it: a download under way, its first page done.
        var file = PacksFile()
        var progress = PackProgress(startedAt: packNow, plannedAt: packNow)
        progress.total = 99
        file.packs = [PackRecord(json: json, status: .downloading, progress: progress)]
        XCTAssertTrue(file.save(files))

        let host = PackTestHost { _, _ in tileAnswer() }
        let library = library(host)
        let id = "battery-chart"
        XCTAssertEqual(library.record(id)?.status, .paused)
        XCTAssertEqual(library.record(id)?.userPaused, false)
        await library.resumeInterrupted(settingsJSON: settings)
        await library.wait(id)
        XCTAssertEqual(library.record(id)?.status, .done)
        XCTAssertFalse(host.urls.isEmpty)
    }

    func testAPauseWaitsForATapAndResumesWhereItStopped() async throws {
        let settings = try packSettings()
        let json = try batteryChartPack(settings: settings)
        let total = try planPack(
            packJson: json, settingsJson: settings, measuredSizes: [:], timeAxes: [:],
            now: packNow
        ).itemCount
        let served = ManagedCount()
        let host = PackTestHost { _, _ in
            if served.next() >= 2 { try await Task.sleep(for: .seconds(60)) }
            return tileAnswer()
        }
        let library = library(host)
        let id = try XCTUnwrap(library.add(json))
        await library.download(id, settingsJSON: settings)
        for _ in 0..<400 {
            if try await scratch.store.usage().tiles >= 2 { break }
            try await Task.sleep(for: .milliseconds(5))
        }
        library.pause(id)
        await library.wait(id)
        XCTAssertEqual(library.record(id)?.status, .paused)
        XCTAssertEqual(library.record(id)?.userPaused, true)
        XCTAssertNotNil(library.record(id)?.progress)
        // Not resumed by itself.
        await library.resumeInterrupted(settingsJSON: settings)
        XCTAssertFalse(library.isRunning(id))

        let asked = host.urls.count
        let answering = PackTestHost { _, _ in tileAnswer() }
        let store = scratch.store
        let resumed = PackLibrary(
            files: files, clock: { packNow },
            context: { packContext(host: answering, store: store, clock: MovingClock(packNow)) })
        await resumed.download(id, settingsJSON: settings)
        await resumed.wait(id)
        XCTAssertEqual(resumed.record(id)?.status, .done)
        XCTAssertEqual(answering.urls.count, Int(total) - 2, "the stored two are kept")
        XCTAssertGreaterThan(asked, 2)
    }

    func testCancellingAFirstDownloadDropsWhatItStoredAndDeletingAPackUnpinsIt() async throws {
        let settings = try packSettings()
        let json = try batteryChartPack(settings: settings)
        let served = ManagedCount()
        let host = PackTestHost { _, _ in
            if served.next() >= 3 { try await Task.sleep(for: .seconds(60)) }
            return tileAnswer()
        }
        let library = library(host)
        let id = try XCTUnwrap(library.add(json))
        await library.download(id, settingsJSON: settings)
        for _ in 0..<400 {
            if try await scratch.store.usage().tiles >= 3 { break }
            try await Task.sleep(for: .milliseconds(5))
        }
        await library.cancel(id)
        XCTAssertEqual(library.record(id)?.status, .cancelled)
        XCTAssertNil(library.record(id)?.progress)
        let found1 = try await scratch.store.packs()
        XCTAssertEqual(found1, [])
        let found2 = try await scratch.store.usage().tiles
        XCTAssertEqual(found2, 0)

        let done = PackTestHost { _, _ in tileAnswer() }
        let store = scratch.store
        let other = PackLibrary(
            files: files, clock: { packNow },
            context: { packContext(host: done, store: store, clock: MovingClock(packNow)) })
        await other.download(id, settingsJSON: settings)
        await other.wait(id)
        let found3 = try await scratch.store.packs()
        XCTAssertEqual(found3, [id])
        await other.delete(id)
        XCTAssertNil(other.record(id))
        let found4 = try await scratch.store.packs()
        XCTAssertEqual(found4, [])
        let found5 = try await scratch.store.usage().tiles
        XCTAssertEqual(found5, 0)
        XCTAssertTrue(PacksFile.load(files).packs.isEmpty)
    }

    func testTheWifiChoiceIsKeptWithThePacks() throws {
        let library = library(PackTestHost { _, _ in tileAnswer() })
        XCTAssertTrue(library.wifiOnly)
        library.wifiOnly = false
        XCTAssertFalse(PacksFile.load(files).wifiOnly)
    }
}

/// A value a callback sets.
final class PackBoxed<T>: @unchecked Sendable {
    var value: T
    init(_ value: T) { self.value = value }
}
