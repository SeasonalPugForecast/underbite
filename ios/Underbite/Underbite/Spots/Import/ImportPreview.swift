import Foundation
@preconcurrency import UnderbiteKit

/// A file of waypoints and tracks (GPX, KML, KMZ or CSV) as its preview shows it. The file is
/// read once and the core parses it; the summary and any error are the core's words, and nothing
/// of the file leaves the device. A waypoint with the name of a spot already saved within 10 m
/// of it is flagged and left unchosen, never merged into that spot. The package tests compile
/// this file too.
nonisolated struct ImportPreview: Identifiable, Equatable, Sendable {
    /// A waypoint of the file.
    struct Waypoint: Identifiable, Equatable, Sendable {
        /// Its place in the file.
        var id: Int
        var point: ImportedWaypoint
        /// The saved spot it repeats (same name, within 10 m), if any.
        var duplicateOf: String?
    }

    /// A track of the file.
    struct Track: Identifiable, Equatable, Sendable {
        /// Its place in the file.
        var id: Int
        var track: ImportedTrack
    }

    let id = UUID()
    /// The name of the file, the preview's title.
    var fileName: String
    var waypoints: [Waypoint] = []
    var tracks: [Track] = []
    /// The core's account of what was read, with its notes, one per line; empty when the file
    /// could not be read.
    var summary = ""
    /// Why the file could not be read, in the core's words.
    var problem: String?
    /// The waypoints to add as spots, by their place in the file.
    var chosenWaypoints: Set<Int> = []
    /// The tracks to save, by their place in the file.
    var chosenTracks: Set<Int> = []

    /// The largest file the core reads (`import::MAX_INPUT_BYTES`); one byte more is read, so
    /// the core says the file is too large without the whole of it being loaded.
    static let maxBytes = 20 * 1024 * 1024

    /// How close a saved spot of the same name has to be for a waypoint to repeat it, metres.
    static let duplicateMetres = 10.0

    var isEmpty: Bool { waypoints.isEmpty && tracks.isEmpty }

    var hasChoice: Bool { !chosenWaypoints.isEmpty || !chosenTracks.isEmpty }

    // MARK: - Reading

    /// Reads the file at `url` (picked in Files, or opened from another app) once and parses it.
    /// The copy the system puts in the app's `Documents/Inbox` is removed once read.
    static func open(
        _ url: URL, spots: [Spot], settingsJSON: String,
        inbox: URL? = ReceivedFile.defaultInbox
    ) -> ImportPreview {
        let (data, name) = bytes(at: url)
        if let inbox, url.standardizedFileURL.path.hasPrefix(inbox.standardizedFileURL.path + "/") {
            try? FileManager.default.removeItem(at: url)
        }
        var preview = read(data, fileName: name, spots: spots, settingsJSON: settingsJSON)
        preview.fileName = url.lastPathComponent
        return preview
    }

    /// The bytes of the file at `url` and the name to tell its type by. A URL another app lends
    /// is security-scoped: access is asked for while reading and given back after. A KMZ that
    /// Files shows as a folder (an unpacked one) is read through its KML document, `doc.kml` or
    /// else the first `.kml` in it.
    static func bytes(at url: URL) -> (data: Data, name: String) {
        let scoped = url.startAccessingSecurityScopedResource()
        defer { if scoped { url.stopAccessingSecurityScopedResource() } }
        var target = url
        if (try? url.resourceValues(forKeys: [.isDirectoryKey]))?.isDirectory == true,
            let document = kmlDocument(in: url)
        {
            target = document
        }
        let data = (try? ReceivedFile.read(target, limit: maxBytes + 1)) ?? Data()
        return (data, target.lastPathComponent)
    }

    /// The KML document of an unpacked KMZ folder.
    static func kmlDocument(in folder: URL) -> URL? {
        let names =
            (try? FileManager.default.contentsOfDirectory(atPath: folder.path))?.sorted() ?? []
        let name =
            names.first { $0.lowercased() == "doc.kml" }
            ?? names.first { $0.lowercased().hasSuffix(".kml") }
        return name.map { folder.appending(path: $0, directoryHint: .notDirectory) }
    }

    /// Parses `data` with the core: the waypoints and tracks with the core's summary, or the
    /// core's words on why it cannot be read. Waypoints repeating a spot in `spots` are flagged;
    /// every other waypoint and every track is chosen.
    static func read(_ data: Data, fileName: String, spots: [Spot], settingsJSON: String)
        -> ImportPreview
    {
        var preview = ImportPreview(fileName: fileName)
        let file: ImportedFile
        do {
            file = try importFile(bytes: data, fileName: fileName)
        } catch ImportError.Invalid(let message) {
            preview.problem = message
            return preview
        } catch {
            preview.problem = error.localizedDescription
            return preview
        }
        preview.summary = file.summary
        preview.waypoints = file.waypoints.enumerated().map { index, point in
            Waypoint(
                id: index, point: point,
                duplicateOf: duplicate(of: point, in: spots, settingsJSON: settingsJSON)?.id)
        }
        preview.tracks = file.tracks.enumerated().map { Track(id: $0, track: $1) }
        preview.chosenWaypoints = Set(preview.waypoints.filter { $0.duplicateOf == nil }.map(\.id))
        preview.chosenTracks = Set(preview.tracks.map(\.id))
        return preview
    }

    // MARK: - Names and duplicates

    /// The name a waypoint is shown and saved under: its own, else its position as the core
    /// words it.
    static func name(of point: ImportedWaypoint, settingsJSON: String) -> String {
        if let name = point.name?.trimmingCharacters(in: .whitespacesAndNewlines), !name.isEmpty {
            return name
        }
        return pointText(latitude: point.latitude, longitude: point.longitude,
            settingsJSON: settingsJSON)
    }

    /// A position as the core words it: "40.7033° N, 74.0170° W".
    static func pointText(latitude: Double, longitude: Double, settingsJSON: String) -> String {
        let point = MapPoint(lat: latitude, lon: longitude)
        return (try? pointWords(point: point, settingsJson: settingsJSON))?.point
            ?? (try? pointWords(point: point, settingsJson: defaultSettingsJson()))?.point
            ?? ""
    }

    /// The saved spot `point` repeats: one with the same name (ignoring case and the spaces
    /// around it) within `duplicateMetres`.
    static func duplicate(of point: ImportedWaypoint, in spots: [Spot], settingsJSON: String)
        -> Spot?
    {
        let near = spots.filter {
            metres(point.latitude, point.longitude, $0.latitude, $0.longitude) <= duplicateMetres
        }
        guard !near.isEmpty else { return nil }
        let name = name(of: point, settingsJSON: settingsJSON)
        return near.first {
            $0.name.trimmingCharacters(in: .whitespacesAndNewlines)
                .caseInsensitiveCompare(name) == .orderedSame
        }
    }

    /// The great-circle distance between two positions, metres.
    static func metres(_ lat1: Double, _ lon1: Double, _ lat2: Double, _ lon2: Double) -> Double {
        let radius = 6_371_008.8
        let p1 = lat1 * .pi / 180
        let p2 = lat2 * .pi / 180
        let dp = (lat2 - lat1) * .pi / 180
        let dl = (lon2 - lon1) * .pi / 180
        let a = sin(dp / 2) * sin(dp / 2) + cos(p1) * cos(p2) * sin(dl / 2) * sin(dl / 2)
        return 2 * radius * asin(min(1, sqrt(a)))
    }

    // MARK: - Choosing

    mutating func toggleWaypoint(_ id: Int) {
        if chosenWaypoints.contains(id) {
            chosenWaypoints.remove(id)
        } else {
            chosenWaypoints.insert(id)
        }
    }

    mutating func toggleTrack(_ id: Int) {
        if chosenTracks.contains(id) {
            chosenTracks.remove(id)
        } else {
            chosenTracks.insert(id)
        }
    }

    /// The chosen waypoints as new spots, in file order. Their water type is not chosen, so
    /// their first fetch suggests one.
    func spotsToAdd(settingsJSON: String, newID: () -> String = { UUID().uuidString }) -> [Spot] {
        waypoints.filter { chosenWaypoints.contains($0.id) }.map { waypoint in
            Spot(
                id: newID(), name: Self.name(of: waypoint.point, settingsJSON: settingsJSON),
                latitude: waypoint.point.latitude, longitude: waypoint.point.longitude,
                waterType: Spot.unknownWaterType, waterTypeChosen: false)
        }
    }

    /// The chosen tracks to save, in file order, imported at `now` (Unix seconds).
    func tracksToSave(now: Int64, newID: () -> String = { UUID().uuidString }) -> [SavedTrack] {
        tracks.filter { chosenTracks.contains($0.id) }.map { item in
            SavedTrack(
                id: newID(), name: item.track.name, file: fileName, importedAt: now,
                segments: item.track.segments.map { segment in
                    segment.map {
                        SavedTrack.Point(latitude: $0.latitude, longitude: $0.longitude, time: $0.time)
                    }
                })
        }
    }

    /// Adds the chosen waypoints to `spots` and saves the chosen tracks in `tracks`, while it
    /// can save them (`TrackStore.canSave`). Returns the ids of the spots added, to fetch.
    @MainActor
    @discardableResult
    func add(
        to spots: SpotStore, tracks: TrackStore, now: Int64,
        newID: () -> String = { UUID().uuidString }
    ) -> [String] {
        let added = spotsToAdd(settingsJSON: spots.settingsJSON, newID: newID)
        for spot in added { spots.add(spot) }
        let saving = tracksToSave(now: now, newID: newID)
        if !saving.isEmpty, tracks.canSave { _ = try? tracks.add(saving) }
        return added.map(\.id)
    }

    // MARK: - The map

    /// At most `limit` points of `segment`, evenly spaced and keeping its ends, for a line on
    /// the preview's map.
    static func thinned<Point>(_ segment: [Point], limit: Int) -> [Point] {
        guard limit >= 2, segment.count > limit else { return segment }
        let step = Double(segment.count - 1) / Double(limit - 1)
        return (0..<limit).map { segment[Int((Double($0) * step).rounded())] }
    }
}
