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

/// The three ways a station's row can stand, one segment each: left to the ranking, always used
/// (pinned) or never used (excluded). Exactly one holds, so a pin and an exclusion never
/// both stand.
nonisolated enum StationMode: CaseIterable, Equatable, Sendable {
    case automatic, always, never

    /// The mode the row's switches stand for; a pin wins over an exclusion, as the core ranks.
    init(_ switches: StationSwitches) {
        self = switches.pinned ? .always : switches.excluded ? .never : .automatic
    }

    /// What picking this mode asks the core to remember: leaving a mode forgets it.
    var choice: StationChoice {
        switch self {
        case .automatic: .clear
        case .always: .pin
        case .never: .exclude
        }
    }
}

extension StationRow {
    /// The reasons that hold for every one of `rows` (when there are at least two), in the
    /// order of the first row: said once under the list instead of on each row.
    static func sharedReasons(_ rows: [StationRow]) -> [String] {
        guard let first = rows.first, rows.count > 1 else { return [] }
        return first.reasons.filter { reason in rows.allSatisfy { $0.reasons.contains(reason) } }
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
