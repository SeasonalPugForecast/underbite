import SwiftUI
import UnderbiteKit

/// A new offline pack: its area on a map (the map's view, or a trip's), how close it zooms,
/// its layers, days and spots, and the estimate the core makes of it (size, items and time)
/// before anything is downloaded. Nothing is downloaded until Download is tapped. On a wide
/// screen the area takes the larger map beside the choices.
struct NewPackView: View {
    let prefill: PackPrefill
    /// Called with the new pack's id once its download has started.
    var onCreated: (String) -> Void = { _ in }

    @Environment(SpotStore.self) private var store
    @Environment(\.horizontalSizeClass) private var sizeClass
    @State private var draft: PackDraft?
    @State private var candidates: [String] = []
    @State private var estimate: Result<PackPlanInfo, PackPlanProblem>?
    @State private var problem: String?
    @State private var lastDays: UInt32 = 3
    @State private var chosenDays: Set<DateComponents> = []
    @State private var starting = false

    init(prefill: PackPrefill, onCreated: @escaping (String) -> Void = { _ in }) {
        self.prefill = prefill
        self.onCreated = onCreated
    }

    private var library: PackLibrary { .shared }

    private var words: PackWords? { try? packWords(settingsJson: store.settingsJSON) }

    var body: some View {
        Group {
            if let words, draft != nil {
                content(words)
            } else {
                ProgressView()
            }
        }
        .navigationTitle("New pack")
        .navigationBarTitleDisplayMode(.inline)
        .onAppear(perform: start)
        .task(id: draft) { await estimateDraft() }
    }

    @ViewBuilder
    private func content(_ words: PackWords) -> some View {
        Group {
            if sizeClass == .regular {
                // The larger map above the choices.
                VStack(spacing: 0) {
                    areaMap
                        .frame(height: 360)
                        .padding(16)
                    form(words, mapInline: false)
                }
            } else {
                form(words, mapInline: true)
            }
        }
        .safeAreaInset(edge: .bottom, spacing: 0) { estimateBar }
    }

    /// The estimate and the download, always in view: nothing goes before the tap.
    private var estimateBar: some View {
        VStack(alignment: .leading, spacing: 6) {
            switch estimate {
            case .success(let plan):
                HStack(alignment: .firstTextBaseline) {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(plan.sizeText).font(.headline)
                        Text(plan.timeText).font(.subheadline).readableText()
                        Text(plan.countsText).font(.subheadline).readableText()
                    }
                    Spacer()
                    downloadButton(plan)
                }
                if let over = plan.overBudgetText {
                    Label(over, systemImage: "exclamationmark.triangle.fill")
                        .font(.subheadline)
                        .foregroundStyle(Palette.warning)
                }
            case .failure(let problem):
                if !problem.message.isEmpty {
                    Label(problem.message, systemImage: "exclamationmark.triangle.fill")
                        .font(.subheadline)
                        .foregroundStyle(Palette.warning)
                }
            case nil:
                ProgressView().frame(maxWidth: .infinity)
            }
            if let problem {
                Label(problem, systemImage: "exclamationmark.triangle.fill")
                    .font(.subheadline)
                    .foregroundStyle(Palette.warning)
            }
        }
        .padding(16)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.bar)
        .accessibilityElement(children: .contain)
    }

    private var areaMap: some View {
        PackAreaMap(area: binding(\.area))
            .accessibilityLabel("Pack area")
    }

    private func form(_ words: PackWords, mapInline: Bool) -> some View {
        Form {
            Section {
                TextField("Name", text: binding(\.name))
                    .accessibilityIdentifier("pack-name")
            }
            if mapInline {
                Section {
                    areaMap
                        .frame(height: 220)
                        .listRowInsets(EdgeInsets())
                } footer: {
                    Text(words.baseMapNote).readableText()
                }
            }
            zoomSection(words)
            layersSection
            datesSection(words)
            spotsSection
            estimateSection(words)
            Section {
                DrawerNotice(text: words.notice)
                if !mapInline {
                    Text(words.baseMapNote).font(.footnote).readableText()
                }
                if library.wifiOnly {
                    Text(words.wifiOnlyDetail).font(.footnote).readableText()
                }
            }
        }
    }

    private func zoomSection(_ words: PackWords) -> some View {
        Section("Zoom") {
            Picker("Zoom", selection: binding(\.zoom)) {
                ForEach(Array(words.zoomPresets.enumerated()), id: \.offset) { index, preset in
                    Text(preset.title).tag(index)
                }
            }
            .pickerStyle(.inline)
            .labelsHidden()
            if let draft, words.zoomPresets.indices.contains(draft.zoom) {
                Text(words.zoomPresets[draft.zoom].detail).font(.footnote).readableText()
            }
        }
    }

    private var layersSection: some View {
        Section("Layers") {
            let titles = layerTitles
            ForEach(candidates, id: \.self) { id in
                Toggle(titles[id] ?? id, isOn: layerBinding(id))
            }
        }
    }

    private func datesSection(_ words: PackWords) -> some View {
        Section("Days") {
            Picker("Days", selection: dateChoice) {
                ForEach(Array(words.dateOptions.enumerated()), id: \.offset) { index, option in
                    Text(option.title).tag(index)
                }
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            let choice = dateChoice.wrappedValue
            if words.dateOptions.indices.contains(choice) {
                Text(words.dateOptions[choice].detail).font(.footnote).readableText()
            }
            switch draft?.dates {
            case .lastDays(let days):
                Stepper(value: lastDaysBinding, in: 1...max(1, words.maxDays)) {
                    Text(datesText(.lastDays(days: days)))
                }
            case .days(let dates):
                MultiDatePicker("Days", selection: $chosenDays)
                    .onChange(of: chosenDays) { _, days in setChosen(days, max: words.maxDays) }
                Text(datesText(.days(dates: dates))).font(.footnote).readableText()
            default:
                EmptyView()
            }
        }
    }

    @ViewBuilder
    private var spotsSection: some View {
        let spots = store.arranged
        if !spots.isEmpty {
            Section("Spots") {
                ForEach(spots) { spot in
                    Toggle(spot.name, isOn: spotBinding(spot.id))
                }
            }
        }
    }

    /// The layers the plan leaves out, and why.
    @ViewBuilder
    private func estimateSection(_ words: PackWords) -> some View {
        if case .success(let plan) = estimate, !plan.skipped.isEmpty {
            Section {
                ForEach(plan.skipped, id: \.layer) { skip in
                    LabeledContent(skip.title, value: skip.reason)
                }
            }
        }
    }

    private func downloadButton(_ plan: PackPlanInfo) -> some View {
        Button {
            Task { await create() }
        } label: {
            Label("Download", systemImage: "arrow.down.circle.fill")
        }
        .buttonStyle(.borderedProminent)
        .disabled(plan.overBudget || plan.itemCount == 0 || starting)
        .accessibilityIdentifier("pack-download")
    }

    // MARK: - State

    private func start() {
        guard draft == nil, let words else { return }
        let spots = store.arranged
        let around = store.activeSpot ?? spots.first
        var prefill = prefill
        if prefill.name.isEmpty, let area = prefill.area {
            prefill.name =
                spots.first { area.contains(latitude: $0.latitude, longitude: $0.longitude) }?
                .name ?? ""
        }
        let made = PackDraft(
            prefill: prefill, defaultZoom: Int(words.defaultZoomPreset),
            spots: spots.map { (id: $0.id, latitude: $0.latitude, longitude: $0.longitude) },
            around: around.map { (latitude: $0.latitude, longitude: $0.longitude) })
        candidates = made.layers + (made.layers.contains(PackDraft.chart) ? [] : [PackDraft.chart])
        if case .lastDays(let days) = made.dates { lastDays = days }
        draft = made
    }

    private var layerTitles: [String: String] {
        let catalogue = (try? mapCatalogue(settingsJson: store.settingsJSON)) ?? []
        return Dictionary(catalogue.map { ($0.id, $0.title) }, uniquingKeysWith: { a, _ in a })
    }

    private func binding<T>(_ path: WritableKeyPath<PackDraft, T>) -> Binding<T> {
        Binding(
            // INVARIANT: the form is shown only once `start()` has made the draft.
            get: { draft![keyPath: path] },
            set: { draft?[keyPath: path] = $0 })
    }

    private func layerBinding(_ id: String) -> Binding<Bool> {
        Binding(
            get: { draft?.layers.contains(id) ?? false },
            set: { on in
                guard var layers = draft?.layers else { return }
                layers.removeAll { $0 == id }
                if on { layers = candidates.filter { layers.contains($0) || $0 == id } }
                draft?.layers = layers
            })
    }

    private func spotBinding(_ id: String) -> Binding<Bool> {
        Binding(
            get: { draft?.spotIDs.contains(id) ?? false },
            set: { on in
                draft?.spotIDs.removeAll { $0 == id }
                if on { draft?.spotIDs.append(id) }
            })
    }

    /// The index of the draft's day choice among the core's options.
    private var dateChoice: Binding<Int> {
        Binding(
            get: {
                switch draft?.dates {
                case .lastDays: 1
                case .days: 2
                default: 0
                }
            },
            set: { index in
                switch index {
                case 1: draft?.dates = .lastDays(days: lastDays)
                case 2: setChosen(chosenDays, max: words?.maxDays ?? 1)
                default: draft?.dates = .latest
                }
            })
    }

    private var lastDaysBinding: Binding<UInt32> {
        Binding(
            get: { lastDays },
            set: {
                lastDays = $0
                draft?.dates = .lastDays(days: $0)
            })
    }

    /// The chosen days, oldest first, at most `max` (the newest kept).
    private func setChosen(_ days: Set<DateComponents>, max: UInt32) {
        let calendar = Calendar.current
        let dates = days.compactMap { calendar.date(from: $0) }.sorted()
        let kept = dates.suffix(Int(max))
        draft?.dates = .days(dates: kept.map { PackDraft.day($0, calendar: calendar) })
    }

    private func datesText(_ dates: PackDatePolicy) -> String {
        (try? packDatesText(dates: dates, settingsJson: store.settingsJSON)) ?? ""
    }

    private func estimateDraft() async {
        guard let draft, let words else { return }
        try? await Task.sleep(for: .milliseconds(300))
        if Task.isCancelled { return }
        let settings = store.settingsJSON
        switch draft.json(
            id: "draft", presets: words.zoomPresets, settingsJSON: settings,
            now: Int64(Date().timeIntervalSince1970), forEstimate: true)
        {
        case .success(let json):
            let planned = await library.estimate(json, settingsJSON: settings)
            if !Task.isCancelled { estimate = planned }
        case .failure(let problem):
            estimate = .failure(problem)
        }
    }

    /// Keeps the pack and starts its download: the tap the download waits for.
    private func create() async {
        guard let draft, let words, !starting else { return }
        starting = true
        defer { starting = false }
        let settings = store.settingsJSON
        let id = UUID().uuidString.lowercased()
        switch draft.json(
            id: id, presets: words.zoomPresets, settingsJSON: settings,
            now: Int64(Date().timeIntervalSince1970))
        {
        case .success(let json):
            problem = nil
            guard library.add(json) != nil else { return }
            await PackBackground.download(id, settingsJSON: settings)
            onCreated(id)
        case .failure(let failure):
            problem = failure.message
        }
    }
}
