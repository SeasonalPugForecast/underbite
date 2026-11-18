//! `import_detect` and `import_file` over the recorded sample files, offline.

use std::path::PathBuf;

use underbite_ffi::import::{ImportError, ImportFormat, import_detect, import_file};

fn sample(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/raw/import")
        .join(name);
    std::fs::read(path).unwrap()
}

#[test]
fn a_gpx_file_is_detected_and_read() {
    let bytes = sample("battery.synthetic.gpx");
    assert_eq!(
        import_detect(bytes.clone(), "x.dat".into()),
        Some(ImportFormat::Gpx)
    );
    let file = import_file(bytes, "battery.gpx".into()).unwrap();
    assert_eq!(file.format, ImportFormat::Gpx);
    assert!(!file.waypoints.is_empty() || !file.tracks.is_empty());
    assert!(file.summary.contains("waypoint"));
    for w in &file.waypoints {
        assert!((40.0..41.5).contains(&w.latitude), "{w:?}");
        assert!((-75.0..-73.0).contains(&w.longitude), "{w:?}");
    }
}

#[test]
fn every_format_is_detected_by_content() {
    for (name, format) in [
        ("croton.synthetic.gpx", ImportFormat::Gpx),
        ("croton.synthetic.kml", ImportFormat::Kml),
        ("croton.synthetic.kmz", ImportFormat::Kmz),
        ("croton.synthetic.csv", ImportFormat::Csv),
    ] {
        let bytes = sample(name);
        assert_eq!(import_detect(bytes.clone(), String::new()), Some(format));
        let file = import_file(bytes, String::new()).unwrap();
        assert_eq!(file.format, format, "{name}");
        assert!(
            !file.waypoints.is_empty() || !file.tracks.is_empty(),
            "{name}"
        );
    }
}

#[test]
fn an_unknown_file_is_refused_in_the_cores_words() {
    assert_eq!(import_detect(b"hello".to_vec(), "notes.txt".into()), None);
    let err = import_file(b"hello".to_vec(), "notes.txt".into()).unwrap_err();
    assert_eq!(
        err,
        ImportError::Invalid {
            message: "the file is not GPX, KML, KMZ or CSV".into()
        }
    );
}

#[test]
fn a_malformed_file_reports_a_line_and_a_bad_archive_does_not_panic() {
    let err = import_file(b"<gpx><wpt lat=\"x\"".to_vec(), "a.gpx".into()).unwrap_err();
    let ImportError::Invalid { message } = err;
    assert!(message.contains("line"), "{message}");
    assert!(import_file(sample("bomb.synthetic.kmz"), "bomb.kmz".into()).is_err());
    // Whatever the archive holds, reading it returns instead of panicking.
    let _ = import_file(sample("escape.synthetic.kmz"), "escape.kmz".into());
}

#[test]
fn a_file_over_the_limit_is_refused_before_it_is_read() {
    let big = vec![b' '; 20 * 1024 * 1024 + 1];
    let ImportError::Invalid { message } = import_file(big, "big.gpx".into()).unwrap_err();
    assert!(message.contains("the limit is"), "{message}");
}
