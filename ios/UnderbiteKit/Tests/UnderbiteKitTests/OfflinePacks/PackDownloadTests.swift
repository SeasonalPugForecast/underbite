import Foundation
import XCTest

@testable import UnderbiteKit

/// Pack downloads against a host that answers from the test: what is stored and pinned, how
/// many requests go at once, the back-off and the pacer, Wi-Fi gating, and resuming after an
/// interruption.
final class PackDownloadTests: XCTestCase {
    private let scratch = ScratchStore()

    private func download(
        _ json: String, settings: String, context: PackContext, wifiOnly: Bool = true,
        from progress: PackProgress? = nil, pageSize: UInt32 = PackRules.pageSize,
        events: EventLog = EventLog()
    ) -> PackDownload {
        PackDownload(
            packJSON: json, settingsJSON: settings, wifiOnly: PackFlag(wifiOnly),
            from: progress ?? PackProgress(startedAt: packNow, plannedAt: packNow),
            context: context, pageSize: pageSize, report: { events.add($0) })
    }

    private func plannedItems(_ json: String, settings: String) throws -> UInt64 {
        try planPack(
            packJson: json, settingsJson: settings, measuredSizes: [:], timeAxes: [:],
            now: packNow
        ).itemCount
    }

    func testEveryTileIsCheckedStoredAndPinned() async throws {
        let settings = try packSettings()
        let json = try batteryChartPack(settings: settings)
        let total = try plannedItems(json, settings: settings)
        XCTAssertGreaterThan(total, 4)
        let host = PackTestHost { _, _ in tileAnswer() }
        let clock = MovingClock(packNow)
        let end = await download(
            json, settings: settings,
            context: packContext(host: host, store: scratch.store, clock: clock)
        ).run()
        guard case .done(let progress) = end else { return XCTFail("\(end)") }
        XCTAssertEqual(progress.total, total)
        XCTAssertEqual(progress.done, total)
        XCTAssertEqual(progress.failed, 0)
        XCTAssertEqual(progress.bytes, total * UInt64(tileBody.count))
        XCTAssertEqual(progress.layerItems["noaa-chart"], total)
        XCTAssertEqual(host.urls.count, Int(total))
        let usage = try await scratch.store.usage(of: "battery-chart")
        XCTAssertEqual(usage.items, Int(total))
        XCTAssertEqual(usage.stored, Int(total))
        // The map finds them: offline it draws what the pack stored.
        let tile = try XCTUnwrap(
            OfflineTileKey(text: try XCTUnwrap(
                packRequests(
                    plan: planPack(
                        packJson: json, settingsJson: settings, measuredSizes: [:],
                        timeAxes: [:], now: packNow
                    ).plan, from: 0, count: 1
                ).first?.key)))
        let record = try await scratch.store.get(.tile(tile), now: packNow + 999_999, allowingStale: true)
        XCTAssertEqual(record?.data, tileBody)
    }

    func testErrorPagesAreNeverStored() async throws {
        let settings = try packSettings()
        let json = try batteryChartPack(settings: settings)
        let total = try plannedItems(json, settings: settings)
        // The chart service answers failures with status 200 and a web page.
        let host = PackTestHost { _, _ in
            tileAnswer(type: "text/html", body: Data("<html>error</html>".utf8))
        }
        let end = await download(
            json, settings: settings,
            context: packContext(host: host, store: scratch.store, clock: MovingClock(packNow))
        ).run()
        guard case .done(let progress) = end else { return XCTFail("\(end)") }
        XCTAssertEqual(progress.failed, total)
        XCTAssertEqual(progress.bytes, 0)
        let usage = try await scratch.store.usage()
        XCTAssertEqual(usage.tiles, 0)
    }

    func testAtMostFourRequestsGoToAHostAtOnce() async throws {
        let settings = try packSettings()
        let json = try batteryChartPack(settings: settings)
        let host = PackTestHost(delay: .milliseconds(20)) { _, _ in tileAnswer() }
        let end = await download(
            json, settings: settings,
            context: packContext(host: host, store: scratch.store, clock: MovingClock(packNow))
        ).run()
        guard case .done = end else { return XCTFail("\(end)") }
        let most = host.mostInFlight.values.max() ?? 0
        XCTAssertLessThanOrEqual(most, PackRules.perHost)
        XCTAssertGreaterThan(most, 1, "requests go side by side")
    }

    func testFailuresAreAskedAgainAfterABackOff() async throws {
        let settings = try packSettings()
        let json = try batteryChartPack(settings: settings)
        let total = try plannedItems(json, settings: settings)
        let first = try XCTUnwrap(
            packRequests(
                plan: planPack(
                    packJson: json, settingsJson: settings, measuredSizes: [:], timeAxes: [:],
                    now: packNow
                ).plan, from: 0, count: 1
            ).first?.request?.url)
        // The first tile fails twice at the transport, then answers; the others answer at once.
        let host = PackTestHost { request, call in
            if request.url == first, call < 2 { throw HostError.Failed(message: "reset") }
            return tileAnswer()
        }
        let pauses = PauseLog()
        let end = await download(
            json, settings: settings,
            context: packContext(
                host: host, store: scratch.store, clock: MovingClock(packNow), pauses: pauses)
        ).run()
        guard case .done(let progress) = end else { return XCTFail("\(end)") }
        XCTAssertEqual(progress.failed, 0)
        XCTAssertEqual(pauses.all, [2, 4])
        XCTAssertEqual(host.urls.filter { $0 == first }.count, 3)
        XCTAssertEqual(host.urls.count, Int(total) + 2)

        // A tile that never answers fails after the tries, and the download goes on.
        let failing = PackTestHost { request, _ in
            if request.url == first { throw HostError.TimedOut(message: "timed out") }
            return tileAnswer()
        }
        let other = ScratchStore()
        let pauses2 = PauseLog()
        let end2 = await download(
            json, settings: settings,
            context: packContext(
                host: failing, store: other.store, clock: MovingClock(packNow), pauses: pauses2)
        ).run()
        guard case .done(let p2) = end2 else { return XCTFail("\(end2)") }
        XCTAssertEqual(p2.failed, 1)
        XCTAssertEqual(p2.done, total)
        XCTAssertEqual(failing.urls.filter { $0 == first }.count, Int(PackRules.attempts))
        XCTAssertEqual(pauses2.all, [2, 4, 8])
    }

    func testAServerAskingToSlowDownIsWaitedForAndCounted() async throws {
        let settings = try packSettings()
        let json = try batteryChartPack(settings: settings)
        let host = PackTestHost { _, call in
            call == 0 ? tileAnswer(503, type: "text/plain", body: Data("busy".utf8)) : tileAnswer()
        }
        let pauses = PauseLog()
        let end = await download(
            json, settings: settings,
            context: packContext(
                host: host, store: scratch.store, clock: MovingClock(packNow), pauses: pauses)
        ).run()
        guard case .done(let progress) = end else { return XCTFail("\(end)") }
        XCTAssertEqual(progress.failed, 0)
        XCTAssertEqual(progress.throttled, progress.total)
        XCTAssertEqual(pauses.all.count, Int(progress.total))
        XCTAssertTrue(pauses.all.allSatisfy { $0 >= 1 && $0 <= PackRules.longestBackOff })
    }

    func testGridBlocksAreLetThroughByThePacerAndCheckedByTheParser() async throws {
        let settings = try packSettings()
        let json = try sstPack(settings: settings)
        let total = try plannedItems(json, settings: settings)
        XCTAssertGreaterThan(total, 0)
        let clock = MovingClock(packNow)
        // The server was found busy a moment ago: nothing goes before the pacer allows it.
        let pacer = GridPacer()
        try pacer.record(node: "pfeg", status: 503, retryAfterS: 30, now: packNow)
        let before = try pacer.maySend(node: "pfeg", now: packNow)
        XCTAssertFalse(before.send)
        let host = PackTestHost { request, _ in
            XCTAssertGreaterThanOrEqual(clock.seconds, packNow + Int64(before.waitS))
            return tileAnswer(type: "application/json;charset=UTF-8", body: murTable())
        }
        let pauses = PauseLog()
        let end = await download(
            json, settings: settings,
            context: packContext(
                host: host, store: scratch.store, clock: clock, pacer: pacer, pauses: pauses)
        ).run()
        guard case .done(let progress) = end else { return XCTFail("\(end)") }
        XCTAssertEqual(progress.failed, 0)
        XCTAssertEqual(progress.done, total)
        XCTAssertGreaterThanOrEqual(pauses.all.first ?? 0, before.waitS)
        let found1 = try await scratch.store.usage().blobs
        XCTAssertEqual(found1, Int(total))
        // The pacer is kept where the map reads it.
        XCTAssertNotNil(host.load(namespace: PackContext.pacerNamespace, key: PackContext.pacerKey))

        // An answer the parser cannot read is not kept.
        let other = ScratchStore()
        let bad = PackTestHost { _, _ in
            tileAnswer(type: "application/json", body: Data(#"{"error": "no"}"#.utf8))
        }
        let end2 = await download(
            json, settings: settings,
            context: packContext(host: bad, store: other.store, clock: MovingClock(packNow))
        ).run()
        guard case .done(let p2) = end2 else { return XCTFail("\(end2)") }
        XCTAssertEqual(p2.done, total)
        let found2 = try await other.store.usage().blobs
        XCTAssertEqual(found2, 0)
    }

    func testWifiOnlyWaitsWhileThePathIsMetered() async throws {
        let settings = try packSettings()
        let json = try batteryChartPack(settings: settings)
        let route = RouteBox(.metered)
        let host = PackTestHost { _, _ in
            XCTAssertEqual(route.value, .unmetered, "nothing is sent on a metered path")
            return tileAnswer()
        }
        let events = EventLog()
        let end = await download(
            json, settings: settings,
            context: packContext(
                host: host, store: scratch.store, clock: MovingClock(packNow), route: route,
                onPause: { _ in route.set(.unmetered) }),
            events: events
        ).run()
        guard case .done(let progress) = end else { return XCTFail("\(end)") }
        XCTAssertEqual(progress.failed, 0)
        XCTAssertEqual(events.waits.first, true)
        XCTAssertEqual(events.waits.last, false)

        // Without "Wi-Fi only" a metered path is used at once.
        let metered = RouteBox(.metered)
        let other = ScratchStore()
        let events2 = EventLog()
        let host2 = PackTestHost { _, _ in tileAnswer() }
        let end2 = await download(
            json, settings: settings,
            context: packContext(
                host: host2, store: other.store, clock: MovingClock(packNow), route: metered),
            wifiOnly: false, events: events2
        ).run()
        guard case .done = end2 else { return XCTFail("\(end2)") }
        XCTAssertTrue(events2.waits.isEmpty)
        XCTAssertFalse(host2.urls.isEmpty)
    }

    func testTurningWifiOnlyOffLetsAWaitingDownloadGo() async throws {
        let settings = try packSettings()
        let json = try batteryChartPack(settings: settings)
        let flag = PackFlag(true)
        let host = PackTestHost { _, _ in
            XCTAssertFalse(flag.value, "nothing is sent while it waits for Wi-Fi")
            return tileAnswer()
        }
        let events = EventLog()
        let end = await PackDownload(
            packJSON: json, settingsJSON: settings, wifiOnly: flag,
            from: PackProgress(startedAt: packNow, plannedAt: packNow),
            context: packContext(
                host: host, store: scratch.store, clock: MovingClock(packNow),
                route: RouteBox(.metered), onPause: { _ in flag.value = false }),
            report: { events.add($0) }
        ).run()
        guard case .done(let progress) = end else { return XCTFail("\(end)") }
        XCTAssertEqual(progress.failed, 0)
        XCTAssertEqual(events.waits.first, true)
        XCTAssertEqual(events.waits.last, false)
    }

    func testAnInterruptedDownloadResumesWhereItStopped() async throws {
        let settings = try packSettings()
        let json = try batteryChartPack(settings: settings)
        let total = try plannedItems(json, settings: settings)
        XCTAssertGreaterThan(total, 6)
        // Three items a page; the host answers five, then hangs until the task is cancelled.
        let served = ManagedCount()
        let host = PackTestHost { _, _ in
            if served.next() >= 5 {
                try await Task.sleep(for: .seconds(60))
            }
            return tileAnswer()
        }
        let events = EventLog()
        let first = download(
            json, settings: settings,
            context: packContext(host: host, store: scratch.store, clock: MovingClock(packNow)),
            pageSize: 3, events: events)
        let running = Task { await first.run() }
        for _ in 0..<400 {
            if try await scratch.store.usage().tiles >= 5 { break }
            try await Task.sleep(for: .milliseconds(5))
        }
        running.cancel()
        let end = await running.value
        guard case .stopped = end else { return XCTFail("\(end)") }
        // Where it stopped is the last page done: the first, three items.
        let kept = try XCTUnwrap(events.committed.last)
        XCTAssertEqual(kept.nextIndex, 3)
        XCTAssertEqual(kept.done, 3)

        // A new launch: the download resumes from what was kept. The two items stored in the
        // unfinished page are not asked for again.
        let again = PackTestHost { _, _ in tileAnswer() }
        let resumed = await download(
            json, settings: settings,
            context: packContext(host: again, store: scratch.store, clock: MovingClock(packNow + 60)),
            from: kept, pageSize: 3
        ).run()
        guard case .done(let progress) = resumed else { return XCTFail("\(resumed)") }
        XCTAssertEqual(progress.done, total)
        XCTAssertEqual(progress.failed, 0)
        XCTAssertEqual(again.urls.count, Int(total) - 5)
        XCTAssertEqual(Set(host.urls.prefix(5)).intersection(again.urls), [])
        let usage = try await scratch.store.usage(of: "battery-chart")
        XCTAssertEqual(usage.stored, Int(total))
    }

    func testARefreshReplacesWhatThePackHoldsOnlyWhenItCompletes() async throws {
        let settings = try packSettings()
        let json = try batteryChartPack(settings: settings)
        let total = try plannedItems(json, settings: settings)
        let host = PackTestHost { _, _ in tileAnswer() }
        let clock = MovingClock(packNow)
        let first = await download(
            json, settings: settings,
            context: packContext(host: host, store: scratch.store, clock: clock)
        ).run()
        guard case .done = first else { return XCTFail("\(first)") }

        // A refresh a day later that fails at the transport: the pack keeps what it held.
        let refreshed = try packMarkRefreshed(packJson: json, now: packNow)
        let down = PackTestHost { _, _ in throw HostError.Failed(message: "offline") }
        var progress = PackProgress(
            startedAt: packNow + 86_400, plannedAt: packNow + 86_400, staging: true)
        progress.startedAt = packNow + 86_400
        let failed = await download(
            refreshed, settings: settings,
            context: packContext(host: down, store: scratch.store, clock: MovingClock(packNow + 86_400)),
            from: progress
        ).run()
        guard case .done(let p) = failed else { return XCTFail("\(failed)") }
        XCTAssertEqual(p.failed, total)
        let held = try await scratch.store.usage(of: "battery-chart")
        XCTAssertEqual(held.stored, Int(total))
        let found3 = try await scratch.store.packs()
        XCTAssertEqual(found3, ["battery-chart"])

        // One that succeeds: the pack holds the new copies under its own pin only.
        let good = await download(
            refreshed, settings: settings,
            context: packContext(host: host, store: scratch.store, clock: MovingClock(packNow + 90_000)),
            from: PackProgress(startedAt: packNow + 90_000, plannedAt: packNow + 90_000, staging: true)
        ).run()
        guard case .done = good else { return XCTFail("\(good)") }
        let found4 = try await scratch.store.packs()
        XCTAssertEqual(found4, ["battery-chart"])
        let tile = try XCTUnwrap(
            OfflineTileKey(text: try XCTUnwrap(
                packRequests(
                    plan: planPack(
                        packJson: json, settingsJson: settings, measuredSizes: [:],
                        timeAxes: [:], now: packNow
                    ).plan, from: 0, count: 1
                ).first?.key)))
        let times = try await scratch.store.times(of: .tile(tile))
        XCTAssertEqual(times?.fetchedAt, packNow + 90_000)
    }
}

/// A counter the host's answers share.
final class ManagedCount: @unchecked Sendable {
    private let lock = NSLock()
    private var n = 0
    /// The count before this call.
    func next() -> Int {
        lock.withLock {
            defer { n += 1 }
            return n
        }
    }
}
