import Combine
import MapKit
import SwiftUI
import UnderbiteKit

/// The map section: Apple's map with the layers the settings show, their credits and legend, the
/// core's "Not for navigation." notice always in view, and the layer drawer. On the iPhone the map
/// fills the screen with its controls over it and the drawer comes up as a sheet; on a wide iPad
/// the drawer is a side panel beside the map, which keeps taking gestures.
struct MapTab: View {
    @Environment(SpotStore.self) private var store
    @State private var controller: MapController?

    var body: some View {
        Group {
            if let controller {
                MapScreen(controller: controller)
            } else {
                Color.clear
            }
        }
        .onAppear {
            guard controller == nil else { return }
            let start =
                MapLaunch.start
                ?? store.activeSpot.map {
                    MapStart(latitude: $0.latitude, longitude: $0.longitude, span: 0.3)
                } ?? .overview
            let made = MapController(start: start)
            let settings = store
            made.writeSettings = { [weak settings] json in try? settings?.updateSettings(json) }
            made.apply(settingsJSON: store.settingsJSON)
            controller = made
        }
        .onChange(of: store.settingsJSON) { _, json in
            controller?.apply(settingsJSON: json)
        }
        .onReceive(
            NotificationCenter.default.publisher(for: .NSProcessInfoPowerStateDidChange)
                .receive(on: RunLoop.main)
        ) { _ in
            controller?.powerStateChanged()
        }
    }
}

/// The map and what is laid over it.
private struct MapScreen: View {
    @Bindable var controller: MapController
    @Environment(SpotStore.self) private var store
    @Environment(\.horizontalSizeClass) private var sizeClass
    @State private var showsLayers = DrawerLaunch.opens

    /// The room MapKit's own logo and legal link take at the bottom of the map.
    private static let appleLegalHeight: CGFloat = 34

    /// The core's notice for the map as a whole (safety text).
    private var mapNotice: String? {
        (try? appWords(settingsJson: store.settingsJSON))?.mapNotice
    }

    var body: some View {
        MapCanvas(controller: controller)
            .ignoresSafeArea()
            .safeAreaInset(edge: .top, spacing: 0) {
                VStack(alignment: .leading, spacing: 8) {
                    if let notice = controller.plan.notice {
                        NavigationNotice(text: notice)
                            .frame(maxWidth: .infinity)
                    }
                    Button {
                        showsLayers.toggle()
                    } label: {
                        Image(systemName: "square.3.layers.3d")
                            .font(.title3)
                            .frame(width: 48, height: 48)
                    }
                    .buttonStyle(.glass)
                    .accessibilityLabel("Layers")
                    .accessibilityIdentifier("map-layers")
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 12)
                .padding(.top, 8)
            }
            .safeAreaInset(edge: .bottom, spacing: 0) {
                VStack(alignment: .leading, spacing: 8) {
                    TimeControls(controller: controller)
                    if let legend = controller.plan.legend {
                        MapLegend(layer: legend, state: controller.legends[legend.id])
                    }
                    MapCredits(credits: controller.plan.credits)
                    if controller.plan.notice == nil, let notice = mapNotice {
                        MapNotice(text: notice)
                            .frame(maxWidth: .infinity)
                    }
                }
                .frame(maxWidth: sizeClass == .regular ? 520 : .infinity, alignment: .leading)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 12)
                // Apple's logo and legal link stay in view under it.
                .padding(.bottom, Self.appleLegalHeight)
            }
            .overlay {
                StationLayer(controller: controller)
            }
            .inspector(isPresented: $showsLayers) {
                LayerDrawer(controller: controller)
                    .inspectorColumnWidth(min: 300, ideal: 360, max: 440)
                    .presentationDetents([.medium, .large])
                    .presentationDragIndicator(.visible)
                    // The map stays usable beside the sheet's medium height.
                    .presentationBackgroundInteraction(.enabled(upThrough: .medium))
            }
            .sheet(item: $controller.inspection) { _ in
                InspectorSheet(controller: controller)
                    .presentationDetents([.medium, .large])
                    .presentationDragIndicator(.visible)
            }
    }
}
