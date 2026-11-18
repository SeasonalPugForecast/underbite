import SwiftUI
import UnderbiteKit

/// A grid layer's colour range: each end picked from the core's round values in the user's
/// unit. Until one is picked the colours fit the view (the legend says so); a range set here
/// is what the map draws, until "fit" lets the view decide again.
struct LayerRangeRow: View {
    let layer: LayerInfo
    @Environment(SpotStore.self) private var store

    var body: some View {
        if let settings = try? Settings(json: store.settingsJSON),
            let own = LayerRange.ownRange(of: layer.id, settings: settings)
        {
            let choices = LayerRange.choices(for: layer.id, settings: settings)
            let user = LayerRange.userRange(of: layer.id, in: settings.map)
            if choices.count > 1 {
                let low = end(
                    "Low", value: user?.low, choices: choices,
                    allowed: { $0 < (user?.high ?? own.high) }
                ) { value in
                    store.editMap { _ = LayerRange.set(layer.id, low: value, own: own, in: &$0) }
                }
                .accessibilityLabel("\(layer.title) colours from")
                let high = end(
                    "High", value: user?.high, choices: choices,
                    allowed: { $0 > (user?.low ?? own.low) }
                ) { value in
                    store.editMap { _ = LayerRange.set(layer.id, high: value, own: own, in: &$0) }
                }
                .accessibilityLabel("\(layer.title) colours to")
                VStack(alignment: .leading, spacing: 4) {
                    // One line when it fits, else the ends under each other.
                    ViewThatFits(in: .horizontal) {
                        HStack(spacing: 6) {
                            Text("Colours from")
                            low
                            Text("to")
                            high
                        }
                        VStack(alignment: .leading, spacing: 4) {
                            Text("Colours from")
                            low
                            Text("to")
                            high
                        }
                    }
                    .font(.subheadline)
                    if user != nil {
                        Button("Fit the colours to the view") {
                            store.editMap { LayerRange.clear(layer.id, in: &$0) }
                        }
                        .font(.subheadline)
                    }
                }
                .buttonStyle(.borderless)
                .accessibilityIdentifier("map-range-\(layer.id)")
            }
        }
    }

    /// A menu of the values one end can take; its label is the end's value, or `placeholder`
    /// while the colours fit the view.
    private func end(
        _ placeholder: String, value: Double?, choices: [DisplayStop],
        allowed: @escaping (Double) -> Bool, pick: @escaping (Double) -> Void
    ) -> some View {
        let label = value.flatMap { v in
            choices.min { abs($0.value - v) < abs($1.value - v) }?.label
        }
        return Menu {
            ForEach(choices.filter { allowed($0.value) }, id: \.value) { stop in
                Button(stop.label) { pick(stop.value) }
            }
        } label: {
            Text(label ?? placeholder)
                .monospacedDigit()
        }
    }
}
