import Foundation
import UnderbiteKit

/// What the sources page does with the settings, without any view: the order of each chain,
/// which sources a chain uses, and the keys' state. Foundation only, so the package tests
/// compile it too.
///
/// A chain is `[sources.<group>] chain`: the sources the settings name, in the order they are
/// tried. A source of the default chain that the user turned off is not in the chain; it stays
/// listed below the used ones so it can be turned on again. Ids this version cannot fetch from
/// stay in a chain: the core skips them at run time.
nonisolated enum SourceChains {
    typealias Kind = Preferences.SourceKind

    /// One line of a chain.
    struct Row: Identifiable, Equatable {
        var id: String
        /// The core's name for the source, or the id when the catalogue has no such source.
        var name: String
        /// In the chain (and so tried, in the order shown).
        var used: Bool
        /// This version can fetch from it.
        var implemented: Bool
        /// The catalogue knows the id.
        var known: Bool
    }

    /// The rows of `kind`: the chain in order, then the default chain's sources left out of it.
    static func rows(
        _ kind: Kind, sources: Settings.Sources, defaults: Settings.Sources, catalogue: [SourceInfo]
    ) -> [Row] {
        let byID = Dictionary(catalogue.map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })
        func row(_ id: String, used: Bool) -> Row {
            let info = byID[id]
            return Row(
                id: id, name: info?.name ?? id, used: used, implemented: info?.implemented ?? false,
                known: info != nil)
        }
        let chain = kind.group(in: sources).chain
        var seen = Set<String>()
        var out = chain.filter { seen.insert($0).inserted }.map { row($0, used: true) }
        for id in kind.group(in: defaults).chain where seen.insert(id).inserted {
            out.append(row(id, used: false))
        }
        return out
    }

    /// Moves the used rows at `offsets` so that they come before the used row now at
    /// `destination` (`List`'s `onMove` semantics, counted in used rows only).
    static func move(_ kind: Kind, fromOffsets offsets: IndexSet, toOffset destination: Int, in sources: inout Settings.Sources) {
        var group = kind.group(in: sources)
        group.chain = moved(group.chain, fromOffsets: offsets, toOffset: destination)
        kind.set(group, in: &sources)
    }

    /// `items` with the elements at `offsets` moved before the element now at `destination`.
    static func moved<T>(_ items: [T], fromOffsets offsets: IndexSet, toOffset destination: Int) -> [T] {
        let valid = offsets.filter { items.indices.contains($0) }
        guard !valid.isEmpty else { return items }
        let picked = valid.map { items[$0] }
        let before = valid.filter { $0 < destination }.count
        var rest = items
        for index in valid.sorted(by: >) { rest.remove(at: index) }
        let at = min(max(destination - before, 0), rest.count)
        rest.insert(contentsOf: picked, at: at)
        return rest
    }

    /// Takes `id` out of the chain, or puts it last when it is not in it.
    static func setUsed(_ used: Bool, id: String, kind: Kind, in sources: inout Settings.Sources) {
        var group = kind.group(in: sources)
        group.chain.removeAll { $0 == id }
        if used { group.chain.append(id) }
        kind.set(group, in: &sources)
    }

    /// The chain the core ships for `kind`; the distance and age limits stay as they are.
    static func reset(_ kind: Kind, defaults: Settings.Sources, in sources: inout Settings.Sources) {
        var group = kind.group(in: sources)
        group.chain = kind.group(in: defaults).chain
        kind.set(group, in: &sources)
    }

    /// Whether `kind` has the chain the core ships.
    static func isDefault(_ kind: Kind, sources: Settings.Sources, defaults: Settings.Sources) -> Bool {
        kind.group(in: sources).chain == kind.group(in: defaults).chain
    }
}

/// Where the user's keys are kept; the Keychain in the app.
nonisolated protocol KeyStoring {
    func value(for account: Keychain.Account) -> String?
    /// Stores `value`, or removes the item when it is empty.
    func set(_ value: String, for account: Keychain.Account)
}

/// The Keychain items of the app.
nonisolated struct KeychainStore: KeyStoring {
    func value(for account: Keychain.Account) -> String? { Keychain.key(for: account) }
    func set(_ value: String, for account: Keychain.Account) { Keychain.setKey(value, for: account) }
}

/// Which keys are saved. Never the keys themselves: a saved key is not shown again, only
/// reported as saved.
nonisolated struct KeyState: Equatable {
    var usgs = false
    var copernicusUser = false
    var copernicusPassword = false

    /// A Copernicus account needs both.
    var copernicusAccount: Bool { copernicusUser && copernicusPassword }

    init() {}

    init(store: some KeyStoring) {
        usgs = Self.saved(store, .usgs)
        copernicusUser = Self.saved(store, .copernicusUser)
        copernicusPassword = Self.saved(store, .copernicusPassword)
    }

    private static func saved(_ store: some KeyStoring, _ account: Keychain.Account) -> Bool {
        !(store.value(for: account) ?? "").isEmpty
    }

    /// Saves what was typed (nothing is saved for blank text) and returns the new state.
    static func save(_ text: String, as account: Keychain.Account, in store: some KeyStoring) -> KeyState {
        if !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { store.set(text, for: account) }
        return KeyState(store: store)
    }

    /// Deletes the Keychain item and returns the new state.
    static func remove(_ account: Keychain.Account, from store: some KeyStoring) -> KeyState {
        store.set("", for: account)
        return KeyState(store: store)
    }
}

/// The settings export: the TOML of the settings document and nothing else. It takes only the
/// settings, so the Keychain is out of its reach.
nonisolated enum SettingsExport {
    static func toml(settingsJSON: String) throws -> String {
        try exportSettingsToml(json: settingsJSON)
    }
}
