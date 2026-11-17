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
//!   distinct stride the zoom range needs, for each of the pack's dates. The primary dataset is
//!   planned; a fallback dataset is not.
//! - [`DEM_LAYER`] as elevation areas of the NCEI DEM, not tiles.
//! - Not packed: base maps Apple draws, station and spot markers, state feature layers, ENC
//!   points, radar frames (minutes old) and layers with no source. They are listed in
//!   [`PackPlan::skipped`] with the reason.
//!
//! The box is clamped to Web Mercator's latitude limit for tiles and to the poles for grids; a
//! box across the antimeridian (west east of east) is split into two.

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

/// What an item is and what it addresses.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemKind {
    /// A raster tile; `date` is the UTC day for dated layers.
    Tile {
        tile: TileKey,
        date: Option<LocalDate>,
    },
    /// A grid cache block of the layer's primary dataset; no date asks for the newest time.
    Grid {
        block: Block,
        date: Option<LocalDate>,
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

/// A pack that cannot be planned.
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
/// `measured` replaces the seed size of a layer's items with the store's average.
///
/// # Errors
/// [`PlanError`] for a pack that is invalid or over the item cap.
pub fn plan(
    pack: &Pack,
    catalogue: impl Fn(&str) -> Option<&'static Layer>,
    measured: &MeasuredSizes,
    now: UnixTime,
) -> Result<PackPlan, PlanError> {
    plan_with_cap(pack, catalogue, measured, now, MAX_ITEMS)
}

struct Builder<'a> {
    cap: usize,
    measured: &'a MeasuredSizes,
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
struct PackDays {
    /// For gridded layers and undated lookups: UTC days, or none for the newest time.
    grid: Vec<Option<LocalDate>>,
    /// For GIBS tiles, where the newest day is the default (yesterday, UTC).
    gibs: Vec<LocalDate>,
}

fn pack_days(policy: &DatePolicy, now: UnixTime) -> Result<PackDays, PlanError> {
    match policy {
        DatePolicy::Latest => Ok(PackDays {
            grid: vec![None],
            gibs: vec![gibs_default_date(now)],
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
                grid: back(now.local_date(UtcOffset(0)))
                    .into_iter()
                    .map(Some)
                    .collect(),
                gibs: back(gibs_default_date(now)),
            })
        }
        DatePolicy::Days(days) => {
            let set: BTreeSet<LocalDate> = days.iter().copied().collect();
            if set.is_empty() || set.len() > MAX_DAYS {
                return Err(PlanError::InvalidDates);
            }
            Ok(PackDays {
                grid: set.iter().copied().map(Some).collect(),
                gibs: set.into_iter().collect(),
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
    days: &PackDays,
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
    days: &PackDays,
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
        _ => days.grid.clone(),
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
    days: &PackDays,
) -> Result<(), Stop> {
    let lo = pack.min_zoom.max(layer.min_zoom.unwrap_or(0));
    let hi = pack.max_zoom.min(layer.max_zoom.unwrap_or(MAX_ZOOM));
    if lo > hi {
        return Err(Stop::Skip(SkipReason::ZoomOutOfRange));
    }
    let dates: &[Option<LocalDate>] = if layer.date_mode == DateMode::None {
        &[None]
    } else {
        &days.grid
    };
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
        let per_date_room = b.room() / dates.len().max(1);
        let side_cells = u64::from(block_side_cells(source.native_step_deg, stride));
        let item_bytes = b.size(layer.id, side_cells.saturating_mul(side_cells) * row);
        for bx in boxes {
            // The bound is loose by a row and a column of blocks; far over the room is certain.
            if block_bound(*bx, side) / 2 > per_date_room.saturating_add(2) {
                return Err(Stop::Cap);
            }
            let blocks = blocks_for_view(source, *bx, f64::from(z));
            if blocks.len() > per_date_room {
                return Err(Stop::Cap);
            }
            for date in dates {
                for block in &blocks {
                    b.items.push(PackItem {
                        key: format!(
                            "{}/{}/{}.{}.{}.{}",
                            layer.id,
                            fmt_date(*date),
                            source.dataset,
                            block.south,
                            block.west,
                            block.stride
                        ),
                        layer: layer.layer_id(),
                        kind: ItemKind::Grid {
                            block: *block,
                            date: *date,
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

fn plan_dem(b: &mut Builder<'_>, pack: &Pack, boxes: &[Bbox]) -> Result<(), PlanError> {
    let pixel = 360.0 / (256.0 * f64::from(pack.max_zoom).exp2());
    let cell = pixel.max(DEM_MIN_CELL_DEG);
    let side = cell * f64::from(DEM_AREA_PX);
    let item_bytes = b.size(DEM_LAYER, SEED_DEM_AREA);
    let cap = b.cap;
    for bx in boxes {
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
                let (west, south) = (ix as f64 * side, iy as f64 * side);
                b.items.push(PackItem {
                    key: format!("{DEM_LAYER}/{cell:.7}/{ix}/{iy}"),
                    layer: LayerId::from_static(DEM_LAYER),
                    kind: ItemKind::Dem {
                        area: Bbox {
                            west,
                            south,
                            east: west + side,
                            north: south + side,
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
        plan(p, catalogue::find, &MeasuredSizes::new(), NOW)
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
        let plan = super::plan(&p, catalogue::find, &m, NOW).unwrap();
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
            plan_with_cap(&small, catalogue::find, &MeasuredSizes::new(), NOW, 3),
            Err(PlanError::TooManyItems { cap: 3 })
        );
    }

    #[test]
    fn dates_multiply_into_the_cap() {
        let p = pack(&["chlorophyll-imagery"], 6, 6, DatePolicy::LastDays(3));
        let m = MeasuredSizes::new();
        let n = covering(6);
        assert!(plan_with_cap(&p, catalogue::find, &m, NOW, 3 * n - 1).is_err());
        assert!(plan_with_cap(&p, catalogue::find, &m, NOW, 3 * n).is_ok());
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
        let plan = run(&p).unwrap();
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
}
