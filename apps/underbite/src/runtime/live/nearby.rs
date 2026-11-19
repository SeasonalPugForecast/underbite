//! Lookups around a place: 3DHP hydrography, PAD-US, Overpass, GBIF, iNaturalist and USGS NAS.
//!
//! Overpass and iNaturalist ask for care: run `live_overpass_*` and `live_inaturalist_*` on
//! their own (each waits a few seconds before its request, and sends one).

use underbite_core::map::Bbox;
use underbite_fetch::Call;
use underbite_providers::access::overpass::{
    self, CacheKey, DEFAULT_ENDPOINT, QueryKind, USER_AGENT as OVERPASS_USER_AGENT,
};
use underbite_providers::access::padus;
use underbite_providers::hydro;
use underbite_providers::request::Coordinates;
use underbite_providers::species::{gbif, inat, nas, request_box};

use super::super::transport::UreqTransport;
use super::{BATTERY, CROTON, ctx, expect, fail, get_call, parsed, sleep_s, transport};

/// The harbour around the Battery.
const HARBOUR: Bbox = Bbox {
    west: -74.1,
    south: 40.6,
    east: -73.9,
    north: 40.8,
};

/// 3DHP: the flowlines and waterbodies around the Croton spot, as discovery asks for them.
#[test]
#[ignore = "live: needs the network"]
fn live_3dhp_croton() {
    let transport = transport();
    let app = ctx();
    let at = Coordinates {
        latitude: CROTON.0,
        longitude: CROTON.1,
    };
    let flowlines = parsed(
        "https://hydro.nationalmap.gov/arcgis/rest/services/3DHP_all/FeatureServer",
        "the flowline query",
        hydro::flowlines_query(at, 2_000.0, &app),
    );
    let fetched = get_call(&transport, &Call::get(flowlines.url.clone()));
    let lines = parsed(
        &flowlines.url,
        "the 3DHP flowlines",
        hydro::parse_flowlines(&fetched.as_response()),
    );
    expect(
        &flowlines.url,
        !lines.is_empty(),
        "no flowline around the Croton spot",
    );

    std::thread::sleep(super::GAP);
    let waterbodies = parsed(
        "https://hydro.nationalmap.gov/arcgis/rest/services/3DHP_all/FeatureServer",
        "the waterbody query",
        hydro::waterbodies_query(at, 2_000.0, &app),
    );
    let fetched = get_call(&transport, &Call::get(waterbodies.url.clone()));
    let bodies = parsed(
        &waterbodies.url,
        "the 3DHP waterbodies",
        hydro::parse_waterbodies(&fetched.as_response()),
    );
    expect(
        &waterbodies.url,
        !bodies.is_empty(),
        "no waterbody around the Croton spot",
    );
}

/// PAD-US public land: the features of a box around Croton Point.
#[test]
#[ignore = "live: needs the network"]
fn live_padus_croton_point() {
    let area = request_box(Bbox {
        west: -73.92,
        south: 41.17,
        east: -73.85,
        north: 41.23,
    })
    .expect("a valid box");
    let request = parsed(
        "https://services.arcgis.com/",
        "the PAD-US request",
        padus::query_request(area, 0, 0.000_1),
    );
    let fetched = get_call(&transport(), &Call::get(request.url.clone()));
    let page = parsed(
        &request.url,
        "the PAD-US page",
        padus::parse_page(&fetched.as_response()),
    );
    expect(
        &request.url,
        !page.features.is_empty(),
        format!("no public land in the box ({} rows)", page.rows),
    );
}

/// Overpass: the places query (ramps, piers, fishing spots) for the 0.1 degree cell holding
/// the Battery. Run alone; it waits a few seconds first and sends one request.
#[test]
#[ignore = "live: needs the network; run alone"]
fn live_overpass_places_battery() {
    sleep_s(5);
    let area = overpass::area_key(BATTERY.0, BATTERY.1).expect("on Earth");
    let key = CacheKey {
        area,
        kind: QueryKind::Places,
    };
    let request = parsed(
        DEFAULT_ENDPOINT,
        "the Overpass request",
        overpass::query_request(DEFAULT_ENDPOINT, key),
    );
    let transport = UreqTransport::new(OVERPASS_USER_AGENT);
    let fetched = get_call(&transport, &Call::get(request.url.clone()));
    let page = parsed(
        &request.url,
        "the Overpass page",
        overpass::parse_page(&fetched.as_response(), QueryKind::Places),
    );
    expect(
        &request.url,
        !page.features.is_empty(),
        format!("no access place in the Battery cell ({} rows)", page.rows),
    );
}

/// GBIF: striped bass occurrences in the harbour box, and the month facet.
#[test]
#[ignore = "live: needs the network"]
fn live_gbif_striped_bass_harbour() {
    let area = request_box(HARBOUR).expect("a valid box");
    let occurrences = parsed(
        "https://api.gbif.org/v1/occurrence/search",
        "the GBIF request",
        gbif::occurrence_request("Morone saxatilis", area, 0, 20),
    );
    let transport = UreqTransport::new(&occurrences.user_agent);
    let fetched = get_call(&transport, &Call::get(occurrences.url.clone()));
    let page = parsed(
        &occurrences.url,
        "the GBIF page",
        gbif::parse_page(&fetched.as_response()),
    );
    expect(
        &occurrences.url,
        !page.records.is_empty(),
        format!("no striped bass record in the harbour ({} rows)", page.rows),
    );

    std::thread::sleep(super::GAP);
    let facet = parsed(
        "https://api.gbif.org/v1/occurrence/search",
        "the GBIF facet request",
        gbif::month_facet_request("Morone saxatilis", area),
    );
    let fetched = get_call(&transport, &Call::get(facet.url.clone()));
    let months = parsed(
        &facet.url,
        "the GBIF month facet",
        gbif::parse_month_facet(&fetched.as_response()),
    );
    expect(
        &facet.url,
        !months.months.is_empty(),
        "the month facet is empty",
    );
}

/// iNaturalist: striped bass observations in the harbour box. Run alone; it waits a few
/// seconds first and sends one request.
#[test]
#[ignore = "live: needs the network; run alone"]
fn live_inaturalist_striped_bass_harbour() {
    sleep_s(5);
    let area = request_box(HARBOUR).expect("a valid box");
    let request = parsed(
        "https://api.inaturalist.org/v1/observations",
        "the iNaturalist request",
        inat::observations_request("Morone saxatilis", area, 1, 5),
    );
    let transport = UreqTransport::new(&request.user_agent);
    let fetched = get_call(&transport, &Call::get(request.url.clone()));
    let page = parsed(
        &request.url,
        "the iNaturalist page",
        inat::parse_page(&fetched.as_response()),
    );
    if page.rows == 0 {
        fail(&request.url, "the iNaturalist answer holds no row");
    }
}

/// USGS Nonindigenous Aquatic Species: fish records in New York.
#[test]
#[ignore = "live: needs the network"]
fn live_usgs_nas_new_york_fishes() {
    let query = nas::NasQuery {
        state: Some("NY".to_owned()),
        huc8: None,
        taxon: nas::NasTaxon::Group("Fishes".to_owned()),
        years: None,
    };
    let request = parsed(
        "https://nas.er.usgs.gov/api/v2/occurrence/search",
        "the NAS request",
        nas::search_request(&query, 0, 20),
    );
    let fetched = get_call(&transport(), &Call::get(request.url.clone()));
    let page = parsed(
        &request.url,
        "the NAS page",
        nas::parse_page(&fetched.as_response()),
    );
    expect(
        &request.url,
        !page.records.is_empty(),
        format!("no fish record in New York ({} rows)", page.rows),
    );
}
