import SwiftUI
@preconcurrency import UnderbiteKit

/// One rule's editor: its name, what it watches (a spot, a trip, or an area drawn on the map),
/// its conditions with thresholds typed in the user's units, how often it may notify, quiet
/// hours and whether notifications name the place. The core reads the thresholds, words the
/// rule as it is being edited and refuses what cannot be saved, with its own messages beside
/// the fields.
struct AlertRuleEditor: View {
    let request: AlertEditorRequest
    /// Shown beside the list (its buttons at the bottom) rather than pushed (in the bar).
    let embedded: Bool
    let onSaved: (String) -> Void
    let onClose: () -> Void

    @Environment(SpotStore.self) private var store
    @State private var draft: AlertDraft?
    @State private var saved: AlertDraft?
    /// The thresholds the core cannot read, as they are typed.
    @State private var typing: [AlertDraft.Field: String] = [:]
    /// What the core refused on saving; cleared by the next edit.
    @State private var refused: [AlertDraft.Field: String] = [:]
    @State private var words: RuleWords?
    @State private var metrics: [AlertMetricInfo] = []
    @State private var drawing = false
    @State private var confirmsDelete = false

    private var trips: TripStore { .shared }

    var body: some View {
        Group {
            if let draft {
                form(draft)
            } else {
                ProgressView()
            }
        }
        // Beside the list, the page keeps its own title.
        .navigationTitle(embedded ? "Alerts" : request.isNew ? "New rule" : "Edit rule")
        .navigationBarTitleDisplayMode(embedded ? .automatic : .inline)
        .toolbar {
            // Pushed, the back button leaves without saving.
            if !embedded {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save", action: save)
                        .disabled(draft == nil)
                        .accessibilityIdentifier("alert-save")
                }
            }
        }
        .safeAreaInset(edge: .bottom) {
            if embedded { bottomBar }
        }
        .onAppear(perform: load)
        .onChange(of: draft) { _, _ in
            refused = [:]
            describe()
        }
        .onChange(of: store.settingsJSON) { _, _ in
            metrics = (try? alertMetrics(settingsJson: store.settingsJSON)) ?? []
            describe()
        }
        .sheet(isPresented: $drawing) {
            AlertBoxPicker(
                initial: draft?.box, around: aroundForBox, settingsJSON: store.settingsJSON
            ) { box in
                draft?.box = box
            }
        }
        .confirmationDialog(
            "Delete this rule?", isPresented: $confirmsDelete, titleVisibility: .visible
        ) {
            Button("Delete", role: .destructive, action: delete)
        }
    }

    private var bottomBar: some View {
        HStack {
            Button(request.isNew ? "Cancel" : "Revert") {
                if request.isNew { onClose() } else { draft = saved }
            }
            .disabled(!request.isNew && draft == saved)
            Spacer()
            Button("Save", action: save)
                .buttonStyle(.borderedProminent)
                .disabled(draft == nil || (!request.isNew && draft == saved))
                .accessibilityIdentifier("alert-save")
        }
        .padding(.horizontal, 20)
        .padding(.vertical, 10)
        .background(.bar)
    }

    // MARK: - Loading, words, saving

    private func load() {
        guard draft == nil else { return }
        metrics = (try? alertMetrics(settingsJson: store.settingsJSON)) ?? []
        let loaded: AlertDraft
        if !request.isNew,
            let document = AlertRuleDocument.find(request.ruleID, inSettings: store.settingsJSON)
        {
            loaded = AlertDraft(document: document, settingsJSON: store.settingsJSON)
        } else {
            loaded = .new(id: request.ruleID, spotID: store.activeID ?? store.arranged.first?.id)
        }
        saved = loaded
        draft = loaded
        describe()
    }

    /// The rule in the core's words as it stands, and the thresholds it cannot read.
    private func describe() {
        guard let draft else { return }
        let (document, problems) = draft.document(settingsJSON: store.settingsJSON)
        typing = problems
        guard let document, let json = try? document.json() else { return }
        words = try? describeAlertRule(
            ruleJson: json, settingsJson: store.settingsJSON,
            spots: AlertRulesModel.spotNames(store.arranged),
            trips: AlertRulesModel.tripNames(trips))
    }

    private func save() {
        guard let draft else { return }
        switch draft.save(into: store.settingsJSON) {
        case .success(let json):
            do {
                try store.updateSettings(json)
                saved = draft
                onSaved(draft.id)
            } catch {
                refused = [.other: AlertWords.message(of: error)]
            }
        case .failure(let problems):
            refused = problems.byField
        }
    }

    private func delete() {
        guard let json = try? removeAlertRule(settingsJson: store.settingsJSON, ruleId: request.ruleID)
        else { return }
        try? store.updateSettings(json)
        onClose()
    }

    private func problem(_ field: AlertDraft.Field) -> String? {
        refused[field] ?? typing[field]
    }

    /// Where the map opens to draw an area: the rule's spot, else the shown one.
    private var aroundForBox: (latitude: Double, longitude: Double)? {
        let spot = draft?.spotID.flatMap { store.spot($0) } ?? store.activeSpot
        return spot.map { (latitude: $0.latitude, longitude: $0.longitude) }
    }

    // MARK: - The form

    private func form(_ current: AlertDraft) -> some View {
        Form {
            if let words {
                Section {
                    VStack(alignment: .leading, spacing: 4) {
                        Text(words.target).font(.headline)
                        Text(words.summary)
                        if let repeating = words.repeat {
                            Text(repeating).foregroundStyle(.readableSecondary)
                        }
                        if let quiet = words.quiet {
                            Text(quiet).foregroundStyle(.readableSecondary)
                        }
                    }
                    .accessibilityElement(children: .combine)
                } header: {
                    Text("Summary")
                }
            }
            Section {
                TextField("Name", text: binding(\.name))
                    .accessibilityIdentifier("alert-name")
                Toggle("On", isOn: binding(\.enabled))
            } header: {
                Text("Rule")
            } footer: {
                problemText(.name)
            }
            targetSection(current)
            conditionsSection(current)
            notifySection(current)
            if let other = problem(.other) {
                Section {
                    Text(other).foregroundStyle(Palette.error)
                }
            }
            if !request.isNew {
                Section {
                    Button("Delete rule", role: .destructive) { confirmsDelete = true }
                }
            }
        }
    }

    private func binding<T>(_ path: WritableKeyPath<AlertDraft, T>) -> Binding<T> {
        Binding {
            // The form shows only once the draft is loaded; the fallbacks are never seen.
            draft.map { $0[keyPath: path] } ?? saved.map { $0[keyPath: path] }
                ?? AlertDraft.new(spotID: nil)[keyPath: path]
        } set: { value in
            draft?[keyPath: path] = value
        }
    }

    @ViewBuilder
    private func problemText(_ field: AlertDraft.Field) -> some View {
        if let message = problem(field) {
            Text(message).foregroundStyle(Palette.error)
        }
    }

    // MARK: - Target

    private func targetSection(_ current: AlertDraft) -> some View {
        Section {
            Picker("Watch", selection: targetKind) {
                Text("Spot").tag(AlertDraft.TargetKind.spot)
                Text("Trip").tag(AlertDraft.TargetKind.trip)
                Text("Area").tag(AlertDraft.TargetKind.box)
            }
            .pickerStyle(.segmented)
            switch current.targetKind {
            case .spot:
                if store.arranged.isEmpty {
                    Text("Add a spot first.").foregroundStyle(.readableSecondary)
                } else {
                    Picker("Spot", selection: binding(\.spotID)) {
                        if current.spotID == nil { Text("Choose a spot").tag(String?.none) }
                        ForEach(store.arranged) { spot in
                            Text(spot.name).tag(Optional(spot.id))
                        }
                    }
                }
            case .trip:
                let names = AlertRulesModel.tripNames(trips)
                if names.isEmpty {
                    Text("Plan a trip first.").foregroundStyle(.readableSecondary)
                } else {
                    Picker("Trip", selection: binding(\.tripID)) {
                        if current.tripID == nil { Text("Choose a trip").tag(String?.none) }
                        ForEach(names, id: \.id) { trip in
                            Text(trip.name).tag(Optional(trip.id))
                        }
                    }
                }
            case .box:
                Button {
                    drawing = true
                } label: {
                    Label(
                        current.box == nil ? "Draw the area on the map" : "Change the area",
                        systemImage: "square.dashed")
                }
                .accessibilityIdentifier("alert-draw-area")
            }
        } header: {
            Text("Watch")
        } footer: {
            VStack(alignment: .leading, spacing: 4) {
                if current.targetKind == .box {
                    Text("An area watches sea surface temperature and chlorophyll from satellites.")
                }
                problemText(.target)
            }
        }
    }

    /// The target kind; conditions the new target cannot have become one it can.
    private var targetKind: Binding<AlertDraft.TargetKind> {
        Binding {
            draft?.targetKind ?? .spot
        } set: { kind in
            guard var next = draft else { return }
            next.targetKind = kind
            let allowed = AlertDraft.kinds(for: kind)
            let gridded = Set(metrics.filter(\.inBoxes).map(\.metric))
            for i in next.conditions.indices {
                if !allowed.contains(next.conditions[i].kind) {
                    next.conditions[i].kind = .measurement
                }
                if kind == .box, !gridded.contains(next.conditions[i].metric) {
                    next.conditions[i].metric = "sea_surface_temp"
                    next.conditions[i].below = ""
                    next.conditions[i].above = ""
                }
            }
            if kind == .trip, next.tripID == nil { next.tripID = trips.trips.first?.id }
            draft = next
        }
    }

    // MARK: - Conditions

    private func conditionsSection(_ current: AlertDraft) -> some View {
        Section {
            ForEach(Array(current.conditions.enumerated()), id: \.element.id) { index, condition in
                AlertConditionRow(
                    condition: conditionBinding(condition.id),
                    kinds: AlertDraft.kinds(for: current.targetKind),
                    metrics: current.targetKind == .box ? metrics.filter(\.inBoxes) : metrics,
                    words: words.flatMap { $0.conditions.indices.contains(index) ? $0.conditions[index] : nil },
                    problem: problem(.condition(condition.id)),
                    belowProblem: problem(.below(condition.id)),
                    aboveProblem: problem(.above(condition.id)),
                    canRemove: current.conditions.count > 1,
                    remove: { draft?.conditions.removeAll { $0.id == condition.id } })
            }
            Button {
                let kind: AlertDraft.ConditionDraft.Kind = .measurement
                var added = AlertDraft.ConditionDraft(kind: kind)
                if current.targetKind == .box { added.metric = "sea_surface_temp" }
                draft?.conditions.append(added)
            } label: {
                Label("Add a condition", systemImage: "plus.circle")
            }
            .accessibilityIdentifier("alert-add-condition")
        } header: {
            Text("When all of these hold")
        } footer: {
            problemText(.conditions)
        }
    }

    private func conditionBinding(_ id: UUID) -> Binding<AlertDraft.ConditionDraft> {
        Binding {
            draft?.conditions.first { $0.id == id } ?? AlertDraft.ConditionDraft(kind: .measurement)
        } set: { value in
            guard let i = draft?.conditions.firstIndex(where: { $0.id == id }) else { return }
            draft?.conditions[i] = value
        }
    }

    // MARK: - Notifying

    private func notifySection(_ current: AlertDraft) -> some View {
        Section {
            Toggle("Remind me while it holds", isOn: repeating)
            if current.repeatHours != nil {
                Stepper(value: repeatHours, in: 1...72) {
                    Text(words?.repeat ?? " ")
                }
                .accessibilityLabel("Hours between reminders")
            }
            Toggle("Quiet hours", isOn: quiet)
            if current.quietStart != nil {
                DatePicker("Start", selection: minutes(\.quietStart), displayedComponents: .hourAndMinute)
                DatePicker("End", selection: minutes(\.quietEnd), displayedComponents: .hourAndMinute)
            }
            Toggle("Name the place in notifications", isOn: binding(\.showName))
        } header: {
            Text("Notify")
        } footer: {
            VStack(alignment: .leading, spacing: 4) {
                problemText(.repeatHours)
                problemText(.quietHours)
                Text(
                    "Quiet hours hold a notification until they end. Off, notifications say \u{201C}your spot\u{201D}, \u{201C}your trip\u{201D} or \u{201C}your area\u{201D} and leave out rule and station names."
                )
            }
        }
    }

    private var repeating: Binding<Bool> {
        Binding {
            draft?.repeatHours != nil
        } set: { on in
            draft?.repeatHours = on ? AlertDraft.defaultRepeatHours : nil
        }
    }

    private var repeatHours: Binding<Int> {
        Binding {
            draft?.repeatHours ?? AlertDraft.defaultRepeatHours
        } set: { hours in
            draft?.repeatHours = hours
        }
    }

    private var quiet: Binding<Bool> {
        Binding {
            draft?.quietStart != nil
        } set: { on in
            draft?.quietStart = on ? AlertDraft.defaultQuietStart : nil
            if on { draft?.quietEnd = AlertDraft.defaultQuietEnd }
        }
    }

    /// A time picker's binding to minutes after midnight, on a fixed day of the phone's
    /// calendar without a clock change.
    private func minutes(_ path: WritableKeyPath<AlertDraft, Int?>) -> Binding<Date> {
        Binding {
            Self.date(minutes: draft?[keyPath: path] ?? 0)
        } set: { date in
            draft?[keyPath: path] = Self.minutes(of: date)
        }
    }

    private func minutes(_ path: WritableKeyPath<AlertDraft, Int>) -> Binding<Date> {
        Binding {
            Self.date(minutes: draft?[keyPath: path] ?? 0)
        } set: { date in
            draft?[keyPath: path] = Self.minutes(of: date)
        }
    }

    private static let day: Date =
        Calendar.current.date(from: DateComponents(year: 2001, month: 1, day: 10)) ?? Date(
            timeIntervalSinceReferenceDate: 0)

    private static func date(minutes: Int) -> Date {
        Calendar.current.date(
            bySettingHour: minutes / 60, minute: minutes % 60, second: 0, of: day) ?? day
    }

    private static func minutes(of date: Date) -> Int {
        let parts = Calendar.current.dateComponents([.hour, .minute], from: date)
        return (parts.hour ?? 0) * 60 + (parts.minute ?? 0)
    }
}

/// One condition of the editor: its kind, and the metric and thresholds, the alert's name or
/// the minutes it needs; the core's sentence for it below.
private struct AlertConditionRow: View {
    @Binding var condition: AlertDraft.ConditionDraft
    let kinds: [AlertDraft.ConditionDraft.Kind]
    let metrics: [AlertMetricInfo]
    let words: String?
    let problem: String?
    let belowProblem: String?
    let aboveProblem: String?
    let canRemove: Bool
    let remove: () -> Void

    private var unit: String {
        metrics.first { $0.metric == condition.metric }?.unit ?? ""
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Picker("Condition", selection: $condition.kind) {
                    ForEach(kinds, id: \.self) { kind in
                        Text(Self.title(kind)).tag(kind)
                    }
                }
                .pickerStyle(.menu)
                .labelsHidden()
                Spacer()
                if canRemove {
                    Button(role: .destructive, action: remove) {
                        Image(systemName: "minus.circle.fill")
                    }
                    .buttonStyle(.borderless)
                    .accessibilityLabel("Remove this condition")
                }
            }
            switch condition.kind {
            case .measurement:
                Picker("Value", selection: $condition.metric) {
                    ForEach(metrics, id: \.metric) { metric in
                        Text(metric.title).tag(metric.metric)
                    }
                }
                threshold("Under", text: $condition.below, problem: belowProblem)
                threshold("Over", text: $condition.above, problem: aboveProblem)
            case .marineAlert:
                TextField("Alert, such as Small Craft Advisory", text: $condition.event)
                    .textInputAutocapitalization(.words)
            case .windowStarts:
                Stepper(value: $condition.withinMinutes, in: 15...720, step: 15) {
                    Text("How soon")
                }
            case .stormAhead, .tripGo:
                EmptyView()
            }
            if let words {
                Text(words)
                    .font(.subheadline)
                    .foregroundStyle(.readableSecondary)
            }
            if let problem {
                Text(problem).font(.subheadline).foregroundStyle(Palette.error)
            }
        }
        .padding(.vertical, 4)
    }

    private func threshold(_ title: String, text: Binding<String>, problem: String?) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack {
                Text(title)
                TextField("None", text: text)
                    .keyboardType(.numbersAndPunctuation)
                    .multilineTextAlignment(.trailing)
                    .accessibilityLabel("\(title), \(unit)")
                Text(unit)
                    .foregroundStyle(.readableSecondary)
                    .accessibilityHidden(true)
            }
            if let problem {
                Text(problem).font(.subheadline).foregroundStyle(Palette.error)
            }
        }
    }

    static func title(_ kind: AlertDraft.ConditionDraft.Kind) -> String {
        switch kind {
        case .measurement: "A measurement"
        case .marineAlert: "A marine alert"
        case .windowStarts: "A fishing window soon"
        case .stormAhead: "A storm or hurricane ahead"
        case .tripGo: "A go day on the trip"
        }
    }
}
