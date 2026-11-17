import SwiftUI
import UnderbiteKit

/// "Not for navigation." as a banner, while a chart, current or depth layer is on screen: whole at
/// every size, on a solid ground, never in Luna's voice (it is the core's safety text).
struct NavigationNotice: View {
    let text: String

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            Image(systemName: "exclamationmark.triangle.fill")
                .foregroundStyle(Palette.warning)
                .accessibilityHidden(true)
            Text(text)
                .font(.subheadline.weight(.bold))
                .foregroundStyle(Color.primary)
                .fixedSize(horizontal: false, vertical: true)
                .multilineTextAlignment(.leading)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 8)
        .background(Color(.systemBackground), in: RoundedRectangle(cornerRadius: 12))
        .overlay(RoundedRectangle(cornerRadius: 12).strokeBorder(Palette.warning, lineWidth: 2))
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("map-notice")
    }
}

/// The map's own "Not for navigation." when no chart, current or depth layer is on: smaller, at
/// the bottom, still whole and on a solid ground.
struct MapNotice: View {
    let text: String

    var body: some View {
        Text(text)
            .font(.footnote.weight(.semibold))
            .foregroundStyle(Color.primary)
            .fixedSize(horizontal: false, vertical: true)
            .multilineTextAlignment(.center)
            .padding(.horizontal, 12)
            .padding(.vertical, 6)
            .background(Color(.systemBackground), in: Capsule())
            .overlay(Capsule().strokeBorder(Color(.separator)))
            .accessibilityIdentifier("map-notice")
    }
}

/// The credit lines of the layers on screen, each in full.
struct MapCredits: View {
    let credits: [String]

    var body: some View {
        if !credits.isEmpty {
            VStack(alignment: .leading, spacing: 2) {
                ForEach(credits, id: \.self) { credit in
                    Text(credit)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            .font(.caption2)
            .foregroundStyle(Color.primary)
            .padding(.horizontal, 8)
            .padding(.vertical, 5)
            .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 8))
            .accessibilityElement(children: .combine)
            .accessibilityIdentifier("map-credits")
        }
    }
}

/// A grid layer's legend: its title, the colour bar in the settings' ramp and range, the values
/// at its ends and middle in the user's units, and the core's caption.
struct MapLegend: View {
    let layer: LayerInfo

    /// The stops labelled under the bar: the ends and the one nearest the middle.
    private var labelled: [LegendStop] {
        let stops = layer.legend
        guard let first = stops.first, let last = stops.last, stops.count > 1 else { return stops }
        let middle = stops.dropFirst().dropLast().min {
            abs($0.fraction - 0.5) < abs($1.fraction - 0.5)
        }
        return [first] + (middle.map { [$0] } ?? []) + [last]
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(layer.title)
                .font(.caption.weight(.semibold))
            LinearGradient(
                stops: layer.legend.map {
                    Gradient.Stop(color: $0.colour.color, location: CGFloat($0.fraction))
                },
                startPoint: .leading, endPoint: .trailing
            )
            .frame(height: 10)
            .clipShape(Capsule())
            .overlay(Capsule().strokeBorder(Color(.separator)))
            .accessibilityHidden(true)
            HStack {
                ForEach(Array(labelled.enumerated()), id: \.offset) { index, stop in
                    if index > 0 { Spacer(minLength: 4) }
                    Text(stop.label)
                }
            }
            .font(.caption2.monospacedDigit())
            if let caption = layer.legendCaption {
                Text(caption)
                    .font(.caption2)
                    .foregroundStyle(.readableSecondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .padding(8)
        .frame(maxWidth: 280, alignment: .leading)
        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 10))
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("map-legend")
    }
}

extension MapColour {
    /// The colour as SwiftUI draws it (straight alpha, sRGB).
    var color: Color {
        Color(
            .sRGB, red: Double(red) / 255, green: Double(green) / 255, blue: Double(blue) / 255,
            opacity: Double(alpha) / 255)
    }
}
