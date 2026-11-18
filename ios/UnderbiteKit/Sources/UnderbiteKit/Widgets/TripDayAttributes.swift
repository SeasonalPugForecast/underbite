import Foundation

// ActivityKit is on iOS and iPadOS; the package tests run on macOS, where it is not available
// at the package's minimum.
#if canImport(ActivityKit) && os(iOS)
    import ActivityKit

    /// The trip-day Live Activity: which trip it is for, fixed while it runs, and the core's
    /// content (`tripActivity(...)`) as its state.
    public struct TripDayAttributes: ActivityAttributes, Sendable, Hashable {
        /// The core's content as it is: every word ready, the countdown's target instants.
        public typealias ContentState = ViewModel.TripDayActivity

        /// The trip's id: one activity per trip, matched by it.
        public var tripID: String

        public init(tripID: String) {
            self.tripID = tripID
        }

        /// The content ActivityKit shows, stale from the core's `staleAt` on.
        public static func content(_ state: ContentState) -> ActivityContent<ContentState> {
            ActivityContent(state: state, staleDate: state.staleDate)
        }
    }
#endif
