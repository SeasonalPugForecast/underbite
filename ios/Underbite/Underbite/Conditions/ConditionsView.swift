import SwiftUI

/// The conditions dashboard of one spot.
struct ConditionsView: View {
    let spotID: String

    var body: some View {
        ContentUnavailableView {
            Label("Conditions", systemImage: "water.waves")
        } description: {
            Text("Tides, flow, water temperature and weather for this spot will show here.")
        }
        .navigationTitle("Conditions")
    }
}
