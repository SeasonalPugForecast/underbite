import SwiftUI

/// The catalogue's species to choose from, searched by common or scientific name (case and
/// accents ignored), returning catalogue ids, never names. A trip chooses any number; a catch
/// one or none, or a name typed in for a species the catalogue lacks (`other`). Ids the
/// catalogue does not have (from a newer build) stay chosen until taken off.
struct SpeciesPicker: View {
    @Environment(\.dismiss) private var dismiss
    /// The catalogue with the user's edits (`SpeciesCatalogue.rows`).
    let rows: [SpeciesListRow]
    @Binding var ids: [String]
    /// More than one may be chosen.
    let allowsMany: Bool
    /// The name typed for a species not in the list; `nil` when that is not offered.
    var other: Binding<String>?

    @State private var search = ""

    var body: some View {
        let typed = search.trimmingCharacters(in: .whitespacesAndNewlines)
        let shown = SpeciesSearch.matching(search, in: rows)
        let selection = SpeciesSelection(ids: ids, allowsMany: allowsMany)
        List {
            if let other {
                typedSections(typed: typed, shown: shown, other: other)
            }
            if allowsMany, typed.isEmpty {
                let unknown = selection.unknown(in: rows)
                if !unknown.isEmpty {
                    Section {
                        ForEach(unknown, id: \.self) { id in
                            row(id, detail: nil, water: nil, chosen: true) { choose(id) }
                        }
                    } footer: {
                        Text("Not in this version's list of species.")
                    }
                }
            }
            Section {
                if !allowsMany, typed.isEmpty {
                    row("None", detail: nil, water: nil, chosen: ids.isEmpty && otherText.isEmpty) {
                        ids = []
                        other?.wrappedValue = ""
                        dismiss()
                    }
                }
                ForEach(shown) { species in
                    row(
                        species.name, detail: species.scientificName, water: species.waterLabel,
                        chosen: selection.contains(species.id)
                    ) { choose(species.id) }
                }
            } footer: {
                if shown.isEmpty, !typed.isEmpty {
                    Text("No species in the list matches “\(typed)”.")
                }
            }
        }
        .searchable(
            text: $search, placement: .navigationBarDrawer(displayMode: .always),
            prompt: Text("Common or scientific name"))
        .navigationTitle("Species")
        .navigationBarTitleDisplayMode(.inline)
    }

    private var otherText: String { other?.wrappedValue ?? "" }

    /// The name typed in, to use for a species not in the list, and the one chosen before.
    @ViewBuilder
    private func typedSections(typed: String, shown: [SpeciesListRow], other: Binding<String>)
        -> some View
    {
        let named = shown.contains {
            $0.name.compare(typed, options: [.caseInsensitive, .diacriticInsensitive]) == .orderedSame
        }
        if !typed.isEmpty, !named {
            Section {
                Button {
                    ids = []
                    other.wrappedValue = typed
                    dismiss()
                } label: {
                    Label("Use “\(typed)”", systemImage: "character.cursor.ibeam")
                }
            } footer: {
                Text("For a species that is not in the list.")
            }
        }
        if ids.isEmpty, !other.wrappedValue.isEmpty, typed.isEmpty {
            Section {
                row(other.wrappedValue, detail: nil, water: nil, chosen: true) { dismiss() }
            }
        }
    }

    private func choose(_ id: String) {
        var selection = SpeciesSelection(ids: ids, allowsMany: allowsMany)
        selection.choose(id)
        ids = selection.ids
        if !allowsMany {
            other?.wrappedValue = ""
            dismiss()
        }
    }

    private func row(
        _ name: String, detail: String?, water: String?, chosen: Bool,
        action: @escaping () -> Void
    ) -> some View {
        Button(action: action) {
            HStack {
                VStack(alignment: .leading, spacing: 2) {
                    Text(name).foregroundStyle(Color.primary)
                    if let detail {
                        Text(detail).font(.caption).italic().foregroundStyle(.readableSecondary)
                    }
                    if let water {
                        Text(water).font(.caption).foregroundStyle(.readableSecondary)
                    }
                }
                Spacer()
                if chosen {
                    Image(systemName: "checkmark")
                        .foregroundStyle(Palette.accent)
                        .accessibilityHidden(true)
                }
            }
            .contentShape(Rectangle())
        }
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(chosen ? .isSelected : [])
    }
}
