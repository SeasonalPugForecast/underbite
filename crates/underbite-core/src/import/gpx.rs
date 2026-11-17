//! GPX 1.0 and 1.1: waypoints, routes and tracks.
//!
//! Elements are matched by local name, so any namespace prefix works. `extensions` are skipped
//! whole. A `DOCTYPE` is refused, which rules out external and internal entity definitions; only
//! the five predefined entities and numeric character references are expanded. A route is read as
//! a one-segment track.

use quick_xml::Reader;
use quick_xml::XmlVersion;
use quick_xml::events::{BytesStart, Event};
use whether_core::time::UnixTime;

use super::{
    ImportError, Imported, MAX_POINTS, Track, TrackPoint, Waypoint, assumed_utc_note,
    check_latitude, check_longitude, check_size, count_newlines, parse_time, strip_bom,
};

/// The deepest element nesting accepted.
const MAX_DEPTH: usize = 64;

/// Reads a GPX document.
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
            Event::Start(e) => state.start(&e, line)?,
            Event::Empty(e) => {
                state.start(&e, line)?;
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
    if !state.saw_root {
        return Err(err(
            lines.at(text, reader.buffer_position()),
            "no <gpx> element".to_owned(),
        ));
    }
    if !state.stack.is_empty() {
        return Err(err(
            lines.at(text, reader.buffer_position()),
            "the file ends inside an open element".to_owned(),
        ));
    }
    let mut out = state.out;
    if state.assumed_utc > 0 {
        out.notes.push(assumed_utc_note(state.assumed_utc));
    }
    Ok(out)
}

fn err(line: u64, message: String) -> ImportError {
    ImportError::Parse {
        format: "GPX",
        line,
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
    Ele,
    Time,
    Desc,
    Cmt,
    TrackName,
}

#[derive(Default)]
struct PointBuilder {
    lat: f64,
    lon: f64,
    name: String,
    ele: Option<f64>,
    time: Option<UnixTime>,
    desc: String,
    cmt: String,
}

#[derive(Default)]
struct State {
    stack: Vec<String>,
    saw_root: bool,
    points: usize,
    assumed_utc: usize,
    point: Option<PointBuilder>,
    track_name: String,
    segments: Vec<Vec<TrackPoint>>,
    segment: Vec<TrackPoint>,
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
    fn path(&self) -> Vec<&str> {
        self.stack.iter().map(String::as_str).collect()
    }

    fn start(&mut self, e: &BytesStart<'_>, line: u64) -> Result<(), ImportError> {
        if self.stack.len() >= MAX_DEPTH {
            return Err(err(line, "elements are nested too deeply".to_owned()));
        }
        let name = e.local_name().into_inner().to_owned();
        if self.stack.is_empty() {
            if self.saw_root {
                return Err(err(line, "more than one root element".to_owned()));
            }
            if name != "gpx" {
                return Err(err(
                    line,
                    format!("the root element is <{name}>, not <gpx>"),
                ));
            }
            self.saw_root = true;
        }
        self.stack.push(name);
        let path = self.path();
        let (is_point, field) = match path.as_slice() {
            ["gpx", "wpt"] | ["gpx", "trk", "trkseg", "trkpt"] | ["gpx", "rte", "rtept"] => {
                (true, None)
            }
            ["gpx", "wpt", "name"] => (false, Some(Field::Name)),
            ["gpx", "wpt", "ele"]
            | ["gpx", "trk", "trkseg", "trkpt", "ele"]
            | ["gpx", "rte", "rtept", "ele"] => (false, Some(Field::Ele)),
            ["gpx", "wpt", "time"]
            | ["gpx", "trk", "trkseg", "trkpt", "time"]
            | ["gpx", "rte", "rtept", "time"] => (false, Some(Field::Time)),
            ["gpx", "wpt", "desc"] => (false, Some(Field::Desc)),
            ["gpx", "wpt", "cmt"] => (false, Some(Field::Cmt)),
            ["gpx", "trk" | "rte", "name"] => (false, Some(Field::TrackName)),
            _ => (false, None),
        };
        let is_track = matches!(path.as_slice(), ["gpx", "trk" | "rte"]);
        let is_seg = matches!(path.as_slice(), ["gpx", "trk", "trkseg"]);
        let depth = self.stack.len();
        if is_track {
            self.track_name.clear();
            self.segments.clear();
            self.segment.clear();
        }
        if is_seg {
            self.segment.clear();
        }
        if is_point {
            self.points += 1;
            if self.points > MAX_POINTS {
                return Err(ImportError::TooManyPoints { max: MAX_POINTS });
            }
            let lat = coordinate(e, "lat", line, check_latitude)?;
            let lon = coordinate(e, "lon", line, check_longitude)?;
            self.point = Some(PointBuilder {
                lat,
                lon,
                ..PointBuilder::default()
            });
        }
        if let Some(f) = field {
            self.capture = Some((depth, f));
            self.buf.clear();
        }
        Ok(())
    }

    fn text(&mut self, s: &str) {
        if self.capture.is_some() && self.buf.len() < 1 << 20 {
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
        let path = self.path();
        let kind = match path.as_slice() {
            ["gpx", "wpt"] => 1,
            ["gpx", "trk", "trkseg", "trkpt"] | ["gpx", "rte", "rtept"] => 2,
            ["gpx", "trk", "trkseg"] => 3,
            ["gpx", "trk"] => 4,
            ["gpx", "rte"] => 5,
            _ => 0,
        };
        match kind {
            1 => {
                if let Some(p) = self.point.take() {
                    self.out.waypoints.push(Waypoint {
                        name: non_empty(&p.name),
                        latitude: p.lat,
                        longitude: p.lon,
                        elevation_m: p.ele,
                        time: p.time,
                        notes: join_notes(&p.desc, &p.cmt),
                    });
                }
            }
            2 => {
                if let Some(p) = self.point.take() {
                    self.segment.push(TrackPoint {
                        latitude: p.lat,
                        longitude: p.lon,
                        elevation_m: p.ele,
                        time: p.time,
                    });
                }
            }
            3 => {
                if !self.segment.is_empty() {
                    self.segments.push(std::mem::take(&mut self.segment));
                }
            }
            4 | 5 => {
                if kind == 5 && !self.segment.is_empty() {
                    self.segments.push(std::mem::take(&mut self.segment));
                }
                if !self.segments.is_empty() {
                    self.out.tracks.push(Track {
                        name: non_empty(&self.track_name),
                        segments: std::mem::take(&mut self.segments),
                    });
                }
            }
            _ => {}
        }
        self.stack.pop();
        Ok(())
    }

    fn assign(&mut self, field: Field, text: &str, line: u64) -> Result<(), ImportError> {
        if field == Field::TrackName {
            text.clone_into(&mut self.track_name);
            return Ok(());
        }
        let Some(p) = self.point.as_mut() else {
            return Ok(());
        };
        match field {
            Field::Name => text.clone_into(&mut p.name),
            Field::Desc => text.clone_into(&mut p.desc),
            Field::Cmt => text.clone_into(&mut p.cmt),
            Field::Ele => {
                let t = text.trim();
                if !t.is_empty() {
                    let v: f64 = t
                        .parse()
                        .ok()
                        .filter(|v: &f64| v.is_finite())
                        .ok_or_else(|| err(line, format!("`{t}` is not an elevation")))?;
                    p.ele = Some(v);
                }
            }
            Field::Time => {
                let t = text.trim();
                if !t.is_empty() {
                    let (time, assumed) = parse_time(t).map_err(|m| err(line, m))?;
                    if assumed {
                        self.assumed_utc += 1;
                    }
                    p.time = Some(time);
                }
            }
            Field::TrackName => {}
        }
        Ok(())
    }
}

fn join_notes(desc: &str, cmt: &str) -> Option<String> {
    match (non_empty(desc), non_empty(cmt)) {
        (Some(d), Some(c)) if d != c => Some(format!("{d}\n{c}")),
        (Some(d), _) => Some(d),
        (None, c) => c,
    }
}

fn coordinate(
    e: &BytesStart<'_>,
    attr: &str,
    line: u64,
    check: fn(f64) -> Result<f64, String>,
) -> Result<f64, ImportError> {
    // Walk the attributes with duplicate checks on, so a repeated attribute is an error.
    let mut found = None;
    for a in e.attributes() {
        let a = a.map_err(|x| err(line, x.to_string()))?;
        if a.key.as_ref() == attr {
            found = Some(a);
        }
    }
    let a = found.ok_or_else(|| err(line, format!("a point has no `{attr}` attribute")))?;
    let value = a
        .normalized_value(XmlVersion::Implicit1_0)
        .map_err(|x| err(line, x.to_string()))?;
    let v: f64 = value
        .trim()
        .parse()
        .map_err(|_| err(line, format!("`{value}` is not a number for `{attr}`")))?;
    check(v).map_err(|m| err(line, m))
}

#[cfg(test)]
mod tests {
    use super::*;

    const BATTERY: &[u8] =
        include_bytes!("../../../../tests/fixtures/raw/import/battery.synthetic.gpx");
    const CROTON: &[u8] =
        include_bytes!("../../../../tests/fixtures/raw/import/croton.synthetic.gpx");

    #[test]
    fn waypoint_at_the_battery() {
        let imp = parse(BATTERY).expect("parse");
        assert_eq!(imp.waypoints.len(), 2);
        let w = &imp.waypoints[0];
        assert_eq!(w.name.as_deref(), Some("The Battery & pier"));
        assert_eq!((w.latitude, w.longitude), (40.7033, -74.0170));
        assert_eq!(w.elevation_m, Some(2.5));
        assert_eq!(w.time, Some(UnixTime(1_748_779_200)));
        assert_eq!(
            w.notes.as_deref(),
            Some("Stripers on the push\nBring a net")
        );
        let w2 = &imp.waypoints[1];
        assert_eq!(w2.name.as_deref(), Some("Slip <2>"));
        assert_eq!(w2.time, Some(UnixTime(1_748_779_200)));
        assert_eq!(
            imp.notes,
            vec!["1 time had no UTC offset and was read as UTC."]
        );
        assert_eq!(imp.tracks.len(), 0);
    }

    #[test]
    fn track_along_the_croton() {
        let imp = parse(CROTON).expect("parse");
        assert_eq!(imp.waypoints.len(), 0);
        assert_eq!(imp.tracks.len(), 2);
        let t = &imp.tracks[0];
        assert_eq!(t.name.as_deref(), Some("Croton River walk"));
        assert_eq!(t.segments.len(), 2);
        assert_eq!(t.segments[0].len(), 3);
        assert_eq!(t.segments[1].len(), 2);
        let p = &t.segments[0][0];
        assert!((p.latitude - 41.2108).abs() < 1e-9);
        assert!((p.longitude + 73.8625).abs() < 1e-9);
        assert_eq!(p.elevation_m, Some(31.0));
        assert!(p.time.is_some());
        let r = &imp.tracks[1];
        assert_eq!(r.name.as_deref(), Some("Route to the dam"));
        assert_eq!(r.segments.len(), 1);
        assert_eq!(r.point_count(), 2);
        assert_eq!(imp.notes.len(), 0);
    }

    #[test]
    fn gpx_10_without_prefix_or_namespace_and_cdata() {
        let doc = br#"<?xml version="1.0"?>
<gpx version="1.0" creator="x" xmlns="http://www.topografix.com/GPX/1/0">
<wpt lat="40.7033" lon="-74.0170"><name><![CDATA[A & B]]></name><desc>d</desc></wpt>
</gpx>"#;
        let imp = parse(doc).expect("parse");
        assert_eq!(imp.waypoints[0].name.as_deref(), Some("A & B"));
        assert_eq!(imp.waypoints[0].notes.as_deref(), Some("d"));
    }

    #[test]
    fn prefixed_namespace_and_extensions_ignored() {
        let doc = br#"<g:gpx xmlns:g="http://www.topografix.com/GPX/1/1" xmlns:x="urn:v">
<g:wpt lat="1" lon="2"><g:name>ok</g:name><g:extensions><x:name>no</x:name><x:ele>junk</x:ele></g:extensions></g:wpt>
</g:gpx>"#;
        let imp = parse(doc).expect("parse");
        assert_eq!(imp.waypoints[0].name.as_deref(), Some("ok"));
    }

    #[test]
    fn self_closing_point_and_bom() {
        let mut doc = b"\xEF\xBB\xBF".to_vec();
        doc.extend_from_slice(b"<gpx><wpt lat=\"10\" lon=\"20\"/></gpx>");
        let imp = parse(&doc).expect("parse");
        assert_eq!(imp.waypoints.len(), 1);
        assert_eq!(imp.waypoints[0].name, None);
    }

    fn line_of(doc: &[u8]) -> u64 {
        match parse(doc) {
            Err(ImportError::Parse { line, .. }) => line,
            other => panic!("expected a parse error, got {other:?}"),
        }
    }

    #[test]
    fn errors_carry_a_line() {
        let doc = b"<gpx>\n<wpt lat=\"91\" lon=\"0\"/>\n</gpx>";
        assert_eq!(line_of(doc), 2);
        let doc = b"<gpx>\n\n<wpt lat=\"1\" lon=\"2\"><time>noon</time></wpt>\n</gpx>";
        assert_eq!(line_of(doc), 3);
        let doc = b"<gpx>\n<wpt lat=\"1,5\" lon=\"2\"/></gpx>";
        assert_eq!(line_of(doc), 2);
        let doc = b"<gpx>\n<wpt lon=\"2\"/></gpx>";
        assert_eq!(line_of(doc), 2);
    }

    #[test]
    fn malformed_inputs_are_errors_not_panics() {
        let cases: &[&[u8]] = &[
            b"",
            b"   ",
            b"<",
            b"<gpx",
            b"<gpx>",
            b"<gpx><wpt lat=\"1\" lon=\"2\">",
            b"<gpx></wpt></gpx>",
            b"<gpx><wpt lat=\"1\" lon=\"2\"></gpx>",
            b"<kml/>",
            b"<gpx/><gpx/>",
            b"<gpx><wpt lat=\"nan\" lon=\"0\"/></gpx>",
            b"<gpx><wpt lat=\"inf\" lon=\"0\"/></gpx>",
            b"<gpx><wpt lat=\"1\" lon=\"2\"><ele>high</ele></wpt></gpx>",
            b"<gpx><wpt lat=\"1\" lon=\"2\"><time>2025-13-45T99:00:00Z</time></wpt></gpx>",
            b"<gpx><wpt lat=\"1\" lon=\"2\"><name>&bogus;</name></wpt></gpx>",
            b"<gpx><wpt lat=\"1\" lon=\"2\"><name>&#xFFFFFFFF;</name></wpt></gpx>",
            b"<gpx><wpt lat='1' lon='2' lat='3'/></gpx>",
            b"<gpx><wpt lat=1 lon=2/></gpx>",
            b"<!-- unterminated",
            b"<![CDATA[ x",
            b"\xFF\xFE<\x00g\x00",
            b"<gpx>\xC3\x28</gpx>",
            b"<?xml version=\"1.0\"?><!DOCTYPE gpx [<!ENTITY x SYSTEM \"file:///etc/passwd\">]><gpx><wpt lat=\"1\" lon=\"2\"><name>&x;</name></wpt></gpx>",
            b"<!DOCTYPE lolz [<!ENTITY a \"aaaa\"><!ENTITY b \"&a;&a;&a;&a;\">]><gpx><wpt lat=\"1\" lon=\"2\"><name>&b;</name></wpt></gpx>",
        ];
        for (i, case) in cases.iter().enumerate() {
            assert!(parse(case).is_err(), "case {i} should be an error");
        }
    }

    #[test]
    fn deep_nesting_is_refused() {
        let mut doc = String::from("<gpx>");
        for _ in 0..200 {
            doc.push_str("<a>");
        }
        assert!(parse(doc.as_bytes()).is_err());
    }

    #[test]
    fn truncations_never_panic() {
        for end in 0..BATTERY.len() {
            let _ = parse(&BATTERY[..end]);
        }
        for end in (0..CROTON.len()).step_by(7) {
            let _ = parse(&CROTON[..end]);
        }
    }

    #[test]
    fn point_limit_is_enforced_while_reading() {
        let mut doc = String::from("<gpx><trk><trkseg>");
        for _ in 0..=MAX_POINTS {
            doc.push_str("<trkpt lat=\"1\" lon=\"2\"/>");
        }
        doc.push_str("</trkseg></trk></gpx>");
        assert_eq!(
            parse(doc.as_bytes()),
            Err(ImportError::TooManyPoints { max: MAX_POINTS })
        );
        let ok = doc.replacen("<trkpt lat=\"1\" lon=\"2\"/>", "", 1);
        assert_eq!(
            parse(ok.as_bytes()).expect("parse").tracks[0].point_count(),
            MAX_POINTS
        );
    }

    #[test]
    fn size_limit() {
        let big = vec![b' '; super::super::MAX_INPUT_BYTES + 1];
        assert!(matches!(parse(&big), Err(ImportError::TooLarge { .. })));
    }
}
