import Foundation

/// What the map's locate button knows: location access, whether it is precise, and whether the
/// map follows the phone's position. A tap turns into the steps to take; opening the map takes
/// none (access is only ever asked for from the button).
nonisolated struct LocateState: Equatable, Sendable {
    enum Access: Equatable, Sendable {
        /// Not asked yet.
        case undecided
        /// Refused, or not allowed on this phone.
        case refused
        /// Allowed "when in use".
        case allowed
    }

    /// What a tap asks for, in order.
    enum Step: Equatable, Sendable {
        /// Ask for location access "when in use".
        case askAccess
        /// Say that location access is off and where to turn it on.
        case explainRefused
        /// Ask for precise location once (access is approximate).
        case askPrecise
        /// Centre the map on the position and follow it.
        case follow
        /// Stop following (the map stays where it is).
        case stopFollowing
    }

    var access: Access = .undecided
    /// Whether the position is precise rather than approximate.
    var precise = true
    /// Whether the map follows the position.
    var following = false
    /// Precise location was asked for since the app started (asked once per launch at most).
    var askedPrecise = false
    /// A tap asked for access: the map follows once it is given.
    var followWhenAllowed = false

    /// Whether the map shows the position.
    var showsPosition: Bool { access == .allowed }

    /// A tap on the button: the steps to take, with the state moved on.
    mutating func tap() -> [Step] {
        switch access {
        case .undecided:
            followWhenAllowed = true
            return [.askAccess]
        case .refused:
            return [.explainRefused]
        case .allowed:
            if following {
                following = false
                return [.stopFollowing]
            }
            var steps: [Step] = []
            if !precise, !askedPrecise {
                askedPrecise = true
                steps.append(.askPrecise)
            }
            following = true
            steps.append(.follow)
            return steps
        }
    }

    /// Location access changed (or was read the first time); true when the map now follows the
    /// position because a tap asked for access and it was given.
    mutating func accessChanged(to access: Access, precise: Bool) -> Bool {
        self.access = access
        self.precise = precise
        if access != .allowed { following = false }
        guard access != .undecided, followWhenAllowed else { return false }
        followWhenAllowed = false
        guard access == .allowed else { return false }
        following = true
        return true
    }

    /// The map stopped following by itself (the map was moved by hand).
    mutating func stoppedFollowing() {
        following = false
    }

    /// The button's symbol.
    var symbol: String {
        switch access {
        case .refused: "location.slash"
        case .undecided, .allowed: following ? "location.fill" : "location"
        }
    }

    /// What VoiceOver reads after the button's label.
    var accessibilityValue: String {
        switch access {
        case .refused: String(localized: "Location access is off", comment: "What VoiceOver reads after the locate button when location access is off")
        case .undecided, .allowed: following ? String(localized: "Following", comment: "What VoiceOver reads after the locate button when the map follows the location") : ""
        }
    }
}
