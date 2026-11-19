import BackgroundTasks
import Foundation
import OSLog
import UnderbiteKit

/// Pack downloads outside the foreground, in the ways iOS 26 allows:
///
/// - **A download someone starts** goes on in the background as a continued-processing task
///   (`BGContinuedProcessingTask`): the system shows its title and progress, and the person may
///   stop it there. When it is stopped, or the system ends it, the download pauses and resumes
///   where it stopped. Where no such task can start (the simulator, a busy system), the
///   download runs while the app is open.
/// - **Refreshes** run in a processing task (`BGProcessingTask`, its own id apart from the
///   alerts' app refresh) when iOS decides, on a network: it resumes the downloads the app was
///   stopped in the middle of and refreshes the packs the core calls stale. Nothing promises
///   when, and the packs list says so in the core's words.
enum PackBackground {
    /// The refresh task's identifier, in the Info.plist's `BGTaskSchedulerPermittedIdentifiers`.
    nonisolated static let refreshTask = "dev.savery.underbite.pack-refresh"
    /// The prefix of a download's continued-processing task: the Info.plist permits
    /// `dev.savery.underbite.pack-download.*`, and each pack's id ends it.
    nonisolated static let downloadPrefix = "dev.savery.underbite.pack-download."
    /// The soonest a refresh may run after it is asked for.
    static let refreshAfter: TimeInterval = 6 * 3_600
    /// How often a continued task's subtitle follows the progress.
    static let subtitleEvery: Duration = .seconds(2)

    nonisolated private static let log = Logger(subsystem: "dev.savery.underbite", category: "packs")

    /// The continued tasks attached to running downloads, by pack id.
    private static var attached: [String: BGContinuedProcessingTask] = [:]
    private static var subtitled: [String: ContinuousClock.Instant] = [:]
    nonisolated(unsafe) private static var registered: Set<String> = []
    nonisolated private static let registering = NSLock()

    /// At launch: the refresh task's handler, and the library's link to the spots (their
    /// metadata is fetched with the pack) and to the continued tasks' progress.
    static func register(library: PackLibrary = .shared, spots: SpotStore = .shared) {
        library.refreshSpots = { ids in await spots.refresh(only: Set(ids)) }
        library.onUpdate = { id in update(id, library: library) }
        registerRefresh()
    }

    /// Asks iOS for a refresh when there is a pack to keep current; refused requests (the
    /// simulator, background work turned off) wait for the next time the app goes to the
    /// background.
    static func schedule(library: PackLibrary = .shared) {
        guard library.records.contains(where: { $0.status == .done || $0.progress != nil }) else {
            return
        }
        let request = BGProcessingTaskRequest(identifier: refreshTask)
        request.requiresNetworkConnectivity = true
        request.requiresExternalPower = false
        request.earliestBeginDate = .now.addingTimeInterval(refreshAfter)
        try? BGTaskScheduler.shared.submit(request)
    }

    /// Starts (or resumes) the download of `id` after a tap, then asks iOS to let it go on in
    /// the background.
    static func download(
        _ id: String, settingsJSON: String, library: PackLibrary = .shared
    ) async {
        await library.download(id, settingsJSON: settingsJSON)
        guard library.isRunning(id), let record = library.record(id) else { return }
        let identifier = downloadPrefix + id
        guard registerContinued(identifier, packID: id) else { return }
        let request = BGContinuedProcessingTaskRequest(
            identifier: identifier, title: record.info?.name ?? "",
            subtitle: subtitle(id, library: library))
        // Start now or not at all: the download runs in the foreground anyway.
        request.strategy = .fail
        do {
            try BGTaskScheduler.shared.submit(request)
        } catch {
            log.info("no continued task: \(String(describing: error), privacy: .public)")
        }
    }

    // MARK: - Registration (called back on a queue of the system's)

    private nonisolated struct TaskBox: @unchecked Sendable {
        let task: BGTask
    }

    private nonisolated static func registerRefresh() {
        let ok = BGTaskScheduler.shared.register(forTaskWithIdentifier: refreshTask, using: nil) {
            @Sendable task in
            let box = TaskBox(task: task)
            Task { @MainActor in await refresh(box.task) }
        }
        if !ok { log.error("the pack refresh task is not permitted") }
    }

    /// Registers the handler of a continued task once per process (a second registration of an
    /// identifier ends the app); false when it is not permitted.
    private nonisolated static func registerContinued(_ identifier: String, packID: String) -> Bool
    {
        registering.withLock {
            if registered.contains(identifier) { return true }
            let ok = BGTaskScheduler.shared.register(
                forTaskWithIdentifier: identifier, using: nil
            ) { @Sendable task in
                let box = TaskBox(task: task)
                Task { @MainActor in await attach(box.task, packID: packID) }
            }
            if ok { registered.insert(identifier) }
            return ok
        }
    }

    // MARK: - Running

    /// A continued task started for the download of `packID`: it lasts as long as the download,
    /// shows its progress, and pauses it when the person or the system stops the task.
    private static func attach(_ task: BGTask, packID: String) async {
        guard let task = task as? BGContinuedProcessingTask else {
            task.setTaskCompleted(success: false)
            return
        }
        let library = PackLibrary.shared
        guard library.isRunning(packID) else {
            task.setTaskCompleted(success: library.record(packID)?.status == .done)
            return
        }
        task.expirationHandler = { @Sendable in
            Task { @MainActor in PackLibrary.shared.pause(packID) }
        }
        attached[packID] = task
        update(packID, library: library)
        await library.wait(packID)
        attached[packID] = nil
        subtitled[packID] = nil
        task.setTaskCompleted(success: library.record(packID)?.status == .done)
    }

    /// The system's refresh: interrupted downloads first, then the stale packs, one at a time,
    /// only on a path the Wi-Fi choice allows. When the time runs out the downloads pause.
    private static func refresh(_ task: BGTask) async {
        schedule()
        let library = PackLibrary.shared
        task.expirationHandler = { @Sendable in
            Task { @MainActor in PackLibrary.shared.stopAll() }
        }
        guard PackRules.maySend(on: PackPath.shared.current, wifiOnly: library.wifiOnly) else {
            task.setTaskCompleted(success: false)
            return
        }
        let settings = SpotStore.shared.settingsJSON
        await library.resumeInterrupted(settingsJSON: settings)
        await library.waitAll()
        for id in library.due(settingsJSON: settings, now: Int64(Date().timeIntervalSince1970)) {
            await library.download(id, settingsJSON: settings)
            await library.wait(id)
        }
        let finished = library.records.allSatisfy { $0.progress == nil }
        log.info("pack refresh done, all finished \(finished, privacy: .public)")
        task.setTaskCompleted(success: finished)
    }

    /// Follows a download's progress in its continued task.
    private static func update(_ id: String, library: PackLibrary) {
        guard let task = attached[id], let progress = library.progress(id) else { return }
        task.progress.totalUnitCount = Int64(clamping: max(progress.total, 1))
        task.progress.completedUnitCount = Int64(clamping: progress.done)
        let now = ContinuousClock.now
        if let last = subtitled[id], now - last < subtitleEvery { return }
        subtitled[id] = now
        task.updateTitle(task.title, subtitle: subtitle(id, library: library))
    }

    /// The core's words for where the download of `id` stands.
    private static func subtitle(_ id: String, library: PackLibrary) -> String {
        let settings = SpotStore.shared.settingsJSON
        guard let p = library.progress(id) else { return "" }
        return
            (try? packProgressText(
                done: p.done, total: p.total, failed: p.failed, bytes: p.bytes,
                settingsJson: settings)) ?? ""
    }
}
