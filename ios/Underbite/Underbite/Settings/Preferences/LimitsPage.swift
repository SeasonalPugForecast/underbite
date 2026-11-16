import SwiftUI
import UnderbiteKit

/// The go/no-go profiles (Kayak, a boat, wading...): add, edit and delete.
struct LimitsPage: View {
    var body: some View {
        PreferencePage(title: "Go/no-go limits") { access in
            let limits = access.settings?.limits ?? []
            Section {
                ForEach(Array(limits.enumerated()), id: \.offset) { index, limit in
                    NavigationLink(value: LimitRoute(index: index)) {
                        VStack(alignment: .leading) {
                            Text(limit.name)
                            Text(summary(limit)).font(.caption).foregroundStyle(.secondary)
                        }
                    }
                }
                .onDelete { offsets in
                    access.edit { settings in settings.limits.remove(atOffsets: offsets) }
                }
                Button("Add a profile", systemImage: "plus") {
                    access.edit { $0.limits.append(Preferences.newLimit(existing: $0.limits)) }
                }
            } footer: {
                Text("A profile says what you will fish in. The conditions card checks the forecast against it.")
            }
        }
        .navigationDestination(for: LimitRoute.self) { LimitEditorPage(index: $0.index) }
    }

    private struct LimitRoute: Hashable { var index: Int }

    /// The switches that are on, as words; the numbers are on the editor's page.
    private func summary(_ limit: Settings.Limit) -> String {
        var parts: [String] = []
        let set = Preferences.LimitField.allCases.filter { $0.value(of: limit) != nil }.count
        if set > 0 { parts.append(set == 1 ? "1 limit" : "\(set) limits") }
        if limit.noSmallCraftAdvisory { parts.append("no small craft advisory") }
        if limit.noLightning { parts.append("no lightning") }
        return parts.isEmpty ? "No limits set" : parts.joined(separator: ", ")
    }
}

/// One profile: its name, each limit and the two weather switches.
struct LimitEditorPage: View {
    let index: Int

    var body: some View {
        PreferencePage(title: "Profile") { access in
            if let limit = access.settings?.limits[preferenceItem: index] {
                Section("Name") {
                    TextField(
                        "Name",
                        text: access.binding(
                            { $0.limits[preferenceItem: index]?.name ?? "" }, fallback: limit.name
                        ) { settings, name in
                            if settings.limits.indices.contains(index) { settings.limits[index].name = name }
                        })
                }
                Section {
                    ForEach(Preferences.LimitField.allCases) { field in
                        QuantityRow(
                            title: field.title, quantity: field.measurement,
                            settingsJSON: access.json, value: field.value(of: limit)
                        ) { value in
                            access.edit { settings in
                                if settings.limits.indices.contains(index) {
                                    field.set(value, on: &settings.limits[index])
                                }
                            }
                        }
                    }
                } header: {
                    Text("Limits")
                } footer: {
                    Text("A limit that is switched off is not checked.")
                }
                Section("Warnings") {
                    Toggle(
                        "No small craft advisory",
                        isOn: access.binding(
                            { $0.limits[preferenceItem: index]?.noSmallCraftAdvisory ?? false },
                            fallback: false
                        ) { settings, on in
                            if settings.limits.indices.contains(index) {
                                settings.limits[index].noSmallCraftAdvisory = on
                            }
                        })
                    Toggle(
                        "No lightning",
                        isOn: access.binding(
                            { $0.limits[preferenceItem: index]?.noLightning ?? false }, fallback: false
                        ) { settings, on in
                            if settings.limits.indices.contains(index) {
                                settings.limits[index].noLightning = on
                            }
                        })
                }
            } else {
                Text("This profile is gone.")
            }
        }
    }
}

fileprivate extension Array {
    /// The element at `index`, or nil when there is none.
    subscript(preferenceItem index: Int) -> Element? { indices.contains(index) ? self[index] : nil }
}
