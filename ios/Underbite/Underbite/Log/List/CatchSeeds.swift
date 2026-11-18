#if DEBUG
    import Foundation

    /// The sample catches of screenshots and tests (`-seedCatches`), at the test places `-seed`
    /// adds.
    enum CatchSeeds {
        /// Adds the sample catches, and the test places they are at to `store` when they are not
        /// there yet.
        @MainActor
        static func seed(into store: SpotStore) {}
    }
#endif
