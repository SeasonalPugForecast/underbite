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

/// The Dynamic Type sizes the map's own controls grow to: the buttons, the sources strip, the
/// legend and the time controls. The sources sheet and the notice in it are not capped.
enum MapChromeType {
    static let largest = DynamicTypeSize.xxxLarge
}

/// The strip at the bottom of the map: the notice (the warning for a chart, current or depth
/// layer, else the map's own) whole and bold, and under it one or two lines naming the sources of
/// the layers on screen. A tap, or the info button, opens the full credit lines in a sheet.
struct MapSourcesStrip: View {
    /// The core's short line, "Sources: Apple Maps · NASA JPL MUR".
    let line: String
    /// The accessibility label of the info button.
    let infoLabel: String
    /// The title of the sheet.
    let sheetTitle: String
    /// The credit lines in full, each once.
    let credits: [String]
    /// The warning of a chart, current or depth layer (safety text).
    let layerNotice: String?
    /// The map's own notice, shown when there is no layer notice (safety text).
    let mapNotice: String?
    @State private var opensSheet = false

    private var notice: String? { layerNotice ?? mapNotice }

    var body: some View {
        if notice != nil || !credits.isEmpty {
            VStack(alignment: .leading, spacing: 3) {
                if let notice {
                    HStack(alignment: .firstTextBaseline, spacing: 6) {
                        if layerNotice != nil {
                            Image(systemName: "exclamationmark.triangle.fill")
                                .foregroundStyle(Palette.warning)
                                .accessibilityHidden(true)
                        }
                        Text(notice)
                            .fontWeight(.bold)
                            .fixedSize(horizontal: false, vertical: true)
                            .accessibilityIdentifier("map-notice")
                    }
                }
                if !credits.isEmpty {
                    Button {
                        opensSheet = true
                    } label: {
                        HStack(alignment: .firstTextBaseline, spacing: 6) {
                            Text(line)
                                .fontWeight(.regular)
                                .lineLimit(2)
                                .multilineTextAlignment(.leading)
                            Spacer(minLength: 0)
                            Image(systemName: "info.circle")
                                .accessibilityHidden(true)
                        }
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel(line)
                    .accessibilityHint(infoLabel)
                    .accessibilityIdentifier("map-credits")
                }
            }
            .font(.caption2)
            .foregroundStyle(Color.primary)
            .padding(.horizontal, 10)
            .padding(.vertical, 5)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(Color(.systemBackground), in: RoundedRectangle(cornerRadius: 10))
            .overlay {
                if layerNotice != nil {
                    RoundedRectangle(cornerRadius: 10).strokeBorder(Palette.warning, lineWidth: 2)
                }
            }
            .dynamicTypeSize(...MapChromeType.largest)
            .sheet(isPresented: $opensSheet) {
                MapSourcesSheet(
                    title: sheetTitle, credits: credits, layerNotice: layerNotice,
                    mapNotice: mapNotice)
            }
        }
    }
}

/// The credit lines of the layers on screen, each in full, with the notice, at the reader's own
/// text size.
struct MapSourcesSheet: View {
    let title: String
    let credits: [String]
    let layerNotice: String?
    let mapNotice: String?
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 10) {
                    if let notice = layerNotice ?? mapNotice {
                        HStack(alignment: .firstTextBaseline, spacing: 8) {
                            if layerNotice != nil {
                                Image(systemName: "exclamationmark.triangle.fill")
                                    .foregroundStyle(Palette.warning)
                                    .accessibilityHidden(true)
                            }
                            Text(notice)
                                .font(.subheadline.weight(.bold))
                                .fixedSize(horizontal: false, vertical: true)
                        }
                        .accessibilityElement(children: .combine)
                        .accessibilityIdentifier("map-sources-notice")
                    }
                    ForEach(credits, id: \.self) { credit in
                        Text(credit)
                            .font(.footnote)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(16)
            }
            .navigationTitle(title)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }
                }
            }
        }
        .presentationDetents([.medium])
        .presentationDragIndicator(.visible)
        .accessibilityIdentifier("map-sources-sheet")
    }
}

/// A grid layer's legend, compact: its title and the colour bar with the values at its ends (and
/// the middle one where it fits) in the user's units. A tap shows the core's captions under it,
/// another hides them; the accessibility element always reads them.
struct MapLegend: View {
    let layer: LayerInfo
    /// The legend for the range drawn when it is not the settings' own (a range fitted to the
    /// view): its bar and the core's caption saying so.
    var state: MapLegendState?
    @State private var showsCaptions = false

    private var stops: [LegendStop] {
        guard let state, !state.stops.isEmpty else { return layer.legend }
        return state.stops
    }

    /// The ends, and the stop nearest the middle.
    private var ends: [LegendStop] {
        let stops = stops
        guard let first = stops.first, let last = stops.last, stops.count > 1 else { return stops }
        return [first, last]
    }

    private var withMiddle: [LegendStop] {
        let stops = stops
        guard let first = stops.first, let last = stops.last, stops.count > 2 else { return ends }
        let middle = stops.dropFirst().dropLast().min {
            abs($0.fraction - 0.5) < abs($1.fraction - 0.5)
        }
        return [first] + (middle.map { [$0] } ?? []) + [last]
    }

    private var captions: [String] {
        [state?.fittedCaption, layer.legendCaption].compactMap { $0 }
    }

    private func labels(_ shown: [LegendStop]) -> some View {
        HStack(spacing: 4) {
            ForEach(Array(shown.enumerated()), id: \.offset) { index, stop in
                if index > 0 { Spacer(minLength: 4) }
                Text(stop.label).lineLimit(1)
            }
        }
        .font(.caption2.monospacedDigit())
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(layer.title)
                .font(.caption.weight(.semibold))
                .lineLimit(1)
            LinearGradient(
                stops: stops.map {
                    Gradient.Stop(color: $0.colour.color, location: CGFloat($0.fraction))
                },
                startPoint: .leading, endPoint: .trailing
            )
            .frame(height: 8)
            .clipShape(Capsule())
            .overlay(Capsule().strokeBorder(Color(.separator)))
            ViewThatFits(in: .horizontal) {
                labels(withMiddle)
                labels(ends)
            }
            if showsCaptions {
                ForEach(captions, id: \.self) { caption in
                    Text(caption)
                        .font(.caption2)
                        .foregroundStyle(.readableSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
        }
        .padding(6)
        .frame(maxWidth: 220, alignment: .leading)
        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 10))
        .dynamicTypeSize(...MapChromeType.largest)
        .contentShape(Rectangle())
        .onTapGesture { withAnimation { showsCaptions.toggle() } }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(
            ([layer.title] + withMiddle.map(\.label) + captions).joined(separator: ", ")
        )
        .accessibilityAddTraits(.isButton)
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
