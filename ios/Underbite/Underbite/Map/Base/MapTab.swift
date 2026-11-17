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
                    // Safety text, whole at every size and on a solid ground whatever the map
                    // shows under it; the map's own controls stay above it.
                    Text(notice)
                        .font(.footnote.weight(.semibold))
                        .foregroundStyle(Color.primary)
                        .fixedSize(horizontal: false, vertical: true)
                        .multilineTextAlignment(.center)
                        .padding(.horizontal, 12)
                        .padding(.vertical, 6)
                        .background(Color(.systemBackground), in: Capsule())
                        .overlay(Capsule().strokeBorder(Color(.separator)))
                        .padding(.bottom, 8)
                        .accessibilityIdentifier("map-notice")
                }
            }
    }
}
