import SwiftUI
@preconcurrency import UnderbiteKit

/// Which rule the editor shows: a saved one, or a new one with an id of its own.
struct AlertEditorRequest: Hashable, Identifiable {
    var ruleID: String
    var isNew: Bool

    var id: String { ruleID }

    static func new() -> AlertEditorRequest {
        AlertEditorRequest(ruleID: UUID().uuidString, isNew: true)
    }
}

/// The alert rules: each with its words, its last state and a switch, "check now", the
/// notification permission with the way to Settings when it is off, and the core's sentence
/// that iOS decides when background checks run. On a wide screen the editor shows beside the
/// list; on a narrow one it is pushed.
struct AlertRulesPage: View {
    @Environment(SpotStore.self) private var store
    @Environment(\.horizontalSizeClass) private var sizeClass
    @Environment(\.scenePhase) private var phase
    @State private var model = AlertRulesModel()
    /// The rule beside the list on a wide screen.
    @State private var selection: AlertEditorRequest?
    /// The rule pushed on a narrow screen.
    @State private var pushed: AlertEditorRequest?
    @State private var width: CGFloat = 0
    @State private var deleting: AlertRuleRow?

    private var trips: TripStore { .shared }

    /// Wide enough for the list and the editor side by side.
    private var wide: Bool { sizeClass == .regular && width >= 700 }

    var body: some View {
        Group {
            if wide {
                HStack(spacing: 0) {
                    list
                        .frame(width: 360)
                    Divider().ignoresSafeArea()
                    Group {
                        if let selection {
                            editor(selection)
                        } else {
                            ContentUnavailableView(
                                "Choose a rule", systemImage: "bell",
                                description: Text("Its conditions show here."))
                        }
                    }
                    .frame(maxWidth: .infinity)
                }
            } else {
                list
            }
        }
        .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { width = $0 }
        .navigationTitle("Alerts")
        .navigationDestination(item: $pushed) { request in
            editor(request)
        }
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Button {
                    open(.new())
                } label: {
                    Label("New rule", systemImage: "plus")
                }
                .accessibilityIdentifier("alerts-new")
            }
        }
        .task {
            #if DEBUG
                if AlertSeeds.wanted { AlertSeeds.seed(into: store) }
            #endif
            reload()
            #if DEBUG
                if let asked = AlertSeeds.requestedRule() {
                    // Once Settings has given the page its width: a rule pushed while it is
                    // still narrow would stay pushed.
                    try? await Task.sleep(for: .seconds(1))
                    open(AlertEditorRequest(ruleID: asked, isNew: false))
                }
            #endif
        }
        .task { await model.refreshPermission() }
        .onChange(of: store.settingsJSON) { reload() }
        .onChange(of: store.spots) { reload() }
        .onChange(of: trips.revision) { reload() }
        .onChange(of: phase) { _, now in
            // Back from Settings, where notifications may have been turned on.
            if now == .active { Task { await model.refreshPermission() } }
        }
        .onChange(of: wide, initial: true) { _, wide in
            guard wide else { return }
            if let pushed {
                selection = pushed
                self.pushed = nil
            }
            if selection == nil, let first = model.rows.first {
                selection = AlertEditorRequest(ruleID: first.id, isNew: false)
            }
        }
        .onChange(of: model.rows.map(\.id)) { _, ids in
            if wide, selection == nil, let first = ids.first {
                selection = AlertEditorRequest(ruleID: first, isNew: false)
            }
        }
        .confirmationDialog(
            "Delete this rule?", isPresented: deletingShown, titleVisibility: .visible,
            presenting: deleting
        ) { row in
            Button("Delete \(row.name)", role: .destructive) { delete(row.id) }
        }
    }

    private func reload() {
        model.reload(settingsJSON: store.settingsJSON, spots: store.arranged, trips: trips)
    }

    private func open(_ request: AlertEditorRequest) {
        selection = request
        if !wide { pushed = request }
    }

    private func editor(_ request: AlertEditorRequest) -> some View {
        AlertRuleEditor(
            request: request, embedded: wide,
            onSaved: { id in
                Task { await model.askIfNeeded() }
                let saved = AlertEditorRequest(ruleID: id, isNew: false)
                if wide { selection = saved } else { pushed = nil }
            },
            onClose: {
                if wide {
                    if request.isNew { selection = model.rows.first.map {
                        AlertEditorRequest(ruleID: $0.id, isNew: false)
                    } }
                } else {
                    pushed = nil
                }
            }
        )
        .id(request)
    }

    // MARK: - The list

    private var list: some View {
        List {
            checkSection
            Section {
                if let problem = model.listProblem {
                    Text(problem).foregroundStyle(Palette.error)
                } else if model.rows.isEmpty {
                    VStack(alignment: .leading, spacing: 8) {
                        Text("No alert rules yet")
                            .font(.headline)
                        Text(
                            "A rule watches a spot, a trip or an area on the map and tells you when all of its conditions hold."
                        )
                        .foregroundStyle(.readableSecondary)
                        Button("New rule") { open(.new()) }
                    }
                    .padding(.vertical, 4)
                }
                ForEach(model.rows, id: \.id) { row in
                    ruleRow(row)
                        .swipeActions(edge: .trailing) {
                            Button("Delete", role: .destructive) { deleting = row }
                        }
                        .contextMenu {
                            Button("Edit", systemImage: "pencil") {
                                open(AlertEditorRequest(ruleID: row.id, isNew: false))
                            }
                            Button("Delete", systemImage: "trash", role: .destructive) {
                                deleting = row
                            }
                        }
                }
            } header: {
                Text("Rules").foregroundStyle(.readableSecondary)
            }
        }
        .listStyle(.insetGrouped)
    }

    private var checkSection: some View {
        Section {
            if model.permission == .off, let texts = model.texts {
                VStack(alignment: .leading, spacing: 6) {
                    Label {
                        Text(texts.notificationsOff)
                    } icon: {
                        Image(systemName: "bell.slash")
                            .foregroundStyle(Palette.warning)
                    }
                    if let url = AlertNotifications.settingsURL {
                        Link(texts.openSettings, destination: url)
                            .accessibilityIdentifier("alerts-open-settings")
                    }
                }
                .padding(.vertical, 2)
            }
            Button {
                Task { await model.checkNow(store: store, trips: trips) }
            } label: {
                HStack {
                    Text(model.checking ? (model.texts?.checking ?? "") : (model.texts?.checkNow ?? ""))
                    Spacer()
                    if model.checking { ProgressView() }
                }
            }
            .disabled(model.checking || model.rows.allSatisfy { !$0.enabled })
            .accessibilityIdentifier("alerts-check-now")
            if let result = model.lastCheck, !model.checking {
                checkResult(result)
            }
        } footer: {
            if let texts = model.texts {
                Text(texts.backgroundTiming)
                    .foregroundStyle(.readableSecondary)
            }
        }
    }

    @ViewBuilder
    private func checkResult(_ result: AlertRulesModel.CheckResult) -> some View {
        if let error = result.error {
            Text(error).foregroundStyle(Palette.error)
        } else if result.notices.isEmpty {
            Text(model.texts?.checkedNothing ?? "")
                .foregroundStyle(.readableSecondary)
        } else {
            ForEach(result.notices, id: \.ruleId) { notice in
                VStack(alignment: .leading, spacing: 2) {
                    Label {
                        Text(notice.title).font(.headline)
                    } icon: {
                        Image(systemName: notice.timeSensitive ? "exclamationmark.triangle.fill" : "bell.fill")
                            .foregroundStyle(notice.timeSensitive ? Palette.warning : Palette.accent)
                    }
                    Text(notice.body)
                        .fixedSize(horizontal: false, vertical: true)
                }
                .accessibilityElement(children: .combine)
            }
        }
    }

    private func ruleRow(_ row: AlertRuleRow) -> some View {
        let chosen = wide && selection?.ruleID == row.id
        return HStack(alignment: .top, spacing: 12) {
            Button {
                open(AlertEditorRequest(ruleID: row.id, isNew: false))
            } label: {
                VStack(alignment: .leading, spacing: 3) {
                    Text(row.name)
                        .font(.headline)
                        .foregroundStyle(Color.primary)
                    Text(row.words.summary)
                        .font(.subheadline)
                        .foregroundStyle(Color.primary)
                    Text(row.words.target)
                        .font(.subheadline)
                        .foregroundStyle(.readableSecondary)
                    HStack(spacing: 6) {
                        Image(systemName: Self.symbol(row.state))
                            .accessibilityHidden(true)
                        Text(row.stateLabel)
                    }
                    .font(.subheadline.weight(.semibold))
                    .foregroundStyle(Self.colour(row.state))
                    if let checked = row.checked {
                        Text(checked)
                            .font(.footnote)
                            .foregroundStyle(.readableSecondary)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityElement(children: .combine)
            .accessibilityHint("Edits the rule")
            Toggle(row.name, isOn: enabledBinding(row))
                .labelsHidden()
                .accessibilityLabel(row.name)
        }
        .padding(.vertical, 4)
        .listRowBackground(chosen ? Palette.accent.opacity(0.14) : nil)
    }

    private func enabledBinding(_ row: AlertRuleRow) -> Binding<Bool> {
        Binding {
            row.enabled
        } set: { on in
            guard
                let json = try? setAlertRuleEnabled(
                    settingsJson: store.settingsJSON, ruleId: row.id, enabled: on)
            else { return }
            try? store.updateSettings(json)
            if on { Task { await model.askIfNeeded() } }
        }
    }

    private var deletingShown: Binding<Bool> {
        Binding { deleting != nil } set: { if !$0 { deleting = nil } }
    }

    private func delete(_ id: String) {
        guard let json = try? removeAlertRule(settingsJson: store.settingsJSON, ruleId: id) else {
            return
        }
        try? store.updateSettings(json)
        if selection?.ruleID == id { selection = nil }
        if pushed?.ruleID == id { pushed = nil }
    }

    static func symbol(_ state: AlertRuleState) -> String {
        switch state {
        case .met: "bell.badge.fill"
        case .notMet: "checkmark.circle"
        case .unchecked, .noData: "questionmark.circle"
        case .orphaned: "exclamationmark.triangle"
        case .notChecked: "clock"
        case .off: "bell.slash"
        }
    }

    static func colour(_ state: AlertRuleState) -> Color {
        switch state {
        case .met: Palette.accent
        case .notMet, .notChecked, .off: Palette.secondaryText
        case .unchecked, .noData: Palette.warning
        case .orphaned: Palette.error
        }
    }
}
