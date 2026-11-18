import Foundation
@preconcurrency import UnderbiteKit

/// The area a box rule watches, drawn as a frame over the map: the box is the part of the map
/// inside the frame, which sits `inset` (a fraction of the map's width and height) in from each
/// edge. The map is Web Mercator, so longitudes are even across it and latitudes are not.
nonisolated enum AlertBoxFrame {
    /// How far in from each edge of the map the frame sits.
    static let inset = 0.1

    /// The box inside the frame for a map showing `west`...`east`, `south`...`north`.
    static func box(west: Double, south: Double, east: Double, north: Double) -> AlertDraft.Box {
        let top = mercator(north), bottom = mercator(south)
        let width = east - west
        return AlertDraft.Box(
            south: latitude(bottom + (top - bottom) * inset),
            west: west + width * inset,
            north: latitude(top - (top - bottom) * inset),
            east: east - width * inset)
    }

    /// The degrees of latitude a map needs to show so that `box` fills the frame, and its
    /// centre: where the map opens for a box already chosen.
    static func start(for box: AlertDraft.Box) -> (latitude: Double, longitude: Double, span: Double)
    {
        let top = mercator(box.north), bottom = mercator(box.south)
        let grow = (top - bottom) * inset / (1 - 2 * inset)
        let north = latitude(top + grow), south = latitude(bottom - grow)
        return (
            latitude: latitude((top + bottom) / 2), longitude: (box.west + box.east) / 2,
            span: north - south
        )
    }

    /// Why the core would refuse `box` as a rule's target (larger than 1°, or upside down), in
    /// its words; `nil` when it takes it. A rule that differs only in its target is put into a
    /// copy of the settings, which is then dropped.
    static func problem(_ box: AlertDraft.Box, settingsJSON: String) -> String? {
        let probe = AlertRuleDocument(
            id: "box-probe-\(UUID().uuidString)", name: "box",
            target: .box(south: box.south, west: box.west, north: box.north, east: box.east),
            allOf: [AlertRuleDocument.Condition(kind: "measurement", metric: "sea_surface_temp", above: 0)])
        guard let json = try? probe.json() else { return nil }
        do {
            _ = try putAlertRule(settingsJson: settingsJSON, ruleJson: json)
            return nil
        } catch AlertError.Invalid(let what, let message) where what.contains(".target") {
            return message
        } catch {
            return nil
        }
    }

    private static func mercator(_ latitude: Double) -> Double {
        let phi = min(max(latitude, -85.05), 85.05) * .pi / 180
        return log(tan(.pi / 4 + phi / 2))
    }

    private static func latitude(_ y: Double) -> Double {
        (2 * atan(exp(y)) - .pi / 2) * 180 / .pi
    }
}
