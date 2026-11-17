import MapKit
import SwiftUI
import UnderbiteKit

/// The stations and the saved spots on the map: what is shown about a tapped pin. Nothing yet.
struct StationLayer: View {
    let controller: MapController

    init(controller: MapController) {
        self.controller = controller
    }

    var body: some View {
        EmptyView()
    }
}

/// Draws the station and spot layers: tide and current stations, buoys, gauges, saved spots.
/// Nothing yet.
final class StationsModule: MapModule {
    private weak var controller: MapController?

    init(controller: MapController) {
        self.controller = controller
    }

    var layerIDs: Set<String> {
        Set(
            (controller?.catalogue ?? [])
                .filter { $0.draw == .stations || $0.draw == .spots }
                .map(\.id))
    }
}
