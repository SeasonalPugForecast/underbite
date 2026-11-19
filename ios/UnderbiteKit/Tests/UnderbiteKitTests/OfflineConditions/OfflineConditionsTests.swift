import Foundation
import XCTest

@testable import UnderbiteKit

/// A value the tests share with the store's `@Sendable` closures.
private final class Shared<Value>: @unchecked Sendable {
    private let lock = NSLock()
    private var held: Value
    init(_ value: Value) { held = value }
    var value: Value {
        get { lock.withLock { held } }
        set { lock.withLock { held = newValue } }
    }
}

/// One call of the store's fetch, as the fetch saw it.
private struct Call: Sendable {
    var spot: String
    var refreshMetadata: Bool
    var cached: [String: Int64]
    var withoutSignal: Bool
}

/// The spots' conditions without signal: how an outcome is read, the station metadata kept in
/// the offline store and passed back, the banner's note and the waits between tries. No
/// network: the fetch is stood in for, the store is a scratch file.
@MainActor
final class OfflineConditionsTests: XCTestCase {
    private let directory = FileManager.default.temporaryDirectory
        .appending(path: "underbite-offline-conditions-\(UUID().uuidString)", directoryHint: .isDirectory)
    private let scratch = ScratchStore()

    override func tearDown() {
        try? FileManager.default.removeItem(at: directory)
    }

    /// 2026-11-18T12:00:00Z.
    private let start: Int64 = 1_795_003_200
    nonisolated private static let harcon =
        "https://api.tidesandcurrents.noaa.gov/mdapi/prod/webapi/stations/8518750/harcon.json?units=metric"
    nonisolated private static let datums =
        "https://api.tidesandcurrents.noaa.gov/mdapi/prod/webapi/stations/8518750/datums.json?units=metric"
    nonisolated private static let note =
        "offline: computed from harmonic constants, other values from Nov 18, 7:00 AM"

    nonisolated private static func issue(_ source: String, _ kind: String, _ message: String) -> SourceIssue {
        SourceIssue(source: source, measurement: nil, kind: kind, message: message)
    }

    nonisolated private static func response(_ key: String, _ body: String, at time: Int64) -> CachedResponse {
        CachedResponse(key: key, body: Data(body.utf8), fetchedAt: time)
    }

    /// A store of the Battery whose fetch answers with `answer` and records each call.
    private func store(
        clock: Shared<Int64>, calls: Shared<[Call]>, simulatesOffline: Bool = false,
        answer: @escaping @Sendable (Int) -> SpotFetchOutcome
    ) -> SpotStore {
        let files = AppFiles(directory: directory)
        let s = SpotStore(
            files: files, host: URLSessionHost(storeURL: files.hostDirectory),
            keys: { Keys(usgs: nil, copernicusUser: nil, copernicusPassword: nil) },
            clock: { clock.value }, lowPower: { false },
            metadataFetcher: { spot, _, _, options, _, cached, host in
                var seen = calls.value
                seen.append(
                    Call(
                        spot: spot.id, refreshMetadata: options.refreshMetadata,
                        cached: Dictionary(uniqueKeysWithValues: cached.map { ($0.key, $0.fetchedAt) }),
                        withoutSignal: host is UnreachableHost))
                calls.value = seen
                return answer(seen.count)
            },
            metadata: SpotMetadata(store: scratch.store), simulatesOffline: simulatesOffline,
            pageBuilder: { _, _, _, _ in #"{"active":0,"spots":[]}"# })
        s.add(SeedSpots.battery)
        return s
    }

    nonisolated private static func online(at time: Int64) -> SpotFetchOutcome {
        SpotFetchOutcome(
            snapshotJson: "online-\(time)", errors: [], suggestion: nil,
            cacheable: [
                Self.response(Self.harcon, #"{"HarmonicConstituents":[]}"#, at: time),
                Self.response(Self.datums, #"{"datums":[]}"#, at: time),
            ])
    }

    nonisolated private static func offline() -> SpotFetchOutcome {
        SpotFetchOutcome(
            snapshotJson: "merged",
            errors: [
                Self.issue("coops", "transport", "The Internet connection appears to be offline."),
                Self.issue("offline", "stale", Self.note),
            ], suggestion: nil)
    }

    // MARK: - Reading an outcome

    func testAnOutcomeIsOnlineOfflineOrUnreachable() {
        XCTAssertEqual(FetchStanding([]), .online)
        XCTAssertEqual(
            FetchStanding([Self.issue("usgs", "rate_limited", "429")]), .online,
            "a source that failed while others answered")
        XCTAssertEqual(FetchStanding(Self.offline().errors), .offline(note: Self.note))
        XCTAssertEqual(
            FetchStanding([Self.issue("coops", "transport", "no route")]),
            .unreachable(message: "no route"))
    }

    // MARK: - The store

    func testMetadataIsKeptPassedBackAndAskedForAgainOnlyWhenSomeoneAsks() async {
        let clock = Shared(start)
        let calls = Shared<[Call]>([])
        let s = store(clock: clock, calls: calls) { _ in Self.online(at: clock.value) }

        // Opening the app: nothing kept yet, the core decides what to ask for.
        await s.refreshOnOpen()
        XCTAssertEqual(calls.value.count, 1)
        XCTAssertEqual(calls.value[0].cached, [:])
        XCTAssertFalse(calls.value[0].refreshMetadata)
        XCTAssertEqual(
            Set(s.cache(SeedSpots.battery.id).metadataKeys ?? []), [Self.harcon, Self.datums])

        // A pull to refresh passes back what was kept, with its own fetch time, and asks again.
        clock.value = start + 600
        await s.refresh(only: [SeedSpots.battery.id])
        XCTAssertEqual(calls.value[1].cached, [Self.harcon: start, Self.datums: start])
        XCTAssertTrue(calls.value[1].refreshMetadata)

        // The next timed refresh passes back the copies of the pull, and does not ask again.
        clock.value = start + 3_600
        await s.refreshOnOpen()
        XCTAssertEqual(calls.value.count, 3)
        XCTAssertEqual(calls.value[2].cached, [Self.harcon: start + 600, Self.datums: start + 600])
        XCTAssertFalse(calls.value[2].refreshMetadata)
    }

    func testWithoutSignalTheMergedSnapshotShowsWithTheNoteAndTheLastSuccessStays() async {
        let clock = Shared(start)
        let calls = Shared<[Call]>([])
        let signal = Shared(true)
        let s = store(clock: clock, calls: calls) { _ in
            signal.value ? Self.online(at: clock.value) : Self.offline()
        }
        let id = SeedSpots.battery.id
        await s.refresh()
        XCTAssertNil(s.offlineNote)

        signal.value = false
        clock.value = start + 7_200
        await s.refresh()
        let held = s.cache(id)
        XCTAssertEqual(held.snapshotJSON, "merged", "the core's merge is what shows")
        XCTAssertEqual(held.lastSuccess, start, "updated stays at the last answer")
        XCTAssertEqual(held.lastAttempt, start + 7_200)
        XCTAssertNil(held.error, "the note is the banner's, not an error")
        XCTAssertEqual(s.offlineNote, Self.note)
        XCTAssertEqual(
            Set(held.metadataKeys ?? []), [Self.harcon, Self.datums],
            "an outcome offering nothing keeps the keys")
        // The kept constants went with the fetch without signal.
        XCTAssertEqual(calls.value[1].cached.count, 2)

        // Not tried again every minute: the interval counts from the try without signal.
        clock.value = start + 7_200 + 120
        await s.refreshOnOpen()
        XCTAssertEqual(calls.value.count, 2)

        // The route came back: the spots waiting are fetched, and an answer ends the banner.
        signal.value = true
        await s.refreshWaiting()
        XCTAssertEqual(calls.value.count, 3)
        XCTAssertNil(s.offlineNote)
        XCTAssertNil(s.cache(id).lastAttempt)
        XCTAssertEqual(s.cache(id).lastSuccess, clock.value)
        await s.refreshWaiting()
        XCTAssertEqual(calls.value.count, 3, "nothing waits any more")
    }

    func testNothingAnsweredAndNothingToMergeKeepsTheOlderSnapshot() async {
        let clock = Shared(start)
        let calls = Shared<[Call]>([])
        let signal = Shared(true)
        let s = store(clock: clock, calls: calls) { _ in
            signal.value
                ? Self.online(at: clock.value)
                : SpotFetchOutcome(
                    snapshotJson: "empty",
                    errors: [Self.issue("coops", "transport", "no route")], suggestion: nil)
        }
        await s.refresh()
        signal.value = false
        await s.refresh()
        let held = s.cache(SeedSpots.battery.id)
        XCTAssertEqual(held.snapshotJSON, "online-\(start)")
        XCTAssertEqual(held.error, "no route")
        XCTAssertNil(s.offlineNote)
    }

    func testDeletingTheSpotLetsItsMetadataGo() async throws {
        let clock = Shared(start)
        let calls = Shared<[Call]>([])
        let s = store(clock: clock, calls: calls) { _ in Self.online(at: clock.value) }
        await s.refresh()
        let pin = SpotMetadata.pin(spot: SeedSpots.battery.id)
        let pinned = try await scratch.store.usage(of: pin)
        XCTAssertEqual(pinned.stored, 2)
        s.remove(SeedSpots.battery.id)
        // The unpin runs on its own task.
        for _ in 0..<50 {
            guard try await scratch.store.usage(of: pin).items > 0 else { break }
            try await Task.sleep(for: .milliseconds(20))
        }
        let after = try await scratch.store.usage()
        XCTAssertEqual(after.blobs, 0)
    }

    func testADebugOfflineLaunchFetchesOnceOnlineThenWithoutSignal() async {
        let clock = Shared(start)
        let calls = Shared<[Call]>([])
        let s = store(clock: clock, calls: calls, simulatesOffline: true) { n in
            n == 1 ? Self.online(at: clock.value) : Self.offline()
        }
        await s.refresh()
        XCTAssertEqual(calls.value.map(\.withoutSignal), [false, true])
        XCTAssertEqual(calls.value[1].cached.count, 2)
        XCTAssertEqual(s.offlineNote, Self.note)

        // Once primed (or with kept data and constants), every fetch is without signal.
        await s.refresh()
        XCTAssertEqual(calls.value.map(\.withoutSignal), [false, true, true])
    }

    func testADebugOfflineLaunchFetchesASpotWithoutSignalRightAfterItsOnlineFetch() async {
        let clock = Shared(start)
        let calls = Shared<[Call]>([])
        let s = store(clock: clock, calls: calls, simulatesOffline: true) { _ in
            calls.value.last?.withoutSignal == true ? Self.offline() : Self.online(at: clock.value)
        }
        s.add(SeedSpots.croton)
        await s.refresh()
        let order = calls.value.map { "\($0.spot) \($0.withoutSignal ? "offline" : "online")" }
        XCTAssertEqual(
            order,
            [
                "\(SeedSpots.battery.id) online", "\(SeedSpots.battery.id) offline",
                "\(SeedSpots.croton.id) online", "\(SeedSpots.croton.id) offline",
            ])
    }

    func testTheBannerShowsTheShownSpotsNote() async {
        let clock = Shared(start)
        let calls = Shared<[Call]>([])
        let signal = Shared(false)
        let river = "offline: values from Nov 18, 7:00 AM"
        let s = store(clock: clock, calls: calls) { _ in
            if signal.value { return Self.online(at: clock.value) }
            let spot = calls.value.last?.spot
            return spot == SeedSpots.croton.id
                ? SpotFetchOutcome(
                    snapshotJson: "merged",
                    errors: [Self.issue("offline", "stale", river)], suggestion: nil)
                : Self.offline()
        }
        s.add(SeedSpots.croton)
        await s.refresh()
        XCTAssertEqual(s.offlineNote, Self.note, "the shown spot's, the Battery's")
        s.setActive(SeedSpots.croton.id)
        XCTAssertEqual(s.offlineNote, river)

        // The river answers again: the banner falls back to the Battery's note.
        signal.value = true
        await s.refresh(only: [SeedSpots.croton.id])
        XCTAssertEqual(s.offlineNote, Self.note)
        s.remove(SeedSpots.battery.id)
        XCTAssertNil(s.offlineNote)
    }

    func testAQueuedSpotCanGoFirst() {
        var queue = RefreshQueue()
        queue.add(["a", "b", "c"])
        queue.addFirst("c")
        queue.addFirst("d")
        XCTAssertEqual(queue.waiting, ["d", "c", "a", "b"])
    }

    // MARK: - The kept metadata in the offline store

    func testClearingTheCacheKeepsTheSpotsMetadata() async throws {
        let metadata = SpotMetadata(store: scratch.store)
        let keys = await metadata.keep(
            [Self.response(Self.harcon, "{}", at: start)], spot: "a", kept: [Self.datums],
            now: start)
        XCTAssertEqual(keys, [Self.datums, Self.harcon])
        try await scratch.store.put(
            .blob("sst/x/last/1"),
            OfflineEntry(data: Data("{}".utf8), contentType: nil, fetchedAt: start, lifetime: 60),
            now: start)
        let freed = try await scratch.store.removeUnpinned()
        XCTAssertEqual(freed, 2)
        let back = await metadata.cached(keys, now: start + 90 * 86_400)
        XCTAssertEqual(back.map(\.key), [Self.harcon], "kept at any age; the missing one skipped")
        XCTAssertEqual(back.first?.fetchedAt, start)
        await metadata.forget(spot: "a")
        let left = await metadata.cached(keys, now: start)
        XCTAssertTrue(left.isEmpty)
    }

    func testStationListsAreKeptInTheCacheNotPinned() async throws {
        let metadata = SpotMetadata(store: scratch.store)
        let list =
            "https://api.tidesandcurrents.noaa.gov/mdapi/prod/webapi/stations.json?type=tidepredictions"
        XCTAssertTrue(SpotMetadata.isStationFile(Self.harcon))
        XCTAssertFalse(SpotMetadata.isStationFile(list))
        XCTAssertFalse(SpotMetadata.isStationFile("https://www.ndbc.noaa.gov/activestations.xml"))
        let keys = await metadata.keep(
            [Self.response(Self.harcon, "{}", at: start), Self.response(list, "[]", at: start)],
            spot: "a", kept: [], now: start)
        XCTAssertEqual(keys, [Self.harcon, list])
        let both = await metadata.cached(keys, now: start)
        XCTAssertEqual(both.count, 2, "both passed back while kept")
        try await scratch.store.removeUnpinned()
        let left = await metadata.cached(keys, now: start)
        XCTAssertEqual(left.map(\.key), [Self.harcon], "the list goes with the cache")
    }

    func testTwoSpotsShareAStationsMetadata() async throws {
        let metadata = SpotMetadata(store: scratch.store)
        let entry = Self.response(Self.harcon, "{}", at: start)
        _ = await metadata.keep([entry], spot: "a", kept: [], now: start)
        _ = await metadata.keep([entry], spot: "b", kept: [], now: start)
        await metadata.forget(spot: "a")
        let held = await metadata.cached([Self.harcon], now: start)
        XCTAssertEqual(held.count, 1, "spot b still holds it")
    }

    // MARK: - The route

    func testTheRouteCountsAsBackOnlyOnceItHolds() async {
        let returns = Shared(0)
        let gate = Shared<[CheckedContinuation<Void, Never>]>([])
        let watch = RouteReturn(
            sleep: { _ in await withCheckedContinuation { c in gate.value.append(c) } },
            onReturn: { returns.value += 1 })
        func release() {
            let waiting = gate.value
            gate.value = []
            for c in waiting { c.resume() }
        }

        // The first path at launch is not a return.
        XCTAssertNil(watch.update(satisfied: true))

        // Lost, back, lost again before it held: nothing.
        watch.update(satisfied: false)
        let brief = watch.update(satisfied: true)
        XCTAssertNotNil(brief)
        watch.update(satisfied: false)
        while gate.value.isEmpty { await Task.yield() }
        release()
        await brief?.value
        XCTAssertEqual(returns.value, 0)

        // Back and held: one call, and further updates while up start nothing.
        let held = watch.update(satisfied: true)
        XCTAssertNil(watch.update(satisfied: true))
        while gate.value.isEmpty { await Task.yield() }
        release()
        await held?.value
        XCTAssertEqual(returns.value, 1)
        XCTAssertNil(watch.update(satisfied: true))
    }
}
