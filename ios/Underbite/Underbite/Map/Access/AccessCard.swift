import SwiftUI
import UnderbiteKit

/// What a tapped access or stocking mark shows: the place's name and the core's words for it
/// (its kind and the access the source records, or a stocked water's stocking lists as the
/// stocking card shows them), the notes on what access means and its source's credit. A group
/// lists its places first.
struct AccessCard: View {
    let cluster: AccessCluster
    let notes: [String]
    let credits: [NearbyCredit]
    /// The catalogue's layers by id, for their credit.
    let layers: [String: LayerInfo]
    let done: () -> Void

    var body: some View {
        NavigationStack {
            Group {
                if let mark = cluster.single {
                    place(mark)
                } else {
                    AccessList(marks: cluster.marks)
                }
            }
            .navigationDestination(for: AccessMark.self) { place($0) }
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done", action: done)
                }
            }
        }
    }

    private func place(_ mark: AccessMark) -> AccessPlace {
        AccessPlace(
            mark: mark, notes: notes, credits: credits.filter { $0.source == mark.source },
            layer: layers[mark.layer])
    }
}

/// The places of a group, each opening its card.
private struct AccessList: View {
    let marks: [AccessMark]

    var body: some View {
        List(marks) { mark in
            NavigationLink(value: mark) {
                HStack(spacing: 12) {
                    AccessSymbol(kind: mark.kind, role: mark.role)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(mark.title)
                            .font(.body.weight(.semibold))
                        Text(mark.subtitle)
                            .font(.subheadline)
                            .foregroundStyle(.readableSecondary)
                    }
                    .fixedSize(horizontal: false, vertical: true)
                }
                .accessibilityElement(children: .ignore)
                .accessibilityLabel(mark.accessibility)
            }
        }
        .readableText()
        .navigationTitle("Here")
        .navigationBarTitleDisplayMode(.inline)
    }
}

/// One place.
private struct AccessPlace: View {
    let mark: AccessMark
    let notes: [String]
    /// The credits of its source.
    let credits: [NearbyCredit]
    let layer: LayerInfo?

    /// A stocked water's stocking card, once its lists answered.
    private var card: ViewModel.StockingCard? {
        guard case .stocking(let pin) = mark.item, let json = pin.cardJson else { return nil }
        return try? ViewModel.decoder().decode(ViewModel.StockingCard.self, from: Data(json.utf8))
    }

    var body: some View {
        List {
            Section {
                HStack(alignment: .top, spacing: 12) {
                    AccessSymbol(kind: mark.kind, role: mark.role)
                    VStack(alignment: .leading, spacing: 4) {
                        Text(mark.title)
                            .font(.headline)
                        Text(mark.subtitle)
                            .font(.subheadline)
                            .foregroundStyle(.readableSecondary)
                        if case .access(let pin) = mark.item {
                            Text(pin.accessLabel)
                                .font(.subheadline)
                            Text(pin.sourceName)
                                .font(.footnote)
                                .foregroundStyle(.readableSecondary)
                        }
                    }
                    .fixedSize(horizontal: false, vertical: true)
                }
                .accessibilityElement(children: .ignore)
                .accessibilityLabel(mark.accessibility)
                .accessibilityIdentifier("access-card")
            }
            if let card {
                StockingDetail(full: card.full)
            }
            if case .access = mark.item, !notes.isEmpty {
                Section {
                    ForEach(notes, id: \.self) { note in
                        Text(note)
                            .font(.footnote)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
            }
            Section {
                if credits.isEmpty, let layer {
                    Text(layer.credit)
                        .font(.caption)
                        .fixedSize(horizontal: false, vertical: true)
                }
                ForEach(credits, id: \.source) { credit in
                    NearbyCreditRow(credit: credit)
                }
                if let footnote = card?.chrome.footnote {
                    Text(footnote)
                        .font(.caption)
                        .foregroundStyle(.readableSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
        }
        .readableText()
        .navigationBarTitleDisplayMode(.inline)
    }
}

/// A mark as the map draws it.
struct AccessSymbol: View {
    let kind: AccessMarkKind
    let role: AccessMarkRole

    var body: some View {
        Canvas { context, size in
            context.withCGContext { cg in
                UIGraphicsPushContext(cg)
                defer { UIGraphicsPopContext() }
                cg.translateBy(x: size.width / 2, y: size.height / 2)
                AccessDrawing.mark(kind, role, in: cg)
            }
        }
        .frame(width: 38, height: 38)
        .accessibilityHidden(true)
    }
}
