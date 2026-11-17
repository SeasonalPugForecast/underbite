//! ERDDAP griddap requests and their parsing (sans-IO), for the NOAA and GLERL nodes.
//!
//! A map layer asks for a subset of a [`GridSource`] at one time over a box, at a stride counted
//! in native grid steps ([`grid_request`], or [`block_request`] for a cache block). The time is
//! the newest (`(last)`) or one the dataset lists: daily products are irregular (cloudy days are
//! missing, and the VIIRS Kd490 pass falls around 19:00Z rather than at noon), so a client reads
//! the time axis first ([`time_axis_request`], [`parse_time_axis`]) and never builds a time from
//! a date. Asking for a time past the end of the axis fails.
//!
//! Responses are ERDDAP's JSON table: `columnNames`, `columnUnits` and one row per cell, `null`
//! where a value is missing. [`parse_grid`] reads each row's coordinates rather than assuming an
//! order (the VIIRS datasets list latitude descending, the others ascending), the time from the
//! `time` column and the unit from `columnUnits`. [`check_response`] maps ERDDAP's failures
//! first. [`ErddapPacer`] keeps requests to each node polite.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use underbite_core::map::grid::{Block, Grid, GridError};
use underbite_core::map::{Bbox, GridHost, GridSource};
use underbite_core::time::{LocalDate, UnixTime, UtcOffset, parse_rfc3339};

use crate::request::Response;

/// The text of ERDDAP's 404 for a subset with nothing in it, such as a time past the end of the
/// axis.
pub const NO_MATCHING_RESULTS: &str = "Your query produced no matching results";

/// The most requests sent to one node in a minute; pfeg answered 429 at about 12.
pub const MAX_REQUESTS_PER_MINUTE: usize = 10;

/// The pause after a node's first 429 or 503, seconds; it doubles on each further one.
pub const FIRST_PAUSE_S: i64 = 60;

/// The longest pause the doubling reaches, seconds.
pub const MAX_PAUSE_S: i64 = 15 * 60;

/// The longest `Retry-After` honoured, seconds.
const MAX_RETRY_AFTER_S: i64 = 3_600;

/// The span [`MAX_REQUESTS_PER_MINUTE`] counts over, seconds.
const PACE_WINDOW_S: i64 = 60;

/// Coordinates in a request keep this many decimals (about 0.1 m).
const COORDINATE_DECIMALS: i32 = 6;

/// Which time of a dataset a request asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GridTime {
    /// The newest time on the dataset's axis (`(last)`).
    Last,
    /// A time the dataset's axis lists; ERDDAP matches it to the nearest one.
    At(UnixTime),
}

/// What an [`ErddapRequest`] asks for; selects its parser.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ErddapResource {
    /// Values over a box: [`parse_grid`], [`parse_grids`] or [`parse_currents`].
    Grid,
    /// The times a dataset holds: [`parse_time_axis`].
    TimeAxis,
}

/// One GET for the host to send. The URL carries no key or personal data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ErddapRequest {
    /// The server, for [`ErddapPacer`].
    pub node: GridHost,
    /// The full URL with the query, brackets percent-encoded.
    pub url: String,
    /// What it asks for.
    pub resource: ErddapResource,
}

/// An ERDDAP request could not be built, or its response read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ErddapError {
    /// The request asks for something that cannot be sent.
    #[error("invalid ERDDAP request: {0}")]
    InvalidRequest(&'static str),
    /// The subset holds nothing (404, "Your query produced no matching results"), such as a
    /// time past the end of the axis or a box outside the grid.
    #[error("ERDDAP has no data for this request")]
    NoData,
    /// The server failed for now (503, "There was a (temporary?) problem. Wait a minute, then
    /// try again."); retry after a pause.
    #[error("ERDDAP is temporarily unavailable")]
    Temporary,
    /// Too many requests (429); back off.
    #[error("ERDDAP asked to slow down")]
    RateLimited {
        /// The `Retry-After` the server sent, seconds.
        retry_after_s: Option<u32>,
    },
    /// Another HTTP status.
    #[error("ERDDAP answered HTTP {status}")]
    Status {
        /// The status code.
        status: u16,
    },
    /// The body is not ERDDAP's JSON table.
    #[error("the response is not an ERDDAP JSON table: {0}")]
    BadTable(String),
    /// The table's coordinates do not make a grid.
    #[error(transparent)]
    Grid(#[from] GridError),
}

impl ErddapError {
    /// Whether the same request may succeed after a pause.
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::Temporary | Self::RateLimited { .. })
    }
}

/// Dataset ids and variable names are letters, digits and underscores.
fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// The ERDDAP server of a source; a host that is not an ERDDAP server is refused.
fn erddap_base(source: &GridSource) -> Result<&'static str, ErddapError> {
    source
        .node
        .erddap_base_url()
        .ok_or(ErddapError::InvalidRequest("host is not an ERDDAP server"))
}

fn check_source(source: &GridSource) -> Result<&'static str, ErddapError> {
    let base = erddap_base(source)?;
    if !valid_name(&source.dataset) {
        return Err(ErddapError::InvalidRequest("dataset id"));
    }
    if source.variables.is_empty() {
        return Err(ErddapError::InvalidRequest("no variables"));
    }
    if !source.variables.iter().all(|v| valid_name(v)) {
        return Err(ErddapError::InvalidRequest("variable name"));
    }
    Ok(base)
}

/// A coordinate as ERDDAP's recorded requests write it: up to six decimals, at least one
/// (`40.0`, `-74.2`, `40.99`).
fn fmt_deg(value: f64) -> String {
    let scale = 10_f64.powi(COORDINATE_DECIMALS);
    let rounded = (value * scale).round() / scale;
    // Avoid `-0.0`.
    let rounded = if rounded == 0.0 { 0.0 } else { rounded };
    let text = rounded.to_string();
    if text.contains('.') {
        text
    } else {
        format!("{text}.0")
    }
}

/// `YYYY-MM-DDTHH:MM:SSZ`.
fn fmt_time(t: UnixTime) -> String {
    let l = t.to_local(UtcOffset(0));
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        l.date.year, l.date.month, l.date.day, l.hour, l.minute, l.second
    )
}

/// One axis constraint, `[(lo):stride:(hi)]` encoded; stride 1 is left out, as in
/// `[(40.0):(40.8)]`.
fn range(lo: f64, hi: f64, stride: u32) -> String {
    if stride > 1 {
        format!("%5B({}):{stride}:({})%5D", fmt_deg(lo), fmt_deg(hi))
    } else {
        format!("%5B({}):({})%5D", fmt_deg(lo), fmt_deg(hi))
    }
}

/// The request for `source`'s variables at `time` over `area`, every `stride`-th cell (0 counts
/// as 1).
///
/// The box is clipped to -90..90 and -180..180 (the datasets used have longitudes -180..180).
/// Each variable gets its own constraints, as ERDDAP wants:
/// `water_u[(last)][(40.0):(40.8)][(-74.2):(-73.0)],water_v[...]`. A dataset with an altitude
/// axis gets `[(0.0)]` between time and latitude.
///
/// # Errors
/// [`ErddapError::InvalidRequest`] for a dataset id or variable that is not a plain name, no
/// variables, or a box with a non-finite edge, `west` east of `east` or `south` north of `north`.
pub fn grid_request(
    source: &GridSource,
    time: GridTime,
    area: Bbox,
    stride: u32,
) -> Result<ErddapRequest, ErddapError> {
    let base = check_source(source)?;
    let edges = [area.west, area.south, area.east, area.north];
    if edges.iter().any(|e| !e.is_finite()) || area.west > area.east || area.south > area.north {
        return Err(ErddapError::InvalidRequest("box"));
    }
    let clip = |v: f64, limit: f64| v.clamp(-limit, limit);
    let stride = stride.max(1);
    let time = match time {
        GridTime::Last => "%5B(last)%5D".to_owned(),
        GridTime::At(t) => format!("%5B({})%5D", fmt_time(t)),
    };
    let altitude = if source.altitude_axis {
        "%5B(0.0)%5D"
    } else {
        ""
    };
    let lat = range(clip(area.south, 90.0), clip(area.north, 90.0), stride);
    let lon = range(clip(area.west, 180.0), clip(area.east, 180.0), stride);
    let query: Vec<String> = source
        .variables
        .iter()
        .map(|v| format!("{v}{time}{altitude}{lat}{lon}"))
        .collect();
    Ok(ErddapRequest {
        node: source.node,
        url: format!(
            "{}/griddap/{}.json?{}",
            base,
            source.dataset,
            query.join(",")
        ),
        resource: ErddapResource::Grid,
    })
}

/// The request for one cache block: its area without the northern and eastern edge cells, at
/// its stride.
///
/// # Errors
/// As [`grid_request`].
pub fn block_request(
    source: &GridSource,
    time: GridTime,
    block: Block,
) -> Result<ErddapRequest, ErddapError> {
    grid_request(
        source,
        time,
        block.request_bbox(source.native_step_deg),
        block.stride,
    )
}

/// The request for the times `source` holds from the start of the UTC day `since` to the newest:
/// `time[(2026-10-20):1:(last)]`. Ten to fourteen days are under a kilobyte.
///
/// # Errors
/// [`ErddapError::InvalidRequest`] for a dataset id that is not a plain name.
pub fn time_axis_request(
    source: &GridSource,
    since: LocalDate,
) -> Result<ErddapRequest, ErddapError> {
    let base = erddap_base(source)?;
    if !valid_name(&source.dataset) {
        return Err(ErddapError::InvalidRequest("dataset id"));
    }
    Ok(ErddapRequest {
        node: source.node,
        url: format!(
            "{}/griddap/{}.json?time%5B({:04}-{:02}-{:02}):1:(last)%5D",
            base, source.dataset, since.year, since.month, since.day
        ),
        resource: ErddapResource::TimeAxis,
    })
}

fn contains(haystack: &[u8], needle: &str) -> bool {
    haystack
        .windows(needle.len())
        .any(|w| w == needle.as_bytes())
}

/// The body of a successful response, or what went wrong.
///
/// # Errors
/// [`ErddapError::NoData`] for a 404 with "Your query produced no matching results",
/// [`ErddapError::Temporary`] for a 503 (ERDDAP's own says "There was a (temporary?) problem";
/// a proxy's says less), [`ErddapError::RateLimited`] for a 429, [`ErddapError::Status`] for
/// any other failure, and [`ErddapError::BadTable`] for a success that is an HTML page.
pub fn check_response<'a>(response: &Response<'a>) -> Result<&'a [u8], ErddapError> {
    match response.status {
        200..=299 => {
            let html = response
                .content_type
                .is_some_and(|c| c.to_ascii_lowercase().contains("html"));
            if html {
                Err(ErddapError::BadTable("an HTML page".to_owned()))
            } else {
                Ok(response.body)
            }
        }
        404 if contains(response.body, NO_MATCHING_RESULTS) => Err(ErddapError::NoData),
        429 => Err(ErddapError::RateLimited {
            retry_after_s: response.retry_after,
        }),
        503 => Err(ErddapError::Temporary),
        status => Err(ErddapError::Status { status }),
    }
}

#[derive(Deserialize)]
struct Envelope {
    table: Table,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Table {
    column_names: Vec<String>,
    #[serde(default)]
    column_units: Vec<Option<String>>,
    rows: Vec<Vec<serde_json::Value>>,
}

impl Table {
    fn read(body: &[u8]) -> Result<Self, ErddapError> {
        serde_json::from_slice::<Envelope>(body)
            .map(|e| e.table)
            .map_err(|e| ErddapError::BadTable(e.to_string()))
    }

    fn column(&self, name: &str) -> Result<usize, ErddapError> {
        self.column_names
            .iter()
            .position(|c| c == name)
            .ok_or_else(|| ErddapError::BadTable(format!("no {name} column")))
    }

    fn unit(&self, column: usize) -> String {
        self.column_units
            .get(column)
            .cloned()
            .flatten()
            .unwrap_or_default()
    }
}

fn bad(what: &str) -> ErddapError {
    ErddapError::BadTable(what.to_owned())
}

fn coordinate(row: &[serde_json::Value], column: usize, name: &str) -> Result<f64, ErddapError> {
    row.get(column)
        .and_then(serde_json::Value::as_f64)
        .filter(|v| v.is_finite())
        .ok_or_else(|| ErddapError::BadTable(format!("a row without a {name}")))
}

/// A value cell: a number, or `null` for missing.
fn cell(row: &[serde_json::Value], column: usize) -> Result<f32, ErddapError> {
    match row.get(column) {
        Some(serde_json::Value::Null) => Ok(f32::NAN),
        Some(v) => v
            .as_f64()
            .map(|v| v as f32)
            .ok_or_else(|| bad("a value that is not a number")),
        None => Err(bad("a short row")),
    }
}

fn sorted_axis(mut axis: Vec<f64>) -> Vec<f64> {
    axis.sort_by(f64::total_cmp);
    axis.dedup();
    axis
}

/// The name of a land/sea flag variable. Where a source lists it, [`parse_grids`] blanks the
/// other variables on land cells and returns no grid for it.
pub const MASK_VARIABLE: &str = "mask";

/// The bit of the MUR `mask` flag that marks land (`flag_masks 1, 2, 4, 8, 16` meaning open sea,
/// land, open lake, open sea with ice, open lake with ice). Lakes, with or without ice, are water
/// and keep their values.
const LAND_FLAG: i64 = 2;

/// One grid per variable of `source`, in its order, from a griddap JSON table.
///
/// A variable named [`MASK_VARIABLE`] is a land/sea flag: its cells with the land bit set turn
/// every other variable to `NaN` there, and no grid is returned for it. A cell without a flag
/// keeps its values.
///
/// Every row is placed by its own latitude and longitude, so the order of the rows does not
/// matter; the grids' latitudes ascend. All rows must share one time.
///
/// # Errors
/// [`ErddapError::BadTable`] for a body that is not a table, a missing `time`, `latitude`,
/// `longitude` or variable column, rows of more than one time, a cell listed twice or a table
/// that is not a full grid; [`ErddapError::NoData`] for a table without rows.
pub fn parse_grids(source: &GridSource, body: &[u8]) -> Result<Vec<Grid>, ErddapError> {
    if source.variables.is_empty() {
        return Err(ErddapError::InvalidRequest("no variables"));
    }
    let table = Table::read(body)?;
    let (time_col, lat_col, lon_col) = (
        table.column("time")?,
        table.column("latitude")?,
        table.column("longitude")?,
    );
    let var_cols = source
        .variables
        .iter()
        .map(|v| table.column(v))
        .collect::<Result<Vec<_>, _>>()?;
    let time_text = table
        .rows
        .first()
        .ok_or(ErddapError::NoData)?
        .get(time_col)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| bad("a row without a time"))?;
    let time = parse_rfc3339(time_text).ok_or_else(|| bad("a time that is not ISO 8601"))?;

    let mut lats = Vec::with_capacity(table.rows.len());
    let mut lons = Vec::with_capacity(table.rows.len());
    for row in &table.rows {
        if row.get(time_col).and_then(serde_json::Value::as_str) != Some(time_text) {
            return Err(bad("rows of more than one time"));
        }
        lats.push(coordinate(row, lat_col, "latitude")?);
        lons.push(coordinate(row, lon_col, "longitude")?);
    }
    let lats = sorted_axis(lats);
    let lons = sorted_axis(lons);
    if lats.len().checked_mul(lons.len()) != Some(table.rows.len()) {
        return Err(bad("rows that do not make a full grid"));
    }

    let cells = table.rows.len();
    let mut seen = vec![false; cells];
    let mut values = vec![vec![f32::NAN; cells]; var_cols.len()];
    let mask_at = source.variables.iter().position(|v| v == MASK_VARIABLE);
    let mut land = vec![false; cells];
    for row in &table.rows {
        let lat = coordinate(row, lat_col, "latitude")?;
        let lon = coordinate(row, lon_col, "longitude")?;
        let (Ok(i), Ok(j)) = (
            lats.binary_search_by(|v| v.total_cmp(&lat)),
            lons.binary_search_by(|v| v.total_cmp(&lon)),
        ) else {
            return Err(bad("a row off the grid"));
        };
        let at = i * lons.len() + j;
        match seen.get_mut(at) {
            Some(s) if !*s => *s = true,
            _ => return Err(bad("a cell listed twice")),
        }
        for (column, grid) in var_cols.iter().zip(values.iter_mut()) {
            if let Some(slot) = grid.get_mut(at) {
                *slot = cell(row, *column)?;
            }
        }
        if let (Some(m), Some(is_land)) = (mask_at, land.get_mut(at)) {
            let flag = values.get(m).and_then(|g| g.get(at)).copied();
            *is_land = flag.is_some_and(|f| f.is_finite() && (f as i64) & LAND_FLAG != 0);
        }
    }

    var_cols
        .iter()
        .zip(values)
        .enumerate()
        .filter(|(i, _)| Some(*i) != mask_at)
        .map(|(_, (&column, mut values))| {
            if mask_at.is_some() {
                for (v, _) in values.iter_mut().zip(&land).filter(|(_, l)| **l) {
                    *v = f32::NAN;
                }
            }
            Grid::new(
                source.clone(),
                time,
                lats.clone(),
                lons.clone(),
                values,
                table.unit(column),
            )
            .map_err(ErddapError::from)
        })
        .collect()
}

/// The grid of `source`'s first variable from a griddap JSON table (see [`parse_grids`]).
///
/// # Errors
/// As [`parse_grids`].
pub fn parse_grid(source: &GridSource, body: &[u8]) -> Result<Grid, ErddapError> {
    parse_grids(source, body)?
        .into_iter()
        .next()
        .ok_or(ErddapError::InvalidRequest("no variables"))
}

/// Surface currents: eastward and northward components on one grid, metres per second.
#[derive(Clone, Debug, PartialEq)]
pub struct CurrentGrid {
    /// Eastward velocity (`water_u`).
    pub u: Grid,
    /// Northward velocity (`water_v`).
    pub v: Grid,
}

impl CurrentGrid {
    /// How many cells have both components.
    pub fn both_count(&self) -> usize {
        self.u
            .values
            .iter()
            .zip(&self.v.values)
            .filter(|(u, v)| u.is_finite() && v.is_finite())
            .count()
    }
}

/// Whether a unit is metres per second as ERDDAP writes it (`m s-1`, `m/s`, `m s^-1`).
fn is_metres_per_second(unit: &str) -> bool {
    let compact: String = unit
        .chars()
        .filter(|c| !matches!(c, ' ' | '.' | '^'))
        .collect();
    matches!(compact.as_str(), "ms-1" | "m/s")
}

/// HF radar currents from a table with `water_u` and `water_v` (`ucsdHfrE6`).
///
/// # Errors
/// [`ErddapError::InvalidRequest`] when `source` does not ask for both components;
/// [`ErddapError::BadTable`] when either is not in metres per second; otherwise as
/// [`parse_grids`].
pub fn parse_currents(source: &GridSource, body: &[u8]) -> Result<CurrentGrid, ErddapError> {
    let index = |name: &str| {
        source
            .variables
            .iter()
            .position(|v| v == name)
            .ok_or(ErddapError::InvalidRequest(
                "currents need water_u and water_v",
            ))
    };
    let (u_at, v_at) = (index("water_u")?, index("water_v")?);
    let grids = parse_grids(source, body)?;
    let take = |at: usize| {
        let grid = grids
            .get(at)
            .cloned()
            .ok_or_else(|| bad("a missing component"))?;
        if is_metres_per_second(&grid.unit) {
            Ok(grid)
        } else {
            Err(ErddapError::BadTable(format!(
                "currents in {:?}, not m s-1",
                grid.unit
            )))
        }
    };
    Ok(CurrentGrid {
        u: take(u_at)?,
        v: take(v_at)?,
    })
}

/// The times a dataset holds, ascending.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TimeAxis {
    times: Vec<UnixTime>,
}

impl TimeAxis {
    /// The times, ascending, each once.
    pub fn times(&self) -> &[UnixTime] {
        &self.times
    }

    /// The newest time.
    pub fn latest(&self) -> Option<UnixTime> {
        self.times.last().copied()
    }

    /// The time listed on the UTC day `day` (the last, if several): the way to ask for a date,
    /// since the time of day varies between datasets and, for some, between days.
    pub fn on_day(&self, day: LocalDate) -> Option<UnixTime> {
        self.times
            .iter()
            .rev()
            .find(|t| t.local_date(UtcOffset(0)) == day)
            .copied()
    }

    /// The newest time at or before `t`, for stepping back to an earlier day.
    pub fn at_or_before(&self, t: UnixTime) -> Option<UnixTime> {
        let k = self.times.partition_point(|&x| x <= t);
        k.checked_sub(1).and_then(|i| self.times.get(i)).copied()
    }
}

/// The times of a `time[(start):1:(last)]` response.
///
/// # Errors
/// [`ErddapError::BadTable`] for a body that is not a table, has no `time` column, or lists a
/// time that is not ISO 8601.
pub fn parse_time_axis(body: &[u8]) -> Result<TimeAxis, ErddapError> {
    let table = Table::read(body)?;
    let column = table.column("time")?;
    let mut times = table
        .rows
        .iter()
        .map(|row| {
            row.get(column)
                .and_then(serde_json::Value::as_str)
                .and_then(parse_rfc3339)
                .ok_or_else(|| bad("a time that is not ISO 8601"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    times.sort_unstable();
    times.dedup();
    Ok(TimeAxis { times })
}

/// Why a request to a node must wait.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("wait until {} before the next request to this ERDDAP node", until.0)]
pub struct PaceWait {
    /// When the next request may be sent.
    pub until: UnixTime,
}

impl PaceWait {
    /// Seconds from `now` until then.
    pub fn after_s(self, now: UnixTime) -> u32 {
        u32::try_from(self.until.0.saturating_sub(now.0).max(0)).unwrap_or(u32::MAX)
    }
}

/// One node's recent requests and pause.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct NodePace {
    /// When requests were sent in the last minute, ascending.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    sent: Vec<UnixTime>,
    /// No request before this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    paused_until: Option<UnixTime>,
    /// The length of the last back-off, seconds; cleared when the node answers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_pause_s: Option<i64>,
}

impl NodePace {
    /// Forgets requests older than the window, and any a clock that went back puts ahead.
    fn prune(&mut self, now: UnixTime) {
        self.sent
            .retain(|t| t.0 <= now.0 && now.0 - t.0 < PACE_WINDOW_S);
    }

    fn ready_at(&self, now: UnixTime) -> UnixTime {
        let mut ready = now.0;
        if let Some(until) = self.paused_until {
            // A clock that went back cannot stretch a pause past the longest one asked for.
            ready = ready.max(until.0.min(now.0 + MAX_RETRY_AFTER_S.max(MAX_PAUSE_S)));
        }
        let recent: Vec<i64> = self
            .sent
            .iter()
            .map(|t| t.0)
            .filter(|&t| t <= now.0 && now.0 - t < PACE_WINDOW_S)
            .collect();
        if let Some(oldest) = recent
            .len()
            .checked_sub(MAX_REQUESTS_PER_MINUTE)
            .and_then(|k| recent.get(k))
        {
            ready = ready.max(oldest + PACE_WINDOW_S);
        }
        UnixTime(ready)
    }
}

/// Request pacing per ERDDAP node, kept by the host between fetches and saved as JSON.
///
/// At most [`MAX_REQUESTS_PER_MINUTE`] requests go to a node in any minute. After a 429 or 503
/// the node is paused for [`FIRST_PAUSE_S`], doubling with each further one up to
/// [`MAX_PAUSE_S`] (or longer if the server's `Retry-After` says so, up to an hour); an answer
/// from the node resets the doubling. Serialized as
/// `{"nodes":{"pfeg":{"sent":[1794000000],"paused_until":1794000060,"last_pause_s":60}}}`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErddapPacer {
    #[serde(default)]
    nodes: BTreeMap<GridHost, NodePace>,
}

impl ErddapPacer {
    /// A pacer with no history.
    pub fn new() -> Self {
        Self::default()
    }

    /// When the next request to `node` may be sent: `now` if it may go now.
    pub fn ready_at(&self, node: GridHost, now: UnixTime) -> UnixTime {
        self.nodes.get(&node).map_or(now, |pace| pace.ready_at(now))
    }

    /// Records a request to `node` sent at `now`, if one may be sent.
    ///
    /// # Errors
    /// [`PaceWait`] with the time the next request may go, when the node is paused or has had
    /// its requests for this minute.
    pub fn try_send(&mut self, node: GridHost, now: UnixTime) -> Result<(), PaceWait> {
        let pace = self.nodes.entry(node).or_default();
        pace.prune(now);
        let until = pace.ready_at(now);
        if until.0 > now.0 {
            return Err(PaceWait { until });
        }
        pace.sent.push(now);
        Ok(())
    }

    /// Records the answer to a request to `node`: a 429 or 503 pauses the node; any other
    /// answer the node itself gave (2xx, or 404 for an empty subset) resets the back-off.
    pub fn on_response(&mut self, node: GridHost, response: &Response<'_>, now: UnixTime) {
        let pace = self.nodes.entry(node).or_default();
        match response.status {
            429 | 503 => {
                let pause = pace
                    .last_pause_s
                    .map_or(FIRST_PAUSE_S, |p| p.saturating_mul(2).min(MAX_PAUSE_S));
                let asked = response
                    .retry_after
                    .map_or(0, |s| i64::from(s).min(MAX_RETRY_AFTER_S));
                let until = now.0.saturating_add(pause.max(asked));
                pace.paused_until = Some(UnixTime(
                    pace.paused_until.map_or(until, |p| p.0.max(until)),
                ));
                pace.last_pause_s = Some(pause);
            }
            200..=299 | 404 => pace.last_pause_s = None,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::fixture_bytes;
    use underbite_core::map::GeoPoint;

    fn source(
        node: GridHost,
        dataset: &str,
        variables: &[&str],
        altitude: bool,
        step: f64,
    ) -> GridSource {
        GridSource {
            node,
            dataset: dataset.to_owned(),
            variables: variables.iter().map(|v| (*v).to_owned()).collect(),
            altitude_axis: altitude,
            native_step_deg: step,
        }
    }

    fn mur() -> GridSource {
        source(
            GridHost::Pfeg,
            "jplMURSST41",
            &["analysed_sst"],
            false,
            0.01,
        )
    }

    fn mur_masked() -> GridSource {
        source(
            GridHost::Pfeg,
            "jplMURSST41",
            &["analysed_sst", MASK_VARIABLE],
            false,
            0.01,
        )
    }

    fn hf_radar() -> GridSource {
        source(
            GridHost::Pfeg,
            "ucsdHfrE6",
            &["water_u", "water_v"],
            false,
            0.054,
        )
    }

    fn chlorophyll() -> GridSource {
        source(
            GridHost::Coastwatch,
            "noaacwN20VIIRSchlaDaily",
            &["chlor_a"],
            true,
            0.0375,
        )
    }

    fn kd490_4km() -> GridSource {
        source(
            GridHost::Coastwatch,
            "noaacwNPPVIIRSkd490Daily",
            &["kd_490"],
            true,
            0.0375,
        )
    }

    fn kd490() -> GridSource {
        source(
            GridHost::Coastwatch,
            "noaacwN21VIIRSkd490NSWUSDaily",
            &["kd_490"],
            true,
            0.0075,
        )
    }

    fn glsea() -> GridSource {
        source(
            GridHost::Glerl,
            "GLSEA_ACSPO_GCS",
            &["sst"],
            false,
            0.014_015_517,
        )
    }

    /// The test box: 40.0 to 40.8 N, 73.0 to 74.2 W.
    fn harbor() -> Bbox {
        Bbox {
            west: -74.2,
            south: 40.0,
            east: -73.0,
            north: 40.8,
        }
    }

    fn at(s: &str) -> UnixTime {
        parse_rfc3339(s).unwrap()
    }

    fn day(year: i32, month: u8, day: u8) -> LocalDate {
        LocalDate { year, month, day }
    }

    fn url(source: &GridSource, time: GridTime, area: Bbox, stride: u32) -> String {
        grid_request(source, time, area, stride).unwrap().url
    }

    fn bad(what: &str) -> ErddapError {
        ErddapError::BadTable(what.to_owned())
    }

    #[test]
    fn erddap_grid_requests_reproduce_the_recorded_urls() {
        assert_eq!(
            url(&mur(), GridTime::Last, harbor(), 5),
            "https://coastwatch.pfeg.noaa.gov/erddap/griddap/jplMURSST41.json?analysed_sst%5B(last)%5D%5B(40.0):5:(40.8)%5D%5B(-74.2):5:(-73.0)%5D"
        );
        assert_eq!(
            url(&hf_radar(), GridTime::Last, harbor(), 1),
            "https://coastwatch.pfeg.noaa.gov/erddap/griddap/ucsdHfrE6.json?water_u%5B(last)%5D%5B(40.0):(40.8)%5D%5B(-74.2):(-73.0)%5D,water_v%5B(last)%5D%5B(40.0):(40.8)%5D%5B(-74.2):(-73.0)%5D"
        );
        let nov4 = GridTime::At(at("2026-11-04T12:00:00Z"));
        assert_eq!(
            url(&chlorophyll(), nov4, harbor(), 1),
            "https://coastwatch.noaa.gov/erddap/griddap/noaacwN20VIIRSchlaDaily.json?chlor_a%5B(2026-11-04T12:00:00Z)%5D%5B(0.0)%5D%5B(40.0):(40.8)%5D%5B(-74.2):(-73.0)%5D"
        );
        assert_eq!(
            url(&kd490_4km(), nov4, harbor(), 1),
            "https://coastwatch.noaa.gov/erddap/griddap/noaacwNPPVIIRSkd490Daily.json?kd_490%5B(2026-11-04T12:00:00Z)%5D%5B(0.0)%5D%5B(40.0):(40.8)%5D%5B(-74.2):(-73.0)%5D"
        );
        assert_eq!(
            url(
                &kd490_4km(),
                GridTime::At(at("2026-11-12T12:00:00Z")),
                harbor(),
                1
            ),
            "https://coastwatch.noaa.gov/erddap/griddap/noaacwNPPVIIRSkd490Daily.json?kd_490%5B(2026-11-12T12:00:00Z)%5D%5B(0.0)%5D%5B(40.0):(40.8)%5D%5B(-74.2):(-73.0)%5D"
        );
        assert_eq!(
            url(
                &kd490(),
                GridTime::At(at("2026-11-11T19:27:55Z")),
                harbor(),
                6
            ),
            "https://coastwatch.noaa.gov/erddap/griddap/noaacwN21VIIRSkd490NSWUSDaily.json?kd_490%5B(2026-11-11T19:27:55Z)%5D%5B(0.0)%5D%5B(40.0):6:(40.8)%5D%5B(-74.2):6:(-73.0)%5D"
        );
        let erie = Bbox {
            west: -83.5,
            south: 41.4,
            east: -82.5,
            north: 42.0,
        };
        assert_eq!(
            url(&kd490(), GridTime::At(at("2026-11-10T18:59:54Z")), erie, 6),
            "https://coastwatch.noaa.gov/erddap/griddap/noaacwN21VIIRSkd490NSWUSDaily.json?kd_490%5B(2026-11-10T18:59:54Z)%5D%5B(0.0)%5D%5B(41.4):6:(42.0)%5D%5B(-83.5):6:(-82.5)%5D"
        );
        let erie_west = Bbox {
            west: -83.6,
            south: 41.3,
            east: -82.6,
            north: 42.1,
        };
        assert_eq!(
            url(&glsea(), GridTime::Last, erie_west, 2),
            "https://apps.glerl.noaa.gov/erddap/griddap/GLSEA_ACSPO_GCS.json?sst%5B(last)%5D%5B(41.3):2:(42.1)%5D%5B(-83.6):2:(-82.6)%5D"
        );
    }

    #[test]
    fn erddap_time_axis_requests_reproduce_the_recorded_urls() {
        let axis = |s: &GridSource, since: LocalDate| {
            let r = time_axis_request(s, since).unwrap();
            assert_eq!((r.node, r.resource), (s.node, ErddapResource::TimeAxis));
            r.url
        };
        assert_eq!(
            axis(&kd490(), day(2026, 10, 20)),
            "https://coastwatch.noaa.gov/erddap/griddap/noaacwN21VIIRSkd490NSWUSDaily.json?time%5B(2026-10-20):1:(last)%5D"
        );
        let n20 = source(
            GridHost::Coastwatch,
            "noaacwN20VIIRSkd490NSWUSDaily",
            &["kd_490"],
            true,
            0.0075,
        );
        assert_eq!(
            axis(&n20, day(2026, 10, 20)),
            "https://coastwatch.noaa.gov/erddap/griddap/noaacwN20VIIRSkd490NSWUSDaily.json?time%5B(2026-10-20):1:(last)%5D"
        );
        assert_eq!(
            axis(&kd490_4km(), day(2026, 10, 20)),
            "https://coastwatch.noaa.gov/erddap/griddap/noaacwNPPVIIRSkd490Daily.json?time%5B(2026-10-20):1:(last)%5D"
        );
        let modis = source(
            GridHost::Pfeg,
            "erdMH1kd4901day_R2022NRT",
            &["Kd_490"],
            true,
            0.041_666_7,
        );
        assert_eq!(
            axis(&modis, day(2026, 11, 5)),
            "https://coastwatch.pfeg.noaa.gov/erddap/griddap/erdMH1kd4901day_R2022NRT.json?time%5B(2026-11-05):1:(last)%5D"
        );
        assert_eq!(
            axis(&chlorophyll(), day(2026, 10, 15)),
            "https://coastwatch.noaa.gov/erddap/griddap/noaacwN20VIIRSchlaDaily.json?time%5B(2026-10-15):1:(last)%5D"
        );
        assert_eq!(
            axis(&glsea(), day(2026, 11, 5)),
            "https://apps.glerl.noaa.gov/erddap/griddap/GLSEA_ACSPO_GCS.json?time%5B(2026-11-05):1:(last)%5D"
        );
    }

    #[test]
    fn erddap_block_requests_stop_one_step_short_of_the_far_edges() {
        let block = Block {
            south: 40,
            west: -74,
            stride: 5,
        };
        let r = block_request(&mur(), GridTime::At(at("2026-11-07T09:00:00Z")), block).unwrap();
        assert_eq!((r.node, r.resource), (GridHost::Pfeg, ErddapResource::Grid));
        assert_eq!(
            r.url,
            "https://coastwatch.pfeg.noaa.gov/erddap/griddap/jplMURSST41.json?analysed_sst%5B(2026-11-07T09:00:00Z)%5D%5B(40.0):5:(40.99)%5D%5B(-74.0):5:(-73.01)%5D"
        );
        let block = Block {
            south: 41,
            west: -84,
            stride: 1,
        };
        assert_eq!(
            block_request(&glsea(), GridTime::Last, block).unwrap().url,
            "https://apps.glerl.noaa.gov/erddap/griddap/GLSEA_ACSPO_GCS.json?sst%5B(last)%5D%5B(41.0):(41.985984)%5D%5B(-84.0):(-83.014016)%5D"
        );
    }

    #[test]
    fn erddap_requests_clip_the_box_and_refuse_bad_input() {
        let wide = Bbox {
            west: -190.0,
            south: -95.0,
            east: 200.0,
            north: 95.0,
        };
        assert_eq!(
            url(&mur(), GridTime::Last, wide, 0),
            "https://coastwatch.pfeg.noaa.gov/erddap/griddap/jplMURSST41.json?analysed_sst%5B(last)%5D%5B(-90.0):(90.0)%5D%5B(-180.0):(180.0)%5D"
        );
        let tiny = Bbox {
            west: -0.000_000_1,
            south: 1e-9,
            east: 0.123_456_789,
            north: 0.5,
        };
        assert_eq!(
            url(&mur(), GridTime::Last, tiny, 1),
            "https://coastwatch.pfeg.noaa.gov/erddap/griddap/jplMURSST41.json?analysed_sst%5B(last)%5D%5B(0.0):(0.5)%5D%5B(0.0):(0.123457)%5D"
        );
        let flipped = Bbox {
            west: -73.0,
            east: -74.2,
            ..harbor()
        };
        assert_eq!(
            grid_request(&mur(), GridTime::Last, flipped, 1),
            Err(ErddapError::InvalidRequest("box"))
        );
        let nan = Bbox {
            north: f64::NAN,
            ..harbor()
        };
        assert_eq!(
            grid_request(&mur(), GridTime::Last, nan, 1),
            Err(ErddapError::InvalidRequest("box"))
        );
        let injected = source(
            GridHost::Pfeg,
            "jplMURSST41",
            &["analysed_sst[0]"],
            false,
            0.01,
        );
        assert_eq!(
            grid_request(&injected, GridTime::Last, harbor(), 1),
            Err(ErddapError::InvalidRequest("variable name"))
        );
        let none = source(GridHost::Pfeg, "jplMURSST41", &[], false, 0.01);
        assert_eq!(
            grid_request(&none, GridTime::Last, harbor(), 1),
            Err(ErddapError::InvalidRequest("no variables"))
        );
        let odd_id = source(GridHost::Pfeg, "../x", &["sst"], false, 0.01);
        assert_eq!(
            time_axis_request(&odd_id, day(2026, 11, 5)),
            Err(ErddapError::InvalidRequest("dataset id"))
        );
        let dem = source(
            GridHost::NceiDem,
            "DEM_global_mosaic",
            &["elevation"],
            false,
            0.005,
        );
        let not_erddap = Err(ErddapError::InvalidRequest("host is not an ERDDAP server"));
        assert_eq!(grid_request(&dem, GridTime::Last, harbor(), 1), not_erddap);
        assert_eq!(time_axis_request(&dem, day(2026, 11, 5)), not_erddap);
    }

    #[test]
    fn erddap_mur_mask_blanks_land_and_keeps_open_water() {
        let body = fixture_bytes("erddap", "jplmursst41-sst-mask-subset-2026-11-17.json");
        let grids = parse_grids(&mur_masked(), &body).unwrap();
        // The mask is consumed: one grid, for the temperature.
        assert_eq!(grids.len(), 1);
        let g = &grids[0];
        assert_eq!((g.lats.len(), g.lons.len()), (17, 25));
        assert_eq!(g.unit, "degree_C");
        let at = |lat: f64, lon: f64| g.sample_nearest(GeoPoint { lat, lon }).unwrap_or(f32::NAN);
        // Central Islip: inland, the source holds 13.585 there but flags it as land.
        assert!(at(40.8, -73.2).is_nan());
        // Open water south of Long Island keeps its value.
        assert_eq!(at(40.4, -73.2), 14.656);
        // A cell the source fills with a value but flags as land (Staten Island side).
        assert!(at(40.6, -74.15).is_nan());
        // Only open-sea cells (flag 1) remain.
        assert_eq!(g.valid_count(), 292);
    }

    #[test]
    fn erddap_mur_mask_leaves_land_transparent_in_a_rendered_tile() {
        use underbite_core::map::TileKey;
        use underbite_core::map::render::{RampRange, ramp, render_rgba};
        let body = fixture_bytes("erddap", "jplmursst41-sst-mask-subset-2026-11-17.json");
        let grids = parse_grids(&mur_masked(), &body).unwrap();
        let thermal = ramp("thermal").unwrap();
        let range = RampRange::linear(10.0, 20.0);
        // Zoom 9 tile over the box; find the pixel of a point by Web Mercator.
        let (z, size) = (9_u8, 256_u32);
        let n = f64::from(1_u32 << z);
        let world = |lat: f64, lon: f64| {
            let x = (lon + 180.0) / 360.0 * n;
            let y = (1.0
                - (lat.to_radians().tan() + 1.0 / lat.to_radians().cos()).ln()
                    / std::f64::consts::PI)
                / 2.0
                * n;
            (x, y)
        };
        let (cx, cy) = world(40.8, -73.2);
        let tile = TileKey {
            z,
            x: cx.floor() as u32,
            y: cy.floor() as u32,
        };
        let rgba = render_rgba(&grids, thermal, range, tile, size).unwrap();
        let alpha = |lat: f64, lon: f64| {
            let (x, y) = world(lat, lon);
            let px = ((x - f64::from(tile.x)) * f64::from(size)) as usize;
            let py = ((y - f64::from(tile.y)) * f64::from(size)) as usize;
            rgba[(py * size as usize + px) * 4 + 3]
        };
        assert_eq!(alpha(40.8, -73.2), 0, "Central Islip must be transparent");
        // The point just offshore is on the same tile? Use any opaque pixel in the tile.
        assert!(
            rgba.iter().skip(3).step_by(4).any(|a| *a > 0),
            "open water is drawn"
        );
    }

    #[test]
    fn erddap_old_body_without_the_mask_is_refused_for_a_masked_source() {
        let body = fixture_bytes("erddap", "jplmursst41-sst-subset-2026-11-07.json");
        assert!(matches!(
            parse_grids(&mur_masked(), &body),
            Err(ErddapError::BadTable(_))
        ));
    }

    #[test]
    fn erddap_mur_request_asks_for_the_mask_with_the_same_stride() {
        let r = grid_request(&mur_masked(), GridTime::Last, harbor(), 5).unwrap();
        let query = r.url.split_once('?').unwrap().1;
        let parts: Vec<&str> = query.split(',').collect();
        assert_eq!(parts.len(), 2);
        assert!(parts[0].starts_with("analysed_sst%5B(last)%5D"));
        assert_eq!(
            parts[1].strip_prefix("mask"),
            parts[0].strip_prefix("analysed_sst")
        );
    }

    #[test]
    fn erddap_mur_subset_is_17_by_25_with_land_missing() {
        let body = fixture_bytes("erddap", "jplmursst41-sst-subset-2026-11-07.json");
        let g = parse_grid(&mur(), &body).unwrap();
        assert_eq!((g.lats.len(), g.lons.len()), (17, 25));
        assert_eq!(g.time, at("2026-11-07T09:00:00Z"));
        assert_eq!(g.unit, "degree_C");
        assert_eq!((g.lats[0], g.lons[0]), (40.0, -74.2));
        // The first two cells of the first row are land.
        assert!(g.value(0, 0).unwrap().is_nan() && g.value(0, 1).unwrap().is_nan());
        assert_eq!(g.value(0, 2), Some(15.795));
        assert_eq!(g.valid_count(), 369);
        assert_eq!(g.values.len() - g.valid_count(), 56);
        // Sampling at a cell centre gives that cell.
        let centre = GeoPoint {
            lat: g.lats[0],
            lon: g.lons[2],
        };
        assert_eq!(g.sample_nearest(centre), Some(15.795));
        assert!((g.sample_bilinear(centre).unwrap() - 15.795).abs() < 1e-5);
        let land = GeoPoint {
            lat: g.lats[0],
            lon: g.lons[0],
        };
        assert_eq!(g.sample_bilinear(land), None);
        let (lo, hi) = g.range_in(harbor()).unwrap();
        assert!(
            (lo - 15.536).abs() < 1e-4 && (hi - 16.509).abs() < 1e-4,
            "{lo} {hi}"
        );
    }

    #[test]
    fn erddap_chlorophyll_subset_is_22_by_33_with_latitude_ascending() {
        let body = fixture_bytes(
            "erddap",
            "noaacwn20viirschladaily-chla-subset-2026-11-04.json",
        );
        let g = parse_grid(&chlorophyll(), &body).unwrap();
        assert_eq!((g.lats.len(), g.lons.len()), (22, 33));
        assert!(g.lats.windows(2).all(|w| w[0] < w[1]));
        assert_eq!((g.lats[0], g.lats[21]), (39.99375, 40.78125));
        assert_eq!(g.unit, "mg m^-3");
        assert_eq!(g.time, at("2026-11-04T12:00:00Z"));
        assert_eq!(g.valid_count(), 555);
        // Each row's value lands at its own coordinates, whatever the order of the rows.
        let table: serde_json::Value = serde_json::from_slice(&body).unwrap();
        for row in table["table"]["rows"].as_array().unwrap() {
            let (lat, lon) = (row[2].as_f64().unwrap(), row[3].as_f64().unwrap());
            let i = g
                .lats
                .iter()
                .position(|v| v.total_cmp(&lat).is_eq())
                .unwrap();
            let j = g
                .lons
                .iter()
                .position(|v| v.total_cmp(&lon).is_eq())
                .unwrap();
            let got = g.value(i, j).unwrap();
            match row[4].as_f64() {
                Some(want) => assert_eq!(got, want as f32),
                None => assert!(got.is_nan()),
            }
        }
    }

    #[test]
    fn erddap_hf_radar_has_both_components_in_190_of_315_cells() {
        let body = fixture_bytes("erddap", "ucsdhfre6-currents-subset-2026-11-08t08.json");
        let c = parse_currents(&hf_radar(), &body).unwrap();
        assert_eq!((c.u.lats.len(), c.u.lons.len()), (15, 21));
        assert_eq!(c.u.values.len(), 315);
        assert_eq!(c.both_count(), 190);
        assert_eq!(c.u.time, at("2026-11-08T08:00:00Z"));
        assert_eq!((c.u.unit.as_str(), c.v.unit.as_str()), ("m s-1", "m s-1"));
        assert_eq!((&c.u.lats, &c.u.lons), (&c.v.lats, &c.v.lons));
        // The fourth cell of the southern row: 0.12 east, 0.07 north.
        assert_eq!((c.u.value(0, 3), c.v.value(0, 3)), (Some(0.12), Some(0.07)));
        let both = parse_grids(&hf_radar(), &body).unwrap();
        assert_eq!(both.len(), 2);
        assert_eq!(both[1].value(0, 3), Some(0.07));
        assert_eq!(
            parse_currents(&mur(), &body),
            Err(ErddapError::InvalidRequest(
                "currents need water_u and water_v"
            ))
        );
    }

    #[test]
    fn erddap_kd490_day_comes_from_the_time_axis() {
        let axis = parse_time_axis(&fixture_bytes(
            "erddap",
            "noaacwn21viirskd490nswusdaily-time-2026-10-20-to-last.json",
        ))
        .unwrap();
        assert_eq!(axis.times().len(), 20);
        assert_eq!(axis.latest(), Some(at("2026-11-14T19:22:22Z")));
        // 2026-11-11 is a pass at 19:27:55Z, not noon; 2026-11-05 has none.
        let t = axis.on_day(day(2026, 11, 11)).unwrap();
        assert_eq!(t, at("2026-11-11T19:27:55Z"));
        assert_eq!(axis.on_day(day(2026, 11, 5)), None);
        assert_eq!(
            axis.at_or_before(at("2026-11-05T23:59:59Z")),
            Some(at("2026-11-04T19:11:01Z"))
        );
        assert_eq!(axis.at_or_before(at("2026-10-01T00:00:00Z")), None);
        assert_eq!(
            url(&kd490(), GridTime::At(t), harbor(), 6),
            "https://coastwatch.noaa.gov/erddap/griddap/noaacwN21VIIRSkd490NSWUSDaily.json?kd_490%5B(2026-11-11T19:27:55Z)%5D%5B(0.0)%5D%5B(40.0):6:(40.8)%5D%5B(-74.2):6:(-73.0)%5D"
        );

        let body = fixture_bytes(
            "erddap",
            "noaacwn21viirskd490nswusdaily-kd490-ny-subset-2026-11-11.json",
        );
        let g = parse_grid(&kd490(), &body).unwrap();
        assert_eq!(g.time, t);
        assert_eq!((g.lats.len(), g.lons.len()), (18, 27));
        assert_eq!((g.values.len(), g.valid_count()), (486, 313));
        assert_eq!(g.unit, "m^-1");
        assert!(g.lats.windows(2).all(|w| w[0] < w[1]));
        assert_eq!(g.lats.last(), Some(&40.79625));
        let (lo, hi) = g.range_in(harbor()).unwrap();
        assert!(
            (lo - 0.081).abs() < 1e-4 && (hi - 1.478).abs() < 1e-4,
            "{lo} {hi}"
        );

        let erie = fixture_bytes(
            "erddap",
            "noaacwn21viirskd490nswusdaily-kd490-erie-subset-2026-11-10.json",
        );
        let g = parse_grid(&kd490(), &erie).unwrap();
        assert_eq!((g.values.len(), g.valid_count()), (322, 94));
        assert_eq!(Some(g.time), axis.on_day(day(2026, 11, 10)));
    }

    #[test]
    fn erddap_other_time_axes_parse() {
        let n20 = parse_time_axis(&fixture_bytes(
            "erddap",
            "noaacwn20viirskd490nswusdaily-time-2026-10-20-to-last.json",
        ))
        .unwrap();
        assert_eq!(n20.latest(), Some(at("2026-11-10T19:52:32Z")));
        let npp = parse_time_axis(&fixture_bytes(
            "erddap",
            "noaacwnppviirskd490daily-time-2026-10-20-to-last.json",
        ))
        .unwrap();
        assert_eq!(npp.latest(), Some(at("2026-11-12T12:00:00Z")));
        assert_eq!(npp.on_day(day(2026, 10, 31)), None);
        let glerl = parse_time_axis(&fixture_bytes(
            "glerl",
            "glsea-acspo-gcs-time-2026-11-05-to-last.json",
        ))
        .unwrap();
        assert_eq!(glerl.times().len(), 11);
        assert!(
            glerl
                .times()
                .iter()
                .all(|t| t.0.rem_euclid(86_400) == 12 * 3_600)
        );
        assert_eq!(glerl.latest(), Some(at("2026-11-15T12:00:00Z")));
    }

    #[test]
    fn erddap_glsea_subsets_parse_with_latitude_ascending_at_noon() {
        let body = fixture_bytes(
            "glerl",
            "glsea-acspo-gcs-sst-lake-erie-west-2026-11-15.json",
        );
        let g = parse_grid(&glsea(), &body).unwrap();
        assert_eq!(g.time, at("2026-11-15T12:00:00Z"));
        assert_eq!((g.lats.len(), g.lons.len()), (29, 37));
        assert_eq!(g.valid_count(), 428);
        assert_eq!(g.unit, "degree_C");
        assert_eq!(g.lats[0], 41.299_671_692_347_3);
        // The response lists the southernmost row first, as the grid keeps it.
        let table: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(table["table"]["rows"][0][1].as_f64(), Some(g.lats[0]));
        let lake = Bbox {
            west: -84.0,
            south: 41.0,
            east: -82.0,
            north: 43.0,
        };
        let (lo, hi) = g.range_in(lake).unwrap();
        assert!(
            (lo - 10.71).abs() < 1e-4 && (hi - 14.07).abs() < 1e-4,
            "{lo} {hi}"
        );

        let older = fixture_bytes("tier2", "glerl-glsea-acspo-sst-erie-2026-11-07.json");
        let g = parse_grid(&glsea(), &older).unwrap();
        assert_eq!(g.time, at("2026-11-07T12:00:00Z"));
        assert_eq!((g.lats.len(), g.lons.len(), g.valid_count()), (8, 8, 53));
        // The first row is the south-west cell.
        assert_eq!(g.value(0, 0), Some(15.13));
    }

    #[test]
    fn erddap_bad_tables_are_refused() {
        let table = |rows: &str| {
            format!(
                r#"{{"table":{{"columnNames":["time","latitude","longitude","sst"],"columnUnits":["UTC","degrees_north","degrees_east","degree_C"],"rows":[{rows}]}}}}"#
            )
        };
        let t = "\"2026-11-07T12:00:00Z\"";
        let ok = table(&format!("[{t},40,-74,1.5],[{t},40,-73,null]"));
        let g = parse_grid(&glsea(), ok.as_bytes()).unwrap();
        assert_eq!(g.value(0, 0), Some(1.5));
        assert!(g.value(0, 1).unwrap().is_nan());

        let refuse = |body: &str| parse_grid(&glsea(), body.as_bytes()).unwrap_err();
        assert!(matches!(
            refuse("<html>oops</html>"),
            ErddapError::BadTable(_)
        ));
        assert_eq!(refuse(&table("")), ErddapError::NoData);
        assert_eq!(
            refuse(&table(&format!(
                "[{t},40,-74,1],[\"2026-11-08T12:00:00Z\",40,-73,2]"
            ))),
            bad("rows of more than one time")
        );
        assert_eq!(
            refuse(&table(&format!("[{t},40,-74,1],[{t},41,-73,2]"))),
            bad("rows that do not make a full grid")
        );
        assert_eq!(
            refuse(&table(&format!("[{t},40,-74,1],[{t},40,-74,2]"))),
            bad("rows that do not make a full grid")
        );
        assert_eq!(
            refuse(&table(&format!(
                "[{t},40,-74,1],[{t},40,-73,2],[{t},41,-74,3],[{t},40,-74,4]"
            ))),
            bad("a cell listed twice")
        );
        assert_eq!(
            refuse(&table(&format!("[{t},\"forty\",-74,1]"))),
            bad("a row without a latitude")
        );
        assert_eq!(
            refuse(&table(&format!("[{t},40,-74,\"x\"]"))),
            bad("a value that is not a number")
        );
        assert_eq!(
            refuse(&table("[\"yesterday\",40,-74,1]")),
            bad("a time that is not ISO 8601")
        );
        assert_eq!(refuse(&table(&format!("[{t},40,-74]"))), bad("a short row"));
        let other_var = source(GridHost::Glerl, "GLSEA_ACSPO_GCS", &["ice"], false, 0.014);
        assert_eq!(
            parse_grid(&other_var, ok.as_bytes()),
            Err(bad("no ice column"))
        );
        assert!(matches!(
            parse_time_axis(b"[]"),
            Err(ErddapError::BadTable(_))
        ));
    }

    #[test]
    fn erddap_currents_in_other_units_are_refused() {
        let body = br#"{"table":{"columnNames":["time","latitude","longitude","water_u","water_v"],"columnUnits":["UTC","degrees_north","degrees_east","cm s-1","cm s-1"],"rows":[["2026-11-08T08:00:00Z",40,-74,12,7]]}}"#;
        assert_eq!(
            parse_currents(&hf_radar(), body),
            Err(bad("currents in \"cm s-1\", not m s-1"))
        );
        assert!(
            ["m s-1", "m/s", "m s^-1"]
                .iter()
                .all(|u| is_metres_per_second(u))
        );
        assert!(!is_metres_per_second("cm/s") && !is_metres_per_second(""));
    }

    #[test]
    fn erddap_failures_map_to_no_data_retry_and_back_off() {
        let no_match: &[u8] = b"Error {\n    code=404;\n    message=\"Not Found: Your query produced no matching results. (time) Start is greater than the axis maximum.\";\n}\n";
        assert_eq!(
            check_response(&Response::new(404, no_match)),
            Err(ErddapError::NoData)
        );
        assert_eq!(
            check_response(&Response::new(404, b"Not Found")),
            Err(ErddapError::Status { status: 404 })
        );
        let busy = b"There was a (temporary?) problem.  Wait a minute, then try again.";
        let err = check_response(&Response::new(503, busy)).unwrap_err();
        assert_eq!(err, ErddapError::Temporary);
        assert!(err.is_retryable());
        let limited = Response {
            retry_after: Some(120),
            ..Response::new(429, b"Too Many Requests")
        };
        let err = check_response(&limited).unwrap_err();
        assert_eq!(
            err,
            ErddapError::RateLimited {
                retry_after_s: Some(120)
            }
        );
        assert!(err.is_retryable());
        assert_eq!(
            check_response(&Response::new(500, b"")),
            Err(ErddapError::Status { status: 500 })
        );
        assert!(!ErddapError::NoData.is_retryable());
        let page = Response {
            content_type: Some("text/html; charset=UTF-8"),
            ..Response::new(200, b"<html/>")
        };
        assert!(matches!(
            check_response(&page),
            Err(ErddapError::BadTable(_))
        ));
        let json = Response {
            content_type: Some("application/json;charset=UTF-8"),
            ..Response::new(200, b"{}")
        };
        assert_eq!(check_response(&json), Ok(&b"{}"[..]));
    }

    const T0: UnixTime = UnixTime(1_794_000_000);

    fn plus(s: i64) -> UnixTime {
        UnixTime(T0.0 + s)
    }

    #[test]
    fn erddap_pacer_refuses_the_eleventh_request_in_a_minute() {
        let mut pacer = ErddapPacer::new();
        for k in 0..10 {
            assert_eq!(
                pacer.try_send(GridHost::Pfeg, plus(k * 5)),
                Ok(()),
                "request {k}"
            );
        }
        let wait = pacer.try_send(GridHost::Pfeg, plus(50)).unwrap_err();
        assert_eq!(wait.until, plus(60));
        assert_eq!(wait.after_s(plus(50)), 10);
        assert_eq!(pacer.ready_at(GridHost::Pfeg, plus(50)), plus(60));
        // Other nodes are counted apart.
        assert_eq!(pacer.try_send(GridHost::Coastwatch, plus(50)), Ok(()));
        assert_eq!(pacer.ready_at(GridHost::Glerl, plus(50)), plus(50));
        // A minute after the first, one more may go; then the second must age out.
        assert_eq!(pacer.try_send(GridHost::Pfeg, plus(60)), Ok(()));
        assert_eq!(
            pacer.try_send(GridHost::Pfeg, plus(61)).unwrap_err().until,
            plus(65)
        );
    }

    #[test]
    fn erddap_pacer_waits_60_s_after_a_429_doubling_to_15_minutes() {
        let mut pacer = ErddapPacer::new();
        let node = GridHost::Pfeg;
        assert_eq!(pacer.try_send(node, T0), Ok(()));
        pacer.on_response(node, &Response::new(429, b""), plus(1));
        assert_eq!(pacer.try_send(node, plus(60)).unwrap_err().until, plus(61));
        assert_eq!(pacer.try_send(node, plus(61)), Ok(()));

        let mut now = 61;
        for pause in [120, 240, 480, 900, 900] {
            pacer.on_response(node, &Response::new(503, b""), plus(now));
            assert_eq!(pacer.ready_at(node, plus(now)), plus(now + pause));
            now += pause;
            assert_eq!(pacer.try_send(node, plus(now)), Ok(()));
        }
        // An answer resets the doubling.
        pacer.on_response(node, &Response::new(200, b"{}"), plus(now));
        pacer.on_response(node, &Response::new(429, b""), plus(now));
        assert_eq!(pacer.ready_at(node, plus(now)), plus(now + 60));
        // A longer Retry-After is honoured, up to an hour.
        let asked = Response {
            retry_after: Some(600),
            ..Response::new(429, b"")
        };
        pacer.on_response(node, &asked, plus(now));
        assert_eq!(pacer.ready_at(node, plus(now)), plus(now + 600));
        let absurd = Response {
            retry_after: Some(86_400),
            ..Response::new(429, b"")
        };
        pacer.on_response(node, &absurd, plus(now));
        assert_eq!(pacer.ready_at(node, plus(now)), plus(now + 3_600));
        // Other failures do not pause.
        let mut fresh = ErddapPacer::new();
        fresh.on_response(node, &Response::new(500, b""), T0);
        assert_eq!(fresh.ready_at(node, T0), T0);
    }

    #[test]
    fn erddap_pacer_survives_a_clock_that_goes_back() {
        let mut pacer = ErddapPacer::new();
        for k in 0..10 {
            pacer.try_send(GridHost::Glerl, plus(86_400 + k)).unwrap();
        }
        pacer.on_response(GridHost::Glerl, &Response::new(429, b""), plus(86_400));
        // A day earlier: the requests "ahead" are forgotten and the pause lasts an hour at most.
        assert_eq!(pacer.ready_at(GridHost::Glerl, T0), plus(3_600));
    }

    #[test]
    fn erddap_pacer_saves_and_loads_as_json() {
        let mut pacer = ErddapPacer::new();
        pacer.try_send(GridHost::Pfeg, T0).unwrap();
        pacer.on_response(GridHost::Pfeg, &Response::new(429, b""), T0);
        pacer.try_send(GridHost::Glerl, T0).unwrap();
        let json = serde_json::to_string(&pacer).unwrap();
        assert_eq!(
            json,
            r#"{"nodes":{"pfeg":{"sent":[1794000000],"paused_until":1794000060,"last_pause_s":60},"glerl":{"sent":[1794000000]}}}"#
        );
        let back: ErddapPacer = serde_json::from_str(&json).unwrap();
        assert_eq!(back, pacer);
        assert_eq!(
            serde_json::from_str::<ErddapPacer>("{}").unwrap(),
            ErddapPacer::new()
        );
    }
}
