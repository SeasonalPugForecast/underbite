//! `.underbite` files and live-shared trips as the app handles them.

use serde_json::{Value, json};
use underbite_ffi::settings::default_settings_json;
use underbite_ffi::share::{
    ShareError, ShareFuzz, SharedKind, TripChange, fuzz_trip, merge_trips, read_file, share_words,
    spot_file, trip_file,
};
use underbite_ffi::view::SpotInfo;

fn trip() -> Value {
    json!({
        "id": "trip-1",
        "name": "Hudson weekend",
        "places": [{
            "spot_id": "battery",
            "name": "The Battery",
            "area": "Lower Hudson",
            "latitude": 40.7033,
            "longitude": -74.0170,
            "time_zone": "America/New_York",
            "water_type": "tidal_river_estuary",
            "region": "US-NY",
        }],
        "first": "2027-07-12",
        "last": "2027-07-14",
        "species": ["morone-saxatilis"],
        "method": "kayak",
        "notes": "Launch at first light.",
        "checklist": [{"id": "c1", "text": "Licence", "done": false}],
        "modified_at": 1_783_000_000,
    })
}

fn battery() -> SpotInfo {
    SpotInfo {
        id: "battery".into(),
        name: "The Battery".into(),
        latitude: 40.7033,
        longitude: -74.0170,
        water_type: "tidal_river_estuary".into(),
        time_zone: "America/New_York".into(),
    }
}

fn place(file: &str) -> Value {
    let v: Value = serde_json::from_str(file).expect("file json");
    v["trip"]["places"][0].clone()
}

#[test]
fn a_trip_file_round_trips_rounded_to_about_a_kilometre() {
    let file = trip_file(trip().to_string(), ShareFuzz::Rounded1Km).expect("file");
    let v: Value = serde_json::from_str(&file).unwrap();
    assert_eq!(v["format"], "underbite");
    assert_eq!(v["v"], 1);
    assert_eq!(v["kind"], "trip");
    assert_eq!(place(&file)["latitude"], 40.7);
    assert_eq!(place(&file)["longitude"], -74.02);
    assert_ne!(place(&file)["fuzz"], "exact");

    let read = read_file(file.into_bytes(), default_settings_json()).expect("read");
    assert_eq!(read.kind, SharedKind::Trip);
    assert_eq!(read.approximate.as_deref(), Some("Approximate location"));
    let back: Value = serde_json::from_str(read.trip_json.as_deref().unwrap()).unwrap();
    assert_eq!(back["name"], "Hudson weekend");
    assert_eq!(back["places"][0]["latitude"], 40.7);
}

#[test]
fn an_exact_file_is_not_approximate_and_area_only_has_no_coordinates() {
    let exact = trip_file(trip().to_string(), ShareFuzz::Exact).expect("file");
    assert_eq!(place(&exact)["latitude"], 40.7033);
    let read = read_file(exact.into_bytes(), default_settings_json()).expect("read");
    assert_eq!(read.approximate, None);

    let area = trip_file(trip().to_string(), ShareFuzz::AreaOnly).expect("file");
    assert_eq!(place(&area).get("latitude"), None);
    assert_eq!(place(&area)["area"], "Lower Hudson");
    assert_eq!(place(&area)["region"], "US-NY");
}

#[test]
fn fuzz_only_coarsens() {
    let area = fuzz_trip(trip().to_string(), ShareFuzz::AreaOnly).expect("fuzzed");
    let again = fuzz_trip(area.clone(), ShareFuzz::Rounded1Km).expect("fuzzed");
    let v: Value = serde_json::from_str(&again).unwrap();
    assert_eq!(v["places"][0]["fuzz"], "area_only");
    assert_eq!(v["places"][0].get("latitude"), None);
    let exact = fuzz_trip(trip().to_string(), ShareFuzz::Exact).expect("fuzzed");
    let v: Value = serde_json::from_str(&exact).unwrap();
    assert_eq!(v["places"][0]["latitude"], 40.7033);
}

#[test]
fn a_spot_file_reads_back_as_a_spot_to_add() {
    let file = spot_file(
        battery(),
        Some("Lower Hudson".into()),
        Some("US-NY".into()),
        "Fish the last of the ebb.".into(),
        ShareFuzz::Rounded1Km,
    )
    .expect("file");
    let read = read_file(file.into_bytes(), default_settings_json()).expect("read");
    assert_eq!(read.kind, SharedKind::Spot);
    assert_eq!(read.notes.as_deref(), Some("Fish the last of the ebb."));
    let spot = read.spot.expect("spot");
    assert_eq!((spot.latitude, spot.longitude), (40.7, -74.02));
    assert_eq!(spot.water_type, "tidal_river_estuary");
    assert!(read.approximate.is_some());

    let area = spot_file(battery(), None, None, String::new(), ShareFuzz::AreaOnly).expect("file");
    let read = read_file(area.into_bytes(), default_settings_json()).expect("read");
    assert_eq!(read.spot, None);
    assert!(read.place_json.is_some());

    let mut bad = battery();
    bad.water_type = "puddle".into();
    assert!(matches!(
        spot_file(bad, None, None, String::new(), ShareFuzz::Exact),
        Err(ShareError::Invalid { .. })
    ));
}

#[test]
fn files_that_cannot_be_used_say_why_in_the_cores_words() {
    let newer = json!({"format": "underbite", "v": 9, "kind": "trip"}).to_string();
    assert_eq!(
        read_file(newer.into_bytes(), default_settings_json()),
        Err(ShareError::NeedsNewerVersion {
            message: "This file needs a newer version of the app.".into()
        })
    );
    let layers = json!({"format": "underbite", "v": 1, "kind": "layers", "layers": {}}).to_string();
    match read_file(layers.into_bytes(), default_settings_json()) {
        Err(ShareError::Unsupported { kind, message }) => {
            assert_eq!(kind, "layers");
            assert!(message.contains("map layers"), "{message}");
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        read_file(b"hello".to_vec(), default_settings_json()),
        Err(ShareError::Unreadable { .. })
    ));
    assert!(matches!(
        read_file(vec![b' '; 1_000_001], default_settings_json()),
        Err(ShareError::TooLarge { .. })
    ));
    let mut backwards = trip();
    backwards["last"] = "2027-07-01".into();
    assert!(matches!(
        trip_file(backwards.to_string(), ShareFuzz::Exact),
        Err(ShareError::DatesBackwards { .. })
    ));
    let mut long = trip();
    long["last"] = "2027-09-01".into();
    assert!(matches!(
        trip_file(long.to_string(), ShareFuzz::Exact),
        Err(ShareError::TooLong { .. })
    ));
    assert!(matches!(
        trip_file("[]".into(), ShareFuzz::Exact),
        Err(ShareError::Invalid { .. })
    ));
}

#[test]
fn a_merge_keeps_both_sides_edits_and_lists_what_changed_under_this_device() {
    let base = trip();
    let mut mine = trip();
    mine["notes"] = "Launch at first light. Bring the net.".into();
    mine["modified_at"] = 1_783_000_100.into();
    let mut theirs = trip();
    theirs["name"] = "Harbor weekend".into();
    theirs["checklist"][0]["done"] = true.into();
    theirs["first"] = "2027-07-13".into();
    theirs["last"] = "2027-07-15".into();
    theirs["modified_at"] = 1_783_000_050.into();

    let merged =
        merge_trips(base.to_string(), mine.to_string(), theirs.to_string()).expect("merged");
    let v: Value = serde_json::from_str(&merged.trip_json).unwrap();
    assert_eq!(v["name"], "Harbor weekend");
    assert_eq!(v["notes"], "Launch at first light. Bring the net.");
    assert_eq!(v["first"], "2027-07-13");
    assert_eq!(v["checklist"][0]["done"], true);
    assert!(merged.changes.contains(&TripChange::Renamed {
        from: "Hudson weekend".into(),
        to: "Harbor weekend".into()
    }));
    assert!(merged.changes.contains(&TripChange::Dates {
        from_first: "2027-07-12".into(),
        from_last: "2027-07-14".into(),
        to_first: "2027-07-13".into(),
        to_last: "2027-07-15".into(),
    }));
    assert!(merged.changes.contains(&TripChange::ChecklistTicked {
        id: "c1".into(),
        text: "Licence".into(),
        done: true
    }));
    assert!(
        !merged
            .changes
            .iter()
            .any(|c| matches!(c, TripChange::Notes { .. })),
        "{:?}",
        merged.changes
    );

    let same = merge_trips(base.to_string(), base.to_string(), base.to_string()).expect("merged");
    assert_eq!(same.changes, Vec::new());
    assert!(merge_trips("{}".into(), base.to_string(), base.to_string()).is_err());
}

#[test]
fn a_participants_rounded_copy_never_overwrites_the_owners_exact_place() {
    let owner = trip();
    let rounded: Value =
        serde_json::from_str(&fuzz_trip(owner.to_string(), ShareFuzz::Rounded1Km).unwrap())
            .unwrap();
    let mut theirs = rounded;
    theirs["notes"] = "See you there.".into();
    theirs["modified_at"] = 1_783_000_200.into();
    let merged =
        merge_trips(owner.to_string(), owner.to_string(), theirs.to_string()).expect("merged");
    let v: Value = serde_json::from_str(&merged.trip_json).unwrap();
    assert_eq!(v["places"][0]["latitude"], 40.7033);
    assert_eq!(v["notes"], "See you there.");
}

#[test]
fn the_share_words_offer_three_precisions_with_about_a_kilometre_first() {
    let w = share_words(default_settings_json()).unwrap();
    let fuzz: Vec<ShareFuzz> = w.fuzz.iter().map(|c| c.fuzz).collect();
    assert_eq!(
        fuzz,
        [ShareFuzz::Exact, ShareFuzz::Rounded1Km, ShareFuzz::AreaOnly]
    );
    assert_eq!(w.default_fuzz, ShareFuzz::Rounded1Km);
    assert_eq!(w.fuzz[1].label, "About 1 km");
    assert!(w.fuzz.iter().all(|c| !c.detail.is_empty()));
    assert_eq!(w.trip_exists, "This trip is already on this device.");
    assert!(share_words("{".to_owned()).is_err());
}
