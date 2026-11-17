//! Working offline across the FFI: pack plans and their requests, tide and current tables, and the cached station metadata that lets a fetch compute tides without signal.
//!
//! A pack is planned by the core ([`underbite_core::map::pack`]); the plan stays on the Rust side
//! behind a [`PackPlanHandle`] and is read a page at a time, each item as a [`HostRequest`] the
//! core built with the same builders the map layers use, so Swift never composes a URL. Tables
//! are computed from harmonic constants the app stored and formatted by the view in the
//! settings' units, clock and voice.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use underbite_config::schema::Config;
use underbite_core::astro::tide::TideHarmonics;
use underbite_core::datum::{Height, VerticalDatum};
use underbite_core::map::catalogue::{self, DateMode};
use underbite_core::map::pack::{self, ItemKind, MeasuredSizes, Pack, PackItem, PackPlan};
use underbite_core::map::{Bbox, GridHost};
use underbite_core::model::{CurrentKind, TideSeries};
use underbite_core::offline::tables::{CurrentStation, TideStation, current_table, tide_table};
use underbite_core::provenance::{Provenance, Quality, Relation, SourceId, ValueKind};
use underbite_core::time::{LocalDate, UnixTime, local_date, local_date_bounds, parse_date, zone};
use underbite_core::view::format::{Formatter, Phase};
use underbite_core::view::strings::{Key, SafetyKey, Strings, map_time, source_name};
use underbite_providers::coops::mdapi;
use underbite_providers::erddap::{self, GridTime};
use underbite_providers::ncei_dem;

use crate::fetch::HostRequest;
use crate::map::{MapError, formatter, get, invalid, layer, node_name, tile_request};
use crate::settings::load;

/// The most days a table covers.
pub const MAX_TABLE_DAYS: u32 = 366;

/// The most items one page of a plan holds.
pub const MAX_PAGE_ITEMS: u32 = 2_000;

/// How long the app may reuse a pack download when the server says nothing, seconds.
const PACK_RESPONSE_AGE_S: u64 = 3_600;

/// Seconds from midnight UTC to the time a dated pack grid is asked for: noon, the nearest
/// moment to every daily layer's own time.
const GRID_TIME_OF_DAY_S: i64 = 12 * 3_600;

// ---------------------------------------------------------------------------------------------
// Pack plans
// ---------------------------------------------------------------------------------------------

/// A planned pack, kept on the Rust side. Read it with [`pack_requests`].
#[derive(Debug, uniffi::Object)]
pub struct PackPlanHandle {
    plan: PackPlan,
}

#[uniffi::export]
impl PackPlanHandle {
    /// The number of downloads in the plan.
    pub fn item_count(&self) -> u64 {
        self.plan.len() as u64
    }
}

/// A layer of the pack that is not planned, with why, in words.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct PackSkip {
    /// The layer's id.
    pub layer: String,
    /// The layer's title, or its id when the catalogue has none.
    pub title: String,
    /// Why it is left out: "Not kept offline", "Not drawn at these zoom levels".
    pub reason: String,
}

/// A pack's plan: what it needs, as numbers and as words.
#[derive(Clone, Debug, uniffi::Record)]
pub struct PackPlanInfo {
    /// The plan itself, for [`pack_requests`].
    pub plan: Arc<PackPlanHandle>,
    /// Downloads in all.
    pub item_count: u64,
    /// Raster tiles.
    pub tile_count: u64,
    /// Grid blocks.
    pub grid_count: u64,
    /// Elevation areas.
    pub dem_count: u64,
    /// The estimated download, bytes.
    pub est_bytes: u64,
    /// The estimated time, seconds.
    pub est_seconds: u64,
    /// The estimated download: "48.3 MB".
    pub size_text: String,
    /// The estimated time: "12 min".
    pub time_text: String,
    /// Both in one line: "48.3 MB, about 12 min".
    pub summary_text: String,
    /// Whether the estimate is over the settings' offline limit.
    pub over_budget: bool,
    /// What to tell the user when it is: "About 1.7 GB: more than your 1.5 GB offline limit."
    pub over_budget_text: Option<String>,
    /// Layers left out and why.
    pub skipped: Vec<PackSkip>,
}

/// Plans the pack `pack_json` (a stored [`Pack`]) against the catalogue, replacing each layer's
/// seed size with `measured_sizes` (the store's average bytes per item, by layer id, `depth-dem`
/// for elevation areas) when the app has one, and reports whether it is over the settings'
/// `offline_max_mb`. A pack that cannot be planned, or needs more than the item cap, is an
/// error in words.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
#[allow(clippy::implicit_hasher)] // UniFFI's map is the standard one
pub fn plan_pack(
    pack_json: String,
    settings_json: String,
    measured_sizes: HashMap<String, u64>,
    now: i64,
) -> Result<PackPlanInfo, MapError> {
    let (config, _) = load(&settings_json)?;
    let strings = Strings::new(config.presentation.voice);
    let pack: Pack = serde_json::from_str(&pack_json)
        .map_err(|e| invalid(format!("the pack could not be read: {e}")))?;
    let measured: MeasuredSizes = measured_sizes.into_iter().collect();
    let plan = pack::plan(&pack, catalogue::find, &measured, UnixTime(now))
        .map_err(|e| invalid(strings.pack_error(&e)))?;
    let size_text = strings.pack_size(plan.est_bytes);
    let time_text = strings.pack_duration(plan.est_seconds);
    let limit_mb = config.map.offline_max_mb;
    let over_budget = plan.exceeds_mb(limit_mb);
    let over_budget_text = over_budget.then(|| {
        strings.fmt(
            Key::PackOverBudget,
            &[
                ("size", &size_text),
                ("limit", &strings.pack_size(u64::from(limit_mb) * 1_000_000)),
            ],
        )
    });
    let skipped = plan
        .skipped
        .iter()
        .map(|s| PackSkip {
            layer: s.layer.as_str().to_owned(),
            title: strings
                .layer_title(s.layer.as_str())
                .unwrap_or(s.layer.as_str())
                .to_owned(),
            reason: strings.pack_skip(s.reason).to_owned(),
        })
        .collect();
    Ok(PackPlanInfo {
        item_count: plan.len() as u64,
        tile_count: plan.tile_count as u64,
        grid_count: plan.grid_count as u64,
        dem_count: plan.dem_count as u64,
        est_bytes: plan.est_bytes,
        est_seconds: plan.est_seconds,
        summary_text: strings.fmt(
            Key::PackSummary,
            &[("size", &size_text), ("time", &time_text)],
        ),
        size_text,
        time_text,
        over_budget,
        over_budget_text,
        skipped,
        plan: Arc::new(PackPlanHandle { plan }),
    })
}

/// What a pack download is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum PackItemKind {
    /// A raster tile.
    Tile,
    /// A grid block (ERDDAP, GLERL).
    Grid,
    /// An elevation area.
    Dem,
}

/// One download of a pack.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct PackRequest {
    /// A stable, unique name for the item, safe as a file name: store its response under it.
    pub key: String,
    /// What it is.
    pub kind: PackItemKind,
    /// The layer's id (`depth-dem` for elevation areas).
    pub layer: String,
    /// The host the request goes to, for pacing: a grid node (`pfeg`, `coastwatch`, `glerl`)
    /// or the server's name.
    pub host: String,
    /// The request, built by the core.
    pub request: HostRequest,
}

fn date_text(d: LocalDate) -> String {
    format!("{:04}-{:02}-{:02}", d.year, d.month, d.day)
}

/// The UTC time a dated grid block is asked for.
fn grid_time(date: Option<LocalDate>) -> Result<GridTime, MapError> {
    let Some(date) = date else {
        return Ok(GridTime::Last);
    };
    let utc = zone("UTC").map_err(|e| invalid(e.to_string()))?;
    let (start, _) = local_date_bounds(date, &utc).map_err(|e| invalid(e.to_string()))?;
    Ok(GridTime::At(UnixTime(start.0 + GRID_TIME_OF_DAY_S)))
}

/// The request of one planned item, built with the builders the map layers use.
fn request_of(item: &PackItem) -> Result<PackRequest, MapError> {
    let layer_id = item.layer.as_str();
    let (kind, host, request) = match &item.kind {
        ItemKind::Tile { tile, date } => {
            let fetch = tile_request(
                layer_id.to_owned(),
                date.map(date_text),
                tile.z,
                tile.x,
                tile.y,
            )?
            .ok_or_else(|| invalid(format!("`{layer_id}` has no tile {}", item.key)))?;
            (PackItemKind::Tile, item.host.clone(), fetch.request)
        }
        ItemKind::Grid { block, date } => {
            let l = layer(layer_id)?;
            let grid = l
                .grid()
                .ok_or_else(|| invalid(format!("`{layer_id}` is not a gridded layer")))?;
            let source = grid.primary.source();
            let built = erddap::block_request(&source, grid_time(*date)?, *block)
                .map_err(|e| invalid(e.to_string()))?;
            (
                PackItemKind::Grid,
                node_name(built.node),
                get(built.url, PACK_RESPONSE_AGE_S),
            )
        }
        ItemKind::Dem { area, cell_deg } => {
            let built = ncei_dem::area_request(
                Bbox {
                    west: area.west,
                    south: area.south,
                    east: area.east,
                    north: area.north,
                },
                *cell_deg,
            )
            .map_err(|e| invalid(e.to_string()))?;
            (
                PackItemKind::Dem,
                node_name(GridHost::NceiDem),
                get(built.url, PACK_RESPONSE_AGE_S),
            )
        }
    };
    Ok(PackRequest {
        key: item.key.clone(),
        kind,
        layer: layer_id.to_owned(),
        host,
        request,
    })
}

/// The requests of up to `count` items of `plan` from item `from` (at most
/// [`MAX_PAGE_ITEMS`] a page); past the end the page is empty. The plan's items are in a fixed
/// order, so paging through it returns each once.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn pack_requests(
    plan: Arc<PackPlanHandle>,
    from: u64,
    count: u32,
) -> Result<Vec<PackRequest>, MapError> {
    let from = usize::try_from(from).unwrap_or(usize::MAX);
    let count = usize::try_from(count.min(MAX_PAGE_ITEMS)).unwrap_or(0);
    plan.plan
        .items(from, count)
        .iter()
        .map(request_of)
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Tables
// ---------------------------------------------------------------------------------------------

/// One line of a tide or current table.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct TableRow {
    /// The event's time, Unix seconds.
    pub time: i64,
    /// The day in the spot's zone: "Nov 8".
    pub date_text: String,
    /// The time of day in the user's clock: "7:03 AM".
    pub time_text: String,
    /// "High", "Low", "Max flood", "Max ebb", "Slack".
    pub kind_text: String,
    /// The height ("4.2 ft MLLW") or the speed ("1.4 kn"); none for a slack.
    pub value_text: Option<String>,
}

/// A tide or current table for a stretch of days, worded by the view.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct TableView {
    /// The events in time order.
    pub rows: Vec<TableRow>,
    /// What the heights are above: "above MLLW"; none for currents.
    pub datum_text: Option<String>,
    /// For currents, where the flood and the ebb set: "Flood toward NE 45°".
    pub directions: Vec<String>,
    /// How the table was made: "Computed on device".
    pub note: String,
    /// The source's credit: "NOAA CO-OPS harmonics".
    pub credit: String,
    /// A safety notice the table is shown with: "Not for navigation." for currents.
    pub notice: Option<String>,
}

/// The window of `days` local days of `time_zone` from `from_date`, as UTC seconds: the start
/// of the first day to the last second of the last.
fn window(from_date: &str, days: u32, time_zone: &str) -> Result<(UnixTime, UnixTime), MapError> {
    if days == 0 || days > MAX_TABLE_DAYS {
        return Err(invalid(format!("days must be 1 to {MAX_TABLE_DAYS}")));
    }
    let first = parse_date(from_date.trim()).map_err(|e| invalid(e.to_string()))?;
    let tz = zone(time_zone)
        .or_else(|_| zone("UTC"))
        .map_err(|e| invalid(e.to_string()))?;
    let (start, _) = local_date_bounds(first, &tz).map_err(|e| invalid(e.to_string()))?;
    // Noon of the last day is a day-length away from any daylight-saving change.
    let noon = UnixTime(start.0 + i64::from(days - 1) * 86_400 + 12 * 3_600);
    let last = local_date(noon, &tz).map_err(|e| invalid(e.to_string()))?;
    let (_, end) = local_date_bounds(last, &tz).map_err(|e| invalid(e.to_string()))?;
    Ok((start, UnixTime(end.0 - 1)))
}

/// The provenance of a table computed on the device at `now`.
fn base(now: UnixTime) -> Provenance {
    Provenance {
        source: SourceId::COOPS_HARMONICS,
        station: None,
        distance_m: None,
        relation: Relation::Nearby,
        observed_at: now,
        kind: ValueKind::Computed,
        quality: Quality::Approved,
    }
}

/// The step of the curve a table computes and the view does not show.
const CURVE_STEP: Duration = Duration::from_hours(1);

/// Whatever went wrong with the constants or the window, in words; the cause is not shown.
fn no_table(strings: Strings, _cause: &impl std::fmt::Debug) -> MapError {
    invalid(strings.get(Key::TableNoConstants))
}

/// The tide table of a station for `days` local days from `from_date` (`YYYY-MM-DD`) in
/// `time_zone`, computed on the device from the station's stored CO-OPS constants:
/// `harcon_json` and `datums_json` are the responses of `harcon.json?units=metric` and
/// `datums.json?units=metric` (a subordinate station's reference station's, with
/// `offsets_json` the subordinate's `tidepredoffsets.json`). Heights are above MLLW, as CO-OPS
/// publishes them; a subordinate station has highs and lows only.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
#[allow(clippy::too_many_arguments)] // the stored responses and the window
pub fn tide_table_view(
    station_id: String,
    harcon_json: String,
    datums_json: String,
    offsets_json: Option<String>,
    settings_json: String,
    time_zone: String,
    from_date: String,
    days: u32,
) -> Result<TableView, MapError> {
    let (config, _) = load(&settings_json)?;
    let strings = Strings::new(config.presentation.voice);
    let offsets = match offsets_json {
        Some(body) => Some(
            mdapi::parse_tide_offsets(body.as_bytes())
                .map_err(|e| no_table(strings, &e))?
                .ok_or_else(|| no_table(strings, &"a reference station has no offsets"))?,
        ),
        None => None,
    };
    let reference_id = offsets
        .as_ref()
        .map_or(station_id.as_str(), |o| o.reference_id.as_str());
    let datums = mdapi::parse_datums(reference_id, datums_json.as_bytes())
        .map_err(|e| no_table(strings, &e))?;
    let harmonics: TideHarmonics = mdapi::parse_tide_harmonics(
        reference_id,
        harcon_json.as_bytes(),
        &datums,
        VerticalDatum::Mllw,
    )
    .map_err(|e| no_table(strings, &e))?;
    let (from, to) = window(&from_date, days, &time_zone)?;
    let source = match &offsets {
        Some(offsets) => TideStation::Subordinate {
            reference: &harmonics,
            offsets,
        },
        None => TideStation::Harmonic(&harmonics),
    };
    let series =
        tide_table(source, base(from), from, to, CURVE_STEP).map_err(|e| no_table(strings, &e))?;
    Ok(tide_view(&config, &time_zone, &series))
}

fn tide_view(config: &Config, time_zone: &str, t: &TideSeries) -> TableView {
    let f = formatter(config, time_zone);
    let mut datum_text = None;
    let rows = t
        .events
        .iter()
        .map(|e| {
            let height = f.height(e.height_m.map(|metres| Height {
                datum: t.datum,
                metres,
            }));
            if datum_text.is_none() {
                let unit = &height.unit;
                let name = unit.split_once(' ').map_or("", |(_, d)| d);
                datum_text = Some(f.strings.fmt(Key::AboveDatum, &[("datum", name)]));
            }
            TableRow {
                time: e.time.0,
                date_text: f.date_of(e.time),
                time_text: f.clock(e.time),
                kind_text: f.strings.tide_kind(e.kind).to_owned(),
                value_text: Some(height.full()),
            }
        })
        .collect();
    TableView {
        rows,
        datum_text,
        directions: Vec::new(),
        note: f.strings.get(Key::ComputedOnDevice).to_owned(),
        credit: source_name(&SourceId::COOPS_HARMONICS),
        notice: None,
    }
}

/// The current table of a station and bin for `days` local days from `from_date` in
/// `time_zone`, computed on the device from stored CO-OPS constants: `harcon_json` is the
/// response of the station's `harcon.json` (a subordinate station's reference bin's, with
/// `offsets_json` the subordinate's `currentpredictionoffsets.json`). Speeds are along the
/// major axis in the settings' unit. Not for navigation.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
#[allow(clippy::too_many_arguments)] // the stored responses and the window
pub fn current_table_view(
    station_id: String,
    bin: Option<u16>,
    harcon_json: String,
    offsets_json: Option<String>,
    settings_json: String,
    time_zone: String,
    from_date: String,
    days: u32,
) -> Result<TableView, MapError> {
    let (config, _) = load(&settings_json)?;
    let strings = Strings::new(config.presentation.voice);
    let offsets = match offsets_json {
        Some(body) => Some(
            mdapi::parse_current_offsets(bin, body.as_bytes())
                .map_err(|e| no_table(strings, &e))?,
        ),
        None => None,
    };
    let harmonics = match &offsets {
        Some(o) => mdapi::parse_current_harmonics(
            &o.reference_id,
            Some(o.reference_bin),
            harcon_json.as_bytes(),
        ),
        None => mdapi::parse_current_harmonics(&station_id, bin, harcon_json.as_bytes()),
    }
    .map_err(|e| no_table(strings, &e))?;
    let (from, to) = window(&from_date, days, &time_zone)?;
    let source = match &offsets {
        Some(offsets) => CurrentStation::Subordinate {
            reference: &harmonics,
            offsets,
        },
        None => CurrentStation::Harmonic(&harmonics),
    };
    let series = current_table(source, base(from), from, to, CURVE_STEP)
        .map_err(|e| no_table(strings, &e))?;
    let f = formatter(&config, &time_zone);
    let rows = series
        .events
        .iter()
        .map(|e| {
            let (kind_text, value_text) = match e.kind {
                CurrentKind::MaxFlood => (
                    f.strings.get(Key::MaxFlood).to_owned(),
                    Some(f.current_speed(Some(e.speed_ms.abs())).full()),
                ),
                CurrentKind::MaxEbb => (
                    f.strings.get(Key::MaxEbb).to_owned(),
                    Some(f.current_speed(Some(e.speed_ms.abs())).full()),
                ),
                CurrentKind::Slack => (f.phase(Phase::Slack).to_owned(), None),
            };
            TableRow {
                time: e.time.0,
                date_text: f.date_of(e.time),
                time_text: f.clock(e.time),
                kind_text,
                value_text,
            }
        })
        .collect();
    let toward = |key: Key, degrees: Option<f32>| {
        degrees.map(|d| {
            f.strings.fmt(
                Key::TowardLine,
                &[
                    ("line", f.strings.get(key)),
                    ("direction", &f.direction(Some(d))),
                ],
            )
        })
    };
    let directions = [
        toward(Key::Flood, series.flood_dir_deg),
        toward(Key::Ebb, series.ebb_dir_deg),
    ]
    .into_iter()
    .flatten()
    .collect();
    Ok(TableView {
        rows,
        datum_text: None,
        directions,
        note: f.strings.get(Key::ComputedOnDevice).to_owned(),
        credit: source_name(&SourceId::COOPS_HARMONICS),
        notice: Some(f.strings.safety(SafetyKey::NotForNavigation).to_owned()),
    })
}

/// The text of a time the offline note quotes: "Nov 8, 4:00 AM" in the spot's zone.
pub(crate) fn note_time(f: &Formatter, t: UnixTime) -> String {
    map_time(f, t, DateMode::Hourly)
}
