import SwiftUI
import UnderbiteKit

/// The stations around one spot, per measurement in the core's order, each with a switch to
/// always use it and one to never use it, and the stations left out with why. Every word and
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
                Section(group.title) {
                    ForEach(group.rows, id: \.key) { row in
                        stationRow(row)
                    }
                    if let title = group.leftOutTitle {
                        DisclosureGroup(title) {
                            ForEach(group.leftOut, id: \.key) { row in
                                stationRow(row)
                            }
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
        .navigationTitle(store.spot(spotID).map { "\($0.name): stations" } ?? "Stations")
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

    private func stationRow(_ row: StationRow) -> some View {
        StationRowView(row: row, switches: StationSwitches.of(row, pending: pending)) { choice in
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

/// One station: its name, where it comes from and how far, why it has its place, and the two
/// switches.
private struct StationRowView: View {
    let row: StationRow
    let switches: StationSwitches
    let choose: (StationChoice) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            VStack(alignment: .leading, spacing: 2) {
                Text(row.name).font(.headline)
                Text(row.detail)
                    .font(.caption)
                    .foregroundStyle(.readableSecondary)
                ForEach(row.reasons, id: \.self) { reason in
                    Text(reason)
                        .font(.caption)
                        .foregroundStyle(
                            row.used || row.excluded
                                ? AnyShapeStyle(.readableSecondary) : AnyShapeStyle(Palette.warning))
                }
            }
            .accessibilityElement(children: .combine)
            Toggle(
                "Always use",
                isOn: Binding(
                    get: { switches.pinned }, set: { choose(.pinSwitched(on: $0)) }))
            Toggle(
                "Never use",
                isOn: Binding(
                    get: { switches.excluded }, set: { choose(.excludeSwitched(on: $0)) }))
        }
        .padding(.vertical, 2)
    }
}
