import SwiftUI
import UnderbiteKit

/// The user's own ranges per river gauge: wadeable below, the best range, blown out above.
/// They are keyed by the gauge (`usgs:01420500`), not by a spot, and kept in cubic feet per
/// second whatever unit is shown.
struct RiversPage: View {
    @State private var gauge = ""
    @State private var gaugeProblem: String?

    var body: some View {
        PreferencePage(title: "River ranges") { access in
            let rivers = access.settings?.rivers ?? []
            Section {
                ForEach(Array(rivers.enumerated()), id: \.offset) { index, river in
                    NavigationLink(value: RiverRoute(index: index)) {
                        Text(river.site)
                    }
                }
                .onDelete { offsets in
                    access.edit { $0.rivers.remove(atOffsets: offsets) }
                }
            } footer: {
                if rivers.isEmpty { Text("No gauges yet.") }
            }
            Section {
                TextField("USGS gauge number, such as 01420500", text: $gauge)
                    .keyboardType(.numbersAndPunctuation)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .onSubmit { add(access) }
                if let gaugeProblem {
                    Text(gaugeProblem).font(.caption).foregroundStyle(.red)
                }
                Button("Add a gauge", systemImage: "plus") { add(access) }
                    .disabled(gauge.trimmingCharacters(in: .whitespaces).isEmpty)
            } header: {
                Text("Add a gauge")
            }
        }
        .navigationDestination(for: RiverRoute.self) { RiverEditorPage(index: $0.index) }
    }

    private struct RiverRoute: Hashable { var index: Int }

    private func add(_ access: SettingsAccess) {
        guard let site = Preferences.riverSite(from: gauge) else {
            gaugeProblem = "A gauge number is 8 or more digits, such as 01420500."
            return
        }
        gaugeProblem = nil
        if access.settings?.rivers.contains(where: { $0.site == site }) == true {
            gaugeProblem = "That gauge is already listed."
            return
        }
        access.edit { $0.rivers.append(Preferences.newRiver(site: site)) }
        gauge = ""
    }
}

/// One gauge's ranges.
struct RiverEditorPage: View {
    let index: Int

    var body: some View {
        PreferencePage(title: "Ranges") { access in
            if let river = access.settings.flatMap({ $0.rivers.indices.contains(index) ? $0.rivers[index] : nil }) {
                Section {
                    Text(river.site)
                } header: {
                    Text("Gauge")
                }
                Section {
                    QuantityRow(
                        title: "Wadeable below", quantity: Preferences.flow,
                        settingsJSON: access.json, value: river.wadeableBelowCfs
                    ) { value in
                        update(access) { $0.wadeableBelowCfs = value }
                    }
                    bestRange(river, access)
                    QuantityRow(
                        title: "Blown out above", quantity: Preferences.floodFlow,
                        settingsJSON: access.json, value: river.blownOutAboveCfs
                    ) { value in
                        update(access) { $0.blownOutAboveCfs = value }
                    }
                } footer: {
                    Text("Your own numbers for this gauge: the flow card shows where today's flow falls.")
                }
            } else {
                Text("This gauge is gone.")
            }
        }
    }

    @ViewBuilder
    private func bestRange(_ river: Settings.RiverRange, _ access: SettingsAccess) -> some View {
        let best = river.bestCfs.flatMap { $0.count == 2 ? $0 : nil }
        let entry = try? Preferences.Entry(Preferences.flow, settingsJSON: access.json)
        Toggle(
            "Best range",
            isOn: Binding(
                get: { best != nil },
                set: { on in
                    guard on, let stops = try? entry?.stops(), stops.count > 2 else {
                        update(access) { $0.bestCfs = nil }
                        return
                    }
                    let low = stops[stops.count / 3].value
                    let high = stops[2 * stops.count / 3].value
                    update(access) { $0.bestCfs = [low, high] }
                }))
        if let best {
            QuantityRow(
                title: "Best from", quantity: Preferences.flow, settingsJSON: access.json,
                value: best[0], optional: false
            ) { value in
                if let value { update(access) { $0.bestCfs = [value, best[1]] } }
            }
            QuantityRow(
                title: "Best up to", quantity: Preferences.flow, settingsJSON: access.json,
                value: best[1], optional: false
            ) { value in
                if let value { update(access) { $0.bestCfs = [best[0], value] } }
            }
        }
    }

    private func update(_ access: SettingsAccess, _ change: @escaping (inout Settings.RiverRange) -> Void) {
        access.edit { settings in
            if settings.rivers.indices.contains(index) { change(&settings.rivers[index]) }
        }
    }
}
