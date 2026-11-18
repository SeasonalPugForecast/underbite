import SwiftUI

/// The three seasons of a species, kept apart, as sections of the profile's list: now (from
/// the open spot's water temperature), the typical months, and the legal season, which is the
/// official pages of a region (opened in Safari) with the regulations disclaimer, never dates
/// or limits. Every word is the core's.
struct SpeciesSeasonsSections: View {
    /// The seasons as built (`SeasonsLoad`), or why they could not be.
    let loaded: Result<SpeciesSeasons, SeasonsLoad.Problem>?
    /// The name of the spot whose water "now" reads.
    let spot: String?
    /// The region of the official pages (ISO 3166-2, `US-NY`).
    @Binding var region: String

    var body: some View {
        switch loaded {
        case .success(let seasons):
            nowSection(seasons.now, spot: spot)
            typicalSection(seasons.typical)
            legalSection(seasons.legal)
        case .failure(let problem):
            Section {
                Label(problem.message, systemImage: "exclamationmark.triangle")
            }
        case nil:
            Section {
                ProgressView().frame(maxWidth: .infinity)
            }
        }
    }

    // MARK: - Now

    private func nowSection(_ now: SpeciesSeasons.Now, spot: String?) -> some View {
        Section {
            VStack(alignment: .leading, spacing: 4) {
                Text(now.stageLabel).font(.headline)
                Text(now.text).fixedSize(horizontal: false, vertical: true)
            }
            .accessibilityElement(children: .combine)
            if let caution = now.caution {
                // Safety text: shown whole, in plain words.
                Label {
                    Text(caution).fixedSize(horizontal: false, vertical: true)
                } icon: {
                    Image(systemName: "exclamationmark.triangle.fill")
                        .foregroundStyle(Palette.warning)
                        .accessibilityHidden(true)
                }
                .font(.subheadline)
            }
            ForEach(now.rows, id: \.self) { row in
                LabeledContent {
                    VStack(alignment: .trailing, spacing: 2) {
                        Text(row.value)
                        if let detail = row.detail {
                            Text(detail).font(.caption).foregroundStyle(.readableSecondary)
                        }
                    }
                    .multilineTextAlignment(.trailing)
                } label: {
                    Text(row.label)
                }
            }
            // Each source and licence once; the profile lists every record it cites.
            ForEach(Self.sources(of: now.credits), id: \.self) { credit in
                HStack(alignment: .firstTextBaseline, spacing: 6) {
                    Text(credit.source)
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(.readableSecondary)
                    if let licence = credit.licence { LicenceBadge(text: licence) }
                }
            }
        } header: {
            Text(now.title)
        } footer: {
            if let spot { Text(spot) }
        }
    }

    /// The credits with one per source and licence, in their order.
    private static func sources(of credits: [SpeciesCredit]) -> [SpeciesCredit] {
        var seen = Set<String>()
        return credits.filter { seen.insert($0.source + "\u{0}" + ($0.licence ?? "")).inserted }
    }

    // MARK: - Typical

    private func typicalSection(_ typical: SpeciesSeasons.Typical) -> some View {
        Section {
            ScrollView(.horizontal) {
                HStack(alignment: .top, spacing: 8) {
                    ForEach(typical.months, id: \.month) { month in
                        MonthCell(month: month)
                    }
                }
                .padding(.vertical, 4)
            }
            .scrollIndicators(.hidden)
            if let basis = typical.months.first?.basis,
                typical.months.allSatisfy({ $0.basis == basis })
            {
                Text(basis).font(.caption).foregroundStyle(.readableSecondary)
            }
        } header: {
            Text(typical.title)
        } footer: {
            if let note = typical.note { Text(note) }
        }
    }

    // MARK: - Legal

    private func legalSection(_ legal: SpeciesSeasons.Legal) -> some View {
        Section {
            LabeledContent {
                TextField("Region", text: $region, prompt: Text("US-NY"))
                    .textInputAutocapitalization(.characters)
                    .autocorrectionDisabled()
                    .multilineTextAlignment(.trailing)
            } label: {
                Text("Region")
            }
            ForEach(legal.links, id: \.self) { link in
                if let url = URL(string: link.link) {
                    Link(destination: url) {
                        HStack {
                            VStack(alignment: .leading, spacing: 2) {
                                Text(link.title)
                                Text(link.detail)
                                    .font(.caption)
                                    .foregroundStyle(.readableSecondary)
                            }
                            Spacer()
                            Image(systemName: "safari")
                                .foregroundStyle(Palette.accent)
                                .accessibilityHidden(true)
                        }
                        .contentShape(Rectangle())
                    }
                    .foregroundStyle(.primary)
                }
            }
            if let empty = legal.empty {
                Text(empty).foregroundStyle(.readableSecondary)
            }
            if let notes = legal.notes {
                LabeledContent(legal.notesTitle) { Text(notes) }
            }
            // The regulations disclaimer: always shown, whole, in plain words.
            Label {
                Text(legal.disclaimer).fixedSize(horizontal: false, vertical: true)
            } icon: {
                Image(systemName: "exclamationmark.shield")
                    .foregroundStyle(Palette.warning)
                    .accessibilityHidden(true)
            }
            .font(.footnote)
        } header: {
            Text(legal.title)
        }
    }
}

/// What the seasons of a species are built from, and building them off the main actor.
nonisolated struct SeasonsLoad: Equatable, Sendable {
    var id: String
    var edits: String
    var settings: String
    var region: String
    /// The spot whose water "now" reads, and when it was last fetched.
    var spot: String?
    var fetched: Int64?

    /// Why the seasons could not be built, in the core's words.
    struct Problem: Error, Equatable {
        var message: String
    }

    /// The seasons with the spot's last snapshot.
    func build(snapshotJSON: String?, now: Int64) -> Result<SpeciesSeasons, Problem> {
        do {
            return .success(
                try SpeciesCatalogue.seasons(
                    id: id, edits: edits, snapshotJSON: snapshotJSON, region: region,
                    settingsJSON: settings, now: now))
        } catch {
            return .failure(Problem(message: SpeciesCatalogue.message(of: error)))
        }
    }
}

/// One month of the typical strip: the month, the stage, and the water and observations when
/// known.
private struct MonthCell: View {
    let month: SpeciesSeasons.Month

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(month.label).font(.subheadline.weight(.semibold))
            Text(month.stageLabel)
                .font(.caption)
                .fixedSize(horizontal: false, vertical: true)
            if let water = month.water {
                Text(water).font(.caption).foregroundStyle(.readableSecondary)
            }
            if let observations = month.observations {
                Text(observations).font(.caption2).foregroundStyle(.readableSecondary)
            }
        }
        .frame(width: 84, alignment: .topLeading)
        .padding(8)
        .background(Color(.tertiarySystemGroupedBackground), in: .rect(cornerRadius: 10))
        .accessibilityElement(children: .combine)
    }
}
