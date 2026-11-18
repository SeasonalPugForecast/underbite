import SwiftUI
@preconcurrency import UnderbiteKit

/// A species profile someone sent: what importing it does to the user's edits (the fields
/// added, the ones already the same, and the ones both edited, where the user's edit stays
/// unless they take the sender's), the import buttons, and the sender's profile with every
/// source and licence. Base values never change: only edits are imported.
struct ReceivedProfileView: View {
    @Environment(SpotStore.self) private var spots
    let profile: ReceivedProfile

    /// Set once the edits are imported.
    @Binding var imported: Bool
    @State private var problem: String?

    private var store: SpeciesEditsStore { .shared }
    private var plan: ProfileImportPlan { profile.shared.plan }

    var body: some View {
        let words = ShareText.words(settingsJSON: spots.settingsJSON)
        VStack(alignment: .leading, spacing: 16) {
            planCard
            if imported, let words {
                Label(words.added, systemImage: "checkmark.circle.fill")
                    .font(.headline)
                    .foregroundStyle(Palette.accent)
            } else {
                actions(words)
            }
            if let problem {
                Label(problem, systemImage: "exclamationmark.triangle")
                    .font(.callout)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if let page = profile.page {
                pageCard(page)
            }
        }
    }

    // MARK: - The plan

    private var planCard: some View {
        VStack(alignment: .leading, spacing: 10) {
            if let page = profile.page {
                VStack(alignment: .leading, spacing: 2) {
                    Text(page.name).font(.title3.bold())
                    Text(page.scientificName).italic().foregroundStyle(.readableSecondary)
                }
                .accessibilityElement(children: .combine)
            }
            if !plan.known {
                Text("This species is not in this version of the app, so its edits cannot be added.")
                    .fixedSize(horizontal: false, vertical: true)
            } else {
                fields("Their edits to add", plan.added, systemImage: "plus.circle")
                fields(
                    "Edited by you too: yours stay unless you take theirs", plan.conflicts,
                    systemImage: "arrow.triangle.branch")
                fields("Already the same as yours", plan.same, systemImage: "equal.circle")
                if plan.added.isEmpty, plan.conflicts.isEmpty {
                    Text("Nothing to add: you have every edit in this profile.")
                        .foregroundStyle(.readableSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color(.secondarySystemGroupedBackground), in: .rect(cornerRadius: 14))
    }

    @ViewBuilder
    private func fields(_ title: String, _ list: [ProfileField], systemImage: String) -> some View {
        if !list.isEmpty {
            VStack(alignment: .leading, spacing: 4) {
                Label(title, systemImage: systemImage)
                    .font(.subheadline.weight(.semibold))
                    .foregroundStyle(.readableSecondary)
                ForEach(list, id: \.field) { field in
                    Text(field.label).padding(.leading, 28)
                }
            }
            .accessibilityElement(children: .combine)
        }
    }

    // MARK: - Importing

    @ViewBuilder
    private func actions(_ words: ShareWords?) -> some View {
        if let words, plan.known, !(plan.added.isEmpty && plan.conflicts.isEmpty) {
            VStack(spacing: 10) {
                Button {
                    importEdits(takeTheirs: false)
                } label: {
                    Text(words.add).frame(maxWidth: .infinity)
                }
                .buttonStyle(.borderedProminent)
                .controlSize(.large)
                if !plan.conflicts.isEmpty {
                    Button {
                        importEdits(takeTheirs: true)
                    } label: {
                        Text("Add, taking their edits").frame(maxWidth: .infinity)
                    }
                    .buttonStyle(.bordered)
                    .controlSize(.large)
                }
            }
            .disabled(!store.canSave)
        }
    }

    private func importEdits(takeTheirs: Bool) {
        do {
            try store.importProfile(
                profile.data, takeTheirs: takeTheirs, settingsJSON: spots.settingsJSON)
            problem = nil
            imported = true
        } catch {
            problem = SpeciesCatalogue.message(of: error)
        }
    }

    // MARK: - The sender's profile

    private func pageCard(_ page: SpeciesProfilePage) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            ForEach(page.fields, id: \.self) { row in
                SpeciesFieldLine(row: row)
                Divider()
            }
            ForEach(page.notices, id: \.self) { notice in
                Label {
                    Text(notice).fixedSize(horizontal: false, vertical: true)
                } icon: {
                    Image(systemName: "info.circle").foregroundStyle(Palette.accent)
                }
                .font(.footnote)
            }
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color(.secondarySystemGroupedBackground), in: .rect(cornerRadius: 14))
    }
}
