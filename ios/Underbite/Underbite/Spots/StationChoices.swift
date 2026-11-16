import Foundation
import UnderbiteKit

/// What a switch on a station's row asks the core to remember.
nonisolated enum StationChoice: Equatable, Sendable {
    case pin, exclude, clear

    /// The choice once the pin switch is set to `on` (off forgets the pin).
    static func pinSwitched(on: Bool) -> StationChoice { on ? .pin : .clear }
    /// The choice once the exclude switch is set to `on` (off forgets the exclusion).
    static func excludeSwitched(on: Bool) -> StationChoice { on ? .exclude : .clear }

    /// The row's two switches once this choice is made: a pin lifts an exclusion and the other
    /// way round, as the core does.
    var switches: StationSwitches {
        switch self {
        case .pin: StationSwitches(pinned: true, excluded: false)
        case .exclude: StationSwitches(pinned: false, excluded: true)
        case .clear: StationSwitches(pinned: false, excluded: false)
        }
    }
}

/// The two switches of a station's row.
nonisolated struct StationSwitches: Equatable, Sendable {
    var pinned: Bool
    var excluded: Bool

    /// The switches of `row`: a choice made and not yet in the core's rows wins over them.
    static func of(_ row: StationRow, pending: [String: StationChoice]) -> StationSwitches {
        pending[row.key]?.switches ?? StationSwitches(pinned: row.pinned, excluded: row.excluded)
    }
}
