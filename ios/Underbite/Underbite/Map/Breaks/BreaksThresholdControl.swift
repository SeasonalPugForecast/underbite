import SwiftUI
import UnderbiteKit

/// A breaks layer's threshold as a slider under its title and the threshold in the user's units
/// ("Breaks from", "1.4 °F per mi"): one thumb that snaps to the core's round stops, weakest on
/// the left, written when the drag ends. The labels are the stops' own and every word is the
/// core's (`breaks_words`, `break_threshold_stops`).
struct BreaksThresholdControl: View {
    let stops: [DisplayStop]
    /// The settings' threshold as a position in `stops`.
    let position: Int
    let words: BreaksWords
    let layer: BreakLayerWords
    let change: (Int) -> Void

    /// The position while the thumb is dragged.
    @State private var dragging: Double?

    private var shown: Int {
        guard let dragging else { return position }
        return Int(dragging.rounded())
    }

    var body: some View {
        if stops.count > 1 {
            VStack(alignment: .leading, spacing: 6) {
                HStack(alignment: .firstTextBaseline) {
                    Text(words.thresholdTitle)
                    Spacer(minLength: 8)
                    Text(dragging == nil ? layer.thresholdText : stops[shown].label)
                        .monospacedDigit()
                        .fontWeight(.semibold)
                }
                .accessibilityHidden(true)
                Slider(
                    value: Binding(
                        get: { dragging ?? Double(position) },
                        set: { dragging = $0 }),
                    in: 0...Double(stops.count - 1), step: 1
                ) {
                    Text(words.thresholdLabel)
                } onEditingChanged: { editing in
                    guard !editing, let dragging else { return }
                    self.dragging = nil
                    let to = Int(dragging.rounded())
                    if to != position { change(to) }
                }
                .tint(Palette.accent)
                .accessibilityLabel(words.thresholdLabel)
                .accessibilityValue(stops[shown].label)
                .accessibilityAdjustableAction { direction in
                    let to = position + (direction == .increment ? 1 : -1)
                    if stops.indices.contains(to) { change(to) }
                }
                .accessibilityIdentifier("map-breaks-threshold-\(layer.id)")
                HStack {
                    Text(stops[0].label)
                    Spacer(minLength: 6)
                    Text(stops[stops.count - 1].label)
                }
                .font(.caption.monospacedDigit())
                .foregroundStyle(.readableSecondary)
                .accessibilityHidden(true)
            }
        }
    }
}
