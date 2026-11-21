import SwiftUI
import UnderbiteKit

/// What a tapped beach forecast zone shows: the zone's name, the rip current risk with its
/// colour and word, the fields the forecast office issued, who issued it and how old it is, the
/// rip current safety line and the credit. Every word is the core's; the card only lays it out.
/// The safety line is safety text, in full.
struct BeachCard: View {
    let zone: BeachZonePin
    let credits: [NearbyCredit]
    let done: () -> Void

    var body: some View {
        NavigationStack {
            List {
                Section {
                    HStack(alignment: .top, spacing: 12) {
                        BeachSwatch(role: BeachRole(core: zone.card.role))
                        VStack(alignment: .leading, spacing: 4) {
                            Text(zone.card.title)
                                .font(.headline)
                            Text(zone.card.risk)
                                .font(.title3.weight(.semibold))
                            if let period = zone.card.period {
                                Text(period)
                                    .font(.subheadline)
                                    .foregroundStyle(.readableSecondary)
                            }
                        }
                        .fixedSize(horizontal: false, vertical: true)
                    }
                    .accessibilityElement(children: .ignore)
                    .accessibilityLabel(zone.card.accessibility)
                    .accessibilityIdentifier("beach-card")
                }
                Section {
                    ForEach(Array(zone.card.rows.enumerated()), id: \.offset) { _, row in
                        LabeledContent {
                            Text(row.value)
                                .multilineTextAlignment(.trailing)
                                .fixedSize(horizontal: false, vertical: true)
                        } label: {
                            Text(row.label)
                        }
                        .accessibilityElement(children: .combine)
                    }
                }
                Section {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(zone.card.issued)
                            .fixedSize(horizontal: false, vertical: true)
                        if let age = zone.card.age {
                            Text(age)
                                .foregroundStyle(.readableSecondary)
                        }
                    }
                    .font(.subheadline)
                    .accessibilityElement(children: .combine)
                }
                Section {
                    // Safety text: in full, never shortened.
                    Text(zone.card.safety)
                        .font(.callout)
                        .fixedSize(horizontal: false, vertical: true)
                        .accessibilityIdentifier("beach-card-safety")
                }
                Section {
                    ForEach(credits, id: \.source) { credit in
                        NearbyCreditRow(credit: credit)
                    }
                }
            }
            .readableText()
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done", action: done)
                }
            }
        }
    }
}

/// A zone's colour as a small swatch: filled in the category's colour, or outlined and dashed
/// for a zone with no forecast issued, as on the map.
struct BeachSwatch: View {
    let role: BeachRole

    var body: some View {
        let colour = Color(uiColor: BeachRenderer.colour(role.rgb))
        RoundedRectangle(cornerRadius: 4)
            .fill(role.filled ? colour.opacity(0.55) : Color.clear)
            .overlay {
                RoundedRectangle(cornerRadius: 4)
                    .strokeBorder(
                        colour, style: StrokeStyle(lineWidth: 2, dash: role.filled ? [] : [4, 3]))
            }
            .frame(width: 22, height: 22)
            .accessibilityHidden(true)
    }
}
