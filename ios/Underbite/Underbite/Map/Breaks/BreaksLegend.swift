import SwiftUI
import UnderbiteKit

/// A breaks layer's legend: the core's caption, sample lines at the threshold, twice it and four
/// times it drawn as the map draws them with their strengths in the user's units, the grid and
/// day the breaks were found in ("From Sea surface temperature, Nov 16"), and the credit, which
/// names that grid and says "Not for navigation." Every word is the core's (`breaks_words`, the
/// `BreakSet`); the safety text is whole and in the plain voice.
struct BreaksLegend: View {
    let layer: BreakLayerWords
    /// Where the breaks came from; none until they are found.
    let from: String?
    /// "Not for navigation." (safety text), shown when the credit does not carry it.
    let notice: String

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(layer.legendCaption)
                .font(.subheadline.weight(.semibold))
                .fixedSize(horizontal: false, vertical: true)
            // The samples side by side while they fit, else one under another.
            ViewThatFits(in: .horizontal) {
                HStack(alignment: .center, spacing: 12) { samples }
                VStack(alignment: .leading, spacing: 4) { samples }
            }
            if let from {
                Text(from)
                    .font(.caption)
                    .foregroundStyle(.readableSecondary)
                    .fixedSize(horizontal: false, vertical: true)
                    .accessibilityIdentifier("map-breaks-from-\(layer.id)")
            }
            Text(layer.credit)
                .font(.caption)
                .fixedSize(horizontal: false, vertical: true)
            // The credit carries the notice already; it is never shown twice.
            if !layer.credit.contains(notice) {
                Text(notice)
                    .font(.caption.weight(.bold))
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("map-breaks-legend-\(layer.id)")
    }

    /// A sample line and its strength for each weight, lightest first.
    private var samples: some View {
        ForEach(Array(layer.legend.enumerated()), id: \.offset) { _, stop in
            HStack(spacing: 6) {
                BreakSample(layer: layer.id, emphasis: stop.emphasis)
                    .frame(width: 26, height: 12)
                Text(stop.label)
                    .font(.caption.monospacedDigit())
                    .fixedSize()
            }
        }
    }
}

/// A short line as the map draws a break of `emphasis`: its width and colour on its halo.
private struct BreakSample: View {
    let layer: String
    let emphasis: Double

    var body: some View {
        let style = BreakStyle.of(layer)
        let width = BreakLines.width(emphasis: emphasis)
        Canvas { context, size in
            var path = Path()
            path.move(to: CGPoint(x: 2, y: size.height / 2))
            path.addLine(to: CGPoint(x: size.width - 2, y: size.height / 2))
            context.stroke(
                path, with: .color(Color(style.haloColour)),
                style: StrokeStyle(lineWidth: width + 2 * BreakStyle.halo, lineCap: .round))
            context.stroke(
                path, with: .color(Color(style.line)),
                style: StrokeStyle(lineWidth: width, lineCap: .round))
        }
        .background(Color(.systemGray4), in: RoundedRectangle(cornerRadius: 3))
        .accessibilityHidden(true)
    }
}
