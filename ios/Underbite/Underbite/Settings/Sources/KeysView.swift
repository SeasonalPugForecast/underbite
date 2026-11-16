import SwiftUI
import UnderbiteKit

/// The keys and accounts of the sources that take one. They live in the Keychain of this
/// phone only: not in the settings, not in an export, not in a backup. A saved key is never
/// shown again, only reported as saved.
struct KeysView: View {
    @Environment(SpotStore.self) private var store
    @State private var keys = KeyState(store: KeychainStore())
    @State private var usgsDraft = ""
    @State private var userDraft = ""
    @State private var passwordDraft = ""

    private static let catalogue = sourceCatalogue()
    private var usgs: SourceInfo? { Self.catalogue.first { $0.id == "usgs" } }
    private var copernicus: SourceInfo? { Self.catalogue.first { $0.id == "copernicus" } }

    var body: some View {
        List {
            Section {
                LabeledContent("Key", value: keys.usgs ? "Key saved" : "No key")
                SecureField("Paste a key", text: $usgsDraft)
                    .modifier(KeyField())
                Button("Save key") {
                    keys = KeyState.save(usgsDraft, as: .usgs, in: KeychainStore())
                    usgsDraft = ""
                }
                .disabled(blank(usgsDraft))
                if keys.usgs {
                    Button("Delete key", role: .destructive) {
                        keys = KeyState.remove(.usgs, from: KeychainStore())
                    }
                }
            } header: {
                Text(usgs?.name ?? "USGS")
            } footer: {
                VStack(alignment: .leading, spacing: 6) {
                    Text("A key is optional. Rivers and lakes work without one; a key of your own raises your phone's allowance of requests.")
                    if let url = usgs?.url.flatMap(URL.init(string:)) {
                        Link("The USGS Water Data site", destination: url)
                    }
                    Text(Self.neverExported)
                }
            }

            Section {
                Toggle("Use Copernicus Marine", isOn: copernicusEnabled)
                    .disabled(!(copernicus?.implemented ?? false) || !keys.copernicusAccount)
                if !(copernicus?.implemented ?? false) {
                    Text("Not available yet. The switch stays off until the app can use it.")
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                }
                LabeledContent("User name", value: keys.copernicusUser ? "Saved" : "Not saved")
                TextField("User name", text: $userDraft)
                    .modifier(KeyField())
                LabeledContent("Password", value: keys.copernicusPassword ? "Saved" : "Not saved")
                SecureField("Password", text: $passwordDraft)
                    .modifier(KeyField())
                Button("Save account") {
                    keys = KeyState.save(userDraft, as: .copernicusUser, in: KeychainStore())
                    keys = KeyState.save(passwordDraft, as: .copernicusPassword, in: KeychainStore())
                    userDraft = ""
                    passwordDraft = ""
                }
                .disabled(blank(userDraft) && blank(passwordDraft))
                if keys.copernicusUser || keys.copernicusPassword {
                    Button("Delete account", role: .destructive) {
                        _ = KeyState.remove(.copernicusUser, from: KeychainStore())
                        keys = KeyState.remove(.copernicusPassword, from: KeychainStore())
                        store.setCopernicusEnabled(false)
                    }
                }
            } header: {
                Text(copernicus?.name ?? "Copernicus Marine")
            } footer: {
                VStack(alignment: .leading, spacing: 6) {
                    Text("Off unless you turn it on, and it needs your own Copernicus Marine account.")
                    if let url = copernicus?.url.flatMap(URL.init(string:)) {
                        Link("Copernicus Marine", destination: url)
                    }
                    Text(Self.neverExported)
                }
            }
        }
        .navigationTitle("Keys and accounts")
        .navigationBarTitleDisplayMode(.inline)
        .onAppear { keys = KeyState(store: KeychainStore()) }
    }

    private static let neverExported =
        "Kept in this phone's Keychain only. Never in the settings, never exported, never sent anywhere but to the source it belongs to."

    private func blank(_ text: String) -> Bool {
        text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    /// Off, and unchangeable, until the app has a Copernicus source.
    private var copernicusEnabled: Binding<Bool> {
        guard copernicus?.implemented ?? false else { return .constant(false) }
        return Binding(
            get: { (try? Settings(json: store.settingsJSON))?.providers.copernicus.enabled ?? false },
            set: { store.setCopernicusEnabled($0) })
    }
}

/// A field for a key: no capitals, no corrections, no stored suggestions.
private struct KeyField: ViewModifier {
    func body(content: Content) -> some View {
        content
            .textContentType(.none)
            .textInputAutocapitalization(.never)
            .autocorrectionDisabled()
    }
}

extension SpotStore {
    /// The opt-in for Copernicus Marine, in the settings (the account itself is in the Keychain).
    func setCopernicusEnabled(_ on: Bool) {
        _ = edit { $0.providers.copernicus.enabled = on }
    }
}
