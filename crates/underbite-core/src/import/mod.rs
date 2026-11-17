//! Waypoints and tracks read from files the user opens: GPX, KML, KMZ and CSV. What is imported
//! stays on the device.
//!
//! Parsers are sans-IO: they take the bytes of the file and return [`Imported`]. They never
//! resolve external entities or DTDs, refuse inputs over [`MAX_INPUT_BYTES`] or with more than
//! [`MAX_POINTS`] points (checked while reading), report errors with a line number and never
//! panic on malformed input.

pub mod csv;
pub mod gpx;
pub mod kml;

use serde::{Deserialize, Serialize};
use whether_core::time::{UnixTime, parse_rfc3339};

/// The largest file accepted, in bytes.
pub const MAX_INPUT_BYTES: usize = 20 * 1024 * 1024;

/// The most points (waypoints plus track and route points) accepted from one file.
pub const MAX_POINTS: usize = 200_000;

/// A named point, from a GPX `wpt` or a CSV row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Waypoint {
    /// The name, when the file gives one.
    pub name: Option<String>,
    /// Degrees north, WGS 84.
    pub latitude: f64,
    /// Degrees east, WGS 84.
    pub longitude: f64,
    /// Elevation in metres, when given.
    pub elevation_m: Option<f64>,
    /// Time of the point, when given.
    pub time: Option<UnixTime>,
    /// Description or comment, when given.
    pub notes: Option<String>,
}

/// One point of a track or route.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrackPoint {
    /// Degrees north, WGS 84.
    pub latitude: f64,
    /// Degrees east, WGS 84.
    pub longitude: f64,
    /// Elevation in metres, when given.
    pub elevation_m: Option<f64>,
    /// Time of the point, when given.
    pub time: Option<UnixTime>,
}

/// A track (or a GPX route, read as one segment).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Track {
    /// The name, when the file gives one.
    pub name: Option<String>,
    /// Segments in file order; never empty, and no segment is empty.
    pub segments: Vec<Vec<TrackPoint>>,
}

impl Track {
    /// The number of points over all segments.
    pub fn point_count(&self) -> usize {
        self.segments.iter().map(Vec::len).sum()
    }
}

/// What a file held.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Imported {
    /// Waypoints in file order.
    pub waypoints: Vec<Waypoint>,
    /// Tracks in file order.
    pub tracks: Vec<Track>,
    /// Things the user should know about how the file was read, for example times without an
    /// offset that were taken as UTC.
    pub notes: Vec<String>,
}

impl Imported {
    /// A short account of what was read, followed by the notes, one sentence per line.
    pub fn summary(&self) -> String {
        let points: usize = self.tracks.iter().map(Track::point_count).sum();
        let mut out = format!(
            "{} {}, {} {} ({} {})",
            self.waypoints.len(),
            plural(self.waypoints.len(), "waypoint", "waypoints"),
            self.tracks.len(),
            plural(self.tracks.len(), "track", "tracks"),
            points,
            plural(points, "point", "points"),
        );
        for note in &self.notes {
            out.push('\n');
            out.push_str(note);
        }
        out
    }
}

fn plural(n: usize, one: &'static str, many: &'static str) -> &'static str {
    if n == 1 { one } else { many }
}

/// The file formats the importer knows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// GPX 1.0 or 1.1.
    Gpx,
    /// KML 2.2.
    Kml,
    /// A zip archive with a KML document.
    Kmz,
    /// Comma (or semicolon) separated waypoints.
    Csv,
}

/// Why a file could not be imported.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ImportError {
    /// The input is larger than [`MAX_INPUT_BYTES`].
    #[error("the file is {len} bytes; the limit is {max} bytes")]
    TooLarge {
        /// Size of the input.
        len: usize,
        /// The limit.
        max: usize,
    },
    /// The file holds more than [`MAX_POINTS`] points.
    #[error("the file holds more than {max} points")]
    TooManyPoints {
        /// The limit.
        max: usize,
    },
    /// The format could not be told from the name or the content.
    #[error("the file is not GPX, KML, KMZ or CSV")]
    Unrecognised,
    /// The format is known but not readable here.
    #[error("{0} files cannot be read yet")]
    Unsupported(&'static str),
    /// The content is malformed or not acceptable.
    #[error("{format} line {line}: {message}")]
    Parse {
        /// Which parser reported it.
        format: &'static str,
        /// One-based line in the file (0 when the file is empty).
        line: u64,
        /// What is wrong.
        message: String,
    },
}

/// Tells the format of a file from its content first and its name second.
pub fn detect(bytes: &[u8], file_name: &str) -> Option<Format> {
    if bytes.starts_with(b"PK\x03\x04") {
        return Some(Format::Kmz);
    }
    let body = strip_bom(bytes);
    let head = &body[..body.len().min(4096)];
    let lower = String::from_utf8_lossy(head).to_ascii_lowercase();
    let trimmed = lower.trim_start();
    if trimmed.starts_with('<') {
        if lower.contains("<gpx") {
            return Some(Format::Gpx);
        }
        if lower.contains("<kml") {
            return Some(Format::Kml);
        }
    }
    let name = file_name.to_ascii_lowercase();
    let ext = name.rsplit_once('.').map(|(_, e)| e);
    match ext {
        Some("gpx") => return Some(Format::Gpx),
        Some("kml") => return Some(Format::Kml),
        Some("kmz") => return Some(Format::Kmz),
        Some("csv") => return Some(Format::Csv),
        _ => {}
    }
    let first = trimmed.lines().next().unwrap_or("");
    if !trimmed.starts_with('<')
        && first.contains("lat")
        && (first.contains("lon") || first.contains("lng"))
    {
        return Some(Format::Csv);
    }
    None
}

/// Detects the format and reads the file.
///
/// KML and KMZ are recognised but answer [`ImportError::Unsupported`] until their readers land.
pub fn import(bytes: &[u8], file_name: &str) -> Result<Imported, ImportError> {
    match detect(bytes, file_name).ok_or(ImportError::Unrecognised)? {
        Format::Gpx => gpx::parse(bytes),
        Format::Csv => csv::parse(bytes),
        Format::Kml => Err(ImportError::Unsupported("KML")),
        Format::Kmz => Err(ImportError::Unsupported("KMZ")),
    }
}

pub(crate) fn count_newlines(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0, |n, b| n + u64::from(*b == b'\n'))
}

pub(crate) fn strip_bom(bytes: &[u8]) -> &[u8] {
    bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes)
}

pub(crate) fn check_size(bytes: &[u8]) -> Result<(), ImportError> {
    if bytes.len() > MAX_INPUT_BYTES {
        return Err(ImportError::TooLarge {
            len: bytes.len(),
            max: MAX_INPUT_BYTES,
        });
    }
    Ok(())
}

/// A latitude in degrees, finite and within range.
pub(crate) fn check_latitude(v: f64) -> Result<f64, String> {
    if v.is_finite() && (-90.0..=90.0).contains(&v) {
        Ok(v)
    } else {
        Err(format!("latitude {v} is outside -90 to 90"))
    }
}

/// A longitude in degrees, finite and within range.
pub(crate) fn check_longitude(v: f64) -> Result<f64, String> {
    if v.is_finite() && (-180.0..=180.0).contains(&v) {
        Ok(v)
    } else {
        Err(format!("longitude {v} is outside -180 to 180"))
    }
}

/// Parses an ISO 8601 time. A time without an offset is read as UTC; the flag says so.
pub(crate) fn parse_time(text: &str) -> Result<(UnixTime, bool), String> {
    let s = text.trim();
    let tail = s.get(19..).unwrap_or("");
    let has_offset = tail.ends_with(['Z', 'z']) || tail.contains(['+', '-']);
    let candidate = if has_offset {
        s.replace('z', "Z")
    } else {
        format!("{s}Z")
    };
    match parse_rfc3339(&candidate) {
        Some(t) => Ok((t, !has_offset)),
        None => Err(format!("`{s}` is not a time like 2025-06-01T12:00:00Z")),
    }
}

/// The note added when times without an offset were taken as UTC.
pub(crate) fn assumed_utc_note(count: usize) -> String {
    format!(
        "{count} {} had no UTC offset and {} read as UTC.",
        plural(count, "time", "times"),
        if count == 1 { "was" } else { "were" },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_by_content_then_name() {
        assert_eq!(
            detect(b"\xEF\xBB\xBF <?xml version=\"1.0\"?><gpx/>", "x.txt"),
            Some(Format::Gpx)
        );
        assert_eq!(detect(b"<kml/>", "a"), Some(Format::Kml));
        assert_eq!(detect(b"PK\x03\x04rest", "a.zip"), Some(Format::Kmz));
        assert_eq!(detect(b"whatever", "Spots.CSV"), Some(Format::Csv));
        assert_eq!(detect(b"Name,Latitude,Longitude\n", "x"), Some(Format::Csv));
        assert_eq!(detect(b"hello", "x"), None);
    }

    #[test]
    fn unsupported_and_unrecognised() {
        assert_eq!(
            import(b"<kml/>", "a.kml"),
            Err(ImportError::Unsupported("KML"))
        );
        assert_eq!(import(b"hello", "x"), Err(ImportError::Unrecognised));
    }

    #[test]
    fn time_without_offset_is_utc() {
        let (a, assumed) = parse_time("2025-06-01T12:00:00").expect("time");
        let (b, explicit) = parse_time("2025-06-01T12:00:00Z").expect("time");
        let (c, _) = parse_time("2025-06-01T08:00:00-04:00").expect("time");
        assert_eq!((a, b, c), (b, b, b));
        assert!(assumed && !explicit);
        assert!(parse_time("yesterday").is_err());
        assert!(parse_time("").is_err());
    }

    #[test]
    fn summary_counts() {
        let imp = Imported {
            notes: vec!["n".into()],
            ..Imported::default()
        };
        assert_eq!(imp.summary(), "0 waypoints, 0 tracks (0 points)\nn");
    }
}
