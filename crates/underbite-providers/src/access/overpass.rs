//! OpenStreetMap through the Overpass API, on the user's action only: ramps, piers, fishing spots and parking.
//!
//! Overpass is a shared public service: every Underbite user draws on the same quota (about 10,000
//! queries and 1 GB a day for everyone, "divide by 100" for a regular user), and an answer takes 25
//! to 35 s. So nothing here runs by itself. This module holds pure functions and small value types
//! that a host drives (sans-IO, no clock: `now` is a parameter):
//!
//! - [`area_key`] and [`area_keys`] cut the world into a fixed 0.1 degree grid; one query covers one
//!   cell and its answer is cached for [`CACHE_DAYS`] days ([`cache_valid`]).
//! - [`query_request`] builds the GET for one cell and one [`QueryKind`]; the box edges are multiples
//!   of 0.1, so never finer than 2 decimals. Overpass boxes are (south, west, north, east).
//! - [`RequestGate`] is the persisted state a host keeps: a per-device daily cap, the 30 s pause after
//!   HTTP 429 or 406 and the back-off after "at capacity" answers. Only a [`UserAction`] may begin a
//!   request.
//! - [`parse_page`] reads the answer into [`AccessFeature`]s. The busy server answers 502 with a
//!   plain-text body, so a body is checked to be JSON before it is read.
//!
//! The query asks for no `meta`, so no user names or changeset ids come back. The data is `ODbL`:
//! credit [`OVERPASS_CREDIT`] with the link [`OVERPASS_COPYRIGHT_URL`].

use serde::{Deserialize, Serialize};
use serde_json::Value;
use underbite_core::map::{Bbox, GeoPoint};
use underbite_core::model::nearby::{AccessFeature, AccessGeometry, AccessKind, AccessLevel};
use underbite_core::provenance::SourceId;
use underbite_core::time::UnixTime;

use crate::request::{QueryBuilder, Response};

/// The public instance. Hosting is decided separately, so a host passes its own endpoint to
/// [`query_request`].
pub const DEFAULT_ENDPOINT: &str = "https://overpass-api.de/api/interpreter";

/// The credit the data's licence (`ODbL`) asks for.
pub const OVERPASS_CREDIT: &str = "\u{a9} OpenStreetMap contributors";

/// Where the copyright and licence are explained.
pub const OVERPASS_COPYRIGHT_URL: &str = "https://www.openstreetmap.org/copyright";

/// The `User-Agent` a host sends. It names the app and its version, never a person.
pub const USER_AGENT: &str = concat!("underbite/", env!("CARGO_PKG_VERSION"));

/// The message for a busy server.
pub const BUSY_MESSAGE: &str = "OpenStreetMap is busy, try later";

/// Days an area's answer stays valid.
pub const CACHE_DAYS: i64 = 30;

/// Requests a device may send in one UTC day unless the host sets another cap. Retries count.
pub const DEFAULT_DAILY_CAP: u32 = 12;

/// Pause after HTTP 429 or 406, seconds.
pub const RATE_LIMIT_PAUSE_SECS: u32 = 30;

/// First back-off after a busy answer, seconds; it doubles for each failure in a row.
pub const BACKOFF_START_SECS: u32 = 30;

/// The longest pause the gate sets, seconds.
pub const BACKOFF_MAX_SECS: u32 = 480;

/// Requests the host may send for one user action: the first and its retries.
pub const MAX_ATTEMPTS: u32 = 3;

/// The most cells one view may ask for (a 3 by 3 block, about 33 km by 25 km).
pub const MAX_AREAS: usize = 9;

/// Elements asked for in a [`QueryKind::Places`] query.
pub const PLACES_LIMIT: usize = 200;

/// Elements asked for in a [`QueryKind::Parking`] query.
pub const PARKING_LIMIT: usize = 100;

const SECS_PER_DAY: i64 = 86_400;

/// Proof that a person asked for the data (a tap), as opposed to a refresh or a background task.
/// [`RequestGate::begin`] needs one.
#[derive(Clone, Copy, Debug)]
pub struct UserAction(());

impl UserAction {
    /// Call this only from the handler of a user's tap or command.
    pub fn requested() -> Self {
        Self(())
    }
}

/// An Overpass request could not be built, or its response read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OverpassError {
    /// The request asks for something that cannot be sent.
    #[error("invalid Overpass request: {0}")]
    InvalidRequest(&'static str),
    /// HTTP 429 or 406: wait before the next request.
    #[error("Overpass asks for a pause of {retry_after_secs} s")]
    RateLimited { retry_after_secs: u32 },
    /// HTTP 502, 503 or 504, or a text body where JSON was due: the server is at capacity.
    #[error("OpenStreetMap is busy, try later")]
    Busy { retry_after_secs: u32 },
    /// Another HTTP failure.
    #[error("Overpass answered HTTP {status}")]
    Status { status: u16 },
    /// HTTP 200 with a `remark` that the query failed (timeout or memory limit).
    #[error("Overpass could not finish the query: {0}")]
    Runtime(String),
    /// The body is not the expected JSON.
    #[error("the Overpass response is not the expected JSON: {0}")]
    Parse(String),
}

impl OverpassError {
    /// Whether the same request may be sent again after a pause.
    pub fn retryable(&self) -> bool {
        matches!(self, Self::RateLimited { .. } | Self::Busy { .. })
    }
}

/// A cell of the fixed 0.1 degree grid, named by its south-west corner in tenths of a degree.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct AreaKey {
    /// Latitude of the south edge, tenths of a degree.
    pub south_tenths: i32,
    /// Longitude of the west edge, tenths of a degree.
    pub west_tenths: i32,
}

impl AreaKey {
    /// The cell's box in degrees; edges are multiples of 0.1.
    pub fn bbox(self) -> Bbox {
        Bbox {
            west: f64::from(self.west_tenths) / 10.0,
            south: f64::from(self.south_tenths) / 10.0,
            east: f64::from(self.west_tenths + 1) / 10.0,
            north: f64::from(self.south_tenths + 1) / 10.0,
        }
    }

    /// A stable text key for a cache, for example `407:-741` (40.7 N, 74.1 W).
    pub fn id(self) -> String {
        format!("{}:{}", self.south_tenths, self.west_tenths)
    }
}

/// What a query asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryKind {
    /// Boat ramps (`leisure=slipway`), fishing places (`leisure=fishing`) and piers
    /// (`man_made=pier`).
    Places,
    /// Parking (`amenity=parking`), a separate request because a cell holds many.
    Parking,
}

impl QueryKind {
    /// The element limit of the query.
    pub fn limit(self) -> usize {
        match self {
            Self::Places => PLACES_LIMIT,
            Self::Parking => PARKING_LIMIT,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Places => "places",
            Self::Parking => "parking",
        }
    }
}

/// What a cache entry is for: one cell and one kind of query.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct CacheKey {
    pub area: AreaKey,
    pub kind: QueryKind,
}

impl CacheKey {
    /// A stable text key, for example `overpass:places:407:-741`.
    pub fn id(self) -> String {
        format!("overpass:{}:{}", self.kind.name(), self.area.id())
    }
}

/// Snaps a value that is within rounding noise of a whole number of tenths to it, and returns the
/// value in tenths.
fn tenths(degrees: f64) -> f64 {
    let v = degrees * 10.0;
    let r = v.round();
    if (v - r).abs() < 1e-6 { r } else { v }
}

/// Tenths as a whole number. The input is finite and within +-1800, so the cast is exact.
#[allow(clippy::cast_possible_truncation)]
fn whole(v: f64) -> i32 {
    v as i32
}

/// The cell holding a position; none for a position that is not on Earth.
pub fn area_key(lat: f64, lon: f64) -> Option<AreaKey> {
    if !((-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon)) {
        return None;
    }
    Some(AreaKey {
        south_tenths: whole(tenths(lat).floor()),
        west_tenths: whole(tenths(lon).floor()),
    })
}

/// The cells a box touches, south to north and west to east. A box that only touches a cell's
/// edge does not include that cell.
///
/// # Errors
/// [`OverpassError::InvalidRequest`] for a box that is not finite, outside the world or inverted,
/// or one that covers more than [`MAX_AREAS`] cells (zoom in).
pub fn area_keys(bbox: Bbox) -> Result<Vec<AreaKey>, OverpassError> {
    let ok = [bbox.west, bbox.south, bbox.east, bbox.north]
        .iter()
        .all(|v| v.is_finite())
        && bbox.west <= bbox.east
        && bbox.south <= bbox.north
        && bbox.west >= -180.0
        && bbox.east <= 180.0
        && bbox.south >= -90.0
        && bbox.north <= 90.0;
    if !ok {
        return Err(OverpassError::InvalidRequest("box"));
    }
    let s0 = whole(tenths(bbox.south).floor());
    let w0 = whole(tenths(bbox.west).floor());
    let s1 = (whole(tenths(bbox.north).ceil()) - 1).max(s0);
    let w1 = (whole(tenths(bbox.east).ceil()) - 1).max(w0);
    let count = (i64::from(s1 - s0) + 1) * (i64::from(w1 - w0) + 1);
    if usize::try_from(count).map_or(true, |c| c > MAX_AREAS) {
        return Err(OverpassError::InvalidRequest("too many areas"));
    }
    let mut keys = Vec::new();
    for south_tenths in s0..=s1 {
        for west_tenths in w0..=w1 {
            keys.push(AreaKey {
                south_tenths,
                west_tenths,
            });
        }
    }
    Ok(keys)
}

/// Whether an answer fetched at `fetched_at` may still be shown at `now`. An entry from the future
/// (a clock that moved back) is not valid.
pub fn cache_valid(fetched_at: UnixTime, now: UnixTime) -> bool {
    let age = now.0.saturating_sub(fetched_at.0);
    (0..CACHE_DAYS * SECS_PER_DAY).contains(&age)
}

/// The seconds from `now` until the entry expires; zero when it has.
pub fn cache_remaining_secs(fetched_at: UnixTime, now: UnixTime) -> i64 {
    if !cache_valid(fetched_at, now) {
        return 0;
    }
    fetched_at
        .0
        .saturating_add(CACHE_DAYS * SECS_PER_DAY)
        .saturating_sub(now.0)
}

/// One GET for the host to send, with the `User-Agent` to send it with. The URL carries no key or
/// personal data, and the box is a grid cell, not the user's position.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OverpassRequest {
    pub url: String,
    pub user_agent: &'static str,
    pub key: CacheKey,
}

/// The query text for one cell: `(south,west,north,east)` boxes, a 25 s server timeout, a 2 MB
/// result cap and `out center tags`, so a way arrives as a point and no `meta` is requested.
pub fn query_text(key: CacheKey) -> String {
    let b = key.area.bbox();
    let area = format!(
        "({:.1},{:.1},{:.1},{:.1})",
        b.south, b.west, b.north, b.east
    );
    let limit = key.kind.limit();
    let body = match key.kind {
        QueryKind::Places => format!(
            "(nwr[\"leisure\"=\"slipway\"]{area};nwr[\"leisure\"=\"fishing\"]{area};nwr[\"man_made\"=\"pier\"]{area};)"
        ),
        QueryKind::Parking => format!("nwr[\"amenity\"=\"parking\"]{area}"),
    };
    format!("[out:json][timeout:25][maxsize:2000000];{body};out center tags {limit};")
}

/// The request for one cell. `endpoint` is the interpreter URL ([`DEFAULT_ENDPOINT`] or the
/// app's own host). Plan it only for a user action, through [`RequestGate::begin`].
///
/// # Errors
/// [`OverpassError::InvalidRequest`] for an endpoint that is not an http or https URL.
pub fn query_request(endpoint: &str, key: CacheKey) -> Result<OverpassRequest, OverpassError> {
    if !(endpoint.starts_with("https://") || endpoint.starts_with("http://"))
        || endpoint.contains(char::is_whitespace)
    {
        return Err(OverpassError::InvalidRequest("endpoint"));
    }
    Ok(OverpassRequest {
        url: QueryBuilder::new(endpoint)
            .param("data", query_text(key))
            .finish(),
        user_agent: USER_AGENT,
        key,
    })
}

/// Why the gate does not allow a request now.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Refused {
    /// The device used its requests for the UTC day.
    #[error("the daily limit of Overpass requests is used up")]
    DailyCap,
    /// A pause is running.
    #[error("Overpass is paused for {wait_secs} s")]
    Paused { wait_secs: u32 },
}

/// What to do after a request finished.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Next {
    /// Nothing more: done, or a failure that waiting does not cure.
    Stop,
    /// Send the same request again once `after_secs` have passed (the gate also enforces it).
    Retry { after_secs: u32 },
}

/// The state a host keeps (and may store) between requests: the daily count, the pause and the
/// run of failures. Every method takes `now`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestGate {
    cap: u32,
    day: i64,
    used: u32,
    pause_until: i64,
    failures: u32,
}

impl Default for RequestGate {
    fn default() -> Self {
        Self::with_cap(DEFAULT_DAILY_CAP)
    }
}

impl RequestGate {
    /// A gate that allows `cap` requests a UTC day.
    pub fn with_cap(cap: u32) -> Self {
        Self {
            cap,
            day: i64::MIN,
            used: 0,
            pause_until: i64::MIN,
            failures: 0,
        }
    }

    fn today(now: UnixTime) -> i64 {
        now.0.div_euclid(SECS_PER_DAY)
    }

    /// Requests left today.
    pub fn remaining_today(&self, now: UnixTime) -> u32 {
        if self.day == Self::today(now) {
            self.cap.saturating_sub(self.used)
        } else {
            self.cap
        }
    }

    /// Whether a request may be sent now.
    ///
    /// # Errors
    /// [`Refused::Paused`] while a pause runs, [`Refused::DailyCap`] when the day's requests are used.
    pub fn check(&self, now: UnixTime) -> Result<(), Refused> {
        if now.0 < self.pause_until {
            let wait = self.pause_until.saturating_sub(now.0);
            return Err(Refused::Paused {
                wait_secs: u32::try_from(wait).unwrap_or(u32::MAX),
            });
        }
        if self.remaining_today(now) == 0 {
            return Err(Refused::DailyCap);
        }
        Ok(())
    }

    /// Counts a request about to be sent. Needs a [`UserAction`]: a request is never planned by a
    /// refresh or a background task.
    ///
    /// # Errors
    /// As [`RequestGate::check`]; nothing is counted then.
    pub fn begin(&mut self, now: UnixTime, _by: UserAction) -> Result<(), Refused> {
        self.check(now)?;
        let today = Self::today(now);
        if self.day != today {
            self.day = today;
            self.used = 0;
        }
        self.used = self.used.saturating_add(1);
        Ok(())
    }

    /// Records how a request ended (`None` for success) and says whether to try again. A failed
    /// request pauses the gate: 30 s after 429 or 406, a doubling back-off (30 s, 60 s, up to 8 min)
    /// after a busy answer, and a longer `Retry-After` is honoured up to 8 min. A host that got no
    /// answer at all (timeout, no network) passes `Some(&OverpassError::Busy { .. })`.
    pub fn finish(&mut self, now: UnixTime, error: Option<&OverpassError>) -> Next {
        let Some(error) = error else {
            self.failures = 0;
            self.pause_until = i64::MIN;
            return Next::Stop;
        };
        self.failures = self.failures.saturating_add(1);
        let pause = match error {
            OverpassError::RateLimited { retry_after_secs } => {
                RATE_LIMIT_PAUSE_SECS.max(*retry_after_secs)
            }
            OverpassError::Busy { retry_after_secs } => {
                backoff_secs(self.failures - 1).max(*retry_after_secs)
            }
            OverpassError::Runtime(_) => RATE_LIMIT_PAUSE_SECS,
            _ => 0,
        }
        .min(BACKOFF_MAX_SECS);
        self.pause_until = now.0.saturating_add(i64::from(pause));
        if error.retryable() && self.failures < MAX_ATTEMPTS {
            Next::Retry { after_secs: pause }
        } else {
            Next::Stop
        }
    }
}

/// The back-off before retry number `failures` (0 for the first): 30 s doubling to 8 min.
pub fn backoff_secs(failures: u32) -> u32 {
    BACKOFF_START_SECS
        .saturating_mul(1u32.checked_shl(failures).unwrap_or(u32::MAX))
        .min(BACKOFF_MAX_SECS)
}

/// The features one answer held.
#[derive(Clone, Debug, PartialEq)]
pub struct OverpassPage {
    /// Ramps, piers, fishing places or parking, as points.
    pub features: Vec<AccessFeature>,
    /// Elements the answer held.
    pub rows: usize,
    /// Elements left out for missing position or no tag of the asked kinds.
    pub skipped: usize,
    /// Whether the answer hit the query's limit, so the cell may hold more.
    pub truncated: bool,
}

/// What an OSM `access` tag says, as the source's record and never legal permission.
pub fn access_level(tag: Option<&str>) -> AccessLevel {
    match tag.map(|t| t.trim().to_ascii_lowercase()).as_deref() {
        Some("yes" | "permissive" | "public" | "designated") => AccessLevel::Open,
        Some("customers" | "permit" | "members" | "delivery" | "destination") => {
            AccessLevel::Restricted
        }
        Some("private" | "no") => AccessLevel::Closed,
        _ => AccessLevel::Unknown,
    }
}

fn kind_of(tags: &Value) -> Option<AccessKind> {
    let is = |k: &str, v: &str| tags.get(k).and_then(Value::as_str) == Some(v);
    if is("leisure", "slipway") {
        Some(AccessKind::Ramp)
    } else if is("leisure", "fishing") {
        Some(AccessKind::FishingSpot)
    } else if is("man_made", "pier") {
        Some(AccessKind::Pier)
    } else if is("amenity", "parking") {
        Some(AccessKind::Parking)
    } else {
        None
    }
}

fn position(e: &Value) -> Option<GeoPoint> {
    let src = e.get("center").unwrap_or(e);
    let lat = src.get("lat")?.as_f64()?;
    let lon = src.get("lon")?.as_f64()?;
    ((-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon))
        .then_some(GeoPoint { lat, lon })
}

/// Whether the body is a JSON object, judged by its first byte: the busy server sends a text body.
fn looks_like_json(body: &[u8]) -> bool {
    body.iter()
        .find(|b| !b.is_ascii_whitespace())
        .is_some_and(|b| *b == b'{')
}

/// Reads one answer. `kind` is what was asked, for the truncation check. The element kinds are
/// the ones the query asks for; an element with none of them is skipped.
///
/// # Errors
/// [`OverpassError::RateLimited`] for HTTP 429 or 406, [`OverpassError::Busy`] for 502, 503 or 504
/// and for a 200 whose body is text about capacity, [`OverpassError::Status`] for other HTTP
/// failures, [`OverpassError::Runtime`] when the `remark` reports a failed query and
/// [`OverpassError::Parse`] for a body that is not JSON with an `elements` list.
pub fn parse_page(response: &Response<'_>, kind: QueryKind) -> Result<OverpassPage, OverpassError> {
    let wait = response.retry_after.unwrap_or(0);
    match response.status {
        200..=299 => {}
        429 | 406 => {
            return Err(OverpassError::RateLimited {
                retry_after_secs: RATE_LIMIT_PAUSE_SECS.max(wait),
            });
        }
        502..=504 => {
            return Err(OverpassError::Busy {
                retry_after_secs: wait,
            });
        }
        status => return Err(OverpassError::Status { status }),
    }
    if !looks_like_json(response.body) {
        let text = String::from_utf8_lossy(response.body);
        return Err(if text.contains("capacity") || text.contains("busy") {
            OverpassError::Busy {
                retry_after_secs: wait,
            }
        } else {
            OverpassError::Parse("the body is not JSON".to_owned())
        });
    }
    let v: Value =
        serde_json::from_slice(response.body).map_err(|e| OverpassError::Parse(e.to_string()))?;
    if let Some(remark) = v.get("remark").and_then(Value::as_str)
        && remark.contains("runtime error")
    {
        return Err(OverpassError::Runtime(remark.to_owned()));
    }
    let rows = v
        .get("elements")
        .and_then(Value::as_array)
        .ok_or_else(|| OverpassError::Parse("no elements".to_owned()))?;
    let mut features = Vec::new();
    let mut skipped = 0;
    for e in rows {
        let tags = e.get("tags").unwrap_or(&Value::Null);
        let (Some(access_kind), Some(at)) = (kind_of(tags), position(e)) else {
            skipped += 1;
            continue;
        };
        let wanted = match kind {
            QueryKind::Places => access_kind != AccessKind::Parking,
            QueryKind::Parking => access_kind == AccessKind::Parking,
        };
        if !wanted {
            skipped += 1;
            continue;
        }
        features.push(AccessFeature {
            source: SourceId::OVERPASS,
            kind: access_kind,
            name: tags
                .get("name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned),
            geometry: AccessGeometry::Point(at),
            access: access_level(tags.get("access").and_then(Value::as_str)),
        });
    }
    Ok(OverpassPage {
        features,
        rows: rows.len(),
        skipped,
        truncated: rows.len() >= kind.limit(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::fixture_bytes;

    fn t(secs: i64) -> UnixTime {
        UnixTime(secs)
    }

    fn places(lat10: i32, lon10: i32) -> CacheKey {
        CacheKey {
            area: AreaKey {
                south_tenths: lat10,
                west_tenths: lon10,
            },
            kind: QueryKind::Places,
        }
    }

    #[test]
    fn area_keys_follow_a_fixed_grid() {
        let battery = area_key(40.7003, -74.0142).unwrap();
        assert_eq!(battery.id(), "407:-741");
        let b = battery.bbox();
        assert_eq!(
            (b.south, b.west, b.north, b.east),
            (40.7, -74.1, 40.8, -74.0)
        );
        assert_eq!(area_key(40.7999, -74.0001).unwrap().id(), "407:-741");
        assert_eq!(area_key(-0.05, 0.05).unwrap().id(), "-1:0");
        assert_eq!(area_key(41.2, -73.9).unwrap().id(), "412:-739");
        assert!(area_key(91.0, 0.0).is_none());
        assert!(area_key(f64::NAN, 0.0).is_none());
        assert_eq!(places(407, -741).id(), "overpass:places:407:-741");
    }

    #[test]
    fn a_box_gives_the_cells_it_touches() {
        let b = |w, s, e, n| Bbox {
            west: w,
            south: s,
            east: e,
            north: n,
        };
        let one = area_keys(b(-74.05, 40.72, -74.01, 40.78)).unwrap();
        assert_eq!(one, vec![area_key(40.75, -74.03).unwrap()]);
        let edge = area_keys(b(-74.1, 40.7, -74.0, 40.8)).unwrap();
        assert_eq!(edge.len(), 1, "edges on the grid add no cell");
        let four = area_keys(b(-74.15, 40.65, -74.05, 40.75)).unwrap();
        assert_eq!(four.len(), 4);
        assert_eq!(four[0].id(), "406:-742");
        assert_eq!(four[3].id(), "407:-741");
        assert_eq!(area_keys(b(-74.0, 40.7, -74.0, 40.7)).unwrap().len(), 1);
        assert!(area_keys(b(-75.0, 40.0, -73.0, 42.0)).is_err());
        assert!(area_keys(b(-73.0, 40.0, -74.0, 42.0)).is_err());
        assert!(area_keys(b(f64::NAN, 40.0, -74.0, 42.0)).is_err());
    }

    #[test]
    fn cache_lasts_thirty_days() {
        let day = 86_400;
        assert!(cache_valid(t(1_000), t(1_000)));
        assert!(cache_valid(t(1_000), t(1_000 + 30 * day - 1)));
        assert!(!cache_valid(t(1_000), t(1_000 + 30 * day)));
        assert!(!cache_valid(t(1_000), t(999)), "from the future");
        assert_eq!(cache_remaining_secs(t(0), t(29 * day)), day);
        assert_eq!(cache_remaining_secs(t(0), t(31 * day)), 0);
        assert!(!cache_valid(t(i64::MIN), t(i64::MAX)));
    }

    #[test]
    fn query_is_a_get_with_the_expected_text() {
        let key = places(407, -741);
        let text = query_text(key);
        assert_eq!(
            text,
            "[out:json][timeout:25][maxsize:2000000];(nwr[\"leisure\"=\"slipway\"](40.7,-74.1,40.8,-74.0);nwr[\"leisure\"=\"fishing\"](40.7,-74.1,40.8,-74.0);nwr[\"man_made\"=\"pier\"](40.7,-74.1,40.8,-74.0););out center tags 200;"
        );
        assert!(!text.contains("meta"));
        let parking = query_text(CacheKey {
            kind: QueryKind::Parking,
            ..key
        });
        assert!(
            parking.contains(
                "nwr[\"amenity\"=\"parking\"](40.7,-74.1,40.8,-74.0);out center tags 100;"
            )
        );
        let r = query_request(DEFAULT_ENDPOINT, key).unwrap();
        assert!(
            r.url
                .starts_with("https://overpass-api.de/api/interpreter?data=%5Bout")
        );
        assert!(!r.url.contains(' '));
        assert_eq!(r.key, key);
        assert!(r.user_agent.starts_with("underbite/"));
        assert!(!r.user_agent.contains('@'));
        let own = query_request("https://overpass.example.invalid/api/interpreter", key).unwrap();
        assert!(own.url.starts_with("https://overpass.example.invalid/"));
        assert!(query_request("ftp://x", key).is_err());
        assert!(query_request("https://a b", key).is_err());
    }

    #[test]
    fn credit_names_openstreetmap() {
        assert_eq!(OVERPASS_CREDIT, "\u{a9} OpenStreetMap contributors");
        assert_eq!(
            OVERPASS_COPYRIGHT_URL,
            "https://www.openstreetmap.org/copyright"
        );
    }

    #[test]
    fn parses_the_new_york_harbor_slipways_and_piers() {
        let body = fixture_bytes("tier2", "overpass-nyharbor-slipways-piers.json");
        let page = parse_page(&Response::new(200, &body), QueryKind::Places).unwrap();
        assert_eq!((page.rows, page.skipped, page.truncated), (30, 0, false));
        assert_eq!(page.features.len(), 30);
        let count = |k| page.features.iter().filter(|f| f.kind == k).count();
        assert_eq!(count(AccessKind::Pier), 15);
        assert_eq!(count(AccessKind::Ramp) + count(AccessKind::FishingSpot), 15);
        assert!(page.features.iter().all(|f| f.source == SourceId::OVERPASS));
        assert!(
            page.features
                .iter()
                .all(|f| matches!(f.geometry, AccessGeometry::Point(_)))
        );
        let kayak = page
            .features
            .iter()
            .find(|f| f.name.as_deref() == Some("Kayak Access"))
            .unwrap();
        assert_eq!(kayak.kind, AccessKind::Ramp);
        assert_eq!(kayak.access, AccessLevel::Open);
        assert_eq!(
            kayak.geometry,
            AccessGeometry::Point(GeoPoint {
                lat: 40.739_225_6,
                lon: -74.011_155_2
            })
        );
        insta::assert_json_snapshot!(page.features);
    }

    #[test]
    fn ways_use_their_centre() {
        let body = fixture_bytes("overpass", "overpass-parking-croton-2026-11-17.json");
        let page = parse_page(&Response::new(200, &body), QueryKind::Parking).unwrap();
        assert_eq!(page.rows, 10);
        assert_eq!(page.features.len(), 10);
        assert!(!page.truncated);
        assert!(page.features.iter().all(|f| f.kind == AccessKind::Parking));
        // Asked for places, parking is not wanted.
        let other = parse_page(&Response::new(200, &body), QueryKind::Places).unwrap();
        assert_eq!((other.features.len(), other.skipped), (0, 10));
    }

    #[test]
    fn a_full_answer_is_marked_truncated() {
        let body = fixture_bytes("overpass", "overpass-access-battery-2026-11-17.json");
        let page = parse_page(&Response::new(200, &body), QueryKind::Places).unwrap();
        assert_eq!(page.rows, 60);
        assert!(!page.truncated, "the limit here is 200");
        let one = br#"{"elements":[{"type":"node","id":1,"lat":1.0,"lon":2.0,"tags":{"leisure":"slipway"}}]}"#;
        let page = parse_page(&Response::new(200, one), QueryKind::Places).unwrap();
        assert_eq!(page.features.len(), 1);
        let full: Vec<String> = (0..QueryKind::Parking.limit())
            .map(|i| {
                format!(
                    r#"{{"type":"node","id":{i},"lat":1.0,"lon":2.0,"tags":{{"amenity":"parking"}}}}"#
                )
            })
            .collect();
        let body = format!(r#"{{"elements":[{}]}}"#, full.join(","));
        let page = parse_page(&Response::new(200, body.as_bytes()), QueryKind::Parking).unwrap();
        assert!(page.truncated);
    }

    #[test]
    fn access_tags_map_and_unknown_is_never_open() {
        assert_eq!(access_level(Some("yes")), AccessLevel::Open);
        assert_eq!(access_level(Some("Permissive")), AccessLevel::Open);
        assert_eq!(access_level(Some("customers")), AccessLevel::Restricted);
        assert_eq!(access_level(Some("permit")), AccessLevel::Restricted);
        assert_eq!(access_level(Some("private")), AccessLevel::Closed);
        assert_eq!(access_level(Some("no")), AccessLevel::Closed);
        for odd in [None, Some(""), Some("maybe")] {
            assert_eq!(access_level(odd), AccessLevel::Unknown);
        }
    }

    #[test]
    fn bad_elements_are_skipped_and_names_trimmed() {
        let body = br#"{"elements":[
          {"type":"node","id":1,"lat":40.7,"lon":-74.0,"tags":{"leisure":"slipway","name":"  Ramp  ","access":"private"}},
          {"type":"node","id":2,"tags":{"leisure":"slipway"}},
          {"type":"node","id":3,"lat":999,"lon":0,"tags":{"leisure":"slipway"}},
          {"type":"node","id":4,"lat":1,"lon":2,"tags":{"shop":"bait"}},
          {"type":"relation","id":5,"center":{"lat":41.0,"lon":-73.0},"tags":{"man_made":"pier","name":""}}]}"#;
        let page = parse_page(&Response::new(200, body), QueryKind::Places).unwrap();
        assert_eq!((page.rows, page.skipped), (5, 3));
        assert_eq!(page.features[0].name.as_deref(), Some("Ramp"));
        assert_eq!(page.features[0].access, AccessLevel::Closed);
        assert_eq!(page.features[1].kind, AccessKind::Pier);
        assert_eq!(page.features[1].name, None);
    }

    #[test]
    fn failures_are_classified() {
        let capacity =
            b"This Overpass API server is currently at capacity. All query workers are busy.";
        let busy = parse_page(&Response::new(502, capacity), QueryKind::Places).unwrap_err();
        assert_eq!(
            busy,
            OverpassError::Busy {
                retry_after_secs: 0
            }
        );
        assert_eq!(busy.to_string(), BUSY_MESSAGE);
        // A 200 with a text body is checked and is not read as JSON.
        assert!(matches!(
            parse_page(&Response::new(200, capacity), QueryKind::Places),
            Err(OverpassError::Busy { .. })
        ));
        assert!(matches!(
            parse_page(&Response::new(200, b"<html>oops"), QueryKind::Places),
            Err(OverpassError::Parse(_))
        ));
        for status in [429, 406] {
            assert_eq!(
                parse_page(&Response::new(status, b""), QueryKind::Places).unwrap_err(),
                OverpassError::RateLimited {
                    retry_after_secs: 30
                }
            );
        }
        let mut slow = Response::new(429, b"");
        slow.retry_after = Some(90);
        assert_eq!(
            parse_page(&slow, QueryKind::Places).unwrap_err(),
            OverpassError::RateLimited {
                retry_after_secs: 90
            }
        );
        assert_eq!(
            parse_page(&Response::new(400, b"bad"), QueryKind::Places).unwrap_err(),
            OverpassError::Status { status: 400 }
        );
        let remark = br#"{"elements":[],"remark":"runtime error: Query timed out in \"query\" at line 3 after 26 seconds."}"#;
        assert!(matches!(
            parse_page(&Response::new(200, remark), QueryKind::Places),
            Err(OverpassError::Runtime(_))
        ));
        assert!(matches!(
            parse_page(&Response::new(200, b"{}"), QueryKind::Places),
            Err(OverpassError::Parse(_))
        ));
        assert!(matches!(
            parse_page(&Response::new(200, b"{\"elements\":"), QueryKind::Places),
            Err(OverpassError::Parse(_))
        ));
    }

    #[test]
    fn the_daily_cap_counts_per_utc_day() {
        let mut gate = RequestGate::with_cap(2);
        let go = UserAction::requested();
        let noon = 12 * 3600;
        assert_eq!(gate.remaining_today(t(noon)), 2);
        gate.begin(t(noon), go).unwrap();
        gate.begin(t(noon + 1), go).unwrap();
        assert_eq!(gate.begin(t(noon + 2), go), Err(Refused::DailyCap));
        assert_eq!(gate.remaining_today(t(noon + 2)), 0);
        // Midnight UTC starts a new day.
        assert_eq!(gate.check(t(86_400)), Ok(()));
        gate.begin(t(86_400), go).unwrap();
        assert_eq!(gate.remaining_today(t(86_401)), 1);
        // A refused request is not counted.
        let mut spent = RequestGate::with_cap(0);
        assert_eq!(spent.begin(t(5), go), Err(Refused::DailyCap));
    }

    #[test]
    fn rate_limit_pauses_for_thirty_seconds() {
        let mut gate = RequestGate::default();
        let go = UserAction::requested();
        gate.begin(t(100), go).unwrap();
        let next = gate.finish(
            t(100),
            Some(&OverpassError::RateLimited {
                retry_after_secs: 30,
            }),
        );
        assert_eq!(next, Next::Retry { after_secs: 30 });
        assert_eq!(gate.check(t(110)), Err(Refused::Paused { wait_secs: 20 }));
        assert_eq!(
            gate.begin(t(129), go),
            Err(Refused::Paused { wait_secs: 1 })
        );
        assert_eq!(gate.check(t(130)), Ok(()));
    }

    #[test]
    fn busy_backs_off_and_gives_up_after_three_attempts() {
        assert_eq!(
            [0, 1, 2, 3, 4, 5, 40].map(backoff_secs),
            [30, 60, 120, 240, 480, 480, 480]
        );
        let mut gate = RequestGate::default();
        let busy = OverpassError::Busy {
            retry_after_secs: 0,
        };
        assert_eq!(
            gate.finish(t(0), Some(&busy)),
            Next::Retry { after_secs: 30 }
        );
        assert_eq!(
            gate.finish(t(30), Some(&busy)),
            Next::Retry { after_secs: 60 }
        );
        assert_eq!(gate.check(t(60)), Err(Refused::Paused { wait_secs: 30 }));
        assert_eq!(gate.finish(t(90), Some(&busy)), Next::Stop);
        // Success clears the run and the pause.
        assert_eq!(gate.finish(t(200), None), Next::Stop);
        assert_eq!(gate.check(t(200)), Ok(()));
        assert_eq!(
            gate.finish(t(300), Some(&busy)),
            Next::Retry { after_secs: 30 }
        );
    }

    #[test]
    fn retry_after_longer_than_the_back_off_is_kept_and_capped() {
        let mut gate = RequestGate::default();
        let long = OverpassError::Busy {
            retry_after_secs: 100,
        };
        assert_eq!(
            gate.finish(t(0), Some(&long)),
            Next::Retry { after_secs: 100 }
        );
        let huge = OverpassError::RateLimited {
            retry_after_secs: 86_400,
        };
        assert_eq!(
            gate.finish(t(0), Some(&huge)),
            Next::Retry {
                after_secs: BACKOFF_MAX_SECS
            }
        );
        // Failures that waiting does not cure do not retry or pause.
        let mut other = RequestGate::default();
        assert_eq!(
            other.finish(t(0), Some(&OverpassError::Status { status: 400 })),
            Next::Stop
        );
        assert_eq!(other.check(t(0)), Ok(()));
        assert_eq!(
            other.finish(t(0), Some(&OverpassError::Runtime("x".into()))),
            Next::Stop
        );
        assert_eq!(other.check(t(1)), Err(Refused::Paused { wait_secs: 29 }));
    }

    #[test]
    fn the_gate_state_round_trips_for_storage() {
        let mut gate = RequestGate::with_cap(5);
        gate.begin(t(10), UserAction::requested()).unwrap();
        gate.finish(
            t(10),
            Some(&OverpassError::Busy {
                retry_after_secs: 0,
            }),
        );
        let json = serde_json::to_string(&gate).unwrap();
        assert_eq!(serde_json::from_str::<RequestGate>(&json).unwrap(), gate);
    }
}
