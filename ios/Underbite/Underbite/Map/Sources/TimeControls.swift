import MapKit
import SwiftUI
import UnderbiteKit

/// The date and time controls of the topmost dated layer on the map. Nothing yet: dated layers
/// show their newest picture.
struct TimeControls: View {
    let controller: MapController

    init(controller: MapController) {
        self.controller = controller
    }

    var body: some View {
        EmptyView()
    }
}

/// The dates and times of the dated layers (satellite days, grid times, radar frames). Nothing
/// yet.
final class SourcesModule: MapModule {
    private weak var controller: MapController?

    init(controller: MapController) {
        self.controller = controller
    }

    var layerIDs: Set<String> { [] }
}
