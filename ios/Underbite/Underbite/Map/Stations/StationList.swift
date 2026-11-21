import SwiftUI
import UnderbiteKit

/// The pins on the map as a list, for a screen reader (the pins are drawn by one overlay, so
/// they are not elements of their own): a row in the layer drawer opens the list; choosing a
/// pin opens the same card a tap on it does.
struct StationListSection: View {
    let module: StationsModule
    @State private var listing = false

    var body: some View {
        Section {
            Button {
                listing = true
            } label: {
                HStack {
                    Label {
                        Text("Stations on screen").foregroundStyle(Color.primary)
                    } icon: {
                        Image(systemName: "list.bullet").foregroundStyle(Palette.accent)
                    }
                    Spacer()
                    Image(systemName: "chevron.forward")
                        .font(.footnote.weight(.semibold))
                        .foregroundStyle(.readableSecondary)
                        .accessibilityHidden(true)
                }
                .frame(minHeight: 44)
            }
            .accessibilityIdentifier("map-station-list")
            .sheet(isPresented: $listing) {
                StationList(pins: module.pinsOnScreen()) { pin in
                    listing = false
                    module.open(pin)
                }
            }
        }
    }
}

/// The pins on screen, each with its name and the core's words for it; a row opens its card.
private struct StationList: View {
    let pins: [MapPin]
    let choose: (MapPin) -> Void
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            List(pins) { pin in
                Button {
                    choose(pin)
                } label: {
                    HStack(spacing: 12) {
                        PinSymbol(kind: pin.kind, role: pin.role)
                            .accessibilityHidden(true)
                        VStack(alignment: .leading, spacing: 2) {
                            Text(pin.name)
                                .font(.body.weight(.semibold))
                                .foregroundStyle(Color.primary)
                            if !pin.label.isEmpty {
                                Text(pin.label)
                                    .font(.subheadline)
                                    .foregroundStyle(.readableSecondary)
                            }
                        }
                        .fixedSize(horizontal: false, vertical: true)
                    }
                    .frame(minHeight: 44)
                }
                .accessibilityElement(children: .ignore)
                .accessibilityLabel(StationPins.spoken(pin))
                .accessibilityAddTraits(.isButton)
            }
            .readableText()
            .navigationTitle("Stations on screen")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }
                }
            }
        }
    }
}
