import Foundation
import UnderbiteKit

/// One section of the layer drawer: a group of the core's catalogue with its layers.
nonisolated struct DrawerGroup: Equatable, Sendable, Identifiable {
    /// The catalogue's group id (`base`, `water`, ...).
    let id: String
    /// The core's title for it.
    let title: String
    /// Base maps, one of which is chosen.
    let isBase: Bool
    /// The group's layers: base maps in catalogue order, overlays in drawing order (first at the
    /// bottom).
    var layers: [LayerInfo]
}

/// What the layer drawer lists and how its edits change the map settings: grouping, the order
/// within a group, and what the battery-friendly mode lets on. Nothing here touches SwiftUI, so
/// the rules are tested on their own.
nonisolated enum DrawerSections {
    /// The catalogue's groups in its order (the base maps first), each overlay group in the
    /// drawing order of `settings.layers`; a layer the settings do not list yet goes on top, in
    /// catalogue order, as the map draws it.
    static func groups(catalogue: [LayerInfo], settings: Settings.MapSettings) -> [DrawerGroup] {
        let position = Dictionary(
            settings.layers.enumerated().map { ($0.element.id, $0.offset) },
            uniquingKeysWith: { first, _ in first })
        var out: [DrawerGroup] = []
        for layer in catalogue {
            if let i = out.firstIndex(where: { $0.id == layer.group }) {
                out[i].layers.append(layer)
            } else {
                out.append(
                    DrawerGroup(
                        id: layer.group, title: layer.groupTitle, isBase: layer.isBase,
                        layers: [layer]))
            }
        }
        let catalogueIndex = Dictionary(
            catalogue.enumerated().map { ($0.element.id, $0.offset) },
            uniquingKeysWith: { first, _ in first })
        for i in out.indices where !out[i].isBase {
            out[i].layers.sort {
                let a = position[$0.id] ?? settings.layers.count + (catalogueIndex[$0.id] ?? 0)
                let b = position[$1.id] ?? settings.layers.count + (catalogueIndex[$1.id] ?? 0)
                return a < b
            }
        }
        return out
    }

    /// Moves layers within one group, as a list's `onMove` reports it: `ids` is the group's
    /// layers as listed. The other layers keep their places in `map.layers`; the group's layers
    /// take the places the group had, in the new order. A layer of the group the settings do not
    /// list yet is added, hidden.
    static func move(
        ids: [String], fromOffsets source: IndexSet, toOffset destination: Int,
        in map: inout Settings.MapSettings
    ) {
        let moved = moving(ids, fromOffsets: source, toOffset: destination)
        guard moved != ids else { return }
        for id in ids where !map.layers.contains(where: { $0.id == id }) {
            guard let added = layerState(id: id, like: map.layers.first) else { continue }
            map.layers.append(added)
        }
        let group = Set(ids)
        let places = map.layers.indices.filter { group.contains(map.layers[$0].id) }
        var states: [String: Settings.MapLayer] = [:]
        for place in places where states[map.layers[place].id] == nil {
            states[map.layers[place].id] = map.layers[place]
        }
        let ordered = moved.compactMap { states[$0] }
        guard ordered.count == places.count else { return }
        for (place, state) in zip(places, ordered) {
            map.layers[place] = state
        }
    }

    /// `list` with the items at `source` moved before the item at `destination` (an index into
    /// `list` before the move; `list.count` for the end), as a list's `onMove` means it.
    static func moving<T>(_ list: [T], fromOffsets source: IndexSet, toOffset destination: Int)
        -> [T]
    {
        let offsets = source.filter { list.indices.contains($0) }
        guard !offsets.isEmpty else { return list }
        let items = offsets.map { list[$0] }
        var out = list
        for offset in offsets.reversed() { out.remove(at: offset) }
        let at = min(max(destination - offsets.filter { $0 < destination }.count, 0), out.count)
        out.insert(contentsOf: items, at: at)
        return out
    }

    /// A hidden layer state for `id` with no opacity, ramp or range of its own, shaped like
    /// `template` (the settings' types have no public initialiser).
    static func layerState(id: String, like template: Settings.MapLayer?) -> Settings.MapLayer? {
        if var state = template {
            state.id = id
            state.shown = false
            state.opacity = nil
            state.ramp = nil
            state.range = nil
            return state
        }
        let json = #"{"id":"\#(id)","shown":false}"#
        return try? JSONDecoder().decode(Settings.MapLayer.self, from: Data(json.utf8))
    }

    /// The overlay rasters switched on (what the battery-friendly mode counts).
    static func shownRasters(in catalogue: [LayerInfo]) -> [LayerInfo] {
        catalogue.filter { !$0.isBase && $0.shown && $0.available && MapPlan.isRaster($0.draw) }
    }

    /// Whether the battery-friendly mode keeps `layer` from being switched on: it is a raster,
    /// it is off, and as many rasters as the mode draws are on already.
    static func batteryBlocks(_ layer: LayerInfo, catalogue: [LayerInfo], battery: Bool) -> Bool {
        battery && !layer.isBase && !layer.shown && MapPlan.isRaster(layer.draw)
            && shownRasters(in: catalogue).count >= MapPlan.maxRastersOnBattery
    }

    /// Whether a layer has an opacity to set: what the map view draws itself (tiles, rendered
    /// grids and arrows).
    static func hasOpacity(_ layer: LayerInfo) -> Bool {
        !layer.isBase && PlannedLayer.Kind(layer.draw) != nil
    }

    /// The legend bar of a layer in the drawer: the one the map draws for the range on screen
    /// when it has one, else the catalogue's.
    static func legend(_ layer: LayerInfo, drawn: MapLegendState?) -> [LegendStop] {
        guard let drawn, !drawn.stops.isEmpty else { return layer.legend }
        return drawn.stops
    }

    /// The stops labelled under a legend bar: the ends and the one nearest the middle.
    static func labelled(_ stops: [LegendStop]) -> [LegendStop] {
        guard let first = stops.first, let last = stops.last, stops.count > 1 else { return stops }
        let middle = stops.dropFirst().dropLast().min {
            abs($0.fraction - 0.5) < abs($1.fraction - 0.5)
        }
        return [first] + (middle.map { [$0] } ?? []) + [last]
    }
}
