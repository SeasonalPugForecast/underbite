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
//!
//! Breaks are found in the grid another layer already shows ([`GridStore::breaks`] for the
//! catalogue's `sst-breaks` and `chlorophyll-breaks`), with their words, legends and threshold
//! stops ([`breaks_words`], [`break_threshold_stops`]).

use std::collections::BTreeMap;

use std::sync::Arc;

use underbite_config::schema::Config;
use underbite_core::map::breaks::{self, BreakKind, BreakOptions};
use underbite_core::map::catalogue::{self, BreakLayer, FeatureKind, Layer, LayerKind};
use underbite_core::map::contours::{self, LabelAnchor, MAX_LEVELS};
use underbite_core::map::dem::{self, DemSourceInfo, SHORE_M, WaterBody};
use underbite_core::map::enc::StructureKind;
use underbite_core::map::grid::{self, Grid};
use underbite_core::map::render::{self, DEPTH, Ramp};
use underbite_core::map::tiles::{MAX_ZOOM, TILE_PX};
use underbite_core::map::{Bbox, GeoPoint, GridHost, Polyline, TileKey};
use underbite_core::time::UnixTime;
use underbite_core::units::HeightUnit;
use underbite_core::view::format::Formatter;
use underbite_core::view::strings::{
    Key, MapQuantity, SafetyKey, break_strength, break_threshold_stops as threshold_stops,
    breaks_few_clear, breaks_from, breaks_needs_source, contour_interval_text, depth_band_text,
    depth_datums_line, depth_label, reef_depths, structure_category,
    structure_group_title as group_title, structure_surveyed, structure_title,
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

/// Below this share of a chlorophyll grid's cells in view holding a value, clouds hid most of the
/// day: [`BreaksState::FewClearCells`]. Land counts as missing too, so the share is low.
pub const FEW_CLEAR_SHARE: f64 = 0.2;

/// How many times the threshold a break must be to be drawn at full weight
/// ([`BreakLine::emphasis`] 1).
const FULL_EMPHASIS_TIMES: f64 = 4.0;

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

/// A box of four edges, checked, clamped to the world and rounded outward to two decimals.
fn view_box(west: f64, south: f64, east: f64, north: f64) -> Result<Bbox, MapError> {
    if ![west, south, east, north].iter().all(|v| v.is_finite()) || west >= east || south >= north {
        return Err(invalid("a view needs west < east and south < north"));
    }
    // The view may be centred on a saved spot: it is kept to two decimals, rounded outward so
    // that it still covers what is on screen, and the same view asks for the same area.
    underbite_providers::species::request_box(Bbox {
        west: west.clamp(-180.0, 180.0),
        south: south.clamp(-85.0, 85.0),
        east: east.clamp(-180.0, 180.0),
        north: north.clamp(-85.0, 85.0),
    })
    .ok_or_else(|| invalid("a view needs west < east and south < north"))
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
    /// area already. The box is rounded outward to hundredths of a degree so that the same view asks
    /// for the same area.
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

    /// What the depths in a view (`west`, `south`, `east`, `north` in degrees) are measured
    /// from, for the depth legend: the vertical datums of the elevation models found at the
    /// places inside it (or within about two kilometres) that [`GridStore::add_dem_sources`]
    /// was given, finest model first, each once. Ask [`GridStore::dem_sources_request`] for the
    /// view's centre to have one. A place's every model is named, not only its finest: which
    /// model the mosaic draws depends on the cell size asked for. Without a place in the view
    /// the list is empty and the line says the datum depends on the model.
    #[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
    pub fn depth_datums(
        &self,
        west: f64,
        south: f64,
        east: f64,
        north: f64,
        settings_json: String,
    ) -> Result<DepthDatums, MapError> {
        let area = view_box(west, south, east, north)?;
        let (config, _) = load(&settings_json)?;
        let f = formatter(&config, "UTC");
        let unknown = f.strings.safety(SafetyKey::MapDepthDatumUnknown);
        // Each model with a cell size (the mosaic's overviews have none), finest first.
        let mut models: Vec<(f64, String)> = self
            .lock()
            .dem_sources_in(area)
            .into_iter()
            .flatten()
            .filter_map(|s| {
                let cell = s.cell_arcsec.filter(|c| c.is_finite())?;
                let datum = s.vertical_datum.as_deref().unwrap_or(unknown);
                Some((cell, datum.to_owned()))
            })
            .collect();
        models.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        let mut datums: Vec<String> = Vec::new();
        for (_, datum) in models {
            if !datums.contains(&datum) {
                datums.push(datum);
            }
        }
        Ok(DepthDatums {
            text: depth_datums_line(&f, &datums),
            datums,
        })
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
        // The finest block that covers the whole tile, the tile's own on a tie, else the block
        // covering most of it (an area fetched for contours shades too: one fetch serves both).
        let grid = {
            let blocks = self.lock().dem_blocks(Some(wanted));
            let own = DemId::Tile {
                z,
                x,
                y,
                px: ncei_dem::DEFAULT_TILE_PX,
            };
            best_block(&blocks, wanted, Some(own)).map(Arc::clone)
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
        let blocks = self.lock().dem_blocks(Some(area));
        let Some(grid) = best_block(&blocks, area, None) else {
            return Ok(Vec::new());
        };
        let f = formatter(&config, "UTC");
        Ok(contour_lines_of(grid, area, zoom, interval, &f))
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
        let grids = self.held(l.id, area);
        let f = formatter(&config, "UTC");
        break_lines_of(&grids, kind, area, &config, &f)
    }

    /// The breaks a breaks layer (`sst-breaks` or `chlorophyll-breaks`) draws in a view at
    /// `zoom`, found in the blocks the store holds for its grid layer's view (the dataset and
    /// time [`GridStore::grid_requests`] last asked for: no request of their own), with what to
    /// say about them. They are found at the stride the view's blocks are fetched at; while none
    /// of those is held, at the finest stride held. A view whose blocks are coarser than a
    /// stride of four draws none ([`BreaksState::ZoomIn`]). Ask again when the zoom, the
    /// threshold or the grid layer's date changes, and when its blocks arrive. Thinned, at most
    /// [`MAX_BREAK_VERTICES`] vertices, strongest first.
    #[allow(clippy::too_many_arguments)] // a view is four edges and a zoom
    #[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
    pub fn breaks(
        &self,
        layer: String,
        west: f64,
        south: f64,
        east: f64,
        north: f64,
        zoom: f64,
        settings_json: String,
    ) -> Result<BreakSet, MapError> {
        let (b, source) = breaks_layer(&layer)?;
        let area = view_box(west, south, east, north)?;
        if !zoom.is_finite() {
            return Err(invalid("the zoom is not a number"));
        }
        let (config, _) = load(&settings_json)?;
        let f = formatter(&config, "UTC");
        let grids = self.held(source.id, area);
        Ok(break_set(&grids, b, source, area, zoom, &config, &f))
    }
}

impl GridStore {
    /// The first grid of each block the store holds for `layer`'s view that overlaps `area`.
    fn held(&self, layer: &str, area: Bbox) -> Vec<Grid> {
        let shared = self.lock().selected(layer, Some(area));
        shared
            .iter()
            .filter_map(|g| g.first().map(|g| Grid::clone(g)))
            .collect()
    }
}

/// What the depths in a view are measured from.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct DepthDatums {
    /// The vertical datums as the service names them ("NAVD 88", "EGM 2008"), finest model
    /// first, each once; "datum not stated" for a model that states none. Empty where no model
    /// list is known for the view.
    pub datums: Vec<String>,
    /// The line for the depth legend (safety text): the datums found, or that the datum depends
    /// on the model; never chart datum.
    pub text: String,
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

/// The spacing of a block's columns, degrees of longitude: the same measure for an area (evenly
/// spaced in degrees) and a Web Mercator tile (evenly spaced in longitude). A single column
/// counts as the coarsest.
fn cell_deg(grid: &Grid) -> f64 {
    match grid.lons.as_slice() {
        [first, .., last] => (last - first) / (grid.lons.len() - 1) as f64,
        _ => f64::INFINITY,
    }
}

/// The block to draw `area` from: of the blocks that cover all of it, the finest (`preferred`
/// on a tie); without one, the block that covers most of it, the finer on a tie. Never the
/// order the blocks are held in, unless two are alike in all of that.
fn best_block(
    blocks: &[(DemId, Arc<Grid>)],
    area: Bbox,
    preferred: Option<DemId>,
) -> Option<&Arc<Grid>> {
    let whole = (area.east - area.west) * (area.north - area.south);
    // (covers all of it, how much of it, the cell, the preferred one)
    let rank = |(id, grid): &(DemId, Arc<Grid>)| {
        let covered = coverage(grid, area);
        let all = covered >= whole * (1.0 - 1e-9);
        (
            all,
            if all { whole } else { covered },
            cell_deg(grid),
            preferred == Some(*id),
        )
    };
    blocks
        .iter()
        .map(|block| (rank(block), &block.1))
        .max_by(|(a, _), (b, _)| {
            a.0.cmp(&b.0)
                .then(a.1.total_cmp(&b.1))
                // A smaller cell ranks higher.
                .then(b.2.total_cmp(&a.2))
                .then(a.3.cmp(&b.3))
        })
        .map(|(_, grid)| grid)
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
    /// The strength: the mean change per kilometre along the line, in °C for temperature and in
    /// log10 units of the concentration for chlorophyll.
    pub value: f64,
    /// How the line is weighted against the settings' threshold, for its width: 0 at the
    /// threshold, 1 at four times it or more, between them by the ratio's logarithm.
    pub emphasis: f64,
}

/// What a breaks layer's lines came to in a view.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum BreaksState {
    /// The lines are the breaks in view.
    Drawn,
    /// The grid has values in view but no change as sharp as the threshold.
    NoBreaks,
    /// Clouds hid most of the day's grid in view: the few lines found are drawn, and an earlier
    /// day may show more.
    FewClearCells,
    /// The view's blocks are too coarse for breaks: zoom in.
    ZoomIn,
    /// The store holds none of the grid layer's blocks for the view yet.
    Waiting,
}

/// The breaks of a layer in a view and what to say about them.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct BreakSet {
    /// Strongest first.
    pub lines: Vec<BreakLine>,
    pub state: BreaksState,
    /// What to say beside or in place of the lines: "Zoom in to see breaks", "Finding breaks…",
    /// "No breaks this sharp in view", "Few clear cells on Nov 16. An earlier day may show
    /// more."; none while breaks are drawn.
    pub state_text: Option<String>,
    /// Which grid the lines were found in: "From Sea surface temperature, Nov 16"; none
    /// without one.
    pub from_text: Option<String>,
    /// The time the grid describes, Unix seconds; none without one.
    pub time: Option<i64>,
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

/// The settings' threshold for a kind of break, canonical per km.
fn threshold_of(config: &Config, kind: BreakKind) -> f64 {
    f64::from(match kind {
        BreakKind::Sst => config.map.breaks.sst_c_per_km,
        BreakKind::Chlorophyll => config.map.breaks.chlorophyll_log10_per_km,
    })
}

/// A line's weight against the threshold: 0 at it, 1 at [`FULL_EMPHASIS_TIMES`] it or more.
fn emphasis(value: f64, threshold: f64) -> f64 {
    if !(value.is_finite() && threshold.is_finite() && threshold > 0.0) {
        return 0.0;
    }
    ((value / threshold).log2() / FULL_EMPHASIS_TIMES.log2()).clamp(0.0, 1.0)
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
    Ok(lines_in(&grid, kind, area, threshold_of(config, kind), f))
}

/// The breaks of a merged grid that reach into `area`, thinned, strongest first, at most
/// [`MAX_BREAK_VERTICES`] vertices.
fn lines_in(
    grid: &Grid,
    kind: BreakKind,
    area: Bbox,
    threshold: f64,
    f: &Formatter,
) -> Vec<BreakLine> {
    let Some(span) = field_span(grid, kind) else {
        return Vec::new();
    };
    let default_spacing = match kind {
        BreakKind::Sst => SST_SPACING,
        BreakKind::Chlorophyll => CHLOROPHYLL_SPACING,
    };
    // Few enough levels for `contours::levels`, whatever the field's range.
    let spacing = default_spacing.max(span / (MAX_LEVELS as f64 / 2.0));
    let options = BreakOptions::new(kind, threshold, spacing);
    let cell_m = grid
        .lats
        .windows(2)
        .next()
        .map_or(1_000.0, |w| (w[1] - w[0]).abs() * 111_000.0);
    let mut pieces: Vec<Polyline> = breaks::breaks_with(grid, kind, &options)
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
            value: piece.value,
            emphasis: emphasis(piece.value, threshold),
        });
    }
    out
}

/// How a breaks layer finds its breaks, and the grid layer it finds them in.
fn breaks_layer(id: &str) -> Result<(BreakLayer, &'static Layer), MapError> {
    let l = layer(id)?;
    let b = *l
        .breaks()
        .ok_or_else(|| invalid(format!("`{id}` is not a breaks layer")))?;
    let source = layer(b.source)?;
    if source.grid().is_none() {
        return Err(invalid(format!("`{}` has no grid", source.id)));
    }
    Ok((b, source))
}

/// The share of the cells of `grids` inside `area` that hold a value, over the blocks held only
/// (a block not fetched yet is not cloud); none without a cell in view.
fn clear_share(grids: &[Grid], area: Bbox) -> Option<f64> {
    let (mut clear, mut all) = (0usize, 0usize);
    for g in grids {
        let width = g.lons.len();
        for (i, lat) in g.lats.iter().enumerate() {
            if !(area.south..=area.north).contains(lat) {
                continue;
            }
            for (j, lon) in g.lons.iter().enumerate() {
                if !(area.west..=area.east).contains(lon) {
                    continue;
                }
                all += 1;
                if g.values.get(i * width + j).is_some_and(|v| v.is_finite()) {
                    clear += 1;
                }
            }
        }
    }
    (all > 0).then(|| clear as f64 / all as f64)
}

/// The breaks of breaks layer `b` in `area` at `zoom`, from the blocks held for its grid layer
/// `source`.
fn break_set(
    grids: &[Grid],
    b: BreakLayer,
    source: &Layer,
    area: Bbox,
    zoom: f64,
    config: &Config,
    f: &Formatter,
) -> BreakSet {
    let s = f.strings;
    let empty = |state: BreaksState, key: Key| BreakSet {
        lines: Vec::new(),
        state,
        state_text: Some(s.get(key).to_owned()),
        from_text: None,
        time: None,
    };
    let native = grids.first().map_or_else(
        || source.grid().map_or(0.0, |g| g.primary.native_step_deg),
        |g| g.source.native_step_deg,
    );
    let view_stride = f64::from(grid::stride_for_zoom(native, zoom));
    if view_stride > MAX_BREAK_STRIDE {
        return empty(BreaksState::ZoomIn, Key::MapBreaksZoomIn);
    }
    // The blocks at the view's stride, else the finest held that is fine enough.
    let near = |g: &Grid, stride: f64| (stride_of(g) - stride).abs() < 0.5;
    let stride = if grids.iter().any(|g| near(g, view_stride)) {
        Some(view_stride)
    } else {
        grids
            .iter()
            .map(stride_of)
            .filter(|&st| st <= MAX_BREAK_STRIDE)
            .min_by(f64::total_cmp)
    };
    let at_stride: Vec<&Grid> = stride.map_or_else(Vec::new, |st| {
        grids.iter().filter(|g| near(g, st)).collect()
    });
    // Blocks of one time only, the newest: the source's newest day may have changed between two
    // blocks.
    let Some(time) = at_stride.iter().map(|g| g.time).max() else {
        return empty(BreaksState::Waiting, Key::MapBreaksLoading);
    };
    let same: Vec<Grid> = at_stride
        .into_iter()
        .filter(|g| g.time == time)
        .cloned()
        .collect();
    let Ok(merged) = Grid::merge(&same) else {
        return empty(BreaksState::Waiting, Key::MapBreaksLoading);
    };
    let lines = lines_in(&merged, b.kind, area, threshold_of(config, b.kind), f);
    let cloudy = b.kind == BreakKind::Chlorophyll
        && clear_share(&same, area).is_some_and(|share| share < FEW_CLEAR_SHARE);
    let (state, state_text) = if cloudy {
        (BreaksState::FewClearCells, Some(breaks_few_clear(f, time)))
    } else if lines.is_empty() {
        (
            BreaksState::NoBreaks,
            Some(s.get(Key::MapBreaksNone).to_owned()),
        )
    } else {
        (BreaksState::Drawn, None)
    };
    let source_title = s.layer_title(source.id).unwrap_or(source.id);
    BreakSet {
        lines,
        state,
        state_text,
        from_text: Some(breaks_from(f, source_title, time)),
        time: Some(time.0),
    }
}

// ---------------------------------------------------------------------------------------------
// Breaks layers: words, legends and the threshold's stops
// ---------------------------------------------------------------------------------------------

/// One weight of a breaks legend: a line drawn at `emphasis` and the strength it stands for.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct BreakLegendStop {
    /// As [`BreakLine::emphasis`]: 0 at the threshold, 1 at four times it.
    pub emphasis: f64,
    /// The strength in the user's units: "1.4 °F per mi".
    pub label: String,
}

/// One breaks layer as its drawer section and legend show it.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct BreakLayerWords {
    /// The breaks layer's id: `sst-breaks`.
    pub id: String,
    /// The grid layer the breaks are found in: `sst`.
    pub source: String,
    /// "SST breaks".
    pub title: String,
    /// The grid layer's title: "Sea surface temperature".
    pub source_title: String,
    /// The settings' threshold, canonical (°C per km, or log10 units per km), to place the
    /// control among [`break_threshold_stops`].
    pub threshold: f64,
    /// The settings' threshold in the user's units: "1.4 °F per mi".
    pub threshold_text: String,
    /// In place of breaks while the grid layer is not on the map: "Show Sea surface temperature
    /// to see its breaks".
    pub needs_source: String,
    /// "Where the temperature changes fastest; heavier lines change faster".
    pub legend_caption: String,
    /// Lines at the threshold, twice it and four times it, lightest first.
    pub legend: Vec<BreakLegendStop>,
    /// The breaks layer's credit, which names the grid it is found in (safety text).
    pub credit: String,
}

/// The words of the breaks layers' controls and states, in the settings' voice and units.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct BreaksWords {
    /// The threshold control's title, before the threshold: "Breaks from".
    pub threshold_title: String,
    /// The control as a screen reader names it: "Break threshold".
    pub threshold_label: String,
    /// In place of breaks when the view is too wide for them: "Zoom in to see breaks".
    pub zoom_in: String,
    /// While the grid is still coming: "Finding breaks…".
    pub loading: String,
    /// "Not for navigation." (safety text).
    pub notice: String,
    /// Each breaks layer of the catalogue, in drawer order.
    pub layers: Vec<BreakLayerWords>,
}

/// The emphases a breaks legend shows, as multiples of the threshold.
const LEGEND_TIMES: [f64; 3] = [1.0, 2.0, FULL_EMPHASIS_TIMES];

fn break_layer_words(l: &Layer, b: BreakLayer, config: &Config, f: &Formatter) -> BreakLayerWords {
    let s = f.strings;
    let source_title = s.layer_title(b.source).unwrap_or(b.source);
    let threshold = threshold_of(config, b.kind);
    BreakLayerWords {
        id: l.id.to_owned(),
        source: b.source.to_owned(),
        title: s.layer_title(l.id).unwrap_or(l.id).to_owned(),
        source_title: source_title.to_owned(),
        threshold,
        threshold_text: break_strength(f, b.kind, threshold),
        needs_source: breaks_needs_source(f, source_title),
        legend_caption: s.layer_legend(l.id).unwrap_or_default().to_owned(),
        legend: LEGEND_TIMES
            .iter()
            .map(|times| BreakLegendStop {
                emphasis: emphasis(threshold * times, threshold),
                label: break_strength(f, b.kind, threshold * times),
            })
            .collect(),
        credit: s.layer_credit(l.id).unwrap_or_default().to_owned(),
    }
}

/// The words of the breaks layers' threshold control and states, and each breaks layer's title,
/// threshold, legend and credit, in the settings' voice and units.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn breaks_words(settings_json: String) -> Result<BreaksWords, MapError> {
    let (config, _) = load(&settings_json)?;
    let f = formatter(&config, "UTC");
    let s = f.strings;
    Ok(BreaksWords {
        threshold_title: s.get(Key::MapBreaksThreshold).to_owned(),
        threshold_label: s.get(Key::MapBreaksThresholdLabel).to_owned(),
        zoom_in: s.get(Key::MapBreaksZoomIn).to_owned(),
        loading: s.get(Key::MapBreaksLoading).to_owned(),
        notice: s.safety(SafetyKey::NotForNavigation).to_owned(),
        layers: catalogue::drawer_order()
            .filter_map(|l| l.breaks().map(|b| break_layer_words(l, *b, &config, &f)))
            .collect(),
    })
}

/// Round thresholds for a breaks layer's control, weakest first, in the user's units: tenths of
/// a degree per mile or kilometre for temperature ("1.5 °F per mi"), ratios of concentrations
/// for chlorophyll ("×1.10 per km"). Each has the value to store in `map.breaks` (canonical: °C
/// per km for `sst-breaks`, log10 units per km for `chlorophyll-breaks`) and its label.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn break_threshold_stops(
    layer: String,
    settings_json: String,
) -> Result<Vec<DisplayStop>, MapError> {
    let (b, _) = breaks_layer(&layer)?;
    let (config, _) = load(&settings_json)?;
    let f = formatter(&config, "UTC");
    Ok(threshold_stops(&f, b.kind)
        .into_iter()
        .map(|(value, label)| DisplayStop { value, label })
        .collect())
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

/// The words of the structure points' states, in the settings' voice.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct StructureWords {
    /// While the points for the view are fetched: "Loading wrecks and obstructions…".
    pub loading: String,
    /// When they could not be fetched.
    pub failed: String,
    /// In place of the points below the zoom of the coarsest band ([`structure_band`] is
    /// `None`): "Zoom in to see wrecks and obstructions".
    pub zoom_in: String,
    /// "Not for navigation." (safety text).
    pub notice: String,
}

/// The words of the structure points' states: loading, failed and too wide a view.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn structure_words(settings_json: String) -> Result<StructureWords, MapError> {
    let (config, _) = load(&settings_json)?;
    let s = formatter(&config, "UTC").strings;
    Ok(StructureWords {
        loading: s.get(Key::MapStructureLoading).to_owned(),
        failed: s.get(Key::MapStructureFailed).to_owned(),
        zoom_in: s.get(Key::MapStructureZoomIn).to_owned(),
        notice: s.safety(SafetyKey::NotForNavigation).to_owned(),
    })
}

/// The title of the list of `count` structure points that share a place on the map:
/// "3 charted features".
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn structure_group_title(count: u32, settings_json: String) -> Result<String, MapError> {
    let (config, _) = load(&settings_json)?;
    Ok(group_title(&formatter(&config, "UTC"), count))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::default_settings_json;

    const NOW: UnixTime = UnixTime(1_794_000_000);

    /// An elevation grid over `area` with cells of `cell` degrees, valued by `height(lat, lon)`
    /// (metres, positive up).
    fn grid(area: Bbox, cell: f64, height: impl Fn(f64, f64) -> f32) -> Grid {
        let axis = |from: f64, to: f64| -> Vec<f64> {
            let n = ((to - from) / cell).round() as usize;
            (0..=n).map(|i| (i as f64).mul_add(cell, from)).collect()
        };
        let (lats, lons) = (axis(area.south, area.north), axis(area.west, area.east));
        let values = lats
            .iter()
            .flat_map(|&lat| lons.iter().map(move |&lon| (lat, lon)))
            .map(|(lat, lon)| height(lat, lon))
            .collect();
        let source = underbite_core::map::GridSource {
            node: GridHost::NceiDem,
            dataset: "DEM_global_mosaic".to_owned(),
            variables: vec!["elevation".to_owned()],
            altitude_axis: false,
            native_step_deg: cell,
        };
        Grid::new(source, NOW, lats, lons, values, "m").unwrap()
    }

    fn area_id(area: Bbox, cell: f64) -> DemId {
        DemId::Area {
            west: micro(area.west),
            south: micro(area.south),
            east: micro(area.east),
            north: micro(area.north),
            cell: micro(cell),
        }
    }

    /// Off the harbour: the blocks reach past the view on every side.
    const BLOCK: Bbox = Bbox {
        west: -74.3,
        south: 40.3,
        east: -73.7,
        north: 40.9,
    };
    const VIEW: Bbox = Bbox {
        west: -74.2,
        south: 40.4,
        east: -73.8,
        north: 40.8,
    };

    /// Deepening to the south, 2 m to 62 m.
    fn deepening_south(lat: f64, _lon: f64) -> f32 {
        (40.9 - lat).mul_add(-100.0, -2.0) as f32
    }

    /// Deepening to the east: a different picture.
    fn deepening_east(_lat: f64, lon: f64) -> f32 {
        (lon + 74.3).mul_add(-50.0, -2.0) as f32
    }

    #[test]
    fn the_finest_block_that_covers_the_view_is_chosen_whatever_the_order() {
        let fine = Arc::new(grid(BLOCK, 0.002, deepening_south));
        let coarse = Arc::new(grid(BLOCK, 0.05, deepening_east));
        let chosen_cell = |blocks: &[(DemId, Arc<Grid>)]| {
            best_block(blocks, VIEW, None).map(|g| (cell_deg(g) * 1e4).round() / 1e4)
        };
        let a = (area_id(BLOCK, 0.002), Arc::clone(&fine));
        let b = (area_id(BLOCK, 0.05), Arc::clone(&coarse));
        assert_eq!(chosen_cell(&[a.clone(), b.clone()]), Some(0.002));
        assert_eq!(chosen_cell(&[b.clone(), a]), Some(0.002));

        // A fine block over part of the view loses to a coarse one over all of it.
        let corner = Bbox {
            west: -74.3,
            south: 40.3,
            east: -74.0,
            north: 40.6,
        };
        let part = (
            area_id(corner, 0.001),
            Arc::new(grid(corner, 0.001, deepening_south)),
        );
        assert_eq!(chosen_cell(&[part.clone(), b.clone()]), Some(0.05));
        // Without a block over all of it, the one over most of it.
        let most_of_it = Bbox {
            west: -74.3,
            south: 40.3,
            east: -73.9,
            north: 40.9,
        };
        let most = (
            area_id(most_of_it, 0.05),
            Arc::new(grid(most_of_it, 0.05, deepening_east)),
        );
        assert_eq!(chosen_cell(&[most, part]), Some(0.05));
        assert_eq!(chosen_cell(&[]), None);

        // Alike in cover and cell: the preferred block (a tile's own) wins.
        let own = (
            DemId::Tile {
                z: 9,
                x: 150,
                y: 192,
                px: 128,
            },
            Arc::new(grid(BLOCK, 0.05, deepening_south)),
        );
        let both = [own.clone(), b.clone()];
        let pick = |preferred| best_block(&both, VIEW, Some(preferred));
        assert!(Arc::ptr_eq(pick(own.0).unwrap(), &own.1));
        assert!(Arc::ptr_eq(pick(b.0).unwrap(), &b.1));
    }

    /// A store holding a fine and a coarse block over the same box, under keys whose order puts
    /// the coarse one last; and the fine grid.
    fn store() -> (GridStore, Grid) {
        let fine = grid(BLOCK, 0.002, deepening_south);
        let store = GridStore::default();
        {
            let mut blocks = store.lock();
            blocks
                .insert_dem(area_id(BLOCK, 0.002), fine.clone(), NOW)
                .unwrap();
            blocks
                .insert_dem(area_id(BLOCK, 0.05), grid(BLOCK, 0.05, deepening_east), NOW)
                .unwrap();
        }
        assert!(area_id(BLOCK, 0.002) < area_id(BLOCK, 0.05));
        (store, fine)
    }

    #[test]
    fn depth_tiles_are_shaded_from_the_finest_block_that_covers_them() {
        let (store, fine) = store();
        // Zoom 10 inside the view.
        let key = TileKey {
            z: 10,
            x: 301,
            y: 385,
        };
        let tile = tile_bbox_of(key);
        assert!(VIEW.west < tile.west && tile.east < VIEW.east, "{tile:?}");
        assert!(
            VIEW.south < tile.south && tile.north < VIEW.north,
            "{tile:?}"
        );
        let drawn = store
            .render_depth_tile(key.z, key.x, key.y, default_settings_json())
            .unwrap()
            .unwrap();
        let expected =
            dem::render_depth_tile(&fine, WaterBody::Sea, None, None, key, TILE_PX).unwrap();
        assert!(drawn == expected, "the tile is the fine block's");
    }

    #[test]
    fn contours_are_drawn_from_the_finest_block_that_covers_the_view() {
        let (store, fine) = store();
        let settings = default_settings_json();
        let lines = store
            .contour_lines(
                VIEW.west,
                VIEW.south,
                VIEW.east,
                VIEW.north,
                10.0,
                settings.clone(),
            )
            .unwrap();
        let (config, _) = load(&settings).unwrap();
        let f = formatter(&config, "UTC");
        let interval = f64::from(config.map.depth.contour_interval_m);
        let expected = contour_lines_of(&fine, VIEW, 10.0, interval, &f);
        assert!(expected.len() > 1);
        assert_eq!(lines, expected);
        // The fine block deepens to the south: its contours run west to east.
        let first = &lines[0].points;
        let (a, b) = (first[0], first[first.len() - 1]);
        assert!(
            (a.lat - b.lat).abs() < 0.01 && (a.lon - b.lon).abs() > 0.1,
            "{a:?} {b:?}"
        );
    }
}
