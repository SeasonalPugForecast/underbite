//! CSV waypoints: a name, a latitude and a longitude per row.
//!
//! The first row is the header. Columns are found by name, in any order and any case: `name`
//! (`title`, `label`), `lat` (`latitude`), `lon` (`lng`, `long`, `longitude`), `notes` (`note`,
//! `description`, `desc`, `comment`) and `time` (`date`). `name`, `notes` and `time` are optional.
//! A byte order mark, CRLF line ends and quoted fields are accepted. The separator is a comma, or
//! a semicolon when the header has no comma. A decimal comma in a coordinate is an error, never a
//! guess.

use ::csv::{ReaderBuilder, StringRecord, Trim};

use super::{
    ImportError, Imported, MAX_POINTS, Waypoint, assumed_utc_note, check_latitude, check_longitude,
    check_size, parse_time, strip_bom,
};

#[derive(Default)]
struct Columns {
    name: Option<usize>,
    lat: Option<usize>,
    lon: Option<usize>,
    notes: Option<usize>,
    time: Option<usize>,
}

fn err(line: u64, message: String) -> ImportError {
    ImportError::Parse {
        format: "CSV",
        line,
        message,
    }
}

/// Reads CSV waypoints.
pub fn parse(bytes: &[u8]) -> Result<Imported, ImportError> {
    check_size(bytes)?;
    let body = strip_bom(bytes);
    let first_line = body.split(|b| *b == b'\n').next().unwrap_or(&[]);
    let delimiter = if first_line.contains(&b',') || !first_line.contains(&b';') {
        b','
    } else {
        b';'
    };
    let mut reader = ReaderBuilder::new()
        .delimiter(delimiter)
        .has_headers(false)
        .flexible(true)
        .trim(Trim::All)
        .from_reader(body);
    let mut records = reader.records();

    let header = match records.next() {
        None => return Err(err(0, "the file is empty".to_owned())),
        Some(Err(e)) => return Err(csv_error(&e)),
        Some(Ok(h)) => h,
    };
    let columns = columns(&header)?;
    let (Some(lat_col), Some(lon_col)) = (columns.lat, columns.lon) else {
        return Err(err(
            1,
            "the header needs a latitude and a longitude column (`lat`, `lon`)".to_owned(),
        ));
    };

    let mut out = Imported::default();
    let mut assumed_utc = 0usize;
    for record in records {
        let record = record.map_err(|e| csv_error(&e))?;
        let line = record.position().map_or(0, ::csv::Position::line);
        if out.waypoints.len() >= MAX_POINTS {
            return Err(ImportError::TooManyPoints { max: MAX_POINTS });
        }
        if record.iter().all(str::is_empty) {
            continue;
        }
        let need = lat_col.max(lon_col) + 1;
        if record.len() < need {
            return Err(err(
                line,
                format!(
                    "the row has {} fields; a coordinate is missing",
                    record.len()
                ),
            ));
        }
        let latitude = coordinate(&record[lat_col], "latitude", line, check_latitude)?;
        let longitude = coordinate(&record[lon_col], "longitude", line, check_longitude)?;
        let time = match columns.time.and_then(|i| record.get(i)) {
            Some(t) if !t.is_empty() => {
                let (time, assumed) = parse_time(t).map_err(|m| err(line, m))?;
                if assumed {
                    assumed_utc += 1;
                }
                Some(time)
            }
            _ => None,
        };
        out.waypoints.push(Waypoint {
            name: text(&record, columns.name),
            latitude,
            longitude,
            elevation_m: None,
            time,
            notes: text(&record, columns.notes),
        });
    }
    if assumed_utc > 0 {
        out.notes.push(assumed_utc_note(assumed_utc));
    }
    Ok(out)
}

fn csv_error(e: &::csv::Error) -> ImportError {
    let line = e.position().map_or(0, ::csv::Position::line);
    let message = match e.kind() {
        ::csv::ErrorKind::Utf8 { .. } => "the text is not valid UTF-8".to_owned(),
        _ => e.to_string(),
    };
    err(line, message)
}

fn text(record: &StringRecord, col: Option<usize>) -> Option<String> {
    col.and_then(|i| record.get(i))
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn columns(header: &StringRecord) -> Result<Columns, ImportError> {
    let mut cols = Columns::default();
    for (i, raw) in header.iter().enumerate() {
        let key = raw.trim_start_matches('\u{feff}').to_ascii_lowercase();
        let slot = match key.as_str() {
            "name" | "title" | "label" => &mut cols.name,
            "lat" | "latitude" => &mut cols.lat,
            "lon" | "lng" | "long" | "longitude" => &mut cols.lon,
            "notes" | "note" | "description" | "desc" | "comment" => &mut cols.notes,
            "time" | "date" => &mut cols.time,
            _ => continue,
        };
        if slot.is_some() {
            return Err(err(1, format!("the column `{raw}` appears twice")));
        }
        *slot = Some(i);
    }
    Ok(cols)
}

fn coordinate(
    value: &str,
    what: &str,
    line: u64,
    check: fn(f64) -> Result<f64, String>,
) -> Result<f64, ImportError> {
    if looks_like_decimal_comma(value) {
        return Err(err(
            line,
            format!("the {what} `{value}` uses a decimal comma; write it with a point, as 40.7033"),
        ));
    }
    let v: f64 = value
        .parse()
        .map_err(|_| err(line, format!("the {what} `{value}` is not a number")))?;
    check(v).map_err(|m| err(line, m))
}

/// `40,7033` and `-74,017`: digits, one comma, digits, no point.
fn looks_like_decimal_comma(value: &str) -> bool {
    let v = value.strip_prefix(['-', '+']).unwrap_or(value);
    match v.split_once(',') {
        Some((a, b)) => {
            !a.is_empty()
                && !b.is_empty()
                && a.bytes().all(|c| c.is_ascii_digit())
                && b.bytes().all(|c| c.is_ascii_digit())
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BATTERY: &[u8] =
        include_bytes!("../../../../tests/fixtures/raw/import/battery.synthetic.csv");
    const CROTON: &[u8] =
        include_bytes!("../../../../tests/fixtures/raw/import/croton.synthetic.csv");
    const COMMA: &[u8] =
        include_bytes!("../../../../tests/fixtures/raw/import/decimal-comma.synthetic.csv");

    #[test]
    fn battery_with_bom_crlf_and_quotes() {
        let imp = parse(BATTERY).expect("parse");
        assert_eq!(imp.waypoints.len(), 2);
        let w = &imp.waypoints[0];
        assert_eq!(w.name.as_deref(), Some("The Battery, pier"));
        assert_eq!((w.latitude, w.longitude), (40.7033, -74.0170));
        assert_eq!(
            w.notes.as_deref(),
            Some("Stripers on the push\r\nsecond line")
        );
        let w2 = &imp.waypoints[1];
        assert_eq!(w2.name.as_deref(), Some("Slip 2"));
        assert_eq!(w2.notes, None);
        assert_eq!(imp.tracks.len(), 0);
    }

    #[test]
    fn croton_semicolons_any_order_and_case() {
        let imp = parse(CROTON).expect("parse");
        assert_eq!(imp.waypoints.len(), 1);
        let w = &imp.waypoints[0];
        assert_eq!(w.name.as_deref(), Some("Croton ford"));
        assert_eq!((w.latitude, w.longitude), (41.2108, -73.8625));
        assert_eq!(w.notes.as_deref(), Some("low water"));
    }

    #[test]
    fn decimal_comma_is_an_error_with_a_message() {
        match parse(COMMA) {
            Err(ImportError::Parse { line, message, .. }) => {
                assert_eq!(line, 2);
                assert!(message.contains("decimal comma"), "{message}");
            }
            other => panic!("expected an error, got {other:?}"),
        }
    }

    #[test]
    fn unquoted_decimal_comma_is_an_error_too() {
        let r = parse(b"name;lat;lon\nA;40,7033;-74,017\n");
        assert!(
            matches!(r, Err(ImportError::Parse { line: 2, .. })),
            "{r:?}"
        );
    }

    #[test]
    fn time_column_and_utc_note() {
        let imp = parse(b"lat,lon,time\n1,2,2025-06-01T12:00:00\n3,4,2025-06-01T12:00:00Z\n3,4,\n")
            .expect("parse");
        assert_eq!(imp.waypoints[0].time, imp.waypoints[1].time);
        assert_eq!(imp.waypoints[2].time, None);
        assert_eq!(
            imp.notes,
            vec!["1 time had no UTC offset and was read as UTC."]
        );
    }

    #[test]
    fn blank_rows_are_skipped_and_header_alias_names_work() {
        let imp = parse(b"LNG,Lat,Title\n\n2,1,A\n\n").expect("parse");
        assert_eq!(imp.waypoints.len(), 1);
        assert_eq!(imp.waypoints[0].name.as_deref(), Some("A"));
    }

    fn line_of(doc: &[u8]) -> u64 {
        match parse(doc) {
            Err(ImportError::Parse { line, .. }) => line,
            other => panic!("expected a parse error, got {other:?}"),
        }
    }

    #[test]
    fn errors_carry_a_line() {
        assert_eq!(line_of(b"name,lat,lon\nA,1,2\nB,x,2\n"), 3);
        assert_eq!(line_of(b"name,lat,lon\nA,95,2\n"), 2);
        assert_eq!(line_of(b"name,lat,lon\nA,1\n"), 2);
        assert_eq!(line_of(b"name,lat\nA,1\n"), 1);
        assert_eq!(line_of(b"lat,lat,lon\n"), 1);
    }

    #[test]
    fn malformed_inputs_are_errors_not_panics() {
        let cases: &[&[u8]] = &[
            b"",
            b"\xEF\xBB\xBF",
            b"\n\n",
            b"name\n",
            b"lat,lon\n1,nan\n",
            b"lat,lon\ninf,2\n",
            b"lat,lon\n\"1,2\n",
            b"lat,lon\n1,2\xFF\n",
            b"lat,lon\n1,2,3,4,5\n1\n",
            b"lat,lon,time\n1,2,never\n",
            b"\xFF\xFE,\x00",
        ];
        for (i, case) in cases.iter().enumerate() {
            let _ = parse(case);
            if i != 8 {
                assert!(parse(case).is_err(), "case {i} should be an error");
            }
        }
    }

    #[test]
    fn extra_fields_are_tolerated_but_short_rows_are_not() {
        let imp = parse(b"lat,lon\n1,2,3,4,5\n").expect("parse");
        assert_eq!(imp.waypoints.len(), 1);
        assert!(parse(b"lat,lon\n1,2,3,4,5\n1\n").is_err());
    }

    #[test]
    fn truncations_never_panic() {
        for end in 0..BATTERY.len() {
            let _ = parse(&BATTERY[..end]);
        }
    }

    #[test]
    fn point_limit_and_size_limit() {
        let mut doc = String::from("lat,lon\n");
        for _ in 0..=MAX_POINTS {
            doc.push_str("1,2\n");
        }
        assert_eq!(
            parse(doc.as_bytes()),
            Err(ImportError::TooManyPoints { max: MAX_POINTS })
        );
        let big = vec![b'a'; super::super::MAX_INPUT_BYTES + 1];
        assert!(matches!(parse(&big), Err(ImportError::TooLarge { .. })));
    }
}
