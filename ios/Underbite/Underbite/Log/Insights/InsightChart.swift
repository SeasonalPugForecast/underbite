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

    /// Horizontal bars, one row per value in the core's order (the data's order), each on a
    /// faint full-length track with the core's label above it. Both axes are hidden: the labels
    /// are the core's words drawn as annotations, and the length has no scale to read.
    private var bars: some View {
        Chart(section.bars) { bar in
            BarMark(
                x: .value("Share", 1.0),
                y: .value("Value", bar.key),
                height: .fixed(Self.bar),
                stacking: .unstacked
            )
            .foregroundStyle(Palette.tertiaryText.opacity(0.12))
            .cornerRadius(Self.bar / 2, style: .continuous)
            .annotation(position: .top, alignment: .leading, spacing: 3) {
                Text(bar.label)
                    .font(.caption)
                    .foregroundStyle(bar.peak ? Color.primary : Palette.secondaryText)
                    .fontWeight(bar.peak ? .semibold : .regular)
                    .lineLimit(1)
            }
            .accessibilityHidden(true)
            BarMark(
                x: .value("Share", bar.fraction),
                y: .value("Value", bar.key),
                height: .fixed(Self.bar),
                stacking: .unstacked
            )
            .foregroundStyle(bar.peak ? Palette.accent : Palette.tertiaryText.opacity(0.6))
            .cornerRadius(Self.bar / 2, style: .continuous)
            .accessibilityLabel(Text(bar.spoken))
            .accessibilityValue(Text(verbatim: ""))
        }
        .chartXScale(domain: 0...1)
        .chartXAxis(.hidden)
        .chartYAxis(.hidden)
        // The labels stop growing at the largest standard size so the rows keep their height;
        // the sentence and VoiceOver carry every value at any size.
        .dynamicTypeSize(...DynamicTypeSize.xxxLarge)
        .frame(height: CGFloat(section.bars.count) * Self.row)
        // Room for the first row's label, which is drawn above its band.
        .padding(.top, 12)
    }

    /// The height of a bar, and of a row: the bar with its label above.
    private static let bar: CGFloat = 8
    private static let row: CGFloat = 30
}
