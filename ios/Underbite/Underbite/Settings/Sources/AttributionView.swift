import SwiftUI
import UnderbiteKit

/// Every source with its credit, licence and link, as the core's catalogue words them, the
/// not-for-navigation statement, and the open-source notices.
struct AttributionView: View {
    private static let catalogue = sourceCatalogue()

    var body: some View {
        List {
            Section("Not for navigation") {
                Text("Underbite is not for navigation. Tides, currents, depths, charts and every other value it shows are for planning a day on the water only. They can be wrong, late or missing. Never rely on them to steer a vessel or to judge a hazard, and always follow official charts, notices and local regulations.")
            }
            Section {
                ForEach(Self.catalogue, id: \.id) { source in
                    VStack(alignment: .leading, spacing: 3) {
                        Text(source.name).font(.headline)
                        Text(source.credit).font(.subheadline)
                        if let license = source.license {
                            Text(license).font(.caption).foregroundStyle(.secondary)
                        }
                        if let url = source.url.flatMap(URL.init(string:)) {
                            Link(url.host() ?? source.name, destination: url).font(.caption)
                        }
                    }
                    .accessibilityElement(children: .contain)
                }
            } header: {
                Text("Data sources")
            } footer: {
                Text("Every value is shown with the source it came from. Regulations are links to the agencies' own pages, not data.")
            }
            Section {
                NavigationLink("Acknowledgements") { AcknowledgementsView() }
            }
        }
        .navigationTitle("Attribution")
        .navigationBarTitleDisplayMode(.inline)
    }
}
