//! The map's view boxes leave the phone no finer than two decimals, rounded outward, even when
//! the view is centred on a saved spot.

use underbite_ffi::depth::{
    StructureBand, StructureLayers, StructureType, state_contour_request, structure_points_request,
};
use underbite_ffi::map::GridStore;

/// A view around a spot in New York Harbor, with the digits a map gives.
const VIEW: [f64; 4] = [
    -74.057_054_912_345,
    40.673_312_654_321,
    -74.017_054_912_345,
    40.733_312_654_321,
];

/// The value of a query parameter of a URL.
fn param(url: &str, name: &str) -> String {
    url.split(['?', '&'])
        .find_map(|p| p.strip_prefix(&format!("{name}=")))
        .unwrap_or_default()
        .to_owned()
}

#[test]
fn the_elevation_area_is_sent_rounded_outward() {
    let [w, s, e, n] = VIEW;
    let request = GridStore::new()
        .dem_area_request(w, s, e, n, 12.0)
        .unwrap()
        .unwrap();
    assert_eq!(
        param(&request.request.url, "bbox"),
        "-74.06,40.67,-74.01,40.74",
        "{}",
        request.request.url
    );
    assert!(
        request
            .key
            .starts_with("dem/area/-74060000/40670000/-74010000/40740000/"),
        "{}",
        request.key
    );
}

#[test]
fn structure_points_are_asked_for_a_rounded_outward_box() {
    let [w, s, e, n] = VIEW;
    let layers = StructureLayers {
        wrecks: Some(36),
        obstructions: None,
        rocks: None,
    };
    let request = structure_points_request(
        StructureBand::Harbour,
        layers,
        StructureType::Wreck,
        w,
        s,
        e,
        n,
        0,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        param(&request.url, "geometry"),
        "-74.06,40.67,-74.01,40.74",
        "{}",
        request.url
    );
}

#[test]
fn state_contours_are_asked_for_a_rounded_outward_box() {
    let request = state_contour_request(
        "lake-contours-in".into(),
        -86.311_234_5,
        39.111_234_5,
        -86.281_234_5,
        39.131_234_5,
        14,
        0,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        param(&request.url, "geometry"),
        "-86.32,39.11,-86.28,39.14",
        "{}",
        request.url
    );
}
