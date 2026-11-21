import Foundation

/// Whether the map's time controls show as the small chip or as the full controls. The map starts
/// with the chip; tapping it opens the controls; choosing a day or the collapse control folds them
/// back; stepping a day or an hour keeps them open so several can be stepped through. Nothing
/// here is saved: the choice lasts as long as the map screen does.
nonisolated struct TimeControlsFold: Equatable, Sendable {
    /// What happened to the controls.
    enum Event: Equatable, Sendable {
        /// The chip was tapped.
        case expand
        /// A day was chosen from the menu, or "last clear day" was tapped.
        case pick
        /// A step button moved the day or the hour.
        case step
        /// The collapse control was tapped.
        case collapse
        /// The map was panned or zoomed.
        case pan
    }

    private(set) var expanded = false

    mutating func apply(_ event: Event) {
        switch event {
        case .expand: expanded = true
        case .step: break
        case .pick, .collapse, .pan: expanded = false
        }
    }
}
