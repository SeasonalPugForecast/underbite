import SwiftUI
import UnderbiteKit

/// The stocking card's full form: the recent and the planned stockings near the spot, each with
/// its waterbody, the view model's summary (species, when, about how many, what size) and its
/// town and county, then the notes on what the numbers mean. With none, the view model's line
/// says so.
struct StockingDetail: View {
    let full: ViewModel.StockingFull

    var body: some View {
        if let empty = full.empty {
            Section { Text(empty) }
        }
        ForEach(Array(full.sections.enumerated()), id: \.offset) { _, section in
            Section(section.title) {
                ForEach(Array(section.rows.enumerated()), id: \.offset) { _, row in
                    StockingRow(row: row)
                }
            }
        }
        if !full.notes.isEmpty {
            Section {
                ForEach(Array(full.notes.enumerated()), id: \.offset) { _, note in
                    Text(note)
                        .font(.footnote)
                        .foregroundStyle(.readableSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
        }
    }
}

/// One stocking: the waterbody, the summary under it and the place in small type.
private struct StockingRow: View {
    let row: ViewModel.Row

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(row.label).font(.subheadline.weight(.semibold))
            Text(row.value).fixedSize(horizontal: false, vertical: true)
            if let detail = row.detail {
                Text(detail).font(.caption).foregroundStyle(.readableSecondary)
            }
        }
        .padding(.vertical, 2)
        .accessibilityElement(children: .combine)
    }
}
