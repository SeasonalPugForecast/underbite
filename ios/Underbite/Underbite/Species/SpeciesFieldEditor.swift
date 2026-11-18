import SwiftUI
@preconcurrency import UnderbiteKit

/// Edits one field of a species profile: names and lures one per line, a choice of the core's
/// words, or temperatures and depths typed in the user's units. The core reads what is typed
/// (`species_edit`) and says what is wrong; nothing is kept until Save. An edited field can be
/// put back to its source's value.
struct SpeciesFieldEditor: View {
    @Environment(SpotStore.self) private var spots
    @Environment(\.dismiss) private var dismiss
    let speciesID: String
    let row: SpeciesFieldRow
    let field: String

    @State private var texts: [Entry]
    @State private var values: [String]
    @State private var picked: [String]
    @State private var problem: String?

    private var store: SpeciesEditsStore { .shared }

    /// A line of a list field.
    private struct Entry: Identifiable, Equatable {
        let id = UUID()
        var text: String
    }

    init(speciesID: String, row: SpeciesFieldRow, field: String) {
        self.speciesID = speciesID
        self.row = row
        self.field = field
        let start = row.edit
        _texts = State(initialValue: (start.isEmpty ? [""] : start).map { Entry(text: $0) })
        let count = SpeciesEditKind(field: field) == .depth ? 8 : 2
        _values = State(
            initialValue: start.count >= count
                ? Array(start.prefix(count)) : start + Array(repeating: "", count: count - start.count))
        _picked = State(initialValue: start)
    }

    private var kind: SpeciesEditKind { SpeciesEditKind(field: field) }

    var body: some View {
        let settings = spots.settingsJSON
        NavigationStack {
            Form {
                switch kind {
                case .texts: textsSection
                case .one: oneSection(SpeciesCatalogue.choices(field: field, settingsJSON: settings))
                case .many: manySection(SpeciesCatalogue.choices(field: field, settingsJSON: settings))
                case .temperature:
                    rangeSection(
                        header: nil, low: 0,
                        unit: SpeciesCatalogue.unitLabel(field: field, settingsJSON: settings))
                case .depth:
                    let unit = SpeciesCatalogue.unitLabel(field: field, settingsJSON: settings)
                    ForEach(Array(Self.seasons.enumerated()), id: \.offset) { index, season in
                        rangeSection(header: season, low: 2 * index, unit: unit)
                    }
                }
                if let problem {
                    Section {
                        Label {
                            Text(problem).fixedSize(horizontal: false, vertical: true)
                        } icon: {
                            Image(systemName: "exclamationmark.triangle.fill")
                                .foregroundStyle(Palette.warning)
                                .accessibilityHidden(true)
                        }
                    }
                }
                Section {
                    if let source = row.source {
                        SpeciesSourceLine(source: source)
                    }
                    if row.edited {
                        Button("Use the source's value", role: .destructive, action: reset)
                            .disabled(!store.canSave)
                    }
                } header: {
                    Text("Now")
                } footer: {
                    Text(row.value)
                }
            }
            .readableText()
            .navigationTitle(row.label)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save", action: save).disabled(!store.canSave)
                }
            }
        }
    }

    /// The four seasons of the depth field, in the order the core reads them.
    private static let seasons = ["Spring", "Summer", "Autumn", "Winter"]

    // MARK: - Sections

    private var textsSection: some View {
        Section {
            ForEach($texts) { $entry in
                TextField(row.label, text: $entry.text)
            }
            .onDelete { texts.remove(atOffsets: $0) }
            Button("Add a line", systemImage: "plus") { texts.append(Entry(text: "")) }
        } footer: {
            Text("One per line; swipe a line to delete it.")
        }
    }

    private func oneSection(_ choices: [UnderbiteKit.SpeciesChoice]) -> some View {
        Section {
            ForEach(choices, id: \.id) { choice in
                choiceRow(choice, chosen: picked.first == choice.id) { picked = [choice.id] }
            }
        }
    }

    private func manySection(_ choices: [UnderbiteKit.SpeciesChoice]) -> some View {
        Section {
            ForEach(choices, id: \.id) { choice in
                choiceRow(choice, chosen: picked.contains(choice.id)) {
                    if picked.contains(choice.id) {
                        picked.removeAll { $0 == choice.id }
                    } else {
                        // In the core's order.
                        let ids = Set(picked + [choice.id])
                        picked = choices.map(\.id).filter(ids.contains)
                    }
                }
            }
        }
    }

    private func choiceRow(
        _ choice: UnderbiteKit.SpeciesChoice, chosen: Bool, action: @escaping () -> Void
    ) -> some View {
        Button(action: action) {
            HStack {
                Text(choice.label).foregroundStyle(Color.primary)
                Spacer()
                if chosen {
                    Image(systemName: "checkmark")
                        .foregroundStyle(Palette.accent)
                        .accessibilityHidden(true)
                }
            }
            .contentShape(Rectangle())
        }
        .accessibilityAddTraits(chosen ? .isSelected : [])
    }

    private func rangeSection(header: String?, low: Int, unit: String) -> some View {
        Section {
            numberField("From", index: low, unit: unit)
            numberField("To", index: low + 1, unit: unit)
        } header: {
            if let header { Text(header) }
        }
    }

    private func numberField(_ title: String, index: Int, unit: String) -> some View {
        LabeledContent {
            HStack(spacing: 6) {
                TextField(title, text: $values[index])
                    .keyboardType(.numbersAndPunctuation)
                    .multilineTextAlignment(.trailing)
                if !unit.isEmpty {
                    Text(unit).foregroundStyle(.readableSecondary)
                }
            }
        } label: {
            Text(title)
        }
    }

    // MARK: - Actions

    private var entered: [String] {
        switch kind {
        case .texts: texts.map(\.text)
        case .one, .many: picked
        case .temperature, .depth: values
        }
    }

    private func save() {
        do {
            try store.edit(
                id: speciesID, field: field, values: entered, settingsJSON: spots.settingsJSON)
            dismiss()
        } catch {
            problem = SpeciesCatalogue.message(of: error)
        }
    }

    private func reset() {
        do {
            try store.reset(id: speciesID, field: field)
            dismiss()
        } catch {
            problem = SpeciesCatalogue.message(of: error)
        }
    }
}
