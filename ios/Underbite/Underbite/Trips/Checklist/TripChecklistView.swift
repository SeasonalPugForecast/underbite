import SwiftUI
@preconcurrency import UnderbiteKit

/// A trip's checklist, row by row as the core gives it: the official licence and regulations
/// pages of each place's region with the disclaimer (opened in Safari, never inside the app),
/// the trip's own items to tick, add, rename and delete, the gear of its method, its offline
/// pack and whether it is shared. A tick is saved to the trip at once. The pack row says what
/// the pack holds, when it was refreshed and whether it needs a refresh, and makes a pack of
/// the trip's places, days and the map's layers. Every word of a row is the core's.
struct TripChecklistView: View {
    @Environment(SpotStore.self) private var spots
    let trips: TripStore
    let tripID: String

    @State private var view: ViewModel.TripView?
    @State private var problem: String?
    @State private var saveError: String?
    @State private var links = TripPackLinks.load(.standard)
    @State private var makingPack: PackSheet?
    @State private var openPack = false
    @State private var newItem = ""
    @State private var renaming: ViewModel.ChecklistRow?
    @State private var renamedText = ""
    @State private var estimate: Result<PackPlanInfo, PackPlanProblem>?
    @State private var now = Int64(Date().timeIntervalSince1970)

    private var library: PackLibrary { .shared }
    private var trip: SavedTrip? { trips.trip(tripID) }
    private var pack: PackRecord? { links.pack(of: tripID, in: library.records) }
    private var words: PackWords? { try? packWords(settingsJson: spots.settingsJSON) }

    private var status: TripAppStatus {
        TripPackLink.appStatus(pack, storedBytes: pack.flatMap { library.storedBytes[$0.id] })
    }

    /// What the rows are built from: the trip, the settings and its pack's state.
    private var buildKey: String {
        let held = pack.map { "\($0.id) \($0.info?.refreshedAt ?? 0) \(library.storedBytes[$0.id] ?? 0)" }
        return [trip?.json ?? "", spots.settingsJSON, held ?? ""].joined(separator: "\n")
    }

    var body: some View {
        List {
            if let saveError {
                Section { WarningLine(text: saveError) }
            }
            if let view {
                rows(view.checklist)
            } else if let problem {
                Section { Text(problem).foregroundStyle(.readableSecondary) }
            } else {
                Section { ProgressView().frame(maxWidth: .infinity) }
            }
        }
        .listStyle(.insetGrouped)
        .readableText()
        .navigationTitle("Checklist")
        .navigationBarTitleDisplayMode(.inline)
        .task(id: buildKey) { await build() }
        .task {
            await library.loadSizes()
            #if DEBUG
                TripChecklistLaunch.apply(
                    trips: trips, tripID: tripID, settingsJSON: spots.settingsJSON
                ) { link($0) }
            #endif
        }
        .task(id: pack?.json) { await estimatePack() }
        .task {
            // The pack's age follows the clock.
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(60))
                now = Int64(Date().timeIntervalSince1970)
            }
        }
        .navigationDestination(isPresented: $openPack) {
            if let pack { PackDetailView(id: pack.id, now: now) }
        }
        .sheet(item: $makingPack) { sheet in
            NavigationStack {
                NewPackView(prefill: sheet.prefill) { id in
                    link(id)
                    makingPack = nil
                }
                .toolbar {
                    ToolbarItem(placement: .cancellationAction) {
                        Button("Cancel") { makingPack = nil }
                    }
                }
            }
            .environment(spots)
            .presentationSizing(.page)
        }
        .alert("Rename", isPresented: renamingShown, presenting: renaming) { row in
            TextField("Item", text: $renamedText)
            Button("Cancel", role: .cancel) {}
            Button("Save") { rename(row.id, to: renamedText) }
        }
    }

    // MARK: - Rows

    @ViewBuilder
    private func rows(_ checklist: [ViewModel.ChecklistRow]) -> some View {
        let pages = checklist.filter { $0.kind == "licence" || $0.kind == "regulations" }
        let disclaimer = checklist.first { $0.kind == "disclaimer" }
        if !pages.isEmpty || disclaimer != nil {
            Section {
                ForEach(pages) { row in LinkRow(row: row) }
                if let disclaimer { WarningLine(text: disclaimer.text) }
            }
        }
        // The pack beside the pages: both are what to have before leaving.
        if let row = checklist.first(where: { $0.kind == "offline_pack" }) {
            packSection(row)
        }
        regionSection
        itemsSection(checklist.filter { $0.kind == "item" })
        let gear = checklist.filter { $0.kind == "gear" }
        if !gear.isEmpty {
            Section("Gear") {
                ForEach(gear) { row in
                    Label(row.text, systemImage: "circle.fill")
                        .labelStyle(GearLabelStyle())
                }
            }
        }
        if let row = checklist.first(where: { $0.kind == "sharing" }) {
            Section { Text(row.text) }
        }
    }

    /// Each place's region (ISO 3166-2), which names the official pages linked above.
    @ViewBuilder
    private var regionSection: some View {
        let places = trip.map { TripChecklistEdit.places($0.json) } ?? []
        if !places.isEmpty {
            Section("Region") {
                ForEach(places) { place in
                    RegionField(place: place) { region in
                        _ = save(TripChecklistEdit.settingRegion(currentJSON, place: place.id, to: region))
                    }
                }
            }
        }
    }

    private func itemsSection(_ items: [ViewModel.ChecklistRow]) -> some View {
        Section("My list") {
            ForEach(items) { row in
                ItemRow(row: row) {
                    _ = save(
                        TripChecklistEdit.ticking(currentJSON, item: row.id, done: !(row.done ?? false)))
                }
                .swipeActions {
                    Button("Delete", role: .destructive) {
                        _ = save(TripChecklistEdit.removing(currentJSON, item: row.id))
                    }
                    Button("Rename") {
                        renamedText = row.text
                        renaming = row
                    }
                }
            }
            TextField("Add an item", text: $newItem)
                .submitLabel(.done)
                .onSubmit(addItem)
                .accessibilityIdentifier("trip-checklist-add")
        }
    }

    /// The core's pack line, then what the pack itself says: what it holds, its days, when it
    /// was refreshed and whether it needs a refresh, or its size before the download.
    private func packSection(_ row: ViewModel.ChecklistRow) -> some View {
        Section {
            Label {
                Text(row.text)
            } icon: {
                Image(systemName: row.done == true ? "checkmark.circle.fill" : "arrow.down.circle")
                    .foregroundStyle(row.done == true ? Palette.accent : Color.secondary)
                    .accessibilityHidden(true)
            }
            if let pack, let words {
                Button {
                    openPack = true
                } label: {
                    HStack {
                        VStack(alignment: .leading, spacing: 4) {
                            Text(pack.info?.name ?? "").font(.headline)
                            packLines(pack, words: words)
                        }
                        Spacer()
                        Image(systemName: "chevron.right")
                            .foregroundStyle(.readableSecondary)
                            .accessibilityHidden(true)
                    }
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityIdentifier("trip-checklist-open-pack")
            }
            Button {
                makingPack = PackSheet(prefill: prefill())
            } label: {
                Label("Make a pack for this trip", systemImage: "plus.circle")
            }
            .disabled(trip == nil)
            .accessibilityIdentifier("trip-checklist-make-pack")
        } footer: {
            if let words {
                VStack(alignment: .leading, spacing: 6) {
                    if library.wifiOnly { Text(words.wifiOnlyDetail).readableText() }
                    DrawerNotice(text: words.notice)
                }
            }
        }
    }

    @ViewBuilder
    private func packLines(_ pack: PackRecord, words: PackWords) -> some View {
        if let line = packRowLine(pack) {
            if pack.info?.refreshedAt != nil {
                LabeledContent("On this device", value: line.sizeText).font(.subheadline)
            } else if case .success(let plan) = estimate {
                LabeledContent("Download", value: plan.sizeText).font(.subheadline)
            }
            Text(line.refreshedText).font(.subheadline)
            Text(line.datesText).font(.subheadline).foregroundStyle(.readableSecondary)
            if let stale = line.staleText {
                Label(stale, systemImage: "clock.badge.exclamationmark")
                    .font(.subheadline)
                    .foregroundStyle(Palette.warning)
            }
        }
        PackStatusLine(record: pack, words: words)
    }

    private func packRowLine(_ pack: PackRecord) -> PackRow? {
        try? packRow(
            packJson: pack.json, storedBytes: UInt64(max(0, library.storedBytes[pack.id] ?? 0)),
            now: now, settingsJson: spots.settingsJSON, timeZone: TimeZone.current.identifier)
    }

    // MARK: - Building

    private func build() async {
        guard let trip else { return }
        let app = status
        let settings = spots.settingsJSON
        let now = Int64(Date().timeIntervalSince1970)
        let built = await Task.detached(priority: .userInitiated) {
            () -> Result<ViewModel.TripView, any Error> in
            // The checklist needs no fetched data: its rows are the trip's own and the app's.
            Result {
                try ViewModel.buildTrip(
                    tripJSON: trip.json, places: [], cachedMetadata: [], app: app,
                    settingsJSON: settings, now: Date(timeIntervalSince1970: TimeInterval(now)))
            }
        }.value
        if Task.isCancelled { return }
        switch built {
        case .success(let built):
            view = built
            problem = nil
        case .failure(let error):
            problem = SpotStore.message(of: error)
        }
    }

    /// The size of the trip's pack before its first download.
    private func estimatePack() async {
        guard let pack, pack.info?.refreshedAt == nil else {
            estimate = nil
            return
        }
        estimate = await library.estimate(pack.json, settingsJSON: spots.settingsJSON)
    }

    /// A new pack of the trip: the box around its places, its days and the map's layers.
    private func prefill() -> PackPrefill {
        let now = Int64(Date().timeIntervalSince1970)
        let json = currentJSON
        let settings = spots.settingsJSON
        let catalogue = (try? mapCatalogue(settingsJson: settings)) ?? []
        return TripPackLink.prefill(
            name: view?.title ?? "", places: (try? tripSpots(tripJson: json, now: now)) ?? [],
            saved: Set(spots.spots.map(\.id)), days: view?.days.map(\.date) ?? [],
            layers: TripPackLink.layers(catalogue), maxDays: words?.maxDays ?? 1)
    }

    // MARK: - Saving

    private var currentJSON: String { trip?.json ?? "{}" }

    private var renamingShown: Binding<Bool> {
        Binding(get: { renaming != nil }, set: { if !$0 { renaming = nil } })
    }

    private func addItem() {
        let text = newItem
        guard
            let json = TripChecklistEdit.adding(
                currentJSON, item: text, id: UUID().uuidString.lowercased())
        else { return }
        if save(json) { newItem = "" }
    }

    private func rename(_ id: String, to text: String) {
        _ = save(TripChecklistEdit.renaming(currentJSON, item: id, to: text))
    }

    /// Saves the edited trip, if there was an edit; whether it was saved.
    @discardableResult
    private func save(_ json: String?) -> Bool {
        guard let json else { return false }
        do {
            try trips.save(json)
            saveError = nil
            return true
        } catch {
            saveError = Self.message(of: error)
            return false
        }
    }

    private func link(_ packID: String) {
        links.packs[tripID] = packID
        links = links.pruned(trips: Set(trips.trips.map(\.id)), packs: library.records)
        links.save(.standard)
    }

    /// The core's words for why a trip could not be saved.
    private static func message(of error: any Error) -> String {
        switch error {
        case ShareError.NeedsNewerVersion(let message), ShareError.DatesBackwards(let message),
            ShareError.TooLong(let message), ShareError.Unreadable(let message),
            ShareError.TooLarge(let message), ShareError.Unsupported(_, let message),
            ShareError.Invalid(_, let message):
            message
        default: error.localizedDescription
        }
    }
}

/// The new-pack screen over the checklist, with what it starts from.
private struct PackSheet: Identifiable {
    let id = UUID()
    let prefill: PackPrefill
}

// MARK: - Row views

/// An official page, opened in Safari: its title and who publishes it, with when the link was
/// checked.
private struct LinkRow: View {
    let row: ViewModel.ChecklistRow

    var body: some View {
        if let link = row.link, let url = URL(string: link) {
            Link(destination: url) {
                HStack {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(row.text)
                        if let detail = row.detail {
                            Text(detail).font(.caption).foregroundStyle(.readableSecondary)
                        }
                    }
                    Spacer()
                    Image(systemName: "safari")
                        .foregroundStyle(Palette.accent)
                        .accessibilityHidden(true)
                }
                .contentShape(Rectangle())
            }
            .accessibilityAddTraits(.isLink)
        } else {
            Text(row.text)
        }
    }
}

/// Safety text, whole and in plain words.
private struct WarningLine: View {
    let text: String

    var body: some View {
        Label {
            Text(text).fixedSize(horizontal: false, vertical: true)
        } icon: {
            Image(systemName: "exclamationmark.triangle.fill")
                .foregroundStyle(Palette.warning)
                .accessibilityHidden(true)
        }
        .font(.subheadline)
    }
}

/// One of the trip's items: a tap ticks it or not.
private struct ItemRow: View {
    let row: ViewModel.ChecklistRow
    let toggle: () -> Void

    private var done: Bool { row.done ?? false }

    var body: some View {
        Button(action: toggle) {
            HStack(spacing: 12) {
                Image(systemName: done ? "checkmark.circle.fill" : "circle")
                    .font(.title3)
                    .foregroundStyle(done ? Palette.accent : Color.secondary)
                    .accessibilityHidden(true)
                Text(row.text)
                    .foregroundStyle(Color.primary)
                    .strikethrough(done)
                Spacer()
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(Text(row.text))
        .accessibilityAddTraits(done ? [.isButton, .isSelected] : .isButton)
    }
}

/// A place's region, saved when the field is left or submitted.
private struct RegionField: View {
    let place: TripChecklistEdit.Place
    let save: (String) -> Void
    @State private var text = ""
    @FocusState private var focused: Bool

    var body: some View {
        LabeledContent {
            TextField("Region", text: $text, prompt: Text("US-NY"))
                .textInputAutocapitalization(.characters)
                .autocorrectionDisabled()
                .multilineTextAlignment(.trailing)
                .focused($focused)
                .onSubmit { save(text) }
        } label: {
            Text(place.name)
        }
        .onAppear { text = place.region }
        .onChange(of: place.region) { _, region in text = region }
        .onChange(of: focused) { _, now in if !now { save(text) } }
    }
}

/// A gear row: a small dot before the item.
private struct GearLabelStyle: LabelStyle {
    func makeBody(configuration: Configuration) -> some View {
        HStack(spacing: 12) {
            configuration.icon
                .font(.system(size: 6))
                .foregroundStyle(.secondary)
                .accessibilityHidden(true)
            configuration.title
        }
    }
}

// MARK: - The way in

/// The trip page's way to its checklist.
struct TripChecklistEntry: View {
    let trips: TripStore
    let tripID: String
    @State private var open = false

    var body: some View {
        Button {
            open = true
        } label: {
            HStack {
                Label("Checklist", systemImage: "checklist").font(.headline)
                Spacer()
                Image(systemName: "chevron.right")
                    .foregroundStyle(.readableSecondary)
                    .accessibilityHidden(true)
            }
            .padding(12)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(
                Color(.secondarySystemGroupedBackground),
                in: RoundedRectangle(cornerRadius: 14, style: .continuous))
            .contentShape(RoundedRectangle(cornerRadius: 14, style: .continuous))
        }
        .buttonStyle(.plain)
        .accessibilityIdentifier("trip-checklist")
        .navigationDestination(isPresented: $open) {
            TripChecklistView(trips: trips, tripID: tripID)
        }
        #if DEBUG
            .task {
                // Once the trip's page has settled: a push asked for while the split view is
                // still showing the page is lost.
                guard TripChecklistLaunch.wanted else { return }
                try? await Task.sleep(for: .seconds(3))
                // A page replaced meanwhile leaves the push to the one that replaced it.
                guard !Task.isCancelled, TripChecklistLaunch.take() else { return }
                open = true
            }
        #endif
    }
}

#if DEBUG
    /// Launch arguments of the screenshot runs: `-openTripChecklist` opens the open trip's
    /// checklist; `-tripRegion <code>` gives the trip's places without a region that one;
    /// `-linkSeededPack` makes the pack `-seedPack` adds the trip's.
    enum TripChecklistLaunch {
        private static var opened = false
        private static var applied = false

        /// Whether the launch asks for the checklist and it has not been opened yet.
        static var wanted: Bool {
            !opened && ProcessInfo.processInfo.arguments.contains("-openTripChecklist")
        }

        /// Takes the one opening the launch asks for.
        static func take() -> Bool {
            guard wanted else { return false }
            opened = true
            return true
        }

        @MainActor
        static func apply(
            trips: TripStore, tripID: String, settingsJSON: String, link: (String) -> Void
        ) {
            guard !applied else { return }
            applied = true
            if let region = LaunchArguments.value(after: "-tripRegion"),
                let saved = trips.trip(tripID)?.json
            {
                var json = saved
                for place in TripChecklistEdit.places(json) where place.region.isEmpty {
                    if let next = TripChecklistEdit.settingRegion(json, place: place.id, to: region) {
                        json = next
                    }
                }
                if json != saved { _ = try? trips.save(json) }
            }
            if ProcessInfo.processInfo.arguments.contains("-linkSeededPack") {
                PacksLaunch.seedIfAsked(settingsJSON: settingsJSON)
                if PackLibrary.shared.record(PacksLaunch.seededID) != nil {
                    link(PacksLaunch.seededID)
                }
            }
        }
    }
#endif
