import Foundation
import UnderbiteKit

/// A grid layer's colour range as the user sets it in the drawer: the ends picked from the
/// core's round values in the user's unit, stored in canonical units in the layer's `range`.
/// The core gives a range the user set precedence over the one fitted to the view
/// (`map_range`), so the map and its legend follow it.
nonisolated enum LayerRange {
    /// The measurement whose units a layer's values are in, for the core's round stops; none for
    /// a layer whose range is not set here.
    static func measurement(for id: String) -> String? {
        switch id {
        case "sst", "great-lakes-sst": "water_temperature"
        case "hf-radar-currents": "current_speed"
        default: nil
        }
    }

    /// The range a layer's colours span when the user sets none: its own.
    static func ownRange(of id: String, settings: Settings) -> ValueRange? {
        var plain = settings
        for i in plain.map.layers.indices where plain.map.layers[i].id == id {
            plain.map.layers[i].range = nil
        }
        guard let json = try? plain.json() else { return nil }
        return try? mapRange(settingsJson: json, layer: id, fitted: nil)
    }

    /// The round values the ends can take, in the user's unit, within the layer's own range.
    static func choices(for id: String, settings: Settings) -> [DisplayStop] {
        guard let measurement = measurement(for: id), let own = ownRange(of: id, settings: settings),
            let json = try? settings.json()
        else { return [] }
        return
            (try? displayStops(
                measurement: measurement, min: own.low, max: own.high, settingsJson: json)) ?? []
    }

    /// The range the user set for a layer, low and high; none when the colours fit the view.
    static func userRange(of id: String, in map: Settings.MapSettings) -> (low: Double, high: Double)? {
        guard let range = map.layers.first(where: { $0.id == id })?.range, range.count == 2 else {
            return nil
        }
        return (range[0], range[1])
    }

    /// Sets one end of a layer's range (the other end stays, or is the layer's own); false, and
    /// nothing changed, when the low end would not be under the high one.
    @discardableResult
    static func set(
        _ id: String, low: Double? = nil, high: Double? = nil, own: ValueRange,
        in map: inout Settings.MapSettings
    ) -> Bool {
        guard let i = map.layers.firstIndex(where: { $0.id == id }) else { return false }
        let current = userRange(of: id, in: map)
        let newLow = low ?? current?.low ?? own.low
        let newHigh = high ?? current?.high ?? own.high
        guard newLow < newHigh else { return false }
        map.layers[i].range = [newLow, newHigh]
        return true
    }

    /// Lets a layer's colours fit the view again.
    static func clear(_ id: String, in map: inout Settings.MapSettings) {
        for i in map.layers.indices where map.layers[i].id == id {
            map.layers[i].range = nil
        }
    }
}
