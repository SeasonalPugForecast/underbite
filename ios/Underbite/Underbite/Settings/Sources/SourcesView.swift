import SwiftUI
import UnderbiteKit

/// The sources: one ordered chain for each kind of data. The first source of a chain that
/// answers serves, the others fill what it lacks. A switch takes a source in or out of the
/// chain, a handle reorders the sources in use, and the limits link to their page.
struct SourcesView: View {
    @Environment(SpotStore.self) private var store
    @State private var editMode: EditMode = .active
    @State private var problem: String?
    @State private var keys = KeyState(store: KeychainStore())

    private static let catalogue = sourceCatalogue()
    private static let shipped = (try? Settings.defaults())?.sources

    var body: some View {
        let settings = try? Settings(json: store.settingsJSON)
        List {
            Section {
                Text("Each kind of data tries its sources in this order. The first one with a reading near enough and fresh enough serves; the rest fill what it lacks.")
                    .font(.callout)
            }
            if let settings, let shipped = Self.shipped {
                ForEach(Preferences.SourceKind.allCases) { kind in
                    ChainSection(
                        kind: kind, settings: settings, shipped: shipped, keys: keys,
                        catalogue: Self.catalogue
                    ) { change in
                        problem = store.edit { change(&$0.sources, shipped) }
                    }
                }
            }
        }
        .readableText()
        .environment(\.editMode, $editMode)
        .navigationTitle("Sources")
        .navigationBarTitleDisplayMode(.inline)
        .onAppear { keys = KeyState(store: KeychainStore()) }
        .alert(
            "That change was not kept",
            isPresented: Binding(get: { problem != nil }, set: { if !$0 { problem = nil } })
        ) {
            Button("OK", role: .cancel) {}
        } message: {
            Text(problem ?? "")
        }
    }
}

/// One kind of data: its chain and its limits.
private struct ChainSection: View {
    let kind: Preferences.SourceKind
    let settings: Settings
    let shipped: Settings.Sources
    let keys: KeyState
    let catalogue: [SourceInfo]
    /// Applies an edit to the sources (with the shipped ones to reset to).
    let edit: ((inout Settings.Sources, Settings.Sources) -> Void) -> Void

    var body: some View {
        let rows = SourceChains.rows(
            kind, sources: settings.sources, defaults: shipped, catalogue: catalogue)
        let used = rows.filter(\.used)
        let unused = rows.filter { !$0.used }
        Section {
            ForEach(used) { row in
                ChainRow(row: row, info: info(row.id), keys: keys, isOn: binding(row))
            }
            .onMove { offsets, destination in
                edit { sources, _ in
                    SourceChains.move(kind, fromOffsets: offsets, toOffset: destination, in: &sources)
                }
            }
            ForEach(unused) { row in
                ChainRow(row: row, info: info(row.id), keys: keys, isOn: binding(row))
            }
            NavigationLink(value: PreferencePageID.freshness) {
                LabeledContent("Limits", value: limits)
            }
            if !SourceChains.isDefault(kind, sources: settings.sources, defaults: shipped) {
                Button("Use the usual order") {
                    edit { sources, shipped in
                        SourceChains.reset(kind, defaults: shipped, in: &sources)
                    }
                }
            }
        } header: {
            Text(kind.title(settingsJSON: (try? settings.json()) ?? ""))
        }
    }

    private func info(_ id: String) -> SourceInfo? { catalogue.first { $0.id == id } }

    private func binding(_ row: SourceChains.Row) -> Binding<Bool> {
        Binding(
            get: { row.used },
            set: { on in
                edit { sources, _ in
                    SourceChains.setUsed(on, id: row.id, kind: kind, in: &sources)
                }
            })
    }

    /// "within 25 km, under 3 h old", as the core words it.
    private var limits: String {
        let group = kind.group(in: settings.sources)
        return (try? sourceLimitsText(
            group: kind.fieldGroup, maxDistanceKm: group.maxDistanceKm,
            maxAgeMinutes: group.maxAgeMinutes.map(Int64.init),
            settingsJson: (try? settings.json()) ?? "")) ?? ""
    }
}

/// A source in a chain: its name, what it supplies, and a switch.
private struct ChainRow: View {
    let row: SourceChains.Row
    let info: SourceInfo?
    let keys: KeyState
    let isOn: Binding<Bool>

    var body: some View {
        Toggle(isOn: isOn) {
            VStack(alignment: .leading, spacing: 2) {
                Text(row.name)
                if let detail {
                    Text(detail).font(.caption).foregroundStyle(.readableSecondary)
                }
            }
        }
        .accessibilityElement(children: .combine)
    }

    private var detail: String? {
        guard row.known else { return "Not a source this version knows; it is skipped" }
        var parts: [String] = []
        if !row.implemented {
            parts.append("Not available yet; it is skipped")
        } else if let info {
            parts.append(info.measurements.joined(separator: ", "))
        }
        if let info, info.keyOptional { parts.append(keys.usgs ? "your key" : "no key") }
        return parts.isEmpty ? nil : parts.joined(separator: " · ")
    }
}
