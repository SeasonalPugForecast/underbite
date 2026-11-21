import SwiftUI
@preconcurrency import UnderbiteKit

/// The map's key for the beach forecast zones while that layer is on: a chip like the grid
/// layers' legend, a swatch and the core's words for each colour, two by two. The layer's credit
/// is in the sources strip below it.
struct BeachKey: View {
    let module: BeachForecastModule
    let controller: MapController

    private static func entries(
        _ module: BeachForecastModule, _ controller: MapController
    ) -> [BeachKeyModel.Entry] {
        BeachKeyModel.entries(
            layerOn: controller.plan.shown.contains { $0.id == BeachZones.layer },
            legend: module.legend.map { (role: $0.role, label: $0.label) })
    }

    /// Whether the key has anything to show.
    static func shows(module: BeachForecastModule, controller: MapController) -> Bool {
        !entries(module, controller).isEmpty
    }

    var body: some View {
        let entries = Self.entries(module, controller)
        if !entries.isEmpty {
            LazyVGrid(
                columns: [GridItem(.flexible(), alignment: .topLeading), GridItem(.flexible(), alignment: .topLeading)],
                alignment: .leading, spacing: 4
            ) {
                ForEach(entries, id: \.role) { entry in
                    HStack(alignment: .top, spacing: 4) {
                        BeachSwatch(role: entry.role)
                            .scaleEffect(0.6)
                            .frame(width: 14, height: 14)
                        Text(entry.label)
                            .font(.caption2)
                            .lineLimit(2)
                            .fixedSize(horizontal: false, vertical: true)
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
