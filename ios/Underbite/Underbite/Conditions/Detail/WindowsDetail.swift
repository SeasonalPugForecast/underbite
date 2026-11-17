import SwiftUI
import UnderbiteKit

/// The snack-times card's full form: each day's windows with the factors that lined up and the
/// numbers behind them, and the enabled factors that had no data. All text is the view model's.
struct WindowsDetail: View {
    let full: ViewModel.WindowsFull

    var body: some View {
        ForEach(full.days, id: \.date) { day in
            Section(day.date) {
                ForEach(day.windows, id: \.startT) { window in
                    WindowRow(window: window)
                }
            }
        }
    }
}

private struct WindowRow: View {
    let window: ViewModel.WindowVm

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(window.range).font(.headline)
            ForEach(Array(window.factors.enumerated()), id: \.offset) { _, factor in
                VStack(alignment: .leading, spacing: 0) {
                    Text(factor.label).font(.subheadline.weight(.semibold))
                    Text(factor.detail).font(.subheadline).foregroundStyle(.readableSecondary)
                }
            }
            if !window.noData.isEmpty {
                Text(window.noData.joined(separator: ", "))
                    .font(.caption)
                    .foregroundStyle(.readableSecondary)
            }
        }
        .padding(.vertical, 2)
        .accessibilityElement(children: .combine)
    }
}
