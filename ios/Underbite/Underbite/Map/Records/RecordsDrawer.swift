import SwiftUI
@preconcurrency import UnderbiteKit

/// The records module's sections of the layer drawer, under the "Water" group: for each of its
/// layers on screen, the button that looks it up in the view (the only way anything is asked),
/// what the lookup said, the key of the marks and the layer's legend words. The species records
/// also choose their species with the species screens' picker. Every word is the core's.
struct RecordsDrawerSection: View {
    let module: RecordsModule
    let controller: MapController
    @State private var picking = false

    var body: some View {
        let shown = Dictionary(
            controller.plan.shown.map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })
        let words = try? nearbyWords(settingsJson: controller.settingsJSON)
        if let layer = shown[RecordMarks.recordsLayer] {
            Section {
                recordsRows(layer: layer, words: words)
            } header: {
                Text(layer.title)
            }
        }
        if let layer = shown[RecordMarks.beachesLayer] {
            Section {
                beachRows(layer: layer, words: words)
            } header: {
                Text(layer.title)
            }
        }
    }

    // MARK: - Species records

    @ViewBuilder
    private func recordsRows(layer: LayerInfo, words: NearbyWords?) -> some View {
        let rows = module.speciesRows()
        let species = rows.first { $0.id == module.speciesID }
        Button {
            picking = true
        } label: {
            HStack {
                Label {
                    if let species {
                        VStack(alignment: .leading, spacing: 2) {
                            Text(species.name).foregroundStyle(Color.primary)
                            Text(species.scientificName).font(.caption).italic()
                                .foregroundStyle(.readableSecondary)
                        }
                    } else {
                        Text("Species").foregroundStyle(Color.primary)
                    }
                } icon: {
                    Image(systemName: "fish").foregroundStyle(Palette.accent)
                }
                Spacer()
                Image(systemName: "chevron.forward")
                    .font(.footnote.weight(.semibold))
                    .foregroundStyle(.readableSecondary)
                    .accessibilityHidden(true)
            }
            .contentShape(Rectangle())
        }
        .accessibilityIdentifier("map-records-species")
        .sheet(isPresented: $picking) {
            NavigationStack {
                SpeciesPicker(
                    rows: rows,
                    ids: Binding(
                        get: { module.speciesID.map { [$0] } ?? [] },
                        set: { module.setSpecies($0.first) }),
                    allowsMany: false)
            }
        }
        if let words {
            LookupButton(
                title: words.findRecords, busy: words.findingRecords,
                finding: module.findingRecords,
                enabled: species != nil && module.canLookUp(RecordMarks.recordsLayer)
            ) {
                module.findRecords()
            }
            .accessibilityIdentifier("map-records-find")
        }
        ForEach(module.recordsNotices, id: \.self) { notice in
            DrawerNotice(text: notice)
        }
        if !module.recordSources.isEmpty {
            VStack(alignment: .leading, spacing: 6) {
                ForEach(module.recordSources) { source in
                    HStack(spacing: 8) {
                        RecordSymbol(kind: source.kind)
                        Text(source.name).font(.footnote)
                    }
                    .accessibilityElement(children: .combine)
                }
            }
        }
        if let caption = layer.legendCaption {
            DrawerFootnote(text: caption)
        }
        ForEach(module.recordsNotes, id: \.self) { note in
            DrawerFootnote(text: note)
        }
    }

    // MARK: - Beach forecasts

    @ViewBuilder
    private func beachRows(layer: LayerInfo, words: NearbyWords?) -> some View {
        if let words {
            LookupButton(
                title: words.findBlooms, busy: words.findingBlooms,
                finding: module.findingBeaches,
                enabled: module.canLookUp(RecordMarks.beachesLayer)
            ) {
                module.findBeaches()
            }
            .accessibilityIdentifier("map-beaches-find")
        }
        ForEach(module.beachNotices, id: \.self) { notice in
            DrawerNotice(text: notice)
        }
        if let caption = layer.legendCaption {
            // Safety text: in full, never shortened.
            DrawerFootnote(text: caption)
        }
    }
}

/// A button that starts a lookup, worded by the core, showing its "looking" words while it runs.
private struct LookupButton: View {
    let title: String
    let busy: String
    let finding: Bool
    let enabled: Bool
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            if finding {
                HStack(spacing: 8) {
                    ProgressView()
                    Text(busy)
                }
            } else {
                Label(title, systemImage: "magnifyingglass")
            }
        }
        .disabled(finding || !enabled)
    }
}
