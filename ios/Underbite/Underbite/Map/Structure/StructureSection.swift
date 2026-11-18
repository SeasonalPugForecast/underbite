import SwiftUI
import UnderbiteKit

/// The structure module's part of the layer drawer (the side panel on the iPad, the sheet on
/// the iPhone), one section per layer it draws while that layer is on: the core's caption for
/// it, what its depths are measured from as the source states it (a lake's surface, a state's
/// list) and the source's terms. The switch, the credit and "Not for navigation." are in the
/// layer's own row. Every word is the core's.
struct StructureSection: View {
    let module: StructureModule

    var body: some View {
        ForEach(module.layers, id: \.id) { layer in
            Section {
                if let caption = layer.legendCaption {
                    Text(caption)
                        .font(.footnote)
                        .fixedSize(horizontal: false, vertical: true)
                }
                if let reference = layer.depthReference {
                    Text(reference)
                        .font(.footnote)
                        .foregroundStyle(.readableSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                if let terms = layer.terms {
                    Text(terms)
                        .font(.caption)
                        .foregroundStyle(.readableSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            } header: {
                HStack(spacing: 8) {
                    Text(layer.title)
                    if module.busy {
                        ProgressView()
                            .controlSize(.small)
                    }
                }
            }
            .accessibilityIdentifier("map-structure-\(layer.id)")
        }
    }
}
