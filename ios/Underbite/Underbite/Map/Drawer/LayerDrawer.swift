import SwiftUI
import UnderbiteKit

/// The layer drawer: the core's catalogue by section, the base maps to choose from and a switch
/// per layer; a layer that cannot be switched on says why. Every word is the catalogue's.
struct LayerDrawer: View {
    let controller: MapController

    init(controller: MapController) {
        self.controller = controller
    }

    /// The catalogue's sections in its order, each with its layers.
    private var sections: [(id: String, title: String, layers: [LayerInfo])] {
        var out: [(id: String, title: String, layers: [LayerInfo])] = []
        for layer in controller.catalogue {
            if let i = out.firstIndex(where: { $0.id == layer.group }) {
                out[i].layers.append(layer)
            } else {
                out.append((layer.group, layer.groupTitle, [layer]))
            }
        }
        return out
    }

    var body: some View {
        List {
            ForEach(sections, id: \.id) { section in
                Section(section.title) {
                    ForEach(section.layers, id: \.id) { layer in
                        LayerRow(layer: layer) { on in controller.setShown(layer.id, on) }
                    }
                }
            }
        }
        .listStyle(.insetGrouped)
    }
}

/// One layer: a switch with the layer's title, or, for a base map, a choice; the reason when it
/// cannot be switched on.
private struct LayerRow: View {
    let layer: LayerInfo
    let set: @MainActor @Sendable (Bool) -> Void

    var body: some View {
        if layer.isBase {
            Button {
                set(true)
            } label: {
                HStack {
                    Text(layer.title)
                        .foregroundStyle(Color.primary)
                    Spacer()
                    if layer.shown {
                        Image(systemName: "checkmark")
                            .foregroundStyle(Palette.accent)
                            .accessibilityHidden(true)
                    }
                }
            }
            .accessibilityAddTraits(layer.shown ? .isSelected : [])
        } else {
            VStack(alignment: .leading, spacing: 4) {
                Toggle(layer.title, isOn: Binding(get: { layer.shown }, set: set))
                    .disabled(!layer.available)
                if let reason = layer.unavailableText {
                    Text(reason)
                        .font(.footnote)
                        .foregroundStyle(.readableSecondary)
                }
            }
        }
    }
}
