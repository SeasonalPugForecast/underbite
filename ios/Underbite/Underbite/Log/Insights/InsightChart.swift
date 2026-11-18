import Charts
import SwiftUI

/// One condition of what worked as a card: its title, the catches as bars with the core's
/// labels, the core's sentence and how many catches have no record of it. A bar's length is its
/// fraction of the longest; there is no value axis, so no number is drawn that the core did not
/// write. VoiceOver reads each bar as the core's spoken line.
struct InsightChart: View {
    let section: InsightsPage.Section
    @Environment(\.sunMode) private var sunMode

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(section.title)
                .font(.subheadline.weight(.semibold))
                .foregroundStyle(.readableSecondary)
                .accessibilityAddTraits(.isHeader)
            if section.charted {
                bars
                if let sentence = section.sentence {
                    Text(sentence)
                        .font(.subheadline)
                        .fixedSize(horizontal: false, vertical: true)
                }
            } else if let tooFew = section.tooFew {
                Text(tooFew)
                    .font(.subheadline)
                    .foregroundStyle(.readableSecondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if let unknown = section.unknown {
                Text(unknown)
                    .font(.caption)
                    .foregroundStyle(.readableSecondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(14)
        .background(
            Color(.secondarySystemGroupedBackground),
            in: RoundedRectangle(cornerRadius: 16, style: .continuous))
        .overlay {
            if sunMode {
                RoundedRectangle(cornerRadius: 16, style: .continuous)
                    .strokeBorder(Palette.cardStroke(sunMode), lineWidth: 2)
            }
        }
    }

    /// Horizontal bars, one row per value in the core's order, labelled with the core's words.
    private var bars: some View {
        let keys = section.bars.map(\.key)
        let labels = Dictionary(
            section.bars.map { ($0.key, $0.label) }, uniquingKeysWith: { first, _ in first })
        return Chart(section.bars) { bar in
            BarMark(
                x: .value("Share", bar.fraction),
                y: .value("Value", bar.key),
                height: .ratio(0.7)
            )
            .foregroundStyle(bar.peak ? Palette.accent : Palette.tertiaryText.opacity(0.55))
            .cornerRadius(3, style: .continuous)
            .accessibilityLabel(Text(bar.spoken))
            .accessibilityValue(Text(verbatim: ""))
        }
        .chartXScale(domain: 0...1)
        .chartYScale(domain: keys)
        .chartXAxis(.hidden)
        .chartYAxis {
            AxisMarks(position: .leading, values: keys) { value in
                AxisValueLabel {
                    if let key = value.as(String.self), let label = labels[key] {
                        Text(label)
                            .font(.caption)
                            .foregroundStyle(.readableSecondary)
                    }
                }
            }
        }
        // The labels stop growing at the largest standard size so the bars keep their room;
        // the sentence and VoiceOver carry every value at any size.
        .dynamicTypeSize(...DynamicTypeSize.xxxLarge)
        .frame(height: CGFloat(section.bars.count) * 19 + 6)
    }
}
