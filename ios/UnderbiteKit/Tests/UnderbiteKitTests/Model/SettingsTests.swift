import Foundation
import XCTest

@testable import UnderbiteKit

/// The settings types against the core: decoding and encoding again drops nothing, and the
/// core accepts what Swift writes.
final class SettingsTests: XCTestCase {
    func testTheSampleRoundTripsThroughTheCore() throws {
        let original = try Fixtures.text("sample-settings")
        let settings = try Settings(json: original)
        XCTAssertEqual(settings.version, 1)
        XCTAssertEqual(settings.limits.map(\.name), ["Kayak", "20 ft boat", "Wading"])
        XCTAssertNil(settings.limits[1].maxCurrentKn)
        XCTAssertEqual(settings.units.height, "ft")
        XCTAssertNil(settings.units.airTemp)
        XCTAssertEqual(settings.sources.waterTemperature.maxAgeMinutes, 180)
        XCTAssertEqual(settings.sources.currents.chain, ["coops", "hf-radar", "copernicus", "open-meteo-marine"])
        XCTAssertEqual(settings.presentation.clock, "12h")
        XCTAssertEqual(settings.cards.order.first, "now")
        XCTAssertNil(settings.cards.river)

        let encoded = try settings.json()
        try Fixtures.assertSameJSON(encoded, original)
        let migrated = try migrateSettings(json: encoded)
        try Fixtures.assertSameJSON(migrated.json, original)
        XCTAssertTrue(migrated.issues.isEmpty)
    }

    func testTheCoreDefaultsRoundTrip() throws {
        let defaults = try defaultSettingsJson()
        let settings = try Settings.defaults()
        try Fixtures.assertSameJSON(settings.json(), defaults)
        try Fixtures.assertSameJSON(migrateSettings(json: settings.json()).json, defaults)
    }

    func testEveryFieldIsCovered() throws {
        // Fill the optional fields the defaults leave out; nothing may be lost on the way.
        var doc = try XCTUnwrap(Fixtures.object(Fixtures.text("sample-settings")) as? [String: Any])
        var units = try XCTUnwrap(doc["units"] as? [String: Any])
        for (key, value) in [
            ("air_temp", "C"), ("stage", "m"), ("lake_level", "ft"), ("tide", "m"),
            ("waves", "m"), ("current", "ms"), ("wind", "mph"),
        ] { units[key] = value }
        doc["units"] = units
        var sources = try XCTUnwrap(doc["sources"] as? [String: Any])
        sources["tide"] = ["chain": ["coops"], "max_distance_km": 40.5, "max_age_minutes": 90]
        doc["sources"] = sources
        var cards = try XCTUnwrap(doc["cards"] as? [String: Any])
        cards["hidden"] = ["sources"]
        for key in [
            "river", "lake", "tidal", "bay", "surf", "nearshore", "offshore", "great_lakes",
            "pond", "ice",
        ] { cards[key] = ["order": ["tide", "now"], "hidden": ["alerts"]] }
        doc["cards"] = cards
        doc["limits"] = [
            [
                "name": "All", "max_wind_kn": 10.5, "max_gust_kn": 15.0, "max_wave_m": 0.5,
                "max_current_kn": 1.0, "min_air_temp_c": -2.0, "min_visibility_km": 3.0,
                "no_small_craft_advisory": false, "no_lightning": true,
            ]
        ]
        doc["rivers"] = [
            [
                "site": "usgs:01375000", "wadeable_below_cfs": 100.0, "best_cfs": [150.0, 300.0],
                "blown_out_above_cfs": 900.0,
            ]
        ]
        doc["providers"] = ["usgs": [String: Any](), "copernicus": ["enabled": true]]
        // Every catalogue layer stays in the list, so the migration has nothing to add.
        var map = try XCTUnwrap(doc["map"] as? [String: Any])
        var layers = try XCTUnwrap(map["layers"] as? [[String: Any]])
        layers[0] = ["id": layers[0]["id"] as Any, "shown": true, "opacity": 0.6, "ramp": "thermal", "range": [10.0, 28.0]]
        map["base"] = "apple-hybrid"
        map["layers"] = layers
        map["presets"] = [
            ["name": "Mine", "layers": [["id": "sst", "shown": true, "opacity": 0.5]]]
        ]
        map["active_preset"] = "Mine"
        map["battery_mode"] = "on"
        map["depth"] = ["band_m": [4.5, 7.5], "contour_interval_m": 2.0, "ramp": "depth"]
        map["breaks"] = ["sst_c_per_km": 0.75, "chlorophyll_log10_per_km": 0.05]
        map["offline_max_mb"] = 800
        doc["map"] = map
        doc["storms"] = ["hurricane_distance_km": 1500.5]
        doc["thresholds"] = ["trout_stress_c": 18.5]
        doc["species"] = ["followed": ["salmo-trutta", "micropterus-dolomieu"]]
        doc["alerts"] = [
            [
                "id": "croton-low", "name": "Croton low", "enabled": false,
                "target": ["kind": "spot", "id": "croton"],
                "all_of": [
                    ["kind": "measurement", "metric": "discharge", "below": 22.5, "above": 1.5],
                    ["kind": "marine_alert", "event": "Gale Warning"],
                    ["kind": "window_starts", "within_minutes": 30],
                    ["kind": "storm_ahead"],
                ],
                "repeat_after_hours": 12,
                "quiet_hours": ["start_minute": 1320, "end_minute": 420],
                "show_name": false,
            ],
            [
                "id": "trip-go", "name": "Trip on", "enabled": true,
                "target": ["kind": "trip", "id": "trip-1"], "all_of": [["kind": "trip_go"]],
                "show_name": true,
            ],
            [
                "id": "warm-box", "name": "Warm water", "enabled": true,
                "target": ["kind": "box", "south": 39.5, "west": -72.5, "north": 40.5, "east": -71.5],
                "all_of": [["kind": "measurement", "metric": "sea_surface_temp", "above": 20.0]],
                "show_name": true,
            ],
        ]
        let full = try Fixtures.string(doc)

        // The core accepts the document as is, and Swift gives it back unchanged.
        let settings = try Settings(json: full)
        let encoded = try settings.json()
        try Fixtures.assertSameJSON(encoded, full)
        try Fixtures.assertSameJSON(migrateSettings(json: full).json, full)
        XCTAssertEqual(settings.cards.greatLakes?.hidden, ["alerts"])
        XCTAssertEqual(settings.rivers[0].bestCfs, [150, 300])
        XCTAssertTrue(settings.providers.copernicus.enabled)
        XCTAssertEqual(settings.map.layers[0].range, [10, 28])
        XCTAssertNil(settings.map.layers[1].opacity)
        XCTAssertEqual(settings.map.presets[0].layers[0].opacity, 0.5)
        XCTAssertEqual(settings.map.depth.bandM, [4.5, 7.5])
        XCTAssertEqual(settings.map.activePreset, "Mine")
        XCTAssertEqual(settings.alerts.map(\.target.kind), ["spot", "trip", "box"])
        XCTAssertEqual(settings.alerts[0].allOf[0].below, 22.5)
        XCTAssertEqual(settings.alerts[0].allOf[2].withinMinutes, 30)
        XCTAssertEqual(settings.alerts[0].quietHours?.startMinute, 1320)
        XCTAssertNil(settings.alerts[1].repeatAfterHours)
        XCTAssertEqual(settings.alerts[2].target.north, 40.5)
        XCTAssertEqual(settings.alerts.map(\.showName), [false, true, true])
        XCTAssertEqual(settings.species.followed, ["salmo-trutta", "micropterus-dolomieu"])
    }

    func testARuleWithoutShowNameNamesItsPlace() throws {
        var doc = try XCTUnwrap(Fixtures.object(Fixtures.text("sample-settings")) as? [String: Any])
        doc["alerts"] = [
            [
                "id": "gale", "name": "Gale", "enabled": true,
                "target": ["kind": "spot", "id": "battery"],
                "all_of": [["kind": "marine_alert", "event": "Gale Warning"]],
            ]
        ]
        let settings = try Settings(json: Fixtures.string(doc))
        XCTAssertEqual(settings.alerts.map(\.showName), [true])
        // The core writes it, and so does Swift.
        let core = try XCTUnwrap(
            Fixtures.object(migrateSettings(json: Fixtures.string(doc)).json) as? [String: Any])
        let rules = try XCTUnwrap(core["alerts"] as? [[String: Any]])
        XCTAssertEqual(rules[0]["show_name"] as? Bool, true)
        try Fixtures.assertSameJSON(settings.json(), migrateSettings(json: Fixtures.string(doc)).json)
    }

    func testSettingsWithoutAlertsDecode() throws {
        let original = try Fixtures.text("sample-settings")
        var doc = try XCTUnwrap(Fixtures.object(original) as? [String: Any])
        doc.removeValue(forKey: "alerts")
        let settings = try Settings(json: Fixtures.string(doc))
        XCTAssertEqual(settings.alerts, [])
        try Fixtures.assertSameJSON(settings.json(), original)
    }

    func testSettingsWithoutSpeciesDecode() throws {
        let original = try Fixtures.text("sample-settings")
        var doc = try XCTUnwrap(Fixtures.object(original) as? [String: Any])
        doc.removeValue(forKey: "species")
        let settings = try Settings(json: Fixtures.string(doc))
        XCTAssertEqual(settings.species.followed, [])
        try Fixtures.assertSameJSON(settings.json(), original)
    }

    func testFollowingASpeciesGoesThroughTheCore() throws {
        var settings = try Settings.defaults()
        XCTAssertEqual(settings.species.followed, [])
        settings.species.follow("micropterus-dolomieu")
        settings.species.follow("salmo-trutta")
        settings.species.follow("micropterus-dolomieu")
        XCTAssertEqual(settings.species.followed, ["micropterus-dolomieu", "salmo-trutta"])
        XCTAssertTrue(settings.species.follows("salmo-trutta"))

        let migrated = try migrateSettings(json: settings.json())
        XCTAssertTrue(migrated.issues.isEmpty)
        XCTAssertEqual(try Settings(json: migrated.json), settings)

        settings.species.unfollow("micropterus-dolomieu")
        XCTAssertEqual(settings.species.followed, ["salmo-trutta"])
        XCTAssertFalse(settings.species.follows("micropterus-dolomieu"))

        // An id the catalogue lacks is dropped by the core, with a warning naming it.
        settings.species.follow("no-such-fish")
        let tidied = try migrateSettings(json: settings.json())
        XCTAssertEqual(try Settings(json: tidied.json).species.followed, ["salmo-trutta"])
        XCTAssertEqual(tidied.issues.map(\.path), ["species.followed[1]"])
        XCTAssertEqual(tidied.issues.map(\.severity), [.warning])
    }

    func testAnEditGoesThroughTheCore() throws {
        var settings = try Settings.defaults()
        settings.units.height = "m"
        settings.presentation.voice = "plain"
        settings.cards.tidal = Settings.CardLayout(order: ["sources", "tide"])
        var extra = try Settings(json: Fixtures.text("sample-settings")).limits[0]
        extra.name = "Another"
        settings.limits.append(extra)
        let migrated = try migrateSettings(json: settings.json())
        XCTAssertTrue(migrated.issues.isEmpty)
        let back = try Settings(json: migrated.json)
        XCTAssertEqual(back.units.height, "m")
        XCTAssertEqual(back.presentation.voice, "plain")
        XCTAssertEqual(back.cards.tidal?.order, ["sources", "tide"])
        XCTAssertEqual(back.limits.count, settings.limits.count)
        XCTAssertEqual(back, settings)
    }
}
