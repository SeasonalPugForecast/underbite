import SwiftUI

/// What the Spots screen has open over it.
private enum SpotsSheet: Identifiable {
    case add
    case areas
    case edit(String)
    case stations(String)

    var id: String {
        switch self {
        case .add: "add"
        case .areas: "areas"
        case .edit(let id): "edit-" + id
        case .stations(let id): "stations-" + id
        }
    }
}

/// The spots, grouped by area, each opening its conditions. A split view: the list in the
/// first column and the conditions in the detail column on the iPad (both show at once), a
/// stack on the iPhone and in a narrow iPad window, where choosing a spot pushes its page. The
/// selection is the spot's id, so it survives the collapse to the compact layout and back.
struct SpotsView: View {
    @Environment(SpotStore.self) private var store
    @Environment(\.horizontalSizeClass) private var sizeClass
    @State private var selection: String?
    @State private var columns: NavigationSplitViewVisibility = .all
    @State private var compactColumn: NavigationSplitViewColumn = .sidebar
    @State private var sheet: SpotsSheet?

    var body: some View {
        NavigationSplitView(columnVisibility: $columns, preferredCompactColumn: $compactColumn) {
            SpotList(selection: $selection, sheet: $sheet)
        } detail: {
            NavigationStack {
                if let id = selection, store.spot(id) != nil {
                    ConditionsView(spotID: id)
                } else {
                    ContentUnavailableView(
                        "Choose a spot", systemImage: "water.waves",
                        description: Text("Its conditions show here."))
                }
            }
        }
        .navigationSplitViewStyle(.balanced)
        .onAppear {
            if sizeClass == .regular, selection == nil { selection = store.activeID }
            #if DEBUG
                DebugLaunch.apply(
                    store: store, openStations: { sheet = .stations($0) },
                    openSpot: { selection = $0 })
            #endif
        }
        .onChange(of: sizeClass) { _, now in
            if now == .regular, selection == nil { selection = store.activeID }
        }
        .onChange(of: selection) { _, id in
            compactColumn = id == nil ? .sidebar : .detail
        }
        .onChange(of: store.spots) { _, spots in
            // A spot deleted while it is open leaves the detail column with nothing.
            if let id = selection, !spots.contains(where: { $0.id == id }) { selection = nil }
        }
        .sheet(item: $sheet) { sheet in
            switch sheet {
            case .add:
                AddSpotView { added in selection = added }
            case .areas:
                AreasView()
            case .edit(let id):
                NavigationStack { SpotEditorView(spotID: id) }
            case .stations(let id):
                NavigationStack {
                    StationsView(spotID: id)
                        .toolbar {
                            ToolbarItem(placement: .confirmationAction) {
                                Button("Done") { self.sheet = nil }
                            }
                        }
                }
            }
        }
    }
}

/// The list column: spots under their areas, with the toolbar to add, edit and add an area.
private struct SpotList: View {
    @Environment(SpotStore.self) private var store
    @Binding var selection: String?
    @Binding var sheet: SpotsSheet?
    @State private var newAreaName = ""
    @State private var asksForArea = false
    @State private var deleting: Spot?
    @State private var removingArea: Area?
    @State private var renamingArea: Area?
    @State private var areaName = ""

    var body: some View {
        let sections = SpotGrouping.sections(spots: store.spots, areas: store.areas)
        List(selection: $selection) {
            ForEach(sections) { section in
                Section {
                    ForEach(section.spots) { spot in
                        SpotRow(spot: spot, listing: store.listing(for: spot.id))
                            .tag(spot.id)
                            .contextMenu { menu(for: spot) }
                            .swipeActions(edge: .leading) {
                                Button {
                                    store.setPinned(spot.id, !spot.pinned)
                                } label: {
                                    Label(spot.pinned ? "Unpin" : "Pin", systemImage: spot.pinned ? "pin.slash" : "pin")
                                }
                                .tint(.orange)
                                Button {
                                    store.setFavourite(spot.id, !spot.favourite)
                                } label: {
                                    Label(
                                        spot.favourite ? "Unfavourite" : "Favourite",
                                        systemImage: spot.favourite ? "star.slash" : "star")
                                }
                                .tint(.yellow)
                            }
                            .swipeActions(edge: .trailing) {
                                Button(role: .destructive) {
                                    deleting = spot
                                } label: {
                                    Label("Delete", systemImage: "trash")
                                }
                                Button {
                                    sheet = .edit(spot.id)
                                } label: {
                                    Label("Edit", systemImage: "pencil")
                                }
                            }
                    }
                    .onMove { source, destination in
                        store.reorder(
                            SpotGrouping.reordered(
                                all: store.arranged.map(\.id), section: section.spots.map(\.id),
                                moving: source, to: destination))
                    }
                } header: {
                    header(for: section)
                }
            }
        }
        .overlay {
            if store.spots.isEmpty { emptyState }
        }
        .refreshable { await store.refresh() }
        .navigationTitle("Spots")
        .toolbar {
            ToolbarItem(placement: .topBarLeading) {
                if !store.spots.isEmpty { EditButton() }
            }
            ToolbarItem(placement: .primaryAction) {
                Menu {
                    Button("Add a spot", systemImage: "mappin.and.ellipse") { sheet = .add }
                    Button("New area", systemImage: "folder.badge.plus") { asksForArea = true }
                    if !store.areas.isEmpty {
                        Button("Arrange areas", systemImage: "folder") { sheet = .areas }
                    }
                } label: {
                    Label("Add", systemImage: "plus")
                }
            }
        }
        .alert("New area", isPresented: $asksForArea) {
            TextField("Name", text: $newAreaName)
            Button("Add") {
                let name = newAreaName.trimmingCharacters(in: .whitespacesAndNewlines)
                if !name.isEmpty { _ = store.addArea(named: name) }
                newAreaName = ""
            }
            Button("Cancel", role: .cancel) { newAreaName = "" }
        } message: {
            Text("Group spots by the water you fish: \"Montauk\", \"Upper Delaware\".")
        }
        .alert(
            "Rename area",
            isPresented: Binding(get: { renamingArea != nil }, set: { if !$0 { renamingArea = nil } })
        ) {
            TextField("Name", text: $areaName)
            Button("Rename") {
                if let area = renamingArea { store.renameArea(area.id, to: areaName) }
                renamingArea = nil
            }
            Button("Cancel", role: .cancel) { renamingArea = nil }
        }
        .confirmationDialog(
            deleting.map { "Delete \($0.name)?" } ?? "Delete spot?",
            isPresented: Binding(get: { deleting != nil }, set: { if !$0 { deleting = nil } }),
            titleVisibility: .visible, presenting: deleting
        ) { spot in
            Button("Delete spot", role: .destructive) { store.remove(spot.id) }
        } message: { _ in
            Text("Its saved conditions are deleted from this device too.")
        }
        .confirmationDialog(
            removingArea.map { "Remove \($0.name)?" } ?? "Remove area?",
            isPresented: Binding(get: { removingArea != nil }, set: { if !$0 { removingArea = nil } }),
            titleVisibility: .visible, presenting: removingArea
        ) { area in
            Button("Remove area", role: .destructive) { store.removeArea(area.id) }
        } message: { _ in
            Text("The spots in it stay in the list, without an area.")
        }
    }

    @ViewBuilder
    private func header(for section: SpotSection) -> some View {
        switch section.kind {
        case .pinned:
            Label("Pinned", systemImage: "pin").foregroundStyle(.readableSecondary)
        case .area(let id):
            if let area = store.areas.first(where: { $0.id == id }) {
                Text(area.name)
                    .foregroundStyle(.readableSecondary)
                    .contextMenu {
                        Button("Rename", systemImage: "pencil") {
                            areaName = area.name
                            renamingArea = area
                        }
                        Button("Arrange areas", systemImage: "folder") { sheet = .areas }
                        Button("Remove area", systemImage: "trash", role: .destructive) {
                            removingArea = area
                        }
                    }
            }
        case .unassigned:
            if !store.areas.isEmpty { Text("Other spots").foregroundStyle(.readableSecondary) }
        }
    }

    @ViewBuilder
    private func menu(for spot: Spot) -> some View {
        Button(spot.pinned ? "Unpin" : "Pin to top", systemImage: spot.pinned ? "pin.slash" : "pin") {
            store.setPinned(spot.id, !spot.pinned)
        }
        Button(
            spot.favourite ? "Remove from favourites" : "Add to favourites",
            systemImage: spot.favourite ? "star.slash" : "star"
        ) {
            store.setFavourite(spot.id, !spot.favourite)
        }
        Button("Edit", systemImage: "pencil") { sheet = .edit(spot.id) }
        Button("Stations", systemImage: "antenna.radiowaves.left.and.right") {
            sheet = .stations(spot.id)
        }
        if !store.areas.isEmpty {
            Menu("Move to area", systemImage: "folder") {
                ForEach(store.arrangedAreas) { area in
                    Button(area.name) { store.assign(spot.id, toArea: area.id) }
                }
                if spot.areaID != nil {
                    Button("No area") { store.assign(spot.id, toArea: nil) }
                }
            }
        }
        Divider()
        Button("Delete", systemImage: "trash", role: .destructive) { deleting = spot }
    }

    private var emptyState: some View {
        VStack(spacing: 0) {
            LunaEmptyState(kind: .spots)
            Button("Add a spot") { sheet = .add }
                .buttonStyle(.borderedProminent)
                .controlSize(.large)
                .padding(.bottom, 32)
        }
    }
}

/// One spot of the list: its name, the kind of water and how fresh its data is, all as the
/// core words them.
private struct SpotRow: View {
    let spot: Spot
    let listing: SpotListing?

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack(spacing: 6) {
                Text(spot.name).font(.headline)
                if spot.favourite {
                    Image(systemName: "star.fill").foregroundStyle(.yellow).imageScale(.small)
                        .accessibilityLabel("Favourite")
                }
                if spot.pinned {
                    Image(systemName: "pin.fill").foregroundStyle(.orange).imageScale(.small)
                        .accessibilityLabel("Pinned")
                }
            }
            if let label = listing?.waterTypeLabel {
                Text(label).font(.subheadline).foregroundStyle(.readableSecondary)
            }
            SpotStatusLine(status: listing?.status)
        }
        .accessibilityElement(children: .combine)
    }
}

/// "updated 6 min ago", or the error, or a spinner while it fetches; one under the other at the
/// accessibility text sizes.
struct SpotStatusLine: View {
    let status: SpotListing.Status?
    @Environment(\.dynamicTypeSize) private var typeSize

    var body: some View {
        let layout =
            typeSize.isAccessibilitySize
            ? AnyLayout(VStackLayout(alignment: .leading, spacing: 2))
            : AnyLayout(HStackLayout(spacing: 6))
        layout {
            if status?.refreshing == true { ProgressView().controlSize(.mini) }
            // Whole at every text size: a narrow column would otherwise cut it short.
            if let updated = status?.updated {
                Text(updated).fixedSize(horizontal: false, vertical: true)
            } else if status?.refreshing != true {
                Text("Not updated yet").fixedSize(horizontal: false, vertical: true)
            }
            if let error = status?.error {
                Text(error).foregroundStyle(Palette.warning)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .font(.caption)
        .foregroundStyle(.readableSecondary)
    }
}

#if DEBUG
    /// Launch arguments of the screenshot runs: `-demoAreas` files the test places under two
    /// areas, `-demoStations` opens the station choices of The Battery.
    @MainActor
    enum DebugLaunch {
        private static var applied = false

        static func apply(
            store: SpotStore, openStations: (String) -> Void, openSpot: (String) -> Void
        ) {
            let arguments = ProcessInfo.processInfo.arguments
            guard !applied else { return }
            applied = true
            if arguments.contains("-demoAreas"), store.areas.isEmpty {
                let river = store.addArea(named: "Lower Hudson")
                let sea = store.addArea(named: "Offshore")
                store.assign(SeedSpots.battery.id, toArea: river.id)
                store.assign(SeedSpots.croton.id, toArea: river.id)
                store.assign(SeedSpots.buoy.id, toArea: sea.id)
            }
            if arguments.contains("-demoStations"), store.spot(SeedSpots.battery.id) != nil {
                openStations(SeedSpots.battery.id)
            }
            if let index = arguments.firstIndex(of: "-open-spot"), index + 1 < arguments.count,
                store.spot(arguments[index + 1]) != nil
            {
                openSpot(arguments[index + 1])
            }
        }
    }
#endif
