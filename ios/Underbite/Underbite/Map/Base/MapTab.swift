import MapKit
import SwiftUI

/// The map section: Apple's map, which the layers, stations and inspector build on.
struct MapTab: View {
    var body: some View {
        Map()
            .mapControls {
                MapCompass()
                MapScaleView()
            }
    }
}
