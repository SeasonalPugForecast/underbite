import SwiftUI

/// The areas in the user's order, with handles to reorder them; tap one to rename it. Removing
/// an area keeps its spots, without an area.
struct AreasView: View {
    @Environment(SpotStore.self) private var store
    @Environment(\.dismiss) private var dismiss
    @State private var renaming: Area?
    @State private var name = ""
    @State private var removing: Area?
    @State private var adding = false

    var body: some View {
        NavigationStack {
            List {
                ForEach(store.arrangedAreas) { area in
                    Button(area.name) { rename(area) }
                        .foregroundStyle(.primary)
                        .accessibilityHint("Renames the area")
                        .contextMenu {
                            Button("Rename", systemImage: "pencil") { rename(area) }
                            Button("Remove area", systemImage: "trash", role: .destructive) {
                                removing = area
                            }
                        }
                }
                .onMove { source, destination in
                    let ids = store.arrangedAreas.map(\.id)
                    store.reorderAreas(
                        SpotGrouping.reordered(all: ids, section: ids, moving: source, to: destination))
                }
                .onDelete { offsets in
                    let areas = store.arrangedAreas
                    removing = offsets.first.flatMap { areas.indices.contains($0) ? areas[$0] : nil }
                }
            }
            .readableText()
            .environment(\.editMode, .constant(.active))
            .overlay {
                if store.areas.isEmpty {
                    ContentUnavailableView(
                        "No areas", systemImage: "folder",
                        description: Text("Group spots by the water you fish."))
                }
            }
            .navigationTitle("Areas")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }
                }
                ToolbarItem(placement: .primaryAction) {
                    Button("New area", systemImage: "folder.badge.plus") {
                        name = ""
                        adding = true
                    }
                }
            }
            .alert("Rename area", isPresented: Binding(get: { renaming != nil }, set: { if !$0 { renaming = nil } })) {
                TextField("Name", text: $name)
                Button("Rename") {
                    if let area = renaming { store.renameArea(area.id, to: name) }
                    renaming = nil
                }
                Button("Cancel", role: .cancel) { renaming = nil }
            }
            .alert("New area", isPresented: $adding) {
                TextField("Name", text: $name)
                Button("Add") {
                    let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
                    if !trimmed.isEmpty { _ = store.addArea(named: trimmed) }
                }
                Button("Cancel", role: .cancel) {}
            } message: {
                Text("Group spots by the water you fish: \"Montauk\", \"Upper Delaware\".")
            }
            .confirmationDialog(
                removing.map { String(localized: "Remove \($0.name)?", comment: "Question asking to remove an area; the argument is its name") } ?? String(localized: "Remove area?", comment: "Question asking to remove an area"),
                isPresented: Binding(get: { removing != nil }, set: { if !$0 { removing = nil } }),
                titleVisibility: .visible, presenting: removing
            ) { area in
                Button("Remove area", role: .destructive) { store.removeArea(area.id) }
            } message: { _ in
                Text("The spots in it stay in the list, without an area.")
            }
        }
    }

    private func rename(_ area: Area) {
        name = area.name
        renaming = area
    }
}
