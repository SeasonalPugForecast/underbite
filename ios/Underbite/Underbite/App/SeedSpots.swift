#if DEBUG
    import Foundation

    /// The test places of screenshots and tests (`-seed`): the same three the command line's
    /// examples use.
    nonisolated enum SeedSpots {
        /// The Battery, New York Harbor (CO-OPS 8518750).
        static let battery = Spot(
            id: "seed-battery", name: "The Battery", latitude: 40.7033, longitude: -74.0170,
            waterType: "tidal_river_estuary", timeZone: "America/New_York", order: 0)

        /// The Croton River site USGS 01375000 (the position its monitoring-location record gives).
        static let croton = Spot(
            id: "seed-croton", name: "Croton River", latitude: 41.2250277777778,
            longitude: -73.8588888888889, waterType: "river_stream",
            timeZone: "America/New_York", order: 1)

        /// NDBC buoy 44025, 30 NM south of Islip (its position in the active-stations list).
        static let buoy = Spot(
            id: "seed-ndbc-44025", name: "Buoy 44025", latitude: 40.258, longitude: -73.175,
            waterType: "offshore", timeZone: "America/New_York", order: 2)

        static var all: [Spot] { [battery, croton, buoy] }
    }

    extension SpotStore {
        /// Adds the test places that are not there yet; the Battery is shown.
        func addSeedSpots() {
            for spot in SeedSpots.all where self.spot(spot.id) == nil { add(spot) }
            if let first = self.spot(SeedSpots.battery.id) { setActive(first.id) }
        }
    }
#endif
