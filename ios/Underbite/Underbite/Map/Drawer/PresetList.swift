import SwiftUI
import UnderbiteKit

/// The drawer's pages besides the layers.
enum DrawerPage: Hashable {
    case presets
}

/// The preset row at the top of the drawer: the active preset's name, opening the presets.
struct PresetLink: View {
    let open: () -> Void
    @Environment(SpotStore.self) private var store

    private var active: String? {
        guard let presets = try? mapPresets(settingsJson: store.settingsJSON) else { return nil }
        return PresetEditing.entries(presets).first { $0.id == presets.active }?.name
    }

    var body: some View {
        Button(action: open) {
            HStack(spacing: 8) {
                LabeledContent("Preset") {
                    if let active { Text(active) }
                }
                .foregroundStyle(Color.primary)
                Image(systemName: "chevron.forward")
                    .font(.footnote.weight(.semibold))
                    .foregroundStyle(.readableSecondary)
                    .accessibilityHidden(true)
            }
        }
        .accessibilityIdentifier("map-presets")
    }
}

/// The presets: tap one to show it; save the layers on the map as a new one or over one; rename
/// or delete the user's own; put the built-in ones back as they came; share any as a file (a page
/// of its own, from a row's menu or a swipe). Names are the core's for
/// the built-in presets and the user's own as typed.
struct PresetList: View {
    let controller: MapController
    /// Back to the layers.
    let back: () -> Void
    @Environment(SpotStore.self) private var store
    @State private var naming: Naming?
    @State private var name = ""
    @State private var confirmingRestore = false
    @State private var deleting: PresetEntry?
    /// The preset being shared as a file.
    @State private var sharing: PresetEntry?

    /// What a name is being typed for.
    private enum Naming: Equatable {
        case new
        case rename(String)
    }

    private var presets: MapPresets? { try? mapPresets(settingsJson: store.settingsJSON) }

    /// The overlay ids this build has.
    private var known: Set<String> {
        Set(controller.catalogue.filter { !$0.isBase }.map(\.id))
    }

    /// The titles of what a preset shows, as the catalogue words them; layers this build does
    /// not have are left out.
    private func summary(_ entry: PresetEntry) -> String {
        entry.layers.compactMap { layer in
            controller.catalogue.first { $0.id == layer.id && !$0.isBase }?.title
        }.joined(separator: " · ")
    }

    var body: some View {
        Group {
            if let sharing {
                PresetSharePage(entry: sharing, catalogue: controller.catalogue) {
                    withAnimation { self.sharing = nil }
                }
            } else {
                list
            }
        }
        #if DEBUG
            .task {
                guard let id = DrawerLaunch.presetToShare(), let presets,
                    let entry = PresetEditing.entries(presets).first(where: { $0.id == id })
                else { return }
                sharing = entry
            }
        #endif
    }

    private var list: some View {
        let presets = presets
        return List {
            if let presets {
                Section {
                    ForEach(PresetEditing.entries(presets)) { entry in
                        row(entry, presets: presets)
                    }
                } footer: {
                    Text("A preset shows its layers and hides the others.")
                }
                Section {
                    Button("Save the layers on the map as a preset…", systemImage: "plus") {
                        name = ""
                        naming = .new
                    }
                    .accessibilityIdentifier("map-preset-save")
                }
                Section {
                    Button("Restore the built-in presets", role: .destructive) {
                        confirmingRestore = true
                    }
                    .disabled(!PresetEditing.hasEdits(presets))
                } footer: {
                    Text("Your own presets stay.")
                }
            }
        }
        .safeAreaInset(edge: .top, spacing: 0) {
            DrawerHeader(title: "Presets", back: back)
        }
        .alert(
            naming == .new ? "New preset" : "Rename preset",
            isPresented: Binding(get: { naming != nil }, set: { if !$0 { naming = nil } })
        ) {
            TextField("Name", text: $name)
            Button("Cancel", role: .cancel) { naming = nil }
            Button(naming == .new ? "Save" : "Rename") { commitName() }
                .disabled(!nameIsUsable)
        } message: {
            if naming == .new {
                Text("The layers on the map, their opacity and their order.")
            }
        }
        .confirmationDialog(
            "Restore the built-in presets?", isPresented: $confirmingRestore,
            titleVisibility: .visible
        ) {
            Button("Restore the built-in presets", role: .destructive) {
                guard let presets else { return }
                edit { PresetEditing.restoreBuiltIns(presets: presets, in: &$0) }
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("What you saved over them is removed. Your own presets stay.")
        }
        .confirmationDialog(
            "Delete \(deleting?.name ?? "")?",
            isPresented: Binding(get: { deleting != nil }, set: { if !$0 { deleting = nil } }),
            titleVisibility: .visible
        ) {
            Button("Delete", role: .destructive) {
                if let deleting { delete(deleting) }
                deleting = nil
            }
            Button("Cancel", role: .cancel) { deleting = nil }
        }
    }

    private func row(_ entry: PresetEntry, presets: MapPresets) -> some View {
        Button {
            edit { PresetEditing.apply(entry, presets: presets, known: known, to: &$0) }
        } label: {
            HStack {
                VStack(alignment: .leading, spacing: 2) {
                    Text(entry.name)
                        .foregroundStyle(Color.primary)
                    let summary = summary(entry)
                    if !summary.isEmpty {
                        Text(summary)
                            .font(.footnote)
                            .foregroundStyle(.readableSecondary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    if entry.edited {
                        Text("Saved with your layers")
                            .font(.footnote)
                            .foregroundStyle(.readableSecondary)
                    }
                }
                Spacer()
                if entry.id == presets.active {
                    Image(systemName: "checkmark")
                        .foregroundStyle(Palette.accent)
                        .accessibilityHidden(true)
                }
            }
        }
        .accessibilityAddTraits(entry.id == presets.active ? .isSelected : [])
        .accessibilityIdentifier("map-preset-\(entry.id)")
        .contextMenu { actions(entry, presets: presets) }
        .swipeActions(edge: .leading) {
            if let share = shareWord {
                Button(share, systemImage: "square.and.arrow.up") { sharing = entry }
                    .tint(Palette.accent)
            }
        }
        .swipeActions(edge: .trailing) {
            if !entry.builtIn || entry.edited {
                Button(entry.builtIn ? "Restore" : "Delete", role: .destructive) {
                    if entry.builtIn { delete(entry) } else { deleting = entry }
                }
            }
            if !entry.builtIn {
                Button("Rename") { startRename(entry) }
            }
        }
    }

    @ViewBuilder
    private func actions(_ entry: PresetEntry, presets: MapPresets) -> some View {
        Button("Save the layers on the map here", systemImage: "square.and.arrow.down") {
            edit { _ = PresetEditing.save(name: entry.id, presets: presets, in: &$0) }
        }
        if let share = shareWord {
            Button(share, systemImage: "square.and.arrow.up") { sharing = entry }
                .accessibilityIdentifier("map-preset-share-\(entry.id)")
        }
        if !entry.builtIn {
            Button("Rename", systemImage: "pencil") { startRename(entry) }
            Button("Delete", systemImage: "trash", role: .destructive) { deleting = entry }
        } else if entry.edited {
            Button("Restore this preset", systemImage: "arrow.uturn.backward", role: .destructive) {
                delete(entry)
            }
        }
    }

    /// The core's word for sharing a preset.
    private var shareWord: String? {
        PresetSharing.words(settingsJSON: store.settingsJSON)?.share
    }

    private func startRename(_ entry: PresetEntry) {
        name = entry.name
        naming = .rename(entry.id)
    }

    private var nameIsUsable: Bool {
        guard let presets else { return false }
        switch naming {
        case .new: return PresetEditing.savedID(for: name, presets: presets) != nil
        case .rename(let id): return PresetEditing.canRename(id, to: name, presets: presets)
        case nil: return false
        }
    }

    private func commitName() {
        guard let presets, let naming else { return }
        let typed = name
        switch naming {
        case .new:
            edit { _ = PresetEditing.save(name: typed, presets: presets, in: &$0) }
        case .rename(let id):
            edit { _ = PresetEditing.rename(id, to: typed, presets: presets, in: &$0) }
        }
        self.naming = nil
    }

    private func delete(_ entry: PresetEntry) {
        guard let presets else { return }
        let fallback = (try? Settings.defaults())?.map.activePreset ?? presets.active
        edit { PresetEditing.delete(entry.id, presets: presets, fallback: fallback, in: &$0) }
    }

    private func edit(_ change: (inout Settings.MapSettings) -> Void) {
        store.editMap(change)
    }
}
