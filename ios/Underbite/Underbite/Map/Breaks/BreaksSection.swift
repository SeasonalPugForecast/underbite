import SwiftUI
import UnderbiteKit

/// The breaks module's part of the layer drawer (the side panel on the iPad, the sheet on the
/// iPhone): for each breaks layer shown, what is said in place of or beside its lines (show the
/// grid layer, zoom in, finding, none this sharp, few clear cells on a cloudy day), the legend
/// with the grid and day the breaks came from and the credit, and the threshold over the core's
/// stops, written to the settings' `map.breaks`. Every word is the core's.
struct BreaksSection: View {
    let module: BreaksModule
    @Environment(SpotStore.self) private var store

    var body: some View {
        if let words = module.words {
            ForEach(module.visible.layers, id: \.id) { shown in
                if let layer = words.layers.first(where: { $0.id == shown.id }) {
                    section(shown, layer: layer, words: words)
                }
            }
        }
    }

    private func section(
        _ shown: BreaksShown.Layer, layer: BreakLayerWords, words: BreaksWords
    ) -> some View {
        let set = module.sets[shown.id]
        let json = module.settingsJSON
        let stops = BreaksThreshold.stops(layer: shown.id, settingsJSON: json)
        return Section {
            if let status = module.status(of: shown) {
                BreaksStatusRow(
                    status: status, text: status.text(words, layer: layer, set: set))
                    .accessibilityIdentifier("map-breaks-status-\(shown.id)")
            }
            // The legend first: on the iPhone it stays in view at the sheet's middle height.
            BreaksLegend(
                layer: layer, from: shown.sourceDrawn ? set?.fromText : nil, notice: words.notice)
            if let position = BreaksThreshold.position(of: layer.threshold, in: stops) {
                BreaksThresholdControl(
                    stops: stops, position: position, words: words, layer: layer
                ) { to in
                    store.editMap {
                        BreaksThreshold.set(to, stops: stops, layer: shown.id, in: &$0)
                    }
                }
            }
        } header: {
            Text(layer.title)
        }
    }
}

/// What a breaks layer says in place of or beside its lines, with a sign of which it is.
private struct BreaksStatusRow: View {
    let status: BreaksStatus
    let text: String

    var body: some View {
        Label {
            Text(text)
                .font(.subheadline)
                .fixedSize(horizontal: false, vertical: true)
        } icon: {
            switch status {
            case .needsSource:
                Image(systemName: "square.3.layers.3d")
                    .foregroundStyle(.readableSecondary)
            case .zoomIn:
                Image(systemName: "plus.magnifyingglass")
                    .foregroundStyle(.readableSecondary)
            case .loading:
                ProgressView()
                    .controlSize(.small)
            case .noBreaks:
                Image(systemName: "line.diagonal")
                    .foregroundStyle(.readableSecondary)
            case .fewClearCells:
                Image(systemName: "cloud")
                    .foregroundStyle(.readableSecondary)
            }
        }
        .accessibilityElement(children: .combine)
    }
}
