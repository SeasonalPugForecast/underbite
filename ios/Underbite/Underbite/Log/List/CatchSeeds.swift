#if DEBUG
    import Foundation
    @preconcurrency import UnderbiteKit

    /// The sample catches of screenshots and tests (`-seedCatches`): five catches at The Battery
    /// and the Croton River, the test places `-seed` adds, each with the conditions of a small
    /// hand-made snapshot of its spot (a tide, water temperature, pressure and wind at the
    /// Battery; flow and water temperature on the Croton). Their times are fixed, so the list
    /// reads the same on every run. `-seedCatches many` adds a season of striped bass at the
    /// Battery, enough catches for what worked to read a pattern.
    nonisolated enum CatchSeeds {
        /// One sample catch.
        struct Seed: Sendable {
            var id: String
            var caughtAt: Int64
            var spot: Spot
            var species: String
            /// The length typed in inches, and in centimetres.
            var inches: String
            var centimetres: String
            /// The weight typed in whole pounds, and in kilograms; blank for none.
            var pounds: String = ""
            var kilograms: String = ""
            var fate: FateId
            var lure: String?
            var method: String?
            var notes: String?
            /// The water temperature, °C, and the flow, m³/s (the Croton only).
            var waterC: Double
            var flow: Double? = nil
            /// Seconds from the last high water to the catch (the Battery only); past the low
            /// that follows it, about 6.25 h, the tide is rising.
            var sinceHigh: Int64 = 14_700
        }

        /// The sample catches, newest first.
        static let seeds: [Seed] = [
            Seed(
                id: "5EED0000-0000-4000-8000-0000000000C1", caughtAt: 1_794_687_600,
                spot: SeedSpots.croton, species: "salmo-trutta", inches: "14", centimetres: "36",
                fate: .released, lure: "Bead-head nymph", method: "Fly",
                notes: "Below the spillway pool.", waterC: 9.4, flow: 2.9),
            Seed(
                id: "5EED0000-0000-4000-8000-0000000000C2", caughtAt: 1_794_136_200,
                spot: SeedSpots.battery, species: "morone-saxatilis", inches: "28",
                centimetres: "71", pounds: "9", kilograms: "4", fate: .released,
                lure: "Bucktail jig", method: "Shore", notes: "First light on the ebb.",
                waterC: 12.8),
            Seed(
                id: "5EED0000-0000-4000-8000-0000000000C3", caughtAt: 1_792_878_000,
                spot: SeedSpots.battery, species: "pomatomus-saltatrix", inches: "22",
                centimetres: "56", pounds: "3", kilograms: "1", fate: .kept,
                lure: "Metal spoon", method: "Shore", waterC: 16.1),
            Seed(
                id: "5EED0000-0000-4000-8000-0000000000C4", caughtAt: 1_791_724_500,
                spot: SeedSpots.battery, species: "centropristis-striata", inches: "15",
                centimetres: "38", fate: .keptForSashimi, method: "Bottom rig",
                waterC: 18.3),
            Seed(
                id: "5EED0000-0000-4000-8000-0000000000C5", caughtAt: 1_790_508_600,
                spot: SeedSpots.croton, species: "micropterus-dolomieu", inches: "16",
                centimetres: "41", pounds: "2", kilograms: "1", fate: .released,
                lure: "Tube jig", method: "Wading", waterC: 17.6, flow: 1.8),
        ]

        /// The striped bass `-seedCatches many` adds: a spring and a fall run at the Battery,
        /// most of them on the first hours of the ebb in 12 to 14 °C water.
        static let more: [Seed] = [
            bass("D1", 1_793_532_600, inches: "30", centimetres: "76", waterC: 13.1, sinceHigh: 9_000),
            bass("D2", 1_793_479_800, inches: "24", centimetres: "61", waterC: 13.6, sinceHigh: 10_800),
            bass("D3", 1_792_840_500, inches: "27", centimetres: "69", waterC: 14.4, sinceHigh: 8_100),
            bass("D4", 1_792_272_600, inches: "33", centimetres: "84", waterC: 15.2, sinceHigh: 26_100),
            bass("D5", 1_791_629_100, inches: "22", centimetres: "56", waterC: 16.4, sinceHigh: 9_900),
            bass("D6", 1_791_065_400, inches: "29", centimetres: "74", waterC: 17.1, sinceHigh: 4_500),
            bass("D7", 1_780_738_200, inches: "35", centimetres: "89", waterC: 17.8, sinceHigh: 29_700),
            bass("D8", 1_780_139_100, inches: "26", centimetres: "66", waterC: 15.9, sinceHigh: 9_300),
            bass("D9", 1_779_581_700, inches: "31", centimetres: "79", waterC: 14.8, sinceHigh: 12_600),
            bass("DA", 1_778_925_000, inches: "25", centimetres: "64", waterC: 13.4, sinceHigh: 7_800),
            bass("DB", 1_778_370_000, inches: "23", centimetres: "58", waterC: 12.6, sinceHigh: 24_300),
            bass("DC", 1_777_717_200, inches: "28", centimetres: "71", waterC: 12.2, sinceHigh: 10_200),
        ]

        /// A striped bass released at the Battery.
        private static func bass(
            _ tag: String, _ caughtAt: Int64, inches: String, centimetres: String, waterC: Double,
            sinceHigh: Int64
        ) -> Seed {
            Seed(
                id: "5EED0000-0000-4000-8000-0000000000\(tag)", caughtAt: caughtAt,
                spot: SeedSpots.battery, species: "morone-saxatilis", inches: inches,
                centimetres: centimetres, fate: .released, lure: "Bucktail jig", method: "Shore",
                waterC: waterC, sinceHigh: sinceHigh)
        }

        /// The catches the launch arguments ask for: the five, and the season of striped bass
        /// after `-seedCatches many`.
        static func seeds(for arguments: [String]) -> [Seed] {
            guard let at = arguments.firstIndex(of: "-seedCatches"), at + 1 < arguments.count,
                arguments[at + 1] == "many"
            else { return seeds }
            return seeds + more
        }

        /// The editor's fields of `seed`, its sizes typed in `units`.
        static func input(_ seed: Seed, units: FishUnitsInfo) -> CatchInput {
            let inches = units.lengthUnit == "inches"
            let pounds = units.weightUnit == "pounds"
            return CatchInput(
                id: seed.id, caughtAt: seed.caughtAt, timeZone: seed.spot.timeZone,
                latitude: seed.spot.latitude, longitude: seed.spot.longitude,
                spotId: seed.spot.id, area: nil, species: seed.species, speciesText: nil,
                lengthText: inches ? seed.inches : seed.centimetres,
                weightText: pounds ? seed.pounds : seed.kilograms, ouncesText: "",
                fate: seed.fate, lure: seed.lure, method: seed.method, notes: seed.notes,
                photos: [])
        }

        /// Saves the sample catches `chosen` into `catches` with their conditions; seeding again
        /// updates them.
        @MainActor
        static func seed(
            catches: CatchStore, settingsJSON: String, chosen: [Seed] = CatchSeeds.seeds
        ) {
            guard let units = try? fishUnitsInfo(settingsJson: settingsJSON) else { return }
            for seed in chosen {
                guard (try? catches.save(input(seed, units: units), settingsJSON: settingsJSON)) != nil
                else { continue }
                _ = try? catches.attachConditions(to: seed.id, snapshotJSON: snapshot(seed))
            }
        }

        /// Adds the sample catches to the log, and the test places they are at to `store` when
        /// they are not there yet.
        @MainActor
        static func seed(into store: SpotStore, catches: CatchStore = .shared) {
            for spot in [SeedSpots.battery, SeedSpots.croton] where store.spot(spot.id) == nil {
                store.add(spot)
            }
            seed(
                catches: catches, settingsJSON: store.settingsJSON,
                chosen: seeds(for: ProcessInfo.processInfo.arguments))
        }

        // MARK: - The hand-made snapshots

        /// A snapshot of the seed's spot around its catch time, shaped like a fetch's.
        static func snapshot(_ seed: Seed) -> String {
            let t = seed.caughtAt
            let high = t - seed.sinceHigh
            let battery = seed.spot.id == SeedSpots.battery.id
            let station = battery ? coops : usgs
            let source = battery ? "coops" : "usgs"
            var water: [String: Any] = [
                "temperature_c": series(
                    source, station, observedAt: t - 360,
                    [(t - 1_800, seed.waterC - 0.1), (t - 360, seed.waterC)])
            ]
            var marine: [String: Any] = [:]
            if let flow = seed.flow {
                water["discharge_m3s"] = series(
                    source, station, observedAt: t - 900, [(t - 1_800, flow), (t - 900, flow)])
            }
            if battery {
                marine["tide"] = [
                    "source_kind": "harmonic_station", "datum": "mllw",
                    "events": [
                        ["time": high, "kind": "high", "height_m": 1.52],
                        ["time": high + 22_500, "kind": "low", "height_m": 0.12],
                        ["time": high + 44_700, "kind": "high", "height_m": 1.61],
                    ] as [[String: Any]],
                    "observed": series(
                        source, station, observedAt: t - 360,
                        [(t - 720, 0.98), (t - 360, 0.95)]),
                    "provenance": provenance(source, station, kind: "predicted", observedAt: t),
                ] as [String: Any]
                marine["observed_pressure_hpa"] = series(
                    source, station, observedAt: t - 360,
                    [
                        (t - 14_400, 1_010.6), (t - 10_800, 1_011.2), (t - 7_200, 1_012.0),
                        (t - 3_600, 1_012.9),
                        (t - 360, 1_013.6),
                    ])
                marine["observed_wind"] = [
                    "provenance": provenance(source, station, kind: "observed", observedAt: t - 360),
                    "samples": [
                        [
                            "time": t - 360,
                            "value": ["speed_ms": 4.1, "gust_ms": 6.2, "from_deg": 300.0],
                        ] as [String: Any]
                    ],
                ] as [String: Any]
            }
            let snapshot: [String: Any] = [
                "spot": [
                    "id": seed.spot.id, "name": seed.spot.name, "latitude": seed.spot.latitude,
                    "longitude": seed.spot.longitude, "water_type": seed.spot.waterType,
                    "time_zone": seed.spot.timeZone,
                ] as [String: Any],
                "water": water, "marine": marine, "fetched_at": t,
                "stations": [station],
                "attribution": [
                    battery
                        ? ["source": "coops", "credit": "NOAA CO-OPS", "url": "https://tidesandcurrents.noaa.gov"]
                        : ["source": "usgs", "credit": "U.S. Geological Survey", "url": "https://waterdata.usgs.gov"]
                ],
            ]
            guard
                let data = try? JSONSerialization.data(
                    withJSONObject: snapshot, options: [.sortedKeys]),
                let text = String(data: data, encoding: .utf8)
            else { return "{}" }
            return text
        }

        /// CO-OPS 8518750, The Battery.
        private static var coops: [String: Any] {
            [
                "source": "coops", "id": "8518750", "name": "The Battery", "kind": "tide_harmonic",
                "latitude": 40.7003, "longitude": -74.0142,
            ]
        }

        /// USGS 01375000, the Croton River at New Croton Dam.
        private static var usgs: [String: Any] {
            [
                "source": "usgs", "id": "01375000", "name": "Croton River at New Croton Dam",
                "kind": ["usgs_site": "stream"], "latitude": SeedSpots.croton.latitude,
                "longitude": SeedSpots.croton.longitude,
            ]
        }

        private static func provenance(
            _ source: String, _ station: [String: Any], kind: String, observedAt: Int64
        ) -> [String: Any] {
            [
                "source": source, "station": station,
                "distance_m": source == "coops" ? 410.0 : 0.0, "relation": "same_water_body",
                "observed_at": observedAt, "kind": kind, "quality": "provisional",
            ]
        }

        private static func series(
            _ source: String, _ station: [String: Any], observedAt: Int64,
            _ samples: [(Int64, Double)]
        ) -> [String: Any] {
            [
                "provenance": provenance(source, station, kind: "observed", observedAt: observedAt),
                "samples": samples.map { ["time": $0.0, "value": $0.1] as [String: Any] },
            ]
        }
    }
#endif
