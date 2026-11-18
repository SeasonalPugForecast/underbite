import SwiftUI
@preconcurrency import UnderbiteKit

/// Draws a box rule's area on the map: the app's own map view (``MapCanvas`` with a
/// ``MapController``, the layers the settings show) under a frame, and the area is what the
/// frame holds. Moving and zooming the map moves the area; an area the core would refuse (over
/// 1° a side) turns the frame red, says why in the core's words and cannot be used.
struct AlertBoxPicker: View {
    let initial: AlertDraft.Box?
    /// Where the map opens without an area yet.
    let around: (latitude: Double, longitude: Double)?
    let settingsJSON: String
    let onChoose: (AlertDraft.Box) -> Void

    @Environment(\.dismiss) private var dismiss
    @State private var controller: MapController?

    /// The area in the frame now, once the map has settled.
    private var box: AlertDraft.Box? {
        guard let region = controller?.region else { return nil }
        return AlertBoxFrame.box(
            west: region.west, south: region.south, east: region.east, north: region.north)
    }

    var body: some View {
        NavigationStack {
            content
                .navigationTitle("Area")
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .cancellationAction) {
                        Button("Cancel") { dismiss() }
                    }
                    ToolbarItem(placement: .confirmationAction) {
                        Button("Use this area") {
                            if let box {
                                onChoose(box)
                                dismiss()
                            }
                        }
                        .disabled(box == nil || problem != nil)
                        .accessibilityIdentifier("alert-use-area")
                    }
                }
        }
        .onAppear(perform: makeController)
    }

    private var problem: String? {
        box.flatMap { AlertBoxFrame.problem($0, settingsJSON: settingsJSON) }
    }

    @ViewBuilder
    private var content: some View {
        if let controller {
            let refused = problem
            VStack(spacing: 0) {
                MapCanvas(controller: controller)
                    .overlay {
                        BoxFrame(refused: refused != nil)
                            .allowsHitTesting(false)
                    }
                    .overlay(alignment: .top) {
                        if let notice = controller.plan.notice {
                            NavigationNotice(text: notice)
                                .padding(8)
                        }
                    }
                VStack(alignment: .leading, spacing: 6) {
                    if let refused {
                        Label {
                            Text(refused)
                        } icon: {
                            Image(systemName: "exclamationmark.triangle.fill")
                        }
                        .foregroundStyle(Palette.error)
                    } else {
                        Text("Move and zoom the map until the frame holds the area to watch.")
                    }
                    MapCredits(credits: controller.plan.credits)
                    if controller.plan.notice == nil, let notice = mapNotice {
                        MapNotice(text: notice)
                    }
                }
                .font(.subheadline)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(12)
                .background(.bar)
            }
        } else {
            Color.clear
        }
    }

    /// The core's notice for the map as a whole (safety text).
    private var mapNotice: String? {
        (try? appWords(settingsJson: settingsJSON))?.mapNotice
    }

    private func makeController() {
        guard controller == nil else { return }
        let start: MapStart
        if let initial {
            let s = AlertBoxFrame.start(for: initial)
            start = MapStart(latitude: s.latitude, longitude: s.longitude, span: s.span)
        } else if let around {
            start = MapStart(latitude: around.latitude, longitude: around.longitude, span: 1)
        } else {
            start = .overview
        }
        let made = MapController(start: start)
        made.apply(settingsJSON: settingsJSON)
        controller = made
    }
}

/// The frame over the map: the area inside it is the rule's, the map outside is dimmed.
private struct BoxFrame: View {
    let refused: Bool

    var body: some View {
        GeometryReader { geometry in
            let size = geometry.size
            let rect = CGRect(
                x: size.width * AlertBoxFrame.inset, y: size.height * AlertBoxFrame.inset,
                width: size.width * (1 - 2 * AlertBoxFrame.inset),
                height: size.height * (1 - 2 * AlertBoxFrame.inset))
            ZStack {
                Path { path in
                    path.addRect(CGRect(origin: .zero, size: size))
                    path.addRect(rect)
                }
                .fill(Color.black.opacity(0.25), style: FillStyle(eoFill: true))
                Path { path in path.addRect(rect) }
                    .stroke(refused ? Palette.error : Palette.accent, lineWidth: 3)
            }
        }
        .accessibilityHidden(true)
    }
}
