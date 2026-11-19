//! Every request that carries a place sends it no finer than two decimals: a spot's coordinates
//! are rounded, and the box of a map view (which may be centred on a saved spot) is rounded
//! outward. The check reads the numbers of the finished URLs.

use underbite_core::map::Bbox;
use underbite_core::time::UnixTime;
use underbite_providers::encdirect::{self, EncBand};
use underbite_providers::hydro;
use underbite_providers::ncei_dem;
use underbite_providers::request::{Coordinates, RoundedPoint};
use underbite_providers::source::FetchContext;
use underbite_providers::statemaps::{self, ContourState};

/// A point in New York Harbor with more digits than may be sent.
const SPOT: Coordinates = Coordinates {
    latitude: 40.703_312_6,
    longitude: -74.017_054_9,
};

/// A view centred on the spot, with the digits a map gives.
const VIEW: Bbox = Bbox {
    west: -74.057_054_912_345,
    south: 40.673_312_654_321,
    east: -73.977_054_912_345,
    north: 40.733_312_654_321,
};

/// Query parameters that are not a place: a generalisation tolerance and a response precision.
const NOT_A_PLACE: [&str; 2] = ["maxAllowableOffset", "geometryPrecision"];

/// Decimals of the longest number written with a decimal point in `text`.
fn most_decimals(text: &str) -> usize {
    text.split(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
        .filter_map(|n| n.split_once('.'))
        .map(|(_, fraction)| fraction.len())
        .max()
        .unwrap_or(0)
}

fn assert_two_decimals(what: &str, url: &str) {
    let query = url.split_once('?').map_or("", |(_, q)| q);
    for param in query.split('&') {
        let (name, value) = param.split_once('=').unwrap_or((param, ""));
        if NOT_A_PLACE.contains(&name) {
            continue;
        }
        assert!(
            most_decimals(value) <= 2,
            "{what}: {name} has more than two decimals in {url}"
        );
    }
}

fn ctx() -> FetchContext {
    FetchContext::new(UnixTime(1_794_000_000), "underbite/test")
}

#[test]
fn places_are_sent_to_at_most_two_decimals() {
    let urls = [
        (
            "ncei point",
            ncei_dem::query_request(RoundedPoint::new(SPOT, 2)).url,
        ),
        (
            "enc points",
            encdirect::points_request(EncBand::Harbour, 1, VIEW, 0)
                .unwrap()
                .url,
        ),
        (
            "indiana contours",
            statemaps::contour_request(ContourState::Indiana, VIEW, 14, 0)
                .unwrap()
                .url,
        ),
        (
            "minnesota contours",
            statemaps::contour_request(ContourState::Minnesota, VIEW, 14, 0)
                .unwrap()
                .url,
        ),
        (
            "flowlines",
            hydro::flowlines_query(SPOT, 2_000.0, &ctx()).unwrap().url,
        ),
        (
            "waterbodies",
            hydro::waterbodies_query(SPOT, 2_000.0, &ctx()).unwrap().url,
        ),
    ];
    for (what, url) in &urls {
        assert!(url.contains('?'), "{what}: {url}");
        assert_two_decimals(what, url);
    }
}

#[test]
fn a_view_box_is_rounded_outward_and_still_covers_the_view() {
    let url = encdirect::points_request(EncBand::Harbour, 1, VIEW, 0)
        .unwrap()
        .url;
    assert!(url.contains("geometry=-74.06,40.67,-73.97,40.74&"), "{url}");
    let url = statemaps::contour_request(ContourState::Indiana, VIEW, 14, 0)
        .unwrap()
        .url;
    assert!(url.contains("geometry=-74.06,40.67,-73.97,40.74&"), "{url}");
}

#[test]
fn the_hydro_envelope_is_rounded_outward() {
    let url = hydro::flowlines_query(SPOT, 2_000.0, &ctx()).unwrap().url;
    let [xmin, ymin, xmax, ymax] = hydro::envelope_from_url(&url).unwrap();
    assert!(xmin < SPOT.longitude && SPOT.longitude < xmax);
    assert!(ymin < SPOT.latitude && SPOT.latitude < ymax);
}
