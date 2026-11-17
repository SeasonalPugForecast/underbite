//! KML 2.2 placemarks and KMZ archives.
//!
//! A `Point` becomes a waypoint and a `LineString` a track. A `MultiGeometry` is split: each of its
//! points and lines is read on its own and carries the placemark's name. A Google `gx:Track` is a
//! track with its `when` and `gx:coord` pairs matched by position, and a `gx:MultiTrack` is one
//! track with a segment per `gx:Track`. Polygons and other geometries are skipped.
//!
//! `Imported` has no field for a group, so each named `Folder` that held something is reported in
//! the notes instead.
//!
//! Elements are matched by local name, so any namespace prefix works. A `DOCTYPE` is refused,
//! which rules out entity definitions; only the five predefined entities and numeric character
//! references are expanded.
//!
//! A KMZ is a zip archive: its `doc.kml`, else the first `.kml` at the root, is read. At most
//! [`MAX_ARCHIVE_ENTRIES`] entries and [`MAX_ARCHIVE_BYTES`] uncompressed bytes are accepted, no
//! entry may name a path outside the archive, and the document is bounded while it is inflated.

use std::io::{Cursor, Read};

use quick_xml::Reader;
use quick_xml::events::Event;
use whether_core::time::UnixTime;
use zip::ZipArchive;

use super::{
    ImportError, Imported, MAX_INPUT_BYTES, MAX_POINTS, Track, TrackPoint, Waypoint,
    assumed_utc_note, check_latitude, check_longitude, check_size, count_newlines, parse_time,
    strip_bom,
};

/// The deepest element nesting accepted.
const MAX_DEPTH: usize = 64;

/// The most entries a KMZ may hold.
pub const MAX_ARCHIVE_ENTRIES: usize = 1000;

/// The most uncompressed bytes a KMZ may hold, over all entries.
pub const MAX_ARCHIVE_BYTES: u64 = 50 * 1024 * 1024;

/// Reads a KMZ archive.
pub fn parse_kmz(bytes: &[u8]) -> Result<Imported, ImportError> {
    check_size(bytes)?;
    let mut zip = ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| kmz_err(format!("not a readable zip archive: {e}")))?;
    if zip.len() > MAX_ARCHIVE_ENTRIES {
        return Err(kmz_err(format!(
            "the archive holds {} entries; the limit is {MAX_ARCHIVE_ENTRIES}",
            zip.len()
        )));
    }
    let mut declared: u64 = 0;
    let mut doc: Option<usize> = None;
    let mut first_kml: Option<usize> = None;
    for i in 0..zip.len() {
        let entry = zip
            .by_index_raw(i)
            .map_err(|e| kmz_err(format!("entry {i} cannot be read: {e}")))?;
        let Some(path) = entry.enclosed_name() else {
            return Err(kmz_err(
                "an entry names a path outside the archive".to_owned(),
            ));
        };
        declared = declared.saturating_add(entry.size());
        if declared > MAX_ARCHIVE_BYTES {
            return Err(ImportError::TooLarge {
                len: usize::try_from(declared).unwrap_or(usize::MAX),
                max: usize::try_from(MAX_ARCHIVE_BYTES).unwrap_or(usize::MAX),
            });
        }
        if entry.is_dir() || path.components().count() != 1 {
            continue;
        }
        if path.as_os_str().eq_ignore_ascii_case("doc.kml") {
            doc.get_or_insert(i);
        } else if path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("kml"))
        {
            first_kml.get_or_insert(i);
        }
    }
    let index = doc
        .or(first_kml)
        .ok_or_else(|| kmz_err("the archive has no KML document at its root".to_owned()))?;
    let entry = zip
        .by_index(index)
        .map_err(|e| kmz_err(format!("the KML document cannot be read: {e}")))?;
    // The header's size may be wrong, so the limit is enforced on what is actually inflated.
    let limit = MAX_INPUT_BYTES as u64 + 1;
    let mut buf = Vec::new();
    entry
        .take(limit)
        .read_to_end(&mut buf)
        .map_err(|e| kmz_err(format!("the KML document cannot be inflated: {e}")))?;
    check_size(&buf)?;
    parse(&buf)
}

/// Reads a KML document.
pub fn parse(bytes: &[u8]) -> Result<Imported, ImportError> {
    check_size(bytes)?;
    let body = strip_bom(bytes);
    let text = std::str::from_utf8(body).map_err(|e| {
        let line = 1 + count_newlines(&body[..e.valid_up_to()]);
        err(line, "the file is not valid UTF-8".to_owned())
    })?;
    let mut state = State::default();
    let mut reader = Reader::from_str(text);
    let mut lines = Lines::default();
    loop {
        let event = reader.read_event();
        let line = lines.at(text, reader.buffer_position());
        let event = event.map_err(|e| err(line, e.to_string()))?;
        match event {
            Event::Start(e) => state.start(e.local_name().into_inner(), line)?,
            Event::Empty(e) => {
                state.start(e.local_name().into_inner(), line)?;
                state.end(line)?;
            }
            Event::End(_) => state.end(line)?,
            Event::Text(t) => state.text(&t.xml10_content()),
            Event::CData(c) => state.text(&c.xml10_content()),
            Event::GeneralRef(r) => {
                let name: &str = &r;
                let resolved = match name {
                    "lt" => '<',
                    "gt" => '>',
                    "amp" => '&',
                    "quot" => '"',
                    "apos" => '\'',
                    _ => match r.resolve_char_ref() {
                        Ok(Some(c)) => c,
                        _ => return Err(err(line, format!("unknown entity `&{name};`"))),
                    },
                };
                let mut buf = [0u8; 4];
                state.text(resolved.encode_utf8(&mut buf));
            }
            Event::DocType(_) => {
                return Err(err(line, "a DOCTYPE is not accepted".to_owned()));
            }
            Event::Eof => break,
            Event::Comment(_) | Event::Decl(_) | Event::PI(_) => {}
        }
    }
    let line = lines.at(text, reader.buffer_position());
    if !state.saw_root {
        return Err(err(line, "no <kml> element".to_owned()));
    }
    if !state.stack.is_empty() {
        return Err(err(line, "the file ends inside an open element".to_owned()));
    }
    let mut out = state.out;
    if state.assumed_utc > 0 {
        out.notes.push(assumed_utc_note(state.assumed_utc));
    }
    Ok(out)
}

fn err(line: u64, message: String) -> ImportError {
    ImportError::Parse {
        format: "KML",
        line,
        message,
    }
}

fn kmz_err(message: String) -> ImportError {
    ImportError::Parse {
        format: "KMZ",
        line: 0,
        message,
    }
}

/// Counts lines incrementally; positions only move forward.
#[derive(Default)]
struct Lines {
    pos: usize,
    line: u64,
}

impl Lines {
    fn at(&mut self, text: &str, pos: u64) -> u64 {
        let end = usize::try_from(pos).unwrap_or(usize::MAX).min(text.len());
        if end > self.pos {
            self.line += count_newlines(&text.as_bytes()[self.pos..end]);
            self.pos = end;
        }
        self.line + 1
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Name,
    Description,
    Stamp,
    Begin,
    FolderName,
    Coordinates { in_point: bool },
    TrackWhen,
    TrackCoord,
}

/// A position: latitude, longitude and elevation.
type Position = (f64, f64, Option<f64>);

enum Item {
    Point(Position),
    /// Segments of one track.
    Track(Vec<Vec<TrackPoint>>),
}

#[derive(Default)]
struct Placemark {
    name: String,
    description: String,
    stamp: Option<UnixTime>,
    begin: Option<UnixTime>,
    items: Vec<Item>,
}

#[derive(Default)]
struct GxTrack {
    whens: Vec<Option<UnixTime>>,
    coords: Vec<Option<Position>>,
}

#[derive(Default)]
struct Frame {
    name: String,
    waypoints: usize,
    tracks: usize,
}

#[derive(Default)]
struct State {
    stack: Vec<String>,
    saw_root: bool,
    points: usize,
    assumed_utc: usize,
    placemark: Option<Placemark>,
    gx_track: Option<GxTrack>,
    in_multi_track: bool,
    folders: Vec<Frame>,
    /// Depth of the element being captured and what it is.
    capture: Option<(usize, Field)>,
    buf: String,
    out: Imported,
}

fn non_empty(s: &str) -> Option<String> {
    let t = s.trim();
    (!t.is_empty()).then(|| t.to_owned())
}

impl State {
    fn parent(&self) -> &str {
        let n = self.stack.len();
        if n >= 2 { &self.stack[n - 2] } else { "" }
    }

    fn start(&mut self, name: &str, line: u64) -> Result<(), ImportError> {
        if self.stack.len() >= MAX_DEPTH {
            return Err(err(line, "elements are nested too deeply".to_owned()));
        }
        if self.stack.is_empty() {
            if self.saw_root {
                return Err(err(line, "more than one root element".to_owned()));
            }
            if name != "kml" {
                return Err(err(
                    line,
                    format!("the root element is <{name}>, not <kml>"),
                ));
            }
            self.saw_root = true;
        }
        self.stack.push(name.to_owned());
        let depth = self.stack.len();
        let parent = self.parent().to_owned();
        let mut field = None;
        match name {
            "Folder" => self.folders.push(Frame::default()),
            "Placemark" => {
                if self.placemark.is_some() {
                    return Err(err(line, "a Placemark is nested in a Placemark".to_owned()));
                }
                self.placemark = Some(Placemark::default());
            }
            "name" if parent == "Folder" => field = Some(Field::FolderName),
            _ if self.placemark.is_none() => {}
            "name" if parent == "Placemark" => field = Some(Field::Name),
            "description" if parent == "Placemark" => field = Some(Field::Description),
            "when" if parent == "TimeStamp" => field = Some(Field::Stamp),
            "begin" if parent == "TimeSpan" => field = Some(Field::Begin),
            "coordinates" if parent == "Point" => {
                field = Some(Field::Coordinates { in_point: true });
            }
            "coordinates" if parent == "LineString" => {
                field = Some(Field::Coordinates { in_point: false });
            }
            "MultiTrack" => {
                self.in_multi_track = true;
                if let Some(p) = self.placemark.as_mut() {
                    p.items.push(Item::Track(Vec::new()));
                }
            }
            "Track" => self.gx_track = Some(GxTrack::default()),
            "when" if parent == "Track" => field = Some(Field::TrackWhen),
            "coord" if parent == "Track" => field = Some(Field::TrackCoord),
            _ => {}
        }
        if let Some(f) = field {
            self.capture = Some((depth, f));
            self.buf.clear();
        }
        Ok(())
    }

    fn text(&mut self, s: &str) {
        if self.capture.is_some() && self.buf.len() <= MAX_INPUT_BYTES {
            self.buf.push_str(s);
        }
    }

    fn end(&mut self, line: u64) -> Result<(), ImportError> {
        let depth = self.stack.len();
        if let Some((d, field)) = self.capture
            && d == depth
        {
            self.capture = None;
            let text = std::mem::take(&mut self.buf);
            self.assign(field, &text, line)?;
        }
        let name = self.stack.pop().unwrap_or_default();
        match name.as_str() {
            "Placemark" => self.finish_placemark(),
            "Folder" => {
                if let Some(f) = self.folders.pop()
                    && let Some(label) = non_empty(&f.name)
                    && f.waypoints + f.tracks > 0
                {
                    self.out.notes.push(format!(
                        "Folder \"{label}\": {} {}, {} {}.",
                        f.waypoints,
                        if f.waypoints == 1 {
                            "waypoint"
                        } else {
                            "waypoints"
                        },
                        f.tracks,
                        if f.tracks == 1 { "track" } else { "tracks" },
                    ));
                }
            }
            "MultiTrack" => self.in_multi_track = false,
            "Track" => {
                if let (Some(t), Some(p)) = (self.gx_track.take(), self.placemark.as_mut()) {
                    let segment: Vec<TrackPoint> = t
                        .coords
                        .iter()
                        .enumerate()
                        .filter_map(|(i, c)| {
                            c.map(|(latitude, longitude, elevation_m)| TrackPoint {
                                latitude,
                                longitude,
                                elevation_m,
                                time: t.whens.get(i).copied().flatten(),
                            })
                        })
                        .collect();
                    if self.in_multi_track
                        && let Some(Item::Track(segments)) = p.items.last_mut()
                    {
                        segments.push(segment);
                    } else {
                        p.items.push(Item::Track(vec![segment]));
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn finish_placemark(&mut self) {
        let Some(p) = self.placemark.take() else {
            return;
        };
        let name = non_empty(&p.name);
        let time = p.stamp.or(p.begin);
        let notes = non_empty(&p.description);
        let mut waypoints = 0;
        let mut tracks = 0;
        for item in p.items {
            match item {
                Item::Point((latitude, longitude, elevation_m)) => {
                    waypoints += 1;
                    self.out.waypoints.push(Waypoint {
                        name: name.clone(),
                        latitude,
                        longitude,
                        elevation_m,
                        time,
                        notes: notes.clone(),
                    });
                }
                Item::Track(segments) => {
                    let segments: Vec<_> = segments.into_iter().filter(|s| !s.is_empty()).collect();
                    if !segments.is_empty() {
                        tracks += 1;
                        self.out.tracks.push(Track {
                            name: name.clone(),
                            segments,
                        });
                    }
                }
            }
        }
        if let Some(f) = self.folders.last_mut() {
            f.waypoints += waypoints;
            f.tracks += tracks;
        }
    }

    fn assign(&mut self, field: Field, text: &str, line: u64) -> Result<(), ImportError> {
        match field {
            Field::FolderName => {
                if let Some(f) = self.folders.last_mut() {
                    text.trim().clone_into(&mut f.name);
                }
            }
            Field::Name => {
                if let Some(p) = self.placemark.as_mut() {
                    text.clone_into(&mut p.name);
                }
            }
            Field::Description => {
                if let Some(p) = self.placemark.as_mut() {
                    text.clone_into(&mut p.description);
                }
            }
            Field::Stamp | Field::Begin | Field::TrackWhen => {
                let time = self.time(text, line)?;
                match field {
                    Field::Stamp => {
                        if let Some(p) = self.placemark.as_mut() {
                            p.stamp = time;
                        }
                    }
                    Field::Begin => {
                        if let Some(p) = self.placemark.as_mut() {
                            p.begin = time;
                        }
                    }
                    _ => {
                        if let Some(t) = self.gx_track.as_mut() {
                            t.whens.push(time);
                        }
                    }
                }
            }
            Field::Coordinates { in_point } => {
                let tuples = coordinate_list(text, line, &mut self.points)?;
                let Some(p) = self.placemark.as_mut() else {
                    return Ok(());
                };
                if in_point {
                    let first = tuples
                        .first()
                        .ok_or_else(|| err(line, "a Point has no coordinates".to_owned()))?;
                    p.items.push(Item::Point(*first));
                } else if !tuples.is_empty() {
                    let segment = tuples
                        .into_iter()
                        .map(|(latitude, longitude, elevation_m)| TrackPoint {
                            latitude,
                            longitude,
                            elevation_m,
                            time: None,
                        })
                        .collect();
                    p.items.push(Item::Track(vec![segment]));
                }
            }
            Field::TrackCoord => {
                let position = if text.trim().is_empty() {
                    None
                } else {
                    self.points += 1;
                    if self.points > MAX_POINTS {
                        return Err(ImportError::TooManyPoints { max: MAX_POINTS });
                    }
                    let parts: Vec<&str> = text.split_whitespace().collect();
                    Some(position(&parts, line)?)
                };
                if let Some(t) = self.gx_track.as_mut() {
                    t.coords.push(position);
                }
            }
        }
        Ok(())
    }

    /// An empty time is `None`; a time without an offset counts towards the note.
    fn time(&mut self, text: &str, line: u64) -> Result<Option<UnixTime>, ImportError> {
        let t = text.trim();
        if t.is_empty() {
            return Ok(None);
        }
        // KML allows a bare date.
        let full;
        let t = if t.len() == 10 && t.as_bytes()[4] == b'-' && t.as_bytes()[7] == b'-' {
            full = format!("{t}T00:00:00");
            full.as_str()
        } else {
            t
        };
        let (time, assumed) = parse_time(t).map_err(|m| err(line, m))?;
        if assumed {
            self.assumed_utc += 1;
        }
        Ok(Some(time))
    }
}

/// `lon,lat[,alt]` tuples separated by whitespace. Spaces around the commas are tolerated.
fn coordinate_list(
    text: &str,
    line: u64,
    points: &mut usize,
) -> Result<Vec<Position>, ImportError> {
    let tight = text.split(',').map(str::trim).collect::<Vec<_>>().join(",");
    let mut out = Vec::new();
    for tuple in tight.split_whitespace() {
        *points += 1;
        if *points > MAX_POINTS {
            return Err(ImportError::TooManyPoints { max: MAX_POINTS });
        }
        let parts: Vec<&str> = tuple.split(',').collect();
        out.push(position(&parts, line)?);
    }
    Ok(out)
}

/// `[lon, lat]` or `[lon, lat, alt]` as text, longitude first.
fn position(parts: &[&str], line: u64) -> Result<Position, ImportError> {
    if !(2..=3).contains(&parts.len()) {
        return Err(err(
            line,
            format!("a coordinate has {} parts, not 2 or 3", parts.len()),
        ));
    }
    let number = |s: &str, what: &str| -> Result<f64, ImportError> {
        s.parse::<f64>()
            .ok()
            .filter(|v| v.is_finite())
            .ok_or_else(|| err(line, format!("`{s}` is not a number for {what}")))
    };
    let lon = check_longitude(number(parts[0], "longitude")?).map_err(|m| err(line, m))?;
    let lat = check_latitude(number(parts[1], "latitude")?).map_err(|m| err(line, m))?;
    let ele = match parts.get(2) {
        Some(s) => Some(number(s, "altitude")?),
        None => None,
    };
    Ok((lat, lon, ele))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::{Format, gpx, import};

    const BATTERY: &[u8] =
        include_bytes!("../../../../tests/fixtures/raw/import/battery.synthetic.kml");
    const CROTON: &[u8] =
        include_bytes!("../../../../tests/fixtures/raw/import/croton.synthetic.kml");
    const FOLDERS: &[u8] =
        include_bytes!("../../../../tests/fixtures/raw/import/folders.synthetic.kml");
    const BATTERY_GPX: &[u8] =
        include_bytes!("../../../../tests/fixtures/raw/import/battery.synthetic.gpx");
    const CROTON_GPX: &[u8] =
        include_bytes!("../../../../tests/fixtures/raw/import/croton.synthetic.gpx");
    const BATTERY_KMZ: &[u8] =
        include_bytes!("../../../../tests/fixtures/raw/import/battery.synthetic.kmz");
    const CROTON_KMZ: &[u8] =
        include_bytes!("../../../../tests/fixtures/raw/import/croton.synthetic.kmz");
    const BOMB_KMZ: &[u8] =
        include_bytes!("../../../../tests/fixtures/raw/import/bomb.synthetic.kmz");
    const ESCAPE_KMZ: &[u8] =
        include_bytes!("../../../../tests/fixtures/raw/import/escape.synthetic.kmz");

    fn zip_of(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
        use std::io::Write;
        let mut out = Vec::new();
        {
            let mut w = zip::ZipWriter::new(Cursor::new(&mut out));
            for (name, data) in entries {
                w.start_file(*name, zip::write::SimpleFileOptions::default())
                    .expect("start");
                w.write_all(data).expect("write");
            }
            w.finish().expect("finish");
        }
        out
    }

    fn kml(body: &str) -> String {
        format!("<kml xmlns=\"http://www.opengis.net/kml/2.2\"><Document>{body}</Document></kml>")
    }

    #[test]
    fn battery_equals_the_gpx() {
        let k = parse(BATTERY).expect("kml");
        assert_eq!(k, gpx::parse(BATTERY_GPX).expect("gpx"));
        assert_eq!(k.waypoints[0].elevation_m, Some(2.5));
        assert_eq!(k.waypoints[1].name.as_deref(), Some("Slip <2>"));
    }

    #[test]
    fn croton_equals_the_gpx() {
        let k = parse(CROTON).expect("kml");
        assert_eq!(k, gpx::parse(CROTON_GPX).expect("gpx"));
    }

    #[test]
    fn multigeometry_folders_and_dates() {
        let imp = parse(FOLDERS).expect("parse");
        let names: Vec<_> = imp.waypoints.iter().map(|w| w.name.as_deref()).collect();
        assert_eq!(
            names,
            [
                Some("Mixed"),
                Some("Mixed"),
                Some("Date only"),
                Some("Loose")
            ]
        );
        assert_eq!(imp.tracks.len(), 1);
        assert_eq!(imp.tracks[0].name.as_deref(), Some("Mixed"));
        assert_eq!(imp.tracks[0].point_count(), 3);
        assert_eq!(imp.waypoints[2].time, Some(UnixTime(1_748_822_400)));
        assert_eq!(
            imp.notes,
            vec![
                "Folder \"Shore\": 1 waypoint, 0 tracks.",
                "Folder \"Harbor\": 2 waypoints, 1 track.",
                "1 time had no UTC offset and was read as UTC.",
            ]
        );
    }

    #[test]
    fn coordinates_tolerate_whitespace_and_longitude_comes_first() {
        let doc = kml(
            "<Placemark><LineString><coordinates>\n\t-74.0 ,\t40.5 , 3\n  -73.5,41\t</coordinates></LineString></Placemark>",
        );
        let imp = parse(doc.as_bytes()).expect("parse");
        let seg = &imp.tracks[0].segments[0];
        assert_eq!(seg.len(), 2);
        assert_eq!(
            (seg[0].latitude, seg[0].longitude, seg[0].elevation_m),
            (40.5, -74.0, Some(3.0))
        );
        assert_eq!(
            (seg[1].latitude, seg[1].longitude, seg[1].elevation_m),
            (41.0, -73.5, None)
        );
    }

    #[test]
    fn gx_track_without_matching_times() {
        let doc = kml(
            "<Placemark><gx:Track xmlns:gx=\"urn:gx\"><when>2025-05-10T13:00:00Z</when><gx:coord>-73.8 41.2</gx:coord><gx:coord>-73.9 41.3</gx:coord></gx:Track></Placemark>",
        );
        let imp = parse(doc.as_bytes()).expect("parse");
        let seg = &imp.tracks[0].segments[0];
        assert!(seg[0].time.is_some());
        assert_eq!(seg[1].time, None);
    }

    #[test]
    fn errors_carry_a_line() {
        let doc = kml("\n<Placemark><Point><coordinates>-74,91</coordinates></Point></Placemark>");
        match parse(doc.as_bytes()) {
            Err(ImportError::Parse {
                format: "KML",
                line: 2,
                message,
            }) => {
                assert!(message.contains("latitude"), "{message}");
            }
            other => panic!("{other:?}"),
        }
        assert!(parse(b"").is_err());
        assert!(parse(b"<gpx/>").is_err());
        assert!(
            parse(
                kml("<Placemark><Point><coordinates>1</coordinates></Point></Placemark>")
                    .as_bytes()
            )
            .is_err()
        );
        assert!(
            parse(
                kml("<Placemark><Point><coordinates>a,b</coordinates></Point></Placemark>")
                    .as_bytes()
            )
            .is_err()
        );
        assert!(
            parse(
                kml("<Placemark><Point><coordinates> </coordinates></Point></Placemark>")
                    .as_bytes()
            )
            .is_err()
        );
        assert!(
            parse(
                kml("<Placemark><TimeStamp><when>soon</when></TimeStamp></Placemark>").as_bytes()
            )
            .is_err()
        );
        assert!(parse(b"<kml><Document>").is_err());
        assert!(parse(b"<kml/><kml/>").is_err());
        assert!(parse(b"\xFF\xFE<kml/>").is_err());
    }

    #[test]
    fn doctype_and_entities_are_refused() {
        let doc = "<!DOCTYPE kml [<!ENTITY x SYSTEM \"file:///etc/passwd\">]><kml><Document><name>&x;</name></Document></kml>";
        assert!(matches!(
            parse(doc.as_bytes()),
            Err(ImportError::Parse { .. })
        ));
        let doc = kml("<name>&bogus;</name>");
        assert!(parse(doc.as_bytes()).is_err());
    }

    #[test]
    fn deep_nesting_is_refused() {
        let mut doc = String::from("<kml>");
        for _ in 0..100 {
            doc.push_str("<Folder>");
        }
        assert!(parse(doc.as_bytes()).is_err());
    }

    #[test]
    fn truncations_never_panic() {
        for input in [BATTERY, CROTON, FOLDERS] {
            for cut in (0..input.len()).step_by(7) {
                let _ = parse(&input[..cut]);
            }
        }
        for cut in (0..BATTERY_KMZ.len()).step_by(5) {
            let _ = parse_kmz(&BATTERY_KMZ[..cut]);
        }
    }

    #[test]
    fn point_limit_is_enforced_while_reading() {
        let tuple = "-74,40 ";
        let doc = kml(&format!(
            "<Placemark><LineString><coordinates>{}</coordinates></LineString></Placemark>",
            tuple.repeat(MAX_POINTS + 1)
        ));
        assert_eq!(
            parse(doc.as_bytes()),
            Err(ImportError::TooManyPoints { max: MAX_POINTS })
        );
    }

    #[test]
    fn kmz_reads_doc_kml() {
        let imp = parse_kmz(BATTERY_KMZ).expect("kmz");
        assert_eq!(imp, parse(BATTERY).expect("kml"));
    }

    #[test]
    fn kmz_without_doc_uses_the_first_root_kml() {
        let imp = parse_kmz(CROTON_KMZ).expect("kmz");
        assert_eq!(imp, parse(CROTON).expect("kml"));
    }

    #[test]
    fn kmz_is_found_by_content() {
        assert_eq!(crate::import::detect(BATTERY_KMZ, "x"), Some(Format::Kmz));
        assert_eq!(
            import(BATTERY_KMZ, "spots.kmz").expect("import"),
            import(BATTERY, "spots.kml").expect("import")
        );
        assert_eq!(
            import(CROTON, "croton.kml").expect("import"),
            import(CROTON_GPX, "croton.gpx").expect("import")
        );
    }

    #[test]
    fn zip_bomb_is_refused_while_inflating() {
        assert!(BOMB_KMZ.len() < 100_000);
        assert!(matches!(
            parse_kmz(BOMB_KMZ),
            Err(ImportError::TooLarge { max, .. }) if max == MAX_INPUT_BYTES
        ));
    }

    #[test]
    fn archive_total_is_limited() {
        let big = vec![b' '; 20 * 1024 * 1024];
        let zip = zip_of(&[
            ("a.bin", big.clone()),
            ("b.bin", big.clone()),
            ("c.bin", big),
            ("doc.kml", BATTERY.to_vec()),
        ]);
        assert!(matches!(parse_kmz(&zip), Err(ImportError::TooLarge { .. })));
    }

    #[test]
    fn archive_entry_count_is_limited() {
        let entries: Vec<(String, Vec<u8>)> = (0..=MAX_ARCHIVE_ENTRIES)
            .map(|i| (format!("f{i}.txt"), Vec::new()))
            .collect();
        let refs: Vec<(&str, Vec<u8>)> = entries
            .iter()
            .map(|(n, d)| (n.as_str(), d.clone()))
            .collect();
        let zip = zip_of(&refs);
        match parse_kmz(&zip) {
            Err(ImportError::Parse {
                format: "KMZ",
                message,
                ..
            }) => {
                assert!(message.contains("entries"), "{message}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn archive_paths_outside_are_refused() {
        match parse_kmz(ESCAPE_KMZ) {
            Err(ImportError::Parse {
                format: "KMZ",
                message,
                ..
            }) => {
                assert!(message.contains("outside"), "{message}");
            }
            other => panic!("{other:?}"),
        }
        let zip = zip_of(&[("/abs/doc.kml", BATTERY.to_vec())]);
        assert!(parse_kmz(&zip).is_err());
    }

    #[test]
    fn archive_without_a_root_document() {
        let zip = zip_of(&[("sub/doc.kml", BATTERY.to_vec()), ("x.txt", b"x".to_vec())]);
        assert!(matches!(
            parse_kmz(&zip),
            Err(ImportError::Parse { format: "KMZ", .. })
        ));
        assert!(matches!(
            parse_kmz(b"PK\x03\x04junk"),
            Err(ImportError::Parse { format: "KMZ", .. })
        ));
    }
}
