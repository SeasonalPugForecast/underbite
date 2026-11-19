import SwiftUI
import UnderbiteKit

/// Every source with its credit, licence, conditions and terms, the bundled datasets, the credit
/// of every map layer, the not-for-navigation statement and the open-source notices. Every word
/// is the core's; this view only lays them out.
struct AttributionView: View {
    @Environment(SpotStore.self) private var store

    private static let catalogue = sourceCatalogue()

    var body: some View {
        let page = try? attributionPage(settingsJson: store.settingsJSON)
        List {
            if let page {
                Section(page.notForNavigationTitle) {
                    Text(page.notForNavigation)
                }
                Section(page.sourcesTitle) {
                    ForEach(Self.catalogue, id: \.id) { source in
                        CreditRow(
                            name: source.name, credit: source.credit, licence: source.license,
                            conditions: source.conditions, links: source.links)
                    }
                }
                Section(page.datasetsTitle) {
                    ForEach(page.datasets, id: \.id) { dataset in
                        CreditRow(
                            name: dataset.name, credit: dataset.credit, licence: dataset.license,
                            conditions: dataset.conditions, links: dataset.links)
                    }
                }
                Section {
                    ForEach(page.mapCredits, id: \.credit) { credit in
                        MapCreditRow(credit: credit)
                    }
                } header: {
                    Text(page.mapLayersTitle)
                } footer: {
                    Text(page.footer)
                }
                Section {
                    NavigationLink(page.acknowledgementsTitle) { AcknowledgementsView() }
                }
            }
        }
        .readableText()
        .navigationTitle("Attribution")
        .navigationBarTitleDisplayMode(.inline)
    }
}

/// One source or dataset: its name, the credit as it asks, its licence, what it asks beyond
/// that, and its links.
private struct CreditRow: View {
    let name: String
    let credit: String
    let licence: String?
    let conditions: String?
    let links: [CreditLink]

    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(name).font(.headline)
            Text(credit).font(.subheadline)
            if let licence {
                Text(licence).font(.caption).foregroundStyle(.readableSecondary)
            }
            if let conditions {
                Text(conditions).font(.caption).foregroundStyle(.readableSecondary)
            }
            ForEach(links, id: \.url) { PageLink(link: $0) }
        }
        .accessibilityElement(children: .contain)
    }
}

/// The credit of one or more map layers that share it.
private struct MapCreditRow: View {
    let credit: MapCredit

    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(credit.title).font(.headline)
            Text(credit.credit).font(.subheadline)
            ForEach(credit.acknowledgements, id: \.self) { acknowledgement in
                Text(acknowledgement).font(.caption).foregroundStyle(.readableSecondary)
            }
            if let terms = credit.terms {
                Text(terms).font(.caption).foregroundStyle(.readableSecondary)
            }
            if let notice = credit.notice {
                Text(notice).font(.caption.bold())
            }
            ForEach(credit.links, id: \.url) { PageLink(link: $0) }
        }
        .accessibilityElement(children: .contain)
    }
}

/// A full-height link target, labelled as the core words it.
private struct PageLink: View {
    let link: CreditLink

    var body: some View {
        if let url = URL(string: link.url) {
            Link(destination: url) {
                Label(link.label, systemImage: "safari")
                    .frame(minHeight: 44, alignment: .leading)
                    .contentShape(Rectangle())
            }
            .font(.subheadline)
            .accessibilityLabel(Text(link.accessibility))
        }
    }
}
