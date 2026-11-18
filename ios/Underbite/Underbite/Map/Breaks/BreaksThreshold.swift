import Foundation
@preconcurrency import UnderbiteKit

/// How sharp a change must be to draw as a break, per breaks layer, kept in the settings'
/// `map.breaks` in the core's canonical units: `sst_c_per_km` (°C per km) for `sst-breaks`,
/// `chlorophyll_log10_per_km` (log10 units per km) for `chlorophyll-breaks`. The control moves
/// over the core's round stops in the user's units (`break_threshold_stops`), each with the value
/// to store and its label; nothing is converted here.
nonisolated enum BreaksThreshold {
    /// The stops a layer's threshold can take, weakest first.
    static func stops(layer: String, settingsJSON: String) -> [DisplayStop] {
        (try? breakThresholdStops(layer: layer, settingsJson: settingsJSON)) ?? []
    }

    /// The stop nearest `value` (canonical), as a position in `stops`; none without stops.
    static func position(of value: Double, in stops: [DisplayStop]) -> Int? {
        stops.indices.min { abs(stops[$0].value - value) < abs(stops[$1].value - value) }
    }

    /// The threshold of `layer` the settings hold, canonical; none for a layer without one.
    static func value(layer: String, in map: Settings.MapSettings) -> Double? {
        switch layer {
        case "sst-breaks": map.breaks.sstCPerKm
        case "chlorophyll-breaks": map.breaks.chlorophyllLog10PerKm
        default: nil
        }
    }

    /// Writes the stop at `position` as the threshold of `layer` into `map`; a position outside
    /// `stops` or a layer without a threshold changes nothing.
    static func set(
        _ position: Int, stops: [DisplayStop], layer: String, in map: inout Settings.MapSettings
    ) {
        guard stops.indices.contains(position) else { return }
        let value = stops[position].value
        switch layer {
        case "sst-breaks": map.breaks.sstCPerKm = value
        case "chlorophyll-breaks": map.breaks.chlorophyllLog10PerKm = value
        default: return
        }
    }
}
