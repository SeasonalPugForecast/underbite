import PhotosUI
import SwiftUI
@preconcurrency import UnderbiteKit

/// Logs a catch, or edits one: when and where, the species, its size typed in the user's units
/// (read by the core, which says what is wrong as it is typed), what became of it, the lure,
/// method and notes, photos, and the conditions it is saved with, each with its source. Photos
/// are picked without access to the photo library and copied into the app without their
/// metadata. Nothing is kept until Save.
struct CatchEditorView: View {
    @Environment(SpotStore.self) private var spots
    @Environment(\.dismiss) private var dismiss
    let catches: CatchStore
    /// The catch to edit; `nil` for a new one.
    let catchID: String?
    /// Called with the catch's id once it is saved.
    let saved: (String) -> Void

    @State private var draft = CatchDraft(now: Date(), spot: nil)
    /// The draft as it was opened, to know whether anything changed.
    @State private var opened: CatchDraft?
    @State private var loadError: String?
    @State private var units: FishUnitsInfo?
    @State private var fates: [FateOption] = []
    @State private var species: [SpeciesChoice] = []
    @State private var lengthProblem: String?
    @State private var weightProblem: String?
    @State private var preview: CatchConditionsPreview?
    @State private var saveError: String?
    @State private var picked: [PhotosPickerItem] = []
    /// Photo files written while the editor is open; removed again unless saved.
    @State private var addedPhotos: [String] = []
    @State private var addingPhotos = 0
    @State private var photoProblem: String?
    @State private var locating = false
    @State private var locationProblem: String?

    var body: some View {
        Group {
            if opened != nil {
                form
            } else if let loadError {
                ContentUnavailableView(
                    "This catch cannot be edited", systemImage: "fish",
                    description: Text(loadError))
            } else {
                ProgressView()
            }
        }
        .navigationTitle(catchID == nil ? "New catch" : "Edit catch")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .cancellationAction) {
                Button("Cancel") { cancel() }
            }
            ToolbarItem(placement: .confirmationAction) {
                Button("Save") { save() }
                    .disabled(!canSave)
            }
        }
        .interactiveDismissDisabled(changed)
        .task { load() }
        // The conditions follow the time and the place, and a fetch that brings a newer snapshot.
        .task(id: previewKey) { await buildPreview() }
        .onChange(of: draft.lengthText) { _, _ in checkLength() }
        .onChange(of: draft.weightText) { _, _ in checkWeight() }
        .onChange(of: draft.ouncesText) { _, _ in checkWeight() }
        .onChange(of: draft.place) { _, place in
            if place == .here, draft.here == nil { Task { await locate() } }
        }
        .onChange(of: picked) { _, items in
            guard !items.isEmpty else { return }
            picked = []
            Task { await addPhotos(items) }
        }
    }

    private var form: some View {
        Form {
            if let saveError {
                Section { ProblemText(text: saveError) }
            }
            whenAndWhere
            // The conditions follow from the time and the place, so they come right after.
            conditionsSection
            fishSection
            Section("How") {
                TextField("Lure", text: $draft.lure)
                TextField("Method", text: $draft.method, prompt: Text("Shore, wading, kayak…"))
            }
            Section("Notes") {
                TextField("Notes", text: $draft.notes, axis: .vertical)
                    .lineLimit(3...)
            }
            photosSection
        }
        .readableText()
    }

    // MARK: - Sections

    private var whenAndWhere: some View {
        Section {
            DatePicker(
                "Time", selection: $draft.caughtAt, in: ...Date(),
                displayedComponents: [.date, .hourAndMinute]
            )
            // The time is read on the clock of the place.
            .environment(\.timeZone, catchZone)
            Picker("Place", selection: $draft.place) {
                ForEach(spots.arranged) { spot in
                    Text(spot.name).tag(CatchDraft.Place?.some(.spot(spot.id)))
                }
                if opened?.place == .stored {
                    Text("Where it was logged").tag(CatchDraft.Place?.some(.stored))
                }
                Text("Current location").tag(CatchDraft.Place?.some(.here))
            }
            if draft.place == .here { hereStatus }
            if draft.spot(in: spots.spots) == nil {
                TextField("Place name", text: $draft.area, prompt: Text("Optional"))
            }
        } header: {
            Text("When and where")
        }
    }

    @ViewBuilder
    private var hereStatus: some View {
        if locating {
            HStack {
                ProgressView()
                Text("Finding where you are…").foregroundStyle(.readableSecondary)
            }
        } else if let locationProblem {
            ProblemText(text: locationProblem)
            Button("Try again") { Task { await locate() } }
        } else if draft.here != nil {
            Label("Your location is set", systemImage: "location.fill")
                .foregroundStyle(.readableSecondary)
        }
    }

    private var fishSection: some View {
        Section {
            NavigationLink {
                SpeciesChoiceView(
                    choices: species, species: $draft.species, text: $draft.speciesText)
            } label: {
                LabeledContent("Species") {
                    Text(speciesName ?? "None").foregroundStyle(.readableSecondary)
                }
            }
            if let units {
                sizeField("Length", text: $draft.lengthText, unit: units.lengthLabel)
                if let lengthProblem { ProblemText(text: lengthProblem) }
                sizeField("Weight", text: $draft.weightText, unit: units.weightLabel)
                if !units.ouncesLabel.isEmpty {
                    sizeField("Ounces", text: $draft.ouncesText, unit: units.ouncesLabel)
                }
                if let weightProblem { ProblemText(text: weightProblem) }
            }
            if !fates.isEmpty {
                Picker("Fate", selection: fateBinding) {
                    ForEach(fates, id: \.id) { option in
                        Text(option.label).tag(option.id)
                    }
                }
                .pickerStyle(.segmented)
            }
        } header: {
            Text("The fish")
        }
    }

    private func sizeField(_ title: String, text: Binding<String>, unit: String) -> some View {
        LabeledContent {
            HStack(spacing: 6) {
                TextField(title, text: text, prompt: Text(""))
                    .keyboardType(.decimalPad)
                    .multilineTextAlignment(.trailing)
                    .accessibilityLabel(Text(title))
                    .accessibilityHint(Text(unit))
                Text(unit)
                    .foregroundStyle(.readableSecondary)
                    .accessibilityHidden(true)
            }
        } label: {
            Text(title)
        }
    }

    private var photosSection: some View {
        Section {
            if !draft.photos.isEmpty {
                ScrollView(.horizontal) {
                    HStack(spacing: 8) {
                        ForEach(draft.photos, id: \.self) { name in
                            CatchPhoto(url: catches.photoURL(name), side: 96)
                                .overlay(alignment: .topTrailing) {
                                    Button {
                                        draft.photos.removeAll { $0 == name }
                                    } label: {
                                        Image(systemName: "xmark.circle.fill")
                                            .font(.title3)
                                            .symbolRenderingMode(.palette)
                                            .foregroundStyle(.white, .black.opacity(0.6))
                                            .padding(4)
                                    }
                                    .buttonStyle(.plain)
                                    .accessibilityLabel(Text("Remove this photo"))
                                }
                        }
                    }
                }
                .scrollIndicators(.hidden)
            }
            if addingPhotos > 0 {
                HStack {
                    ProgressView()
                    Text("Adding photos…").foregroundStyle(.readableSecondary)
                }
            }
            if let photoProblem { ProblemText(text: photoProblem) }
            PhotosPicker(
                selection: $picked, maxSelectionCount: 10, selectionBehavior: .ordered,
                matching: .images, preferredItemEncoding: .current
            ) {
                Label("Add photos", systemImage: "photo.badge.plus")
            }
        } header: {
            Text("Photos")
        } footer: {
            Text("Photos are copied into the app, smaller and without their location, time or camera details.")
        }
    }

    @ViewBuilder
    private var conditionsSection: some View {
        if let preview {
            Section(preview.title) {
                if let empty = preview.none, preview.conditions.isEmpty {
                    Text(empty).foregroundStyle(.readableSecondary)
                }
                ForEach(preview.conditions, id: \.self) { line in
                    VStack(alignment: .leading, spacing: 2) {
                        LabeledContent {
                            Text(line.value).multilineTextAlignment(.trailing)
                        } label: {
                            Text(line.label)
                        }
                        Text(line.footnote)
                            .font(.caption)
                            .foregroundStyle(.readableSecondary)
                    }
                    .accessibilityElement(children: .combine)
                }
            }
        }
    }

    // MARK: - State

    private var catchZone: TimeZone {
        TimeZone(identifier: draft.timeZone(spots: spots.spots)) ?? .current
    }

    private var speciesName: String? {
        if let id = draft.species {
            return species.first { $0.id == id }?.name ?? id
        }
        let typed = draft.speciesText.trimmingCharacters(in: .whitespacesAndNewlines)
        return typed.isEmpty ? nil : typed
    }

    private var fateBinding: Binding<FateId> {
        Binding {
            draft.shownFate(fates.map(\.id))
        } set: { chosen in
            draft.pick(chosen)
        }
    }

    private var changed: Bool {
        opened != nil && (opened != draft || !addedPhotos.isEmpty)
    }

    private var canSave: Bool {
        opened != nil && lengthProblem == nil && weightProblem == nil && addingPhotos == 0
            && draft.position(spots: spots.spots) != nil
    }

    /// What the conditions depend on.
    private struct PreviewKey: Equatable {
        var at: Int64
        var place: CatchDraft.Place?
        var here: CatchDraft.Position?
        var snapshot: Int64?
        var opened: Bool
    }

    private var previewKey: PreviewKey {
        PreviewKey(
            at: draft.caughtAtSeconds, place: draft.place, here: draft.here,
            snapshot: draft.snapshotSpot(spots: spots.spots).flatMap {
                spots.listing(for: $0.id)?.status.lastSuccess
            },
            opened: opened != nil)
    }

    /// The snapshot the catch's conditions come from: the last fetch of its spot, or of a saved
    /// spot close by.
    private var snapshotJSON: String? {
        draft.snapshotSpot(spots: spots.spots).flatMap { spots.cache($0.id).snapshotJSON }
    }

    // MARK: - Actions

    private func load() {
        guard opened == nil, loadError == nil else { return }
        let settings = spots.settingsJSON
        units = try? fishUnitsInfo(settingsJson: settings)
        fates = (try? fateOptions(settingsJson: settings)) ?? []
        species = SpeciesChoice.catalogue(settingsJSON: settings)
        if let catchID {
            do {
                let input = try logEditor(book: catches.book, id: catchID, settingsJson: settings)
                draft = CatchDraft(editing: input, spots: spots.spots)
            } catch {
                loadError = CatchStore.message(error)
                return
            }
        } else {
            draft = CatchDraft(now: Date(), spot: spots.activeSpot ?? spots.arranged.first)
            if draft.place == .here { Task { await locate() } }
        }
        opened = draft
        checkLength()
        checkWeight()
    }

    private func checkLength() {
        lengthProblem =
            (try? checkLengthText(text: draft.lengthText, settingsJson: spots.settingsJSON)) ?? nil
    }

    private func checkWeight() {
        weightProblem =
            (try? checkWeightText(
                weight: draft.weightText, ounces: draft.ouncesText,
                settingsJson: spots.settingsJSON)) ?? nil
    }

    private func buildPreview() async {
        guard opened != nil, let input = draft.input(spots: spots.spots) else {
            preview = nil
            return
        }
        let book = catches.book
        let snapshot = snapshotJSON
        let attach = draft.needsConditions(spots: spots.spots)
        let settings = spots.settingsJSON
        let now = Int64(Date().timeIntervalSince1970)
        preview = await Task.detached(priority: .userInitiated) {
            CatchConditionsPreview.build(
                book: book, input: input, snapshotJSON: snapshot, attach: attach,
                settingsJSON: settings, now: now)
        }.value
    }

    private func locate() async {
        guard !locating else { return }
        locating = true
        locationProblem = nil
        defer { locating = false }
        do {
            let fix = try await CurrentLocation.fix(reuseRecent: true)
            draft.here = CatchDraft.Position(latitude: fix.latitude, longitude: fix.longitude)
        } catch {
            locationProblem = error.localizedDescription
        }
    }

    private func addPhotos(_ items: [PhotosPickerItem]) async {
        photoProblem = nil
        addingPhotos += items.count
        for item in items {
            defer { addingPhotos -= 1 }
            do {
                guard let data = try await item.loadTransferable(type: Data.self) else {
                    throw CatchPhotos.Failure.unreadable
                }
                let name = try await catches.addPhoto(data)
                addedPhotos.append(name)
                draft.photos.append(name)
            } catch {
                photoProblem = "A photo could not be added. It may still be downloading from iCloud."
            }
        }
    }

    private func cancel() {
        catches.removePhotos(addedPhotos)
        addedPhotos = []
        dismiss()
    }

    private func save() {
        guard let input = draft.input(spots: spots.spots) else { return }
        let attach = draft.needsConditions(spots: spots.spots)
        let snapshot = snapshotJSON
        do {
            let id = try catches.save(input, settingsJSON: spots.settingsJSON)
            if attach, let snapshot {
                // Nothing near the catch time leaves the catch as it is.
                _ = try? catches.attachConditions(to: id, snapshotJSON: snapshot)
            }
            let kept = Set(draft.photos)
            let dropped = ((opened?.photos ?? []) + addedPhotos).filter { !kept.contains($0) }
            catches.removePhotos(dropped)
            addedPhotos = []
            saved(id)
            dismiss()
        } catch {
            saveError = CatchStore.message(error)
        }
    }
}

/// The catalogue's species to choose from, searched by common or scientific name, or a species
/// typed in for one that is not there.
private struct SpeciesChoiceView: View {
    @Environment(\.dismiss) private var dismiss
    let choices: [SpeciesChoice]
    @Binding var species: String?
    @Binding var text: String
    @State private var search = ""

    var body: some View {
        let typed = search.trimmingCharacters(in: .whitespacesAndNewlines)
        let shown = SpeciesChoice.matching(search, in: choices)
        List {
            if !typed.isEmpty, !shown.contains(where: { $0.name.caseInsensitiveCompare(typed) == .orderedSame }) {
                Section {
                    Button {
                        choose(nil, typed: typed)
                    } label: {
                        Label("Use “\(typed)”", systemImage: "character.cursor.ibeam")
                    }
                } footer: {
                    Text("For a species that is not in the list.")
                }
            }
            if species == nil, !text.isEmpty, typed.isEmpty {
                Section {
                    row(text, detail: nil, chosen: true) { choose(nil, typed: text) }
                }
            }
            Section {
                if typed.isEmpty {
                    row("None", detail: nil, chosen: species == nil && text.isEmpty) {
                        choose(nil, typed: "")
                    }
                }
                ForEach(shown) { choice in
                    row(choice.name, detail: choice.scientificName, chosen: species == choice.id) {
                        choose(choice.id, typed: "")
                    }
                }
            }
        }
        .searchable(text: $search, placement: .navigationBarDrawer(displayMode: .always))
        .navigationTitle("Species")
        .navigationBarTitleDisplayMode(.inline)
    }

    private func row(_ name: String, detail: String?, chosen: Bool, action: @escaping () -> Void)
        -> some View
    {
        Button(action: action) {
            HStack {
                VStack(alignment: .leading, spacing: 2) {
                    Text(name).foregroundStyle(Color.primary)
                    if let detail {
                        Text(detail).font(.caption).italic().foregroundStyle(.readableSecondary)
                    }
                }
                Spacer()
                if chosen {
                    Image(systemName: "checkmark")
                        .foregroundStyle(Palette.accent)
                        .accessibilityHidden(true)
                }
            }
            .contentShape(Rectangle())
        }
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(chosen ? .isSelected : [])
    }

    private func choose(_ id: String?, typed: String) {
        species = id
        text = typed
        dismiss()
    }
}

/// What is wrong, in the core's words or the app's.
private struct ProblemText: View {
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
