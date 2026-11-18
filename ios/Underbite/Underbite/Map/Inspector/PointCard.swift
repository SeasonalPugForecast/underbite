import SwiftUI
import UnderbiteKit

/// What the inspector shows about a tapped point: each layer on screen with its value there, the
/// time it is valid for and its source; the nearest station and how far it is; the point's
/// coordinates (exact, local); "Save as spot" and "Measure from here". Every value and word about
/// the point is the core's.
struct PointCard: View {
    let module: InspectorModule
    let settingsJSON: String
    @Environment(SpotStore.self) private var store
    @Environment(\.dismiss) private var dismiss
    @State private var name = ""
    @State private var saved: String?

    var body: some View {
        NavigationStack {
            Group {
                if let pick = module.pick {
                    content(pick)
                } else {
                    Color.clear
                }
            }
            .navigationTitle(module.pick?.place ?? "")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }
                }
            }
        }
    }

    private func content(_ pick: PointPick) -> some View {
        List {
            if !pick.rows.isEmpty {
                Section {
                    ForEach(pick.rows) { row in
                        LayerValueRow(row: row)
                    }
                }
            }
            if let nearest = pick.nearest {
                Section("Nearest station") {
                    HStack(alignment: .top, spacing: 12) {
                        PinSymbol(kind: nearest.pin.kind, role: nearest.pin.role)
                        VStack(alignment: .leading, spacing: 2) {
                            Text(nearest.pin.name)
                                .font(.body.weight(.semibold))
                            Text(nearest.distance)
                                .font(.subheadline.monospacedDigit())
                            if !nearest.pin.label.isEmpty {
                                Text(nearest.pin.label)
                                    .font(.subheadline)
                                    .foregroundStyle(.readableSecondary)
                            }
                        }
                        .fixedSize(horizontal: false, vertical: true)
                    }
                    .accessibilityElement(children: .combine)
                    .accessibilityIdentifier("inspector-nearest")
                }
            }
            Section {
                Text(pick.place)
                    .font(.body.monospacedDigit())
                    .textSelection(.enabled)
                    .accessibilityIdentifier("inspector-point")
                if let spotID = savedSpot {
                    NavigationLink {
                        ConditionsView(spotID: spotID)
                    } label: {
                        Label("Open spot", systemImage: "water.waves")
                    }
                    .accessibilityIdentifier("inspector-open-spot")
                } else {
                    TextField("Spot name", text: $name)
                        .textInputAutocapitalization(.words)
                        .submitLabel(.done)
                        .onSubmit(save)
                        .accessibilityIdentifier("inspector-spot-name")
                    Button("Save as spot", systemImage: "plus.circle", action: save)
                        .disabled(trimmedName.isEmpty)
                        .accessibilityIdentifier("inspector-save-spot")
                }
            }
            Section {
                Button("Measure from here", systemImage: "ruler") {
                    module.measureFromPick()
                }
                .accessibilityIdentifier("inspector-measure")
            }
        }
        .readableText()
    }

    private var trimmedName: String { name.trimmingCharacters(in: .whitespacesAndNewlines) }

    /// The spot saved from this point, while it exists.
    private var savedSpot: String? {
        guard let saved, store.spot(saved) != nil else { return nil }
        return saved
    }

    /// Saves a spot at the point, at its exact coordinates, with the name given. Its kind of
    /// water is not chosen, so the first fetch suggests one.
    private func save() {
        guard let pick = module.pick, !trimmedName.isEmpty else { return }
        let spot = Spot(
            name: trimmedName, latitude: pick.coordinate.latitude,
            longitude: pick.coordinate.longitude, waterType: Spot.unknownWaterType,
            waterTypeChosen: false)
        store.add(spot)
        saved = spot.id
        Task { await store.refresh(only: [spot.id]) }
    }
}

/// One layer's value at the point.
private struct LayerValueRow: View {
    let row: PointRow

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(row.label)
                .font(.headline)
            Text(row.value)
                .font(.title3.monospacedDigit())
            if !row.time.isEmpty {
                Text(row.time)
                    .font(.subheadline)
                    .foregroundStyle(.readableSecondary)
            }
            Text(row.source)
                .font(.footnote)
                .foregroundStyle(.readableSecondary)
        }
        .fixedSize(horizontal: false, vertical: true)
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("inspector-row-\(row.layer)")
    }
}
