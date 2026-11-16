import SwiftUI
import UnderbiteKit

/// Sources, units, cards, limits, privacy and attribution.
struct SettingsView: View {
    var body: some View {
        NavigationStack {
            List {
                Section {
                    Text("Sources, units, cards, limits and privacy will be set here.")
                        .foregroundStyle(.secondary)
                }
                Section("About") {
                    LabeledContent("Core", value: coreVersion())
                }
            }
            .navigationTitle("Settings")
        }
    }
}
