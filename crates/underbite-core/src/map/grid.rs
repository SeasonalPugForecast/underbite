//! Gridded values (ERDDAP and GLERL grids): blocks with ascending axes.
//!
//! A map view asks for fixed square [`Block`]s at a stride chosen for its zoom
//! ([`blocks_for_view`]), so that the same block serves every view that overlaps it and caches
//! under one [`BlockKey`]. A block is 1° × 1° at fine strides and grows with the stride in wide
//! views ([`block_side_deg`]), so a screen needs a handful of requests at any zoom. The blocks of one dataset and time join with [`Grid::merge`]; a grid
//! answers point queries ([`Grid::sample_bilinear`], [`Grid::sample_nearest`]) and the value range
//! of an area ([`Grid::range_in`]).

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::map::{Bbox, GeoPoint, GridSource};
use crate::time::UnixTime;

/// Seconds in a day.
const DAY: i64 = 86_400;

/// The side of a block at a fine stride, degrees; coarser strides use [`block_side_deg`].
pub const BLOCK_DEG: f64 = 1.0;

/// The block sides, degrees: the whole numbers that divide both 90 and 180, so that blocks tile
/// the world exactly and none crosses a pole or the antimeridian.
pub const BLOCK_SIDES_DEG: [u16; 12] = [1, 2, 3, 5, 6, 9, 10, 15, 18, 30, 45, 90];

/// A 1° block with fewer cells than this along a side is too small to be worth a request, and the
/// block grows ([`block_side_deg`]).
pub const MIN_BLOCK_SIDE_CELLS: u32 = 20;

/// Below this zoom a view is "wide": cells are [`WIDE_PIXELS_PER_CELL`] tile pixels, not one.
pub const WIDE_ZOOM: f64 = 8.0;

/// Tile pixels per cell in a wide view, so a screen of blocks stays a few requests.
pub const WIDE_PIXELS_PER_CELL: f64 = 3.0;

/// The most cells a block holds at the stride [`stride_for_zoom`] picks.
pub const MAX_BLOCK_CELLS: u32 = 10_000;

/// The width of a Web Mercator tile, pixels.
pub const TILE_PX: f64 = 256.0;

/// The most cells [`Grid::merge`] builds: about 2,000 by 2,000.
pub const MAX_MERGED_CELLS: usize = 4_000_000;

/// Datasets whose recent days are revised after first release. MUR SST: "the data for the most
/// recent 7 days is usually revised every day".
const REVISED_DATASETS: &[&str] = &["jplMURSST41"];

/// How far back a revised dataset changes its days, seconds.
pub const REVISION_WINDOW_S: i64 = 7 * DAY;

/// How long a block of a revised day is kept before it is fetched again, seconds.
pub const REVISED_MAX_AGE_S: i64 = DAY;

/// A grid could not be built.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GridError {
    /// An axis has no points.
    #[error("the grid has an empty axis")]
    EmptyAxis,
    /// An axis holds a value that is not a number, or is not strictly monotonic.
    #[error("the {axis} axis is not strictly monotonic and finite")]
    BadAxis { axis: &'static str },
    /// `values` does not hold one value per cell.
    #[error("{found} values for {lats} latitudes by {lons} longitudes")]
    ShapeMismatch {
        lats: usize,
        lons: usize,
        found: usize,
    },
}

/// A block of values at one time over a latitude and longitude axis.
///
/// `values` is row-major: the value at latitude index `i` and longitude index `j` is
/// `values[i * lons.len() + j]`. A missing value (land, cloud) is NaN. `lats` and `lons` are both
/// ascending. The axes need not be evenly spaced: a Web Mercator row grid is a list of latitudes.
/// Build a grid with [`Grid::new`], which keeps those promises; the fields are public for reading.
#[derive(Clone, Debug, PartialEq)]
pub struct Grid {
    pub source: GridSource,
    /// The instant the values describe.
    pub time: UnixTime,
    /// Degrees north, ascending.
    pub lats: Vec<f64>,
    /// Degrees east, ascending.
    pub lons: Vec<f64>,
    /// NaN where a value is missing.
    pub values: Vec<f32>,
    /// The values' unit as the source states it (`degree_C`, `m s-1`, `mg m-3`).
    pub unit: String,
}

fn strictly_ascending(axis: &[f64]) -> bool {
    axis.iter().all(|v| v.is_finite()) && axis.windows(2).all(|w| w[0] < w[1])
}

impl Grid {
    /// Builds a grid from rows as the source lists them. A descending latitude axis (the VIIRS
    /// datasets list the northernmost row first) is reversed together with its rows so that
    /// `lats` ascends.
    ///
    /// # Errors
    /// [`GridError`] for an empty or non-monotonic axis, a longitude axis that descends, or a
    /// `values` length that is not `lats.len() * lons.len()`.
    pub fn new(
        source: GridSource,
        time: UnixTime,
        mut lats: Vec<f64>,
        lons: Vec<f64>,
        mut values: Vec<f32>,
        unit: impl Into<String>,
    ) -> Result<Self, GridError> {
        if lats.is_empty() || lons.is_empty() {
            return Err(GridError::EmptyAxis);
        }
        if !strictly_ascending(&lons) {
            return Err(GridError::BadAxis { axis: "longitude" });
        }
        if values.len() != lats.len() * lons.len() {
            return Err(GridError::ShapeMismatch {
                lats: lats.len(),
                lons: lons.len(),
                found: values.len(),
            });
        }
        if !strictly_ascending(&lats) {
            lats.reverse();
            if !strictly_ascending(&lats) {
                return Err(GridError::BadAxis { axis: "latitude" });
            }
            values = values
                .chunks_exact(lons.len())
                .rev()
                .flatten()
                .copied()
                .collect();
        }
        Ok(Self {
            source,
            time,
            lats,
            lons,
            values,
            unit: unit.into(),
        })
    }
}

/// Blocks could not be merged into one grid.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MergeError {
    /// No blocks were given.
    #[error("no blocks to merge")]
    Empty,
    /// The blocks come from different datasets or times, or state different units.
    #[error("the blocks differ in {0}")]
    Mismatch(&'static str),
    /// The merged grid would exceed [`MAX_MERGED_CELLS`].
    #[error("the merged grid would hold {0} cells")]
    TooLarge(usize),
    /// The merged axes do not make a grid.
    #[error(transparent)]
    Grid(#[from] GridError),
}

impl Grid {
    /// The value at latitude index `i` and longitude index `j`; `None` outside the grid.
    pub fn value(&self, i: usize, j: usize) -> Option<f32> {
        if i >= self.lats.len() || j >= self.lons.len() {
            return None;
        }
        self.values.get(i * self.lons.len() + j).copied()
    }

    /// How many cells hold a value.
    pub fn valid_count(&self) -> usize {
        self.values.iter().filter(|v| v.is_finite()).count()
    }

    /// The value at `at`, interpolated between the four surrounding cell centres.
    ///
    /// This is the one sampling rule of the map: [`render`](crate::map::render) colours its tiles
    /// with it, so the point inspector reads what the tile shows. Missing cells are left out and
    /// the others reweighted, so a point near a coast still gets the water's value; but a point
    /// whose nearest cell centre is missing (land, cloud) has no value. At a cell centre the
    /// result is that cell's value. A cell stands for the square around its centre, so the grid
    /// reaches half a step beyond its outermost centres, where the edge cells' values are used
    /// (this lets neighbouring blocks join without a seam); `None` beyond that.
    pub fn sample_bilinear(&self, at: GeoPoint) -> Option<f32> {
        let lone = half_step(self.source.native_step_deg);
        let row = locate(&self.lats, at.lat, lone)?;
        let col = locate_lon(&self.lons, at.lon, lone)?;
        self.sample_cell(row, col)
    }

    /// The reweighted bilinear value of a located point; see [`Grid::sample_bilinear`].
    pub(crate) fn sample_cell(&self, row: Cell, col: Cell) -> Option<f32> {
        let corners = [
            (row.lo, col.lo, (1.0 - row.w) * (1.0 - col.w)),
            (row.lo, col.hi, (1.0 - row.w) * col.w),
            (row.hi, col.lo, row.w * (1.0 - col.w)),
            (row.hi, col.hi, row.w * col.w),
        ];
        let nearest = corners.iter().copied().max_by(|a, b| a.2.total_cmp(&b.2))?;
        if !self.value(nearest.0, nearest.1)?.is_finite() {
            return None;
        }
        let (mut sum, mut weight) = (0.0_f64, 0.0_f64);
        for (i, j, w) in corners {
            match self.value(i, j) {
                Some(v) if w > 0.0 && v.is_finite() => {
                    sum += w * f64::from(v);
                    weight += w;
                }
                _ => {}
            }
        }
        (weight > 0.0).then(|| (sum / weight) as f32)
    }

    /// The value of the cell whose centre is nearest to `at`; `None` where it is missing or
    /// `at` lies more than half a cell beyond the grid's edge cells.
    pub fn sample_nearest(&self, at: GeoPoint) -> Option<f32> {
        let half = half_step(self.source.native_step_deg);
        let i = nearest(&self.lats, at.lat, half)?;
        let j = nearest(&self.lons, at.lon, half)?;
        self.value(i, j).filter(|v| v.is_finite())
    }

    /// The smallest and largest value of the cells whose centres lie in `area` (edges included);
    /// `None` when no such cell holds a value.
    pub fn range_in(&self, area: Bbox) -> Option<(f32, f32)> {
        let rows = axis_span(&self.lats, area.south, area.north)?;
        let cols = axis_span(&self.lons, area.west, area.east)?;
        let mut range: Option<(f32, f32)> = None;
        for i in rows {
            for j in cols.clone() {
                if let Some(v) = self.value(i, j).filter(|v| v.is_finite()) {
                    range = Some(range.map_or((v, v), |(lo, hi)| (lo.min(v), hi.max(v))));
                }
            }
        }
        range
    }

    /// Joins blocks of one dataset and time into one grid over the union of their axes.
    ///
    /// Cells no block covers are missing. Where blocks overlap, a value wins over a missing one
    /// and a later block over an earlier. The blocks should share a stride: blocks fetched at
    /// different zooms interleave into a grid with holes.
    ///
    /// # Errors
    /// [`MergeError`] when no block is given, the blocks differ in dataset, node, time or unit, or
    /// the result would be too large.
    pub fn merge(blocks: &[Grid]) -> Result<Grid, MergeError> {
        let first = blocks.first().ok_or(MergeError::Empty)?;
        for g in blocks {
            if g.source.dataset != first.source.dataset || g.source.node != first.source.node {
                return Err(MergeError::Mismatch("dataset"));
            }
            if g.time != first.time {
                return Err(MergeError::Mismatch("time"));
            }
            if g.unit != first.unit {
                return Err(MergeError::Mismatch("unit"));
            }
        }
        let lats = union_axis(blocks.iter().map(|g| g.lats.as_slice()));
        let lons = union_axis(blocks.iter().map(|g| g.lons.as_slice()));
        let cells = lats.len().saturating_mul(lons.len());
        if cells > MAX_MERGED_CELLS {
            return Err(MergeError::TooLarge(cells));
        }
        let mut values = vec![f32::NAN; cells];
        for g in blocks {
            let rows = positions(&lats, &g.lats);
            let cols = positions(&lons, &g.lons);
            for (i, row) in rows.iter().enumerate() {
                for (j, col) in cols.iter().enumerate() {
                    let (Some(row), Some(col)) = (row, col) else {
                        continue;
                    };
                    let Some(v) = g.value(i, j).filter(|v| !v.is_nan()) else {
                        continue;
                    };
                    if let Some(slot) = values.get_mut(row * lons.len() + col) {
                        *slot = v;
                    }
                }
            }
        }
        Ok(Grid::new(
            first.source.clone(),
            first.time,
            lats,
            lons,
            values,
            first.unit.clone(),
        )?)
    }
}

/// Where a coordinate falls between two grid points: the value is
/// `axis[lo] * (1 - w) + axis[hi] * w`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Cell {
    pub(crate) lo: usize,
    pub(crate) hi: usize,
    pub(crate) w: f64,
}

/// Locates a coordinate on an ascending axis. Grid values stand for cells centred on the axis
/// points, so the grid reaches half a step beyond its outermost points; there the value of the
/// edge points is used (`lone_half` for an axis of one point). None outside that.
pub(crate) fn locate(axis: &[f64], x: f64, lone_half: f64) -> Option<Cell> {
    let (&first, &last) = (axis.first()?, axis.last()?);
    if !x.is_finite() {
        return None;
    }
    let n = axis.len();
    let (below, above) = match (axis.get(1), n.checked_sub(2).and_then(|k| axis.get(k))) {
        (Some(&second), Some(&before_last)) => ((second - first) / 2.0, (last - before_last) / 2.0),
        _ => (lone_half, lone_half),
    };
    if x < first - below || x > last + above {
        return None;
    }
    if x <= first {
        return Some(Cell {
            lo: 0,
            hi: 0,
            w: 0.0,
        });
    }
    if x >= last {
        return Some(Cell {
            lo: n - 1,
            hi: n - 1,
            w: 0.0,
        });
    }
    // first < x < last on an ascending axis: 1 <= hi <= n - 1.
    let hi = axis.partition_point(|&a| a <= x).clamp(1, n - 1);
    let lo = hi - 1;
    let span = axis[hi] - axis[lo];
    let w = if span > 0.0 {
        ((x - axis[lo]) / span).clamp(0.0, 1.0)
    } else {
        0.0
    };
    Some(Cell { lo, hi, w })
}

/// Locates a longitude, also trying it a turn east and west for grids on 0 to 360 degrees.
pub(crate) fn locate_lon(axis: &[f64], lon: f64, lone_half: f64) -> Option<Cell> {
    locate(axis, lon, lone_half)
        .or_else(|| locate(axis, lon + 360.0, lone_half))
        .or_else(|| locate(axis, lon - 360.0, lone_half))
}

/// Half the native step, or 0 for a step that is not a positive number.
pub(crate) fn half_step(step: f64) -> f64 {
    if step.is_finite() && step > 0.0 {
        step / 2.0
    } else {
        0.0
    }
}

/// The index of the axis value nearest to `x`, if `x` is within half a spacing of the ends
/// (`lone_half` for an axis of one value).
fn nearest(axis: &[f64], x: f64, lone_half: f64) -> Option<usize> {
    let (&first, &last) = (axis.first()?, axis.last()?);
    if !x.is_finite() {
        return None;
    }
    let (lo_half, hi_half) = match (
        axis.get(1),
        axis.len().checked_sub(2).and_then(|k| axis.get(k)),
    ) {
        (Some(&second), Some(&before_last)) => ((second - first) / 2.0, (last - before_last) / 2.0),
        _ => (lone_half, lone_half),
    };
    // A little slack so that a point exactly on the outer edge of an edge cell belongs to it.
    let slack = 1e-9;
    if x < first - lo_half - slack || x > last + hi_half + slack {
        return None;
    }
    let k = axis.partition_point(|&a| a < x);
    let below = k
        .checked_sub(1)
        .and_then(|i| axis.get(i).map(|&a| (i, x - a)));
    let above = axis.get(k).map(|&a| (k, a - x));
    match (below, above) {
        (Some((i, d0)), Some((j, d1))) => Some(if d0 <= d1 { i } else { j }),
        (Some((i, _)), None) => Some(i),
        (None, Some((j, _))) => Some(j),
        (None, None) => None,
    }
}

/// The indices of an ascending axis whose values lie in `lo..=hi`.
fn axis_span(axis: &[f64], lo: f64, hi: f64) -> Option<std::ops::Range<usize>> {
    if !(lo.is_finite() && hi.is_finite()) || lo > hi {
        return None;
    }
    let start = axis.partition_point(|&a| a < lo);
    let end = axis.partition_point(|&a| a <= hi);
    (start < end).then_some(start..end)
}

/// The sorted union of several axes, each value once.
fn union_axis<'a>(axes: impl Iterator<Item = &'a [f64]>) -> Vec<f64> {
    let mut all: Vec<f64> = axes.flatten().copied().collect();
    all.sort_by(f64::total_cmp);
    all.dedup();
    all
}

/// Where each value of `part` sits in `whole`.
fn positions(whole: &[f64], part: &[f64]) -> Vec<Option<usize>> {
    part.iter()
        .map(|v| whole.binary_search_by(|w| w.total_cmp(v)).ok())
        .collect()
}

/// The stride that makes a grid cell about one tile pixel at `zoom` (three below [`WIDE_ZOOM`]),
/// never finer than the native step and coarse enough that a 1° block holds at most
/// [`MAX_BLOCK_CELLS`].
///
/// A pixel is measured along the equator (`360° / (256 · 2^zoom)`); in latitude it is smaller by
/// the cosine of the latitude, which "about one pixel" allows for. A step that is not a positive
/// number gives stride 1.
pub fn stride_for_zoom(native_step_deg: f64, zoom: f64) -> u32 {
    if !(native_step_deg.is_finite() && native_step_deg > 0.0) {
        return 1;
    }
    let zoom = if zoom.is_finite() {
        zoom.clamp(0.0, 30.0)
    } else {
        0.0
    };
    let per_cell = if zoom < WIDE_ZOOM {
        WIDE_PIXELS_PER_CELL
    } else {
        1.0
    };
    let pixel_deg = per_cell * 360.0 / (TILE_PX * zoom.exp2());
    let for_pixel = (pixel_deg / native_step_deg).round();
    // A block side of n cells at stride s holds ceil(cells per degree / s) cells, and n² must
    // stay within the limit.
    let max_side = f64::from(MAX_BLOCK_CELLS).sqrt().floor();
    let for_size = (BLOCK_DEG / native_step_deg / max_side - 1e-9).ceil();
    for_pixel.max(for_size).clamp(1.0, f64::from(u32::MAX)) as u32
}

/// The most cells along one side of a block: the square root of [`MAX_BLOCK_CELLS`].
const MAX_SIDE_CELLS: u32 = 100;

/// The cells along `side_deg` degrees at `stride`, at most.
fn cells_along(side_deg: f64, native_step_deg: f64, stride: u32) -> u32 {
    if !(native_step_deg.is_finite() && native_step_deg > 0.0) {
        return 1;
    }
    let per_side = side_deg / (native_step_deg * f64::from(stride.max(1)));
    (per_side - 1e-9).ceil().clamp(1.0, f64::from(u32::MAX)) as u32
}

/// The side of a block, in degrees, for a dataset at `stride`.
///
/// 1° while a 1° block holds at least [`MIN_BLOCK_SIDE_CELLS`] cells along a side. Coarser strides
/// (wide views) make that block too small to pay for a request, so it grows to the largest of
/// [`BLOCK_SIDES_DEG`] that holds at most [`MAX_BLOCK_CELLS`] cells. Blocks are aligned to
/// multiples of their side.
pub fn block_side_deg(native_step_deg: f64, stride: u32) -> u16 {
    if !(native_step_deg.is_finite() && native_step_deg > 0.0) {
        return 1;
    }
    if cells_along(BLOCK_DEG, native_step_deg, stride) >= MIN_BLOCK_SIDE_CELLS {
        return 1;
    }
    BLOCK_SIDES_DEG
        .iter()
        .rev()
        .copied()
        .find(|&side| cells_along(f64::from(side), native_step_deg, stride) <= MAX_SIDE_CELLS)
        .unwrap_or(1)
}

/// The number of cells along one side of a block at `stride`, at most.
pub fn block_side_cells(native_step_deg: f64, stride: u32) -> u32 {
    let side = block_side_deg(native_step_deg, stride);
    cells_along(f64::from(side), native_step_deg, stride)
}

/// A square block of a grid, named by its south-west corner in whole degrees, at a stride counted
/// in native grid steps. Its side is [`block_side_deg`] of the dataset's native step and the
/// stride, and its corner is a multiple of that side.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Block {
    /// The southern edge, degrees north (-90 to 89).
    pub south: i16,
    /// The western edge, degrees east (-180 to 179).
    pub west: i16,
    /// Every how many native cells a value is taken.
    pub stride: u32,
}

impl Block {
    /// The block's area, for a dataset with this native step.
    pub fn bounds(self, native_step_deg: f64) -> Bbox {
        let side = f64::from(block_side_deg(native_step_deg, self.stride));
        let (south, west) = (f64::from(self.south), f64::from(self.west));
        Bbox {
            west,
            south,
            east: west + side,
            north: south + side,
        }
    }

    /// The area to request: the block without its northern and eastern edge cells, which belong
    /// to the neighbouring blocks. ERDDAP matches each end to the nearest cell centre, so the
    /// far ends stop one native step short of the edge.
    pub fn request_bbox(self, native_step_deg: f64) -> Bbox {
        let b = self.bounds(native_step_deg);
        let inset = if native_step_deg.is_finite() && native_step_deg > 0.0 {
            native_step_deg.min(b.east - b.west)
        } else {
            0.0
        };
        Bbox {
            east: b.east - inset,
            north: b.north - inset,
            ..b
        }
    }

    /// The cache key of this block of `dataset` at `time`.
    pub fn key(self, dataset: &str, time: UnixTime) -> BlockKey {
        BlockKey {
            dataset: dataset.to_owned(),
            time,
            block: self,
        }
    }
}

/// What a fetched block is cached under: the dataset, the time of the values, and the block with
/// its stride.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct BlockKey {
    /// The ERDDAP dataset id.
    pub dataset: String,
    /// The time the values describe, as the response states it (`(last)` resolved).
    pub time: UnixTime,
    /// The block and its stride.
    pub block: Block,
}

impl fmt::Display for BlockKey {
    /// A file-name-safe form: `jplMURSST41.1794042000.40.-74.5` (dataset, Unix time, south,
    /// west, stride).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}.{}.{}.{}.{}",
            self.dataset, self.time.0, self.block.south, self.block.west, self.block.stride
        )
    }
}

/// The blocks that cover `view` at `zoom`, nearest to the view's centre first, all at the stride
/// [`stride_for_zoom`] picks for `source` and the side [`block_side_deg`] gives for it.
///
/// The view is clipped to -90..90 and -180..180; a view with a non-finite edge, or with `west`
/// east of `east` or `south` north of `north`, has no blocks. A view across the antimeridian is
/// two views. A whole-world view at a fine stride gives tens of thousands of blocks: the caller
/// decides from which zoom a layer is drawn (the catalogue's `min_zoom`).
pub fn blocks_for_view(source: &GridSource, view: Bbox, zoom: f64) -> Vec<Block> {
    let edges = [view.west, view.south, view.east, view.north];
    if edges.iter().any(|e| !e.is_finite()) || view.west > view.east || view.south > view.north {
        return Vec::new();
    }
    let stride = stride_for_zoom(source.native_step_deg, zoom);
    let side = block_side_deg(source.native_step_deg, stride);
    let side_deg = f64::from(side);
    let (south, north) = (view.south.max(-90.0), view.north.min(90.0));
    let (west, east) = (view.west.max(-180.0), view.east.min(180.0));
    if south > north || west > east {
        return Vec::new();
    }
    // Block indices: block `k` spans `k * side` to `(k + 1) * side` degrees.
    let index = |lo: f64, hi: f64, extent: i32| {
        let step = i32::from(side);
        let (min, max) = (-extent / step, extent / step - 1);
        let first = ((lo / side_deg).floor() as i32).clamp(min, max);
        let last = ((hi / side_deg).ceil() as i32 - 1).clamp(first, max);
        first..=last
    };
    let centre = (f64::midpoint(south, north), f64::midpoint(west, east));
    let shrink = centre.0.to_radians().cos().max(0.01);
    let mut blocks: Vec<(f64, Block)> = Vec::new();
    for s in index(south, north, 90) {
        for w in index(west, east, 180) {
            let (s_deg, w_deg) = (s * i32::from(side), w * i32::from(side));
            let block = Block {
                // The indices are clamped to -90..=89 and -180..=179 degrees.
                south: i16::try_from(s_deg).unwrap_or(0),
                west: i16::try_from(w_deg).unwrap_or(0),
                stride,
            };
            let d_lat = f64::from(s_deg) + side_deg / 2.0 - centre.0;
            let d_lon = (f64::from(w_deg) + side_deg / 2.0 - centre.1) * shrink;
            blocks.push((d_lat * d_lat + d_lon * d_lon, block));
        }
    }
    blocks.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    blocks.into_iter().map(|(_, b)| b).collect()
}

/// Whether a cached block of `dataset` for `time`, fetched at `fetched`, may still be used at
/// `now`.
///
/// Most datasets never change a released time, so their blocks keep. MUR SST revises its most
/// recent 7 days every day: a block fetched while its time was within that window is used for
/// [`REVISED_MAX_AGE_S`] and then fetched again; one fetched after the window keeps.
pub fn block_is_fresh(dataset: &str, time: UnixTime, fetched: UnixTime, now: UnixTime) -> bool {
    if !REVISED_DATASETS.contains(&dataset) {
        return true;
    }
    if fetched.0.saturating_sub(time.0) >= REVISION_WINDOW_S {
        return true;
    }
    let age = now.0.saturating_sub(fetched.0);
    (0..REVISED_MAX_AGE_S).contains(&age)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::ErddapNode;

    fn source() -> GridSource {
        GridSource {
            node: ErddapNode::Coastwatch,
            dataset: "noaacwN20VIIRSchlaDaily".to_owned(),
            variables: vec!["chlor_a".to_owned()],
            altitude_axis: true,
            native_step_deg: 0.0375,
        }
    }

    fn build(lats: Vec<f64>, lons: Vec<f64>, values: Vec<f32>) -> Result<Grid, GridError> {
        Grid::new(
            source(),
            UnixTime(1_794_000_000),
            lats,
            lons,
            values,
            "mg m-3",
        )
    }

    #[test]
    fn ascending_latitudes_are_kept() {
        let g = build(
            vec![40.0, 40.1],
            vec![-74.0, -73.9, -73.8],
            vec![1., 2., 3., 4., 5., 6.],
        )
        .unwrap();
        assert_eq!(g.lats, [40.0, 40.1]);
        assert_eq!(g.values, [1., 2., 3., 4., 5., 6.]);
    }

    #[test]
    fn descending_latitudes_are_reversed_with_their_rows() {
        let g = build(
            vec![40.2, 40.1, 40.0],
            vec![-74.0, -73.9],
            vec![1., 2., 3., f32::NAN, 5., 6.],
        )
        .unwrap();
        assert_eq!(g.lats, [40.0, 40.1, 40.2]);
        assert_eq!(g.lons, [-74.0, -73.9]);
        assert_eq!(g.values[..2], [5., 6.]);
        assert_eq!(g.values[2], 3.);
        assert!(g.values[3].is_nan());
        assert_eq!(g.values[4..], [1., 2.]);
        assert_eq!(g.unit, "mg m-3");
    }

    #[test]
    fn uneven_latitude_spacing_is_allowed() {
        assert!(build(vec![40.0, 40.1, 40.3], vec![-74.0], vec![1., 2., 3.]).is_ok());
    }

    #[test]
    fn bad_shapes_and_axes_are_refused() {
        assert_eq!(build(vec![], vec![1.0], vec![]), Err(GridError::EmptyAxis));
        assert_eq!(
            build(vec![40.0, 40.1], vec![-74.0], vec![1.0]),
            Err(GridError::ShapeMismatch {
                lats: 2,
                lons: 1,
                found: 1
            })
        );
        assert_eq!(
            build(vec![40.0, 40.0], vec![-74.0], vec![1.0, 2.0]),
            Err(GridError::BadAxis { axis: "latitude" })
        );
        assert_eq!(
            build(vec![40.0], vec![-73.9, -74.0], vec![1.0, 2.0]),
            Err(GridError::BadAxis { axis: "longitude" })
        );
        assert_eq!(
            build(vec![f64::NAN], vec![-74.0], vec![1.0]),
            Err(GridError::BadAxis { axis: "latitude" })
        );
    }

    const MUR_STEP: f64 = 0.01;
    const VIIRS_750M_STEP: f64 = 0.0075;
    const GLSEA_STEP: f64 = 0.014_015_517;

    fn mur() -> GridSource {
        GridSource {
            node: ErddapNode::Pfeg,
            dataset: "jplMURSST41".to_owned(),
            variables: vec!["analysed_sst".to_owned()],
            altitude_axis: false,
            native_step_deg: MUR_STEP,
        }
    }

    /// 40.0..40.2 N by -74.0..-73.8 E at 0.1 degrees; the middle row's west cell is land.
    fn three_by_three() -> Grid {
        Grid::new(
            mur(),
            UnixTime(1_794_042_000),
            vec![40.0, 40.1, 40.2],
            vec![-74.0, -73.9, -73.8],
            vec![10., 11., 12., f32::NAN, 14., 15., 16., 17., 18.],
            "degree_C",
        )
        .unwrap()
    }

    fn at(lat: f64, lon: f64) -> GeoPoint {
        GeoPoint { lat, lon }
    }

    fn close(a: Option<f32>, b: f32) -> bool {
        a.is_some_and(|a| (a - b).abs() < 1e-4)
    }

    #[test]
    fn stride_makes_a_cell_about_a_pixel() {
        // From zoom 8 a cell is a pixel: at zoom 8 a pixel is 360 / 65536 = 0.0055 degrees.
        assert_eq!(stride_for_zoom(MUR_STEP, 8.0), 1);
        assert_eq!(stride_for_zoom(GLSEA_STEP, 8.0), 1);
        // At 0.001 degrees a pixel is 5 cells, but a 1 degree block would hold 1,000,000.
        assert_eq!(stride_for_zoom(0.001, 8.0), 10);
        // Below zoom 8 a cell is three pixels: at zoom 5 a pixel is 360 / 8192 = 0.0439 degrees,
        // so a cell is about 13 MUR cells.
        assert_eq!(stride_for_zoom(MUR_STEP, 5.0), 13);
        assert_eq!(stride_for_zoom(MUR_STEP, 0.0), 422);
        assert_eq!(stride_for_zoom(GLSEA_STEP, 6.0), 5);
        // Fractional zooms are allowed.
        assert_eq!(stride_for_zoom(MUR_STEP, 4.5), 19);
    }

    #[test]
    fn stride_is_never_finer_than_native_or_too_big_for_a_block() {
        // Zoomed in far: a pixel is smaller than a cell, so every cell, unless the block would
        // hold more than 10,000.
        assert_eq!(stride_for_zoom(GLSEA_STEP, 14.0), 1);
        assert_eq!(stride_for_zoom(MUR_STEP, 14.0), 1);
        assert_eq!(block_side_cells(MUR_STEP, 1), 100);
        assert_eq!(stride_for_zoom(VIIRS_750M_STEP, 14.0), 2);
        assert_eq!(block_side_cells(VIIRS_750M_STEP, 2), 67);
        for step in [
            MUR_STEP,
            VIIRS_750M_STEP,
            GLSEA_STEP,
            0.0375,
            0.001,
            0.000_1,
        ] {
            for z in 0..=20 {
                let stride = stride_for_zoom(step, f64::from(z));
                assert!(stride >= 1);
                let side = block_side_cells(step, stride);
                assert!(side * side <= MAX_BLOCK_CELLS, "{step} at z{z}: {side}");
            }
        }
    }

    #[test]
    fn stride_survives_nonsense() {
        assert_eq!(stride_for_zoom(0.0, 5.0), 1);
        assert_eq!(stride_for_zoom(-0.01, 5.0), 1);
        assert_eq!(stride_for_zoom(f64::NAN, 5.0), 1);
        assert_eq!(stride_for_zoom(MUR_STEP, f64::NAN), 422);
        assert_eq!(stride_for_zoom(MUR_STEP, -3.0), 422);
        assert_eq!(stride_for_zoom(MUR_STEP, 99.0), 1);
        assert_eq!(stride_for_zoom(1e-300, 0.0), u32::MAX);
        assert_eq!(block_side_cells(0.0, 3), 1);
    }

    #[test]
    fn the_harbor_view_needs_two_blocks_nearest_first() {
        let view = Bbox {
            west: -74.2,
            south: 40.0,
            east: -73.0,
            north: 40.8,
        };
        let blocks = blocks_for_view(&mur(), view, 9.0);
        assert_eq!(
            blocks,
            [
                Block {
                    south: 40,
                    west: -74,
                    stride: 1
                },
                Block {
                    south: 40,
                    west: -75,
                    stride: 1
                },
            ]
        );
        assert_eq!(
            blocks[0].request_bbox(MUR_STEP),
            Bbox {
                west: -74.0,
                south: 40.0,
                east: -73.01,
                north: 40.99,
            }
        );
        assert_eq!(
            blocks[1].bounds(MUR_STEP),
            Bbox {
                west: -75.0,
                south: 40.0,
                east: -74.0,
                north: 41.0,
            }
        );
    }

    #[test]
    fn views_on_whole_degrees_and_at_the_edges_of_the_world() {
        let square = Bbox {
            west: -74.0,
            south: 40.0,
            east: -72.0,
            north: 41.0,
        };
        let blocks = blocks_for_view(&mur(), square, 8.0);
        let mut corners: Vec<(i16, i16)> = blocks.iter().map(|b| (b.south, b.west)).collect();
        corners.sort_unstable();
        assert_eq!(corners, [(40, -74), (40, -73)]);
        assert!(blocks.iter().all(|b| b.stride == 1));

        let point = Bbox {
            west: -74.0,
            south: 40.5,
            east: -74.0,
            north: 40.5,
        };
        assert_eq!(blocks_for_view(&mur(), point, 10.0).len(), 1);

        let pole = Bbox {
            west: 179.5,
            south: 89.5,
            east: 200.0,
            north: 95.0,
        };
        let blocks = blocks_for_view(&mur(), pole, 10.0);
        assert_eq!((blocks[0].south, blocks[0].west), (89, 179));
        assert_eq!(blocks.len(), 1);

        let world = Bbox {
            west: -180.0,
            south: -90.0,
            east: 180.0,
            north: 90.0,
        };
        // At zoom 0 a block is 90 degrees: two rows of four.
        assert_eq!(blocks_for_view(&mur(), world, 0.0).len(), 8);
        // At a fine stride it is the whole world in 1 degree blocks.
        assert_eq!(blocks_for_view(&mur(), world, 10.0).len(), 360 * 180);
    }

    #[test]
    fn blocks_grow_with_the_stride_but_never_past_the_cell_limit() {
        // MUR keeps 1 degree blocks while they hold 20 cells a side (stride 5); at stride 6 the
        // block grows to 6 degrees (100 cells a side), at stride 9 to 9 degrees (100 a side).
        assert_eq!(block_side_deg(MUR_STEP, 1), 1);
        assert_eq!(block_side_deg(MUR_STEP, 5), 1);
        assert_eq!(block_side_deg(MUR_STEP, 6), 6);
        assert_eq!(block_side_deg(MUR_STEP, 7), 6);
        assert_eq!(block_side_deg(MUR_STEP, 9), 9);
        assert_eq!(block_side_deg(MUR_STEP, 422), 90);
        assert_eq!(block_side_cells(MUR_STEP, 9), 100);
        assert_eq!(block_side_deg(f64::NAN, 9), 1);
        for step in [MUR_STEP, VIIRS_750M_STEP, GLSEA_STEP, 0.0375, 0.054] {
            for z in 0..=14 {
                let stride = stride_for_zoom(step, f64::from(z));
                let side = block_side_deg(step, stride);
                assert!(BLOCK_SIDES_DEG.contains(&side));
                let cells = block_side_cells(step, stride);
                assert!(cells * cells <= MAX_BLOCK_CELLS, "{step} z{z}: {cells}");
            }
        }
        for side in BLOCK_SIDES_DEG {
            assert_eq!(180 % side, 0);
            assert_eq!(90 % side, 0);
        }
    }

    #[test]
    fn wide_blocks_are_aligned_and_tile_the_view() {
        let view = Bbox {
            west: -82.45,
            south: 27.5,
            east: -65.35,
            north: 54.1,
        };
        let blocks = blocks_for_view(&mur(), view, 5.0);
        assert!(blocks.iter().all(|b| b.stride == 13));
        for b in &blocks {
            assert_eq!(b.south % 10, 0);
            assert_eq!(b.west % 10, 0);
        }
        let area = |b: &Block| b.bounds(MUR_STEP);
        let covers = |lat: f64, lon: f64| {
            blocks
                .iter()
                .filter(|b| {
                    let a = area(b);
                    (a.south..a.north).contains(&lat) && (a.west..a.east).contains(&lon)
                })
                .count()
        };
        // Every point of the view is in exactly one block.
        let mut lat = view.south;
        while lat <= view.north {
            let mut lon = view.west;
            while lon <= view.east {
                assert_eq!(covers(lat, lon), 1, "{lat},{lon}");
                lon += 0.7;
            }
            lat += 0.7;
        }
        // The first block is the one nearest the centre of the view.
        let first = area(&blocks[0]);
        assert!((first.south..first.north).contains(&40.8));
        // A block's request stops one native step short of its far edges.
        let r = blocks[0].request_bbox(MUR_STEP);
        assert!((r.east - (first.east - MUR_STEP)).abs() < 1e-9);
    }

    #[test]
    fn bad_views_have_no_blocks() {
        let flipped = Bbox {
            west: -73.0,
            south: 40.0,
            east: -74.0,
            north: 40.8,
        };
        assert_eq!(blocks_for_view(&mur(), flipped, 8.0), []);
        let nan = Bbox {
            west: f64::NAN,
            ..flipped
        };
        assert_eq!(blocks_for_view(&mur(), nan, 8.0), []);
        let off_the_map = Bbox {
            west: -74.0,
            south: 95.0,
            east: -73.0,
            north: 99.0,
        };
        assert_eq!(blocks_for_view(&mur(), off_the_map, 8.0), []);
    }

    #[test]
    fn block_keys_name_dataset_time_block_and_stride() {
        let block = Block {
            south: 40,
            west: -74,
            stride: 5,
        };
        let key = block.key("jplMURSST41", UnixTime(1_794_042_000));
        assert_eq!(key.to_string(), "jplMURSST41.1794042000.40.-74.5");
        let other_stride = Block { stride: 4, ..block }.key("jplMURSST41", key.time);
        assert_ne!(key, other_stride);
        let json = serde_json::to_string(&key).unwrap();
        assert_eq!(
            json,
            r#"{"dataset":"jplMURSST41","time":1794042000,"block":{"south":40,"west":-74,"stride":5}}"#
        );
        assert_eq!(serde_json::from_str::<BlockKey>(&json).unwrap(), key);
    }

    #[test]
    fn recent_mur_days_expire_after_a_day_and_older_ones_keep() {
        let day = 86_400;
        let t = UnixTime(1_794_042_000);
        let after = |d: f64| UnixTime(t.0 + (d * f64::from(day)) as i64);
        assert!(block_is_fresh("jplMURSST41", t, after(1.0), after(1.9)));
        assert!(!block_is_fresh("jplMURSST41", t, after(1.0), after(2.0)));
        // Fetched late in the window: still revisable, fetched again a day later.
        assert!(!block_is_fresh("jplMURSST41", t, after(6.5), after(7.6)));
        // Fetched once the week is over: keeps.
        assert!(block_is_fresh("jplMURSST41", t, after(7.0), after(400.0)));
        // A clock that went back does not keep a revisable block.
        assert!(!block_is_fresh("jplMURSST41", t, after(2.0), after(1.0)));
        // Other datasets never change a released time.
        assert!(block_is_fresh(
            "noaacwN21VIIRSkd490NSWUSDaily",
            t,
            after(1.0),
            after(30.0)
        ));
    }

    #[test]
    fn sampling_at_a_cell_centre_returns_the_cell() {
        let g = three_by_three();
        for (i, &lat) in g.lats.iter().enumerate() {
            for (j, &lon) in g.lons.iter().enumerate() {
                let want = g.value(i, j).unwrap();
                if want.is_nan() {
                    assert_eq!(g.sample_bilinear(at(lat, lon)), None);
                    assert_eq!(g.sample_nearest(at(lat, lon)), None);
                } else {
                    assert!(close(g.sample_bilinear(at(lat, lon)), want), "{i},{j}");
                    assert_eq!(g.sample_nearest(at(lat, lon)), Some(want));
                }
            }
        }
    }

    #[test]
    fn bilinear_interpolates_and_skips_missing_cells() {
        let g = three_by_three();
        // Between 10, 11, 14 and a missing cell, nearest to 14: the three present cells.
        assert!(close(
            g.sample_bilinear(at(40.06, -73.94)),
            (10.0 * 0.4 * 0.4 + 11.0 * 0.4 * 0.6 + 14.0 * 0.6 * 0.6) / (1.0 - 0.6 * 0.4)
        ));
        // Halfway between 14, 15, 17 and 18.
        assert!(close(g.sample_bilinear(at(40.15, -73.85)), 16.0));
        // Along an edge.
        assert!(close(g.sample_bilinear(at(40.2, -73.95)), 16.5));
        // Nearest to the missing cell: no value.
        assert_eq!(g.sample_bilinear(at(40.09, -73.99)), None);
        // Half a step past the outermost centres the edge cells' values are used; beyond it none.
        assert!(close(g.sample_bilinear(at(40.24, -73.9)), 17.0));
        assert_eq!(g.sample_bilinear(at(40.26, -73.9)), None);
        assert_eq!(g.sample_bilinear(at(40.1, -74.06)), None);
        assert_eq!(g.sample_bilinear(at(f64::NAN, -73.9)), None);
    }

    #[test]
    fn nearest_reaches_half_a_cell_past_the_edges() {
        let g = three_by_three();
        assert_eq!(g.sample_nearest(at(40.04, -73.86)), Some(11.0));
        assert_eq!(g.sample_nearest(at(40.06, -73.86)), Some(14.0));
        assert_eq!(g.sample_nearest(at(40.24, -73.76)), Some(18.0));
        assert_eq!(g.sample_nearest(at(40.26, -73.8)), None);
        assert_eq!(g.sample_nearest(at(40.0, -74.06)), None);

        let one = build(vec![40.0], vec![-74.0], vec![3.0]).unwrap();
        // A lone cell reaches half the native step (0.0375 / 2).
        assert_eq!(one.sample_nearest(at(40.018, -73.982)), Some(3.0));
        assert_eq!(one.sample_nearest(at(40.02, -74.0)), None);
        assert_eq!(one.sample_bilinear(at(40.0, -74.0)), Some(3.0));
    }

    #[test]
    fn range_counts_finite_cells_in_the_box() {
        let g = three_by_three();
        let all = Bbox {
            west: -75.0,
            south: 39.0,
            east: -73.0,
            north: 41.0,
        };
        assert_eq!(g.range_in(all), Some((10.0, 18.0)));
        let west_column = Bbox {
            west: -74.0,
            south: 40.0,
            east: -74.0,
            north: 40.15,
        };
        assert_eq!(g.range_in(west_column), Some((10.0, 10.0)));
        let land_only = Bbox {
            west: -74.01,
            south: 40.09,
            east: -73.99,
            north: 40.11,
        };
        assert_eq!(g.range_in(land_only), None);
        let elsewhere = Bbox {
            west: -70.0,
            south: 30.0,
            east: -69.0,
            north: 31.0,
        };
        assert_eq!(g.range_in(elsewhere), None);
        assert_eq!(g.valid_count(), 8);
    }

    #[test]
    fn neighbouring_blocks_merge_into_one_grid() {
        let west = Grid::new(
            mur(),
            UnixTime(1_794_042_000),
            vec![40.0, 40.5],
            vec![-75.0, -74.5],
            vec![1., 2., 3., 4.],
            "degree_C",
        )
        .unwrap();
        let east = Grid::new(
            mur(),
            UnixTime(1_794_042_000),
            vec![40.0, 40.5],
            vec![-74.0, -73.5],
            vec![5., f32::NAN, 7., 8.],
            "degree_C",
        )
        .unwrap();
        let north = Grid::new(
            mur(),
            UnixTime(1_794_042_000),
            vec![41.0],
            vec![-75.0, -74.5, -74.0, -73.5],
            vec![9., 10., 11., 12.],
            "degree_C",
        )
        .unwrap();
        let merged = Grid::merge(&[east.clone(), west.clone(), north]).unwrap();
        assert_eq!(merged.lats, [40.0, 40.5, 41.0]);
        assert_eq!(merged.lons, [-75.0, -74.5, -74.0, -73.5]);
        let row0 = &merged.values[..4];
        assert_eq!(row0[..3], [1., 2., 5.]);
        assert!(row0[3].is_nan());
        assert_eq!(merged.values[4..], [3., 4., 7., 8., 9., 10., 11., 12.]);
        assert!(close(merged.sample_bilinear(at(40.25, -74.25)), 4.5));

        // An overlapping block fills a hole but a missing value never hides one.
        let patch = Grid::new(
            mur(),
            UnixTime(1_794_042_000),
            vec![40.0],
            vec![-74.0, -73.5],
            vec![f32::NAN, 6.],
            "degree_C",
        )
        .unwrap();
        let merged = Grid::merge(&[east.clone(), patch]).unwrap();
        assert_eq!(merged.values, [5., 6., 7., 8.]);
        assert_eq!(merged.unit, "degree_C");
    }

    #[test]
    fn blocks_of_different_days_or_datasets_do_not_merge() {
        let a = three_by_three();
        let later = Grid {
            time: UnixTime(a.time.0 + 86_400),
            ..a.clone()
        };
        assert_eq!(
            Grid::merge(&[a.clone(), later]),
            Err(MergeError::Mismatch("time"))
        );
        let other = Grid {
            source: source(),
            ..a.clone()
        };
        assert_eq!(
            Grid::merge(&[a.clone(), other]),
            Err(MergeError::Mismatch("dataset"))
        );
        let kelvin = Grid {
            unit: "K".to_owned(),
            ..a.clone()
        };
        assert_eq!(
            Grid::merge(&[a.clone(), kelvin]),
            Err(MergeError::Mismatch("unit"))
        );
        assert_eq!(Grid::merge(&[]), Err(MergeError::Empty));
        let alone = Grid::merge(std::slice::from_ref(&a)).unwrap();
        assert_eq!((&alone.lats, &alone.lons), (&a.lats, &a.lons));
        assert_eq!(alone.valid_count(), 8);
    }

    #[test]
    fn sampling_a_grid_with_broken_fields_does_not_panic() {
        let mut g = three_by_three();
        g.values.truncate(4);
        g.lats = vec![40.2, 40.0, 40.1];
        let p = at(40.1, -73.9);
        let _ = g.sample_bilinear(p);
        let _ = g.sample_nearest(p);
        let _ = g.range_in(Bbox {
            west: -75.0,
            south: 39.0,
            east: -73.0,
            north: 41.0,
        });
        let _ = Grid::merge(&[g]);
    }
}
