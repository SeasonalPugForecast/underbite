//! The catch log calls: bytes in and out, the editor, conditions, views, insights and export.

use serde_json::{Value, json};
use underbite_core::model::snapshot::testdata::sample_snapshot;
use underbite_ffi::log::{
    CatchFilter, CatchInput, CatchPoint, FateId, LogError, PlaceName, catch_cells,
    catch_detail_view, catch_list_view, catch_points, check_length_text, check_weight_text,
    fate_options, fish_units_info, insights_json, log_attach_conditions, log_delete, log_editor,
    log_export_csv, log_export_gpx, log_export_words, log_load, log_merge, log_new, log_save,
};
use underbite_ffi::settings::default_settings_json;

fn settings(edit: impl FnOnce(&mut Value)) -> String {
    let mut v: Value = serde_json::from_str(&default_settings_json()).unwrap();
    edit(&mut v);
    v.to_string()
}

fn us() -> String {
    default_settings_json()
}

fn metric_plain() -> String {
    settings(|v| {
        v["units"] = json!({ "discharge": "m3s", "height": "m", "speed": "ms", "water_temp": "C",
            "pressure": "hPa", "distance": "km" });
        v["presentation"]["voice"] = json!("plain");
    })
}

fn input(id: &str) -> CatchInput {
    let snap = sample_snapshot();
    CatchInput {
        id: id.into(),
        caught_at: snap.fetched_at.0,
        time_zone: "America/New_York".into(),
        latitude: 40.7033,
        longitude: -74.017,
        spot_id: None,
        area: Some("The Battery".into()),
        species: Some("morone-saxatilis".into()),
        species_text: None,
        length_text: "28.5".into(),
        weight_text: "9".into(),
        ounces_text: "8".into(),
        fate: FateId::Released,
        lure: Some("  white bucktail ".into()),
        method: None,
        notes: Some(String::new()),
        photos: vec![],
    }
}

fn saved(book: &[u8], input: CatchInput, settings: &str, now: i64) -> Vec<u8> {
    log_save(book.to_vec(), input, settings.into(), now)
        .unwrap()
        .book
}

#[test]
fn a_new_log_is_empty_and_loads_to_the_same_bytes() {
    let book = log_new();
    let loaded = log_load(book.clone()).unwrap();
    assert_eq!(loaded.book, book);
    assert_eq!((loaded.counts.catches, loaded.counts.deleted), (0, 0));
    let list: Value = serde_json::from_str(&catch_list_view(book, us(), vec![]).unwrap()).unwrap();
    assert_eq!(list["rows"], json!([]));
    assert!(list["empty"]["title"].is_string());
}

#[test]
fn bad_bytes_and_a_newer_log_are_refused_in_the_cores_words() {
    let LogError::Book { message } = log_load(b"nope".to_vec()).unwrap_err() else {
        panic!("not a book error");
    };
    assert!(
        message.starts_with("the catch log is not valid"),
        "{message}"
    );
    let LogError::Book { message } = log_load(vec![0xff, 0xfe]).unwrap_err() else {
        panic!("not a book error");
    };
    assert!(
        message.starts_with("the catch log is not valid"),
        "{message}"
    );
    let LogError::Book { message } = log_load(br#"{"v":99,"entries":[]}"#.to_vec()).unwrap_err()
    else {
        panic!("not a book error");
    };
    assert!(message.contains("newer than this app reads"), "{message}");
}

#[test]
fn saving_reads_sizes_in_the_users_units_and_the_editor_gives_them_back() {
    let book = saved(&log_new(), input("c1"), &us(), 1_000);
    let editor = log_editor(book.clone(), "c1".into(), us()).unwrap();
    assert_eq!(editor.length_text, "28.5");
    assert_eq!(editor.weight_text, "9");
    assert_eq!(editor.ounces_text, "8");
    assert_eq!(editor.lure.as_deref(), Some("white bucktail"));
    assert_eq!(editor.notes, None);
    assert_eq!(editor.species.as_deref(), Some("morone-saxatilis"));
    // The same catch in metric units.
    let m = log_editor(book.clone(), "c1".into(), metric_plain()).unwrap();
    assert_eq!(m.length_text, "72.4");
    assert_eq!(m.weight_text, "4.31");
    assert_eq!(m.ounces_text, "");
    let info = fish_units_info(us()).unwrap();
    assert_eq!(
        (info.length_unit.as_str(), info.weight_unit.as_str()),
        ("inches", "pounds")
    );
    assert_ne!(info.ounces_label, "");
    let info = fish_units_info(metric_plain()).unwrap();
    assert_eq!(
        (info.length_unit.as_str(), info.weight_unit.as_str()),
        ("centimetres", "kilograms")
    );
    assert_eq!(info.ounces_label, "");
}

#[test]
fn a_bad_field_names_itself_and_says_why_in_the_users_words() {
    let mut bad = input("c1");
    bad.length_text = "abc".into();
    let LogError::Field { field, message } = log_save(log_new(), bad, us(), 1).unwrap_err() else {
        panic!("not a field error");
    };
    assert_eq!(field, "length");
    assert!(message.contains("12.5"), "{message}");

    let mut bad = input("c1");
    bad.ounces_text = "17".into();
    let LogError::Field { field, .. } = log_save(log_new(), bad, us(), 1).unwrap_err() else {
        panic!("not a field error");
    };
    assert_eq!(field, "ounces");

    let mut bad = input("c1");
    bad.latitude = 123.0;
    assert!(matches!(
        log_save(log_new(), bad, us(), 1).unwrap_err(),
        LogError::Field { field, .. } if field == "location"
    ));
    let mut bad = input("c1");
    bad.photos = vec!["../secret".into()];
    assert!(matches!(
        log_save(log_new(), bad, us(), 1).unwrap_err(),
        LogError::Field { field, .. } if field == "photos"
    ));
    let mut bad = input("c1");
    bad.species = Some("Striped Bass".into());
    assert!(matches!(
        log_save(log_new(), bad, us(), 1).unwrap_err(),
        LogError::Field { field, .. } if field == "species"
    ));
    let mut bad = input("c1");
    bad.time_zone = "Mars/Olympus".into();
    assert!(matches!(
        log_save(log_new(), bad, us(), 1).unwrap_err(),
        LogError::Field { field, .. } if field == "time_zone"
    ));
}

#[test]
fn the_size_checks_return_the_message_or_nothing() {
    assert_eq!(check_length_text("28.5".into(), us()).unwrap(), None);
    assert_eq!(check_length_text(String::new(), us()).unwrap(), None);
    assert!(check_length_text("0".into(), us()).unwrap().is_some());
    assert_eq!(
        check_weight_text("9".into(), "8".into(), us()).unwrap(),
        None
    );
    assert!(
        check_weight_text("9".into(), "16".into(), us())
            .unwrap()
            .is_some()
    );
    // Ounces do not matter in kilograms.
    assert_eq!(
        check_weight_text("4.5".into(), "99".into(), metric_plain()).unwrap(),
        None
    );
}

#[test]
fn an_edit_keeps_the_creation_time_and_the_conditions_and_supersedes_the_old_record() {
    let snap = serde_json::to_string(&sample_snapshot()).unwrap();
    let book = saved(&log_new(), input("c1"), &us(), 1_000);
    let attached = log_attach_conditions(book, "c1".into(), snap, None, 1_001).unwrap();
    assert!(attached.changed);
    let mut edit = input("c1");
    edit.fate = FateId::Kept;
    // A clock that ran backwards still wins over the record it edits.
    let book = saved(&attached.book, edit, &us(), 500);
    let doc: Value = serde_json::from_slice(&book).unwrap();
    let entry = &doc["entries"][0];
    assert_eq!(entry["created_at"], 1_000);
    assert_eq!(entry["fate"], "kept");
    assert!(entry["modified_at"].as_i64().unwrap() > 1_001);
    assert!(entry["conditions"].is_object());
    assert_eq!(doc["entries"].as_array().unwrap().len(), 1);
}

#[test]
fn an_edit_that_moves_the_catch_drops_the_conditions_of_the_old_time_and_place() {
    let snap = serde_json::to_string(&sample_snapshot()).unwrap();
    let book = saved(&log_new(), input("c1"), &us(), 1_000);
    let book = log_attach_conditions(book, "c1".into(), snap.clone(), None, 1_001)
        .unwrap()
        .book;
    let conditions = |book: &[u8]| {
        let doc: Value = serde_json::from_slice(book).unwrap();
        doc["entries"][0]["conditions"].clone()
    };
    assert!(conditions(&book).is_object());

    // A later time, with no snapshot for it: the old conditions go.
    let mut later = input("c1");
    later.caught_at += 6 * 3_600;
    assert!(conditions(&saved(&book, later, &us(), 2_000)).is_null());

    // Another place, likewise.
    let mut elsewhere = input("c1");
    elsewhere.latitude = 40.9;
    let moved = saved(&book, elsewhere, &us(), 2_000);
    assert!(conditions(&moved).is_null());

    // With a snapshot for the new time and place, the app attaches its conditions again.
    let attached = log_attach_conditions(moved, "c1".into(), snap, None, 2_001).unwrap();
    assert!(attached.changed);
    assert!(conditions(&attached.book).is_object());

    // An edit that keeps the time and place keeps them.
    let mut notes = input("c1");
    notes.notes = Some("on the drop".into());
    assert!(conditions(&saved(&book, notes, &us(), 2_000)).is_object());
}

#[test]
fn conditions_come_from_the_snapshot_and_show_in_the_detail_with_their_sources() {
    let snap = sample_snapshot();
    let json = serde_json::to_string(&snap).unwrap();
    let book = saved(&log_new(), input("c1"), &us(), 1_000);
    let attached =
        log_attach_conditions(book.clone(), "c1".into(), json.clone(), None, 2_000).unwrap();
    assert!(attached.changed);
    let detail: Value = serde_json::from_str(
        &catch_detail_view(attached.book.clone(), "c1".into(), us(), vec![]).unwrap(),
    )
    .unwrap();
    assert_eq!(detail["row"]["has_conditions"], true);
    let lines = detail["conditions"].as_array().unwrap();
    assert_ne!(lines.len(), 0);
    assert!(lines.iter().all(|l| l["footnote"].is_string()));

    // A snapshot far from the catch time holds nothing near it: unchanged.
    let far =
        log_attach_conditions(book.clone(), "c1".into(), json.clone(), Some(0), 2_000).unwrap();
    assert!(!far.changed);
    assert_eq!(far.book, book);

    assert!(matches!(
        log_attach_conditions(book.clone(), "zz".into(), json, None, 1).unwrap_err(),
        LogError::NotFound { id } if id == "zz"
    ));
    assert!(matches!(
        log_attach_conditions(book, "c1".into(), "{".into(), None, 1).unwrap_err(),
        LogError::Invalid { .. }
    ));
}

#[test]
fn the_list_and_detail_are_in_the_users_units_and_voice() {
    let mut other = input("c2");
    other.caught_at += 3_600;
    other.spot_id = Some("battery".into());
    other.fate = FateId::KeptForSashimi;
    let book = saved(&saved(&log_new(), input("c1"), &us(), 1), other, &us(), 2);
    let spots = vec![PlaceName {
        spot_id: "battery".into(),
        name: "The Battery".into(),
    }];
    let list: Value =
        serde_json::from_str(&catch_list_view(book.clone(), us(), spots.clone()).unwrap()).unwrap();
    let rows = list["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["id"], "c2", "newest first");
    assert_eq!(rows[0]["fate_label"], "Kept for sashimi");
    assert!(rows[0]["size"].as_str().unwrap().contains("in"));

    let plain: Value =
        serde_json::from_str(&catch_list_view(book.clone(), metric_plain(), spots).unwrap())
            .unwrap();
    assert_eq!(plain["rows"][0]["fate_label"], "Kept");
    assert!(plain["rows"][0]["size"].as_str().unwrap().contains("cm"));

    assert!(matches!(
        catch_detail_view(book, "zz".into(), us(), vec![]).unwrap_err(),
        LogError::NotFound { .. }
    ));
}

#[test]
fn fate_options_follow_the_voice() {
    let luna = fate_options(us()).unwrap();
    assert_eq!(
        luna.iter()
            .map(|o| (o.id, o.label.as_str()))
            .collect::<Vec<_>>(),
        [
            (FateId::Released, "Released"),
            (FateId::Kept, "Kept"),
            (FateId::KeptForSashimi, "Kept for sashimi"),
        ]
    );
    let plain = fate_options(metric_plain()).unwrap();
    assert_eq!(
        plain.iter().map(|o| o.id).collect::<Vec<_>>(),
        [FateId::Released, FateId::Kept]
    );
}

#[test]
fn deleting_leaves_a_tombstone_and_returns_the_photos() {
    let mut with_photo = input("c1");
    with_photo.photos = vec!["a.jpg".into()];
    let book = saved(&log_new(), with_photo, &us(), 1);
    let gone = log_delete(book, "c1".into(), 5).unwrap();
    assert!(gone.deleted);
    assert_eq!(gone.photos, ["a.jpg"]);
    let loaded = log_load(gone.book.clone()).unwrap();
    assert_eq!((loaded.counts.catches, loaded.counts.deleted), (0, 1));
    let text = String::from_utf8(gone.book.clone()).unwrap();
    assert!(!text.contains("40.7033") && !text.contains("white bucktail"));
    let again = log_delete(gone.book, "c1".into(), 6).unwrap();
    assert!(!again.deleted);
    assert!(matches!(
        log_editor(again.book, "c1".into(), us()).unwrap_err(),
        LogError::NotFound { .. }
    ));
}

#[test]
fn merging_takes_the_later_edit_either_way_round() {
    let base = saved(&log_new(), input("c1"), &us(), 10);
    let mut edit = input("c1");
    edit.fate = FateId::Kept;
    let later = saved(&base, edit, &us(), 20);
    let other = saved(&log_new(), input("c2"), &us(), 5);
    let a = log_merge(base.clone(), later.clone()).unwrap();
    let b = log_merge(later, base).unwrap();
    assert_eq!(a.book, b.book);
    assert_eq!(a.changed, 1);
    assert_eq!(b.changed, 0);
    let c = log_merge(a.book, other).unwrap();
    assert_eq!(log_load(c.book).unwrap().counts.catches, 2);
}

#[test]
fn insights_and_the_heat_map_follow_the_filter() {
    let mut book = log_new();
    for i in 0..3 {
        let mut e = input(&format!("c{i}"));
        e.caught_at += i * 3_600;
        book = saved(&book, e, &us(), 1);
    }
    let mut snail = input("c9");
    snail.species = Some("alosa-sapidissima".into());
    snail.latitude = 40.9;
    book = saved(&book, snail, &us(), 1);

    let all: Value =
        serde_json::from_str(&insights_json(book.clone(), CatchFilter::default(), us()).unwrap())
            .unwrap();
    assert_eq!(all["total"], 4);
    assert!(all["notice"].is_string());
    let bass = CatchFilter {
        species: Some("morone-saxatilis".into()),
        ..CatchFilter::default()
    };
    let one: Value =
        serde_json::from_str(&insights_json(book.clone(), bass.clone(), us()).unwrap()).unwrap();
    assert_eq!(one["total"], 3);

    let cells = catch_cells(book.clone(), bass, 500.0).unwrap();
    assert_eq!(cells.iter().map(|c| c.count).sum::<u32>(), 3);
    assert!(cells.iter().all(|c| c.total == 3));
    assert_eq!(
        catch_cells(book.clone(), CatchFilter::default(), 0.0).unwrap(),
        []
    );
    assert!(matches!(
        catch_cells(
            book,
            CatchFilter {
                species: Some("Bad Id".into()),
                ..CatchFilter::default()
            },
            500.0
        )
        .unwrap_err(),
        LogError::Field { .. }
    ));
}

#[test]
fn the_map_pins_live_catches_where_they_were_made() {
    let mut north = input("c2");
    north.latitude = 40.9;
    north.longitude = -73.9;
    let book = saved(
        &saved(&saved(&log_new(), input("c1"), &us(), 1), north, &us(), 1),
        input("c3"),
        &us(),
        1,
    );
    let book = log_delete(book, "c3".into(), 9).unwrap().book;
    assert_eq!(
        catch_points(book).unwrap(),
        [
            CatchPoint {
                id: "c1".into(),
                latitude: 40.7033,
                longitude: -74.017,
            },
            CatchPoint {
                id: "c2".into(),
                latitude: 40.9,
                longitude: -73.9,
            },
        ]
    );
    assert_eq!(catch_points(log_new()).unwrap(), []);
    assert!(matches!(
        catch_points(b"nope".to_vec()).unwrap_err(),
        LogError::Book { .. }
    ));
}

#[test]
fn exports_are_made_on_request_from_live_catches_only() {
    let book = saved(
        &saved(&log_new(), input("c1"), &us(), 1),
        input("c2"),
        &us(),
        1,
    );
    let book = log_delete(book, "c2".into(), 9).unwrap().book;
    let csv = log_export_csv(book.clone()).unwrap();
    assert!(csv.contains("length_m"));
    assert!(csv.contains("c1") && !csv.contains("c2"));
    let gpx = log_export_gpx(book).unwrap();
    assert!(gpx.contains("<gpx"));
    assert_eq!(gpx.matches("<wpt").count(), 1);
}

#[test]
fn the_export_warns_of_exact_places_in_the_same_words_in_both_voices() {
    let luna = log_export_words(us()).unwrap();
    let plain = log_export_words(metric_plain()).unwrap();
    assert_eq!(luna.warning, plain.warning);
    assert!(luna.warning.contains("exact place"));
    assert_eq!(luna.csv, "Spreadsheet (CSV)");
    assert_eq!(luna.gpx, "Waypoints (GPX)");
    assert!(log_export_words("{".to_owned()).is_err());
}
