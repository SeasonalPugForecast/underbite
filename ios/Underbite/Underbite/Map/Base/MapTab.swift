import Combine
import MapKit
import SwiftUI
import UnderbiteKit

/// The map section: Apple's map with the layers the settings show, their credits and legend, the
/// core's "Not for navigation." notice always in view, the layer drawer, and the phone's position
/// with its locate button. On the iPhone the map fills the screen with its controls over it and
/// the drawer comes up as a sheet; on a wide iPad the drawer is a side panel beside the map, which
/// keeps taking gestures.
struct MapTab: View {
    @Environment(SpotStore.self) private var store
    @State private var controller: MapController?
    @State private var locator: MapLocator?

    var body: some View {
        Group {
            if let controller, let locator {
                MapScreen(controller: controller, locator: locator)
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
            locator = MapLocator()
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
    @Bindable var locator: MapLocator
    @Environment(SpotStore.self) private var store
    @Environment(\.horizontalSizeClass) private var sizeClass
    @State private var showsLayers = DrawerLaunch.opens
    @State private var mapView: MKMapView?
    @Environment(\.openURL) private var openURL

    /// The room MapKit's own logo and legal link take at the bottom of the map.
    private static let appleLegalHeight: CGFloat = 34

    /// The core's notice for the map as a whole (safety text).
    private var mapNotice: String? {
        (try? appWords(settingsJson: store.settingsJSON))?.mapNotice
    }

    /// The core's words for the sources strip: the short line for the layers on screen.
    private var sources: MapSources? {
        try? mapSources(
            settingsJson: store.settingsJSON, layerIds: controller.plan.shown.map(\.id))
    }

    var body: some View {
        ZStack(alignment: .topTrailing) {
            MapCanvas(controller: controller, locator: locator) { mapView = $0 }
                .ignoresSafeArea()
            // Like Apple Maps' controls: a column at the top trailing edge.
            VStack(spacing: 8) {
                Button {
                    showsLayers.toggle()
                } label: {
                    Image(systemName: "square.3.layers.3d")
                        .font(.title3)
                        .frame(width: 44, height: 44)
                }
                .buttonStyle(.glass)
                .accessibilityLabel("Layers")
                .accessibilityIdentifier("map-layers")
                LocateButton(locator: locator)
                if let inspector = controller.modules.lazy.compactMap({ $0 as? InspectorModule }).first {
                    PinModeButton(module: inspector)
                }
                // Shows itself when the map is turned, and a tap turns it back to north.
                if let mapView {
                    MapCompass(map: mapView)
                        .frame(width: 44, height: 44)
                }
            }
            .dynamicTypeSize(...MapChromeType.largest)
            .padding(.trailing, 12)
            .padding(.top, 8)
        }
            .safeAreaInset(edge: .bottom, spacing: 0) {
                VStack(alignment: .leading, spacing: 6) {
                    TimeControls(controller: controller)
                    if let legend = controller.plan.legend {
                        MapLegend(layer: legend, state: controller.legends[legend.id])
                    }
                    if let sources {
                        MapSourcesStrip(
                            line: sources.line, infoLabel: sources.infoLabel,
                            sheetTitle: sources.sheetTitle, credits: controller.plan.credits,
                            layerNotice: controller.plan.notice, mapNotice: mapNotice)
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
            .overlay {
                InspectorSheet(controller: controller)
            }
            .alert(
                CurrentLocation.Failure.denied.errorDescription ?? "",
                isPresented: $locator.explainsRefused
            ) {
                if let settings = URL(string: UIApplication.openSettingsURLString) {
                    Button("Open Settings") { openURL(settings) }
                }
                Button("OK", role: .cancel) {}
            }
    }
}

/// MapKit's compass for the map view: it shows only while the map is turned from north.
private struct MapCompass: UIViewRepresentable {
    let map: MKMapView

    func makeUIView(context: Context) -> MKCompassButton {
        let button = MKCompassButton(mapView: map)
        button.compassVisibility = .adaptive
        button.accessibilityIdentifier = "map-compass"
        return button
    }

    func updateUIView(_ button: MKCompassButton, context: Context) {
        // Setting it again has the button look at the map's heading afresh.
        button.compassVisibility = .visible
        button.compassVisibility = .adaptive
    }
}

/// The locate button: the first tap asks for location access ("when in use"), then the map
/// centres on the phone's position and follows it; a tap while following stops. With approximate
/// access it asks once for precise location.
private struct LocateButton: View {
    let locator: MapLocator

    var body: some View {
        Button {
            locator.tap()
        } label: {
            Image(systemName: locator.state.symbol)
                .font(.title3)
                .frame(width: 44, height: 44)
                .contentTransition(.symbolEffect(.replace))
        }
        .buttonStyle(.glass)
        .accessibilityLabel("My location")
        .accessibilityValue(locator.state.accessibilityValue)
        .accessibilityIdentifier("map-locate")
    }
}

/// The pin mode button: while it is on, a tap on empty map drops the marker and opens the
/// inspector. A short hint says so when it turns on.
private struct PinModeButton: View {
    let module: InspectorModule
    @State private var hint = false
    @State private var hideHint: Task<Void, Never>?

    var body: some View {
        Button {
            module.pinMode.toggle()
            hideHint?.cancel()
            hint = module.pinMode
            if module.pinMode {
                hideHint = Task {
                    try? await Task.sleep(for: .seconds(3))
                    if !Task.isCancelled { hint = false }
                }
            }
        } label: {
            // Not the pin of the Spots tab: a finger tapping the map.
            Image(systemName: "hand.tap")
                .font(.title3)
                .frame(width: 44, height: 44)
        }
        .buttonStyle(.glass)
        .tint(module.pinMode ? Palette.accent : nil)
        .accessibilityLabel("Drop a pin")
        .accessibilityHint("Tap the map to drop a pin")
        .accessibilityAddTraits(module.pinMode ? .isSelected : [])
        .accessibilityIdentifier("map-pin-mode")
        .overlay(alignment: .trailing) {
            if hint {
                Text("Tap the map to drop a pin")
                    .font(.footnote)
                    .padding(.horizontal, 12)
                    .padding(.vertical, 8)
                    .glassEffect(.regular, in: .capsule)
                    .fixedSize()
                    .offset(x: -52)
                    .allowsHitTesting(false)
                    .transition(.opacity)
            }
        }
        .animation(.default, value: hint)
    }
}
