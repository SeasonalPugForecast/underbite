import SwiftUI
@preconcurrency import UnderbiteKit

/// The map's key for the beach forecast zones while that layer is on: a chip like the grid
/// layers' legend, a swatch and the core's words for each colour. The layer's credit is in the
/// sources strip below it.
struct BeachKey: View {
    let module: BeachForecastModule
    let controller: MapController

    var body: some View {
        let on = controller.plan.shown.contains { $0.id == BeachZones.layer }
        let entries = BeachKeyModel.entries(
            layerOn: on, legend: module.legend.map { (role: $0.role, label: $0.label) })
        if !entries.isEmpty {
            VStack(alignment: .leading, spacing: 3) {
                ForEach(entries, id: \.role) { entry in
                    HStack(spacing: 6) {
                        BeachSwatch(role: entry.role)
                            .scaleEffect(0.6)
                            .frame(width: 14, height: 14)
                        Text(entry.label)
                            .font(.caption2)
                            .lineLimit(1)
                            .minimumScaleFactor(0.8)
                    }
                }
            }
            .padding(6)
            .frame(maxWidth: 220, alignment: .leading)
            .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 10))
            .dynamicTypeSize(...MapChromeType.largest)
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(entries.map(\.label).joined(separator: ", "))
            .accessibilityIdentifier("map-beach-key")
        }
    }
}
