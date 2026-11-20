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
    @State private var previewing: ConditionsPreview?

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
                WaterHereSection(
                    pick: pick,
                    add: { card in adding = SpotPrefill(card: card, at: pick.coordinate) },
                    preview: { card in startPreview(pick, card: card) })
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
                // With the water here found, its own button sits beside "Add as spot".
                if pick.water.card == nil {
                    Button("Preview conditions", systemImage: "eye") {
                        startPreview(pick, card: nil)
                    }
                    .accessibilityIdentifier("inspector-preview")
                }
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
        .sheet(item: $previewing, onDismiss: { previewing?.close() }) { preview in
            ConditionsPreviewView(preview: preview)
        }
        .task { await openLaunchPreview() }
    }

    /// Opens the conditions at the point without saving it, with the water's name and type
    /// when the water here is found.
    private func startPreview(_ pick: PointPick, card: WaterHereCard?) {
        let prefill =
            card.map { SpotPrefill(card: $0, at: pick.coordinate) }
            ?? SpotPrefill(
                latitude: pick.coordinate.latitude, longitude: pick.coordinate.longitude, name: "",
                waterType: nil)
        previewing = store.makePreview(prefill, place: pick.place)
    }

    /// Debug builds: `-mapPreview` opens the preview once the water here has been looked up
    /// (screenshots).
    private func openLaunchPreview() async {
        #if DEBUG
            guard ProcessInfo.processInfo.arguments.contains("-mapPreview") else { return }
            for _ in 0..<60 {
                try? await Task.sleep(for: .seconds(1))
                if let pick = module.pick, pick.water != .looking, previewing == nil {
                    startPreview(pick, card: pick.water.card)
                    return
                }
            }
        #endif
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
    let preview: (WaterHereCard) -> Void
    @State private var showsAllStocking = false

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
            // Said only when the kind has not already said it.
            if !card.waterTypeText.isEmpty {
                Text(card.waterTypeText)
                    .font(.subheadline)
                    .foregroundStyle(.readableSecondary)
            }
        }
        .fixedSize(horizontal: false, vertical: true)
        .accessibilityElement(children: .combine)
        // The actions come right under the water, before what is long.
        Button(pick.waterWords?.addAsSpot ?? "", systemImage: "plus.circle") { add(card) }
            .accessibilityIdentifier("inspector-water-add")
        Button("Preview conditions", systemImage: "eye") { preview(card) }
            .accessibilityIdentifier("inspector-water-preview")
        let blocks = WaterHereBlocks.after(
            hasStocking: card.stockingTitle != nil && !card.stocking.isEmpty,
            hasLinks: card.linksTitle != nil && !card.links.isEmpty,
            hasCredits: !card.credits.isEmpty)
        ForEach(blocks, id: \.self) { block in
            switch block {
            case .stocking: stocking(card)
            case .regulations: regulations(card)
            case .credits: credits(card)
            }
        }
    }

    @ViewBuilder
    private func stocking(_ card: WaterHereCard) -> some View {
        let lines = WaterHereBlocks.stockingLines(
            card.stocking, showAll: card.stockingShowAll, expanded: showsAllStocking)
        VStack(alignment: .leading, spacing: 4) {
            Text(card.stockingTitle ?? "").font(.headline)
            ForEach(Array(lines.enumerated()), id: \.offset) { _, line in
                Text(line).font(.subheadline)
            }
        }
        .fixedSize(horizontal: false, vertical: true)
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("inspector-water-stocking")
        if let label = card.stockingShowAll, !showsAllStocking {
            Button(label, systemImage: "chevron.down") {
                withAnimation { showsAllStocking = true }
            }
            .accessibilityIdentifier("inspector-water-stocking-all")
        }
    }

    @ViewBuilder
    private func regulations(_ card: WaterHereCard) -> some View {
        if let title = card.linksTitle {
            Text(title).font(.headline)
        }
        if let note = card.linksNote {
            Text(note)
                .font(.subheadline)
                .accessibilityIdentifier("inspector-water-links-note")
        }
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

    @ViewBuilder
    private func credits(_ card: WaterHereCard) -> some View {
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
