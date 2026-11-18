import SwiftUI

/// One field of a profile: its label, its value in the user's units, and where the value came
/// from with its licence, every word the core's. An edited value is marked.
struct SpeciesFieldLine: View {
    let row: SpeciesFieldRow

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(alignment: .firstTextBaseline) {
                Text(row.label)
                    .font(.subheadline)
                    .foregroundStyle(.readableSecondary)
                Spacer(minLength: 8)
                if row.edited {
                    Image(systemName: "pencil")
                        .font(.caption)
                        .foregroundStyle(Palette.accent)
                        .accessibilityHidden(true)
                }
            }
            Text(row.value)
                .foregroundStyle(row.source == nil ? AnyShapeStyle(.readableSecondary) : AnyShapeStyle(.primary))
                .fixedSize(horizontal: false, vertical: true)
            if let source = row.source {
                SpeciesSourceLine(source: source)
            }
        }
        .padding(.vertical, 2)
        .accessibilityElement(children: .combine)
    }
}

/// Where a value came from: the source's name and its licence, then the record it cites.
struct SpeciesSourceLine: View {
    let source: SpeciesSource

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack(alignment: .firstTextBaseline, spacing: 6) {
                Text(source.name)
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.readableSecondary)
                LicenceBadge(text: source.licenceLabel, nonCommercial: source.licence == "cc_by_nc4")
            }
            if let reference = source.reference, !reference.isEmpty {
                Text(reference)
                    .font(.caption2)
                    .foregroundStyle(.readableSecondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }
}

/// A licence ("CC BY-NC 4.0", "Public domain") in a small capsule.
struct LicenceBadge: View {
    let text: String
    var nonCommercial = false

    var body: some View {
        Text(text)
            .font(.caption2.weight(.semibold))
            .foregroundStyle(nonCommercial ? Palette.accent : Palette.secondaryText)
            .padding(.horizontal, 6)
            .padding(.vertical, 2)
            .overlay(
                Capsule().strokeBorder(
                    nonCommercial ? Palette.accent : Palette.secondaryText, lineWidth: 1))
            .fixedSize()
    }
}

/// Every source of a page once, with its citation and licence; a source with a page opens it
/// in Safari.
struct SpeciesCreditsSection: View {
    let credits: [SpeciesCredit]
    let notices: [String]

    var body: some View {
        if !notices.isEmpty {
            Section {
                ForEach(notices, id: \.self) { notice in
                    Label {
                        Text(notice).fixedSize(horizontal: false, vertical: true)
                    } icon: {
                        Image(systemName: "info.circle").foregroundStyle(Palette.accent)
                    }
                    .font(.footnote)
                }
            }
        }
        if !credits.isEmpty {
            Section("Sources") {
                ForEach(credits, id: \.self) { credit in
                    VStack(alignment: .leading, spacing: 4) {
                        HStack(alignment: .firstTextBaseline, spacing: 6) {
                            if let link = credit.link.flatMap(URL.init(string:)) {
                                Link(destination: link) {
                                    Label(credit.source, systemImage: "arrow.up.right.square")
                                        .labelStyle(TrailingIcon())
                                }
                                .font(.subheadline.weight(.semibold))
                            } else {
                                Text(credit.source).font(.subheadline.weight(.semibold))
                            }
                            if let licence = credit.licence {
                                LicenceBadge(text: licence)
                            }
                        }
                        if credit.text != credit.source {
                            Text(credit.text)
                                .font(.caption)
                                .foregroundStyle(.readableSecondary)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                    }
                    .padding(.vertical, 2)
                }
            }
        }
    }
}

/// A label with its icon after the title.
private struct TrailingIcon: LabelStyle {
    func makeBody(configuration: Configuration) -> some View {
        HStack(spacing: 4) {
            configuration.title
            configuration.icon.font(.caption).accessibilityHidden(true)
        }
    }
}
