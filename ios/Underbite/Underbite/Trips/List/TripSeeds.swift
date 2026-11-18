#if DEBUG
    import Foundation

    /// The sample trip of screenshots and tests (`-seedTrip`): three days at The Battery and the
    /// Croton River, the test places `-seed` adds, starting three days from today.
    enum TripSeeds {
        /// The sample trip's id: seeding again updates it.
        nonisolated static let id = "5EED0000-0000-4000-8000-000000000001"

        /// The sample trip at the test places of `spots`, from `today` (`YYYY-MM-DD`).
        nonisolated static func trip(today: String, spots: [Spot], areas: [Area] = [])
            -> String
        {
            var draft = TripDraft(
                first: TripDates.adding(3, to: today) ?? today, days: 3,
                places: [SeedSpots.battery.id, SeedSpots.croton.id])
            draft.id = id
            draft.name = "Hudson stripers"
            draft.species = "morone-saxatilis"
            draft.method = "shore"
            draft.notes = "Incoming tide at the Battery, then upriver to the Croton."
            return draft.json(spots: spots, areas: areas, settingsJSON: "{}")
        }

        /// Adds the sample trip to `trips`, and the test places it is at to `store` when they
        /// are not there yet.
        @MainActor
        static func seed(into store: SpotStore, trips: TripStore = .shared) {
            for spot in [SeedSpots.battery, SeedSpots.croton] where store.spot(spot.id) == nil {
                store.add(spot)
            }
            let json = trip(
                today: TripDates.day(of: Date()), spots: store.spots, areas: store.areas)
            _ = try? trips.save(json)
        }
    }
#endif
