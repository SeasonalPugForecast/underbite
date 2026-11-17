import MapKit
import SwiftUI
import UnderbiteKit

/// The map section: Apple's map, which the layers, stations and inspector build on, with the
/// core's "Not for navigation." notice always in view.
struct MapTab: View {
    @Environment(SpotStore.self) private var store

    var body: some View {
        Map()
            .mapControls {
                MapCompass()
                MapScaleView()
            }
            .safeAreaInset(edge: .bottom) {
                if let notice = (try? appWords(settingsJson: store.settingsJSON))?.mapNotice {
                    // Safety text, whole at every size; the map's own controls stay above it.
                    Text(notice)
                        .font(.footnote.weight(.semibold))
                        .fixedSize(horizontal: false, vertical: true)
                        .multilineTextAlignment(.center)
                        .padding(.horizontal, 12)
                        .padding(.vertical, 6)
                        .background(.regularMaterial, in: Capsule())
                        .padding(.bottom, 8)
                        .accessibilityIdentifier("map-notice")
                }
            }
    }
}
