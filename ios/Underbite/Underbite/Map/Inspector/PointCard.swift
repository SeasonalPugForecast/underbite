import SwiftUI
@preconcurrency import UnderbiteKit

/// What the inspector shows about a tapped point: the water there (its name and kind, where the
/// point is against it, its stocking lines and the official regulation and licence links, with
/// "Add as spot"); each layer on screen with its value there, the time it is valid for and its
/// source; the nearest station and how far it is; the point's coordinates (exact, local); "Save
/// as spot" and "Measure from here". Every value and word about the point is the core's.
struct PointCard: View {
    let module: InspectorModule
    let settingsJSON: String
    @Environment(SpotStore.self) private var store
    @Environment(\.dismiss) private var dismiss
    @State private var name = ""
    @State private var saved: String?
    @State private var adding: SpotPrefill?

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
            if pick.water.showsSection {
                WaterHereSection(pick: pick) { card in
                    adding = SpotPrefill(card: card, at: pick.coordinate)
                }
            }
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
        .sheet(item: $adding) { prefill in
            AddSpotView(prefill: prefill) { added in saved = added }
        }
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

/// The point's water: the core's loading words, then the water with what applies to it, or the
/// core's failure words. Credits the sources it draws on. Links are links to the agencies'
/// pages, with the date each was checked; the app states no rule itself.
private struct WaterHereSection: View {
    let pick: PointPick
    let add: (WaterHereCard) -> Void

    var body: some View {
        Section(pick.waterWords?.title ?? "") {
            switch pick.water {
            case .looking:
                HStack(spacing: 12) {
                    ProgressView()
                    Text(pick.waterWords?.loading ?? "")
                        .foregroundStyle(.readableSecondary)
                }
                .accessibilityElement(children: .combine)
                .accessibilityIdentifier("inspector-water-loading")
            case .failed(let message):
                Text(message)
                    .foregroundStyle(Palette.warning)
                    .accessibilityIdentifier("inspector-water-failed")
            case .found(let card):
                found(card)
            case .hidden:
                EmptyView()
            }
        }
    }

    @ViewBuilder
    private func found(_ card: WaterHereCard) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(card.title)
                .font(.title3.weight(.semibold))
                .accessibilityIdentifier("inspector-water-title")
            Text(card.kindText)
                .font(.body)
            Text(card.position)
                .font(.subheadline)
                .foregroundStyle(.readableSecondary)
            Text(card.waterTypeText)
                .font(.subheadline)
                .foregroundStyle(.readableSecondary)
        }
        .fixedSize(horizontal: false, vertical: true)
        .accessibilityElement(children: .combine)
        if let title = card.stockingTitle, !card.stocking.isEmpty {
            VStack(alignment: .leading, spacing: 4) {
                Text(title).font(.headline)
                ForEach(Array(card.stocking.enumerated()), id: \.offset) { _, line in
                    Text(line).font(.subheadline)
                }
            }
            .fixedSize(horizontal: false, vertical: true)
            .accessibilityElement(children: .combine)
            .accessibilityIdentifier("inspector-water-stocking")
        }
        if let title = card.linksTitle, !card.links.isEmpty {
            Text(title).font(.headline)
            ForEach(card.links, id: \.url) { link in
                if let url = URL(string: link.url) {
                    Link(destination: url) {
                        VStack(alignment: .leading, spacing: 2) {
                            Text(link.title)
                            Text(link.detail)
                                .font(.footnote)
                                .foregroundStyle(.readableSecondary)
                        }
                    }
                    .accessibilityIdentifier("inspector-water-link")
                }
            }
            if let disclaimer = card.disclaimer {
                Text(disclaimer)
                    .font(.footnote)
                    .foregroundStyle(.readableSecondary)
            }
        }
        Button(pick.waterWords?.addAsSpot ?? "", systemImage: "plus.circle") { add(card) }
            .accessibilityIdentifier("inspector-water-add")
        ForEach(card.credits, id: \.name) { credit in
            VStack(alignment: .leading, spacing: 2) {
                Text(credit.text)
                if let licence = credit.licence {
                    Text(licence)
                }
            }
            .font(.footnote)
            .foregroundStyle(.readableSecondary)
            .fixedSize(horizontal: false, vertical: true)
            .accessibilityElement(children: .combine)
        }
    }
}
