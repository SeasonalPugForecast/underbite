import SwiftUI
import UnderbiteKit

/// The cards editor: the cards in order with a handle and a switch each, for every water type
/// or for one that has a layout of its own. Edits are saved as they are made, to the layout the
/// configuration says (the water type's own when it has one, else the default list). Settings
/// opens this same view.
struct CustomizeView: View {
    @Environment(SpotStore.self) private var store

    /// The water type edited at first, when opened from a spot.
    var waterType: String?
    /// The core's words for `waterType`, when the caller has them.
    var waterTypeLabel: String?

    @State private var target: String?
    @State private var ownLayout = false
    @State private var editable: [CardChoice] = []
    @State private var choices = CardChoices(all: [])
    @State private var confirmingRestore = false
    @State private var problem: String?
    @State private var started = false
    @State private var editMode: EditMode = .active

    /// The water types of the user's spots, with the core's label for each.
    private var types: [WaterTypeChoice] {
        var seen = Set<String>()
        var out: [WaterTypeChoice] = []
        for spot in store.arranged where seen.insert(spot.waterType).inserted {
            out.append(WaterTypeChoice(id: spot.waterType, label: label(for: spot.waterType)))
        }
        if let waterType, seen.insert(waterType).inserted {
            out.append(WaterTypeChoice(id: waterType, label: label(for: waterType)))
        }
        return out
    }

    private struct WaterTypeChoice: Identifiable {
        var id: String
        var label: String
    }

    private func label(for type: String) -> String {
        if type == waterType, let waterTypeLabel { return waterTypeLabel }
        for spot in store.spots where spot.waterType == type {
            if let found = store.listing(for: spot.id)?.waterTypeLabel { return found }
        }
        return waterTypeLabel ?? ""
    }

    var body: some View {
        List {
            if !types.isEmpty {
                Section {
                    Picker("Cards for", selection: $target) {
                        Text("All water types").tag(String?.none)
                        ForEach(types) { type in
                            Text(type.label).tag(String?.some(type.id))
                        }
                    }
                    if let target {
                        Toggle(
                            "Own layout for \(label(for: target))",
                            isOn: Binding(get: { ownLayout }, set: { setOwnLayout($0) }))
                    }
                } footer: {
                    Text(footer)
                }
            }
            Section {
                ForEach($editable) { $choice in
                    Toggle(choice.title, isOn: $choice.shown)
                }
                .onMove { source, destination in
                    editable.move(fromOffsets: source, toOffset: destination)
                }
            } header: {
                Text("Cards")
            } footer: {
                Text("Drag the handles to reorder. A card with no data for a spot does not show.")
            }
            if let problem {
                Section { Text(problem).foregroundStyle(.red) }
            }
            Section {
                Button("Restore the default layout", role: .destructive) {
                    confirmingRestore = true
                }
            }
        }
        // The list stays in editing mode: handles and switches, one tap away from nothing else.
        .environment(\.editMode, $editMode)
        .navigationTitle("Customize")
        .navigationBarTitleDisplayMode(.inline)
        .confirmationDialog(
            "Restore the default layout?", isPresented: $confirmingRestore, titleVisibility: .visible
        ) {
            Button("Restore the default layout", role: .destructive) { restore() }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("The order and visibility of these cards go back to the defaults.")
        }
        .onChange(of: editable) { _, edited in
            guard started, edited != choices.editable else { return }
            commit(edited)
        }
        .onChange(of: target) { _, _ in
            guard started else { return }
            reload(detectOwn: true)
        }
        .onChange(of: editMode) { _, mode in
            if mode != .active { editMode = .active }
        }
        .onAppear {
            guard !started else { return }
            target = waterType
            reload(detectOwn: true)
            started = true
        }
    }

    private var footer: String {
        guard let target else {
            return "These cards and their order show for every water type that has no layout of its own."
        }
        return ownLayout
            ? "Changes here only affect \(label(for: target))."
            : "Changes here affect every water type without a layout of its own."
    }

    private func currentSettings() -> Settings? {
        do {
            return try Settings(json: store.settingsJSON)
        } catch {
            problem = String(describing: error)
            return nil
        }
    }

    /// Reads the list again from the settings.
    private func reload(detectOwn: Bool) {
        guard let settings = currentSettings() else { return }
        do {
            if detectOwn {
                ownLayout =
                    try target.map { try CardLayoutEditing.usesOwnLayout(settings: settings, waterType: $0) }
                    ?? false
            }
            choices = try CardLayoutEditing.choices(settings: settings, waterType: target)
            editable = choices.editable
            problem = nil
        } catch {
            problem = String(describing: error)
        }
    }

    private func save(_ settings: Settings) {
        do {
            try store.updateSettings(try settings.json())
            Task { await store.rebuildPage() }
            problem = nil
        } catch {
            problem = String(describing: error)
        }
    }

    private func commit(_ edited: [CardChoice]) {
        guard let settings = currentSettings() else { return }
        var all = choices
        all.setEditable(edited)
        var changed = settings
        CardLayoutEditing.apply(all, to: &changed, waterType: target, ownLayout: ownLayout)
        choices = all
        save(changed)
    }

    private func setOwnLayout(_ own: Bool) {
        guard let target, let settings = currentSettings() else { return }
        var changed = settings
        do {
            try CardLayoutEditing.setOwnLayout(
                own, choices: choices, in: &changed, waterType: target)
        } catch {
            problem = String(describing: error)
            return
        }
        ownLayout = own
        save(changed)
        reload(detectOwn: false)
    }

    private func restore() {
        guard let settings = currentSettings() else { return }
        var changed = settings
        do {
            try CardLayoutEditing.restoreDefaults(
                in: &changed, waterType: target, ownLayout: ownLayout)
        } catch {
            problem = String(describing: error)
            return
        }
        save(changed)
        reload(detectOwn: true)
    }
}

/// The editor in a sheet, with Done (a spot's Customize).
struct CustomizeSheet: View {
    var waterType: String?
    var waterTypeLabel: String?
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            CustomizeView(waterType: waterType, waterTypeLabel: waterTypeLabel)
                .toolbar {
                    ToolbarItem(placement: .confirmationAction) {
                        Button("Done") { dismiss() }
                    }
                }
        }
    }
}
