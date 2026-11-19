import Charts
import SwiftUI
import UnderbiteKit

/// A displayed value: the text the view made, then its unit, smaller. One text, so a value
/// too wide for the line at a large text size wraps rather than being cut.
struct QuantityText: View {
    let quantity: ViewModel.Quantity
    var font: Font = .system(.title, design: .rounded, weight: .bold)
    @Environment(\.self) private var environment

    var body: some View {
        if quantity.value == nil && quantity.unit.isEmpty {
            // A sentence (a forecast, an advisory), not a number.
            Text(quantity.text)
                .font(.headline)
                .fixedSize(horizontal: false, vertical: true)
        } else if quantity.unit.isEmpty {
            Text(quantity.text).font(font)
                .fixedSize(horizontal: false, vertical: true)
        } else {
            Text(valueAndUnit)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    /// The value and the unit as one attributed run, not a localised format: the line has no
    /// words of its own, so it must not be looked up in a catalog (a pseudolanguage would
    /// double its placeholders). The unit takes the readable secondary colour, resolved here
    /// because an attribute holds a colour, not a style.
    private var valueAndUnit: AttributedString {
        var value = AttributedString(quantity.text + " ")
        value.font = font
        var unit = AttributedString(quantity.unit)
        unit.font = .subheadline
        unit.foregroundColor = ReadableSecondary().resolve(in: environment)
        return value + unit
    }
}

/// A card's compact form: the headline, up to two lines and a sparkline, which moves under
/// the words at the accessibility text sizes so they keep the card's width.
struct CompactBody: View {
    let compact: ViewModel.Compact
    @Environment(\.dynamicTypeSize) private var typeSize

    var body: some View {
        let stacked = typeSize.isAccessibilitySize
        let layout =
            stacked
            ? AnyLayout(VStackLayout(alignment: .leading, spacing: 8))
            : AnyLayout(HStackLayout(alignment: .center, spacing: 12))
        layout {
            VStack(alignment: .leading, spacing: 4) {
                QuantityText(quantity: compact.headline)
                ForEach(Array(compact.lines.enumerated()), id: \.offset) { _, line in
                    Text(line)
                        .font(.subheadline)
                        .foregroundStyle(.readableSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            .frame(maxWidth: stacked ? .infinity : nil, alignment: .leading)
            if !stacked { Spacer(minLength: 0) }
            if compact.spark.count >= 2 {
                Sparkline(values: compact.spark)
                    .frame(width: stacked ? 140 : 84, height: stacked ? 56 : 40)
            }
        }
    }
}

/// A line through values already in display units; no axes, the card's words carry the meaning.
struct Sparkline: View {
    let values: [Double]
    @Environment(\.sunMode) private var sunMode

    var body: some View {
        Chart(Array(values.enumerated()), id: \.offset) { index, value in
            LineMark(x: .value("Order", index), y: .value("Value", value))
                .interpolationMethod(.monotone)
                .foregroundStyle(.tint)
                .lineStyle(StrokeStyle(lineWidth: 2 * Palette.strokeScale(sunMode)))
        }
        .chartXAxis(.hidden)
        .chartYAxis(.hidden)
        .chartYScale(domain: .automatic(includesZero: false))
        .accessibilityHidden(true)
    }
}
