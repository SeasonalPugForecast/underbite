import Foundation
import Observation
import UnderbiteKit

/// The offline packs of the app: their definitions and downloads, kept in the packs file
/// (``PacksFile``), and the downloads running. A pack is downloaded only when asked
/// (``download(_:settingsJSON:)``: a tap, or a refresh of a pack the user downloaded before).
///
/// Pause and cancel stop a download's task; a paused one resumes where it stopped. A download
/// the app was stopped in the middle of (the system ended it, or the background time ran out)
/// is paused when the app opens again and resumes by itself when the packs list is shown or
/// the background refresh runs. Cancelling a first download drops what it stored; cancelling
/// a refresh keeps what the pack held before. Deleting a pack unpins its items, and the store
/// drops those no other pack holds.
@MainActor
@Observable
final class PackLibrary {
    static let shared = PackLibrary()

    /// The packs file as last saved.
    private(set) var file: PacksFile
    /// Per pack whose download runs, the counts so far.
    private(set) var live: [String: PackProgress] = [:]
    /// Per pack, what it holds on the device, bytes (``loadSizes()``).
    private(set) var storedBytes: [String: Int64] = [:]

    /// Fetches the spots of a pack, after its download (the spot store's refresh): their
    /// snapshots and station metadata are kept with the pack.
    @ObservationIgnored var refreshSpots: ([String]) async -> Void = { _ in }
    /// Hears every change of a running download (the background task's progress).
    @ObservationIgnored var onUpdate: ((String) -> Void)?

    @ObservationIgnored private let files: AppFiles
    @ObservationIgnored private let context: () -> PackContext
    @ObservationIgnored private let clock: @Sendable () -> Int64
    @ObservationIgnored private var jobs: [String: Task<Void, Never>] = [:]
    @ObservationIgnored private var cancelling: Set<String> = []
    @ObservationIgnored private var axes: TimeAxes?
    /// "Wi-Fi only" as the running downloads read it.
    @ObservationIgnored private let wifiFlag: PackFlag

    /// - Parameters:
    ///   - files: where the packs file is.
    ///   - context: what a download uses; made for each download (the pacer as last saved).
    init(
        files: AppFiles = .standard,
        clock: @escaping @Sendable () -> Int64 = { Int64(Date().timeIntervalSince1970) },
        context: (() -> PackContext)? = nil
    ) {
        self.files = files
        self.clock = clock
        self.context = context ?? { PackLibrary.liveContext(clock: clock) }
        var file = PacksFile.load(files)
        // A download running when the app stopped stopped with it: paused, to resume.
        for i in file.packs.indices where file.packs[i].status.isRunning {
            file.packs[i].status = .paused
            file.packs[i].userPaused = false
        }
        self.file = file
        wifiFlag = PackFlag(file.wifiOnly)
    }

    /// What a download uses in the app: the fetch host, the device's store, the pacer the map
    /// saved, the device's path.
    nonisolated static func liveContext(clock: @escaping @Sendable () -> Int64) -> PackContext {
        let host = URLSessionHost(storeURL: AppFiles.standard.hostDirectory)
        return PackContext(
            host: host, store: MapOffline.store, pacer: PackContext.savedPacer(host),
            clock: clock,
            pause: { seconds in try? await Task.sleep(for: .seconds(Int(seconds))) },
            route: { PackPath.shared.current })
    }

    // MARK: - Reading

    var records: [PackRecord] { file.packs }

    func record(_ id: String) -> PackRecord? {
        file.packs.first { $0.id == id }
    }

    /// Downloads wait while the path is expensive or constrained; on by default. A running
    /// download follows a change at its next request.
    var wifiOnly: Bool {
        get { file.wifiOnly }
        set {
            guard newValue != file.wifiOnly else { return }
            file.wifiOnly = newValue
            wifiFlag.value = newValue
            save()
        }
    }

    /// The counts to show for `id`: the running download's, else where it stopped.
    func progress(_ id: String) -> PackProgress? {
        live[id] ?? record(id)?.progress
    }

    /// Whether a download of `id` is running in this process.
    func isRunning(_ id: String) -> Bool { jobs[id] != nil }

    /// Reads what each pack holds in the store.
    func loadSizes() async {
        let store = context().store
        var sizes: [String: Int64] = [:]
        for record in file.packs {
            sizes[record.id] = (try? await store.usage(of: record.id))?.bytes ?? 0
        }
        storedBytes = sizes
    }

    // MARK: - Planning

    /// The times of the gridded layers of `packJSON` a plan needs: none for a pack of the
    /// newest data, which holds each layer's newest time.
    func timeAxes(for packJSON: String, settingsJSON: String) async -> [String: [Int64]] {
        guard let info = PackInfo(json: packJSON), !info.isLatest,
            let catalogue = try? mapCatalogue(settingsJson: settingsJSON)
        else { return [:] }
        let gridded = catalogue.filter {
            info.layers.contains($0.id) && ($0.draw == .gridRaster || $0.draw == .gridArrows)
        }
        guard !gridded.isEmpty else { return [:] }
        let axes = self.axes ?? TimeAxes(host: context().host, clock: clock)
        self.axes = axes
        var out: [String: [Int64]] = [:]
        for layer in gridded {
            let times = await axes.times(
                TimeAxes.Key(layer: layer.id, dataset: nil), dates: layer.dates,
                settingsJSON: settingsJSON, timeZone: "UTC")
            if let times { out[layer.id] = times.map(\.time) }
        }
        return out
    }

    /// The plan of `packJSON` now, as the estimate before a download shows it.
    func estimate(_ packJSON: String, settingsJSON: String) async -> Result<PackPlanInfo, PackPlanProblem> {
        let axes = await timeAxes(for: packJSON, settingsJSON: settingsJSON)
        let (measured, now) = (file.measured, clock())
        return await Task.detached(priority: .userInitiated) {
            do {
                return .success(
                    try planPack(
                        packJson: packJSON, settingsJson: settingsJSON, measuredSizes: measured,
                        timeAxes: axes, now: now))
            } catch {
                return .failure(PackPlanProblem(message: PackErrors.message(of: error)))
            }
        }.value
    }

    // MARK: - Changing

    /// Keeps a new pack (the core's JSON, from `pack_new`); returns its id.
    @discardableResult
    func add(_ json: String) -> String? {
        guard let info = PackInfo(json: json), record(info.id) == nil else { return nil }
        file.packs.append(PackRecord(json: json))
        save()
        return info.id
    }

    /// Downloads `id`: resumes where it stopped, or starts a download of the pack as planned
    /// now (a refresh when it was downloaded before).
    func download(_ id: String, settingsJSON: String) async {
        guard jobs[id] == nil, let record = record(id), let info = record.info else { return }
        let now = clock()
        var progress: PackProgress
        if let stopped = record.progress {
            progress = stopped
        } else {
            progress = PackProgress(
                startedAt: now, plannedAt: now,
                axes: await timeAxes(for: record.json, settingsJSON: settingsJSON),
                staging: info.refreshedAt != nil)
        }
        guard jobs[id] == nil else { return }
        if progress.startedAt > now { progress.startedAt = now }
        change(id) {
            $0.status = .downloading
            $0.progress = progress
            $0.userPaused = false
            $0.problem = nil
        }
        live[id] = progress
        start(id, record.json, progress: progress, settingsJSON: settingsJSON)
    }

    private func start(_ id: String, _ json: String, progress: PackProgress, settingsJSON: String) {
        let (stream, events) = AsyncStream.makeStream(of: PackEvent.self)
        let download = PackDownload(
            packJSON: json, settingsJSON: settingsJSON, measured: file.measured,
            wifiOnly: wifiFlag, from: progress, context: context(),
            report: { events.yield($0) })
        jobs[id] = Task { [weak self] in
            let listening = Task { @MainActor [weak self] in
                for await event in stream { self?.handle(event, for: id) }
            }
            let end = await download.run()
            events.finish()
            await listening.value
            await self?.finish(id, end)
        }
    }

    private func handle(_ event: PackEvent, for id: String) {
        switch event {
        case .progress(let progress):
            live[id] = progress
        case .committed(let progress):
            live[id] = progress
            change(id) { $0.progress = progress }
        case .waitingForWifi(let waiting):
            change(id, saving: false) {
                guard $0.status.isRunning else { return }
                $0.status = waiting ? .waitingForWifi : .downloading
            }
        }
        onUpdate?(id)
    }

    private func finish(_ id: String, _ end: PackRunEnd) async {
        jobs[id] = nil
        live[id] = nil
        let cancelled = cancelling.remove(id) != nil
        guard let record = record(id) else { return }
        switch end {
        case .done(let progress):
            let allFailed = progress.total > 0 && progress.failed == progress.total
            // Nothing came: the pack keeps its last refresh (and stays due for another).
            let json =
                allFailed
                ? record.json
                : (try? packMarkRefreshed(packJson: record.json, now: clock())) ?? record.json
            change(id) {
                $0.json = json
                $0.status = allFailed ? .failed : .done
                $0.progress = nil
                $0.lastDone = progress.done
                $0.lastFailed = progress.failed
            }
            file.measure(progress)
            save()
            await loadSizes()
            onUpdate?(id)
            if let spots = record.info?.spots, !spots.isEmpty { await refreshSpots(spots) }
            return
        case .stopped:
            if cancelled {
                await dropDownload(id)
            } else {
                change(id) { $0.status = .paused }
            }
        case .failed(let message):
            change(id) {
                $0.status = .failed
                $0.problem = message
                $0.progress = nil
            }
        }
        await loadSizes()
        onUpdate?(id)
    }

    /// Pauses the download of `id`; it resumes where it stopped.
    func pause(_ id: String) {
        guard let job = jobs[id] else { return }
        change(id) { $0.userPaused = true }
        job.cancel()
    }

    /// Stops every download (the background time ran out); each resumes where it stopped.
    func stopAll() {
        for job in jobs.values { job.cancel() }
    }

    /// Cancels the download of `id`: a first download's items are dropped; a refresh's, and
    /// the pack keeps what it held.
    func cancel(_ id: String) async {
        if let job = jobs[id] {
            cancelling.insert(id)
            job.cancel()
            await job.value
        } else {
            await dropDownload(id)
            await loadSizes()
        }
    }

    private func dropDownload(_ id: String) async {
        guard let record = record(id) else { return }
        let store = context().store
        let staging = record.progress?.staging ?? false
        _ = try? await store.unpin(PackRules.pin(id, staging: staging))
        let held = record.info?.refreshedAt != nil
        change(id) {
            $0.progress = nil
            $0.userPaused = false
            $0.status = held ? .done : .cancelled
        }
    }

    /// Deletes `id`: its download stops and the store drops what no other pack holds.
    func delete(_ id: String) async {
        if let job = jobs[id] {
            cancelling.insert(id)
            job.cancel()
            await job.value
        }
        let store = context().store
        _ = try? await store.unpin(PackRules.pin(id, staging: true))
        _ = try? await store.unpin(id)
        file.packs.removeAll { $0.id == id }
        storedBytes[id] = nil
        save()
    }

    /// Resumes the downloads the app was stopped in the middle of (not those paused by a tap).
    func resumeInterrupted(settingsJSON: String) async {
        for record in file.packs
        where record.status == .paused && !record.userPaused && record.progress != nil {
            await download(record.id, settingsJSON: settingsJSON)
        }
    }

    /// The packs a refresh is due for at `now`: downloaded, not running, and stale by the
    /// core's rule (`pack_row`).
    func due(settingsJSON: String, now: Int64) -> [String] {
        file.packs.compactMap { record in
            guard jobs[record.id] == nil, record.progress == nil,
                record.info?.refreshedAt != nil,
                record.status == .done || record.status == .failed,
                let row = try? packRow(
                    packJson: record.json, storedBytes: 0, now: now, settingsJson: settingsJSON,
                    timeZone: "UTC"),
                row.stale
            else { return nil }
            return record.id
        }
    }

    /// Waits for the download of `id`, if one runs.
    func wait(_ id: String) async {
        await jobs[id]?.value
    }

    /// Waits for every download running.
    func waitAll() async {
        while let job = jobs.values.first { await job.value }
    }

    // MARK: - The file

    private func change(_ id: String, saving: Bool = true, _ edit: (inout PackRecord) -> Void) {
        guard let i = file.packs.firstIndex(where: { $0.id == id }) else { return }
        var record = file.packs[i]
        edit(&record)
        guard record != file.packs[i] else { return }
        file.packs[i] = record
        if saving { save() }
    }

    private func save() {
        file.save(files)
    }
}

/// Why a pack cannot be planned, in the core's words.
nonisolated struct PackPlanProblem: Error, Equatable, Sendable {
    let message: String
}
