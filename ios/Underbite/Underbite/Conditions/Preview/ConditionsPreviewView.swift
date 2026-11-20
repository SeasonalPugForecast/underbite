import SwiftUI
import UnderbiteKit

/// The conditions page of a place that is not saved, in a sheet of its own. "Save as spot" opens
/// the new-spot flow at the same place with the same name and water type and closes the preview
/// once the spot is saved; closing the preview any other way keeps nothing.
struct ConditionsPreviewView: View {
    let preview: ConditionsPreview
    @Environment(\.dismiss) private var dismiss
    @State private var saving: SpotPrefill?

    var body: some View {
        NavigationStack {
            ConditionsView(spotID: preview.id, preview: preview)
                .toolbar {
                    ToolbarItem(placement: .cancellationAction) {
                        Button("Done") { dismiss() }
                    }
                    ToolbarItem(placement: .confirmationAction) {
                        Button("Save as spot", systemImage: "plus.circle") {
                            saving = preview.savePrefill
                        }
                        .accessibilityIdentifier("preview-save-spot")
                    }
                }
        }
        .sheet(item: $saving) { prefill in
            AddSpotView(prefill: prefill) { _ in dismiss() }
        }
        .onDisappear { preview.close() }
    }
}
