import SwiftUI
@preconcurrency import UnderbiteKit

/// A map layer preset someone sent: its name, the layers it shows and the core's notes on layers
/// left out or switched off, then adding it to the user's presets. A preset with the name of one
/// already here is added under another name or in place of that one, never silently over it.
/// Adding it changes neither the map nor the active preset. Every word is the core's.
struct ReceivedPresetView: View {
    @Environment(SpotStore.self) private var spots
    let preset: ReceivedPreset

    /// Set once the preset is added.
    @Binding var added: Bool
    /// The name it was added under.
    @State private var addedAs: String?
    @State private var naming = false
    @State private var name = ""

    private var presets: MapPresets? { try? mapPresets(settingsJson: spots.settingsJSON) }

    var body: some View {
        let words = PresetSharing.words(settingsJSON: spots.settingsJSON)
        let catalogue = (try? mapCatalogue(settingsJson: spots.settingsJSON)) ?? []
        VStack(alignment: .leading, spacing: 16) {
            LayerPresetCard(
                name: addedAs ?? preset.name,
                titles: LayerPresetCard.titles(preset.shownIDs, catalogue: catalogue),
                notes: preset.notes)
            if let words {
                if added {
                    Label(words.added, systemImage: "checkmark.circle.fill")
                        .font(.headline)
                        .foregroundStyle(Palette.accent)
                } else {
                    actions(words)
                }
            }
        }
        .alert(words?.nameTitle ?? "", isPresented: $naming) {
            TextField(words?.nameField ?? "", text: $name)
            Button(words?.cancel ?? "", role: .cancel) {}
            Button(words?.add ?? "") { add(as: name, replacing: false) }
                .disabled(!(presets.map { PresetSharing.canAdd(as: name, presets: $0) } ?? false))
        } message: {
            Text(words?.nameDetail ?? "")
        }
    }

    @ViewBuilder
    private func actions(_ words: PresetShareWords) -> some View {
        let taken = presets.flatMap { PresetSharing.existing(named: preset.name, presets: $0) }
        VStack(alignment: .leading, spacing: 10) {
            if taken != nil {
                Text(words.exists)
                    .font(.callout)
                    .foregroundStyle(.readableSecondary)
                    .fixedSize(horizontal: false, vertical: true)
                Button {
                    name = preset.name
                    naming = true
                } label: {
                    Text(words.rename).frame(maxWidth: .infinity)
                }
                .buttonStyle(.borderedProminent)
                .controlSize(.large)
                Button {
                    add(as: preset.name, replacing: true)
                } label: {
                    Text(words.replace).frame(maxWidth: .infinity)
                }
                .buttonStyle(.bordered)
                .controlSize(.large)
            } else {
                Button {
                    add(as: preset.name, replacing: false)
                } label: {
                    Text(words.add).frame(maxWidth: .infinity)
                }
                .buttonStyle(.borderedProminent)
                .controlSize(.large)
            }
        }
    }

    private func add(as name: String, replacing: Bool) {
        guard let presets else { return }
        var savedAs: String?
        spots.editMap { map in
            savedAs = PresetSharing.add(
                preset, as: name, replacing: replacing, presets: presets, in: &map)
        }
        guard let savedAs else { return }
        // A built-in preset replaced keeps its own name; anything else is listed as typed.
        addedAs = presets.presets.first { $0.builtIn && $0.id == savedAs }?.name ?? savedAs
        added = true
    }
}
