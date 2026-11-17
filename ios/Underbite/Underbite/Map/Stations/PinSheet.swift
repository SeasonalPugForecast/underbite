import SwiftUI
import UnderbiteKit

/// What a tapped pin shows: a station's name and the core's words for it (its kind and latest
/// value, when the map has one), its layer's credit and notice, and "Add as spot", or "Open
/// spot" for a saved spot or a station a spot is already at. A group of pins lists them first.
struct PinSheet: View {
    let cluster: PinCluster
    /// The catalogue's layers by id, for their credit and notice.
    let layers: [String: LayerInfo]
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            Group {
                if let pin = cluster.single {
                    PinCard(pin: pin, layer: layers[pin.layer])
                } else {
                    PinList(pins: cluster.pins)
                }
            }
            .navigationDestination(for: MapPin.self) { pin in
                PinCard(pin: pin, layer: layers[pin.layer])
            }
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }
                }
            }
        }
    }
}

/// The pins of a group, each opening its card.
private struct PinList: View {
    let pins: [MapPin]

    var body: some View {
        List(pins) { pin in
            NavigationLink(value: pin) {
                HStack(spacing: 12) {
                    PinSymbol(kind: pin.kind, role: pin.role)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(pin.name)
                            .font(.body.weight(.semibold))
                        if !pin.label.isEmpty {
                            Text(pin.label)
                                .font(.subheadline)
                                .foregroundStyle(.readableSecondary)
                        }
                    }
                    .fixedSize(horizontal: false, vertical: true)
                }
                .accessibilityElement(children: .combine)
            }
        }
        .readableText()
        .navigationTitle("Here")
        .navigationBarTitleDisplayMode(.inline)
    }
}

/// One station or spot.
private struct PinCard: View {
    let pin: MapPin
    let layer: LayerInfo?
    @Environment(SpotStore.self) private var store
    @State private var added: String?

    /// The saved spot this pin stands for or is at.
    private var spotID: String? {
        if let id = pin.spotID { return store.spot(id) == nil ? nil : id }
        if let added, store.spot(added) != nil { return added }
        return StationPins.spot(
            at: pin, among: store.spots.map { ($0.id, $0.latitude, $0.longitude) })
    }

    var body: some View {
        List {
            Section {
                HStack(alignment: .top, spacing: 12) {
                    PinSymbol(kind: pin.kind, role: pin.role)
                    VStack(alignment: .leading, spacing: 4) {
                        Text(pin.name)
                            .font(.headline)
                        if let words = words, !words.isEmpty {
                            Text(words)
                                .font(.subheadline)
                                .foregroundStyle(.readableSecondary)
                        }
                    }
                    .fixedSize(horizontal: false, vertical: true)
                }
                .accessibilityElement(children: .combine)
                .accessibilityIdentifier("pin-card")
            } footer: {
                if let layer {
                    VStack(alignment: .leading, spacing: 4) {
                        if let notice = layer.notice {
                            Text(notice)
                                .font(.footnote.weight(.bold))
                                .foregroundStyle(Color.primary)
                        }
                        Text(layer.credit)
                    }
                    .fixedSize(horizontal: false, vertical: true)
                }
            }
            Section {
                if let spotID {
                    NavigationLink {
                        ConditionsView(spotID: spotID)
                    } label: {
                        Label("Open spot", systemImage: "water.waves")
                    }
                    .accessibilityIdentifier("pin-open-spot")
                } else {
                    Button("Add as spot", systemImage: "plus.circle", action: add)
                        .accessibilityIdentifier("pin-add-spot")
                }
            }
        }
        .readableText()
        .navigationBarTitleDisplayMode(.inline)
    }

    /// The core's words for the pin; for a saved spot, its kind of water as the list shows it.
    private var words: String? {
        if let id = pin.spotID { return store.listing(for: id)?.waterTypeLabel }
        return pin.label
    }

    /// Saves a spot at the station, named as its source names it. Its kind of water is not
    /// chosen, so the first fetch suggests one.
    private func add() {
        let spot = Spot(
            name: pin.name, latitude: pin.latitude, longitude: pin.longitude,
            waterType: Spot.unknownWaterType, waterTypeChosen: false)
        store.add(spot)
        added = spot.id
        Task { await store.refresh(only: [spot.id]) }
    }
}

/// A pin's mark as the map draws it, for the sheet.
struct PinSymbol: View {
    let kind: PinKind
    let role: PinRole

    var body: some View {
        Canvas { context, size in
            context.withCGContext { cg in
                UIGraphicsPushContext(cg)
                defer { UIGraphicsPopContext() }
                cg.translateBy(x: size.width / 2, y: size.height / 2)
                PinDrawing.pin(kind, role, in: cg)
            }
        }
        .frame(width: 36, height: 36)
        .accessibilityHidden(true)
    }
}
