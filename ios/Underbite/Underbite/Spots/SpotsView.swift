import SwiftUI

/// The areas and spots; each spot opens its conditions.
struct SpotsView: View {
    var body: some View {
        NavigationStack {
            ContentUnavailableView {
                Label("No spots yet", systemImage: "mappin.and.ellipse")
            } description: {
                Text("The places you fish will be listed here, each with its conditions.")
            }
            .navigationTitle("Spots")
        }
    }
}
