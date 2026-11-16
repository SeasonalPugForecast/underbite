import SwiftUI
import UnderbiteKit

extension SpotStore {
    /// Applies `change` to the settings if the result is valid; otherwise leaves them and
    /// returns the reason in words. `nil` means the change was kept.
    func edit(_ change: (inout Settings) -> Void) -> String? {
        do {
            try updateSettings(Preferences.applying(settingsJSON, change))
            return nil
        } catch {
            return Preferences.message(for: error)
        }
    }
}

/// What a preference page reads and writes: the store's settings, with the reason an edit was
/// refused shown by the page.
struct SettingsAccess {
    let store: SpotStore
    let problem: Binding<String?>

    /// The settings as they are now.
    var settings: Settings? { try? Settings(json: store.settingsJSON) }

    var json: String { store.settingsJSON }

    func edit(_ change: (inout Settings) -> Void) {
        problem.wrappedValue = store.edit(change)
    }

    /// A binding to one value of the settings; writing it is one validated edit.
    func binding<T>(
        _ read: @escaping (Settings) -> T, fallback: T,
        _ write: @escaping (inout Settings, T) -> Void
    ) -> Binding<T> {
        Binding(
            get: { settings.map(read) ?? fallback },
            set: { value in edit { write(&$0, value) } })
    }
}

/// A form for one preference page: it gives its content the settings and shows why an edit
/// was not kept.
struct PreferencePage<Content: View>: View {
    let title: String
    @ViewBuilder let content: (SettingsAccess) -> Content
    @Environment(SpotStore.self) private var store
    @State private var problem: String?

    var body: some View {
        Form {
            content(SettingsAccess(store: store, problem: $problem))
        }
        .navigationTitle(title)
        .navigationBarTitleDisplayMode(.inline)
        .alert(
            "That change was not kept",
            isPresented: Binding(
                get: { problem != nil }, set: { if !$0 { problem = nil } })
        ) {
            Button("OK", role: .cancel) {}
        } message: {
            Text(problem ?? "")
        }
    }
}

/// One value of the settings: a switch when it may be unset, the core's label for the value, a
/// slider over the core's round stops and a field for typing it in the user's unit. The value
/// is stored in the settings' own unit; only its label follows the unit setting.
struct QuantityRow: View {
    let title: String
    let quantity: Preferences.Quantity
    let settingsJSON: String
    /// The stored value in the field's unit.
    let value: Double?
    /// Whether the value may be unset (shows a switch).
    var optional = true
    var offLabel = "Not set"
    let onChange: (Double?) -> Void

    @State private var typed = ""
    @State private var typedProblem: String?

    var body: some View {
        let entry = Preferences.Entry(quantity, settingsJSON: settingsJSON)
        let stops = (try? Preferences.stopsHolding(value, entry: entry)) ?? []
        VStack(alignment: .leading, spacing: 8) {
            if optional {
                Toggle(
                    title,
                    isOn: Binding(
                        get: { value != nil },
                        set: { on in onChange(on ? defaultValue(stops) : nil) }))
            } else {
                Text(title)
            }
            if let value, stops.count > 1 {
                LabeledContent("Now", value: entry.label(of: value))
                    .font(.subheadline)
                Slider(
                    value: Binding(
                        get: { Double(Preferences.nearest(stops, to: value) ?? 0) },
                        set: { index in
                            let at = min(max(Int(index.rounded()), 0), stops.count - 1)
                            if stops[at].value != value { onChange(stops[at].value) }
                        }),
                    in: 0...Double(stops.count - 1), step: 1
                ) {
                    Text(title)
                } minimumValueLabel: {
                    Text(stops.first?.label ?? "").font(.caption2)
                } maximumValueLabel: {
                    Text(stops.last?.label ?? "").font(.caption2)
                }
                .accessibilityValue(entry.label(of: value))
                TextField("Type a value", text: $typed)
                    .keyboardType(.numbersAndPunctuation)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .submitLabel(.done)
                    .onSubmit { commit(entry) }
                if let typedProblem {
                    Text(typedProblem).font(.caption).foregroundStyle(.red)
                }
            } else if optional {
                Text(offLabel).font(.subheadline).foregroundStyle(.secondary)
            }
        }
    }

    private func defaultValue(_ stops: [Preferences.Stop]) -> Double? {
        stops.isEmpty ? nil : stops[stops.count / 2].value
    }

    private func commit(_ entry: Preferences.Entry) {
        let text = typed
        guard !text.trimmingCharacters(in: .whitespaces).isEmpty else { return }
        do {
            onChange(try entry.field(fromTyped: text))
            typed = ""
            typedProblem = nil
        } catch {
            typedProblem = Preferences.message(for: error)
        }
    }
}
