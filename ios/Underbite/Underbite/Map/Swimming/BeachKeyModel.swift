import Foundation

/// The compact key of the beach forecast's zone colours on the map: one entry per colour, the
/// core's words beside a swatch. It shows while the layer is on and the core has given its words
/// (they come with the zones it found); the drawer's own key is the same list.
nonisolated enum BeachKeyModel {
    struct Entry: Equatable, Sendable {
        var role: BeachRole
        var label: String
    }

    /// The entries to show: none while the layer is off, or when the core has no words yet;
    /// otherwise the core's legend in its order, with entries that repeat a colour left out.
    static func entries(layerOn: Bool, legend: [(role: String, label: String)]) -> [Entry] {
        guard layerOn else { return [] }
        var seen: Set<BeachRole> = []
        var out: [Entry] = []
        for item in legend where !item.label.isEmpty {
            let role = BeachRole(core: item.role)
            guard seen.insert(role).inserted else { continue }
            out.append(Entry(role: role, label: item.label))
        }
        return out
    }
}
