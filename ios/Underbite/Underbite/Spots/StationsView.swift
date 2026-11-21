import SwiftUI
import UnderbiteKit

/// The stations around one spot, per measurement in the core's order, each with a control to
/// leave it to the ranking, always use it or never use it, and the stations left out with why. Every word and
/// distance is the core's. The list shows the stations kept from the last lookup at once and
/// follows the new lookup; the choices go through the store, on its own host, and apply from
/// the next fetch.
struct StationsView: View {
    @Environment(SpotStore.self) private var store
    let spotID: String

    /// Choices made and not yet in the core's rows.
    @State private var pending: [String: StationChoice] = [:]
    @State private var lookedUp = false
    @State private var failure: String?

    var body: some View {
        let groups = store.stationRows(for: spotID)
        let looking = store.discovering.contains(spotID) || !lookedUp
        List {
            ForEach(groups, id: \.measurement) { group in
                let shared = StationRow.sharedReasons(group.rows)
                Section {
                    ForEach(group.rows, id: \.key) { row in
                        stationRow(row, hiding: shared)
                    }
                    if let title = group.leftOutTitle {
                        DisclosureGroup(title) {
                            ForEach(group.leftOut, id: \.key) { row in
                                stationRow(row)
                            }
                        }
                    }
                } header: {
                    Text(group.title)
                } footer: {
                    // What every row would say, said once.
                    if !shared.isEmpty {
                        VStack(alignment: .leading, spacing: 2) {
                            ForEach(shared, id: \.self) { Text($0) }
                        }
                    }
                }
            }
            Section {
                if let problem = store.stationProblems[spotID] {
                    Text("Some stations could not be looked up: \(problem)")
                        .foregroundStyle(Palette.warning)
                }
            } footer: {
                Text(
                    "A pin or an exclusion holds for this spot, in every measurement the station serves, from the next refresh. The others are ranked by distance, water body and freshness."
                )
            }
        }
        .readableText()
        .overlay {
            if groups.isEmpty {
                if looking {
                    LunaLoading()
                } else {
                    LunaEmptyState(kind: .conditions)
                }
            }
        }
        .toolbar {
            if looking, !groups.isEmpty {
                ToolbarItem(placement: .status) {
                    ProgressView().accessibilityLabel("Looking up stations")
                }
            }
        }
        .navigationTitle(store.spot(spotID).map { String(localized: "\($0.name): stations", comment: "Navigation title of the stations of a spot; the argument is the spot name") } ?? String(localized: "Stations", comment: "Navigation title of the stations of a spot"))
        .navigationBarTitleDisplayMode(.inline)
        .alert(
            "The choice was not saved",
            isPresented: Binding(get: { failure != nil }, set: { if !$0 { failure = nil } })
        ) {
            Button("OK", role: .cancel) {}
        } message: {
            Text(failure ?? "")
        }
        .task(id: spotID) {
            await store.discoverStations(for: spotID)
            lookedUp = true
        }
    }

    private func stationRow(_ row: StationRow, hiding shared: [String] = []) -> some View {
        StationRowView(row: row, hiding: shared, switches: StationSwitches.of(row, pending: pending)) { choice in
            apply(choice, to: row)
        }
    }

    /// Saves the choice through the store, shows it at once, and fetches the spot again so
    /// its page follows. Not tied to the list: closing it does not drop the fetch.
    private func apply(_ choice: StationChoice, to row: StationRow) {
        pending[row.key] = choice
        let id = spotID
        Task {
            do {
                try await store.chooseStation(choice, key: row.key, for: id)
            } catch {
                failure = SpotStore.message(of: error)
            }
            pending[row.key] = nil
            await store.refresh(only: [id])
        }
    }
}

/// One station: its name, where it comes from and how far, why it has its place (but for what
/// the section says once), and a control for automatic, always or never.
private struct StationRowView: View {
    let row: StationRow
    /// Reasons the section's footer says for every row.
    var hiding: [String] = []
    let switches: StationSwitches
    let choose: (StationChoice) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            VStack(alignment: .leading, spacing: 2) {
                Text(row.name).font(.headline)
                Text(row.detail)
                    .font(.caption)
                    .foregroundStyle(.readableSecondary)
                ForEach(row.reasons.filter { !hiding.contains($0) }, id: \.self) { reason in
                    Text(reason)
                        .font(.caption)
                        .foregroundStyle(
                            row.used || row.excluded
                                ? AnyShapeStyle(.readableSecondary) : AnyShapeStyle(Palette.warning))
                }
            }
            .accessibilityElement(children: .combine)
            // The station's name is the label VoiceOver reads for the control.
            Picker(
                row.name,
                selection: Binding(
                    get: { StationMode(switches) }, set: { choose($0.choice) })
            ) {
                Text("Automatic").tag(StationMode.automatic)
                Text("Always").tag(StationMode.always)
                Text("Never").tag(StationMode.never)
            }
            .pickerStyle(.segmented)
            .labelsHidden()
        }
        .padding(.vertical, 2)
    }
}
