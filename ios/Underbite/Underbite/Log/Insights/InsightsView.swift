import SwiftUI

/// "What worked": the conditions of the angler's own catches. The log links here.
struct InsightsView: View {
    var body: some View {
        ContentUnavailableView(
            "What worked", systemImage: "chart.bar.xaxis",
            description: Text("The conditions of your own catches will show here."))
            .navigationTitle("What worked")
    }
}
