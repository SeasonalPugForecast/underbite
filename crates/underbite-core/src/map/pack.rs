//! Offline packs: a box, a zoom range, layers, dates and spots, planned as tile, grid-block and
//! elevation-area downloads with a size and time estimate before anything is fetched.
//!
//! A [`Pack`] is stored by the app and is not part of the configuration. [`plan`] turns it into a
//! [`PackPlan`]: every item to download, in a deterministic order, with the counts, the byte
//! estimate and the time estimate. The plan is read a page at a time ([`PackPlan::items`]) so a
//! large one never crosses the FFI as a single list.
//!
//! # Hard item cap
//!
//! A plan holds at most [`MAX_ITEMS`] (50,000) items. Tile counts grow fourfold per zoom level, so
//! the cap is checked while the plan is built (the covering function is asked for no more than
//! the room left), never after: a pack that is too large is refused with
//! [`PlanError::TooManyItems`] without building it. At the 10 to 40 KB a tile costs, 50,000 items
//! are 0.5 to 2 GB, around the default `offline_max_mb`; [`PackPlan::exceeds_mb`] reports a plan
//! that is over the user's own limit.
//!
//! # What a pack holds
//!
//! - Tiled layers (the NOAA chart, GEBCO, GIBS imagery) as tiles, clamped per layer to the zooms
//!   it serves: no tile is planned past a layer's deepest zoom. Planning is in zoom levels, never
//!   in the NOAA cache's own levels.
//! - Gridded layers (ERDDAP, GLERL) as the cache blocks [`blocks_for_view`] chooses, one set per
//!   distinct stride the zoom range needs, at the times the map asks for them: the newest as
//!   `(last)`, older days at the times of the layer's time axis ([`TimeAxes`]). A dataset's time
//!   of day varies, so a day alone names no time; without its axis a layer holds its newest time
//!   only. The primary dataset is planned; a fallback dataset is not.
//! - [`DEM_LAYER`] as elevation areas of the NCEI DEM, not tiles.
//! - Not packed: base maps Apple draws, station and spot markers, state feature layers, ENC
//!   points, radar frames (minutes old) and layers with no source. They are listed in
//!   [`PackPlan::skipped`] with the reason.
//!
//! The box is clamped to Web Mercator's latitude limit for tiles and to the poles for grids; a
//! box across the antimeridian (west east of east) is split into two.
//!
//! # Keys
//!
//! A grid block and an elevation area are keyed as the map's grid store keys the same
//! download, so a pack's response is what the map reads:
//! `{layer}/{dataset}/{time or last}/{south}/{west}/{stride}` for a block (Unix seconds) and
//! `dem/area/{west}/{south}/{east}/{north}/{cell}` for an area (millionths of a degree). A tile
//! is `{layer}/{date or latest}/{z}/{x}/{y}`.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::map::catalogue::{Availability, DateMode, Layer, LayerKind, TileSource};
use crate::map::grid::{Block, block_side_cells, block_side_deg, blocks_for_view, stride_for_zoom};
use crate::map::tiles::{
    MAX_LATITUDE, MAX_ZOOM, NCDS_ZOOMS, gibs_default_date, previous_day, tiles_covering,
};
use crate::map::{Bbox, GridHost, GridSource, LayerId, TileKey};
use crate::time::{LocalDate, UnixTime, UtcOffset};

/// The stored format version of a [`Pack`].
pub const PACK_VERSION: u32 = 1;

/// The most items a plan may hold; a pack that needs more is refused.
pub const MAX_ITEMS: usize = 50_000;

/// The most days a `LastDays` or `Days` policy may name.
pub const MAX_DAYS: usize = 31;

/// The id the elevation areas are filed under in a pack's `layers`. It is not a catalogue layer:
/// the DEM feeds depth readouts and contours.
pub const DEM_LAYER: &str = "depth-dem";

/// A spot's id, as the model names it.
pub type SpotId = String;

/// The measured average size in bytes of one item of a layer, by layer id (the store's averages);
/// [`DEM_LAYER`] keys the elevation areas.
pub type MeasuredSizes = BTreeMap<String, u64>;

/// The times a gridded layer's primary dataset holds, by layer id, as the map's time axis lists
/// them (any order).
pub type TimeAxes = BTreeMap<String, Vec<UnixTime>>;

/// The latest axis time taken, 9999-12-31T23:59:59Z; later or negative ones are ignored.
const MAX_AXIS_TIME: i64 = 253_402_300_799;

/// Seed size of a GIBS true-colour JPEG tile.
pub const SEED_GIBS_JPEG: u64 = 12_700;
/// Seed size of a GIBS SST PNG tile.
pub const SEED_GIBS_SST: u64 = 5_200;
/// Seed size of a GIBS chlorophyll PNG tile.
pub const SEED_GIBS_CHLOROPHYLL: u64 = 13_700;
/// Seed size of a NOAA chart WMTS tile (9.5 to 12.4 KB at zooms 14 to 16).
pub const SEED_NCDS_WMTS: u64 = 11_000;
/// Seed size of a NOAA chart WMS tile of display groups 2 and 3.
pub const SEED_NCDS_WMS: u64 = 39_000;
/// Seed size of a GEBCO tile.
pub const SEED_GEBCO: u64 = 22_600;
/// Seed size of a 128 px float32 elevation area.
pub const SEED_DEM_AREA: u64 = 67_000;
/// Seed size of one grid cell in an ERDDAP JSON answer.
pub const SEED_GRID_CELL: u64 = 60;
/// Extra bytes a second variable adds to a grid cell's row.
pub const SEED_GRID_EXTRA_VARIABLE: u64 = 15;

/// Requests an ERDDAP node accepts per minute (the provider's pacer allows the same).
pub const ERDDAP_REQUESTS_PER_MINUTE: u64 = 10;
/// Requests in flight at once to any other host.
pub const OTHER_CONCURRENCY: u64 = 4;
/// Seconds one request to a non-ERDDAP host takes, on average, in tenths.
pub const OTHER_DECISECONDS_PER_REQUEST: u64 = 5;

/// The cells along one side of an elevation area.
pub const DEM_AREA_PX: u32 = 128;
/// The finest elevation cell planned, in degrees (the DEM's 1/3 arc-second).
pub const DEM_MIN_CELL_DEG: f64 = 1.0 / 3.0 / 3600.0;

const DEM_HOST: &str = "gis.ngdc.noaa.gov";

/// Which days a pack holds for dated layers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DatePolicy {
    /// The newest data only.
    Latest,
    /// The last `n` UTC days, newest first (1 to [`MAX_DAYS`]).
    LastDays(u32),
    /// These days.
    Days(Vec<LocalDate>),
}

/// An offline pack as the app stores it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Pack {
    /// The stored format version, [`PACK_VERSION`].
    #[serde(default = "pack_version")]
    pub version: u32,
    pub id: String,
    pub name: String,
    pub bbox: Bbox,
    pub min_zoom: u8,
    pub max_zoom: u8,
    pub layers: Vec<LayerId>,
    pub dates: DatePolicy,
    pub spots: Vec<SpotId>,
    pub created_at: UnixTime,
    /// When the pack's data was last refreshed; none until the first download completes.
    #[serde(default)]
    pub refreshed_at: Option<UnixTime>,
}

fn pack_version() -> u32 {
    PACK_VERSION
}

/// How long a pack of the newest data only ([`DatePolicy::Latest`]) stays current after a
/// refresh, seconds: a day, as most of its layers have a new day by then.
pub const LATEST_FRESH_S: i64 = 86_400;

/// How long any pack stays current after a refresh, seconds: a week.
pub const FRESH_S: i64 = 7 * 86_400;

/// The deepest zoom a new pack may ask for: the deepest any tiled layer of the catalogue
/// serves (the NOAA chart's).
pub const PACK_MAX_ZOOM: u8 = *NCDS_ZOOMS.end();

/// The longest a pack's id may be, in bytes.
pub const MAX_ID_LEN: usize = 64;

/// The number of days the `LastDays` choice starts at.
pub const DEFAULT_LAST_DAYS: u32 = 3;

/// Why a pack needs a refresh.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stale {
    /// A pack of the newest data only, refreshed more than [`LATEST_FRESH_S`] ago.
    NewestOverADay,
    /// Any pack refreshed more than [`FRESH_S`] ago.
    OverAWeek,
}

impl Pack {
    /// Whether the pack needs a refresh at `now`, and why. A pack never downloaded is not
    /// stale (it has nothing to be out of date), and neither is one refreshed after `now`.
    pub fn staleness(&self, now: UnixTime) -> Option<Stale> {
        let age = now.0.saturating_sub(self.refreshed_at?.0);
        if age > FRESH_S {
            Some(Stale::OverAWeek)
        } else if self.dates == DatePolicy::Latest && age > LATEST_FRESH_S {
            Some(Stale::NewestOverADay)
        } else {
            None
        }
    }

    /// Whether the pack needs a refresh at `now` (see [`Pack::staleness`]).
    pub fn is_stale(&self, now: UnixTime) -> bool {
        self.staleness(now).is_some()
    }

    /// Records a completed download at `now`.
    pub fn mark_refreshed(&mut self, now: UnixTime) {
        self.refreshed_at = Some(now);
    }
}

/// How close a pack's deepest zoom comes, as the choices the app offers. Each reaches the
/// deepest zoom its name says and starts four levels above it, so the area can still be seen
/// zoomed out without signal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZoomReach {
    /// Zooms 6 to 10 (about 120 m a pixel at 40° N): a run offshore, where the temperature
    /// and colour breaks and the canyons are kilometres wide.
    Offshore,
    /// Zooms 8 to 12 (about 30 m a pixel): a bay or a stretch of coast, its reefs and ledges.
    Coast,
    /// Zooms 10 to 14 (about 7 m a pixel): a harbour, estuary or river reach, its channel
    /// edges, docks and bridges.
    Harbour,
    /// Zooms 12 to 16 (about 2 m a pixel): piers, jetties and single rocks.
    Close,
}

impl ZoomReach {
    /// Every choice, from the widest to the closest.
    pub const ALL: [Self; 4] = [Self::Offshore, Self::Coast, Self::Harbour, Self::Close];

    /// The choice the app starts at.
    pub const DEFAULT: Self = Self::Harbour;

    /// The shallowest and the deepest zoom.
    pub const fn zooms(self) -> (u8, u8) {
        match self {
            Self::Offshore => (6, 10),
            Self::Coast => (8, 12),
            Self::Harbour => (10, 14),
            Self::Close => (12, 16),
        }
    }
}

/// What a new pack is made of. [`PackDraft::create`] checks it and turns it into a [`Pack`].
#[derive(Clone, Debug, PartialEq)]
pub struct PackDraft {
    /// The host's id for the pack (a UUID in the app): 1 to [`MAX_ID_LEN`] ASCII letters,
    /// digits, `-` or `_`, so it is safe as a file name.
    pub id: String,
    pub name: String,
    /// `west` east of `east` crosses the antimeridian.
    pub bbox: Bbox,
    pub min_zoom: u8,
    pub max_zoom: u8,
    pub layers: Vec<LayerId>,
    pub dates: DatePolicy,
    pub spots: Vec<SpotId>,
}

impl PackDraft {
    /// The pack, created at `now` and not yet downloaded. The name is trimmed; layers, spots
    /// and chosen days lose their repeats (days are sorted).
    ///
    /// # Errors
    /// [`PlanError::InvalidId`] or [`PlanError::InvalidName`] for an id or a name that cannot
    /// be used; [`PlanError::InvalidBox`] for a box with an edge that is not finite or off the
    /// world, `south` not south of `north`, or no width; [`PlanError::InvalidZooms`] for zooms
    /// that are reversed or deeper than [`PACK_MAX_ZOOM`]; [`PlanError::InvalidDates`] as
    /// planning would refuse the dates.
    pub fn create(self, now: UnixTime) -> Result<Pack, PlanError> {
        let id_ok = (1..=MAX_ID_LEN).contains(&self.id.len())
            && self
                .id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_');
        if !id_ok {
            return Err(PlanError::InvalidId);
        }
        let name = self.name.trim();
        if name.is_empty() {
            return Err(PlanError::InvalidName);
        }
        let b = self.bbox;
        let lat = |v: f64| (-90.0..=90.0).contains(&v);
        let lon = |v: f64| (-180.0..=180.0).contains(&v);
        if !(lat(b.south) && lat(b.north) && lon(b.west) && lon(b.east))
            || b.south >= b.north
            || b.west.total_cmp(&b.east).is_eq()
        {
            return Err(PlanError::InvalidBox);
        }
        if self.min_zoom > self.max_zoom || self.max_zoom > PACK_MAX_ZOOM {
            return Err(PlanError::InvalidZooms);
        }
        pack_days(&self.dates, now)?;
        let dates = match self.dates {
            DatePolicy::Days(days) => DatePolicy::Days(
                days.into_iter()
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect(),
            ),
            other => other,
        };
        Ok(Pack {
            version: PACK_VERSION,
            id: self.id,
            name: name.to_owned(),
            bbox: b,
            min_zoom: self.min_zoom,
            max_zoom: self.max_zoom,
            layers: unique(self.layers),
            dates,
            spots: unique(self.spots),
            created_at: now,
            refreshed_at: None,
        })
    }
}

/// The items in their first order, without repeats.
fn unique<T: Ord + Clone>(items: Vec<T>) -> Vec<T> {
    let mut seen = BTreeSet::new();
    items
        .into_iter()
        .filter(|i| seen.insert(i.clone()))
        .collect()
}

/// What an item is and what it addresses.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemKind {
    /// A raster tile; `date` is the UTC day for dated layers.
    Tile {
        tile: TileKey,
        date: Option<LocalDate>,
    },
    /// A grid cache block of the layer's primary dataset at one of its times; none asks for
    /// the newest (`(last)`), as the map does.
    Grid {
        block: Block,
        time: Option<UnixTime>,
    },
    /// An elevation area at a cell size in degrees.
    Dem { area: Bbox, cell_deg: f64 },
}

/// One download.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PackItem {
    /// A stable, unique name for the item, safe as a file name.
    pub key: String,
    pub layer: LayerId,
    pub kind: ItemKind,
    /// The host name the request goes to.
    pub host: String,
    pub est_bytes: u64,
}

/// Why a layer of the pack is not planned.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    /// The catalogue does not know the id.
    Unknown,
    /// The layer is listed but cannot be turned on.
    Unavailable,
    /// The layer is not stored in packs (a base Apple draws, markers, feature layers, radar).
    NotPackable,
    /// The pack's zooms lie outside the layer's.
    ZoomOutOfRange,
}

/// A layer left out of a plan.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkippedLayer {
    pub layer: LayerId,
    pub reason: SkipReason,
}

/// A pack that cannot be planned or created.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum PlanError {
    /// The box has a non-finite edge or `south` is north of `north`.
    #[error("the pack's box is not valid")]
    InvalidBox,
    /// The zooms are reversed or past the deepest zoom.
    #[error("the pack's zoom range is not valid")]
    InvalidZooms,
    /// No days, too many days or a `LastDays` of 0.
    #[error("the pack's dates are not valid")]
    InvalidDates,
    /// The plan would hold more than the cap.
    #[error("the pack needs more than {cap} downloads")]
    TooManyItems { cap: usize },
    /// A new pack's id is empty, too long or not safe as a file name.
    #[error("the pack's id is not valid")]
    InvalidId,
    /// A new pack's name is blank.
    #[error("the pack has no name")]
    InvalidName,
}

/// A planned pack.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PackPlan {
    items: Vec<PackItem>,
    pub tile_count: usize,
    pub grid_count: usize,
    pub dem_count: usize,
    /// The estimated download in bytes.
    pub est_bytes: u64,
    /// The estimated time in seconds, hosts counted one after the other.
    pub est_seconds: u64,
    /// Layers of the pack that were left out, and why.
    pub skipped: Vec<SkippedLayer>,
}

impl PackPlan {
    /// The number of items.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether there is nothing to download.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Up to `count` items from index `from`; past the end the page is empty.
    pub fn items(&self, from: usize, count: usize) -> &[PackItem] {
        let start = from.min(self.items.len());
        let end = start.saturating_add(count).min(self.items.len());
        &self.items[start..end]
    }

    /// Whether the estimate is over a limit in megabytes (`offline_max_mb`; 1 MB is 1,000,000
    /// bytes).
    pub fn exceeds_mb(&self, max_mb: u32) -> bool {
        self.est_bytes > u64::from(max_mb).saturating_mul(1_000_000)
    }
}

/// Plans `pack` against a catalogue lookup (usually [`find`](crate::map::catalogue::find)).
/// `measured` replaces the seed size of a layer's items with the store's average; `axes` are
/// the gridded layers' time axes, from which older days are planned.
///
/// # Errors
/// [`PlanError`] for a pack that is invalid or over the item cap.
pub fn plan(
    pack: &Pack,
    catalogue: impl Fn(&str) -> Option<&'static Layer>,
    measured: &MeasuredSizes,
    axes: &TimeAxes,
    now: UnixTime,
) -> Result<PackPlan, PlanError> {
    plan_with_cap(pack, catalogue, measured, axes, now, MAX_ITEMS)
}

struct Builder<'a> {
    cap: usize,
    measured: &'a MeasuredSizes,
    axes: &'a TimeAxes,
    items: Vec<PackItem>,
    tile_count: usize,
    grid_count: usize,
    dem_count: usize,
}

impl Builder<'_> {
    fn room(&self) -> usize {
        self.cap.saturating_sub(self.items.len())
    }

    fn size(&self, layer: &str, seed: u64) -> u64 {
        self.measured.get(layer).copied().unwrap_or(seed)
    }
}

enum Stop {
    Skip(SkipReason),
    Cap,
}

fn plan_with_cap(
    pack: &Pack,
    catalogue: impl Fn(&str) -> Option<&'static Layer>,
    measured: &MeasuredSizes,
    axes: &TimeAxes,
    now: UnixTime,
    cap: usize,
) -> Result<PackPlan, PlanError> {
    let boxes = split_box(pack.bbox)?;
    if pack.min_zoom > pack.max_zoom || pack.max_zoom > MAX_ZOOM {
        return Err(PlanError::InvalidZooms);
    }
    let days = pack_days(&pack.dates, now)?;
    let mut b = Builder {
        cap,
        measured,
        axes,
        items: Vec::new(),
        tile_count: 0,
        grid_count: 0,
        dem_count: 0,
    };
    let mut skipped = Vec::new();
    let mut seen = BTreeSet::new();
    for id in &pack.layers {
        if !seen.insert(id.clone()) {
            continue;
        }
        if id.as_str() == DEM_LAYER {
            plan_dem(&mut b, pack, &boxes)?;
            continue;
        }
        let Some(layer) = catalogue(id.as_str()) else {
            skipped.push(SkippedLayer {
                layer: id.clone(),
                reason: SkipReason::Unknown,
            });
            continue;
        };
        match plan_layer(&mut b, pack, layer, &boxes, &days) {
            Ok(()) => {}
            Err(Stop::Skip(reason)) => skipped.push(SkippedLayer {
                layer: id.clone(),
                reason,
            }),
            Err(Stop::Cap) => return Err(PlanError::TooManyItems { cap }),
        }
    }
    let est_bytes = b
        .items
        .iter()
        .fold(0_u64, |t, i| t.saturating_add(i.est_bytes));
    let est_seconds = estimate_seconds(&b.items);
    Ok(PackPlan {
        tile_count: b.tile_count,
        grid_count: b.grid_count,
        dem_count: b.dem_count,
        items: b.items,
        est_bytes,
        est_seconds,
        skipped,
    })
}

/// The days a pack asks for.
struct PackDays<'a> {
    /// For dated tiles other than GIBS: UTC days, or none for the newest.
    utc: Vec<Option<LocalDate>>,
    /// For GIBS tiles, where the newest day is the default (yesterday, UTC).
    gibs: Vec<LocalDate>,
    /// For gridded layers, which take their times from the layer's axis.
    policy: &'a DatePolicy,
}

/// The times a dated gridded layer is planned at: the newest as `(last)` when the policy holds
/// its day, then the older times of `axis` on the policy's days, newest first. `LastDays` counts
/// back from the newest time's UTC day, so a layer's latency never asks for a day it does not
/// have yet. Without an axis only the newest time can be named.
fn grid_times(policy: &DatePolicy, axis: Option<&Vec<UnixTime>>) -> Vec<Option<UnixTime>> {
    let mut times: Vec<UnixTime> = axis
        .into_iter()
        .flatten()
        .copied()
        .filter(|t| (0..=MAX_AXIS_TIME).contains(&t.0))
        .collect();
    times.sort_unstable();
    times.dedup();
    let Some(&newest) = times.last() else {
        return vec![None];
    };
    let day = |t: UnixTime| t.local_date(UtcOffset(0));
    let keep: Box<dyn Fn(LocalDate) -> bool + '_> = match policy {
        DatePolicy::Latest => return vec![None],
        DatePolicy::LastDays(n) => {
            let mut first = day(newest);
            for _ in 1..*n {
                first = previous_day(first);
            }
            Box::new(move |d| d >= first)
        }
        DatePolicy::Days(days) => Box::new(|d| days.contains(&d)),
    };
    let mut out = Vec::new();
    if keep(day(newest)) {
        out.push(None);
    }
    out.extend(
        times
            .iter()
            .rev()
            .skip(1)
            .filter(|t| keep(day(**t)))
            .map(|t| Some(*t)),
    );
    out
}

fn pack_days(policy: &DatePolicy, now: UnixTime) -> Result<PackDays<'_>, PlanError> {
    match policy {
        DatePolicy::Latest => Ok(PackDays {
            utc: vec![None],
            gibs: vec![gibs_default_date(now)],
            policy,
        }),
        DatePolicy::LastDays(n) => {
            let n = usize::try_from(*n).unwrap_or(usize::MAX);
            if n == 0 || n > MAX_DAYS {
                return Err(PlanError::InvalidDates);
            }
            let back = |first: LocalDate| {
                let mut day = first;
                let mut out = Vec::with_capacity(n);
                for _ in 0..n {
                    out.push(day);
                    day = previous_day(day);
                }
                out
            };
            Ok(PackDays {
                utc: back(now.local_date(UtcOffset(0)))
                    .into_iter()
                    .map(Some)
                    .collect(),
                gibs: back(gibs_default_date(now)),
                policy,
            })
        }
        DatePolicy::Days(days) => {
            let set: BTreeSet<LocalDate> = days.iter().copied().collect();
            if set.is_empty() || set.len() > MAX_DAYS {
                return Err(PlanError::InvalidDates);
            }
            Ok(PackDays {
                utc: set.iter().copied().map(Some).collect(),
                gibs: set.into_iter().collect(),
                policy,
            })
        }
    }
}

/// The box as one or two boxes with `west <= east`, edges clamped to the world.
fn split_box(b: Bbox) -> Result<Vec<Bbox>, PlanError> {
    let edges = [b.west, b.south, b.east, b.north];
    if edges.iter().any(|e| !e.is_finite()) || b.south > b.north {
        return Err(PlanError::InvalidBox);
    }
    let south = b.south.clamp(-90.0, 90.0);
    let north = b.north.clamp(-90.0, 90.0);
    let lon = |v: f64| v.clamp(-180.0, 180.0);
    if b.west <= b.east {
        return Ok(vec![Bbox {
            west: lon(b.west),
            south,
            east: lon(b.east),
            north,
        }]);
    }
    Ok(vec![
        Bbox {
            west: lon(b.west),
            south,
            east: 180.0,
            north,
        },
        Bbox {
            west: -180.0,
            south,
            east: lon(b.east),
            north,
        },
    ])
}

/// The box limited to Web Mercator's latitudes, or none when it lies wholly beyond them.
fn mercator_box(b: Bbox) -> Option<Bbox> {
    let (south, north) = (b.south.max(-MAX_LATITUDE), b.north.min(MAX_LATITUDE));
    (south <= north).then_some(Bbox { south, north, ..b })
}

fn host_of(source: TileSource) -> &'static str {
    match source {
        TileSource::Gibs(_) => "gibs.earthdata.nasa.gov",
        TileSource::NcdsWmts | TileSource::NcdsWms { .. } => "gis.charttools.noaa.gov",
        TileSource::GebcoWms => "wms.gebco.net",
        TileSource::Radar => "mesonet.agron.iastate.edu",
    }
}

fn grid_host_name(node: GridHost) -> &'static str {
    match node {
        GridHost::Pfeg => "coastwatch.pfeg.noaa.gov",
        GridHost::Coastwatch => "coastwatch.noaa.gov",
        GridHost::Glerl => "apps.glerl.noaa.gov",
        GridHost::NceiDem => DEM_HOST,
    }
}

fn tile_seed(layer: &Layer, source: TileSource) -> u64 {
    match source {
        TileSource::Gibs(spec) if spec.ext == "jpg" => SEED_GIBS_JPEG,
        TileSource::Gibs(_) if layer.id.contains("chlorophyll") => SEED_GIBS_CHLOROPHYLL,
        TileSource::Gibs(_) => SEED_GIBS_SST,
        TileSource::NcdsWmts => SEED_NCDS_WMTS,
        TileSource::NcdsWms { .. } => SEED_NCDS_WMS,
        TileSource::GebcoWms => SEED_GEBCO,
        TileSource::Radar => 0,
    }
}

fn fmt_date(d: Option<LocalDate>) -> String {
    d.map_or_else(
        || "latest".to_owned(),
        |d| format!("{:04}-{:02}-{:02}", d.year, d.month, d.day),
    )
}

fn plan_layer(
    b: &mut Builder<'_>,
    pack: &Pack,
    layer: &'static Layer,
    boxes: &[Bbox],
    days: &PackDays<'_>,
) -> Result<(), Stop> {
    if !matches!(layer.availability, Availability::Available) {
        return Err(Stop::Skip(SkipReason::Unavailable));
    }
    match layer.kind {
        LayerKind::Tiles(TileSource::Radar) => Err(Stop::Skip(SkipReason::NotPackable)),
        LayerKind::Tiles(source) => plan_tiles(b, pack, layer, source, boxes, days),
        LayerKind::Grid(grid) if layer.date_mode != DateMode::Frames => {
            plan_grid(b, pack, layer, &grid.primary.source(), boxes, days)
        }
        _ => Err(Stop::Skip(SkipReason::NotPackable)),
    }
}

fn plan_tiles(
    b: &mut Builder<'_>,
    pack: &Pack,
    layer: &'static Layer,
    source: TileSource,
    boxes: &[Bbox],
    days: &PackDays<'_>,
) -> Result<(), Stop> {
    let mut lo = pack.min_zoom.max(layer.min_zoom.unwrap_or(0));
    let mut hi = pack.max_zoom.min(layer.max_zoom.unwrap_or(MAX_ZOOM));
    match source {
        TileSource::Gibs(spec) => hi = hi.min(spec.max_zoom),
        TileSource::NcdsWmts | TileSource::NcdsWms { .. } => {
            lo = lo.max(*NCDS_ZOOMS.start());
            hi = hi.min(*NCDS_ZOOMS.end());
        }
        _ => {}
    }
    if lo > hi {
        return Err(Stop::Skip(SkipReason::ZoomOutOfRange));
    }
    let dates: Vec<Option<LocalDate>> = match (source, layer.date_mode) {
        (TileSource::Gibs(_), _) => days.gibs.iter().copied().map(Some).collect(),
        (_, DateMode::None) => vec![None],
        _ => days.utc.clone(),
    };
    let item_bytes = b.size(layer.id, tile_seed(layer, source));
    let host = host_of(source);
    for z in lo..=hi {
        for bx in boxes {
            let Some(bx) = mercator_box(*bx) else {
                continue;
            };
            // One more than the room per date: getting it means the plan is over the cap, and
            // the covering function stops there instead of listing a fourfold-per-zoom set.
            let per_date_room = b.room() / dates.len().max(1);
            let tiles = tiles_covering(bx, z, per_date_room.saturating_add(1));
            if tiles.len() > per_date_room {
                return Err(Stop::Cap);
            }
            for date in &dates {
                for tile in &tiles {
                    b.items.push(PackItem {
                        key: format!(
                            "{}/{}/{}/{}/{}",
                            layer.id,
                            fmt_date(*date),
                            tile.z,
                            tile.x,
                            tile.y
                        ),
                        layer: layer.layer_id(),
                        kind: ItemKind::Tile {
                            tile: *tile,
                            date: *date,
                        },
                        host: host.to_owned(),
                        est_bytes: item_bytes,
                    });
                    b.tile_count += 1;
                }
            }
        }
    }
    Ok(())
}

/// An upper bound on the blocks covering `bx` at a block side, before they are built.
fn block_bound(bx: Bbox, side_deg: f64) -> usize {
    let axis = |lo: f64, hi: f64| ((hi - lo) / side_deg).ceil().max(0.0) as usize + 1;
    axis(bx.south, bx.north).saturating_mul(axis(bx.west, bx.east))
}

fn plan_grid(
    b: &mut Builder<'_>,
    pack: &Pack,
    layer: &'static Layer,
    source: &GridSource,
    boxes: &[Bbox],
    days: &PackDays<'_>,
) -> Result<(), Stop> {
    let lo = pack.min_zoom.max(layer.min_zoom.unwrap_or(0));
    let hi = pack.max_zoom.min(layer.max_zoom.unwrap_or(MAX_ZOOM));
    if lo > hi {
        return Err(Stop::Skip(SkipReason::ZoomOutOfRange));
    }
    let times = if layer.date_mode == DateMode::None {
        vec![None]
    } else {
        grid_times(days.policy, b.axes.get(layer.id))
    };
    let dates = times.as_slice();
    let variables = u64::try_from(source.variables.len().max(1)).unwrap_or(1);
    let row = SEED_GRID_CELL + SEED_GRID_EXTRA_VARIABLE * (variables - 1);
    let host = grid_host_name(source.node);
    // One set of blocks per distinct stride the zooms need; the stride never grows with zoom.
    let mut strides = BTreeSet::new();
    for z in lo..=hi {
        let stride = stride_for_zoom(source.native_step_deg, f64::from(z));
        if !strides.insert(stride) {
            continue;
        }
        let side = f64::from(block_side_deg(source.native_step_deg, stride));
        let side_cells = u64::from(block_side_cells(source.native_step_deg, stride));
        let item_bytes = b.size(layer.id, side_cells.saturating_mul(side_cells) * row);
        for bx in boxes {
            // The room left, after the other half of a box across the antimeridian.
            let per_date_room = b.room() / dates.len().max(1);
            // The bound is loose by a row and a column of blocks; far over the room is certain.
            if block_bound(*bx, side) / 2 > per_date_room.saturating_add(2) {
                return Err(Stop::Cap);
            }
            let blocks = blocks_for_view(source, *bx, f64::from(z));
            if blocks.len() > per_date_room {
                return Err(Stop::Cap);
            }
            for time in dates {
                for block in &blocks {
                    b.items.push(PackItem {
                        key: format!(
                            "{}/{}/{}/{}/{}/{}",
                            layer.id,
                            source.dataset,
                            time.map_or_else(|| "last".to_owned(), |t| t.0.to_string()),
                            block.south,
                            block.west,
                            block.stride
                        ),
                        layer: layer.layer_id(),
                        kind: ItemKind::Grid {
                            block: *block,
                            time: *time,
                        },
                        host: host.to_owned(),
                        est_bytes: item_bytes,
                    });
                    b.grid_count += 1;
                }
            }
        }
    }
    Ok(())
}

/// Degrees as an elevation key holds them: millionths.
fn micro(deg: f64) -> i64 {
    (deg * 1e6).round() as i64
}

fn plan_dem(b: &mut Builder<'_>, pack: &Pack, boxes: &[Bbox]) -> Result<(), PlanError> {
    let pixel = 360.0 / (256.0 * f64::from(pack.max_zoom).exp2());
    // Kept to millionths, as the key holds it, so the request is the key's.
    let cell_micro = micro(pixel.max(DEM_MIN_CELL_DEG)).max(1);
    let cell = cell_micro as f64 / 1e6;
    let side = cell * f64::from(DEM_AREA_PX);
    let item_bytes = b.size(DEM_LAYER, SEED_DEM_AREA);
    let cap = b.cap;
    for bx in boxes {
        // A box of no width or height (the far half of one that starts on the antimeridian)
        // has no area to hold.
        if bx.east <= bx.west || bx.north <= bx.south {
            continue;
        }
        // Areas are aligned to multiples of their side so packs share them.
        let first = |lo: f64| (lo / side).floor() as i64;
        let last = |lo: f64, hi: f64| ((hi / side).ceil() as i64 - 1).max(first(lo));
        let (ix0, ix1) = (first(bx.west), last(bx.west, bx.east));
        let (iy0, iy1) = (first(bx.south), last(bx.south, bx.north));
        let count = (ix1 - ix0 + 1).saturating_mul(iy1 - iy0 + 1);
        if usize::try_from(count).unwrap_or(usize::MAX) > b.room() {
            return Err(PlanError::TooManyItems { cap });
        }
        for iy in (iy0..=iy1).rev() {
            for ix in ix0..=ix1 {
                let (west, south) = (micro(ix as f64 * side), micro(iy as f64 * side));
                let (east, north) = (micro((ix + 1) as f64 * side), micro((iy + 1) as f64 * side));
                let deg = |m: i64| m as f64 / 1e6;
                b.items.push(PackItem {
                    key: format!("dem/area/{west}/{south}/{east}/{north}/{cell_micro}"),
                    layer: LayerId::from_static(DEM_LAYER),
                    kind: ItemKind::Dem {
                        area: Bbox {
                            west: deg(west),
                            south: deg(south),
                            east: deg(east),
                            north: deg(north),
                        },
                        cell_deg: cell,
                    },
                    host: DEM_HOST.to_owned(),
                    est_bytes: item_bytes,
                });
                b.dem_count += 1;
            }
        }
    }
    Ok(())
}

/// Seconds to fetch the items: an ERDDAP node at its pace, any other host at a fixed low
/// concurrency; hosts are counted one after the other.
fn estimate_seconds(items: &[PackItem]) -> u64 {
    let mut per_host: BTreeMap<&str, u64> = BTreeMap::new();
    for item in items {
        *per_host.entry(item.host.as_str()).or_default() += 1;
    }
    let erddap = [GridHost::Pfeg, GridHost::Coastwatch, GridHost::Glerl].map(grid_host_name);
    per_host
        .into_iter()
        .map(|(host, n)| {
            if erddap.contains(&host) {
                (n * 60).div_ceil(ERDDAP_REQUESTS_PER_MINUTE)
            } else {
                (n.div_ceil(OTHER_CONCURRENCY) * OTHER_DECISECONDS_PER_REQUEST).div_ceil(10)
            }
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::catalogue;

    const NOW: UnixTime = UnixTime(1_794_042_000);

    fn harbour() -> Bbox {
        Bbox {
            west: -74.10,
            south: 40.60,
            east: -73.95,
            north: 40.75,
        }
    }

    fn pack(layers: &[&str], min_zoom: u8, max_zoom: u8, dates: DatePolicy) -> Pack {
        Pack {
            version: PACK_VERSION,
            id: "p".into(),
            name: "Harbour".into(),
            bbox: harbour(),
            min_zoom,
            max_zoom,
            layers: layers.iter().map(|l| LayerId::new(*l)).collect(),
            dates,
            spots: vec![],
            created_at: NOW,
            refreshed_at: None,
        }
    }

    fn run(p: &Pack) -> Result<PackPlan, PlanError> {
        plan(
            p,
            catalogue::find,
            &MeasuredSizes::new(),
            &TimeAxes::new(),
            NOW,
        )
    }

    fn covering(z: u8) -> usize {
        tiles_covering(harbour(), z, usize::MAX).len()
    }

    #[test]
    fn chart_tiles_match_the_covering_function() {
        let p = pack(&["noaa-chart"], 10, 14, DatePolicy::Latest);
        let plan = run(&p).unwrap();
        let expected: usize = (10..=14).map(covering).sum();
        assert_eq!(plan.tile_count, expected);
        assert_eq!(plan.len(), expected);
        assert_eq!(plan.est_bytes, expected as u64 * SEED_NCDS_WMTS);
        assert_eq!(plan.grid_count + plan.dem_count, 0);
    }

    #[test]
    fn gibs_sst_stops_at_its_deepest_zoom() {
        let p = pack(&["sst-imagery"], 5, 12, DatePolicy::Latest);
        let plan = run(&p).unwrap();
        let max = plan
            .items(0, usize::MAX)
            .iter()
            .map(|i| match i.kind {
                ItemKind::Tile { tile, .. } => tile.z,
                _ => 0,
            })
            .max();
        assert_eq!(max, Some(7));
        let expected: usize = (5..=7).map(covering).sum();
        assert_eq!(plan.tile_count, expected);
        assert_eq!(plan.est_bytes, expected as u64 * SEED_GIBS_SST);
    }

    #[test]
    fn three_days_of_chlorophyll_plan_three_dates() {
        let p = pack(&["chlorophyll-imagery"], 6, 6, DatePolicy::LastDays(3));
        let plan = run(&p).unwrap();
        let dates: BTreeSet<_> = plan
            .items(0, usize::MAX)
            .iter()
            .filter_map(|i| match i.kind {
                ItemKind::Tile { date, .. } => date,
                _ => None,
            })
            .collect();
        assert_eq!(dates.len(), 3);
        assert_eq!(plan.tile_count, 3 * covering(6));
        assert_eq!(
            plan.est_bytes,
            3 * covering(6) as u64 * SEED_GIBS_CHLOROPHYLL
        );
        let keys: BTreeSet<_> = plan.items(0, usize::MAX).iter().map(|i| &i.key).collect();
        assert_eq!(keys.len(), plan.len());
    }

    #[test]
    fn measured_sizes_replace_seeds() {
        let p = pack(&["noaa-chart"], 12, 12, DatePolicy::Latest);
        let mut m = MeasuredSizes::new();
        m.insert("noaa-chart".into(), 1000);
        let plan = super::plan(&p, catalogue::find, &m, &TimeAxes::new(), NOW).unwrap();
        assert_eq!(plan.est_bytes, plan.tile_count as u64 * 1000);
    }

    #[test]
    fn an_over_cap_plan_is_refused() {
        let mut p = pack(&["noaa-chart"], 8, 20, DatePolicy::Latest);
        p.bbox = Bbox {
            west: -80.0,
            south: 30.0,
            east: -70.0,
            north: 45.0,
        };
        assert_eq!(run(&p), Err(PlanError::TooManyItems { cap: MAX_ITEMS }));
        let small = pack(&["noaa-chart"], 12, 12, DatePolicy::Latest);
        assert_eq!(
            plan_with_cap(
                &small,
                catalogue::find,
                &MeasuredSizes::new(),
                &TimeAxes::new(),
                NOW,
                3
            ),
            Err(PlanError::TooManyItems { cap: 3 })
        );
    }

    #[test]
    fn dates_multiply_into_the_cap() {
        let p = pack(&["chlorophyll-imagery"], 6, 6, DatePolicy::LastDays(3));
        let m = MeasuredSizes::new();
        let n = covering(6);
        assert!(plan_with_cap(&p, catalogue::find, &m, &TimeAxes::new(), NOW, 3 * n - 1).is_err());
        assert!(plan_with_cap(&p, catalogue::find, &m, &TimeAxes::new(), NOW, 3 * n).is_ok());
    }

    #[test]
    fn paging_returns_every_item_once() {
        let p = pack(&["noaa-chart", "gebco-relief"], 8, 11, DatePolicy::Latest);
        let plan = run(&p).unwrap();
        let mut keys = Vec::new();
        let mut from = 0;
        loop {
            let page = plan.items(from, 7);
            if page.is_empty() {
                break;
            }
            keys.extend(page.iter().map(|i| i.key.clone()));
            from += page.len();
        }
        assert_eq!(keys.len(), plan.len());
        assert_eq!(keys.iter().collect::<BTreeSet<_>>().len(), plan.len());
        assert_eq!(plan.items(plan.len() + 5, 10).len(), 0);
        assert_eq!(plan.items(usize::MAX, usize::MAX).len(), 0);
    }

    #[test]
    fn grids_plan_blocks_per_date_and_are_not_tiles() {
        let p = pack(&["sst"], 8, 10, DatePolicy::LastDays(2));
        let plan =
            super::plan(&p, catalogue::find, &MeasuredSizes::new(), &mur_axis(), NOW).unwrap();
        assert_eq!(plan.tile_count, 0);
        assert!(plan.grid_count > 0);
        assert_eq!(plan.grid_count % 2, 0);
        assert!(plan.items(0, 1)[0].host.contains("pfeg"));
        // ERDDAP paces at 10 requests a minute: six seconds a request.
        assert_eq!(plan.est_seconds, plan.grid_count as u64 * 6);
        assert!(plan.est_bytes > 0);
    }

    #[test]
    fn dem_areas_are_planned_apart_from_tiles() {
        let p = pack(&[DEM_LAYER], 12, 14, DatePolicy::Latest);
        let plan = run(&p).unwrap();
        assert!(plan.dem_count > 0);
        assert_eq!(plan.tile_count, 0);
        assert_eq!(plan.est_bytes, plan.dem_count as u64 * SEED_DEM_AREA);
        let ItemKind::Dem { area, .. } = plan.items(0, 1)[0].kind else {
            panic!("not a DEM item");
        };
        assert!(area.east > area.west && area.north > area.south);
        // The key is the elevation store's: the area and the cell in millionths of a degree.
        for item in plan.items(0, usize::MAX) {
            let ItemKind::Dem { area, cell_deg } = item.kind else {
                panic!("not a DEM item");
            };
            let m = |deg: f64| (deg * 1e6).round() as i64;
            assert_eq!(
                item.key,
                format!(
                    "dem/area/{}/{}/{}/{}/{}",
                    m(area.west),
                    m(area.south),
                    m(area.east),
                    m(area.north),
                    m(cell_deg)
                )
            );
        }
    }

    fn at(time: &str) -> UnixTime {
        crate::time::parse_rfc3339(time).unwrap()
    }

    /// MUR's time axis on [`NOW`] (2026-11-07T09:00Z): daily at 09:00Z, the newest a day old.
    fn mur_axis() -> TimeAxes {
        let times = [
            "2026-11-04T09:00:00Z",
            "2026-11-06T09:00:00Z",
            "2026-11-05T09:00:00Z",
            "2026-11-03T09:00:00Z",
        ];
        TimeAxes::from([("sst".to_owned(), times.map(at).to_vec())])
    }

    fn planned_times(plan: &PackPlan) -> BTreeSet<Option<UnixTime>> {
        plan.items(0, usize::MAX)
            .iter()
            .filter_map(|i| match i.kind {
                ItemKind::Grid { time, .. } => Some(time),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn grid_days_count_back_from_the_newest_time_on_the_axis() {
        let m = MeasuredSizes::new();
        let p = pack(&["sst"], 8, 8, DatePolicy::LastDays(3));
        let plan = super::plan(&p, catalogue::find, &m, &mur_axis(), NOW).unwrap();
        // The newest as `(last)`, then the axis's own times: never a time of a day the
        // dataset does not have yet.
        assert_eq!(
            planned_times(&plan),
            BTreeSet::from([
                None,
                Some(at("2026-11-05T09:00:00Z")),
                Some(at("2026-11-04T09:00:00Z"))
            ])
        );
        // Keys are the map store's.
        for item in plan.items(0, usize::MAX) {
            let ItemKind::Grid { block, time } = item.kind else {
                panic!("not a grid item");
            };
            let time = time.map_or_else(|| "last".to_owned(), |t| t.0.to_string());
            assert_eq!(
                item.key,
                format!(
                    "sst/jplMURSST41/{time}/{}/{}/{}",
                    block.south, block.west, block.stride
                )
            );
        }
        // Without the axis only the newest can be named.
        assert_eq!(planned_times(&run(&p).unwrap()), BTreeSet::from([None]));
        // Chosen days take the axis's times on them, the newest's day as `(last)`.
        let day = |day| LocalDate {
            year: 2026,
            month: 11,
            day,
        };
        let chosen = pack(
            &["sst"],
            8,
            8,
            DatePolicy::Days(vec![day(7), day(6), day(3)]),
        );
        let plan = super::plan(&chosen, catalogue::find, &m, &mur_axis(), NOW).unwrap();
        assert_eq!(
            planned_times(&plan),
            BTreeSet::from([None, Some(at("2026-11-03T09:00:00Z"))])
        );
        // The latest is `(last)` whatever the axis says.
        let latest = pack(&["sst"], 8, 8, DatePolicy::Latest);
        let plan = super::plan(&latest, catalogue::find, &m, &mur_axis(), NOW).unwrap();
        assert_eq!(planned_times(&plan), BTreeSet::from([None]));
    }

    #[test]
    fn unpackable_layers_are_reported() {
        let p = pack(
            &["radar", "spots", "nonsense", "apple-standard"],
            8,
            9,
            DatePolicy::Latest,
        );
        let plan = run(&p).unwrap();
        assert!(plan.is_empty());
        let reasons: Vec<_> = plan.skipped.iter().map(|s| s.reason).collect();
        assert_eq!(
            reasons,
            [
                SkipReason::NotPackable,
                SkipReason::NotPackable,
                SkipReason::Unknown,
                SkipReason::NotPackable
            ]
        );
    }

    #[test]
    fn a_box_across_the_antimeridian_is_split() {
        let mut p = pack(&["noaa-chart"], 6, 6, DatePolicy::Latest);
        p.bbox = Bbox {
            west: 179.0,
            south: -10.0,
            east: -179.0,
            north: 10.0,
        };
        let plan = run(&p).unwrap();
        let xs: BTreeSet<u32> = plan
            .items(0, usize::MAX)
            .iter()
            .filter_map(|i| match i.kind {
                ItemKind::Tile { tile, .. } => Some(tile.x),
                _ => None,
            })
            .collect();
        assert!(xs.contains(&0) && xs.contains(&63));
        assert_eq!(xs.len(), 2);
    }

    #[test]
    fn the_grid_cap_counts_both_halves_of_a_box_across_the_antimeridian() {
        let mut p = pack(&["sst"], 8, 8, DatePolicy::Latest);
        p.bbox = Bbox {
            west: 178.5,
            south: -1.0,
            east: -178.5,
            north: 1.0,
        };
        let m = MeasuredSizes::new();
        let axes = TimeAxes::new();
        let all = run(&p).unwrap().len();
        assert!(all >= 2);
        assert_eq!(
            plan_with_cap(&p, catalogue::find, &m, &axes, NOW, all - 1),
            Err(PlanError::TooManyItems { cap: all - 1 })
        );
        assert_eq!(
            plan_with_cap(&p, catalogue::find, &m, &axes, NOW, all).map(|plan| plan.len()),
            Ok(all)
        );
    }

    #[test]
    fn boxes_without_area_hold_no_elevation_areas() {
        let mut p = pack(&[DEM_LAYER], 10, 10, DatePolicy::Latest);
        // The east half of the split box is a line on the antimeridian.
        p.bbox = Bbox {
            west: 180.0,
            south: 10.0,
            east: -179.9,
            north: 10.1,
        };
        let plan = run(&p).unwrap();
        assert!(plan.dem_count > 0);
        for item in plan.items(0, usize::MAX) {
            let ItemKind::Dem { area, .. } = item.kind else {
                panic!("not a DEM item");
            };
            assert!(area.west < -179.9 && area.east > -180.0, "{area:?}");
        }
        // A box of no width or height holds no area.
        p.bbox = Bbox {
            west: -74.0,
            south: 90.0,
            east: -73.0,
            north: 90.0,
        };
        assert_eq!(run(&p).unwrap().dem_count, 0);
    }

    #[test]
    fn polar_boxes_are_clamped_and_bad_input_refused() {
        let mut p = pack(&["noaa-chart"], 3, 3, DatePolicy::Latest);
        p.bbox = Bbox {
            west: -10.0,
            south: 80.0,
            east: 10.0,
            north: 90.0,
        };
        assert!(!run(&p).unwrap().is_empty());
        p.bbox = Bbox {
            west: -10.0,
            south: 87.0,
            east: 10.0,
            north: 90.0,
        };
        assert!(run(&p).unwrap().is_empty());
        p.bbox = Bbox {
            west: f64::NAN,
            south: 0.0,
            east: 1.0,
            north: 1.0,
        };
        assert_eq!(run(&p), Err(PlanError::InvalidBox));
        let mut q = pack(&["noaa-chart"], 9, 8, DatePolicy::Latest);
        assert_eq!(run(&q), Err(PlanError::InvalidZooms));
        q.min_zoom = 1;
        q.max_zoom = 30;
        assert_eq!(run(&q), Err(PlanError::InvalidZooms));
        for dates in [
            DatePolicy::LastDays(0),
            DatePolicy::LastDays(99),
            DatePolicy::Days(vec![]),
        ] {
            let bad = pack(&["noaa-chart"], 5, 5, dates);
            assert_eq!(run(&bad), Err(PlanError::InvalidDates));
        }
    }

    #[test]
    fn budget_check_uses_megabytes() {
        let p = pack(&["noaa-chart"], 12, 14, DatePolicy::Latest);
        let plan = run(&p).unwrap();
        assert!(plan.exceeds_mb(0));
        assert!(!plan.exceeds_mb(1500));
    }

    #[test]
    fn packs_round_trip_with_their_version() {
        let day = LocalDate {
            year: 2026,
            month: 10,
            day: 1,
        };
        let p = pack(&["noaa-chart"], 8, 10, DatePolicy::Days(vec![day]));
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(serde_json::from_str::<Pack>(&json).unwrap(), p);
        let without_version = json.replace("\"version\":1,", "");
        assert_eq!(
            serde_json::from_str::<Pack>(&without_version)
                .unwrap()
                .version,
            PACK_VERSION
        );
    }

    const HOUR: i64 = 3_600;

    fn refreshed(dates: DatePolicy, ago_s: Option<i64>) -> Pack {
        let mut p = pack(&["noaa-chart"], 10, 14, dates);
        p.refreshed_at = ago_s.map(|s| UnixTime(NOW.0 - s));
        p
    }

    #[test]
    fn a_newest_only_pack_is_stale_after_a_day() {
        let at = |ago| refreshed(DatePolicy::Latest, Some(ago)).staleness(NOW);
        assert_eq!(at(LATEST_FRESH_S), None);
        assert_eq!(at(LATEST_FRESH_S + 1), Some(Stale::NewestOverADay));
        assert_eq!(at(FRESH_S), Some(Stale::NewestOverADay));
        assert_eq!(at(FRESH_S + 1), Some(Stale::OverAWeek));
    }

    #[test]
    fn any_pack_is_stale_after_a_week() {
        for dates in [
            DatePolicy::LastDays(3),
            DatePolicy::Days(vec![NOW.local_date(UtcOffset(0))]),
        ] {
            let at = |ago| refreshed(dates.clone(), Some(ago)).staleness(NOW);
            assert_eq!(at(30 * HOUR), None);
            assert_eq!(at(FRESH_S), None);
            assert_eq!(at(FRESH_S + 1), Some(Stale::OverAWeek));
        }
    }

    #[test]
    fn a_pack_never_downloaded_or_refreshed_later_is_not_stale() {
        assert!(!refreshed(DatePolicy::Latest, None).is_stale(NOW));
        assert!(!refreshed(DatePolicy::Latest, Some(-HOUR)).is_stale(NOW));
        let mut p = refreshed(DatePolicy::Latest, Some(FRESH_S * 2));
        assert!(p.is_stale(NOW));
        p.mark_refreshed(NOW);
        assert_eq!(p.refreshed_at, Some(NOW));
        assert!(!p.is_stale(NOW));
    }

    fn draft() -> PackDraft {
        PackDraft {
            id: "3F2504E0-4F89-11D3-9A0C-0305E82C3301".into(),
            name: "  Harbour ".into(),
            bbox: harbour(),
            min_zoom: 10,
            max_zoom: 14,
            layers: ["noaa-chart", "sst", "noaa-chart", DEM_LAYER]
                .map(LayerId::new)
                .to_vec(),
            dates: DatePolicy::Latest,
            spots: vec!["a".into(), "b".into(), "a".into()],
        }
    }

    #[test]
    fn a_draft_becomes_a_pack_not_yet_downloaded() {
        let p = draft().create(NOW).unwrap();
        assert_eq!(p.version, PACK_VERSION);
        assert_eq!(p.id, "3F2504E0-4F89-11D3-9A0C-0305E82C3301");
        assert_eq!(p.name, "Harbour");
        assert_eq!(p.layers, ["noaa-chart", "sst", DEM_LAYER].map(LayerId::new));
        assert_eq!(p.spots, ["a", "b"]);
        assert_eq!((p.created_at, p.refreshed_at), (NOW, None));
        assert!(run(&p).is_ok());
    }

    #[test]
    fn chosen_days_are_sorted_without_repeats() {
        let d = |day| LocalDate {
            year: 2026,
            month: 11,
            day,
        };
        let p = PackDraft {
            dates: DatePolicy::Days(vec![d(20), d(18), d(20)]),
            ..draft()
        }
        .create(NOW)
        .unwrap();
        assert_eq!(p.dates, DatePolicy::Days(vec![d(18), d(20)]));
    }

    #[test]
    fn a_draft_across_the_antimeridian_is_kept() {
        let bbox = Bbox {
            west: 179.5,
            south: 51.0,
            east: -179.5,
            north: 52.0,
        };
        let p = PackDraft { bbox, ..draft() }.create(NOW).unwrap();
        assert_eq!(p.bbox, bbox);
    }

    #[test]
    fn bad_drafts_are_refused_with_their_reason() {
        let with_box = |west, south, east, north| PackDraft {
            bbox: Bbox {
                west,
                south,
                east,
                north,
            },
            ..draft()
        };
        let cases = [
            (
                PackDraft {
                    id: String::new(),
                    ..draft()
                },
                PlanError::InvalidId,
            ),
            (
                PackDraft {
                    id: "a/b".into(),
                    ..draft()
                },
                PlanError::InvalidId,
            ),
            (
                PackDraft {
                    id: "x".repeat(MAX_ID_LEN + 1),
                    ..draft()
                },
                PlanError::InvalidId,
            ),
            (
                PackDraft {
                    name: " \n".into(),
                    ..draft()
                },
                PlanError::InvalidName,
            ),
            (with_box(-74.1, 40.7, -73.9, 40.6), PlanError::InvalidBox),
            (with_box(-74.1, 40.6, -74.1, 40.7), PlanError::InvalidBox),
            (with_box(-74.1, 40.6, -73.9, 40.6), PlanError::InvalidBox),
            (
                with_box(-74.1, f64::NAN, -73.9, 40.7),
                PlanError::InvalidBox,
            ),
            (with_box(-190.0, 40.6, -73.9, 40.7), PlanError::InvalidBox),
            (with_box(-74.1, 40.6, -73.9, 91.0), PlanError::InvalidBox),
            (
                PackDraft {
                    min_zoom: 15,
                    ..draft()
                },
                PlanError::InvalidZooms,
            ),
            (
                PackDraft {
                    max_zoom: PACK_MAX_ZOOM + 1,
                    ..draft()
                },
                PlanError::InvalidZooms,
            ),
            (
                PackDraft {
                    dates: DatePolicy::LastDays(0),
                    ..draft()
                },
                PlanError::InvalidDates,
            ),
            (
                PackDraft {
                    dates: DatePolicy::Days(vec![]),
                    ..draft()
                },
                PlanError::InvalidDates,
            ),
        ];
        for (d, error) in cases {
            assert_eq!(d.clone().create(NOW), Err(error), "{d:?}");
        }
        let deepest = PackDraft {
            min_zoom: PACK_MAX_ZOOM,
            max_zoom: PACK_MAX_ZOOM,
            ..draft()
        };
        assert!(deepest.create(NOW).is_ok());
    }

    #[test]
    fn the_deepest_pack_zoom_is_the_deepest_any_layer_serves() {
        let deepest = catalogue::LAYERS
            .iter()
            .filter_map(|l| l.max_zoom)
            .max()
            .unwrap();
        assert!(deepest <= PACK_MAX_ZOOM);
        assert_eq!(PACK_MAX_ZOOM, 20);
    }

    #[test]
    fn zoom_presets_widen_and_plan_a_harbour_within_the_cap() {
        let mut last = (0, 0);
        for reach in ZoomReach::ALL {
            let (lo, hi) = reach.zooms();
            assert!(lo < hi && hi <= PACK_MAX_ZOOM, "{reach:?}");
            assert!(lo > last.0 && hi > last.1, "{reach:?}");
            last = (lo, hi);
            let p = pack(
                &["noaa-chart", "sst", DEM_LAYER],
                lo,
                hi,
                DatePolicy::Latest,
            );
            let plan = run(&p).unwrap();
            assert!(plan.tile_count > 0 && !plan.exceeds_mb(1500), "{reach:?}");
        }
        assert!(ZoomReach::ALL.contains(&ZoomReach::DEFAULT));
    }
}
