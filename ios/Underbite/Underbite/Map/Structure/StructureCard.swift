import SwiftUI
import UnderbiteKit

/// What a tapped structure mark shows, a sheet on the iPhone and a popover on the iPad. A
/// charted point: its kind, the chart's category as charted, its name, its survey date, why no
/// depth is shown, the credit and "Not for navigation.". A reef site: its name, its layer,
/// its water, the depths as the state lists them with what they are measured from, the state's
/// comments, the credit and the notice. A group lists its marks first. Every word is the
/// core's; the safety text is whole and in the plain voice.
struct StructureCard: View {
    let cluster: StructureCluster
    /// The catalogue's layers by id, for their title, credit, notice and depth reference.
    let layers: [String: LayerInfo]
    let done: () -> Void

    var body: some View {
        NavigationStack {
            Group {
                if let mark = cluster.single {
                    StructureDetail(mark: mark, layer: layers[mark.layer])
                } else {
                    StructureList(marks: cluster.marks)
                }
            }
            .navigationDestination(for: StructureMark.self) {
                StructureDetail(mark: $0, layer: layers[$0.layer])
            }
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done", action: done)
                }
            }
        }
    }
}

/// The marks of a group, each opening its card.
private struct StructureList: View {
    let marks: [StructureMark]

    var body: some View {
        List(marks) { mark in
            NavigationLink(value: mark) {
                HStack(spacing: 12) {
                    StructureSymbol(kind: mark.kind)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(mark.title)
                            .font(.body.weight(.semibold))
                        if let subtitle = mark.subtitle {
                            Text(subtitle)
                                .font(.subheadline)
                                .foregroundStyle(.readableSecondary)
                        }
                    }
                    .fixedSize(horizontal: false, vertical: true)
                }
                .accessibilityElement(children: .ignore)
                .accessibilityLabel(mark.accessibility)
            }
        }
        .readableText()
        .navigationBarTitleDisplayMode(.inline)
    }
}

/// One charted point or reef site.
private struct StructureDetail: View {
    let mark: StructureMark
    let layer: LayerInfo?

    var body: some View {
        List {
            Section {
                HStack(alignment: .top, spacing: 12) {
                    StructureSymbol(kind: mark.kind)
                    VStack(alignment: .leading, spacing: 4) {
                        Text(mark.title)
                            .font(.headline)
                        if let layer {
                            Text(layer.title)
                                .font(.footnote)
                                .foregroundStyle(.readableSecondary)
                        }
                    }
                    .fixedSize(horizontal: false, vertical: true)
                }
                .accessibilityElement(children: .combine)
                .accessibilityIdentifier("structure-card")
                switch mark.item {
                case .point(let point):
                    PointFacts(point: point)
                case .reef(let reef):
                    ReefFacts(reef: reef, layer: layer)
                }
            }
            if let layer {
                Section {
                    if let caption = layer.legendCaption {
                        Text(caption)
                            .font(.caption)
                            .foregroundStyle(.readableSecondary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    Text(layer.credit)
                        .font(.caption)
                        .fixedSize(horizontal: false, vertical: true)
                    // The credit may carry the notice already; it is never shown twice.
                    if let notice = layer.notice, !layer.credit.contains(notice) {
                        Text(notice)
                            .font(.caption.weight(.bold))
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
            }
        }
        .readableText()
        .navigationBarTitleDisplayMode(.inline)
    }
}

/// A charted point's words: its category as charted, its name, its survey date, and why its
/// depth is not shown.
private struct PointFacts: View {
    let point: StructurePoint

    var body: some View {
        if let category = point.category {
            Text(category)
                .fixedSize(horizontal: false, vertical: true)
        }
        if let name = point.name {
            Text(name)
                .fixedSize(horizontal: false, vertical: true)
        }
        if let surveyed = point.surveyed {
            Text(surveyed)
                .foregroundStyle(.readableSecondary)
        }
        Text(point.depthNote)
            .font(.footnote.weight(.semibold))
            .fixedSize(horizontal: false, vertical: true)
            .accessibilityIdentifier("structure-depth-note")
    }
}

/// A reef site's words: its water, its depths as the state lists them with what they are
/// measured from, and the state's comments.
private struct ReefFacts: View {
    let reef: ReefInfo
    let layer: LayerInfo?

    var body: some View {
        if let water = reef.bodyOfWater {
            Text(water)
                .fixedSize(horizontal: false, vertical: true)
        }
        if let depth = reef.depthText {
            VStack(alignment: .leading, spacing: 4) {
                Text(depth)
                    .font(.body.monospacedDigit())
                if let reference = layer?.depthReference {
                    Text(reference)
                        .font(.footnote)
                        .foregroundStyle(.readableSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            .accessibilityElement(children: .combine)
        }
        if let comments = reef.comments {
            Text(comments)
                .font(.footnote)
                .fixedSize(horizontal: false, vertical: true)
        }
    }
}

/// A mark as the map draws it.
struct StructureSymbol: View {
    let kind: StructureMarkKind

    var body: some View {
        Canvas { context, size in
            context.withCGContext { cg in
                UIGraphicsPushContext(cg)
                defer { UIGraphicsPopContext() }
                cg.translateBy(x: size.width / 2, y: size.height / 2)
                StructureDrawing.mark(kind, in: cg)
            }
        }
        .frame(width: 38, height: 38)
        .accessibilityHidden(true)
    }
}
