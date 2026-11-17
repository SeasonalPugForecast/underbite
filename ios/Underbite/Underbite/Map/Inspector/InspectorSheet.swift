import SwiftUI
import UnderbiteKit

/// The point inspector: each gridded layer's value where the map was tapped, worded by the core
/// with the time it is valid for and its source.
struct InspectorSheet: View {
    let controller: MapController

    init(controller: MapController) {
        self.controller = controller
    }

    var body: some View {
        List(controller.inspection?.rows ?? [], id: \.layer) { row in
            VStack(alignment: .leading, spacing: 4) {
                Text(row.label)
                    .font(.headline)
                Text(row.valueText)
                    .font(.title3.monospacedDigit())
                if !row.timeText.isEmpty {
                    Text(row.timeText)
                        .font(.subheadline)
                        .foregroundStyle(.readableSecondary)
                }
                Text(row.source)
                    .font(.footnote)
                    .foregroundStyle(.readableSecondary)
            }
            .accessibilityElement(children: .combine)
        }
        .listStyle(.plain)
    }
}
