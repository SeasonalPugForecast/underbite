import SwiftUI
@preconcurrency import UnderbiteKit

/// A species: its profile, every value with its source and licence, or its seasons; follow it
/// for the species card, edit a field or put it back, and share the profile as a `.underbite`
/// file. Every word and value is the core's.
struct SpeciesProfileScreen: View {
    @Environment(SpotStore.self) private var spots
    let id: String

    @State private var page: Page = .profile
    @State private var editing: Editing?
    @State private var resettingAll = false
    @State private var problem: String?
    @State private var seasons: Result<SpeciesSeasons, SeasonsLoad.Problem>?
    /// The region of the official regulation pages (ISO 3166-2), kept on this device.
    @AppStorage("speciesRegion") private var region = ""

    private var store: SpeciesEditsStore { .shared }

    enum Page: Hashable {
        case profile, seasons
    }

    /// The field being edited.
    private struct Editing: Identifiable {
        var row: SpeciesFieldRow
        var field: String
        var id: String { field }
    }

    var body: some View {
        let loaded = Result {
            try SpeciesCatalogue.profile(id: id, edits: store.edits, settingsJSON: spots.settingsJSON)
        }
        Group {
            switch loaded {
            case .success(let profile):
                content(profile)
            case .failure(let error):
                ContentUnavailableView(
                    "This species cannot be shown", systemImage: "fish",
                    description: Text(SpeciesCatalogue.message(of: error)))
            }
        }
        .sheet(item: $editing) { editing in
            SpeciesFieldEditor(speciesID: id, row: editing.row, field: editing.field)
                .environment(spots)
        }
    }

    private func content(_ profile: SpeciesProfilePage) -> some View {
        List {
            Section {
                header(profile)
            }
            switch page {
            case .profile:
                Section {
                    ForEach(profile.fields, id: \.self) { row in
                        fieldRow(row)
                    }
                } footer: {
                    Text("Touch a value to change it; your edits stay on this device.")
                }
                SpeciesCreditsSection(credits: profile.credits, notices: profile.notices)
            case .seasons:
                SpeciesSeasonsSections(loaded: seasons, spot: seasonsSpot?.name, region: $region)
            }
        }
        .listStyle(.insetGrouped)
        .readableText()
        .navigationTitle(profile.name)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItemGroup(placement: .primaryAction) {
                if profile.edited {
                    Button("Reset all", systemImage: "arrow.uturn.backward") { resettingAll = true }
                        .disabled(!store.canSave)
                }
                shareButton(profile)
            }
        }
        .confirmationDialog(
            "Put back every value of \(profile.name)?", isPresented: $resettingAll,
            titleVisibility: .visible
        ) {
            Button("Reset all edits", role: .destructive) { reset(field: nil) }
        } message: {
            Text("Your edits of this species are removed; the sources' values show again.")
        }
        .task(id: page == .seasons ? seasonsLoad : nil) {
            guard page == .seasons, let load = seasonsLoad else { return }
            let snapshot = load.spot.flatMap { spots.cache($0).snapshotJSON }
            let now = Int64(Date().timeIntervalSince1970)
            seasons = await Task.detached(priority: .userInitiated) {
                load.build(snapshotJSON: snapshot, now: now)
            }.value
        }
    }

    // MARK: - Header

    private func header(_ profile: SpeciesProfilePage) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            VStack(alignment: .leading, spacing: 2) {
                Text(profile.name).font(.title2.bold())
                Text(profile.scientificName).italic().foregroundStyle(.readableSecondary)
            }
            .accessibilityElement(children: .combine)
            .accessibilityAddTraits(.isHeader)
            followButton
            if let problem {
                Label(problem, systemImage: "exclamationmark.triangle")
                    .font(.callout)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Picker("Show", selection: $page) {
                Text("Profile").tag(Page.profile)
                Text("Seasons").tag(Page.seasons)
            }
            .pickerStyle(.segmented)
        }
        .padding(.vertical, 4)
    }

    /// Follow or stop following the species: followed species show on the species card.
    private var followButton: some View {
        let follows = followed
        return Button {
            problem = spots.edit { settings in
                if follows {
                    settings.species.unfollow(id)
                } else {
                    settings.species.follow(id)
                }
            }
        } label: {
            Label(
                follows ? "Following" : "Follow",
                systemImage: follows ? "star.fill" : "star")
        }
        .buttonStyle(.bordered)
        .accessibilityHint(
            follows
                ? Text("Stops showing this species on the species card.")
                : Text("Shows this species on the species card of your spots."))
    }

    private var followed: Bool {
        (try? Settings(json: spots.settingsJSON))?.species.follows(id) ?? false
    }

    // MARK: - Fields

    @ViewBuilder
    private func fieldRow(_ row: SpeciesFieldRow) -> some View {
        if let field = row.field {
            Button {
                editing = Editing(row: row, field: field)
            } label: {
                SpeciesFieldLine(row: row).contentShape(Rectangle())
            }
            .foregroundStyle(.primary)
            .disabled(!store.canSave)
            .accessibilityHint(Text("Edits this value."))
            .swipeActions {
                if row.edited {
                    Button("Reset", systemImage: "arrow.uturn.backward") { reset(field: field) }
                }
            }
            .contextMenu {
                Button("Edit", systemImage: "pencil") { editing = Editing(row: row, field: field) }
                if row.edited {
                    Button("Use the source's value", systemImage: "arrow.uturn.backward") {
                        reset(field: field)
                    }
                }
                sourceLink(row.source)
            }
        } else {
            SpeciesFieldLine(row: row)
                .contextMenu { sourceLink(row.source) }
        }
    }

    @ViewBuilder
    private func sourceLink(_ source: SpeciesSource?) -> some View {
        if let source, let url = source.link.flatMap(URL.init(string:)) {
            Link(destination: url) {
                Label(source.name, systemImage: "safari")
            }
        }
    }

    // MARK: - Sharing

    @ViewBuilder
    private func shareButton(_ profile: SpeciesProfilePage) -> some View {
        if let text = try? speciesShareFile(id: id, edits: store.edits) {
            let file = UnderbiteFile(
                fileName: UnderbiteFile.fileName(for: profile.name), data: Data(text.utf8))
            ShareLink(item: file, preview: SharePreview(profile.name)) {
                Label("Share", systemImage: "square.and.arrow.up")
            }
        }
    }

    // MARK: - Seasons

    /// The spot whose water the seasons' "now" reads: the open one, else the first.
    private var seasonsSpot: Spot? { spots.activeSpot ?? spots.arranged.first }

    private var seasonsLoad: SeasonsLoad? {
        let spot = seasonsSpot
        return SeasonsLoad(
            id: id, edits: store.edits, settings: spots.settingsJSON, region: region,
            spot: spot?.id,
            fetched: spot.flatMap { spots.listing(for: $0.id)?.status.lastSuccess })
    }

    // MARK: - Actions

    private func reset(field: String?) {
        do {
            try store.reset(id: id, field: field)
            problem = nil
        } catch {
            problem = SpeciesCatalogue.message(of: error)
        }
    }
}
