import SwiftUI
import UnderbiteKit

/// The depth module's part of the layer drawer (the side panel on the iPad, the sheet on the
/// iPhone): the depth band over the core's stops, written to the settings' `map.depth.band_m`,
/// and the legend with the datum note, the NCEI credit and "Not for navigation.".
struct DepthSection: View {
    let module: DepthModule
    @Environment(SpotStore.self) private var store

    var body: some View {
        if let info = module.info {
            Section {
                let json = module.settingsJSON
                let stops = DepthBand.stops(settingsJSON: json)
                let band = (try? Settings(json: json))?.map.depth.bandM
                DepthBandControl(stops: stops, band: DepthBand.positions(of: band, in: stops)) {
                    positions in
                    store.editMap { DepthBand.set(positions, stops: stops, in: &$0) }
                }
                DepthLegend(info: info)
            } header: {
                Text(info.title)
            }
        }
    }
}
