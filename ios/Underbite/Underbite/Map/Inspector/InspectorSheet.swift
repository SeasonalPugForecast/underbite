import SwiftUI
import UnderbiteKit

/// The point inspector and the measuring tool over the map: the inspector as a sheet on the
/// iPhone and a popover pointing at the tapped point on the iPad; the measured line's card as a
/// low sheet on the iPhone (the map stays usable above it, so taps move the line's end) and a
/// panel in the map's corner on the iPad.
struct InspectorSheet: View {
    let controller: MapController

    init(controller: MapController) {
        self.controller = controller
    }

    private var module: InspectorModule? {
        controller.modules.lazy.compactMap { $0 as? InspectorModule }.first
    }

    var body: some View {
        if let module {
            InspectorLayer(module: module, controller: controller)
        }
    }
}

private struct InspectorLayer: View {
    let module: InspectorModule
    let controller: MapController
    @Environment(\.horizontalSizeClass) private var sizeClass
    @State private var stops = PointSheetStops()

    /// The room MapKit's own logo and legal link take at the bottom of the map.
    private static let appleLegalHeight: CGFloat = 34

    private var panel: Bool { sizeClass == .regular }

    private static let glance = PresentationDetent.height(330)
    private static let water = PresentationDetent.fraction(0.6)

    /// The sheet's resting height as a detent, and back.
    private var detent: Binding<PresentationDetent> {
        Binding(
            get: {
                switch stops.stop {
                case .glance: Self.glance
                case .water: Self.water
                case .full: .large
                }
            },
            set: { new in
                stops.rested(at: new == Self.glance ? .glance : new == Self.water ? .water : .full)
            })
    }

    var body: some View {
        let pick = Binding(get: { module.pick }, set: { if $0 == nil { module.close() } })
        let measuring = Binding(
            get: { !panel && module.line != nil },
            set: { if !$0 { module.stopMeasuring() } })
        ZStack(alignment: .bottomTrailing) {
            GeometryReader { _ in
                // The popover is attached before `position`, which would make its anchor the
                // whole layer: it points at the one-point view at the tapped point.
                Color.clear
                    .frame(width: 1, height: 1)
                    .popover(item: pick, attachmentAnchor: .point(.center)) { _ in
                        PointCard(module: module, settingsJSON: controller.settingsJSON)
                            .frame(idealWidth: 400, idealHeight: 520)
                            .presentationDetents([Self.glance, Self.water, .large], selection: detent)
                            .presentationDragIndicator(.visible)
                            // The map stays usable above the sheet up to the middle height.
                            .presentationBackgroundInteraction(.enabled(upThrough: Self.water))
                            .presentationCompactAdaptation(.sheet)
                    }
                    .position(module.pick?.point ?? .zero)
            }
            // The map view under it ignores the safe area too: the same points.
            .ignoresSafeArea()
            .accessibilityHidden(true)
            if panel, module.line != nil {
                MeasureCard(module: module, settingsJSON: controller.settingsJSON)
                    .frame(width: 360)
                    .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 16))
                    .padding(.trailing, 12)
                    .padding(.bottom, Self.appleLegalHeight + 12)
            }
        }
        .sheet(isPresented: measuring) {
            MeasureCard(module: module, settingsJSON: controller.settingsJSON)
                .presentationDetents([.height(330), .large])
                .presentationDragIndicator(.visible)
                // Taps on the map above the sheet move the line's end.
                .presentationBackgroundInteraction(.enabled(upThrough: .height(330)))
        }
        .onChange(of: module.pick?.id) { stops.reset() }
        .onChange(of: module.pick?.water.card != nil) { _, found in
            // The buttons come with the water, not with the loading words.
            stops.water(shown: found)
        }
        .onChange(of: controller.gridRevisions) { module.refresh() }
        .onChange(of: controller.moduleRevision) { module.refresh() }
        .onChange(of: controller.settingsJSON) { module.refresh() }
    }
}
