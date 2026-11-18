//! Nearby lookups as the app drives them: plan, answer each round from recorded responses
//! (`tests/fixtures/raw`), read. New York Harbor for access and species records, Queens for
//! NAS, Westchester for stocking.

#![allow(clippy::assert_is_empty)]

// In a module of the same name, so `cargo test -p underbite-ffi nearby` selects these.
mod nearby {
    use std::path::Path;

    use underbite_core::time::parse_rfc3339;
    use underbite_ffi::fetch::{Header, HostResponse};
    use underbite_ffi::nearby::{
        AccessShape, NearbyArea, NearbyError, NearbyPlan, NearbyRequest, NearbyResponse,
        NearbyResults, RecordsQuery, nearby_limits, nearby_words, plan_access, plan_blooms,
        plan_records, plan_stocking,
    };
    use underbite_ffi::settings::default_settings_json;

    fn now() -> i64 {
        parse_rfc3339("2026-11-17T15:00:00Z").expect("time").0
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
        }
    }

    fn area(west: f64, south: f64, east: f64, north: f64) -> NearbyArea {
        NearbyArea {
            west,
            south,
            east,
            north,
        }
    }

    /// The Battery and the lower Hudson's mouth, inside four Overpass cells.
    fn battery() -> NearbyArea {
        area(-74.05, 40.68, -73.99, 40.74)
    }

    /// New York Harbor, as the species fixtures were asked for.
    fn harbor() -> NearbyArea {
        area(-74.4, 40.5, -73.7, 40.9)
    }

    fn settings() -> String {
        default_settings_json()
    }

    /// Answers every request of the round with `answer`, and reads them.
    fn round(
        plan: &NearbyPlan,
        at: i64,
        answer: impl Fn(&NearbyRequest) -> Option<HostResponse>,
    ) -> NearbyResults {
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
        plan.read(responses, at)
    }

    fn header<'a>(r: &'a NearbyRequest, name: &str) -> Option<&'a str> {
        r.request
            .headers
            .iter()
            .find(|h: &&Header| h.name.eq_ignore_ascii_case(name))
            .map(|h| h.value.as_str())
    }

    const EMPTY_OVERPASS: &[u8] = br#"{"version":0.6,"elements":[]}"#;
    const EMPTY_PADUS: &[u8] = br#"{"type":"FeatureCollection","features":[]}"#;

    // ---------------------------------------------------------------------------------------------
    // Access
    // ---------------------------------------------------------------------------------------------

    #[test]
    #[allow(clippy::too_many_lines)]
    fn access_asks_padus_and_overpass_then_answers_overpass_from_the_kept_state() {
        let plan = plan_access(battery(), false, String::new(), settings(), now()).expect("plan");
        let requests = plan.requests();
        let padus: Vec<_> = requests.iter().filter(|r| r.source == "padus").collect();
        let osm: Vec<_> = requests.iter().filter(|r| r.source == "overpass").collect();
        assert_eq!(padus.len(), 1);
        assert_eq!(osm.len(), 4, "four 0.1 degree cells, places only");
        // The box leaves rounded outward to at most 2 decimals; Overpass cells are tenths of a
        // degree.
        let url = &padus[0].request.url;
        let geometry = url
            .split('&')
            .find_map(|p| p.strip_prefix("geometry="))
            .expect("a box");
        let edges: Vec<f64> = geometry
            .split(',')
            .map(|e| {
                assert!(e.split('.').nth(1).is_none_or(|d| d.len() <= 2), "{e}");
                e.parse().expect("a number")
            })
            .collect();
        assert_eq!(edges.len(), 4);
        assert!(edges[0] <= -74.05 && edges[1] <= 40.68 && edges[2] >= -73.99 && edges[3] >= 40.74);
        for r in &osm {
            assert_eq!(r.queue, "overpass");
            assert_eq!(r.delay_ms, 0);
            assert!(header(r, "User-Agent").is_some_and(|ua| ua.starts_with("underbite/")));
            assert!(!r.request.url.contains("meta"));
            assert_eq!(r.request.max_age_s, Some(30 * 86_400));
        }
        let first_cell = osm[0].id.clone();

        let battery_osm = fixture("overpass/overpass-access-battery-2026-11-17.json");
        let croton_padus = fixture("padus/padus-croton-geojson-offset0.0005-5rows-2026-11-17.json");
        let results = round(&plan, now(), |r| {
            Some(match r.source.as_str() {
                "padus" => response(200, &croton_padus),
                _ if r.id == first_cell => response(200, &battery_osm),
                _ => response(200, EMPTY_OVERPASS),
            })
        });

        let land: Vec<_> = results
            .access
            .iter()
            .filter(|p| p.kind == "public_land")
            .collect();
        assert!(land.iter().any(|p| p.title == "Cortlandt Park"
            && p.access == "open"
            && p.access_label == "Open to the public, as the source records it"
            && p.source_name == "USGS PAD-US"
            && matches!(&p.shape, AccessShape::Polygon { rings } if !rings.is_empty())));
        let ramps: Vec<_> = results.access.iter().filter(|p| p.kind == "ramp").collect();
        assert!(!ramps.is_empty());
        assert!(ramps.iter().all(|p| p.kind_label == "Boat ramp"
            && p.source == "overpass"
            && p.shape == AccessShape::Point));
        assert!(results.access.iter().all(|p| !p.accessibility.is_empty()));
        let osm_credit = results
            .credits
            .iter()
            .find(|c| c.source == "overpass")
            .expect("OpenStreetMap credit");
        assert_eq!(osm_credit.text, "\u{a9} OpenStreetMap contributors");
        assert_eq!(osm_credit.licence.as_deref(), Some("ODbL 1.0"));
        assert!(
            results
                .credits
                .iter()
                .any(|c| c.source == "padus" && c.text.contains("PAD-US"))
        );
        assert!(
            results
                .notes
                .iter()
                .any(|n| n.contains("not as permission"))
        );
        // The full PAD-US page asks for the next one.
        let next = plan.requests();
        assert_eq!(next.len(), 1);
        assert!(next[0].request.url.contains("resultOffset=500"));
        let results = round(&plan, now(), |_| Some(response(200, EMPTY_PADUS)));
        assert!(plan.requests().is_empty());
        let state = results.state_json.expect("access keeps a state");

        // An hour later the cells come from the state: only PAD-US is asked again.
        let later = plan_access(battery(), false, state, settings(), now() + 3_600).expect("plan");
        let requests = later.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].source, "padus");
        let cached = later.results();
        assert_eq!(
            cached.access.iter().filter(|p| p.kind == "ramp").count(),
            ramps.len()
        );
        assert!(
            cached
                .notices
                .iter()
                .any(|n| n == "OpenStreetMap places saved on this phone 1 h ago"),
            "{:?}",
            cached.notices
        );
        // Cells that were just fetched are not marked.
        assert!(
            !results
                .notices
                .iter()
                .any(|n| n.contains("saved on this phone")),
            "{:?}",
            results.notices
        );
    }

    #[test]
    fn the_daily_overpass_cap_holds_across_plans() {
        let mut state = String::new();
        for _ in 0..3 {
            let plan = plan_access(battery(), false, state, settings(), now()).expect("plan");
            assert_eq!(
                plan.requests()
                    .iter()
                    .filter(|r| r.source == "overpass")
                    .count(),
                4
            );
            state = plan.results().state_json.expect("state");
        }
        let plan = plan_access(battery(), false, state.clone(), settings(), now()).expect("plan");
        assert!(plan.requests().iter().all(|r| r.source != "overpass"));
        assert!(
        plan.results()
            .notices
            .iter()
            .any(|n| n.starts_with("OpenStreetMap: this phone's lookups for today are used up"))
    );
        // A new UTC day gives the count back.
        let tomorrow =
            plan_access(battery(), false, state, settings(), now() + 86_400).expect("plan");
        assert!(tomorrow.requests().iter().any(|r| r.source == "overpass"));
    }

    #[test]
    fn a_busy_overpass_is_retried_after_a_pause_then_given_up() {
        // Inside one cell, with parking: two Overpass requests.
        let one_cell = area(-74.03, 40.71, -74.01, 40.72);
        let plan = plan_access(one_cell, true, String::new(), settings(), now()).expect("plan");
        let busy = b"This Overpass API server is currently at capacity.";
        let answer = |r: &NearbyRequest| {
            Some(if r.source == "overpass" {
                HostResponse {
                    content_type: Some("text/plain".into()),
                    ..response(502, busy)
                }
            } else {
                response(200, EMPTY_PADUS)
            })
        };
        assert_eq!(
            plan.requests()
                .iter()
                .filter(|r| r.source == "overpass")
                .count(),
            2
        );
        let first = round(&plan, now(), answer);
        let retries = plan.requests();
        assert!(!retries.is_empty());
        assert!(retries.iter().all(|r| r.source == "overpass"));
        assert!(retries.iter().all(|r| r.delay_ms >= 30_000));
        assert!(first.issues.iter().any(|i| i.source == "overpass"));
        // Three failures in a row and the gate stops.
        let mut last = first;
        for _ in 0..3 {
            if plan.requests().is_empty() {
                break;
            }
            last = round(&plan, now() + 60, answer);
        }
        assert!(plan.requests().is_empty());
        assert!(
            last.notices
                .iter()
                .any(|n| n == "OpenStreetMap is busy; try again later"),
            "{:?}",
            last.notices
        );
        // The pause is kept: a new tap right away is refused with the wait.
        let state = last.state_json.expect("state");
        let again = plan_access(one_cell, false, state, settings(), now() + 61).expect("plan");
        assert!(again.requests().iter().all(|r| r.source != "overpass"));
        assert!(
            again
                .results()
                .notices
                .iter()
                .any(|n| n.starts_with("OpenStreetMap asked to slow down; try again in"))
        );
    }

    #[test]
    fn wide_views_are_refused_with_a_zoom_in_message() {
        let wide = area(-74.5, 40.4, -73.5, 41.4);
        assert_eq!(
            plan_access(wide, false, String::new(), settings(), now()).unwrap_err(),
            NearbyError::ZoomIn {
                message: "Zoom in to look for ramps, piers and public land".into()
            }
        );
        let query = RecordsQuery {
            scientific_name: "Morone saxatilis".into(),
            page: 1,
            sources: Vec::new(),
            nas_state: None,
            nas_huc8: None,
        };
        let state_wide = area(-79.8, 40.5, -71.8, 45.0);
        assert!(matches!(
            plan_records(state_wide, query, String::new(), settings(), now()),
            Err(NearbyError::ZoomIn { message }) if message == "Zoom in to look for species records"
        ));
        assert!(matches!(
            plan_stocking(state_wide, settings(), now()),
            Err(NearbyError::ZoomIn { message }) if message == "Zoom in to look for stocked waters"
        ));
        // Across the antimeridian a whole ocean is as wide as it looks.
        assert!(matches!(
            plan_stocking(area(170.0, 50.0, -170.0, 52.0), settings(), now()),
            Err(NearbyError::ZoomIn { .. })
        ));
        // Public land fits a 0.4 degree view, OpenStreetMap's cells do not.
        let mid = area(-74.2, 40.5, -73.8, 40.9);
        let plan = plan_access(mid, false, String::new(), settings(), now()).expect("plan");
        assert!(plan.requests().iter().all(|r| r.source == "padus"));
        assert!(
            plan.results().notices.contains(
                &"Zoom in to look for ramps, piers and parking from OpenStreetMap".into()
            )
        );
    }

    #[test]
    fn bad_input_is_refused_rather_than_guessed() {
        let nan = area(f64::NAN, 40.0, -74.0, 41.0);
        assert!(matches!(
            plan_access(nan, false, String::new(), settings(), now()),
            Err(NearbyError::Invalid { .. })
        ));
        let flipped = area(-74.1, 40.8, -74.0, 40.7);
        assert!(matches!(
            plan_stocking(flipped, settings(), now()),
            Err(NearbyError::Invalid { .. })
        ));
        assert!(matches!(
            plan_access(battery(), false, "{not json".into(), settings(), now()),
            Err(NearbyError::Invalid { .. })
        ));
        let query = |name: &str, sources: &[&str], state: Option<&str>| RecordsQuery {
            scientific_name: name.into(),
            page: 1,
            sources: sources.iter().map(|s| (*s).to_owned()).collect(),
            nas_state: state.map(str::to_owned),
            nas_huc8: None,
        };
        for q in [
            query("  ", &[], None),
            query("Morone saxatilis", &["ebird"], None),
            query("Channa argus", &["usgs-nas"], Some("New York")),
        ] {
            assert!(matches!(
                plan_records(battery(), q, String::new(), settings(), now()),
                Err(NearbyError::Invalid { .. })
            ));
        }
    }

    // ---------------------------------------------------------------------------------------------
    // Species records
    // ---------------------------------------------------------------------------------------------

    fn striped_bass(sources: &[&str]) -> RecordsQuery {
        RecordsQuery {
            scientific_name: "Morone saxatilis".into(),
            page: 1,
            sources: sources.iter().map(|s| (*s).to_owned()).collect(),
            nas_state: None,
            nas_huc8: None,
        }
    }

    fn species_answer(r: &NearbyRequest) -> Option<HostResponse> {
        Some(match r.source.as_str() {
            "inaturalist" => response(
                200,
                &fixture("tier2/inaturalist-observations-striped-bass-nyharbor.synthetic.json"),
            ),
            "gbif" => response(
                200,
                &fixture("tier2/gbif-occurrence-striped-bass-nyharbor.synthetic.json"),
            ),
            _ => return None,
        })
    }

    #[test]
    fn records_carry_their_licence_page_and_credit_and_name_no_observer() {
        let plan = plan_records(
            harbor(),
            striped_bass(&[]),
            String::new(),
            settings(),
            now(),
        )
        .expect("plan");
        let requests = plan.requests();
        assert_eq!(requests.len(), 2, "no NAS without a state");
        let inat = requests
            .iter()
            .find(|r| r.source == "inaturalist")
            .expect("iNaturalist");
        let gbif = requests.iter().find(|r| r.source == "gbif").expect("GBIF");
        assert_eq!(inat.request.max_age_s, Some(300));
        assert_eq!(gbif.request.max_age_s, Some(600));
        assert!(inat.request.url.contains("swlng=-74.40"));
        assert!(gbif.request.url.contains("decimalLongitude=-74.40,-73.70"));
        assert!(header(inat, "User-Agent").is_some_and(|ua| ua.starts_with("underbite/")));

        let results = round(&plan, now(), species_answer);
        assert_eq!(results.records.len(), 6);
        assert!(results.more, "GBIF has more pages");
        for pin in &results.records {
            assert_eq!(pin.licence, "CC0");
            assert!(
                pin.link
                    .as_deref()
                    .is_some_and(|l| l.starts_with("https://"))
            );
            assert!(!pin.credit.is_empty() && pin.credit_url.is_some());
            let link_title = match pin.source.as_str() {
                "inaturalist" => "Record on iNaturalist",
                _ => "Record on GBIF",
            };
            assert_eq!(pin.link_title.as_deref(), Some(link_title));
            let text = format!("{pin:?}").to_lowercase();
            for personal in ["user", "login", "recordedby", "rightsholder", "observer"] {
                assert!(!text.contains(personal), "{personal} in {text}");
            }
        }
        let blurred = results
            .records
            .iter()
            .find(|p| p.id == "inaturalist:9100002")
            .expect("the obscured observation");
        assert!(blurred.obscured);
        assert_eq!(blurred.title, "Striped Bass");
        assert_eq!(blurred.seen, "Seen Oct 30 2026");
        assert_eq!(
            blurred.position.as_deref(),
            Some("Position blurred by the source, within 17 mi")
        );
        let photo = blurred.photo.as_ref().expect("an open-licence photo");
        assert_eq!(photo.caption, "Photo: CC0");
        let names: Vec<_> = results.credits.iter().map(|c| c.text.as_str()).collect();
        assert!(names.contains(&"iNaturalist") && names.contains(&"GBIF.org"));
        assert!(
            results.notes.contains(
                &"Records show where a species was reported, not where it is now.".into()
            )
        );

        // A next tap right away waits iNaturalist's interval.
        let state = results.state_json.expect("state");
        let next = RecordsQuery {
            page: 2,
            ..striped_bass(&["inaturalist"])
        };
        let again = plan_records(harbor(), next, state, settings(), now()).expect("plan");
        let r = again.requests();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].delay_ms, nearby_limits().inaturalist_min_interval_ms);
        assert!(r[0].request.url.contains("page=2"));
    }

    #[test]
    fn a_rate_limited_source_waits_out_its_pause() {
        let plan = plan_records(
            harbor(),
            striped_bass(&[]),
            String::new(),
            settings(),
            now(),
        )
        .expect("plan");
        let results = round(&plan, now(), |r| {
            if r.source == "inaturalist" {
                Some(HostResponse {
                    retry_after: Some("120".into()),
                    ..response(429, b"{}")
                })
            } else {
                species_answer(r)
            }
        });
        assert!(
            results
                .notices
                .contains(&"iNaturalist asked to slow down; try again in 2 min".into()),
            "{:?}",
            results.notices
        );
        assert!(results.issues.iter().any(|i| i.kind == "rate_limited"));
        let state = results.state_json.expect("state");
        let soon = plan_records(
            harbor(),
            striped_bass(&[]),
            state.clone(),
            settings(),
            now() + 10,
        )
        .expect("plan");
        let sources: Vec<_> = soon.requests().into_iter().map(|r| r.source).collect();
        assert_eq!(sources, ["gbif"]);
        let later = plan_records(harbor(), striped_bass(&[]), state, settings(), now() + 200)
            .expect("plan");
        assert!(later.requests().iter().any(|r| r.source == "inaturalist"));
    }

    #[test]
    fn nas_records_are_read_by_state_and_kept_to_the_view() {
        let query = RecordsQuery {
            scientific_name: "Channa argus".into(),
            page: 1,
            sources: vec!["usgs-nas".into()],
            nas_state: Some("NY".into()),
            nas_huc8: None,
        };
        let queens = area(-73.9, 40.7, -73.8, 40.8);
        let plan =
            plan_records(queens, query.clone(), String::new(), settings(), now()).expect("plan");
        let requests = plan.requests();
        assert_eq!(requests.len(), 1);
        let url = &requests[0].request.url;
        assert!(
            url.contains("state=NY")
                && url.contains("genus=Channa")
                && url.contains("species=argus")
        );
        let page = fixture("tier2/usgs-nas-occurrence-snakehead-ny.json");
        let first = round(&plan, now(), |_| Some(response(200, &page)));
        assert_eq!(first.records.len(), 3);
        // The page was full: the next one is asked, and an empty one ends it.
        let next = plan.requests();
        assert_eq!(next.len(), 1);
        assert!(next[0].request.url.contains("offset=3"));
        let done = round(&plan, now(), |_| {
            Some(response(
                200,
                br#"{"results":[],"endOfRecords":"true","count":0}"#,
            ))
        });
        assert!(plan.requests().is_empty());
        let pin = &done.records[0];
        assert_eq!(pin.licence, "Public domain");
        assert!(pin.link.is_some());
        assert_eq!(pin.link_title.as_deref(), Some("Record at USGS NAS"));
        assert!(
            pin.credit
                .contains("Nonindigenous Aquatic Species Database")
        );
        assert!(pin.credit.contains("Accessed 2026-11-17"));
        assert!(
            done.notes
                .iter()
                .any(|n| n.starts_with("These data vary in accuracy"))
        );

        // Outside the view, the state's records are not shown.
        let bronx = area(-73.95, 40.85, -73.85, 40.9);
        let plan = plan_records(bronx, query, String::new(), settings(), now()).expect("plan");
        let results = round(&plan, now(), |_| {
            Some(response(
                200,
                br#"{"results":[],"endOfRecords":"true","count":0}"#,
            ))
        });
        assert!(results.records.is_empty());
        let other = round(&plan, now(), |_| None);
        assert!(other.records.is_empty());
    }

    // ---------------------------------------------------------------------------------------------
    // Stocking
    // ---------------------------------------------------------------------------------------------

    #[test]
    fn stocked_waters_carry_the_stocking_card_once_the_lists_answer() {
        let westchester = area(-73.95, 41.0, -73.7, 41.3);
        let plan = plan_stocking(westchester, settings(), now()).expect("plan");
        let before = plan.results();
        assert!(before.state_json.is_none());
        let kensico = before
            .stocking
            .iter()
            .find(|p| p.waterbody == "Kensico Reservoir")
            .expect("Kensico Reservoir is in the catalogue");
        assert_eq!(kensico.kind, "lake");
        assert!(kensico.card_json.is_none());
        assert_eq!(kensico.label, "Kensico Reservoir");

        let requests = plan.requests();
        assert!(!requests.is_empty());
        for r in &requests {
            assert!(r.request.url.starts_with("https://data.ny.gov/resource/"));
            assert_eq!(header(r, "Accept"), Some("application/json"));
            // Names and a county only: no coordinates leave the phone.
            assert!(!r.request.url.contains("41."));
        }
        let actual = fixture("tier2/socrata-ny-e52k-ymww-westchester-2025.json");
        let planned = fixture("tier2/socrata-ny-d9y2-n436-westchester.json");
        let results = round(&plan, now(), |r| {
            Some(if r.request.url.contains("e52k-ymww") {
                response(200, &actual)
            } else {
                response(200, &planned)
            })
        });
        let kensico = results
            .stocking
            .iter()
            .find(|p| p.waterbody == "Kensico Reservoir")
            .expect("Kensico Reservoir");
        let card: serde_json::Value =
            serde_json::from_str(kensico.card_json.as_deref().expect("a card")).expect("json");
        assert_eq!(card["chrome"]["id"], "stocking");
        let summary = card["full"]["recent"][0]["summary"]
            .as_str()
            .expect("summary");
        assert!(
            summary.starts_with("Brown Trout · Apr 2025 · about 8,920"),
            "{summary}"
        );
        assert_eq!(kensico.label, card["compact"]["lines"][0].as_str().unwrap());
        assert!(
            results
                .credits
                .iter()
                .any(|c| c.source == "ny-dec-stocking" && c.text.contains("NYS DEC"))
        );
        assert!(plan.requests().is_empty());

        // Out at sea there is nothing to ask about.
        let sea = plan_stocking(area(-73.0, 39.5, -72.8, 39.7), settings(), now()).expect("plan");
        assert!(sea.requests().is_empty());
        let results = sea.results();
        assert!(results.stocking.is_empty() && results.credits.is_empty());
        assert_eq!(results.notices, ["No recent stocking reported nearby"]);
    }

    // ---------------------------------------------------------------------------------------------
    // Beach bloom forecasts and the words
    // ---------------------------------------------------------------------------------------------

    fn zip_response() -> HostResponse {
        let mut r = response(
            200,
            &fixture("hab/noaa-hab-gulf-rif-currentmodelrun-2026-11-17.zip"),
        );
        r.content_type = Some("application/octet-stream".into());
        r
    }

    /// Clearwater Beach and the beaches around it.
    fn clearwater() -> NearbyArea {
        area(-83.1, 27.8, -82.6, 28.2)
    }

    #[test]
    fn blooms_list_the_beaches_in_the_view_with_the_credit_and_the_advice() {
        let at = parse_rfc3339("2026-11-17T09:00:00Z").expect("time").0;
        let plan = plan_blooms(clearwater(), settings(), at).expect("plan");
        let requests = plan.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].source, "noaa-hab");
        assert!(
            requests[0]
                .request
                .url
                .ends_with("RIFv2_currentmodelrun.zip")
        );
        assert_eq!(requests[0].request.max_age_s, Some(3_600));
        // The region-wide file is asked for without the view.
        assert!(!requests[0].request.url.contains("27."));
        let results = round(&plan, at, |_| Some(zip_response()));
        assert!(plan.requests().is_empty());
        let pin = results
            .blooms
            .iter()
            .find(|p| p.name == "Clearwater Beach")
            .expect("Clearwater Beach");
        assert_eq!(pin.level, "very_low");
        assert_eq!(pin.level_label, "Very low");
        assert!(
            pin.detail.starts_with("Respiratory irritation: very low"),
            "{}",
            pin.detail
        );
        assert!(pin.forecast_for.contains("Nov 17"), "{}", pin.forecast_for);
        assert!(
            pin.link
                .as_deref()
                .is_some_and(|l| l.starts_with("https://"))
        );
        assert!(
            results
                .blooms
                .iter()
                .all(|p| (27.8..=28.2).contains(&p.lat))
        );
        assert!(results.credits.iter().any(|c| {
            c.source == "noaa-hab"
                && c.text
                    .contains("NOAA National Centers for Coastal Ocean Science")
        }));
        assert!(
            results
                .notes
                .iter()
                .any(|n| n.contains("make people and dogs sick"))
        );
        assert!(results.state_json.is_none());
    }

    #[test]
    fn blooms_outside_the_gulf_ask_for_nothing_and_say_so() {
        let plan = plan_blooms(harbor(), settings(), now()).expect("plan");
        assert!(plan.requests().is_empty());
        let results = plan.results();
        assert!(results.blooms.is_empty() && results.credits.is_empty());
        assert_eq!(
            results.notices,
            ["The beach bloom forecast covers the Gulf coast of Florida and Texas only"]
        );
    }

    #[test]
    fn blooms_in_a_view_without_beaches_or_with_an_old_run_say_so() {
        let at = parse_rfc3339("2026-11-17T09:00:00Z").expect("time").0;
        let plan = plan_blooms(area(-87.0, 29.0, -86.5, 29.5), settings(), at).expect("plan");
        let results = round(&plan, at, |_| Some(zip_response()));
        assert!(results.blooms.is_empty());
        assert_eq!(results.notices, ["No forecast beaches in this view"]);

        let old = parse_rfc3339("2026-11-20T09:00:00Z").expect("time").0;
        let plan = plan_blooms(clearwater(), settings(), old).expect("plan");
        let results = round(&plan, old, |_| Some(zip_response()));
        assert!(results.blooms.is_empty());
        assert_eq!(results.notices, ["No recent bloom forecast"]);
    }

    #[test]
    fn the_lookup_buttons_have_their_words() {
        let w = nearby_words(settings()).expect("words");
        assert_eq!(w.find_access, "Find ramps and piers here");
        assert_eq!(w.finding_access, "Looking for ramps and piers\u{2026}");
        assert_eq!(w.find_records, "Find species records here");
        assert_eq!(w.finding_records, "Looking for species records\u{2026}");
        assert_eq!(w.find_blooms, "Show beach bloom forecasts here");
        assert_eq!(w.finding_blooms, "Looking for beach forecasts\u{2026}");
    }
}
