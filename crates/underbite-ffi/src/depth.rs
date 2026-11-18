//! Depth across the FFI: elevation grids in the map store, depth shading, contour and break lines, structure points and the depth inspector row.
//!
//! Engine-neutral, like the rest of the map: the app draws what these calls describe. Every word is
//! written by the core's view (contour and break labels in the user's units, "approximate depth"
//! with its datum, "as charted", credits and "Not for navigation"); numbers cross only where the
//! app draws with them.
//!
//! Depths from the NCEI elevation model are always *approximate depth*: metres relative to the
//! source model's vertical datum, never chart datum, never compared with charted soundings or tide
//! heights. Charted depths of structure points (`VALSOU`) are never shown.

use std::collections::BTreeMap;

use std::sync::Arc;

use underbite_config::schema::Config;
use underbite_core::map::breaks::{self, BreakKind, BreakOptions};
use underbite_core::map::catalogue::{FeatureKind, LayerKind};
use underbite_core::map::contours::{self, LabelAnchor, MAX_LEVELS};
use underbite_core::map::dem::{self, DemSourceInfo, SHORE_M, WaterBody};
use underbite_core::map::enc::StructureKind;
use underbite_core::map::grid::Grid;
use underbite_core::map::render::{self, DEPTH, Ramp};
use underbite_core::map::tiles::{MAX_ZOOM, TILE_PX};
use underbite_core::map::{Bbox, GeoPoint, GridHost, Polyline, TileKey};
use underbite_core::time::UnixTime;
use underbite_core::units::HeightUnit;
use underbite_core::view::format::Formatter;
use underbite_core::view::strings::{
    Key, MapQuantity, SafetyKey, break_strength, contour_interval_text, depth_band_text,
    depth_label, reef_depths, structure_category, structure_surveyed, structure_title,
};
use underbite_providers::encdirect::{self, EncBand, EncError};
use underbite_providers::ncei_dem::{self, DemCrs, DemError};
use underbite_providers::request::{Coordinates, RoundedPoint};
use underbite_providers::statemaps::{self, ContourState, StateMapError};

use crate::fetch::{HostRequest, HostResponse, fetched};
use crate::map::{
    DemId, GridRequest, GridStore, LegendStop, MapColour, MapError, MapPoint, formatter, get,
    invalid, layer, node_name, tile_key,
};
use crate::settings::{DisplayStop, load};

// ---------------------------------------------------------------------------------------------
// Limits
// ---------------------------------------------------------------------------------------------

/// The id the depth inspector row, shading and legend go by: the elevation model is one source
/// under the store, not a catalogue layer of its own.
pub const DEPTH_LAYER: &str = "depth-dem";

/// How long the app may reuse an elevation response, seconds: the mosaic changes with releases,
/// not days.
const DEM_RESPONSE_AGE_S: u64 = 30 * 86_400;

/// How long the app may reuse the GEBCO capabilities, seconds.
const CAPABILITIES_AGE_S: u64 = 30 * 86_400;

/// The widest area, in degrees, one elevation request covers.
pub const MAX_AREA_SPAN_DEG: f64 = 5.0;

/// The most pixels along the longer side of an area request: about 260,000 values, an eighth of
/// the store.
pub const MAX_AREA_PX: f64 = 512.0;

/// The most contour levels one call draws; a finer interval is doubled until it fits.
const MAX_CONTOUR_LEVELS: usize = 120;

/// The most vertices one [`GridStore::contour_lines`] call returns.
pub const MAX_CONTOUR_VERTICES: usize = 60_000;

/// The most vertices one [`GridStore::break_lines`] call returns.
pub const MAX_BREAK_VERTICES: usize = 40_000;

/// The coarsest stride of the blocks breaks are found on: a strided grid weakens the gradient, so
/// the view asks for blocks at (nearly) the grid's own resolution and wider ones draw no breaks.
pub const MAX_BREAK_STRIDE: f64 = 4.5;

/// Screen pixels a contour line must run to carry a label.
const LABEL_MIN_PX: f64 = 220.0;

/// The metres one screen pixel covers at the equator at zoom 0.
const EQUATOR_M_PER_PX: f64 = 156_543.033_928;

/// The deepest depth the band's stops reach, metres: the depth ramp's own range.
const BAND_MAX_M: f64 = 100.0;

/// The spacing of the lines the breaks are cut from, per kind: °C for temperature, log10 units
/// for chlorophyll.
const SST_SPACING: f64 = 0.5;
const CHLOROPHYLL_SPACING: f64 = 0.1;

// ---------------------------------------------------------------------------------------------
// Elevation blocks in the store
// ---------------------------------------------------------------------------------------------

/// Degrees as the key holds them: millionths.
fn micro(deg: f64) -> i64 {
    (deg * 1e6).round() as i64
}

const fn from_micro(m: i64) -> f64 {
    m as f64 / 1e6
}

impl DemId {
    /// `dem/tile/z/x/y/px` or `dem/area/west/south/east/north/cell`, degrees in millionths.
    fn key(&self) -> String {
        match *self {
            Self::Tile { z, x, y, px } => format!("dem/tile/{z}/{x}/{y}/{px}"),
            Self::Area {
                west,
                south,
                east,
                north,
                cell,
            } => format!("dem/area/{west}/{south}/{east}/{north}/{cell}"),
        }
    }

    fn parse(key: &str) -> Result<Self, MapError> {
        let bad = || invalid(format!("`{key}` is not an elevation key"));
        let parts: Vec<&str> = key.split('/').collect();
        let number = |s: &str| s.parse::<i64>().map_err(|_| bad());
        match parts.as_slice() {
            ["dem", "tile", z, x, y, px] => {
                let id = Self::Tile {
                    z: z.parse().map_err(|_| bad())?,
                    x: x.parse().map_err(|_| bad())?,
                    y: y.parse().map_err(|_| bad())?,
                    px: px.parse().map_err(|_| bad())?,
                };
                if let Self::Tile { z, x, y, px } = id {
                    tile_key(z, x, y)?;
                    if px == 0 || px > ncei_dem::MAX_IMAGE_PX {
                        return Err(bad());
                    }
                }
                Ok(id)
            }
            ["dem", "area", west, south, east, north, cell] => {
                let id = Self::Area {
                    west: number(west)?,
                    south: number(south)?,
                    east: number(east)?,
                    north: number(north)?,
                    cell: number(cell)?,
                };
                match id {
                    Self::Area { cell, .. } if cell <= 0 => Err(bad()),
                    _ => Ok(id),
                }
            }
            _ => Err(bad()),
        }
    }

    const fn crs(&self) -> DemCrs {
        match self {
            Self::Tile { .. } => DemCrs::WebMercator,
            Self::Area { .. } => DemCrs::Wgs84,
        }
    }
}

/// A box of four edges, checked and clamped to the world.
fn view_box(west: f64, south: f64, east: f64, north: f64) -> Result<Bbox, MapError> {
    if ![west, south, east, north].iter().all(|v| v.is_finite()) || west >= east || south >= north {
        return Err(invalid("a view needs west < east and south < north"));
    }
    Ok(Bbox {
        west: west.clamp(-180.0, 180.0),
        south: south.clamp(-85.0, 85.0),
        east: east.clamp(-180.0, 180.0),
        north: north.clamp(-85.0, 85.0),
    })
}

/// A busy or failing service the app may ask again later, else what was wrong.
fn dem_error(e: &DemError) -> MapError {
    let busy = |status: i64| matches!(status, 429 | 502 | 503 | 504);
    match e {
        DemError::Status { status } if busy(i64::from(*status)) => {
            MapError::RetryLater { after_s: 60 }
        }
        DemError::Service {
            code: Some(code), ..
        } if busy(*code) => MapError::RetryLater { after_s: 60 },
        other => invalid(other.to_string()),
    }
}

fn point_of(p: GeoPoint) -> MapPoint {
    MapPoint {
        lat: p.lat,
        lon: p.lon,
    }
}

fn points_of(line: &Polyline) -> Vec<MapPoint> {
    line.points.iter().copied().map(point_of).collect()
}

/// The metres one screen pixel covers at `lat` at web zoom `zoom`.
fn metres_per_pixel(zoom: f64, lat: f64) -> f64 {
    (EQUATOR_M_PER_PX * lat.to_radians().cos().abs() / 2f64.powf(zoom)).max(0.01)
}

#[uniffi::export]
impl GridStore {
    /// The request for the elevation of Web Mercator tile `z`/`x`/`y`, 128 pixels a side, for
    /// shading ([`GridStore::render_depth_tile`]) and the inspector; none when the store holds it
    /// already (the elevation model does not change by the day). Hand the response to
    /// [`GridStore::add_dem`] under the key. The service states no rate limit: keep to a handful
    /// of tiles per view.
    pub fn dem_tile_request(&self, z: u8, x: u32, y: u32) -> Result<Option<GridRequest>, MapError> {
        let key = tile_key(z, x, y)?;
        let px = ncei_dem::DEFAULT_TILE_PX;
        let id = DemId::Tile { z, x, y, px };
        if self.lock().has_dem(&id) {
            return Ok(None);
        }
        let request = ncei_dem::tile_request(key, px).map_err(|e| dem_error(&e))?;
        Ok(Some(GridRequest {
            key: id.key(),
            node: node_name(GridHost::NceiDem),
            request: get(request.url, DEM_RESPONSE_AGE_S),
        }))
    }

    /// The request for the elevation of a view (`west`, `south`, `east`, `north` in degrees at
    /// `zoom`), for contours and the inspector: one image with cells of about a screen pixel, at
    /// most 512 pixels a side and a view of at most five degrees. None when the store holds the
    /// area already. The box is kept to millionths of a degree so that the same view asks for
    /// the same area.
    pub fn dem_area_request(
        &self,
        west: f64,
        south: f64,
        east: f64,
        north: f64,
        zoom: f64,
    ) -> Result<Option<GridRequest>, MapError> {
        let area = view_box(west, south, east, north)?;
        if area.east - area.west > MAX_AREA_SPAN_DEG || area.north - area.south > MAX_AREA_SPAN_DEG
        {
            return Err(invalid("the view is too wide for contours: zoom in"));
        }
        if !zoom.is_finite() {
            return Err(invalid("the zoom is not a number"));
        }
        let pixel_deg =
            360.0 / f64::from(TILE_PX) / 2f64.powf(zoom.clamp(0.0, f64::from(MAX_ZOOM)));
        let longest = (area.east - area.west).max(area.north - area.south);
        let cell = pixel_deg.max(longest / MAX_AREA_PX);
        let id = DemId::Area {
            west: micro(area.west),
            south: micro(area.south),
            east: micro(area.east),
            north: micro(area.north),
            cell: micro(cell).max(1),
        };
        if self.lock().has_dem(&id) {
            return Ok(None);
        }
        let DemId::Area {
            west,
            south,
            east,
            north,
            cell,
        } = id
        else {
            return Err(invalid("not an area"));
        };
        let request = ncei_dem::area_request(
            Bbox {
                west: from_micro(west),
                south: from_micro(south),
                east: from_micro(east),
                north: from_micro(north),
            },
            from_micro(cell),
        )
        .map_err(|e| dem_error(&e))?;
        Ok(Some(GridRequest {
            key: id.key(),
            node: node_name(GridHost::NceiDem),
            request: get(request.url, DEM_RESPONSE_AGE_S),
        }))
    }

    /// Stores the response to a request from [`GridStore::dem_tile_request`] or
    /// [`GridStore::dem_area_request`] under its `key`, fetched at `now`. The service answers
    /// failures with a JSON error object, often with status 200: those come back as errors, a
    /// busy service as [`MapError::RetryLater`].
    #[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
    pub fn add_dem(&self, key: String, response: HostResponse, now: i64) -> Result<(), MapError> {
        let id = DemId::parse(&key)?;
        let fetched = fetched(response).map_err(|e| invalid(e.to_string()))?;
        let response = fetched.as_response();
        let body = ncei_dem::check_response(&response).map_err(|e| dem_error(&e))?;
        let image = ncei_dem::parse_export(body, id.crs()).map_err(|e| dem_error(&e))?;
        let grid = image.into_grid(UnixTime(now)).map_err(|e| dem_error(&e))?;
        self.lock().insert_dem(id, grid, UnixTime(now))
    }

    /// The request for the elevation models that cover a point, whose vertical datum the depth
    /// inspector row names. The point is rounded to the settings' coordinate precision before it
    /// is sent.
    #[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
    pub fn dem_sources_request(
        &self,
        lat: f64,
        lon: f64,
        settings_json: String,
    ) -> Result<HostRequest, MapError> {
        let at = checked_point(lat, lon)?;
        let (config, _) = load(&settings_json)?;
        let rounded = RoundedPoint::new(
            Coordinates {
                latitude: at.lat,
                longitude: at.lon,
            },
            config.sources.coordinate_precision,
        );
        Ok(get(
            ncei_dem::query_request(rounded).url,
            DEM_RESPONSE_AGE_S,
        ))
    }

    /// Remembers the models in the response to [`GridStore::dem_sources_request`] for the place
    /// `lat`, `lon`: the inspector uses the finest one's datum for points within about two
    /// kilometres. Without a response the row says "datum not stated".
    #[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
    pub fn add_dem_sources(
        &self,
        lat: f64,
        lon: f64,
        response: HostResponse,
    ) -> Result<(), MapError> {
        let at = checked_point(lat, lon)?;
        let fetched = fetched(response).map_err(|e| invalid(e.to_string()))?;
        let response = fetched.as_response();
        let body = ncei_dem::check_response(&response).map_err(|e| dem_error(&e))?;
        let sources: Vec<DemSourceInfo> = ncei_dem::parse_query(body)
            .map_err(|e| dem_error(&e))?
            .iter()
            .map(DemSourceInfo::from)
            .collect();
        self.lock().set_dem_sources(at, sources);
        Ok(())
    }

    /// A 256-pixel PNG of the depth shading of tile `z`/`x`/`y` from the elevation block the store
    /// holds: water coloured by depth with the settings' depth ramp, the depth band highlighted,
    /// land and shore transparent; none when the store has no elevation tile there. Depths are
    /// approximate: say so wherever they are read out.
    #[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
    pub fn render_depth_tile(
        &self,
        z: u8,
        x: u32,
        y: u32,
        settings_json: String,
    ) -> Result<Option<Vec<u8>>, MapError> {
        let key = tile_key(z, x, y)?;
        let (config, _) = load(&settings_json)?;
        let wanted = tile_bbox_of(key);
        // The tile's own block when held, else the block covering most of it (an area fetched
        // for contours shades too: one fetch serves both).
        let grid = {
            let blocks = self.lock().dem_blocks(Some(wanted));
            let exact = blocks.iter().find(|(id, _)| {
                matches!(id, DemId::Tile { z: bz, x: bx, y: by, .. } if (*bz, *bx, *by) == (z, x, y))
            });
            exact
                .or_else(|| {
                    blocks.iter().max_by(|(_, a), (_, b)| {
                        coverage(a, wanted).total_cmp(&coverage(b, wanted))
                    })
                })
                .map(|(_, grid)| Arc::clone(grid))
        };
        let Some(grid) = grid else {
            return Ok(None);
        };
        dem::render_depth_tile(
            &grid,
            WaterBody::Sea,
            config.map.depth.band_m,
            config.map.depth.ramp.as_deref(),
            key,
            TILE_PX,
        )
        .map(Some)
        .map_err(|e| invalid(e.to_string()))
    }

    /// Depth contours of the elevation the store holds for a view (`west`, `south`, `east`,
    /// `north` in degrees at `zoom`), at the settings' contour interval, labelled in the user's
    /// depth unit; ask the store for the area first ([`GridStore::dem_area_request`]). Lines are
    /// thinned to about half a screen pixel and at most [`MAX_CONTOUR_VERTICES`] vertices come
    /// back, shallow to deep: ask per visible area. Where the interval would give more than 120
    /// levels it is doubled until it does not. Empty where the store holds nothing for the view.
    #[allow(clippy::too_many_arguments)] // a view is four edges and a zoom
    #[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
    pub fn contour_lines(
        &self,
        west: f64,
        south: f64,
        east: f64,
        north: f64,
        zoom: f64,
        settings_json: String,
    ) -> Result<Vec<ContourLine>, MapError> {
        let area = view_box(west, south, east, north)?;
        if !zoom.is_finite() {
            return Err(invalid("the zoom is not a number"));
        }
        let (config, _) = load(&settings_json)?;
        let interval = f64::from(config.map.depth.contour_interval_m);
        if !(interval.is_finite() && interval > 0.0) {
            return Err(invalid("the contour interval is not a positive number"));
        }
        // The lock is held only to find the grid; contouring works on a shared copy.
        let best = self
            .lock()
            .dem_blocks(Some(area))
            .into_iter()
            .map(|(_, grid)| grid)
            .max_by(|a, b| coverage(a, area).total_cmp(&coverage(b, area)));
        let Some(grid) = best else {
            return Ok(Vec::new());
        };
        let f = formatter(&config, "UTC");
        Ok(contour_lines_of(&grid, area, zoom, interval, &f))
    }

    /// The breaks of a satellite layer (`sst` or `chlorophyll`) in a view: stretches of lines of
    /// equal value where the change per kilometre is at least the settings' threshold, each with
    /// its strength in the user's units ("1.4 °F per mi"). Found on the blocks the store holds for
    /// the layer's view ([`GridStore::grid_requests`]), joined, at the grid's own resolution:
    /// blocks coarser than a stride of 4 draw nothing, so ask for the blocks at zoom 8 or deeper.
    /// Thinned, at most [`MAX_BREAK_VERTICES`] vertices, strongest first.
    #[allow(clippy::too_many_arguments)] // a view is four edges
    #[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
    pub fn break_lines(
        &self,
        layer: String,
        west: f64,
        south: f64,
        east: f64,
        north: f64,
        settings_json: String,
    ) -> Result<Vec<BreakLine>, MapError> {
        let l = self::layer(&layer)?;
        let kind = match MapQuantity::of_layer(l.id) {
            Some(MapQuantity::Temperature) if l.grid().is_some() => BreakKind::Sst,
            Some(MapQuantity::Chlorophyll) if l.grid().is_some() => BreakKind::Chlorophyll,
            _ => return Err(invalid(format!("`{layer}` has no breaks"))),
        };
        let area = view_box(west, south, east, north)?;
        let (config, _) = load(&settings_json)?;
        let shared = self.lock().selected(l.id, Some(area));
        let grids: Vec<Grid> = shared
            .iter()
            .filter_map(|g| g.first().map(|g| Grid::clone(g)))
            .collect();
        let f = formatter(&config, "UTC");
        break_lines_of(&grids, kind, area, &config, &f)
    }
}

/// The area of `grid` inside `area`, square degrees, for choosing the block that fills a view.
fn coverage(grid: &Grid, area: Bbox) -> f64 {
    let Some(e) = crate::map::extent(grid) else {
        return 0.0;
    };
    let width = (e.east.min(area.east) - e.west.max(area.west)).max(0.0);
    let height = (e.north.min(area.north) - e.south.max(area.south)).max(0.0);
    width * height
}

fn tile_bbox_of(key: TileKey) -> Bbox {
    underbite_core::map::tiles::tile_bbox(key)
}

fn checked_point(lat: f64, lon: f64) -> Result<GeoPoint, MapError> {
    if lat.is_finite()
        && lon.is_finite()
        && (-90.0..=90.0).contains(&lat)
        && (-180.0..=180.0).contains(&lon)
    {
        Ok(GeoPoint { lat, lon })
    } else {
        Err(invalid("a point needs a latitude and a longitude in range"))
    }
}

// ---------------------------------------------------------------------------------------------
// Contours
// ---------------------------------------------------------------------------------------------

/// Where to write a line's label and how to turn it.
#[derive(Clone, Copy, Debug, PartialEq, uniffi::Record)]
pub struct LabelAt {
    pub lat: f64,
    pub lon: f64,
    /// The direction the text runs, degrees clockwise from true north, 0 to 180 so that it reads
    /// from west to east (90 is level text).
    pub angle_deg: f64,
}

impl From<&LabelAnchor> for LabelAt {
    fn from(a: &LabelAnchor) -> Self {
        Self {
            lat: a.at.lat,
            lon: a.at.lon,
            angle_deg: a.angle_deg,
        }
    }
}

/// A depth contour line to draw.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct ContourLine {
    pub points: Vec<MapPoint>,
    /// The depth in the user's unit: "30 ft", "10 m".
    pub label: String,
    /// Where to write the label; none on a line too short to carry it.
    pub label_at: Option<LabelAt>,
    /// Every fifth level (of the interval used) is major: draw it heavier.
    pub major: bool,
}

/// The levels, in metres positive up, of the depth contours of a grid whose values run from
/// `low` to `high`: multiples of `interval` below the shore, the interval doubled until at most
/// [`MAX_CONTOUR_LEVELS`] fit. Returns the levels and the interval used.
fn contour_levels(low: f64, high: f64, interval: f64) -> (Vec<f64>, f64) {
    let top = high.min(-f64::from(SHORE_M));
    if !(low.is_finite() && top.is_finite()) || low > top {
        return (Vec::new(), interval);
    }
    let mut step = interval;
    // A step that doubles past the range ends with one level or none.
    while (top - low) / step + 1.0 > MAX_CONTOUR_LEVELS.min(MAX_LEVELS) as f64 {
        step *= 2.0;
    }
    (contours::levels(low, top, step), step)
}

fn contour_lines_of(
    grid: &Grid,
    area: Bbox,
    zoom: f64,
    interval: f64,
    f: &Formatter,
) -> Vec<ContourLine> {
    let Some((low, high)) = grid.range_in(area) else {
        return Vec::new();
    };
    let (levels, step) = contour_levels(f64::from(low), f64::from(high), interval);
    if levels.is_empty() {
        return Vec::new();
    }
    let lat = f64::midpoint(area.north, area.south);
    let pixel_m = metres_per_pixel(zoom.clamp(0.0, f64::from(MAX_ZOOM)), lat);
    // No lock is held here: contouring and thinning are the expensive part.
    let mut lines = Vec::new();
    let mut vertices = 0;
    for line in contours::contours(grid, &levels) {
        let line = contours::simplify(&line, pixel_m / 2.0);
        if line.points.len() < 2 {
            continue;
        }
        if vertices + line.points.len() > MAX_CONTOUR_VERTICES {
            break;
        }
        vertices += line.points.len();
        lines.push(line);
    }
    let anchors: BTreeMap<usize, LabelAnchor> =
        contours::label_anchors(&lines, LABEL_MIN_PX * pixel_m)
            .into_iter()
            .map(|a| (a.line, a))
            .collect();
    lines
        .iter()
        .enumerate()
        .map(|(i, line)| ContourLine {
            points: points_of(line),
            label: depth_label(f, -line.value),
            label_at: anchors.get(&i).map(LabelAt::from),
            major: (line.value / step).round().rem_euclid(5.0) == 0.0,
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Breaks
// ---------------------------------------------------------------------------------------------

/// A break to draw: a stretch of a line where the change is sharp.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct BreakLine {
    pub points: Vec<MapPoint>,
    /// The strength in the user's units: "1.4 °F per mi", "×1.1 per km".
    pub strength_text: String,
}

/// The stride a block was fetched at, from its spacing against the dataset's own.
fn stride_of(grid: &Grid) -> f64 {
    let step = match grid.lons.as_slice() {
        [a, b, ..] => (b - a).abs(),
        _ => return 1.0,
    };
    let native = grid.source.native_step_deg.abs();
    if native > 0.0 { step / native } else { 1.0 }
}

/// The span of the field a break kind is taken of, for choosing the spacing.
fn field_span(grid: &Grid, kind: BreakKind) -> Option<f64> {
    let (mut low, mut high) = (f64::MAX, f64::MIN);
    for &v in &grid.values {
        let v = f64::from(v);
        let v = match kind {
            BreakKind::Sst if v.is_finite() => v,
            BreakKind::Chlorophyll if v.is_finite() && v > 0.0 => v.log10(),
            _ => continue,
        };
        low = low.min(v);
        high = high.max(v);
    }
    (low <= high).then_some(high - low)
}

fn break_lines_of(
    grids: &[Grid],
    kind: BreakKind,
    area: Bbox,
    config: &Config,
    f: &Formatter,
) -> Result<Vec<BreakLine>, MapError> {
    let Some(finest) = grids.iter().map(stride_of).min_by(f64::total_cmp) else {
        return Ok(Vec::new());
    };
    if finest > MAX_BREAK_STRIDE {
        return Ok(Vec::new());
    }
    let same: Vec<Grid> = grids
        .iter()
        .filter(|g| (stride_of(g) - finest).abs() < 0.5)
        .cloned()
        .collect();
    let grid = Grid::merge(&same).map_err(|e| invalid(e.to_string()))?;
    let Some(span) = field_span(&grid, kind) else {
        return Ok(Vec::new());
    };
    let (threshold, default_spacing) = match kind {
        BreakKind::Sst => (config.map.breaks.sst_c_per_km, SST_SPACING),
        BreakKind::Chlorophyll => (
            config.map.breaks.chlorophyll_log10_per_km,
            CHLOROPHYLL_SPACING,
        ),
    };
    // Few enough levels for `contours::levels`, whatever the field's range.
    let spacing = default_spacing.max(span / (MAX_LEVELS as f64 / 2.0));
    let options = BreakOptions::new(kind, f64::from(threshold), spacing);
    let cell_m = grid
        .lats
        .windows(2)
        .next()
        .map_or(1_000.0, |w| (w[1] - w[0]).abs() * 111_000.0);
    let mut pieces: Vec<Polyline> = breaks::breaks_with(&grid, kind, &options)
        .into_iter()
        .filter(|p| {
            p.points.iter().any(|q| {
                (area.west..=area.east).contains(&q.lon)
                    && (area.south..=area.north).contains(&q.lat)
            })
        })
        .map(|p| contours::simplify(&p, cell_m / 2.0))
        .filter(|p| p.points.len() >= 2)
        .collect();
    pieces.sort_by(|a, b| b.value.total_cmp(&a.value));
    let mut vertices = 0;
    let mut out = Vec::new();
    for piece in pieces {
        if vertices + piece.points.len() > MAX_BREAK_VERTICES {
            break;
        }
        vertices += piece.points.len();
        out.push(BreakLine {
            points: points_of(&piece),
            strength_text: break_strength(f, kind, piece.value),
        });
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// Depth layer: words, legend and the band's stops
// ---------------------------------------------------------------------------------------------

/// The depth shading as the drawer and the legend show it.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct DepthLayerInfo {
    /// [`DEPTH_LAYER`], the id the inspector row carries.
    pub id: String,
    pub title: String,
    /// The credit line to show with the layer (safety text).
    pub credit: String,
    /// "Not for navigation." (safety text).
    pub notice: String,
    /// What the depths are measured from (safety text).
    pub depth_reference: String,
    pub legend_caption: String,
    /// The colour ramp the settings give it, else the layer's own.
    pub ramp: String,
    /// The legend bar, shallow to deep, with depths in the user's unit.
    pub legend: Vec<LegendStop>,
}

/// The ramp the depth shading is drawn with (the settings' depth ramp, else the layer's own) and
/// its legend bar over the ramp's own range, shallow to deep, labelled in the user's depth unit.
pub(crate) fn depth_legend(config: &Config, f: &Formatter) -> (&'static Ramp, Vec<LegendStop>) {
    let ramp = config
        .map
        .depth
        .ramp
        .as_deref()
        .and_then(render::ramp)
        .unwrap_or(&DEPTH);
    let legend = render::legend(ramp, ramp.default_range)
        .into_iter()
        .map(|stop| LegendStop {
            fraction: stop.fraction,
            colour: MapColour::from(stop.colour),
            label: depth_label(f, stop.value),
        })
        .collect();
    (ramp, legend)
}

/// The depth shading's words, credit, notice and legend for the settings' ramp and units.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn depth_layer_info(settings_json: String) -> Result<DepthLayerInfo, MapError> {
    let (config, _) = load(&settings_json)?;
    let f = formatter(&config, "UTC");
    let s = f.strings;
    let (ramp, legend) = depth_legend(&config, &f);
    Ok(DepthLayerInfo {
        id: DEPTH_LAYER.to_owned(),
        title: s.get(Key::MapDepthTitle).to_owned(),
        credit: s.safety(SafetyKey::MapCreditBathymetry).to_owned(),
        notice: s.safety(SafetyKey::NotForNavigation).to_owned(),
        depth_reference: s.safety(SafetyKey::MapReferenceBathymetry).to_owned(),
        legend_caption: s.safety(SafetyKey::MapLegendBathymetry).to_owned(),
        ramp: ramp.id.to_owned(),
        legend,
    })
}

/// The words of the depth layers' controls and states, in the settings' voice and units.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct DepthWords {
    /// The shading's title: "Water depth".
    pub shading_title: String,
    /// The contours' title: "Depth contours".
    pub contours_title: String,
    /// The settings' contour interval under the contours' switch: "Every 10 ft". Where a view
    /// would hold too many lines the drawn interval is a multiple of it.
    pub contours_detail: String,
    /// The depth band control's title: "Highlight".
    pub band_title: String,
    /// The words before the shallow and the deep end: "from", "to".
    pub band_from: String,
    pub band_to: String,
    /// The control without a band: "Off".
    pub band_off: String,
    /// The ends and the clear button as a screen reader names them: "Highlight from",
    /// "Highlight to", "Clear the highlight".
    pub band_from_label: String,
    pub band_to_label: String,
    pub band_clear_label: String,
    /// The settings' band in the user's unit, shallow first: "15–25 ft"; "Off" without one.
    pub band_text: String,
    /// In place of depth when the view is wider than one elevation area
    /// ([`GridStore::dem_area_request`] refuses it): "Zoom in to see depth".
    pub zoom_in: String,
    /// While the elevation for the view is fetched.
    pub loading: String,
    /// When it could not be fetched.
    pub failed: String,
    /// "Not for navigation." (safety text).
    pub notice: String,
}

/// The words of the depth layers' controls (the band's labels, the contour interval) and of
/// their states (too wide a view, loading, failed), in the settings' voice and units.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn depth_words(settings_json: String) -> Result<DepthWords, MapError> {
    let (config, _) = load(&settings_json)?;
    let f = formatter(&config, "UTC");
    let s = f.strings;
    let depth = &config.map.depth;
    let band = depth
        .band_m
        .map(|(shallow, deep)| (f64::from(shallow), f64::from(deep)));
    Ok(DepthWords {
        shading_title: s
            .layer_title("depth-shading")
            .unwrap_or_default()
            .to_owned(),
        contours_title: s
            .layer_title("depth-contours")
            .unwrap_or_default()
            .to_owned(),
        contours_detail: contour_interval_text(&f, f64::from(depth.contour_interval_m)),
        band_title: s.get(Key::MapDepthHighlight).to_owned(),
        band_from: s.get(Key::MapDepthFrom).to_owned(),
        band_to: s.get(Key::MapDepthTo).to_owned(),
        band_off: s.get(Key::MapDepthOff).to_owned(),
        band_from_label: s.get(Key::MapDepthHighlightFrom).to_owned(),
        band_to_label: s.get(Key::MapDepthHighlightTo).to_owned(),
        band_clear_label: s.get(Key::MapDepthHighlightClear).to_owned(),
        band_text: depth_band_text(&f, band),
        zoom_in: s.get(Key::MapDepthZoomIn).to_owned(),
        loading: s.get(Key::MapDepthLoading).to_owned(),
        failed: s.get(Key::MapDepthFailed).to_owned(),
        notice: s.safety(SafetyKey::NotForNavigation).to_owned(),
    })
}

/// The depth band's stops in feet, shallow to deep: five-foot steps to 30 ft, where a band is
/// usually set ("15–25 ft"), then coarser, within the depth ramp's 100 m (328 ft).
const BAND_STOPS_FT: [f32; 15] = [
    0.0, 5.0, 10.0, 15.0, 20.0, 25.0, 30.0, 40.0, 50.0, 75.0, 100.0, 150.0, 200.0, 250.0, 300.0,
];

/// The depth band's stops in metres, shallow to deep: one-metre steps to 6 m, then coarser, to
/// the depth ramp's 100 m.
const BAND_STOPS_M: [f32; 17] = [
    0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 8.0, 10.0, 15.0, 20.0, 25.0, 30.0, 40.0, 50.0, 75.0, 100.0,
];

/// Round stops for the depth band's slider, from the surface to 100 m, in the user's depth unit,
/// shallow to deep: fine where a band is usually set (every 5 ft to 30 ft, every metre to 6 m),
/// coarser below. Each has the value to store (canonical metres) and its label ("15 ft").
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn depth_band_stops(settings_json: String) -> Result<Vec<DisplayStop>, MapError> {
    let (config, _) = load(&settings_json)?;
    let f = formatter(&config, "UTC");
    let unit = f.units.height;
    let stops: &[f32] = match unit {
        HeightUnit::Ft => &BAND_STOPS_FT,
        HeightUnit::M => &BAND_STOPS_M,
    };
    Ok(stops
        .iter()
        .map(|&v| f64::from(unit.to_canonical(v)))
        .filter(|&m| m <= BAND_MAX_M + 1e-3)
        .map(|value| DisplayStop {
            value,
            label: depth_label(&f, value),
        })
        .collect())
}

// ---------------------------------------------------------------------------------------------
// GEBCO
// ---------------------------------------------------------------------------------------------

/// The GEBCO WMS capabilities, which state the grid's year and citation.
#[uniffi::export]
pub fn gebco_capabilities_request() -> HostRequest {
    get(
        "https://wms.gebco.net/mapserv?request=GetCapabilities&service=WMS&version=1.3.0"
            .to_owned(),
        CAPABILITIES_AGE_S,
    )
}

/// The credit for the GEBCO layer from the capabilities response ("Imagery reproduced from the
/// `GEBCO_2026` Grid, … Not for navigation."), so that the year follows each release. A response
/// that is not usable gives the layer's own credit (safety text).
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn gebco_credit(response: HostResponse, settings_json: String) -> Result<String, MapError> {
    let (config, _) = load(&settings_json)?;
    let f = formatter(&config, "UTC");
    let from_service = (response.status / 100 == 2)
        .then(|| String::from_utf8_lossy(&response.body).into_owned())
        .and_then(|xml| dem::gebco_credit(&xml));
    Ok(from_service.unwrap_or_else(|| f.strings.safety(SafetyKey::MapCreditGebco).to_owned()))
}

// ---------------------------------------------------------------------------------------------
// Structure points (ENC Direct)
// ---------------------------------------------------------------------------------------------

/// A chart scale band of ENC Direct.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum StructureBand {
    /// 1:150,001 to 1:600,000 (zoom 9 and 10).
    Coastal,
    /// 1:50,001 to 1:150,000 (zoom 11 and 12).
    Approach,
    /// 1:5,000 to 1:50,000 (zoom 13 and deeper).
    Harbour,
}

impl From<StructureBand> for EncBand {
    fn from(band: StructureBand) -> Self {
        match band {
            StructureBand::Coastal => Self::Coastal,
            StructureBand::Approach => Self::Approach,
            StructureBand::Harbour => Self::Harbour,
        }
    }
}

impl From<EncBand> for StructureBand {
    fn from(band: EncBand) -> Self {
        match band {
            EncBand::Coastal => Self::Coastal,
            EncBand::Approach => Self::Approach,
            EncBand::Harbour => Self::Harbour,
        }
    }
}

/// What a charted structure point is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum StructureType {
    Wreck,
    /// A snag, a foul area, a fish haven and the like; the category says which.
    Obstruction,
    /// An underwater or awash rock.
    Rock,
}

impl From<StructureType> for StructureKind {
    fn from(kind: StructureType) -> Self {
        match kind {
            StructureType::Wreck => Self::Wreck,
            StructureType::Obstruction => Self::Obstruction,
            StructureType::Rock => Self::Rock,
        }
    }
}

impl From<StructureKind> for StructureType {
    fn from(kind: StructureKind) -> Self {
        match kind {
            StructureKind::Wreck => Self::Wreck,
            StructureKind::Obstruction => Self::Obstruction,
            StructureKind::Rock => Self::Rock,
        }
    }
}

/// The scale band that suits a map zoom; none above zoom 9, where the points are not asked for.
#[uniffi::export]
pub fn structure_band(zoom: u8) -> Option<StructureBand> {
    EncBand::for_zoom(zoom).map(StructureBand::from)
}

/// The request for a band's layer list. The ids of the point layers differ per band and are never
/// written down: ask, then read them with [`parse_structure_layers`]. The answer holds for a day.
#[uniffi::export]
pub fn structure_layers_request(band: StructureBand) -> HostRequest {
    get(
        encdirect::layers_request(band.into()).url,
        encdirect::LAYER_LIST_TTL_S,
    )
}

/// The ids of a band's point layers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Record)]
pub struct StructureLayers {
    pub wrecks: Option<u32>,
    pub obstructions: Option<u32>,
    pub rocks: Option<u32>,
}

impl StructureLayers {
    /// The layer of a kind, for [`structure_points_request`].
    fn of(&self, kind: StructureType) -> Option<u32> {
        match kind {
            StructureType::Wreck => self.wrecks,
            StructureType::Obstruction => self.obstructions,
            StructureType::Rock => self.rocks,
        }
    }
}

/// An ENC Direct failure as the app handles it: a server hiccup (the service answers about one
/// request in four with a 200 and an error body) or a busy one is [`MapError::RetryLater`] for
/// the first attempts, else it is final. `attempt` counts the requests already made for this
/// thing (0 after the first response).
fn enc_error(e: &EncError, attempt: u32) -> MapError {
    encdirect::retry_after(e, attempt).map_or_else(
        || invalid(e.to_string()),
        |after_s| MapError::RetryLater { after_s },
    )
}

/// Reads a band's layer list into the ids of its wreck, obstruction and rock layers.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn parse_structure_layers(
    response: HostResponse,
    attempt: u32,
) -> Result<StructureLayers, MapError> {
    let fetched = fetched(response).map_err(|e| invalid(e.to_string()))?;
    let layers =
        encdirect::parse_layers(&fetched.as_response()).map_err(|e| enc_error(&e, attempt))?;
    Ok(StructureLayers {
        wrecks: layers.id(StructureKind::Wreck),
        obstructions: layers.id(StructureKind::Obstruction),
        rocks: layers.id(StructureKind::Rock),
    })
}

/// The request for one kind of structure point inside a box, one page of 1,000; `offset` is 0
/// for the first page, then the number of points already read while the page said there is more.
/// `layers` are the band's ids from [`parse_structure_layers`]; none where the band has no layer
/// of the kind.
#[uniffi::export]
#[allow(clippy::too_many_arguments)] // a box is four edges
pub fn structure_points_request(
    band: StructureBand,
    layers: StructureLayers,
    kind: StructureType,
    west: f64,
    south: f64,
    east: f64,
    north: f64,
    offset: u32,
) -> Result<Option<HostRequest>, MapError> {
    let Some(layer) = layers.of(kind) else {
        return Ok(None);
    };
    let area = view_box(west, south, east, north)?;
    let request = encdirect::points_request(band.into(), layer, area, offset)
        .map_err(|e| invalid(e.to_string()))?;
    Ok(Some(get(request.url, 3_600)))
}

/// A charted wreck, obstruction or rock, worded. Its depth is never shown.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct StructurePoint {
    /// Stable within a band's layer: `kind/object id`.
    pub id: String,
    pub kind: StructureType,
    /// "Wreck", "Obstruction", "Rock", "Fish haven".
    pub title: String,
    /// The chart's own category, as charted: "As charted: dangerous wreck".
    pub category: Option<String>,
    /// The charted name.
    pub name: Option<String>,
    /// "Surveyed 2019".
    pub surveyed: Option<String>,
    pub lat: f64,
    pub lon: f64,
    pub fish_haven: bool,
    /// Why no depth is shown (safety text).
    pub depth_note: String,
}

/// One page of structure points.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct StructurePage {
    pub points: Vec<StructurePoint>,
    /// Another page may hold more: ask again with `offset` advanced by the points read.
    pub more: bool,
    /// How many features were left out for a missing id or position.
    pub skipped: u32,
    /// How many features the page held, for advancing `offset`.
    pub read: u32,
}

/// Reads one page of structure points of `kind`, worded for the settings. `attempt` counts the
/// requests already made for this page: a transient error body is retried once, a busy server
/// twice ([`MapError::RetryLater`]).
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn parse_structure_points(
    kind: StructureType,
    response: HostResponse,
    attempt: u32,
    settings_json: String,
) -> Result<StructurePage, MapError> {
    let (config, _) = load(&settings_json)?;
    let f = formatter(&config, "UTC");
    let fetched = fetched(response).map_err(|e| invalid(e.to_string()))?;
    let page = encdirect::parse_points(kind.into(), &fetched.as_response())
        .map_err(|e| enc_error(&e, attempt))?;
    let note = f.strings.safety(SafetyKey::MapStructureDepthNotShown);
    let read = page.points.len() + page.skipped;
    Ok(StructurePage {
        more: page.more,
        skipped: u32::try_from(page.skipped).unwrap_or(u32::MAX),
        read: u32::try_from(read).unwrap_or(u32::MAX),
        points: page
            .points
            .iter()
            .map(|p| {
                let fish_haven = p.is_fish_haven();
                StructurePoint {
                    id: format!("{}/{}", structure_slug(p.kind), p.object_id),
                    kind: p.kind.into(),
                    title: structure_title(&f, p.kind, fish_haven).to_owned(),
                    category: p
                        .category
                        .as_deref()
                        .map(|c| structure_category(&f, c, p.category_may_be_cut)),
                    name: p.name.clone(),
                    surveyed: p.source_date.map(|d| structure_surveyed(&f, d)),
                    lat: p.position.lat,
                    lon: p.position.lon,
                    fish_haven,
                    depth_note: note.to_owned(),
                }
            })
            .collect(),
    })
}

const fn structure_slug(kind: StructureKind) -> &'static str {
    match kind {
        StructureKind::Wreck => "wreck",
        StructureKind::Obstruction => "obstruction",
        StructureKind::Rock => "rock",
    }
}

// ---------------------------------------------------------------------------------------------
// State lake contours and artificial reefs
// ---------------------------------------------------------------------------------------------

fn state_error(e: &StateMapError) -> MapError {
    match e {
        StateMapError::Status {
            status: 429 | 502 | 503 | 504,
        } => MapError::RetryLater { after_s: 60 },
        other => invalid(other.to_string()),
    }
}

/// The state and coverage of a state contour layer.
fn contour_layer(id: &str) -> Result<(ContourState, Bbox), MapError> {
    let l = layer(id)?;
    match l.kind {
        LayerKind::Features(feature) if feature.kind == FeatureKind::LakeContours => {
            let state = match feature.state {
                "Indiana" => ContourState::Indiana,
                "Minnesota" => ContourState::Minnesota,
                other => return Err(invalid(format!("no contour source for {other}"))),
            };
            Ok((state, feature.coverage))
        }
        _ => Err(invalid(format!("`{id}` is not a lake contour layer"))),
    }
}

/// The request for a state's lake contours inside a view at `zoom`, one page of 2,000 lines
/// generalised for the zoom; `offset` is 0 for the first page, then the number of lines already
/// read while the page said there is more. None where the view is outside the state's data or
/// above the layer's shallowest zoom.
#[uniffi::export]
#[allow(clippy::too_many_arguments)] // a view is four edges and a zoom
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn state_contour_request(
    layer: String,
    west: f64,
    south: f64,
    east: f64,
    north: f64,
    zoom: u8,
    offset: u32,
) -> Result<Option<HostRequest>, MapError> {
    let (state, coverage) = contour_layer(&layer)?;
    let area = view_box(west, south, east, north)?;
    let l = self::layer(&layer)?;
    if l.min_zoom.is_some_and(|min| zoom < min) || !crate::map::overlaps(area, coverage) {
        return Ok(None);
    }
    let request =
        statemaps::contour_request(state, area, zoom, offset).map_err(|e| state_error(&e))?;
    Ok(Some(get(request.url, 86_400)))
}

/// A lake contour line a state publishes.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct StateContourLine {
    pub points: Vec<MapPoint>,
    /// The depth below the lake surface in the user's unit: "10 ft". Not the elevation model's
    /// approximate depth.
    pub label: String,
    pub label_at: Option<LabelAt>,
    /// Indiana's index contours; Minnesota does not mark any.
    pub major: bool,
    /// The lake's name as the source gives it.
    pub lake: Option<String>,
}

/// One page of lake contours.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct StateContours {
    pub lines: Vec<StateContourLine>,
    /// Another page may hold more.
    pub more: bool,
    /// How many features the page held, for advancing `offset`.
    pub read: u32,
}

/// Reads a page of `layer`'s lake contours, labelled in the user's depth unit, with a label
/// where a line is long enough at `zoom`.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn parse_state_contours(
    layer: String,
    response: HostResponse,
    zoom: u8,
    settings_json: String,
) -> Result<StateContours, MapError> {
    let (state, _) = contour_layer(&layer)?;
    let (config, _) = load(&settings_json)?;
    let f = formatter(&config, "UTC");
    let fetched = fetched(response).map_err(|e| invalid(e.to_string()))?;
    let page =
        statemaps::parse_contours(state, &fetched.as_response()).map_err(|e| state_error(&e))?;
    let polylines: Vec<Polyline> = page.lines.iter().map(|l| l.line.clone()).collect();
    let mid_lat = polylines
        .iter()
        .find_map(|p| p.points.first().map(|q| q.lat))
        .unwrap_or(0.0);
    let pixel_m = metres_per_pixel(f64::from(zoom.min(MAX_ZOOM)), mid_lat);
    let anchors: BTreeMap<usize, LabelAnchor> =
        contours::label_anchors(&polylines, LABEL_MIN_PX * pixel_m)
            .into_iter()
            .map(|a| (a.line, a))
            .collect();
    let read = page.lines.len() + page.skipped;
    Ok(StateContours {
        more: page.more,
        read: u32::try_from(read).unwrap_or(u32::MAX),
        lines: page
            .lines
            .iter()
            .enumerate()
            .map(|(i, l)| StateContourLine {
                points: points_of(&l.line),
                label: depth_label(&f, -l.line.value),
                label_at: anchors.get(&i).map(LabelAt::from),
                major: l.index.unwrap_or(false),
                lake: l.lake.clone(),
            })
            .collect(),
    })
}

/// The request for the artificial reefs of New York.
#[uniffi::export]
pub fn reefs_request() -> HostRequest {
    get(statemaps::reefs_request().url, 7 * 86_400)
}

/// An artificial reef site: the general area where material was placed.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct ReefInfo {
    pub name: String,
    /// The middle of the listed corners, for a marker.
    pub lat: f64,
    pub lon: f64,
    /// The corners of the area.
    pub corners: Vec<MapPoint>,
    pub body_of_water: Option<String>,
    /// "Depth 20–70 ft" in the user's unit, as the state lists the minimum and maximum; no datum
    /// is stated.
    pub depth_text: Option<String>,
    /// The state's own comments, as written.
    pub comments: Option<String>,
}

/// Reads the reef table, worded for the settings.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn parse_reefs(
    response: HostResponse,
    settings_json: String,
) -> Result<Vec<ReefInfo>, MapError> {
    let (config, _) = load(&settings_json)?;
    let f = formatter(&config, "UTC");
    let fetched = fetched(response).map_err(|e| invalid(e.to_string()))?;
    let reefs = statemaps::parse_reefs(&fetched.as_response()).map_err(|e| state_error(&e))?;
    Ok(reefs
        .sites
        .iter()
        .map(|r| ReefInfo {
            name: r.name.clone(),
            lat: r.position.lat,
            lon: r.position.lon,
            corners: r.corners.iter().copied().map(point_of).collect(),
            body_of_water: r.body_of_water.clone(),
            depth_text: reef_depths(&f, r.min_depth_m, r.max_depth_m),
            comments: r.comments.clone(),
        })
        .collect())
}
