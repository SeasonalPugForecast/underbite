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

        /// The sample limits profile: a kayak.
        static var limits: [String: Any] {
            [
            "name": "Kayak", "max_wind_kn": 12.0, "max_gust_kn": 18.0, "max_wave_m": 0.6,
            "no_small_craft_advisory": true, "no_lightning": true,
            ]
        }

        static var all: [Spot] { [battery, croton, buoy] }

        /// Beltzville Lake, Pennsylvania: a reservoir with an operator's releases and inflow,
        /// and a lake the satellite bloom estimate covers.
        static let beltzville = Spot(
            id: "seed-beltzville", name: "Beltzville Lake", latitude: 40.8550, longitude: -75.6270,
            waterType: "lake_reservoir", timeZone: "America/New_York", order: 3)

        /// Clearwater Beach, Florida: a Gulf beach with a coastal bloom forecast.
        static let clearwater = Spot(
            id: "seed-clearwater", name: "Clearwater Beach", latitude: 27.9780,
            longitude: -82.8300, waterType: "surf", timeZone: "America/New_York", order: 4)

        /// The places of the bloom and reservoir screens (`-seedBloom`).
        static var bloom: [Spot] { [beltzville, clearwater] }

        /// Whether the launch asks for the bloom places.
        static var wantsBloom: Bool { ProcessInfo.processInfo.arguments.contains("-seedBloom") }
    }

    extension SpotStore {
        /// Adds the test places that are not there yet; the Battery is shown. With `-seedBloom`
        /// also the lake and the beach, and the bloom card in their cards.
        func addSeedSpots() {
            let bloom = SeedSpots.wantsBloom ? SeedSpots.bloom : []
            for spot in SeedSpots.all + bloom where self.spot(spot.id) == nil { add(spot) }
            if let first = self.spot(SeedSpots.battery.id) { setActive(first.id) }
            if !bloom.isEmpty { addSeedBloomCard(for: bloom.map(\.waterType)) }
        }

        /// Gives each of `waterTypes` a layout of its own: the default order with the bloom card
        /// after the water temperature, which the shipped order leaves out. The default order
        /// itself stays as shipped, so the river keeps its own cards (stocking among them).
        private func addSeedBloomCard(for waterTypes: [String]) {
            let keys = ["lake_reservoir": "lake", "surf": "surf"]
            guard
                let data = settingsJSON.data(using: .utf8),
                var settings = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any],
                var cards = settings["cards"] as? [String: Any],
                var order = cards["order"] as? [String]
            else { return }
            if !order.contains("bloom") {
                order.insert("bloom", at: (order.firstIndex(of: "water_temp") ?? -1) + 1)
            }
            for type in waterTypes {
                guard let key = keys[type], cards[key] == nil else { continue }
                cards[key] = ["order": order, "hidden": [String]()]
            }
            settings["cards"] = cards
            guard
                let out = try? JSONSerialization.data(
                    withJSONObject: settings, options: [.sortedKeys]),
                let json = String(data: out, encoding: .utf8)
            else { return }
            try? updateSettings(json)
        }

        /// Puts a sample `[[limits]]` profile into the settings and the go/no-go card into the
        /// card order (`-seedLimits`): the shipped order leaves that card out. Plain test
        /// values, saved like any settings change.
        func addSeedLimits() {
            guard
                let data = settingsJSON.data(using: .utf8),
                var settings = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any]
            else { return }
            settings["limits"] = [SeedSpots.limits]
            if var cards = settings["cards"] as? [String: Any],
                var order = cards["order"] as? [String], !order.contains("go_no_go")
            {
                order.insert("go_no_go", at: (order.firstIndex(of: "windows") ?? -1) + 1)
                cards["order"] = order
                settings["cards"] = cards
            }
            guard
                let out = try? JSONSerialization.data(
                    withJSONObject: settings, options: [.sortedKeys]),
                let json = String(data: out, encoding: .utf8)
            else { return }
            try? updateSettings(json)
        }
    }
#endif
