//! USGS Water Data (OGC API and Statistics) at the Croton River site, anonymously.
//!
//! USGS rate-limits an anonymous address after about ten requests in a minute, so each test
//! sends two or three and pauses between them; run these tests one at a time.

use underbite_core::model::Measurement as M;
use underbite_core::provenance::{SourceId, StationKind, UsgsSiteType};
use underbite_core::time::UtcOffset;
use underbite_fetch::Call;
use underbite_providers::request::Request;
use underbite_providers::source::{Fragments, Source};
use underbite_providers::usgs::Usgs;
use underbite_providers::usgs::sites;
use underbite_providers::usgs::statistics::Reading;

use super::{
    CROTON, ctx, expect, fail, get_call, now, parsed, point_target, requirements, run, sleep_s,
    station, station_target, transport,
};

/// Seconds between two USGS requests.
const PAUSE_S: u64 = 4;

fn croton() -> underbite_core::provenance::StationRef {
    station(
        SourceId::USGS,
        "USGS-01375000",
        "Croton River at New Croton Dam",
        StationKind::UsgsSite(UsgsSiteType::Stream),
        (41.2250, -73.8589),
        None,
    )
}

/// `latest-continuous` (and any follow-up the plan makes) for discharge, gage height and
/// water temperature at the Croton site.
#[test]
#[ignore = "live: needs the network"]
fn live_usgs_latest_continuous_croton() {
    let wanted = [M::Discharge, M::GageHeight, M::WaterTemperature];
    let target = station_target(croton(), &wanted, None);
    let found = run(&Usgs::new(), &target, &requirements(&wanted), 2);
    found.expect_all("USGS latest values at 01375000", &[M::Discharge]);
}

/// `daily` means of discharge at the Croton site.
#[test]
#[ignore = "live: needs the network"]
fn live_usgs_daily_croton() {
    let target = station_target(croton(), &[M::Discharge], None);
    let ctx = ctx();
    let usgs = Usgs::new();
    let requests = parsed(
        "https://api.waterdata.usgs.gov/ogcapi/v0/collections/daily/items",
        "the daily request",
        usgs.daily_requests(&target, &ctx),
    );
    let Some(request) = requests.first() else {
        fail(
            "https://api.waterdata.usgs.gov/ogcapi/v0/collections/daily/items",
            "no daily request planned",
        );
    };
    let fetched = get_call(&transport(), &Call::from(request));
    let mut out = Fragments::new();
    parsed(
        &request.url,
        "the USGS daily means",
        usgs.parse(request, &fetched.as_response(), &ctx, &mut out),
    );
    expect(
        &request.url,
        !out.is_empty(),
        "the daily means hold no discharge",
    );
}

/// `monitoring-locations` and `time-series-metadata` around the Croton spot, as discovery asks
/// for them (the stream request only, to stay within the anonymous limit).
#[test]
#[ignore = "live: needs the network"]
fn live_usgs_site_lists_croton() {
    let target = point_target(CROTON, &[M::Discharge]);
    let ctx = ctx();
    let transport = transport();
    let half_width = 0.3;
    let locations = parsed(
        "https://api.waterdata.usgs.gov/ogcapi/v0/collections/monitoring-locations/items",
        "the site list request",
        sites::locations_requests(&target, half_width, &ctx),
    );
    let streams: Option<&Request> = locations
        .iter()
        .find(|r| r.url.contains("site_type_code=ST&"));
    let Some(request) = streams.or_else(|| locations.first()) else {
        fail(
            "https://api.waterdata.usgs.gov/ogcapi/v0/collections/monitoring-locations/items",
            "no site list request planned",
        );
    };
    let fetched = get_call(&transport, &Call::from(request));
    let stations = parsed(
        &request.url,
        "the USGS site list",
        sites::parse_locations(&fetched.body),
    );
    expect(
        &request.url,
        stations.iter().any(|s| s.id.ends_with("01375000")),
        format!(
            "the list has {} sites but not the Croton site 01375000",
            stations.len()
        ),
    );

    sleep_s(PAUSE_S);
    let request = parsed(
        "https://api.waterdata.usgs.gov/ogcapi/v0/collections/time-series-metadata/items",
        "the series list request",
        sites::metadata_request(&target, half_width, &ctx),
    );
    let fetched = get_call(&transport, &Call::from(&request));
    let offers = parsed(
        &request.url,
        "the USGS series list",
        sites::parse_metadata(&fetched.body),
    );
    expect(
        &request.url,
        offers.values().any(|m| m.contains_key(&M::Discharge)),
        format!(
            "none of the {} sites near Croton offers discharge",
            offers.len()
        ),
    );
}

/// The Statistics API (day-of-year normals, a beta service): the percentile of a discharge at
/// the Croton site, and the normals over a few days.
#[test]
#[ignore = "live: needs the network"]
fn live_usgs_statistics_croton() {
    let target = station_target(croton(), &[M::Discharge], None);
    let ctx = ctx();
    let usgs = Usgs::new();
    let transport = transport();
    let reading = Reading {
        measurement: M::Discharge,
        datum: None,
        value: 2.0,
        at: now(),
    };
    let request = parsed(
        "https://api.waterdata.usgs.gov/statistics/v0/observationNormals",
        "the normals request",
        usgs.statistics_request(&target, &reading, UtcOffset(-5 * 3_600), &ctx),
    );
    let fetched = get_call(&transport, &Call::from(&request));
    let mut out = Fragments::new();
    parsed(
        &request.url,
        "the USGS normals",
        usgs.parse(&request, &fetched.as_response(), &ctx, &mut out),
    );
    expect(
        &request.url,
        out.fragments()
            .iter()
            .any(|f| f.measurement == M::Discharge),
        "the normals gave no percentile for discharge",
    );

    sleep_s(PAUSE_S);
    let today = now().local_date(UtcOffset(-5 * 3_600));
    let requests = parsed(
        "https://api.waterdata.usgs.gov/statistics/v0/observationNormals",
        "the normals range request",
        usgs.statistics_range_requests(&target, M::Discharge, None, today, today, &ctx),
    );
    let Some(request) = requests.first() else {
        fail(
            "https://api.waterdata.usgs.gov/statistics/v0/observationNormals",
            "no normals range request planned",
        );
    };
    let fetched = get_call(&transport, &Call::from(request));
    let range = parsed(
        &request.url,
        "the USGS normals range",
        usgs.parse_statistics_range(request, &fetched.as_response(), &ctx),
    );
    let day = format!("{:02}-{:02}", today.month, today.day);
    expect(
        &request.url,
        range.day(&day).is_some(),
        format!("the normals have no day {day}"),
    );
}
