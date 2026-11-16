import Charts
import SwiftUI
import UnderbiteKit

/// A displayed value: the text the view made, then its unit, smaller.
struct QuantityText: View {
    let quantity: ViewModel.Quantity
    var font: Font = .system(.title, design: .rounded, weight: .bold)

    var body: some View {
        if quantity.value == nil && quantity.unit.isEmpty {
            // A sentence (a forecast, an advisory), not a number.
            Text(quantity.text)
                .font(.headline)
                .fixedSize(horizontal: false, vertical: true)
        } else {
            HStack(alignment: .firstTextBaseline, spacing: 4) {
                Text(quantity.text).font(font)
                if !quantity.unit.isEmpty {
                    Text(quantity.unit).font(.subheadline).foregroundStyle(.secondary)
                }
            }
        }
    }
}

/// A card's compact form: the headline, up to two lines and a sparkline.
struct CompactBody: View {
    let compact: ViewModel.Compact

    var body: some View {
        HStack(alignment: .center, spacing: 12) {
            VStack(alignment: .leading, spacing: 4) {
                QuantityText(quantity: compact.headline)
                ForEach(Array(compact.lines.enumerated()), id: \.offset) { _, line in
                    Text(line)
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            Spacer(minLength: 0)
            if compact.spark.count >= 2 {
                Sparkline(values: compact.spark)
                    .frame(width: 84, height: 40)
            }
        }
    }
}

/// A line through values already in display units; no axes, the card's words carry the meaning.
struct Sparkline: View {
    let values: [Double]

    var body: some View {
        Chart(Array(values.enumerated()), id: \.offset) { index, value in
            LineMark(x: .value("Order", index), y: .value("Value", value))
                .interpolationMethod(.monotone)
                .foregroundStyle(.tint)
        }
        .chartXAxis(.hidden)
        .chartYAxis(.hidden)
        .chartYScale(domain: .automatic(includesZero: false))
        .accessibilityHidden(true)
    }
}
