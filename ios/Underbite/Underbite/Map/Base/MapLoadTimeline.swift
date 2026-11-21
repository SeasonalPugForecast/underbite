import Foundation
import os

/// What the map's loading passes through, in the order a launch meets them.
nonisolated enum MapLoadPhase: String, Sendable, CaseIterable {
    /// MapKit finished loading the base map's own data.
    case baseMap = "base-map"
    /// MapKit drew the base map in full.
    case baseRendered = "base-rendered"
    /// The map first settled on a region (the layers start loading).
    case regionSettled = "region"
    /// A grid layer's time axis was asked for.
    case axisAsked = "axis-asked"
    /// A grid layer's time axis was served: `memory`, `store`, `network` or `none`.
    case axis
    /// A grid layer's first block: `cache`, `cache-stale` or `network`.
    case block
    /// A tiled layer's first tile: `memory`, `store`, `store-stale` or `network`.
    case tile
}

/// The first time each milestone of a map launch was reached, as milliseconds since the launch.
/// Pure: the time is passed in, so a test sets it. A milestone is one phase of one layer (a
/// phase without a layer has the empty one); only the first of each is kept.
nonisolated struct MapLoadTimeline: Equatable, Sendable {
    struct Milestone: Equatable, Sendable {
        let phase: MapLoadPhase
        let layer: String
        let source: String
        let ms: Int
    }

    /// The launch, in seconds on whatever clock the caller reads.
    let start: Double
    private(set) var milestones: [Milestone] = []

    init(start: Double) {
        self.start = start
    }

    /// Notes that `phase` of `layer` was reached at `now` (seconds). Returns the milestone when
    /// it is the first of its phase and layer, none when it was reached before.
    @discardableResult
    mutating func note(
        _ phase: MapLoadPhase, layer: String = "", source: String = "", at now: Double
    ) -> Milestone? {
        guard milestone(phase, layer: layer) == nil else { return nil }
        let ms = max(0, Int(((now - start) * 1000).rounded()))
        let found = Milestone(phase: phase, layer: layer, source: source, ms: ms)
        milestones.append(found)
        return found
    }

    func milestone(_ phase: MapLoadPhase, layer: String = "") -> Milestone? {
        milestones.first { $0.phase == phase && $0.layer == layer }
    }

    /// How long `layer`'s axis took from asked for to served, in milliseconds.
    func axisWait(layer: String) -> Int? {
        guard let asked = milestone(.axisAsked, layer: layer),
            let served = milestone(.axis, layer: layer)
        else { return nil }
        return served.ms - asked.ms
    }

    /// The milestones in the order reached, one line: `phase[layer]=source@ms`.
    func summary() -> String {
        milestones.map { m in
            var text = m.phase.rawValue
            if !m.layer.isEmpty { text += "[\(m.layer)]" }
            if !m.source.isEmpty { text += "=\(m.source)" }
            return text + "@\(m.ms)"
        }.joined(separator: " ")
    }
}

/// An interval of a signposter, ended once.
nonisolated struct MapLoadInterval: Sendable {
    fileprivate let name: StaticString
    fileprivate let state: OSSignpostIntervalState
    fileprivate let signposter: OSSignposter

    func end() {
        signposter.endInterval(name, state)
    }
}

/// The map's load measurements: the milestones of the launch (``MapLoadTimeline``), each logged
/// as it is first reached under category `map-load`, signposted intervals for the waits, and a
/// summary line once the loading is quiet or after a minute. It logs layer ids, sources and
/// milliseconds only: never a coordinate or a URL.
nonisolated final class MapLoad: Sendable {
    static let subsystem = "dev.savery.underbite"
    static let category = "map-load"
    /// Seconds without a new milestone after which the loading counts as settled.
    static let quiet: Double = 10
    /// Seconds after which the summary is printed whatever is still loading.
    static let limit: Double = 60

    /// The map's one recorder; its launch is the first time it is touched.
    static let shared = MapLoad(timers: true)

    private struct State {
        var timeline: MapLoadTimeline
        var summarized = false
        var generation = 0
    }

    private let state: OSAllocatedUnfairLock<State>
    private let clock: @Sendable () -> Double
    private let emit: @Sendable (String) -> Void
    private let timers: Bool
    private let signposter = OSSignposter(subsystem: MapLoad.subsystem, category: MapLoad.category)

    /// - Parameters:
    ///   - clock: seconds, steady (tests set it).
    ///   - emit: takes each log line (tests collect them).
    ///   - timers: whether the summary comes by itself (quiet, or the limit).
    init(
        clock: @escaping @Sendable () -> Double = { ProcessInfo.processInfo.systemUptime },
        emit: (@Sendable (String) -> Void)? = nil, timers: Bool = false
    ) {
        self.clock = clock
        self.emit =
            emit ?? { line in
                Logger(subsystem: MapLoad.subsystem, category: MapLoad.category)
                    .notice("\(line, privacy: .public)")
            }
        self.timers = timers
        state = OSAllocatedUnfairLock(initialState: State(timeline: MapLoadTimeline(start: clock())))
        if timers { wait(Self.limit) { $0.finish() } }
    }

    /// Starts the recorder (its launch is when it is first touched).
    func begin() {}

    /// Notes a milestone, logging it when it is the first of its phase and layer.
    func mark(_ phase: MapLoadPhase, layer: String = "", source: String = "") {
        let now = clock()
        let found = state.withLock { state -> (MapLoadTimeline.Milestone, Int)? in
            guard !state.summarized,
                let found = state.timeline.note(phase, layer: layer, source: source, at: now)
            else { return nil }
            state.generation += 1
            return (found, state.generation)
        }
        guard let (milestone, generation) = found else { return }
        var line = "milestone phase=\(phase.rawValue)"
        if !layer.isEmpty { line += " layer=\(layer)" }
        if !source.isEmpty { line += " source=\(source)" }
        emit(line + " ms=\(milestone.ms)")
        if timers {
            wait(Self.quiet) { load in
                if load.state.withLock({ $0.generation }) == generation { load.finish() }
            }
        }
    }

    /// Starts a signposted interval named `name` (a wait of `layer`, or of the whole map).
    func interval(_ name: StaticString, layer: String = "") -> MapLoadInterval {
        let state = signposter.beginInterval(
            name, id: signposter.makeSignpostID(), "\(layer, privacy: .public)")
        return MapLoadInterval(name: name, state: state, signposter: signposter)
    }

    /// Logs the summary line, once.
    func finish() {
        let line = state.withLock { state -> String? in
            guard !state.summarized else { return nil }
            state.summarized = true
            return state.timeline.summary()
        }
        if let line { emit("summary \(line)") }
    }

    private func wait(_ seconds: Double, then act: @escaping @Sendable (MapLoad) -> Void) {
        Task { [weak self] in
            try? await Task.sleep(for: .seconds(seconds))
            if let self { act(self) }
        }
    }
}
