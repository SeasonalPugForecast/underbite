import Foundation

/// What a tap on the map does. A tap on a pin goes to the pin's module first; a tap on empty
/// map adds a point to a line being measured, drops the marker only while pin mode is on, and
/// otherwise does nothing, so panning and stopping a fling never open the inspector. A long press
/// always drops the marker.
nonisolated enum MapTapPolicy {
    /// What the inspector does with a tap no pin took.
    enum Action: Equatable, Sendable {
        /// The tap is one more point of the line being measured.
        case extendLine
        /// The marker drops and the inspector opens at the point.
        case dropMarker
        /// The tap is not used.
        case ignore
    }

    /// The action for a tap on empty map.
    static func action(measuring: Bool, pinMode: Bool) -> Action {
        if measuring { return .extendLine }
        return pinMode ? .dropMarker : .ignore
    }

    /// The first of `modules`, topmost first, that takes `tap`; none when no one did.
    static func deliver<Module, Tap>(
        _ tap: Tap, to modules: [Module], takes: (Module, Tap) -> Bool
    ) -> Module? {
        modules.first { takes($0, tap) }
    }
}
