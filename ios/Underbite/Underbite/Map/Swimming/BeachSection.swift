import SwiftUI
import UnderbiteKit

/// The beach forecast's part of the layer drawer, under the "Water" group: the button that looks
/// the zones up for the view on screen (the only way anything is asked), what the lookup said,
/// the key of the colours, the layer's words on what the colours mean (safety text, in full)
/// and the credit. Every word is the core's.
struct BeachSection: View {
    let module: BeachForecastModule
    let controller: MapController

    var body: some View {
        let shown = controller.plan.shown.first { $0.id == BeachZones.layer }
        if let layer = shown, let words = try? nearbyWords(settingsJson: controller.settingsJSON) {
            Section {
                Button {
                    module.find()
                } label: {
                    HStack(spacing: 10) {
                        if module.busy {
                            ProgressView()
                        } else {
                            Image(systemName: "water.waves")
                                .accessibilityHidden(true)
                        }
                        Text(module.busy ? words.findingBeaches : words.findBeaches)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
                .disabled(module.busy || !module.canLookUp)
                .accessibilityIdentifier("map-find-beaches")
                ForEach(module.notices, id: \.self) { notice in
                    DrawerNotice(text: notice)
                        .accessibilityIdentifier("map-beach-notice")
                }
                if !module.legend.isEmpty {
                    VStack(alignment: .leading, spacing: 6) {
                        ForEach(module.legend, id: \.role) { item in
                            HStack(spacing: 8) {
                                BeachSwatch(role: BeachRole(core: item.role))
                                Text(item.label).font(.footnote)
                            }
                            .accessibilityElement(children: .combine)
                        }
                    }
                }
                if let caption = layer.legendCaption {
                    DrawerFootnote(text: caption)
                }
                ForEach(module.notes, id: \.self) { note in
                    DrawerFootnote(text: note)
                }
                ForEach(module.credits, id: \.source) { credit in
                    NearbyCreditRow(credit: credit)
                }
            } header: {
                Text(layer.title).drawerHeader()
            }
        }
    }
}
