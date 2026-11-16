import SwiftUI
import UnderbiteKit

/// The stations discovered around one spot, per measurement in the core's order, each with a
/// switch to always use it and one to never use it. The core remembers the choices for the spot
/// (with its station lists) and applies them from the next fetch; this screen asks it to
/// discover again so the list shows them at once.
struct StationsView: View {
    @Environment(SpotStore.self) private var store
    let spotID: String

    @State private var listing = StationListing(json: nil)
    @State private var loading = true
    @State private var failure: String?
    @State private var problem: String?
    @State private var names = Dictionary(
        uniqueKeysWithValues: sourceCatalogue().map { ($0.id, $0.name) })

    var body: some View {
        List {
            ForEach(listing.groups) { group in
                Section(group.title) {
                    ForEach(group.rows) { row in
                        StationRow(
                            row: row, sourceName: names[row.sourceID] ?? row.sourceID.uppercased()
                        ) { choice in
                            Task { await apply(choice, to: row) }
                        }
                    }
                }
            }
            Section {
                if let problem {
                    Text(problem).foregroundStyle(.orange)
                }
            } footer: {
                Text(
                    "A pin or an exclusion holds for this spot, in every measurement the station serves, from the next refresh. The core ranks the others by distance, water body and freshness."
                )
            }
        }
        .overlay {
            if listing.isEmpty {
                if loading {
                    LunaLoading()
                } else {
                    LunaEmptyState(kind: .conditions)
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
            listing = StationListing(json: store.cache(spotID).stationsJSON)
            if !listing.isEmpty { loading = false }
            await discover()
            loading = false
        }
    }

    /// Asks the core for the stations around the spot again and shows them; what it could not
    /// reach is said under the list.
    private func discover() async {
        guard let spot = store.spot(spotID) else { return }
        let settings = store.settingsJSON
        let host = StationChoiceWriter.host
        let now = Int64(Date().timeIntervalSince1970)
        do {
            let found = try await discoverStations(
                spot: spot.info, settingsJson: settings, keys: Keychain.keys(),
                allowMetered: true, now: now, host: host)
            let fresh = StationListing(json: found.stationsJson)
            if !fresh.isEmpty || listing.isEmpty { listing = fresh }
            problem = found.errors.first.map { "Some station lists could not be read: \($0.message)" }
        } catch {
            problem = "The stations could not be looked up: \(error)"
        }
    }

    private func apply(_ choice: StationChoice, to row: StationListing.Row) async {
        do {
            try StationChoiceWriter.write(choice, key: row.key, spotID: spotID)
        } catch {
            failure = String(describing: error)
            return
        }
        await discover()
        // The next fetch uses the choice; do it now so the page follows.
        let id = spotID
        Task { await store.refresh(only: [id]) }
    }
}

/// One station: its name, where it comes from, and the two switches.
private struct StationRow: View {
    let row: StationListing.Row
    let sourceName: String
    let choose: (StationChoice) -> Void

    private var detail: String {
        let bin = row.bin.map { ", bin \($0)" } ?? ""
        return "\(sourceName), \(row.stationID)\(bin)"
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            VStack(alignment: .leading, spacing: 2) {
                Text(row.name).font(.headline)
                Text(detail)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            .accessibilityElement(children: .combine)
            Toggle(
                "Always use",
                isOn: Binding(
                    get: { row.pinned }, set: { choose(.pinSwitched(on: $0)) }))
            Toggle(
                "Never use",
                isOn: Binding(
                    get: { row.excluded }, set: { choose(.excludeSwitched(on: $0)) }))
        }
        .padding(.vertical, 2)
    }
}

/// Writes a choice through the core's pin calls, into the same station store the fetches read
/// (the host's directory), so the next fetch and discovery see it.
@MainActor
enum StationChoiceWriter {
    /// The host over the app's station store.
    static var host: URLSessionHost { URLSessionHost(storeURL: AppFiles.standard.hostDirectory) }

    static func write(_ choice: StationChoice, key: String, spotID: String) throws {
        let host = Self.host
        let now = Int64(Date().timeIntervalSince1970)
        switch choice {
        case .pin: try pinStation(spotId: spotID, key: key, now: now, host: host)
        case .exclude: try excludeStation(spotId: spotID, key: key, now: now, host: host)
        case .clear: try clearStation(spotId: spotID, key: key, now: now, host: host)
        }
    }
}
