//! Exporting the log as CSV and GPX.
//!
//! Both exports are data, not a display: numbers are in canonical units with the unit in the
//! column name (`length_m`, `weight_kg`, `water_temperature_c`), and nothing is converted to the
//! user's units. Only live entries are written, never tombstones, and photos are never included.
//! The files hold exact coordinates; the host warns before sharing them.
//!
//! CSV follows RFC 4180 (CRLF line ends, quotes where a cell needs them). Because spreadsheet
//! applications run a cell that starts with `=`, `+`, `-` or `@`, a text cell that does is
//! written with a leading `'`; numeric cells are written as plain numbers and never touched.
//! GPX is 1.1 with one `<wpt>` per catch.

use jiff::Timestamp;
use serde::Serialize;

use super::{CatchConditions, CatchEntry, LogBook};
use crate::time::{UnixTime, zone};
use crate::trip::SpeciesId;

const HEADER: [&str; 39] = [
    "id",
    "caught_at_utc",
    "caught_at_local",
    "time_zone",
    "latitude",
    "longitude",
    "spot",
    "area",
    "species_id",
    "species",
    "length_m",
    "weight_kg",
    "fate",
    "lure",
    "method",
    "notes",
    "light",
    "tide_direction",
    "tide_hours_since_previous",
    "tide_hours_to_next",
    "tide_height_m",
    "tide_height_datum",
    "current_speed_ms",
    "current_set",
    "flow_m3s",
    "gage_height_m",
    "water_temperature_c",
    "air_temperature_c",
    "turbidity",
    "turbidity_unit",
    "pressure_hpa",
    "pressure_trend",
    "wind_speed_ms",
    "wind_gust_ms",
    "wind_from_deg",
    "moon_phase",
    "moon_illuminated_fraction",
    "solunar_kind",
    "solunar_start_utc",
];

/// The serde name of a unit-only enum, such as `kept_for_sashimi`.
fn tag<T: Serialize>(value: &T) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(s)) => s,
        _ => String::new(),
    }
}

fn utc_iso(t: UnixTime) -> Option<String> {
    let ts = Timestamp::from_second(t.0).ok()?;
    Some(ts.strftime("%Y-%m-%dT%H:%M:%SZ").to_string())
}

fn local_iso(t: UnixTime, tz: &str) -> Option<String> {
    let ts = Timestamp::from_second(t.0).ok()?;
    let z = ts.to_zoned(zone(tz).ok()?);
    Some(z.strftime("%Y-%m-%dT%H:%M:%S%:z").to_string())
}

/// A finite number as the shortest text that reads back as the same value.
fn num<T: std::fmt::Display + Copy + Into<f64>>(v: Option<T>) -> String {
    match v {
        Some(x) if x.into().is_finite() => x.to_string(),
        _ => String::new(),
    }
}

/// A free-text cell, guarded against spreadsheet formulas.
fn text(v: Option<&str>) -> String {
    let s = v.unwrap_or_default();
    if s.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        format!("'{s}")
    } else {
        s.to_owned()
    }
}

fn row(e: &CatchEntry) -> Vec<String> {
    let c = e.conditions.clone().unwrap_or_default();
    let CatchConditions {
        tide,
        current,
        flow,
        gage_height,
        water_temperature_c,
        turbidity,
        pressure_hpa,
        pressure_trend,
        wind,
        air_temperature_c,
        moon,
        solunar,
        light,
        ..
    } = &c;
    let tide_v = tide.as_ref().map(|s| &s.value);
    let tide_h = tide_v.and_then(|t| t.height);
    let cur = current.as_ref().map(|s| &s.value);
    let wind_v = wind.as_ref().map(|s| &s.value);
    let flow_v = flow.as_ref().map(|s| &s.value);
    let turb = turbidity.as_ref().map(|s| s.value);
    vec![
        text(Some(&e.id)),
        utc_iso(e.caught_at).unwrap_or_default(),
        local_iso(e.caught_at, &e.time_zone).unwrap_or_default(),
        text(Some(&e.time_zone)),
        num(Some(e.location.latitude)),
        num(Some(e.location.longitude)),
        text(e.location.spot_id.as_deref()),
        text(e.location.area.as_deref()),
        e.species
            .as_ref()
            .map(|s| text(Some(s.as_str())))
            .unwrap_or_default(),
        text(e.species_text.as_deref()),
        num(e.length_m),
        num(e.weight_kg),
        tag(&e.fate),
        text(e.lure.as_deref()),
        text(e.method.as_deref()),
        text(e.notes.as_deref()),
        light.as_ref().map(tag).unwrap_or_default(),
        tide_v.map(|t| tag(&t.direction)).unwrap_or_default(),
        num(tide_v.map(|t| t.hours_since_previous)),
        num(tide_v.map(|t| t.hours_to_next)),
        num(tide_h.map(|h| h.metres)),
        tide_h.map(|h| tag(&h.datum)).unwrap_or_default(),
        num(cur.map(|x| x.speed_ms)),
        cur.and_then(|x| x.set.as_ref().map(tag))
            .unwrap_or_default(),
        num(flow_v.map(|x| x.discharge_m3s)),
        num(gage_height.as_ref().map(|g| g.value.height.metres)),
        num(water_temperature_c.as_ref().map(|s| s.value)),
        num(air_temperature_c.as_ref().map(|s| s.value)),
        num(turb.map(|t| t.value)),
        turb.map(|t| tag(&t.unit)).unwrap_or_default(),
        num(pressure_hpa.as_ref().map(|s| s.value)),
        pressure_trend
            .as_ref()
            .map(|s| tag(&s.value))
            .unwrap_or_default(),
        num(wind_v.map(|w| w.speed_ms)),
        num(wind_v.and_then(|w| w.gust_ms)),
        num(wind_v.and_then(|w| w.from_deg)),
        moon.as_ref().map(|m| tag(&m.phase)).unwrap_or_default(),
        num(moon.as_ref().map(|m| m.illuminated_fraction)),
        solunar.as_ref().map(|s| tag(&s.kind)).unwrap_or_default(),
        solunar
            .as_ref()
            .and_then(|s| utc_iso(s.start))
            .unwrap_or_default(),
    ]
}

/// The live entries as CSV text: a header row, then one row per catch in log order.
///
/// Columns are named with their units. Times are given twice: `caught_at_utc` as ISO 8601 UTC and
/// `caught_at_local` as ISO 8601 with the offset of the entry's own time zone. Text cells that
/// start with `=`, `+`, `-`, `@`, a tab or a carriage return get a leading `'`. Lines end in
/// CRLF.
#[must_use]
pub fn export_csv(book: &LogBook) -> String {
    let mut w = csv::WriterBuilder::new()
        .terminator(csv::Terminator::CRLF)
        .from_writer(Vec::new());
    // Writing to a Vec cannot fail; a failure would only shorten the output.
    let _ = w.write_record(HEADER);
    for e in book.live() {
        let _ = w.write_record(row(e));
    }
    let bytes = w.into_inner().unwrap_or_default();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Escapes text for XML content, dropping characters XML 1.0 cannot carry.
fn xml(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\t' | '\n' | '\r' => out.push(ch),
            c if (c as u32) < 0x20 || c == '\u{FFFE}' || c == '\u{FFFF}' => {}
            c => out.push(c),
        }
    }
    out
}

fn waypoint(e: &CatchEntry, out: &mut String) {
    use std::fmt::Write;
    let _ = writeln!(
        out,
        "  <wpt lat=\"{}\" lon=\"{}\">",
        e.location.latitude, e.location.longitude
    );
    if let Some(t) = utc_iso(e.caught_at) {
        let _ = writeln!(out, "    <time>{t}</time>");
    }
    let name = e
        .species_text
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| e.species.as_ref().map(SpeciesId::as_str))
        .unwrap_or("Catch");
    let _ = writeln!(out, "    <name>{}</name>", xml(name));
    let mut desc = Vec::new();
    if let Some(m) = e.length_m.filter(|m| m.is_finite()) {
        desc.push(format!("length {m} m"));
    }
    if let Some(k) = e.weight_kg.filter(|k| k.is_finite()) {
        desc.push(format!("weight {k} kg"));
    }
    desc.push(format!("fate {}", tag(&e.fate)));
    for (label, v) in [("lure", &e.lure), ("method", &e.method)] {
        if let Some(v) = v.as_deref().filter(|v| !v.trim().is_empty()) {
            desc.push(format!("{label} {v}"));
        }
    }
    let mut d = desc.join("; ");
    if let Some(n) = e.notes.as_deref().filter(|n| !n.trim().is_empty()) {
        d.push('\n');
        d.push_str(n);
    }
    let _ = writeln!(out, "    <desc>{}</desc>", xml(&d));
    out.push_str("  </wpt>\n");
}

/// The live entries as a GPX 1.1 document with one `<wpt>` per catch: its position, `<time>` in
/// UTC, a `<name>` (the species text, else the species id, else "Catch") and a `<desc>` with size,
/// fate, lure, method and notes. Waypoints are in log order.
#[must_use]
pub fn export_gpx(book: &LogBook) -> String {
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <gpx version=\"1.1\" creator=\"Underbite\" xmlns=\"http://www.topografix.com/GPX/1/1\">\n",
    );
    for e in book.live() {
        waypoint(e, &mut out);
    }
    out.push_str("</gpx>\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::{csv as csv_import, gpx};
    use crate::log::{CatchLocation, Fate, Light, Sourced};
    use crate::provenance::SourceId;

    fn entry(id: &str, caught: i64, lat: f64, lon: f64) -> CatchEntry {
        CatchEntry::new(
            id,
            UnixTime(caught),
            "America/New_York",
            CatchLocation::new(lat, lon),
            UnixTime(caught),
        )
    }

    fn book() -> LogBook {
        let mut b = LogBook::new();
        let mut a = entry("0001", 1_794_000_000, 40.7003, -74.0142);
        a.location.spot_id = Some("battery".into());
        a.species = Some(SpeciesId::new("morone-saxatilis").unwrap());
        a.species_text = Some("Striped bass, schoolie".into());
        a.length_m = Some(0.71);
        a.weight_kg = Some(3.9);
        a.fate = Fate::KeptForSashimi;
        a.lure = Some("white \"bucktail\"".into());
        a.method = Some("jigging".into());
        a.notes = Some("Rips & eddies <fast>,\nline two \u{1F41F}".into());
        a.conditions = Some(CatchConditions {
            at: a.caught_at,
            water_temperature_c: Some(Sourced {
                value: 16.0,
                source: SourceId::COOPS,
                station: Some("8518750".into()),
                station_name: Some("The Battery".into()),
                age_s: 120,
            }),
            light: Some(Light::Dawn),
            ..CatchConditions::default()
        });
        b.upsert(a);
        let mut c = entry("0002", 1_794_100_000, 41.2108, -73.8625);
        c.notes = Some("=HYPERLINK(\"http://x\")".into());
        c.species_text = Some("-2 cm".into());
        b.upsert(c);
        b.upsert(entry("0003", 1_794_200_000, 40.5, -74.0));
        b.delete("0003", UnixTime(1_794_300_000));
        b
    }

    #[test]
    fn csv_snapshot() {
        insta::assert_snapshot!(export_csv(&book()));
    }

    #[test]
    fn gpx_snapshot() {
        insta::assert_snapshot!(export_gpx(&book()));
    }

    #[test]
    fn tombstones_are_not_exported() {
        let csv = export_csv(&book());
        assert!(!csv.contains("0003"));
        assert!(!export_gpx(&book()).contains("40.5"));
        assert_eq!(csv.matches("\r\n").count(), 3);
    }

    #[test]
    fn csv_reads_back_with_the_same_cells() {
        let text = export_csv(&book());
        let mut r = ::csv::Reader::from_reader(text.as_bytes());
        let rows: Vec<_> = r.records().map(|x| x.unwrap()).collect();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].len(), HEADER.len());
        assert_eq!(&rows[0][15], "Rips & eddies <fast>,\nline two \u{1F41F}");
        assert_eq!(&rows[0][13], "white \"bucktail\"");
        // Formula-looking text is neutralised, numbers are not.
        assert_eq!(&rows[1][15], "'=HYPERLINK(\"http://x\")");
        assert_eq!(&rows[1][9], "'-2 cm");
        assert_eq!(&rows[0][5], "-74.0142");
        assert!(rows[0][2].ends_with("-04:00") || rows[0][2].ends_with("-05:00"));
        assert!(rows[0][1].ends_with('Z'));
    }

    #[test]
    fn csv_reimports_as_waypoints() {
        let imp = csv_import::parse(export_csv(&book()).as_bytes()).unwrap();
        let pts: Vec<_> = imp
            .waypoints
            .iter()
            .map(|w| (w.latitude, w.longitude))
            .collect();
        assert_eq!(pts, [(40.7003, -74.0142), (41.2108, -73.8625)]);
    }

    #[test]
    fn gpx_reimports_to_the_same_coordinates() {
        let imp = gpx::parse(export_gpx(&book()).as_bytes()).unwrap();
        let pts: Vec<_> = imp
            .waypoints
            .iter()
            .map(|w| (w.latitude, w.longitude))
            .collect();
        assert_eq!(pts, [(40.7003, -74.0142), (41.2108, -73.8625)]);
        assert_eq!(
            imp.waypoints[0].name.as_deref(),
            Some("Striped bass, schoolie")
        );
    }

    #[test]
    fn empty_book_and_xml_hostile_text() {
        assert_eq!(export_csv(&LogBook::new()).lines().count(), 1);
        let g = export_gpx(&LogBook::new());
        assert_eq!(gpx::parse(g.as_bytes()).unwrap().waypoints.len(), 0);
        let mut b = LogBook::new();
        let mut e = entry("x", 0, 1.0, 2.0);
        e.species_text = Some("a&b<c>\u{0}\u{7}".into());
        b.upsert(e);
        let imp = gpx::parse(export_gpx(&b).as_bytes()).unwrap();
        assert_eq!(imp.waypoints[0].name.as_deref(), Some("a&b<c>"));
    }
}
