import SwiftUI
import UnderbiteKit

/// What the packs sheet opens on.
enum PacksStart: String, Identifiable {
    /// The list of packs.
    case list
    /// A new pack of the map's view.
    case new

    var id: String { rawValue }
}

/// The drawer's way to the offline packs: a row that opens them over the map, with the map's
/// view and layers for a new pack.
struct PacksDrawerSection: View {
    let controller: MapController
    @Environment(SpotStore.self) private var store
    @State private var shown: PacksStart?
    @State private var launched = false

    var body: some View {
        Section {
            Button {
                shown = .list
            } label: {
                Label("Offline packs", systemImage: "arrow.down.square")
            }
            .foregroundStyle(.primary)
            .accessibilityIdentifier("map-drawer-packs")
        }
        .sheet(item: $shown) { start in
            PacksView(start: start, mapArea: mapArea, mapLayers: mapLayers)
                .environment(store)
                // The list and a pack side by side on the iPad, with room for the area's map.
                .presentationSizing(.page)
        }
        .onAppear {
            guard !launched else { return }
            launched = true
            #if DEBUG
                PacksLaunch.seedIfAsked(settingsJSON: store.settingsJSON)
                if let start = PacksLaunch.start { shown = start }
            #endif
        }
    }

    /// The map's view, once it has settled.
    private var mapArea: PackBox? {
        controller.region.map(PackBox.init(region:))
    }

    /// The layers a new pack starts with: the NOAA chart (Apple's base map is not kept
    /// offline, the chart stands in for it), then those the map shows.
    private var mapLayers: [String] {
        let shown = controller.catalogue.filter { $0.shown && !$0.isBase }.map(\.id)
        return [PackDraft.chart] + shown.filter { $0 != PackDraft.chart }
    }
}

/// The packs' launch arguments, read in debug builds only (screenshots and checks):
/// `-packs list|new` opens the packs over the map's drawer (`-mapDrawer` opens it);
/// `-seedPack` adds a small NOAA chart pack around The Battery (zooms 12 to 13), not
/// downloaded; `-downloadSeededPack` downloads it.
enum PacksLaunch {
    static var start: PacksStart? {
        LaunchArguments.value(after: "-packs").flatMap(PacksStart.init(rawValue:))
    }

    #if DEBUG
        /// The seeded pack's id.
        static let seededID = "seed-battery-chart"

        private static var arguments: [String] { ProcessInfo.processInfo.arguments }

        /// Adds the seeded pack (and downloads it) when the launch asks.
        static func seedIfAsked(settingsJSON: String, library: PackLibrary = .shared) {
            guard arguments.contains("-seedPack") else { return }
            if library.record(seededID) == nil,
                let json = try? packNew(
                    id: seededID, name: SeedSpots.battery.name,
                    area: PackBox(west: -74.03, south: 40.695, east: -74.0, north: 40.715),
                    minZoom: 12, maxZoom: 13, layers: [PackDraft.chart], dates: .latest,
                    spotIds: [SeedSpots.battery.id], settingsJson: settingsJSON,
                    now: Int64(Date().timeIntervalSince1970))
            {
                library.add(json)
            }
            guard arguments.contains("-downloadSeededPack"),
                library.record(seededID)?.status != .done
            else { return }
            Task { await PackBackground.download(seededID, settingsJSON: settingsJSON) }
        }
    #endif
}
