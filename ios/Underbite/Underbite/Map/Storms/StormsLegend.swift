import SwiftUI
@preconcurrency import UnderbiteKit

/// The storms layer's legend: a sample of each thing the map draws, as it draws it (a model's band
/// in its colour and dash, the storm's wider band, NHC's official track, the observed track, a
/// guidance model's band, NHC's forecast cone) beside the core's words for it, and the link to
/// how NHC makes its cone.
struct StormsLegend: View {
    let layer: StormLayer

    var body: some View {
        if !layer.legend.isEmpty {
            VStack(alignment: .leading, spacing: 6) {
                ForEach(Array(layer.legend.enumerated()), id: \.offset) { _, item in
                    HStack(alignment: .firstTextBaseline, spacing: 8) {
                        StormLegendSample(item: item)
                            .frame(width: 34, height: 14)
                            .alignmentGuide(.firstTextBaseline) { $0[VerticalAlignment.center] + 4 }
                        Text(item.label)
                            .font(.caption)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    .accessibilityElement(children: .combine)
                }
                if layer.legend.contains(where: { $0.kind == .cone }),
                    let url = URL(string: layer.coneLink)
                {
                    Link(layer.coneLinkTitle, destination: url)
                        .font(.caption.weight(.semibold))
                }
            }
            .accessibilityIdentifier("map-storms-legend")
        }
    }
}

/// A short sample of a legend item, drawn as the map draws it.
private struct StormLegendSample: View {
    let item: StormLegendItem

    var body: some View {
        Canvas { context, size in
            let colour = item.colour.color
            var line = Path()
            line.move(to: CGPoint(x: 5, y: size.height / 2))
            line.addLine(to: CGPoint(x: size.width - 5, y: size.height / 2))
            switch item.kind {
            case .modelBand, .guidanceBand, .stormBand:
                let width: Double =
                    item.kind == .stormBand
                    ? StormMapRules.bandWidth(isStorm: true)
                    : item.kind == .guidanceBand
                        ? StormMapRules.aidBandWidth : StormMapRules.bandWidth(isStorm: false)
                let outline = line.strokedPath(
                    StrokeStyle(lineWidth: width, lineCap: .round, lineJoin: .round))
                context.fill(outline, with: .color(colour.opacity(StormMapRules.fillAlpha)))
                context.stroke(
                    outline, with: .color(colour),
                    style: StrokeStyle(
                        lineWidth: StormMapRules.edgeWidth, lineCap: .round,
                        dash: StormMapRules.dash(item.role).map { CGFloat($0) }))
            case .officialTrack:
                context.stroke(
                    line, with: .color(.white),
                    style: StrokeStyle(
                        lineWidth: StormMapRules.officialWidth + 2 * StormMapRules.casingWidth,
                        lineCap: .round))
                context.stroke(
                    line, with: .color(colour),
                    style: StrokeStyle(lineWidth: StormMapRules.officialWidth, lineCap: .round))
            case .observedTrack:
                context.stroke(
                    line, with: .color(colour),
                    style: StrokeStyle(
                        lineWidth: StormMapRules.pastWidth, lineCap: .round,
                        dash: StormMapRules.pastDash.map { CGFloat($0) }))
            case .cone:
                let rect = Path(
                    roundedRect: CGRect(x: 2, y: 1, width: size.width - 4, height: size.height - 2),
                    cornerRadius: size.height / 2)
                context.fill(rect, with: .color(colour.opacity(StormMapRules.coneFillAlpha)))
                context.stroke(
                    rect, with: .color(colour),
                    style: StrokeStyle(lineWidth: StormMapRules.coneEdgeWidth))
            }
        }
        .background(Color(.systemGray5), in: RoundedRectangle(cornerRadius: 3))
        .accessibilityHidden(true)
    }
}
