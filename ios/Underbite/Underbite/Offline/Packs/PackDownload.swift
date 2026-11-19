import Foundation
import UnderbiteKit

/// What a pack download reports as it goes.
nonisolated enum PackEvent: Sendable {
    /// After each item: the counts so far (shown, not kept).
    case progress(PackProgress)
    /// After each page: where the download stands, to keep (it resumes from there).
    case committed(PackProgress)
    /// The path does not allow sending (no route, or "Wi-Fi only" on a metered path), or
    /// allows it again.
    case waitingForWifi(Bool)
}

/// How a download ended.
nonisolated enum PackRunEnd: Equatable, Sendable {
    /// Every item was handled; some may have failed.
    case done(PackProgress)
    /// Paused, cancelled or cut short by the system: it resumes from this.
    case stopped(PackProgress)
    /// The pack could not be planned (the core's words).
    case failed(String)
}

/// What a download uses from outside, so that tests stand in for each.
nonisolated struct PackContext: Sendable {
    /// The app's fetch host: every request goes through it.
    let host: FetchHost
    /// The device's store (``MapOffline/store`` in the app).
    let store: OfflineStore
    /// ERDDAP's limits per server, shared with the map through the fetch host's store.
    let pacer: GridPacer
    /// Unix seconds.
    let clock: @Sendable () -> Int64
    /// Waits this many seconds (ends early when the task is cancelled).
    let pause: @Sendable (UInt32) async -> Void
    /// The path to the network now.
    let route: @Sendable () -> PackRoute

    /// Where the map keeps the pacer in the fetch host's store; the packs use the same, so a
    /// server found busy by either waits for both.
    static let pacerNamespace = "map"
    static let pacerKey = "pacer"

    /// The pacer as last saved by the map or a pack.
    static func savedPacer(_ host: FetchHost) -> GridPacer {
        host.load(namespace: pacerNamespace, key: pacerKey)
            .flatMap { try? GridPacer.fromJson(json: $0) } ?? GridPacer()
    }

    func savePacer() {
        host.store(namespace: Self.pacerNamespace, key: Self.pacerKey, value: pacer.toJson())
    }
}

/// One download of a pack: its plan read a page at a time from `pack_requests`, each request
/// sent through the fetch host (at most ``PackRules/perHost`` at once to a host; grid and depth
/// requests let through by the ERDDAP pacer), each answer checked by the core before it is
/// stored (`classify_tile` for a tile, the grid store's parser for a grid block or a depth
/// area), so an error page never becomes pack data, and every item pinned to the pack.
///
/// A failed request is asked again after a back-off, up to ``PackRules/attempts`` tries; a
/// server asking to slow down is waited for. Without a usable path (none, or a metered one with
/// "Wi-Fi only") nothing is sent until there is one. Cancelling the task stops the download:
/// what it stored stays pinned, and a download started again from the last ``PackEvent/committed``
/// progress skips what it already stored.
///
/// A refresh (`staging`) pins its items apart; when it completes, the pack's pin moves to them
/// and what only the old download held is dropped.
actor PackDownload {
    private let packJSON: String
    private let settingsJSON: String
    private let measured: [String: UInt64]
    private let wifiOnly: PackFlag
    private let context: PackContext
    private let report: @Sendable (PackEvent) -> Void
    private let pageSize: UInt32
    private var progress: PackProgress

    /// - Parameters:
    ///   - measured: average bytes per item by layer, for the plan's estimate.
    ///   - wifiOnly: the "Wi-Fi only" choice, read before each request (it may change while
    ///     the download runs).
    ///   - progress: where to start: a new download's, or where one stopped.
    ///   - pageSize: items read from the plan at a time (tests use small pages).
    init(
        packJSON: String, settingsJSON: String, measured: [String: UInt64] = [:],
        wifiOnly: PackFlag, from progress: PackProgress, context: PackContext,
        pageSize: UInt32 = PackRules.pageSize,
        report: @escaping @Sendable (PackEvent) -> Void = { _ in }
    ) {
        self.pageSize = max(1, pageSize)
        self.packJSON = packJSON
        self.settingsJSON = settingsJSON
        self.measured = measured
        self.wifiOnly = wifiOnly
        self.progress = progress
        self.context = context
        self.report = report
    }

    /// Runs the download to its end, or until the task is cancelled.
    func run() async -> PackRunEnd {
        let plan: PackPlanInfo
        do {
            plan = try planPack(
                packJson: packJSON, settingsJson: settingsJSON, measuredSizes: measured,
                timeAxes: progress.axes, now: progress.plannedAt)
        } catch {
            return .failed(PackErrors.message(of: error))
        }
        guard let pack = PackInfo(json: packJSON) else { return .failed("") }
        if plan.overBudget { return .failed(plan.overBudgetText ?? plan.summaryText) }
        progress.total = plan.itemCount
        let pin = PackRules.pin(pack.id, staging: progress.staging)
        report(.progress(progress))
        while progress.nextIndex < progress.total {
            if Task.isCancelled { return .stopped(progress) }
            let page = packRequests(
                plan: plan.plan, from: progress.nextIndex, count: pageSize)
            if page.isEmpty { break }
            do {
                try await context.store.pin(pin, page.compactMap(PackRules.storeKey))
            } catch {
                return .failed(PackErrors.message(of: error))
            }
            guard let counted = await run(page) else {
                context.savePacer()
                return .stopped(progress)
            }
            progress = counted
            progress.nextIndex += UInt64(page.count)
            context.savePacer()
            report(.committed(progress))
        }
        if progress.staging {
            do {
                if progress.total > 0, progress.failed == progress.total {
                    // Nothing new came: the pack keeps exactly what it held.
                    try await context.store.unpin(pin)
                } else {
                    try await moveStagedPin(pack.id, plan: plan.plan)
                }
            } catch {
                return .failed(PackErrors.message(of: error))
            }
        }
        return .done(progress)
    }

    /// After a refresh: the pack's pin moves to the refreshed items, dropping rows only the old
    /// download held, then the separate pin goes.
    private func moveStagedPin(_ id: String, plan: PackPlanHandle) async throws {
        let store = context.store
        try await store.unpin(id)
        var from: UInt64 = 0
        let total = plan.itemCount()
        while from < total {
            let page = packRequests(plan: plan, from: from, count: pageSize)
            if page.isEmpty { break }
            try await store.pin(id, page.compactMap(PackRules.storeKey))
            from += UInt64(page.count)
        }
        try await store.unpin(PackRules.pin(id, staging: true))
    }

    /// The items of one page, side by side within the limits; the counts after it, or none
    /// when the task was cancelled before every item was handled.
    private func run(_ page: [PackRequest]) async -> PackProgress? {
        var live = progress
        let (context, wifiOnly, startedAt, report) = (
            context, wifiOnly, progress.startedAt, report
        )
        return await withTaskGroup(of: ItemDone.self) { group -> PackProgress? in
            var pending = Array(page.indices)
            var slots = PackSlots()
            var stopped = false
            while true {
                while !stopped, !Task.isCancelled,
                    let pos = slots.firstWithRoom(pending.map { page[$0].host })
                {
                    let index = pending.remove(at: pos)
                    let item = page[index]
                    _ = slots.take(item.host)
                    group.addTask {
                        await Self.fetch(
                            item, index: index, context: context, wifiOnly: wifiOnly,
                            startedAt: startedAt, report: report)
                    }
                }
                guard let done = await group.next() else { break }
                let item = page[done.index]
                slots.release(item.host)
                live.throttled += done.throttled
                switch done.result {
                case .stored(let bytes):
                    live.done += 1
                    live.bytes += UInt64(bytes)
                    live.layerBytes[item.layer, default: 0] += UInt64(bytes)
                    live.layerItems[item.layer, default: 0] += 1
                case .kept(let bytes):
                    live.done += 1
                    live.bytes += UInt64(bytes)
                case .empty:
                    live.done += 1
                case .failed:
                    live.done += 1
                    live.failed += 1
                case .stopped:
                    stopped = true
                }
                if !stopped { report(.progress(live)) }
            }
            return stopped || Task.isCancelled || !pending.isEmpty ? nil : live
        }
    }

    // MARK: - One item

    nonisolated struct ItemDone: Sendable {
        let index: Int
        let result: ItemResult
        /// Answers asking to slow down.
        let throttled: UInt64
    }

    nonisolated enum ItemResult: Equatable, Sendable {
        /// Checked and stored: this many bytes.
        case stored(Int)
        /// Stored earlier in this download (before an interruption).
        case kept(Int)
        /// The source has nothing there (a cloudy day, outside its grid).
        case empty
        /// No good answer after every try.
        case failed
        /// The task was cancelled.
        case stopped
    }

    /// Fetches, checks and stores one item.
    nonisolated static func fetch(
        _ item: PackRequest, index: Int, context c: PackContext, wifiOnly: PackFlag,
        startedAt: Int64, report: @Sendable (PackEvent) -> Void
    ) async -> ItemDone {
        var throttled: UInt64 = 0
        func end(_ result: ItemResult) -> ItemDone {
            ItemDone(index: index, result: result, throttled: throttled)
        }
        guard let request = item.request, let key = PackRules.storeKey(item) else {
            return end(.failed)
        }
        if let kept = try? await c.store.times(of: key), kept.fetchedAt >= startedAt {
            let record = try? await c.store.get(key, now: c.clock(), allowingStale: true)
            return end(.kept(record?.data.count ?? 0))
        }
        let paced = item.kind != .tile
        var tries: UInt32 = 0
        while true {
            guard await waitForRoute(c, wifiOnly: wifiOnly, report: report) else {
                return end(.stopped)
            }
            if paced {
                guard await waitForPacer(item.host, c) else { return end(.stopped) }
            }
            let response: HostResponse
            do {
                response = try await c.host.send(request: request)
            } catch {
                if Task.isCancelled { return end(.stopped) }
                // Lost the route: wait for it again, not a try.
                if c.route() == .none { continue }
                tries += 1
                if tries >= PackRules.attempts { return end(.failed) }
                await c.pause(PackRules.backOff(after: tries))
                continue
            }
            if Task.isCancelled { return end(.stopped) }
            if response.status == 429 || response.status == 503 { throttled += 1 }
            let now = c.clock()
            let step: Step
            switch item.kind {
            case .tile:
                step = await storeTile(item, request, key, response, tries: tries, c, now: now)
            case .grid, .dem:
                step = await storeGrid(item, request, key, response, c, now: now)
                if case .wait = step, !(response.status == 429 || response.status == 503) {
                    throttled += 1
                }
            }
            switch step {
            case .done(let result):
                return end(result)
            case .wait(let seconds):
                tries += 1
                if tries >= PackRules.attempts { return end(.failed) }
                await c.pause(PackRules.serverWait(seconds))
            }
        }
    }

    private enum Step {
        case done(ItemResult)
        /// The server asked to slow down: ask again after this many seconds.
        case wait(UInt32)
    }

    /// A tile answer: an image the core accepts is stored with the request's lifetime.
    private static func storeTile(
        _ item: PackRequest, _ request: HostRequest, _ key: OfflineKey,
        _ response: HostResponse, tries: UInt32, _ c: PackContext, now: Int64
    ) async -> Step {
        let outcome =
            (try? classifyTile(
                layer: item.layer, status: response.status, contentType: response.contentType,
                bodyLen: UInt64(response.body.count), attempt: tries)) ?? .failed
        switch outcome {
        case .image:
            let entry = OfflineEntry(
                data: response.body, contentType: response.contentType, etag: response.etag,
                fetchedAt: now, lifetime: UInt64(request.maxAgeS ?? 0))
            let stored = (try? await c.store.put(key, entry, now: now)) == true
            return .done(stored ? .stored(response.body.count) : .failed)
        case .noData, .tooDeep:
            return .done(.empty)
        case .retry(let after):
            return .wait(after)
        case .failed:
            return .done(.failed)
        }
    }

    /// A grid block or depth area: stored once the core's parser reads it; the pacer hears
    /// every answer.
    private static func storeGrid(
        _ item: PackRequest, _ request: HostRequest, _ key: OfflineKey,
        _ response: HostResponse, _ c: PackContext, now: Int64
    ) async -> Step {
        try? c.pacer.record(
            node: item.host, status: response.status,
            retryAfterS: response.retryAfter.flatMap {
                UInt32($0.trimmingCharacters(in: .whitespaces))
            }, now: now)
        do {
            // A store of its own, dropped after the check: the pack keeps the body.
            let check = GridStore()
            if item.kind == .dem {
                try check.addDem(key: item.key, response: response, now: now)
            } else {
                try check.addGrid(key: item.key, response: response, now: now)
            }
        } catch MapError.NoData {
            return .done(.empty)
        } catch MapError.RetryLater(let after) {
            return .wait(after)
        } catch {
            return .done(.failed)
        }
        let entry = OfflineEntry(
            data: response.body, contentType: response.contentType, fetchedAt: now,
            lifetime: UInt64(request.maxAgeS ?? 0))
        let stored = (try? await c.store.put(key, entry, now: now)) == true
        return .done(stored ? .stored(response.body.count) : .failed)
    }

    /// Waits until the path lets the download send; false when the task was cancelled.
    private static func waitForRoute(
        _ c: PackContext, wifiOnly: PackFlag, report: @Sendable (PackEvent) -> Void
    ) async -> Bool {
        var waiting = false
        while !PackRules.maySend(on: c.route(), wifiOnly: wifiOnly.value) {
            if Task.isCancelled { return false }
            if !waiting {
                waiting = true
                report(.waitingForWifi(true))
            }
            await c.pause(PackRules.routeCheck)
        }
        if waiting { report(.waitingForWifi(false)) }
        return !Task.isCancelled
    }

    /// Waits until the pacer lets a request go to `node`; false when the task was cancelled.
    private static func waitForPacer(_ node: String, _ c: PackContext) async -> Bool {
        while !Task.isCancelled {
            guard let decision = try? c.pacer.maySend(node: node, now: c.clock()) else {
                return true
            }
            if decision.send { return true }
            await c.pause(max(1, decision.waitS))
        }
        return false
    }
}
