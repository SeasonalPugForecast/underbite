import SwiftUI

/// The conditions of one spot: a plain page until the dashboard replaces it. It shows the
/// spot's name, the kind of water and when the data was updated, as the core words them.
struct ConditionsView: View {
    @Environment(SpotStore.self) private var store
    let spotID: String

    var body: some View {
        let spot = store.spot(spotID)
        let listing = store.listing(for: spotID)
        List {
            Section {
                if let label = listing?.waterTypeLabel {
                    LabeledContent("Water", value: label)
                }
                SpotStatusLine(status: listing?.status)
            }
            Section {
                ContentUnavailableView {
                    Label("Conditions", systemImage: "water.waves")
                } description: {
                    Text("Tides, flow, water temperature and weather for this spot will show here.")
                }
            }
        }
        .navigationTitle(spot?.name ?? "Conditions")
        .navigationBarTitleDisplayMode(.inline)
        .refreshable { await store.refresh(only: [spotID]) }
        .task(id: spotID) { await store.show(spotID) }
    }
}
