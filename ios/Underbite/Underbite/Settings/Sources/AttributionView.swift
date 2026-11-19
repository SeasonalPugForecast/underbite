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
                Section("Not for navigation") {
                    Text(page.notForNavigation)
                }
            }
            Section {
                ForEach(Self.catalogue, id: \.id) { source in
                    CreditRow(
                        name: source.name, credit: source.credit, licence: source.license,
                        conditions: source.conditions, site: source.url, terms: source.termsUrl)
                }
            } header: {
                Text("Data sources")
            }
            if let page {
                Section {
                    ForEach(page.datasets, id: \.id) { dataset in
                        CreditRow(
                            name: dataset.name, credit: dataset.credit, licence: dataset.license,
                            conditions: dataset.conditions, site: nil, terms: dataset.termsUrl)
                    }
                } header: {
                    Text("Bundled data and links")
                }
                Section {
                    ForEach(page.mapCredits, id: \.credit) { credit in
                        MapCreditRow(credit: credit)
                    }
                } header: {
                    Text("Map layers")
                } footer: {
                    Text(page.footer)
                }
            }
            Section {
                NavigationLink("Acknowledgements") { AcknowledgementsView() }
            }
        }
        .readableText()
        .navigationTitle("Attribution")
        .navigationBarTitleDisplayMode(.inline)
    }
}

/// One source or dataset: its name, the credit as it asks, its licence, what it asks beyond
/// that, and links to its site and to the page that states its terms.
private struct CreditRow: View {
    let name: String
    let credit: String
    let licence: String?
    let conditions: String?
    let site: String?
    let terms: String?

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
            if let url = terms.flatMap(URL.init(string:)) {
                PageLink(url: url, kind: "Terms", label: "\(name) terms")
            }
            if let url = site.flatMap(URL.init(string:)), site != terms {
                PageLink(url: url, kind: "Website", label: "\(name) website")
            }
        }
        .accessibilityElement(children: .contain)
    }
}

/// The credit of one or more map layers that share it.
private struct MapCreditRow: View {
    let credit: MapCredit

    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(credit.layers.joined(separator: ", ")).font(.headline)
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
            if let url = credit.termsUrl.flatMap(URL.init(string:)) {
                PageLink(url: url, kind: "Terms", label: "\(credit.layers.first ?? "") terms")
            }
        }
        .accessibilityElement(children: .contain)
    }
}

/// A full-height link target, read as the page's name and not its address.
private struct PageLink: View {
    let url: URL
    /// What the page is: "Terms" or "Website".
    let kind: String
    let label: String

    var body: some View {
        Link(destination: url) {
            Label("\(kind): \(url.host() ?? label)", systemImage: "safari")
                .frame(minHeight: 44, alignment: .leading)
                .contentShape(Rectangle())
        }
        .font(.subheadline)
        .accessibilityLabel(Text(label))
    }
}
