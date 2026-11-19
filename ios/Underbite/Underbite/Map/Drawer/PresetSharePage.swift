import SwiftUI
import UnderbiteKit

/// Shares a map layer preset as a `.underbite` file, as a page of the drawer: the preset's name
/// and the layers it shows, what the file holds (no places, keys or dates), then the system's
/// share sheet. A page rather than a sheet, as a sheet asked for inside the drawer the iPhone
/// shows as a sheet is never presented. Every word is the core's; the file is written by the
/// core. Files sent before are removed when it opens.
struct PresetSharePage: View {
    let entry: PresetEntry
    /// The overlays of this build, for the layers' titles.
    let catalogue: [LayerInfo]
    /// Back to the presets.
    let back: () -> Void
    @Environment(SpotStore.self) private var store

    var body: some View {
        let words = PresetSharing.words(settingsJSON: store.settingsJSON)
        let map = try? Settings(json: store.settingsJSON).map
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                if let map {
                    let json = PresetSharing.layersJSON(for: entry, map: map)
                    LayerPresetCard(
                        name: entry.name,
                        titles: LayerPresetCard.titles(
                            PresetSharing.shownIDs(layersJSON: json), catalogue: catalogue))
                }
                if let words {
                    Text(words.shareDetail)
                        .font(.footnote)
                        .foregroundStyle(.readableSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                if let map {
                    link(map: map, words: words)
                }
            }
            .frame(maxWidth: 560, alignment: .leading)
            .frame(maxWidth: .infinity)
            .padding(20)
        }
        .readableText()
        .safeAreaInset(edge: .top, spacing: 0) {
            DrawerHeader(title: Text(verbatim: words?.share ?? ""), back: back)
        }
        .onAppear { UnderbiteFile.sweep() }
    }

    /// The system's share button for the preset's file, or the core's text when it cannot be
    /// written.
    @ViewBuilder
    private func link(map: Settings.MapSettings, words: PresetShareWords?) -> some View {
        switch Result(catching: { try PresetSharing.file(for: entry, map: map) }) {
        case .success(let file):
            ShareLink(item: file, preview: SharePreview(entry.name)) {
                Label(words?.shareFile ?? "", systemImage: "square.and.arrow.up")
                    .frame(maxWidth: .infinity)
            }
            .buttonStyle(.borderedProminent)
            .controlSize(.large)
            .accessibilityIdentifier("map-preset-share-file")
        case .failure(let error):
            Label(ShareText.message(of: error), systemImage: "exclamationmark.triangle")
                .font(.callout)
                .fixedSize(horizontal: false, vertical: true)
        }
    }
}

/// A preset as a card: its name and the titles of the layers it shows, as the catalogue words
/// them.
struct LayerPresetCard: View {
    let name: String
    let titles: [String]
    var notes: [String] = []

    /// The titles of `ids` in the catalogue; layers this build does not have are left out.
    static func titles(_ ids: [String], catalogue: [LayerInfo]) -> [String] {
        ids.compactMap { id in catalogue.first { $0.id == id && !$0.isBase }?.title }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(name)
                .font(.title3.bold())
                .accessibilityAddTraits(.isHeader)
            if !titles.isEmpty {
                VStack(alignment: .leading, spacing: 6) {
                    ForEach(Array(titles.enumerated()), id: \.offset) { _, title in
                        Label(title, systemImage: "square.3.layers.3d")
                            .font(.subheadline)
                    }
                }
            }
            ForEach(Array(notes.enumerated()), id: \.offset) { _, note in
                Label(note, systemImage: "info.circle")
                    .font(.footnote)
                    .foregroundStyle(.readableSecondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color(.secondarySystemGroupedBackground), in: .rect(cornerRadius: 14))
    }
}
