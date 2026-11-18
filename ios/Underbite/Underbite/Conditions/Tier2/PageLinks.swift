import SwiftUI
import UnderbiteKit

/// A source's own pages (a reservoir operator's release schedules, a bloom bulletin), each a
/// link with the view model's label. Nothing when there are none.
struct PageLinks: View {
    let links: [ViewModel.PageLink]

    var body: some View {
        let usable = links.compactMap { link in
            URL(string: link.url).map { (label: link.label, url: $0) }
        }
        if !usable.isEmpty {
            Section {
                ForEach(Array(usable.enumerated()), id: \.offset) { _, link in
                    Link(destination: link.url) {
                        Label(link.label, systemImage: "arrow.up.forward.square")
                    }
                }
            }
        }
    }
}

/// The view model's sections as label and value rows. A row without a label is a sentence of
/// its own (the satellite product's description) and reads as one, whole.
struct LabeledSections: View {
    let sections: [ViewModel.Section]

    var body: some View {
        ForEach(Array(sections.enumerated()), id: \.offset) { _, section in
            Section(section.title) {
                ForEach(Array(section.rows.enumerated()), id: \.offset) { _, row in
                    if row.label.isEmpty {
                        Text(row.value)
                            .font(.subheadline)
                            .fixedSize(horizontal: false, vertical: true)
                    } else {
                        VStack(alignment: .leading, spacing: 2) {
                            LabeledContent(row.label, value: row.value)
                            if let detail = row.detail {
                                Text(detail).font(.caption).foregroundStyle(.readableSecondary)
                            }
                        }
                    }
                }
            }
        }
    }
}
