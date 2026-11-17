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
                            Text(summary(limit, json: access.json)).font(.caption).foregroundStyle(.readableSecondary)
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

    /// The limits that are set and the switches that are on, as the core words them.
    private func summary(_ limit: Settings.Limit, json: String) -> String {
        let set = Preferences.LimitField.allCases.filter { $0.value(of: limit) != nil }.count
        return (try? limitSummary(
            setCount: UInt32(set), noSmallCraftAdvisory: limit.noSmallCraftAdvisory,
            noLightning: limit.noLightning, settingsJson: json)) ?? ""
    }
}

/// One profile: its name, each limit and the two weather switches.
struct LimitEditorPage: View {
    let index: Int

    var body: some View {
        PreferencePage(title: "Profile") { access in
            if let limit = access.settings?.limits[preferenceItem: index] {
                Section("Name") {
                    ProfileNameField(name: limit.name) { name in
                        access.edit { settings in
                            if settings.limits.indices.contains(index) { settings.limits[index].name = name }
                        }
                    }
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

/// The name of a profile. Typing changes a draft only; the name is written when it is
/// submitted or the field loses focus, so clearing the field to type another name is not an
/// edit. An empty draft goes back to the stored name.
private struct ProfileNameField: View {
    let name: String
    let commit: (String) -> Void
    @State private var draft: String
    @FocusState private var focused: Bool

    init(name: String, commit: @escaping (String) -> Void) {
        self.name = name
        self.commit = commit
        _draft = State(initialValue: name)
    }

    var body: some View {
        TextField("Name", text: $draft)
            .focused($focused)
            .submitLabel(.done)
            .onSubmit(finish)
            .onChange(of: focused) { _, isFocused in if !isFocused { finish() } }
            .onChange(of: name) { _, stored in draft = stored }
    }

    private func finish() {
        if let renamed = Preferences.renamed(draft: draft, current: name) {
            commit(renamed)
        }
        draft = name
    }
}

fileprivate extension Array {
    /// The element at `index`, or nil when there is none.
    subscript(preferenceItem index: Int) -> Element? { indices.contains(index) ? self[index] : nil }
}
