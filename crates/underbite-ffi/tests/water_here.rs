//! The water at a dropped marker as the app drives it: plan, answer each round from recorded
//! 3DHP responses (`tests/fixtures/raw/hydro`), read. The Croton River at its gauge, the lower
//! Hudson and New York Harbor at the Battery, the New Croton Reservoir, and open land in
//! Manhattan.

#![allow(clippy::assert_is_empty)]

mod water_here {
    use std::path::Path;

    use underbite_core::time::parse_rfc3339;
    use underbite_ffi::fetch::HostResponse;
    use underbite_ffi::map::MapPoint;
    use underbite_ffi::nearby::{NearbyRequest, NearbyResponse};
    use underbite_ffi::settings::default_settings_json;
    use underbite_ffi::water_here::{
        WaterHerePlan, WaterHereResult, WaterHereStatus, plan_water_here, water_here_words,
    };

    fn now() -> i64 {
        parse_rfc3339("2026-11-19T15:00:00Z").expect("time").0
    }

    fn fixture(path: &str) -> Vec<u8> {
        let full = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/raw")
            .join(path);
        std::fs::read(&full).unwrap_or_else(|e| panic!("{}: {e}", full.display()))
    }

    fn response(status: u16, body: &[u8]) -> HostResponse {
        HostResponse {
            status,
            body: body.to_vec(),
            content_type: Some("application/json".into()),
            date: None,
            expires: None,
            last_modified: None,
            etag: None,
            retry_after: None,
            rate_limit_remaining: None,
            from_cache: false,
        }
    }

    /// The Croton River at its USGS gauge.
    const CROTON: MapPoint = MapPoint {
        lat: 41.225_027_777_777_8,
        lon: -73.8588,
    };
    /// In the New Croton Reservoir, off the dam.
    const RESERVOIR: MapPoint = MapPoint {
        lat: 41.23,
        lon: -73.84,
    };
    /// In the Hudson off Tribeca.
    const HUDSON: MapPoint = MapPoint {
        lat: 40.712,
        lon: -74.02,
    };
    /// In the Upper Bay off the Battery.
    const BATTERY: MapPoint = MapPoint {
        lat: 40.7,
        lon: -74.02,
    };
    /// Greenwich Village, nearly a kilometre from the Hudson.
    const LAND: MapPoint = MapPoint {
        lat: 40.73,
        lon: -73.99,
    };
    /// The tide station at the Battery.
    const TIDE: MapPoint = MapPoint {
        lat: 40.7006,
        lon: -74.0142,
    };

    fn plan(
        point: MapPoint,
        region: Option<&str>,
        tide: &[MapPoint],
    ) -> std::sync::Arc<WaterHerePlan> {
        plan_water_here(
            point,
            region.map(str::to_owned),
            tide.to_vec(),
            default_settings_json(),
            now(),
        )
        .expect("plan")
    }

    /// Answers each request of the round with the body `answer` picks, and reads them.
    fn round(
        plan: &WaterHerePlan,
        answer: impl Fn(&NearbyRequest) -> Option<HostResponse>,
    ) -> WaterHereResult {
        let responses = plan
            .requests()
            .iter()
            .filter_map(|r| {
                answer(r).map(|response| NearbyResponse {
                    id: r.id.clone(),
                    response,
                })
            })
            .collect();
        plan.read(responses, now())
    }

    /// Both 3DHP queries answered from the recorded files of one place; then, when the plan asks
    /// the wetland inventory, an answer with no polygons (the tide stations decide).
    fn hydro(plan: &WaterHerePlan, lines: &str, bodies: &str) -> WaterHereResult {
        let result = hydro_only(plan, lines, bodies);
        if asks_tide(plan) {
            return round(plan, |_| Some(response(200, EMPTY_WETLANDS)));
        }
        result
    }

    /// An answer of the wetland inventory with no polygons.
    const EMPTY_WETLANDS: &[u8] = b"{\"features\":[]}";

    /// The round holds the wetland inventory's request.
    fn asks_tide(plan: &WaterHerePlan) -> bool {
        let requests = plan.requests();
        !requests.is_empty() && requests.iter().all(|r| r.source == "usfws-nwi")
    }

    /// Both 3DHP queries answered from the recorded files of one place.
    fn hydro_only(plan: &WaterHerePlan, lines: &str, bodies: &str) -> WaterHereResult {
        let lines = fixture(lines);
        let bodies = fixture(bodies);
        round(plan, |r| {
            Some(response(
                200,
                if r.request.url.contains("/MapServer/50/") {
                    &lines
                } else {
                    &bodies
                },
            ))
        })
    }

    fn croton(plan: &WaterHerePlan) -> WaterHereResult {
        hydro(
            plan,
            "hydro/hydro-50-croton-geom.json",
            "hydro/hydro-60-croton-geom.json",
        )
    }

    fn battery(plan: &WaterHerePlan) -> WaterHereResult {
        hydro(
            plan,
            "hydro/hydro-50-battery-geom.json",
            "hydro/hydro-60-battery-geom.json",
        )
    }

    /// The digits after the decimal point of every number in the `geometry` parameter.
    fn decimals(url: &str) -> Vec<usize> {
        let geometry = url
            .split(['?', '&'])
            .find_map(|p| p.strip_prefix("geometry="))
            .expect("a geometry");
        geometry
            .split(',')
            .map(|n| n.split_once('.').map_or(0, |(_, d)| d.len()))
            .collect()
    }

    #[test]
    fn the_requests_carry_the_point_rounded_outward_to_two_decimals() {
        let plan = plan(CROTON, Some("US-NY"), &[]);
        let requests = plan.requests();
        assert_eq!(requests.len(), 2, "one flowline and one waterbody query");
        let lines = requests
            .iter()
            .find(|r| r.request.url.contains("/MapServer/50/"))
            .expect("flowlines");
        let bodies = requests
            .iter()
            .find(|r| r.request.url.contains("/MapServer/60/"))
            .expect("waterbodies");
        // The box around the point: four numbers, none finer than a hundredth of a degree,
        // and it covers the point.
        assert!(
            decimals(&lines.request.url).iter().all(|d| *d <= 2),
            "{:?}",
            lines.request
        );
        let g = lines
            .request
            .url
            .split(['?', '&'])
            .find_map(|p| p.strip_prefix("geometry="))
            .unwrap();
        let n: Vec<f64> = g.split(',').map(|v| v.parse().unwrap()).collect();
        assert!(n[0] < CROTON.lon && CROTON.lon < n[2], "{g}");
        assert!(n[1] < CROTON.lat && CROTON.lat < n[3], "{g}");
        // The waterbody query is a point rounded to the same grid, with a distance.
        let centre = bodies
            .request
            .url
            .split(['?', '&'])
            .find_map(|p| p.strip_prefix("geometry="))
            .unwrap();
        assert_eq!(centre, "-73.86,41.23");
        // Neither URL holds the tapped digits.
        for r in &requests {
            let url = &r.request.url;
            assert!(
                !url.contains("73.8588") && !url.contains("41.22502"),
                "{url}"
            );
            assert!(r.request.headers.is_empty());
            assert_eq!(r.request.max_age_s, Some(30 * 24 * 3600));
        }
    }

    #[test]
    fn the_croton_river_at_the_gauge_is_a_river_or_stream_with_its_regulation_links() {
        let plan = plan(CROTON, Some("US-NY"), &[]);
        assert_eq!(plan.result().status, WaterHereStatus::Looking);
        let result = croton(&plan);
        assert_eq!(result.status, WaterHereStatus::Found, "{result:?}");
        let card = result.card.expect("a card");
        assert_eq!(card.title, "Croton River");
        assert!(card.named);
        // The gauge sits in a mapped stretch of river: the water type is the app's river or
        // stream.
        assert_eq!(card.kind, "river");
        assert_eq!(card.kind_text, "River");
        assert_eq!(card.water_type, "river_stream");
        // The kind says it already: no second line.
        assert_eq!(card.water_type_text, "");
        assert_eq!(card.spot_name.as_deref(), Some("Croton River"));
        let titles: Vec<&str> = card.links.iter().map(|l| l.title.as_str()).collect();
        assert!(titles.contains(&"Freshwater regulations"), "{titles:?}");
        assert!(titles.contains(&"Fishing licence"), "{titles:?}");
        assert!(card.links.iter().all(|l| l.url.starts_with("https://")));
        assert!(card.disclaimer.is_some());
        // The saved region already covers the point: the state lines played no part.
        assert!(
            !card
                .credits
                .iter()
                .any(|c| c.text.starts_with("State lines"))
        );
        assert!(card.credits.iter().any(|c| c.name == "USGS 3DHP"
            && c.text == "U.S. Geological Survey, 3D National Hydrographic Program"));
    }

    #[test]
    fn a_brook_away_from_any_waterbody_is_a_stream() {
        let hunter = MapPoint {
            lat: 41.278_67,
            lon: -73.835_51,
        };
        let plan = plan(hunter, Some("US-NY"), &[]);
        let result = hydro(
            &plan,
            "hydro/hydro-50-hunter-brook-geom.json",
            "hydro/hydro-60-hunter-brook-geom.json",
        );
        let card = result.card.expect("a card");
        assert_eq!(card.title, "Hunter Brook");
        assert_eq!(
            (card.kind.as_str(), card.kind_text.as_str()),
            ("stream", "Stream")
        );
        assert_eq!(card.position, "The marker is on this water");
        assert_eq!(card.water_type, "river_stream");
    }

    #[test]
    fn a_stocked_water_of_new_york_asks_the_lists_and_shows_their_lines() {
        let plan = plan(CROTON, Some("US-NY"), &[]);
        let found = croton(&plan);
        assert!(found.card.expect("a card").stocking.is_empty());
        // The catalogue's Croton River, asked for by county and name only.
        let requests = plan.requests();
        assert_eq!(requests.len(), 2, "{requests:?}");
        for r in &requests {
            assert_eq!(r.source, "ny-dec-stocking");
            assert!(r.request.url.starts_with("https://data.ny.gov/resource/"));
            assert!(!r.request.url.contains("41.") && !r.request.url.contains("73."));
        }
        let actual =
            fixture("stocking-ny/socrata-ny-e52k-ymww-westchester-croton-river-2026-11-18.json");
        let planned =
            fixture("stocking-ny/socrata-ny-d9y2-n436-westchester-croton-river-2026-11-18.json");
        let result = round(&plan, |r| {
            Some(response(
                200,
                if r.request.url.contains("e52k-ymww") {
                    &actual
                } else {
                    &planned
                },
            ))
        });
        let card = result.card.expect("a card");
        assert_eq!(card.stocking_title.as_deref(), Some("Stocking"));
        assert!(
            card.stocking
                .first()
                .is_some_and(|l| l.starts_with("Rainbow Trout · Mar 2025")),
            "{:?}",
            card.stocking
        );
        assert!(
            card.credits.iter().any(|c| c.name.contains("NY")),
            "{:?}",
            card.credits
        );
        assert!(plan.requests().is_empty());
    }

    #[test]
    fn without_a_saved_region_the_point_names_its_state() {
        let plan = plan(CROTON, None, &[]);
        let card = croton(&plan).card.expect("a card");
        let titles: Vec<&str> = card.links.iter().map(|l| l.title.as_str()).collect();
        assert!(titles.contains(&"Freshwater regulations"), "{titles:?}");
        assert!(titles.contains(&"Fishing licence"), "{titles:?}");
        assert!(
            card.links
                .iter()
                .all(|l| l.detail.starts_with("New York State")),
            "{:?}",
            card.links
        );
        assert_eq!(card.links_note, None, "one state, nothing to say");
        assert!(
            card.credits
                .iter()
                .any(|c| c.text == "State lines: U.S. Census Bureau, cartographic boundary files"),
            "{:?}",
            card.credits
        );
        // New York stocking is asked for, by county and name, with no point in the request.
        let requests = plan.requests();
        assert_eq!(requests.len(), 2, "{requests:?}");
        assert!(requests.iter().all(|r| r.source == "ny-dec-stocking"));
        let actual =
            fixture("stocking-ny/socrata-ny-e52k-ymww-westchester-croton-river-2026-11-18.json");
        let planned =
            fixture("stocking-ny/socrata-ny-d9y2-n436-westchester-croton-river-2026-11-18.json");
        let card = round(&plan, |r| {
            Some(response(
                200,
                if r.request.url.contains("e52k-ymww") {
                    &actual
                } else {
                    &planned
                },
            ))
        })
        .card
        .expect("a card");
        assert!(
            card.stocking
                .first()
                .is_some_and(|l| l.starts_with("Rainbow Trout")),
            "{:?}",
            card.stocking
        );
    }

    #[test]
    fn a_saved_region_keeps_its_links_first_and_the_points_state_is_added() {
        let plan = plan(CROTON, Some("US-NJ"), &[]);
        let card = croton(&plan).card.expect("a card");
        assert_eq!(
            card.links_note.as_deref(),
            Some("On the New Jersey–New York line: both states' rules are linked")
        );
        let details: Vec<&str> = card.links.iter().map(|l| l.detail.as_str()).collect();
        assert!(details.first().is_some_and(|d| d.starts_with("New Jersey")));
        assert!(details.iter().any(|d| d.starts_with("New York State")));
        // New York is among the regions, so its stocking lists are asked for.
        assert_eq!(plan.requests().len(), 2);
    }

    #[test]
    fn the_lower_river_names_both_states_on_its_line() {
        // The water off the Battery, a few hundred metres from the New Jersey side.
        let at = MapPoint {
            lat: 40.7033,
            lon: -74.0170,
        };
        let plan = plan(at, None, &[TIDE]);
        let result = battery(&plan);
        let card = result.card.expect("a card");
        assert_eq!(
            card.links_note.as_deref(),
            Some("On the New York–New Jersey line: both states' rules are linked")
        );
        let details: Vec<&str> = card.links.iter().map(|l| l.detail.as_str()).collect();
        assert!(
            details.iter().any(|d| d.starts_with("New York State")),
            "{details:?}"
        );
        assert!(
            details.iter().any(|d| d.starts_with("New Jersey")),
            "{details:?}"
        );
    }

    #[test]
    fn a_failed_stocking_list_leaves_the_water_without_lines() {
        let plan = plan(CROTON, Some("US-NY"), &[]);
        croton(&plan);
        let result = round(&plan, |_| Some(response(500, b"")));
        let card = result.card.expect("a card");
        assert!(card.stocking.is_empty());
    }

    #[test]
    fn the_new_croton_reservoir_is_a_lake_the_marker_is_in() {
        let plan = plan(RESERVOIR, Some("US-NY"), &[]);
        let card = croton(&plan).card.expect("a card");
        assert_eq!(card.title, "New Croton Reservoir");
        assert_eq!(card.kind, "lake");
        assert_eq!(card.position, "The marker is on this water");
        assert_eq!(card.water_type, "lake_reservoir");
    }

    #[test]
    fn the_lower_hudson_is_a_tidal_river_with_a_tide_station_on_it() {
        let plan = plan(HUDSON, Some("US-NY"), &[TIDE]);
        let result = battery(&plan);
        let card = result.card.expect("a card");
        assert_eq!(card.kind, "tidal_river", "{card:?}");
        assert_eq!(card.kind_text, "Tidal river");
        assert_eq!(card.water_type, "tidal_river_estuary");
        let titles: Vec<&str> = card.links.iter().map(|l| l.title.as_str()).collect();
        assert!(titles.contains(&"Saltwater regulations"), "{titles:?}");
        assert!(!titles.contains(&"Freshwater regulations"), "{titles:?}");
        // Without a tide station the same water is a river.
        let plan = self::plan(HUDSON, Some("US-NY"), &[]);
        let card = battery(&plan).card.expect("a card");
        assert_eq!(card.kind, "river");
    }

    #[test]
    fn the_upper_bay_at_the_battery_is_a_bay_or_harbor() {
        let plan = plan(BATTERY, Some("US-NY"), &[TIDE]);
        let card = battery(&plan).card.expect("a card");
        assert_eq!(card.title, "Upper Bay");
        assert_eq!(card.kind_text, "Bay or harbor");
        assert_eq!(card.water_type, "bay_inshore");
        assert_eq!(card.position, "The marker is on this water");
    }

    /// The Hudson at Poughkeepsie.
    const POUGHKEEPSIE: MapPoint = MapPoint {
        lat: 41.7003,
        lon: -73.9399,
    };

    fn poughkeepsie(plan: &WaterHerePlan) -> WaterHereResult {
        hydro_only(
            plan,
            "hydro/hydro-50-poughkeepsie-geom.json",
            "hydro/hydro-60-poughkeepsie-geom.json",
        )
    }

    /// Answers the wetland inventory's one request with `status` and `body`.
    fn tide_round(plan: &WaterHerePlan, status: u16, body: &[u8]) -> WaterHereResult {
        assert!(asks_tide(plan), "{:?}", plan.requests());
        round(plan, |_| Some(response(status, body)))
    }

    #[test]
    fn the_inventory_makes_the_hudson_at_poughkeepsie_a_tidal_river_without_any_tide_station() {
        let plan = plan(POUGHKEEPSIE, Some("US-NY"), &[]);
        // The card waits for the inventory, which is the one request of the next round.
        assert_eq!(poughkeepsie(&plan).status, WaterHereStatus::Looking);
        let requests = plan.requests();
        assert_eq!(requests.len(), 1, "{requests:?}");
        let url = &requests[0].request.url;
        assert_eq!(requests[0].source, "usfws-nwi");
        assert!(
            url.contains("/Wetlands/MapServer/0/query?where=1=1&"),
            "{url}"
        );
        assert!(url.contains("geometry=-73.95,41.69,-73.93,41.71&"), "{url}");
        assert!(url.contains("returnGeometry=false"), "{url}");
        assert!(
            !url.contains("73.9399") && !url.contains("41.7003"),
            "{url}"
        );
        assert_eq!(decimals(url), [2, 2, 2, 2]);
        let result = tide_round(
            &plan,
            200,
            &fixture("tidal-limit/nwi-0-poughkeepsie-box.json"),
        );
        assert_eq!(result.status, WaterHereStatus::Found, "{result:?}");
        let card = result.card.expect("a card");
        assert_eq!(card.kind, "tidal_river", "{card:?}");
        assert_eq!(card.kind_text, "Tidal river");
        assert_eq!(card.water_type, "tidal_river_estuary");
        assert!(card.credits.iter().any(|c| c.name
            == "USFWS National Wetlands Inventory"
            && c.text
                == "Tidal reach: U.S. Fish and Wildlife Service, National Wetlands Inventory"
            && c.licence.as_deref() == Some("U.S. Government data, no licence stated")));
        let titles: Vec<&str> = card.links.iter().map(|l| l.title.as_str()).collect();
        assert!(titles.contains(&"Saltwater regulations"), "{titles:?}");
        assert!(
            plan.requests().is_empty() || plan.requests().iter().all(|r| r.source != "usfws-nwi")
        );
    }

    #[test]
    fn estuarine_polygons_alone_make_the_water_an_estuary() {
        let plan = plan(POUGHKEEPSIE, Some("US-NY"), &[]);
        poughkeepsie(&plan);
        let result = tide_round(&plan, 200, &fixture("tidal-limit/nwi-0-battery-box.json"));
        let card = result.card.expect("a card");
        assert_eq!(card.kind, "estuary", "{card:?}");
        assert_eq!(card.water_type, "tidal_river_estuary");
    }

    #[test]
    fn a_river_the_inventory_calls_not_tidal_stays_a_river_even_beside_a_tide_station() {
        let station = MapPoint {
            lat: 41.7005,
            lon: -73.9401,
        };
        let plan = plan(POUGHKEEPSIE, Some("US-NY"), &[station]);
        poughkeepsie(&plan);
        let result = tide_round(&plan, 200, &fixture("tidal-limit/nwi-0-croton-box.json"));
        let card = result.card.expect("a card");
        assert_eq!(card.kind, "river", "{card:?}");
        assert_eq!(card.water_type, "river_stream");
        assert!(
            card.credits
                .iter()
                .any(|c| c.name == "USFWS National Wetlands Inventory")
        );
    }

    #[test]
    fn an_empty_answer_or_a_firewall_page_falls_back_on_the_tide_stations_and_is_not_credited() {
        let station = MapPoint {
            lat: 41.7005,
            lon: -73.9401,
        };
        let html: &[u8] = b"<html><body>The URL you requested has been blocked</body></html>";
        for (status, body) in [(200, EMPTY_WETLANDS), (200, html), (403, html), (500, b"")] {
            let with = plan(POUGHKEEPSIE, Some("US-NY"), &[station]);
            poughkeepsie(&with);
            let card = tide_round(&with, status, body).card.expect("a card");
            assert_eq!(card.kind, "tidal_river", "{status}");
            assert!(
                !card.credits.iter().any(|c| c.name.contains("Wetlands")),
                "{status}: {:?}",
                card.credits
            );
            let without = plan(POUGHKEEPSIE, Some("US-NY"), &[]);
            poughkeepsie(&without);
            let card = tide_round(&without, status, body).card.expect("a card");
            assert_eq!(card.kind, "river", "{status}");
        }
        // No answer at all counts as failed too.
        let plan = plan(POUGHKEEPSIE, Some("US-NY"), &[station]);
        poughkeepsie(&plan);
        let result = plan.read(Vec::new(), now());
        assert_eq!(result.card.expect("a card").kind, "tidal_river");
    }

    #[test]
    fn a_lake_or_land_sends_nothing_to_the_inventory() {
        let plan = plan(RESERVOIR, Some("US-NY"), &[]);
        let result = hydro_only(
            &plan,
            "hydro/hydro-50-croton-geom.json",
            "hydro/hydro-60-croton-geom.json",
        );
        assert_eq!(result.status, WaterHereStatus::Found);
        assert!(plan.requests().iter().all(|r| r.source != "usfws-nwi"));
        let plan = plan_land();
        let result = hydro_only(
            &plan,
            "hydro/hydro-50-battery-geom.json",
            "hydro/hydro-60-battery-geom.json",
        );
        assert_eq!(result.status, WaterHereStatus::Nothing);
        assert!(plan.requests().is_empty());
    }

    fn plan_land() -> std::sync::Arc<WaterHerePlan> {
        plan(LAND, Some("US-NY"), &[TIDE])
    }

    #[test]
    fn open_land_has_no_water_and_no_section() {
        let plan = plan(LAND, Some("US-NY"), &[TIDE]);
        let result = battery(&plan);
        assert_eq!(result.status, WaterHereStatus::Nothing);
        assert!(result.card.is_none());
        assert!(plan.requests().is_empty());
    }

    #[test]
    fn a_failed_answer_says_so_in_the_core_words() {
        let plan = plan(CROTON, Some("US-NY"), &[]);
        let result = round(&plan, |r| {
            r.request
                .url
                .contains("/MapServer/50/")
                .then(|| response(503, b"busy"))
        });
        assert_eq!(result.status, WaterHereStatus::Failed);
        assert_eq!(
            result.message.as_deref(),
            Some("USGS 3DHP could not be reached")
        );
        assert!(result.card.is_none());
        assert!(plan.requests().is_empty());
        let words = water_here_words(default_settings_json()).expect("words");
        assert_eq!(words.failed, "USGS 3DHP could not be reached");
        assert_eq!(words.title, "Water here");
        assert_eq!(words.loading, "Looking up the water here…");
        assert_eq!(words.add_as_spot, "Add as spot");
    }

    #[test]
    fn a_full_page_is_followed_once() {
        let plan = plan(CROTON, None, &[]);
        let mut page: serde_json::Value =
            serde_json::from_slice(&fixture("hydro/hydro-50-croton-geom.json")).expect("json");
        page["exceededTransferLimit"] = serde_json::Value::Bool(true);
        let page = serde_json::to_vec(&page).expect("json");
        let bodies = fixture("hydro/hydro-60-croton-geom.json");
        let first = round(&plan, |r| {
            Some(response(
                200,
                if r.request.url.contains("/MapServer/50/") {
                    &page
                } else {
                    &bodies
                },
            ))
        });
        assert_eq!(first.status, WaterHereStatus::Looking);
        let more = plan.requests();
        assert_eq!(more.len(), 1, "{more:?}");
        assert!(more[0].request.url.contains("/MapServer/50/"));
        assert!(
            more[0].request.url.contains("&resultOffset=2500"),
            "{:?}",
            more[0].request
        );
        // That page is full too, but only one more is read.
        let second = round(&plan, |_| Some(response(200, &page)));
        // The inventory is asked once the water is placed; the card waits for it.
        assert_eq!(second.status, WaterHereStatus::Looking);
        let third = round(&plan, |_| Some(response(200, EMPTY_WETLANDS)));
        assert_eq!(third.status, WaterHereStatus::Found);
        // Only the stocking lists of the stocked Croton River follow.
        assert!(
            plan.requests()
                .iter()
                .all(|r| r.source == "ny-dec-stocking")
        );
    }

    #[test]
    fn a_point_out_of_range_is_refused() {
        let at = MapPoint {
            lat: 91.0,
            lon: 0.0,
        };
        assert!(plan_water_here(at, None, Vec::new(), default_settings_json(), now()).is_err());
    }
}
