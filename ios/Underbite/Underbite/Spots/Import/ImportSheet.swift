import MapKit
import SwiftUI
import UniformTypeIdentifiers
import UnderbiteKit

/// The file types the importer reads. GPX, KML and KMZ belong to other apps and are imported
/// in the app's Info.plist; CSV is the system's.
enum ImportTypes {
    /// GPX, as Xcode declares it on the Mac.
    static let gpx = UTType(importedAs: "com.topografix.gpx", conformingTo: .xml)
    /// KML and KMZ, as Google Earth declares them.
    static let kml = UTType(importedAs: "com.google.earth.kml", conformingTo: .xml)
    static let kmz = UTType(importedAs: "com.google.earth.kmz", conformingTo: .zip)

    static let all: [UTType] = [gpx, kml, kmz, .commaSeparatedText]
}

/// Where a file to import comes in, one per window: the Spots tab's file picker, or a file
/// opened from another app (routed here by the app's open-URL hook). It reads the file off the
/// main actor and holds the preview the sheet shows.
@MainActor
@Observable
final class ImportInbox {
    /// Whether the file picker is up.
    var choosing = false
    /// The file being previewed.
    var preview: ImportPreview?

    /// Whether a file opened in the app goes to the importer: every file but a `.underbite` one
    /// (the app opens no others), so the core says what it makes of it.
    static func accepts(_ url: URL) -> Bool {
        url.isFileURL && url.pathExtension.lowercased() != UnderbiteFile.pathExtension
    }

    /// Asks for a file.
    func choose() { choosing = true }

    /// Reads and previews the file at `url`, against the spots of `store`.
    func open(_ url: URL, store: SpotStore) {
        let spots = store.spots
        let settings = store.settingsJSON
        Task {
            let preview = await Task.detached(priority: .userInitiated) {
                ImportPreview.open(url, spots: spots, settingsJSON: settings)
            }.value
            self.preview = preview
        }
    }
}

extension View {
    /// The file picker of the importer and the sheet that previews a file to import, for
    /// `inbox`, which the views inside find in the environment.
    func receivesImports(_ inbox: ImportInbox) -> some View {
        modifier(ReceivesImports(inbox: inbox))
    }
}

/// The Spots tab's menu item that asks for a file to import.
struct ImportMenuItem: View {
    @Environment(ImportInbox.self) private var inbox: ImportInbox?

    var body: some View {
        Button("Import waypoints and tracks", systemImage: "square.and.arrow.down") {
            inbox?.choose()
        }
    }
}

private struct ReceivesImports: ViewModifier {
    @Environment(SpotStore.self) private var spots
    @Bindable var inbox: ImportInbox

    func body(content: Content) -> some View {
        content
            .environment(inbox)
            .fileImporter(
                isPresented: $inbox.choosing, allowedContentTypes: ImportTypes.all,
                allowsMultipleSelection: false
            ) { result in
                guard case .success(let urls) = result, let url = urls.first else { return }
                inbox.open(url, store: spots)
            }
            .sheet(item: $inbox.preview) { preview in
                ImportSheet(preview: preview, tracks: .shared)
                    .environment(spots)
            }
            #if DEBUG
                .task {
                    guard ImportLaunch.sample() else { return }
                    // Once the tabs have settled: a sheet asked for while the first screen is
                    // still appearing is lost.
                    try? await Task.sleep(for: .seconds(2))
                    inbox.preview = ImportPreview.read(
                        ImportSample.gpx, fileName: ImportSample.fileName, spots: spots.spots,
                        settingsJSON: spots.settingsJSON)
                }
            #endif
    }
}

/// The preview of a file to import: the core's summary or its words on why the file cannot be
/// read, a map of what it holds, its waypoints (one repeating a saved spot is flagged and left
/// unchosen) and its tracks, each to choose; adding makes the chosen waypoints spots and saves
/// the chosen tracks.
struct ImportSheet: View {
    @Environment(SpotStore.self) private var spots
    @Environment(\.dismiss) private var dismiss
    @State var preview: ImportPreview
    let tracks: TrackStore

    @State private var added = false

    private var words: ShareWords? { ShareText.words(settingsJSON: spots.settingsJSON) }

    var body: some View {
        let words = words
        NavigationStack {
            List {
                Section {
                    if let problem = preview.problem {
                        Label(problem, systemImage: "exclamationmark.triangle")
                            .fixedSize(horizontal: false, vertical: true)
                    } else {
                        Text(preview.summary)
                            .fixedSize(horizontal: false, vertical: true)
                            .accessibilityIdentifier("import-summary")
                    }
                    if added, let words {
                        Label(words.added, systemImage: "checkmark.circle.fill")
                            .font(.headline)
                            .foregroundStyle(Palette.accent)
                    }
                }
                if !preview.isEmpty {
                    Section {
                        ImportMap(preview: preview, settingsJSON: spots.settingsJSON)
                            .frame(height: 220)
                            .listRowInsets(EdgeInsets())
                    }
                }
                if !preview.waypoints.isEmpty {
                    Section("Waypoints") {
                        ForEach(preview.waypoints) { waypoint in
                            WaypointRow(
                                waypoint: waypoint,
                                chosen: preview.chosenWaypoints.contains(waypoint.id),
                                flag: waypoint.duplicateOf == nil ? nil : words?.spotExists,
                                settingsJSON: spots.settingsJSON
                            ) { preview.toggleWaypoint(waypoint.id) }
                            .disabled(added)
                        }
                    }
                }
                if !preview.tracks.isEmpty {
                    Section("Tracks") {
                        ForEach(preview.tracks) { track in
                            TrackRow(
                                track: track, chosen: preview.chosenTracks.contains(track.id),
                                settingsJSON: spots.settingsJSON
                            ) { preview.toggleTrack(track.id) }
                            .disabled(added || !tracks.canSave)
                        }
                    }
                }
            }
            .readableText()
            .navigationTitle(preview.fileName)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: added ? .confirmationAction : .cancellationAction) {
                    if added {
                        Button("Done") { dismiss() }
                    } else {
                        Button("Cancel", role: .cancel) { dismiss() }
                    }
                }
                if !added, let words, !preview.isEmpty {
                    ToolbarItem(placement: .confirmationAction) {
                        Button(words.add, action: add).disabled(!canAdd)
                    }
                }
            }
        }
        // On the iPad, a page-sized sheet: the map and the lists fit side by side with less
        // scrolling than in a form sheet.
        .presentationSizing(.page)
    }

    /// Something is chosen that can be added: a waypoint, or a track while tracks can be saved.
    private var canAdd: Bool {
        !preview.chosenWaypoints.isEmpty || (!preview.chosenTracks.isEmpty && tracks.canSave)
    }

    private func add() {
        let ids = preview.add(to: spots, tracks: tracks, now: Self.now)
        added = true
        guard !ids.isEmpty else { return }
        // Their water type is not chosen: the first fetch suggests one.
        Task { await spots.refresh(only: Set(ids)) }
    }

    private static var now: Int64 { Int64(Date().timeIntervalSince1970) }
}

/// The waypoints and tracks of a file on a map: at most `markerLimit` waypoints, and each
/// track segment thinned to at most `lineLimit` points.
private struct ImportMap: View {
    let preview: ImportPreview
    let settingsJSON: String
    @State private var camera = MapCameraPosition.automatic

    private static let markerLimit = 300
    private static let lineLimit = 1_000

    var body: some View {
        let lines = preview.tracks.flatMap { track in
            track.track.segments.map { segment in
                ImportPreview.thinned(segment, limit: Self.lineLimit).map {
                    CLLocationCoordinate2D(latitude: $0.latitude, longitude: $0.longitude)
                }
            }
        }
        Map(position: $camera) {
            ForEach(Array(lines.enumerated()), id: \.offset) { _, line in
                MapPolyline(coordinates: line, contourStyle: .straight)
                    .stroke(Palette.accent, lineWidth: 4)
            }
            ForEach(preview.waypoints.prefix(Self.markerLimit)) { waypoint in
                Marker(
                    ImportPreview.name(of: waypoint.point, settingsJSON: settingsJSON),
                    coordinate: CLLocationCoordinate2D(
                        latitude: waypoint.point.latitude, longitude: waypoint.point.longitude))
            }
        }
        .accessibilityHidden(true)
    }
}

/// A waypoint to choose: its name (or position), its position, its notes, and the flag when it
/// repeats a saved spot.
private struct WaypointRow: View {
    let waypoint: ImportPreview.Waypoint
    let chosen: Bool
    let flag: String?
    let settingsJSON: String
    let toggle: () -> Void

    var body: some View {
        Button(action: toggle) {
            HStack(alignment: .firstTextBaseline, spacing: 12) {
                ChoiceMark(chosen: chosen)
                VStack(alignment: .leading, spacing: 2) {
                    Text(ImportPreview.name(of: waypoint.point, settingsJSON: settingsJSON))
                        .font(.headline)
                    Text(
                        ImportPreview.pointText(
                            latitude: waypoint.point.latitude,
                            longitude: waypoint.point.longitude, settingsJSON: settingsJSON)
                    )
                    .font(.subheadline)
                    .foregroundStyle(.readableSecondary)
                    if let notes = waypoint.point.notes, !notes.isEmpty {
                        Text(notes)
                            .font(.subheadline)
                            .foregroundStyle(.readableSecondary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    if let flag {
                        Label(flag, systemImage: "exclamationmark.circle")
                            .font(.subheadline.weight(.semibold))
                            .foregroundStyle(Palette.warning)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(chosen ? .isSelected : [])
    }
}

/// A track to choose: its name, else where it starts.
private struct TrackRow: View {
    let track: ImportPreview.Track
    let chosen: Bool
    let settingsJSON: String
    let toggle: () -> Void

    var body: some View {
        Button(action: toggle) {
            HStack(alignment: .firstTextBaseline, spacing: 12) {
                ChoiceMark(chosen: chosen)
                Label {
                    Text(title).font(.headline)
                } icon: {
                    Image(systemName: "point.topleft.down.to.point.bottomright.curvepath")
                        .foregroundStyle(Palette.accent)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(chosen ? .isSelected : [])
    }

    private var title: String {
        if let name = track.track.name?.trimmingCharacters(in: .whitespacesAndNewlines),
            !name.isEmpty
        {
            return name
        }
        guard let first = track.track.segments.first?.first else { return "" }
        return ImportPreview.pointText(
            latitude: first.latitude, longitude: first.longitude, settingsJSON: settingsJSON)
    }
}

/// A filled check when chosen, an empty circle when not.
private struct ChoiceMark: View {
    let chosen: Bool

    var body: some View {
        Image(systemName: chosen ? "checkmark.circle.fill" : "circle")
            .font(.title3)
            .foregroundStyle(chosen ? Palette.accent : Color.secondary)
            .accessibilityHidden(true)
    }
}

#if DEBUG
    /// Launch argument of the screenshot runs: `-importFile sample` previews the synthetic GPX
    /// (`ImportSample`), as if it had been opened from Files.
    enum ImportLaunch {
        private static var applied = false

        static func sample() -> Bool {
            guard !applied, LaunchArguments.value(after: "-importFile") == "sample" else {
                return false
            }
            applied = true
            return true
        }
    }
#endif
