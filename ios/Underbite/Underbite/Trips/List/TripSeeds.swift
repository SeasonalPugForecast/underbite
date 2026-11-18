#if DEBUG
    import Foundation

    /// The sample trip of screenshots and tests (`-seedTrip`): three days at The Battery and the
    /// Croton River, the test places `-seed` adds.
    enum TripSeeds {
        /// Adds the sample trip, with the test places it is at.
        @MainActor
        static func seed(into store: SpotStore) {}
    }
#endif
