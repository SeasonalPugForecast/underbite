import SwiftUI
import UnderbiteKit

/// The depth layers' legend: the core's "Approximate depth below the surface", while the shading
/// is shown the colour bar in the settings' depth ramp with the depths at its ends and middle in
/// the user's unit, what the depths in the view are measured from (the datums of the elevation
/// models there, never chart datum), the NCEI credit and "Not for navigation." Every word is the
/// core's (`depth_layer_info`, `depth_datums`); the safety text is whole and in the plain voice.
struct DepthLegend: View {
    let info: DepthLayerInfo
    /// Whether to draw the colour bar (the shading is shown).
    var bar = true
    /// The line saying what the depths are measured from.
    let datums: String

    /// The stops labelled under the bar: the ends and the one nearest the middle.
    private var labelled: [LegendStop] {
        let stops = info.legend
        guard let first = stops.first, let last = stops.last, stops.count > 1 else { return stops }
        let middle = stops.dropFirst().dropLast().min {
            abs($0.fraction - 0.5) < abs($1.fraction - 0.5)
        }
        return [first] + (middle.map { [$0] } ?? []) + [last]
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(info.legendCaption)
                .font(.subheadline.weight(.semibold))
                .fixedSize(horizontal: false, vertical: true)
            if bar && !info.legend.isEmpty {
                LinearGradient(
                    stops: info.legend.map {
                        Gradient.Stop(color: $0.colour.color, location: CGFloat($0.fraction))
                    },
                    startPoint: .leading, endPoint: .trailing
                )
                .frame(height: 12)
                .clipShape(Capsule())
                .overlay(Capsule().strokeBorder(Color(.separator)))
                .accessibilityHidden(true)
                HStack {
                    ForEach(Array(labelled.enumerated()), id: \.offset) { index, stop in
                        if index > 0 { Spacer(minLength: 4) }
                        Text(stop.label)
                    }
                }
                .font(.caption.monospacedDigit())
            }
            Text(datums)
                .font(.caption)
                .foregroundStyle(.readableSecondary)
                .fixedSize(horizontal: false, vertical: true)
                .accessibilityIdentifier("map-depth-datums")
            Text(info.credit)
                .font(.caption)
                .fixedSize(horizontal: false, vertical: true)
            // The credit carries the notice already; it is never shown twice.
            if !info.credit.contains(info.notice) {
                Text(info.notice)
                    .font(.caption.weight(.bold))
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("map-depth-legend")
    }
}
