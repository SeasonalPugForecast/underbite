import SwiftUI

/// Edits one spot: its name, water type, area, pin and favourite, with its stations and a
/// confirmed delete. Changes are held until Save, and applied to the spot as it is then (the
/// order may have changed meanwhile). A changed water type fetches the spot again.
struct SpotEditorView: View {
    @Environment(SpotStore.self) private var store
    @Environment(\.dismiss) private var dismiss
    let spotID: String

    @State private var name = ""
    @State private var waterType = ""
    @State private var areaID: String?
    @State private var pinned = false
    @State private var favourite = false
    @State private var loaded = false
    @State private var confirmsDelete = false

    var body: some View {
        Form {
            Section("Name") {
                TextField("Name", text: $name)
                    .textInputAutocapitalization(.words)
            }
            Section {
                Picker("Water", selection: $waterType) {
                    ForEach(WaterKind.all, id: \.id) { Text($0.label).tag($0.id) }
                }
                if let hint = suggestion {
                    VStack(alignment: .leading, spacing: 4) {
                        Text("Suggested: \(WaterKind.label(hint.waterType))")
                        ForEach(hint.reasons, id: \.self) { reason in
                            Text(reason).font(.caption).foregroundStyle(.secondary)
                        }
                    }
                    .accessibilityElement(children: .combine)
                    Button("Use the suggestion") { waterType = hint.waterType }
                }
            } header: {
                Text("Kind of water")
            } footer: {
                Text("It decides which cards and sources the spot uses.")
            }
            Section("Area") {
                Picker("Area", selection: $areaID) {
                    Text("None").tag(String?.none)
                    ForEach(store.areas) { Text($0.name).tag(Optional($0.id)) }
                }
            }
            Section {
                Toggle("Pin to the top", isOn: $pinned)
                Toggle("Favourite", isOn: $favourite)
            }
            Section {
                NavigationLink {
                    StationsView(spotID: spotID)
                } label: {
                    Label("Stations", systemImage: "antenna.radiowaves.left.and.right")
                }
            } footer: {
                Text("Choose the stations this spot always uses or never uses.")
            }
            Section {
                Button("Delete spot", role: .destructive) { confirmsDelete = true }
            }
        }
        .navigationTitle("Edit spot")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .cancellationAction) {
                Button("Cancel") { dismiss() }
            }
            ToolbarItem(placement: .confirmationAction) {
                Button("Save", action: save)
                    .disabled(name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
        }
        .confirmationDialog("Delete \(name)?", isPresented: $confirmsDelete, titleVisibility: .visible) {
            Button("Delete spot", role: .destructive) {
                store.remove(spotID)
                dismiss()
            }
        } message: {
            Text("Its saved conditions are deleted from this device too.")
        }
        .onAppear(perform: load)
    }

    /// The core's suggestion, while the user has not chosen a water type.
    private var suggestion: WaterTypeHint? {
        guard let spot = store.spot(spotID), !spot.waterTypeChosen else { return nil }
        guard let hint = store.cache(spotID).suggestion, hint.waterType != waterType else { return nil }
        return hint
    }

    private func load() {
        guard !loaded, let spot = store.spot(spotID) else { return }
        loaded = true
        name = spot.name
        waterType = spot.waterType
        areaID = spot.areaID
        pinned = spot.pinned
        favourite = spot.favourite
    }

    private func save() {
        guard var spot = store.spot(spotID) else { return dismiss() }
        let typeChanged = spot.waterType != waterType
        spot.name = name.trimmingCharacters(in: .whitespacesAndNewlines)
        if typeChanged {
            spot.waterType = waterType
            spot.waterTypeChosen = true
        }
        spot.areaID = areaID
        spot.pinned = pinned
        spot.favourite = favourite
        store.update(spot)
        let id = spotID
        Task {
            if typeChanged {
                await store.refresh(only: [id])
            } else {
                await store.rebuildPage()
            }
        }
        dismiss()
    }
}
