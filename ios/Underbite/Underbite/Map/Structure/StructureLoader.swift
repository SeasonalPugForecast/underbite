import Foundation
@preconcurrency import UnderbiteKit

/// Fetches what the structure layers draw for the view on screen, through the app's fetch host
/// and the core's requests and parsers only:
///
/// - **Charted points** from ENC Direct to GIS: the scale band for the zoom
///   (`structure_band`), the band's layer ids read from its own layer list
///   (`parse_structure_layers`, kept for the session), then the wrecks, obstructions and rocks
///   of the box, a few pages of each (`structure_points_request`, `parse_structure_points`).
/// - **Lake contours** of each state layer shown, a few pages for the box and zoom
///   (`state_contour_request`, `parse_state_contours`); none outside the state's data.
/// - **New York's artificial reefs**, one list (`reefs_request`, `parse_reefs`).
///
/// Points and contours are asked by box (``StructureBox``): a box fetched lately that covers the
/// view serves it again without a request, a box being fetched is not asked twice, and a box
/// whose fetch failed is not asked again for a minute. The core says when a failure is worth a
/// retry (ENC Direct's transient error bodies, a busy server) and after how long; a retry asks
/// past the cache, which may hold the error body; once a minute has passed, the latest view is
/// asked for again by itself. The latest view decides what is shown; a box fetched for an
/// earlier one is still kept.
@MainActor
final class StructureLoader {
    /// What the layers on screen need.
    nonisolated struct Wants: Equatable, Sendable {
        /// The charted points.
        var points = false
        /// New York's reefs.
        var reefs = false
        /// The lake contour layers.
        var contours: [String] = []

        var any: Bool { points || reefs || !contours.isEmpty }
    }

    typealias Pause = @Sendable (Duration) async -> Void

    /// The most pages of one kind of point, or of one layer's contours, asked for one box.
    nonisolated static let pageLimit: UInt32 = 3
    /// The longest pause a busy service asks for that the loader waits out by itself, seconds.
    nonisolated static let longestWait: UInt32 = 60
    /// How long a box whose fetch failed is left alone, seconds; then the latest view is asked
    /// for again.
    nonisolated static let coolOff: Int64 = 60
    /// The most requests for one thing, retries included (the core decides when to stop first).
    nonisolated static let tries = 4

    /// The charted points for the latest view.
    private(set) var points: [StructurePoint] = []
    /// New York's reefs, while they are wanted.
    private(set) var reefs: [ReefInfo] = []
    /// Per lake contour layer, its lines for the latest view.
    private(set) var contours: [String: [StateContourLine]] = [:]
    /// Something the latest view wants is being fetched.
    var busy: Bool { wanted.values.contains { inFlight.contains($0) } }
    /// What is shown changed, or a fetch started or ended.
    var onChange: () -> Void = {}

    private let host: FetchHost
    private let clock: @Sendable () -> Int64
    private let pause: Pause
    private var settingsJSON = ""
    private var wants = Wants()
    /// Per band, the ids of its point layers.
    private var bandLayers: [String: StructureLayers] = [:]
    private var heldPoints = StructureHeld<[StructurePoint]>()
    private var heldContours = StructureHeld<[StateContourLine]>(limit: 8)
    private var heldReefs: [ReefInfo]?
    /// Per slot (`points`, `reefs`, `contours/<layer>`), the fetch the latest view waits for.
    private var wanted: [String: String] = [:]
    private var inFlight: Set<String> = []
    private var failedAt: [String: Int64] = [:]
    /// The latest view asked for, to ask again after a failure.
    private var last: (region: MapRegion, wants: Wants, json: String)?
    private var retry: Task<Void, Never>?

    init(
        host: FetchHost, clock: @escaping @Sendable () -> Int64,
        pause: @escaping Pause = { duration in try? await Task.sleep(for: duration) }
    ) {
        self.host = host
        self.clock = clock
        self.pause = pause
    }

    /// Brings what `wants` needs for `region` (worded for `json`, the settings). The task ends
    /// when the fetches it started do (tests wait for it); none when nothing needs fetching.
    @discardableResult
    func load(_ region: MapRegion, wants: Wants, settingsJSON json: String) -> Task<Void, Never>? {
        if json != settingsJSON {
            // The words change with the settings: what was kept is worded again.
            settingsJSON = json
            heldPoints.removeAll()
            heldContours.removeAll()
            heldReefs = nil
            failedAt = [:]
        }
        self.wants = wants
        last = (region, wants, json)
        let before = (points, reefs, contours, busy)
        var tasks: [Task<Void, Never>] = []
        if let task = loadPoints(region) { tasks.append(task) }
        if let task = loadReefs() { tasks.append(task) }
        for layer in Set(contours.keys).subtracting(wants.contours) {
            contours[layer] = nil
            wanted["contours/\(layer)"] = nil
        }
        for layer in wants.contours {
            if let task = loadContours(layer, region: region) { tasks.append(task) }
        }
        if before != (points, reefs, contours, busy) { onChange() }
        guard !tasks.isEmpty else { return nil }
        return Task {
            for task in tasks { await task.value }
        }
    }

    /// Whether a fetch under `key` may start now: none of it running, none failed lately.
    private func start(_ key: String) -> Bool {
        guard !inFlight.contains(key) else { return false }
        if let failed = failedAt[key], clock() - failed < Self.coolOff { return false }
        failedAt[key] = nil
        inFlight.insert(key)
        return true
    }

    // MARK: - Charted points

    private func loadPoints(_ region: MapRegion) -> Task<Void, Never>? {
        let zoom = StructureMarks.zoomStep(region.zoom)
        guard wants.points, let band = structureBand(zoom: zoom) else {
            wanted["points"] = nil
            points = []
            return nil
        }
        let tag = "\(band)"
        if let held = heldPoints.covering(region, tag: tag) {
            wanted["points"] = nil
            points = held
            return nil
        }
        let box = StructureBox.around(region, zoom: Int(zoom))
        let key = "points|\(tag)|\(box.key)"
        wanted["points"] = key
        guard start(key) else { return nil }
        let (host, pause, json, layers) = (host, pause, settingsJSON, bandLayers[tag])
        return Task {
            let found = await Task.detached(priority: .userInitiated) {
                await Self.fetchPoints(
                    band: band, layers: layers, box: box, settingsJSON: json, host: host,
                    pause: pause)
            }.value
            self.pointsLanded(found, key: key, tag: tag, box: box, json: json)
        }
    }

    /// What a points fetch came to.
    nonisolated struct PointsFound: Sendable {
        /// The band's layer ids, if they were read.
        let layers: StructureLayers?
        let points: [StructurePoint]
        /// Every kind was read (up to the page limit).
        let complete: Bool
    }

    nonisolated static func fetchPoints(
        band: StructureBand, layers known: StructureLayers?, box: StructureBox,
        settingsJSON json: String, host: FetchHost, pause: Pause
    ) async -> PointsFound {
        var layers = known
        if layers == nil {
            layers = await ask(structureLayersRequest(band: band), host: host, pause: pause) {
                try parseStructureLayers(response: $0, attempt: $1)
            }
        }
        guard let layers else { return PointsFound(layers: nil, points: [], complete: false) }
        var points: [StructurePoint] = []
        var complete = true
        for kind in [StructureType.wreck, .obstruction, .rock] {
            var offset: UInt32 = 0
            for _ in 0..<pageLimit {
                guard
                    let request = try? structurePointsRequest(
                        band: band, layers: layers, kind: kind, west: box.west, south: box.south,
                        east: box.east, north: box.north, offset: offset)
                else { break }
                guard
                    let page = await ask(request, host: host, pause: pause, parse: {
                        try parseStructurePoints(
                            kind: kind, response: $0, attempt: $1, settingsJson: json)
                    })
                else {
                    complete = false
                    break
                }
                points += page.points
                guard page.more, page.read > 0 else { break }
                offset += page.read
            }
        }
        return PointsFound(layers: layers, points: points, complete: complete)
    }

    private func pointsLanded(
        _ found: PointsFound, key: String, tag: String, box: StructureBox, json: String
    ) {
        inFlight.remove(key)
        guard json == settingsJSON else { return }
        if let layers = found.layers { bandLayers[tag] = layers }
        if found.complete {
            heldPoints.hold(found.points, box: box, tag: tag)
        } else {
            failedAt[key] = clock()
            retryLater()
        }
        if wanted["points"] == key {
            wanted["points"] = nil
            // A fetch that brought nothing leaves what is shown.
            if found.complete || !found.points.isEmpty { points = found.points }
        }
        onChange()
    }

    // MARK: - Reefs

    private func loadReefs() -> Task<Void, Never>? {
        guard wants.reefs else {
            wanted["reefs"] = nil
            reefs = []
            return nil
        }
        if let heldReefs {
            wanted["reefs"] = nil
            reefs = heldReefs
            return nil
        }
        let key = "reefs"
        wanted["reefs"] = key
        guard start(key) else { return nil }
        let (host, pause, json) = (host, pause, settingsJSON)
        return Task {
            let found = await Task.detached(priority: .userInitiated) {
                await Self.ask(reefsRequest(), host: host, pause: pause, tries: 2) { response, _ in
                    try parseReefs(response: response, settingsJson: json)
                }
            }.value
            self.reefsLanded(found, key: key, json: json)
        }
    }

    private func reefsLanded(_ found: [ReefInfo]?, key: String, json: String) {
        inFlight.remove(key)
        guard json == settingsJSON else { return }
        if let found {
            heldReefs = found
        } else {
            failedAt[key] = clock()
            retryLater()
        }
        if wanted["reefs"] == key {
            wanted["reefs"] = nil
            if let found, wants.reefs { reefs = found }
        }
        onChange()
    }

    // MARK: - Lake contours

    private func loadContours(_ layer: String, region: MapRegion) -> Task<Void, Never>? {
        let slot = "contours/\(layer)"
        let zoom = StructureMarks.zoomStep(region.zoom)
        let tag = "\(layer)|\(zoom)"
        if let held = heldContours.covering(region, tag: tag) {
            wanted[slot] = nil
            contours[layer] = held
            return nil
        }
        let box = StructureBox.around(region, zoom: Int(zoom))
        // None outside the state's data or above the layer's zoom: nothing to ask.
        guard
            let first = try? stateContourRequest(
                layer: layer, west: box.west, south: box.south, east: box.east, north: box.north,
                zoom: zoom, offset: 0)
        else {
            wanted[slot] = nil
            contours[layer] = []
            return nil
        }
        let key = "\(slot)|\(zoom)|\(box.key)"
        wanted[slot] = key
        guard start(key) else { return nil }
        let (host, pause, json) = (host, pause, settingsJSON)
        return Task {
            let found = await Task.detached(priority: .userInitiated) {
                await Self.fetchContours(
                    layer: layer, first: first, box: box, zoom: zoom, settingsJSON: json,
                    host: host, pause: pause)
            }.value
            self.contoursLanded(
                found, layer: layer, key: key, tag: tag, box: box, json: json)
        }
    }

    nonisolated static func fetchContours(
        layer: String, first: HostRequest, box: StructureBox, zoom: UInt8,
        settingsJSON json: String, host: FetchHost, pause: Pause
    ) async -> [StateContourLine]? {
        var lines: [StateContourLine] = []
        var request = first
        var offset: UInt32 = 0
        for page in 0..<pageLimit {
            guard
                let found = await ask(
                    request, host: host, pause: pause, tries: 2,
                    parse: { response, _ in
                        try parseStateContours(
                            layer: layer, response: response, zoom: zoom, settingsJson: json)
                    })
            else { return page == 0 ? nil : lines }
            lines += found.lines
            guard found.more, found.read > 0 else { break }
            offset += found.read
            guard
                let next = try? stateContourRequest(
                    layer: layer, west: box.west, south: box.south, east: box.east,
                    north: box.north, zoom: zoom, offset: offset)
            else { break }
            request = next
        }
        return lines
    }

    private func contoursLanded(
        _ found: [StateContourLine]?, layer: String, key: String, tag: String, box: StructureBox,
        json: String
    ) {
        inFlight.remove(key)
        guard json == settingsJSON else { return }
        if let found {
            heldContours.hold(found, box: box, tag: tag)
        } else {
            failedAt[key] = clock()
            retryLater()
        }
        let slot = "contours/\(layer)"
        if wanted[slot] == key {
            wanted[slot] = nil
            if let found, wants.contours.contains(layer) { contours[layer] = found }
        }
        onChange()
    }

    /// Asks for the latest view again once the cool-off has passed (a failed box is left alone
    /// until then); one retry waits at a time.
    private func retryLater() {
        guard retry == nil else { return }
        let pause = self.pause
        retry = Task { [weak self] in
            await pause(.seconds(Int(Self.coolOff) + 1))
            guard let self else { return }
            self.retry = nil
            guard let last = self.last, last.wants.any else { return }
            self.load(last.region, wants: last.wants, settingsJSON: last.json)
        }
    }

    // MARK: - Asking

    /// Sends `request` and reads the answer with `parse` (given the response and the number of
    /// requests already made for it), asking again as long as the core says to and waiting as
    /// long as it says, past the cache. None when it fails for good.
    nonisolated static func ask<T>(
        _ request: HostRequest, host: FetchHost, pause: Pause, tries: Int = tries,
        parse: (HostResponse, UInt32) throws -> T
    ) async -> T? {
        var request = request
        var attempt: UInt32 = 0
        while attempt < UInt32(tries) {
            guard let response = try? await host.send(request: request) else { return nil }
            do {
                return try parse(response, attempt)
            } catch MapError.RetryLater(let after) {
                guard after <= longestWait, attempt + 1 < UInt32(tries) else { return nil }
                await pause(.seconds(Int(after)))
                attempt += 1
                request = HostRequest(
                    url: request.url, headers: request.headers, method: request.method,
                    body: request.body, fresh: true, noStore: request.noStore,
                    maxAgeS: request.maxAgeS)
            } catch {
                return nil
            }
        }
        return nil
    }
}
