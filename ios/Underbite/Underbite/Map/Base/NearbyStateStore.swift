import Foundation
import UnderbiteKit

/// The nearby lookups' one state, as the core writes it (`NearbyResults.state_json`): the
/// Overpass gate and cell cache of the access lookups, the iNaturalist pacing and the GBIF pause
/// of the species records. Every map module that looks things up keeps it here, so their parts
/// never overwrite each other.
///
/// A lookup plans through ``plan(_:)``, which keeps what planning changed and gives the lookup
/// its base, and hands back the state the core returns as it goes. Lookups run side by side,
/// so a save keeps only what that lookup changed from its base and takes the rest as the store
/// holds it now: an access lookup's Overpass cells and a records lookup's pacing both stay.
/// Reads and writes are on the main actor, one at a time.
@MainActor
final class NearbyStateStore {
    /// The app's store, in `Application Support/underbite`.
    static let shared = NearbyStateStore(files: .standard)

    private let file: NearbyStateFile

    init(files: AppFiles) {
        file = NearbyStateFile(files: files)
    }

    /// The state kept now; empty for none.
    func load() -> String {
        file.load()
    }

    /// The plan `make` gives for the kept state, and the state to hand back with its saves
    /// (``save(_:base:)``). What planning itself changed (a request counted against a daily
    /// cap) is kept at once, so the next lookup plans with it even while this one runs. When
    /// the core cannot read the kept state but plans with none, the state is set aside and that
    /// plan is used; any other error is thrown.
    func plan(_ make: (String) throws -> NearbyPlan) throws -> (plan: NearbyPlan, base: String) {
        var state = file.load()
        let plan: NearbyPlan
        do {
            plan = try make(state)
        } catch NearbyError.Invalid where !state.isEmpty {
            plan = try make("")
            file.setAside()
            state = ""
        }
        guard let planned = plan.results().stateJson else { return (plan, state) }
        // Planning and this save run without a pause on the main actor: no other lookup plans
        // in between, so a base is never older than the state kept when it was taken.
        save(planned, base: state)
        return (plan, planned)
    }

    /// Keeps what `state` (from a lookup planned with `base`) changed, over the state kept now.
    @discardableResult
    func save(_ state: String, base: String) -> Bool {
        file.save(Self.merge(base: base, mine: state, current: file.load()))
    }

    /// `current` with the top-level parts `mine` changed from `base` taken from `mine`. With
    /// nothing kept, or text that is not a JSON object, `mine` as it is.
    nonisolated static func merge(base: String, mine: String, current: String) -> String {
        guard let changed = object(mine) else { return mine }
        guard var merged = object(current) else { return mine }
        let before = object(base) ?? [:]
        for (key, value) in changed where !same(before[key], value) {
            merged[key] = value
        }
        for key in before.keys where changed[key] == nil {
            merged[key] = nil
        }
        guard JSONSerialization.isValidJSONObject(merged),
            let data = try? JSONSerialization.data(withJSONObject: merged, options: [.sortedKeys]),
            let text = String(data: data, encoding: .utf8)
        else { return mine }
        return text
    }

    private nonisolated static func object(_ text: String) -> [String: Any]? {
        guard !text.isEmpty else { return nil }
        return (try? JSONSerialization.jsonObject(with: Data(text.utf8))) as? [String: Any]
    }

    private nonisolated static func same(_ a: Any?, _ b: Any) -> Bool {
        guard let a else { return false }
        return (a as AnyObject).isEqual(b)
    }
}

/// The file of the nearby state: `nearby-state.json`, written atomically; a file that cannot
/// be read as text is set aside as `nearby-state.json.bad` and never written over.
nonisolated struct NearbyStateFile: Sendable {
    static let name = "nearby-state.json"
    let files: AppFiles

    /// The kept state; empty for none (or a file that cannot be read as text, set aside).
    func load() -> String {
        do {
            return try files.utf8(Self.name) ?? ""
        } catch {
            _ = files.setAside(Self.name)
            return ""
        }
    }

    /// Keeps `json`; false when it could not be written.
    @discardableResult
    func save(_ json: String) -> Bool {
        files.write(Data(json.utf8), Self.name)
    }

    /// Sets the file aside; returns the name it is kept under.
    @discardableResult
    func setAside() -> String? {
        guard files.exists(Self.name) else { return nil }
        return files.setAside(Self.name)
    }
}
