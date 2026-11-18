import SwiftUI
@preconcurrency import UnderbiteKit

/// The species profiles, from Settings: the catalogue searched by common or scientific name
/// (case and accents ignored), the followed and edited ones marked. Side by side with the
/// chosen profile on a wide screen (the iPad), a list that pushes the profile on a narrow one.
struct SpeciesBrowser: View {
    @Environment(SpotStore.self) private var spots
    @Environment(\.horizontalSizeClass) private var sizeClass
    @State private var search = ""
    /// The species shown beside the list on a wide screen.
    @State private var selection: String?
    /// The species pushed on a narrow screen.
    @State private var pushed: String?
    @State private var width: CGFloat = 0

    private var store: SpeciesEditsStore { .shared }

    /// Wide enough for the list and a profile side by side.
    private var wide: Bool { sizeClass == .regular && width >= 700 }

    var body: some View {
        let rows = SpeciesCatalogue.rows(edits: store.edits, settingsJSON: spots.settingsJSON)
        let followed = Set((try? Settings(json: spots.settingsJSON))?.species.followed ?? [])
        Group {
            if wide {
                HStack(spacing: 0) {
                    list(rows, followed: followed)
                        .frame(width: 340)
                    Divider().ignoresSafeArea()
                    Group {
                        if let selection {
                            SpeciesProfileScreen(id: selection).id(selection)
                        } else {
                            ContentUnavailableView(
                                "Choose a species", systemImage: "fish",
                                description: Text("Its profile and seasons show here."))
                        }
                    }
                    .frame(maxWidth: .infinity)
                }
            } else {
                list(rows, followed: followed)
            }
        }
        .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { width = $0 }
        .navigationTitle("Species")
        .navigationDestination(item: $pushed) { id in
            SpeciesProfileScreen(id: id)
        }
        #if DEBUG
            .task {
                guard let asked = SpeciesLaunch.species() else { return }
                // Once Settings has given the page its width: a profile pushed while it is
                // still narrow would stay pushed.
                try? await Task.sleep(for: .seconds(1))
                open(asked)
            }
        #endif
        .onChange(of: wide, initial: true) { _, wide in
            guard wide else { return }
            // The profile pushed on the narrow screen shows beside the list instead.
            if let pushed {
                selection = pushed
                self.pushed = nil
            }
            if selection == nil { selection = rows.first?.id }
        }
    }

    private func open(_ id: String) {
        selection = id
        if !wide { pushed = id }
    }

    private func list(_ rows: [SpeciesListRow], followed: Set<String>) -> some View {
        let shown = SpeciesSearch.matching(search, in: rows)
        return List {
            if !store.canSave {
                Section {
                    Label(
                        "Your species edits could not be read, so new ones cannot be saved.",
                        systemImage: "exclamationmark.triangle")
                }
            }
            ForEach(shown) { row in
                Button {
                    open(row.id)
                } label: {
                    SpeciesRowLine(row: row, followed: followed.contains(row.id))
                }
                .foregroundStyle(.primary)
                .listRowBackground(
                    wide && selection == row.id ? Palette.accent.opacity(0.15) : nil)
                .accessibilityAddTraits(wide && selection == row.id ? .isSelected : [])
            }
        }
        .overlay {
            if shown.isEmpty, !search.isEmpty {
                ContentUnavailableView.search(text: search)
            }
        }
        .searchable(
            text: $search, placement: .navigationBarDrawer(displayMode: .always),
            prompt: Text("Common or scientific name"))
        .readableText()
    }
}

/// A species of the list: its name, scientific name and kind of water, marked when followed
/// or edited.
private struct SpeciesRowLine: View {
    let row: SpeciesListRow
    let followed: Bool

    var body: some View {
        HStack(alignment: .firstTextBaseline) {
            VStack(alignment: .leading, spacing: 2) {
                Text(row.name)
                Text(row.scientificName)
                    .font(.caption).italic()
                    .foregroundStyle(.readableSecondary)
                if let water = row.waterLabel {
                    Text(water).font(.caption).foregroundStyle(.readableSecondary)
                }
            }
            Spacer(minLength: 8)
            if row.edited {
                Image(systemName: "pencil")
                    .foregroundStyle(.readableSecondary)
                    .accessibilityLabel(Text("Edited"))
            }
            if followed {
                Image(systemName: "star.fill")
                    .foregroundStyle(Palette.accent)
                    .accessibilityLabel(Text("Following"))
            }
        }
        .contentShape(Rectangle())
        .accessibilityElement(children: .combine)
    }
}

#if DEBUG
    /// Launch argument of the screenshot runs: `-species <id>` opens that species' profile on
    /// the species page (`-settings species`).
    enum SpeciesLaunch {
        private static var applied = false

        static func species() -> String? {
            guard !applied, let id = LaunchArguments.value(after: "-species") else { return nil }
            applied = true
            return id
        }
    }
#endif
