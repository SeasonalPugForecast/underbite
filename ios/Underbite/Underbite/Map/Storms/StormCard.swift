import SwiftUI
@preconcurrency import UnderbiteKit

/// What a tapped hurricane shows, a sheet on the iPhone and a popover on the iPad: its title,
/// strength (wind, pressure, movement) and advisory time, NHC's advisory link, what NHC's forecast
/// cone is and how NHC makes it, the storm safety text, the credit and "Not for navigation.".
/// Every word is the core's; the safety text is whole and in the plain voice.
struct StormCard: View {
    let hurricane: StormHurricane
    let layer: StormLayer
    let done: () -> Void

    var body: some View {
        NavigationStack {
            List {
                Section {
                    Label {
                        Text(hurricane.title)
                            .font(.headline)
                            .fixedSize(horizontal: false, vertical: true)
                    } icon: {
                        Image(systemName: "hurricane")
                            .foregroundStyle(hurricane.colour.color)
                    }
                    .accessibilityIdentifier("map-storm-card-title")
                    Text(hurricane.strength)
                        .fixedSize(horizontal: false, vertical: true)
                    Text(hurricane.advisory)
                        .foregroundStyle(.readableSecondary)
                    if let url = hurricane.advisoryUrl.flatMap(URL.init(string:)) {
                        Link(layer.advisoryLinkTitle, destination: url)
                            .accessibilityIdentifier("map-storm-card-advisory")
                    }
                }
                if let cone = layer.coneNote {
                    Section {
                        Text(cone)
                            .font(.subheadline)
                            .fixedSize(horizontal: false, vertical: true)
                        if let url = URL(string: layer.coneLink) {
                            Link(layer.coneLinkTitle, destination: url)
                                .font(.subheadline)
                        }
                    }
                }
                Section {
                    Text(layer.safety)
                        .font(.subheadline)
                        .fixedSize(horizontal: false, vertical: true)
                    ForEach(layer.credits, id: \.self) { credit in
                        Text(credit)
                            .font(.caption)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    Text(layer.notice)
                        .font(.caption.weight(.bold))
                }
            }
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done", action: done)
                }
            }
        }
    }
}
