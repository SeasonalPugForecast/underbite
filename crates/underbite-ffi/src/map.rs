//! The map across the FFI: the layer catalogue, tile and grid requests, rendering, the point
//! inspector and measuring.
//!
//! Engine-neutral: the app draws what these calls describe. Every word and value the map shows is
//! written by the core's view (titles, credits, legend labels, inspector values, distances);
//! numbers cross only where the app draws with them (coordinates, opacity, colours, fractions of
//! a tile or a legend bar). Times are Unix seconds; dates are `YYYY-MM-DD` UTC days.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use underbite_config::schema::Config;
use underbite_core::map::catalogue::{
    self, AppleBase, DateMode, GridDraw, GridLayer, GridSpec, Group, Layer, LayerKind, PRESETS,
    resolve_preset,
};
use underbite_core::map::grid::{self, Block, Grid};
use underbite_core::map::render::{self, Ramp, RampRange};
use underbite_core::map::tiles::{
    MAX_ZOOM, TILE_PX, TileOutcome as CoreTileOutcome, cache_lifetime_s, classify, over_zoom,
    previous_day,
};
use underbite_core::map::{Bbox, GeoPoint, GridHost, TileKey, tiles};
use underbite_core::time::{LocalDate, UnixTime, UtcOffset, parse_date};
use underbite_core::view::format::Formatter;
use underbite_core::view::strings::{
    Key, MapQuantity, SafetyKey, Strings, map_current, map_measure, map_time, map_value,
};
use underbite_providers::erddap::{self, ErddapError, ErddapPacer, FIRST_PAUSE_S, GridTime};
use underbite_providers::request::Response;
use whether_providers::radar;

use crate::fetch::{HostMethod, HostRequest, HostResponse, fetched};
use crate::settings::{SettingsError, load};

// ---------------------------------------------------------------------------------------------
// Errors and shared helpers
// ---------------------------------------------------------------------------------------------

/// Why a map call failed.
#[derive(Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum MapError {
    /// The call cannot be answered as asked: an unknown layer, a bad tile or date, unreadable
    /// settings or an unreadable response.
    #[error("{message}")]
    Invalid {
        /// What is wrong.
        message: String,
    },
    /// The source has nothing for this request (a time past the end of its data, a box outside
    /// its grid).
    #[error("the source has no data for this request")]
    NoData,
    /// The source is busy or asked to slow down: ask again after `after_s` seconds.
    #[error("the source is busy; try again in {after_s} s")]
    RetryLater {
        /// Seconds to wait.
        after_s: u32,
    },
}

impl From<SettingsError> for MapError {
    fn from(e: SettingsError) -> Self {
        invalid(e.to_string())
    }
}

fn invalid(message: impl Into<String>) -> MapError {
    MapError::Invalid {
        message: message.into(),
    }
}

/// The catalogue's layer or base map with this id.
fn layer(id: &str) -> Result<&'static Layer, MapError> {
    catalogue::find(id).ok_or_else(|| invalid(format!("`{id}` is not a map layer")))
}

/// A formatter for the settings' units, voice and clock, showing times in `time_zone` (an IANA
/// name; an unknown one is UTC).
fn formatter(config: &Config, time_zone: &str) -> Formatter {
    Formatter::new(
        config.units.display_units(),
        config.presentation.voice,
        config.presentation.clock,
        time_zone,
    )
}

/// A formatter for words that depend on nothing the user chose, such as a date.
fn neutral_formatter() -> Formatter {
    formatter(&Config::default(), "UTC")
}

/// A Web Mercator tile key, checked against its zoom level.
fn tile_key(z: u8, x: u32, y: u32) -> Result<TileKey, MapError> {
    let count = 1u64 << z.min(MAX_ZOOM);
    if z > MAX_ZOOM || u64::from(x) >= count || u64::from(y) >= count {
        return Err(invalid(format!("{z}/{x}/{y} is not a map tile")));
    }
    Ok(TileKey { z, x, y })
}

/// A plain GET the app may cache for `max_age_s` when the server says nothing.
fn get(url: String, max_age_s: u64) -> HostRequest {
    HostRequest {
        url,
        headers: Vec::new(),
        method: HostMethod::Get,
        body: None,
        fresh: false,
        no_store: false,
        max_age_s: Some(u32::try_from(max_age_s).unwrap_or(u32::MAX)),
    }
}

/// The colour ramp and value range a layer is drawn with: the ramp named (else the layer's own)
/// over the layer's own range, narrowed to `low`..`high` when both are given (canonical units).
fn ramp_and_range(
    layer: &Layer,
    ramp: Option<&str>,
    low: Option<f64>,
    high: Option<f64>,
) -> Result<(&'static Ramp, RampRange), MapError> {
    let chosen = match ramp {
        Some(id) => render::ramp(id).ok_or_else(|| invalid(format!("`{id}` is not a ramp")))?,
        None => layer
            .colour_ramp()
            .ok_or_else(|| invalid(format!("`{}` does not colour values", layer.id)))?,
    };
    let own = layer.default_range().unwrap_or(chosen.default_range);
    let range = match (low, high) {
        (Some(low), Some(high)) => own.with_bounds(low, high),
        _ => own,
    };
    if !range.is_valid() {
        return Err(invalid("the colour range cannot be drawn"));
    }
    Ok((chosen, range))
}

/// A colour as the app draws it: 0 to 255 per channel, straight alpha.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Record)]
pub struct MapColour {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
    pub alpha: u8,
}

impl From<render::Rgba> for MapColour {
    fn from(c: render::Rgba) -> Self {
        Self {
            red: c.r,
            green: c.g,
            blue: c.b,
            alpha: c.a,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The catalogue
// ---------------------------------------------------------------------------------------------

/// How a layer reaches the map.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum LayerDraw {
    /// Apple's standard map.
    AppleStandard,
    /// Apple's satellite imagery.
    AppleSatellite,
    /// Apple's imagery with labels.
    AppleHybrid,
    /// Raster tiles: ask [`tile_request`] for each tile.
    Tiles,
    /// A grid rendered on the device: [`GridStore::render_tile`].
    GridRaster,
    /// A grid drawn as arrows: [`GridStore::arrows`].
    GridArrows,
    /// Station pins.
    Stations,
    /// The user's spots.
    Spots,
    /// Lines or points a state agency publishes.
    Features,
    /// Charted wrecks and obstructions as points.
    EncPoints,
    /// Listed, without a source yet.
    Unsourced,
}

/// What a layer's date control offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum LayerDates {
    /// The layer has no date.
    None,
    /// One field per UTC day: a date picker.
    Daily,
    /// One field per hour: a time slider.
    Hourly,
    /// A loop of recent frames.
    Frames,
}

impl From<DateMode> for LayerDates {
    fn from(mode: DateMode) -> Self {
        match mode {
            DateMode::None => Self::None,
            DateMode::Daily => Self::Daily,
            DateMode::Hourly => Self::Hourly,
            DateMode::Frames => Self::Frames,
        }
    }
}

/// One stop of a legend bar.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct LegendStop {
    /// Where the stop sits along the bar, 0 (low end) to 1 (high end).
    pub fraction: f64,
    pub colour: MapColour,
    /// The value at the stop in the user's unit: "59 °F".
    pub label: String,
}

/// One layer or base map, worded for the drawer.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct LayerInfo {
    /// The id the settings use (`"sst"`).
    pub id: String,
    /// The drawer section's id: `base`, `depth`, `structure`, `water`, `weather`, `stations`,
    /// `mine`.
    pub group: String,
    /// The section's title in the user's voice.
    pub group_title: String,
    /// A base map: one is chosen at a time.
    pub is_base: bool,
    /// The layer's title in the user's voice.
    pub title: String,
    pub draw: LayerDraw,
    pub dates: LayerDates,
    /// The shallowest zoom the layer is drawn at.
    pub min_zoom: Option<u8>,
    /// The deepest zoom the layer's tiles exist at; [`tile_request`] over-zooms past it.
    pub max_zoom: Option<u8>,
    /// The credit line to show with the layer (safety text).
    pub credit: String,
    /// The acknowledgements for the sources page (safety text).
    pub acknowledgements: Vec<String>,
    /// "Not for navigation." on charts, currents and depth (safety text); none elsewhere.
    pub notice: Option<String>,
    /// Whether the layer can be turned on.
    pub available: bool,
    /// Why not, in place of its switch.
    pub unavailable_text: Option<String>,
    /// Whether the settings show it (for a base map: whether it is the chosen one).
    pub shown: bool,
    /// The opacity the settings give it, 0 to 1, else the layer's own.
    pub opacity: f32,
    /// The layer's own opacity, 0 to 1.
    pub default_opacity: f32,
    /// The colour ramp the settings give it, else the layer's own; none for a layer that does
    /// not colour values.
    pub ramp: Option<String>,
    /// The caption under the legend.
    pub legend_caption: Option<String>,
    /// The legend bar of a grid layer, low to high, in the settings' ramp and range.
    pub legend: Vec<LegendStop>,
    /// What the layer's depths are measured from, as the source states it (safety text).
    pub depth_reference: Option<String>,
    /// The source's terms, as far as they are known (safety text).
    pub terms: Option<String>,
    /// For a grid layer, its datasets: the primary first, then the fallback.
    pub datasets: Vec<String>,
}

const fn group_id(group: Group) -> &'static str {
    match group {
        Group::Base => "base",
        Group::Depth => "depth",
        Group::Structure => "structure",
        Group::Water => "water",
        Group::Weather => "weather",
        Group::Stations => "stations",
        Group::Mine => "mine",
    }
}

const fn draw(kind: &LayerKind) -> LayerDraw {
    match kind {
        LayerKind::AppleBase(AppleBase::Standard) => LayerDraw::AppleStandard,
        LayerKind::AppleBase(AppleBase::Satellite) => LayerDraw::AppleSatellite,
        LayerKind::AppleBase(AppleBase::Hybrid) => LayerDraw::AppleHybrid,
        LayerKind::Tiles(_) => LayerDraw::Tiles,
        LayerKind::Grid(g) => match g.draw {
            GridDraw::Raster => LayerDraw::GridRaster,
            GridDraw::Arrows => LayerDraw::GridArrows,
        },
        LayerKind::Stations(_) => LayerDraw::Stations,
        LayerKind::Spots => LayerDraw::Spots,
        LayerKind::Features(_) => LayerDraw::Features,
        LayerKind::EncPoints => LayerDraw::EncPoints,
        LayerKind::Unsourced => LayerDraw::Unsourced,
    }
}

/// The legend of a grid layer in the settings' ramp and range; empty for other layers.
fn legend_of(
    l: &Layer,
    f: &Formatter,
    ramp: Option<&str>,
    range: Option<(f32, f32)>,
) -> Vec<LegendStop> {
    let (Some(_), Some(quantity)) = (l.grid(), MapQuantity::of_layer(l.id)) else {
        return Vec::new();
    };
    let (low, high) = range.map_or((None, None), |(lo, hi)| {
        (Some(f64::from(lo)), Some(f64::from(hi)))
    });
    // A stored ramp or range that cannot be drawn shows the layer's own.
    let Ok((ramp, range)) =
        ramp_and_range(l, ramp, low, high).or_else(|_| ramp_and_range(l, None, None, None))
    else {
        return Vec::new();
    };
    render::legend(ramp, range)
        .into_iter()
        .map(|s| LegendStop {
            fraction: s.fraction,
            colour: s.colour.into(),
            label: map_value(f, quantity, s.value),
        })
        .collect()
}

fn layer_info(l: &Layer, config: &Config, f: &Formatter) -> LayerInfo {
    let s = f.strings;
    let state = config.map.layers.iter().find(|st| st.id.as_str() == l.id);
    let shown = if l.is_base() {
        config.map.base.as_str() == l.id
    } else {
        state.is_some_and(|st| st.shown)
    };
    let stored_ramp = state
        .and_then(|st| st.ramp.as_deref())
        .filter(|id| render::ramp(id).is_some());
    let (available, unavailable_text) = match l.availability {
        catalogue::Availability::Available => (true, None),
        catalogue::Availability::Unavailable { reason } => {
            (false, Some(s.unavailable_reason(reason).to_owned()))
        }
    };
    LayerInfo {
        id: l.id.to_owned(),
        group: group_id(l.group).to_owned(),
        group_title: s.map_group(l.group).to_owned(),
        is_base: l.is_base(),
        title: s.layer_title(l.id).unwrap_or(l.id).to_owned(),
        draw: draw(&l.kind),
        dates: l.date_mode.into(),
        min_zoom: l.min_zoom,
        max_zoom: l.max_zoom,
        credit: s.layer_credit(l.id).unwrap_or_default().to_owned(),
        acknowledgements: s
            .layer_acknowledgements(l.id)
            .into_iter()
            .map(str::to_owned)
            .collect(),
        notice: l
            .not_for_navigation
            .then(|| s.safety(SafetyKey::NotForNavigation).to_owned()),
        available,
        unavailable_text,
        shown: shown && available,
        opacity: state
            .and_then(|st| st.opacity)
            .filter(|o| (0.0..=1.0).contains(o))
            .unwrap_or(l.default_opacity),
        default_opacity: l.default_opacity,
        ramp: stored_ramp.or(l.ramp).map(str::to_owned),
        legend_caption: s.layer_legend(l.id).map(str::to_owned),
        legend: legend_of(l, f, stored_ramp, state.and_then(|st| st.range)),
        depth_reference: s.layer_reference(l.id).map(str::to_owned),
        terms: s.layer_licence(l.id).map(str::to_owned),
        datasets: l
            .grid()
            .map(|g| g.specs().map(|s| s.dataset.to_owned()).collect())
            .unwrap_or_default(),
    }
}

/// Every base map, then every overlay in drawer order, worded in the settings' voice and units,
/// with what the settings show and how.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn map_catalogue(settings_json: String) -> Result<Vec<LayerInfo>, MapError> {
    let (config, _) = load(&settings_json)?;
    let f = formatter(&config, "UTC");
    Ok(catalogue::BASES
        .iter()
        .chain(catalogue::LAYERS.iter())
        .map(|l| layer_info(l, &config, &f))
        .collect())
}

/// How a preset shows one layer.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct PresetLayerInfo {
    /// The layer's id.
    pub id: String,
    /// The opacity the preset gives it; none keeps the layer's own.
    pub opacity: Option<f32>,
}

/// A preset: one of the built-in ones or one the user saved.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct PresetInfo {
    /// The built-in id (`"inshore-flats"`) or the user's own name; what the settings store as
    /// the active preset.
    pub id: String,
    /// The name to show: a built-in preset's in the user's voice, the user's own as typed.
    pub name: String,
    /// A built-in preset rather than one the user saved.
    pub built_in: bool,
    /// The base map a built-in preset starts on; a user's preset keeps the current one.
    pub base: Option<String>,
    /// The layers it shows, every other is hidden.
    pub layers: Vec<PresetLayerInfo>,
}

/// The presets and which one is active.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct MapPresets {
    /// The built-in presets, then the user's own.
    pub presets: Vec<PresetInfo>,
    /// The id of the active preset: the user's own when its name matches the stored one, else the
    /// built-in preset the stored id names, else `inshore-flats`.
    pub active: String,
}

/// The presets, worded in the settings' voice, and the active one.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn map_presets(settings_json: String) -> Result<MapPresets, MapError> {
    let (config, _) = load(&settings_json)?;
    let strings = Strings::new(config.presentation.voice);
    let mut presets: Vec<PresetInfo> = PRESETS
        .iter()
        .map(|p| PresetInfo {
            id: p.id.to_owned(),
            name: strings.preset_name(p.id).unwrap_or(p.id).to_owned(),
            built_in: true,
            base: Some(p.base.to_owned()),
            layers: p
                .layers
                .iter()
                .map(|pl| PresetLayerInfo {
                    id: pl.id.to_owned(),
                    opacity: pl.opacity,
                })
                .collect(),
        })
        .collect();
    presets.extend(config.map.presets.iter().map(|p| {
        PresetInfo {
            id: p.name.clone(),
            name: p.name.clone(),
            built_in: false,
            base: None,
            layers: p
                .layers
                .iter()
                .filter(|st| st.shown)
                .map(|st| PresetLayerInfo {
                    id: st.id.as_str().to_owned(),
                    opacity: st.opacity,
                })
                .collect(),
        }
    }));
    let stored = config.map.active_preset.as_str();
    let active = if config.map.presets.iter().any(|p| p.name == stored) {
        stored.to_owned()
    } else {
        resolve_preset(stored)
            .unwrap_or_else(|_| catalogue::default_preset())
            .id
            .to_owned()
    };
    Ok(MapPresets { presets, active })
}

// ---------------------------------------------------------------------------------------------
// Tiles
// ---------------------------------------------------------------------------------------------

/// The part of a fetched tile to draw when the map is deeper than the layer's tiles: crop this
/// square out of the fetched image and scale it to the tile. Fractions of the fetched tile's
/// side, from its top-left corner.
#[derive(Clone, Copy, Debug, PartialEq, uniffi::Record)]
pub struct TileCrop {
    pub left: f64,
    pub top: f64,
    pub size: f64,
}

/// A tile to fetch for a map tile.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct TileFetch {
    /// The request; its `max_age_s` is the service's usual lifetime.
    pub request: HostRequest,
    /// The tile fetched: the map tile itself, or its ancestor at the layer's deepest zoom.
    pub z: u8,
    pub x: u32,
    pub y: u32,
    /// None when the fetched tile is the map tile; else the part to draw.
    pub crop: Option<TileCrop>,
}

/// The fetch for map tile `z`/`x`/`y` of a tiled layer: none where the layer has nothing (above
/// its shallowest zoom, or a zoom its source has no level for). `date` (`YYYY-MM-DD`, a UTC day)
/// picks a satellite layer's day; none asks for the source's newest, partial picture. Radar is
/// the latest frame (the loop's frames come from [`radar_frames`]). A map tile deeper than the
/// layer's tiles fetches the ancestor with the part to crop.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn tile_request(
    layer: String,
    date: Option<String>,
    z: u8,
    x: u32,
    y: u32,
) -> Result<Option<TileFetch>, MapError> {
    let l = self::layer(&layer)?;
    let service = l
        .tile_service()
        .ok_or_else(|| invalid(format!("`{layer}` is not a tiled layer")))?;
    let key = tile_key(z, x, y)?;
    let date: Option<LocalDate> = date
        .as_deref()
        .map(|d| parse_date(d.trim()).map_err(|e| invalid(e.to_string())))
        .transpose()?;
    if l.min_zoom.is_some_and(|min| z < min) {
        return Ok(None);
    }
    let over = l.max_zoom.map(|max| over_zoom(key, max));
    let fetch_key = over.map_or(key, |o| o.ancestor);
    let Some(url) = l.tile_url(fetch_key, date, 0) else {
        return Ok(None);
    };
    Ok(Some(TileFetch {
        request: get(url, cache_lifetime_s(service, None)),
        z: fetch_key.z,
        x: fetch_key.x,
        y: fetch_key.y,
        crop: over.filter(|o| o.ancestor != key).map(|o| TileCrop {
            left: o.left,
            top: o.top,
            size: o.size,
        }),
    }))
}

/// What a tile response means.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum TileOutcome {
    /// An image to draw.
    Image,
    /// Nothing for that date or place: try the previous day, or draw nothing.
    NoData,
    /// Deeper than the layer's tiles: draw the ancestor ([`tile_request`] does that).
    TooDeep,
    /// Ask again after `after_s` seconds.
    Retry { after_s: u32 },
    /// Give up on the tile.
    Failed,
}

/// Classifies a tile response of `layer`. `attempt` counts the requests already made for the
/// tile (0 on the first response), so that a bad answer is retried a bounded number of times. An
/// image needs status 200, an image content type and a body: the chart service answers failures
/// with status 200 and an HTML or JSON page.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn classify_tile(
    layer: String,
    status: u16,
    content_type: Option<String>,
    body_len: u64,
    attempt: u32,
) -> Result<TileOutcome, MapError> {
    let service = self::layer(&layer)?
        .tile_service()
        .ok_or_else(|| invalid(format!("`{layer}` is not a tiled layer")))?;
    let body_len = usize::try_from(body_len).unwrap_or(usize::MAX);
    Ok(
        match classify(service, status, content_type.as_deref(), body_len, attempt) {
            CoreTileOutcome::Image => TileOutcome::Image,
            CoreTileOutcome::NoData => TileOutcome::NoData,
            CoreTileOutcome::TooDeep => TileOutcome::TooDeep,
            CoreTileOutcome::Retry { after_s } => TileOutcome::Retry { after_s },
            CoreTileOutcome::Failed => TileOutcome::Failed,
        },
    )
}

/// A day for a satellite layer: the `YYYY-MM-DD` the requests take and its label ("Nov 7").
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct MapDate {
    pub date: String,
    pub label: String,
}

fn map_date(d: LocalDate) -> MapDate {
    MapDate {
        date: format!("{:04}-{:02}-{:02}", d.year, d.month, d.day),
        label: neutral_formatter().date(d),
    }
}

/// The day a satellite layer opens on: yesterday in UTC, since today's picture is partial.
#[uniffi::export]
pub fn gibs_default_date(now: i64) -> MapDate {
    map_date(tiles::gibs_default_date(UnixTime(now)))
}

/// The day before `date` (`YYYY-MM-DD`), for stepping back past a cloudy day.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn previous_date(date: String) -> Result<MapDate, MapError> {
    let d = parse_date(date.trim()).map_err(|e| invalid(e.to_string()))?;
    Ok(map_date(previous_day(d)))
}

/// The side of every tile the map draws, in pixels.
#[uniffi::export]
pub fn map_tile_size() -> u32 {
    TILE_PX
}

// ---------------------------------------------------------------------------------------------
// Gridded layers: requests, the store, rendering, arrows and the point inspector
// ---------------------------------------------------------------------------------------------

/// The most values a [`GridStore`] keeps: two million (about 8 MB of `f32`, axes included). A
/// block holds at most 10,000 cells per variable, so this is some two hundred blocks, several
/// screens of every grid layer.
pub const MAX_STORE_VALUES: usize = 2_000_000;

/// The most blocks a [`GridStore`] keeps, whatever their size.
pub const MAX_STORE_BLOCKS: usize = 512;

/// The most block requests one view asks for, nearest to the view's centre first. A phone at a
/// layer's shallowest zoom needs about a dozen; a larger screen gets its middle first.
pub const MAX_REQUESTS_PER_VIEW: usize = 24;

/// How long a block asked for at the newest time (`(last)`) is kept before it is asked again:
/// the newest time moves on.
const LATEST_MAX_AGE_S: i64 = 3 * 3_600;

/// How long the app may reuse a block response when the server says nothing: the newest time
/// moves on, a dated block keeps (a revised day is refreshed by the store).
const LATEST_RESPONSE_AGE_S: u64 = 3_600;
const DATED_RESPONSE_AGE_S: u64 = 86_400;

/// The most arrows one call returns.
const MAX_ARROWS: usize = 4_000;

/// Days of times a daily layer's time axis lists, and hours an hourly one's.
const DAILY_AXIS_DAYS: i64 = 14;
const HOURLY_AXIS_DAYS: i64 = 2;

/// One block request for a view, with the key to hand its response back under.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct GridRequest {
    /// Pass it to [`GridStore::add_grid`] with the response.
    pub key: String,
    /// The server, for [`GridPacer`]: `pfeg`, `coastwatch` or `glerl`.
    pub node: String,
    pub request: HostRequest,
}

/// A current arrow to draw.
#[derive(Clone, Copy, Debug, PartialEq, uniffi::Record)]
pub struct MapArrow {
    pub lat: f64,
    pub lon: f64,
    /// The direction the water goes toward, degrees clockwise from true north: the arrow's
    /// rotation.
    pub toward_deg: f32,
    /// The speed's place in the layer's range, 0 to 1: the arrow's length.
    pub fraction: f32,
    /// The ramp's colour for the speed.
    pub colour: MapColour,
}

/// One layer's value at a point, worded.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct InspectorRow {
    /// The layer's id.
    pub layer: String,
    /// The layer's title: "Sea surface temperature".
    pub label: String,
    /// The value in the user's unit ("61 °F", "0.3 kn toward NE 45°"), or that there is none.
    pub value_text: String,
    /// The layer's credit (safety text).
    pub source: String,
    /// When the value is valid: "Nov 7", "Nov 8, 4:00 AM".
    pub time_text: String,
}

/// One time a gridded layer has, with its label.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct LayerTime {
    /// Unix seconds, as [`GridStore::grid_requests`] takes it.
    pub time: i64,
    /// "Nov 7" for a daily layer (its UTC day), "Nov 8, 4:00 AM" for an hourly one.
    pub label: String,
}

/// The node's name as requests and the pacer use it.
fn node_name(node: GridHost) -> String {
    serde_json::to_value(node)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// A grid layer and the dataset asked for: the named one, else the primary.
fn grid_spec<'a>(
    l: &'a Layer,
    dataset: Option<&str>,
) -> Result<(&'a GridLayer, &'a GridSpec), MapError> {
    let grid = l
        .grid()
        .ok_or_else(|| invalid(format!("`{}` is not a gridded layer", l.id)))?;
    let spec = match dataset {
        Some(d) => grid
            .spec(d.trim())
            .ok_or_else(|| invalid(format!("`{d}` is not a dataset of `{}`", l.id)))?,
        None => &grid.primary,
    };
    Ok((grid, spec))
}

fn erddap_error(e: &ErddapError) -> MapError {
    match e {
        ErddapError::NoData => MapError::NoData,
        ErddapError::Temporary => MapError::RetryLater {
            after_s: u32::try_from(FIRST_PAUSE_S).unwrap_or(60),
        },
        ErddapError::RateLimited { retry_after_s } => MapError::RetryLater {
            after_s: retry_after_s.unwrap_or(u32::try_from(FIRST_PAUSE_S).unwrap_or(60)),
        },
        other => invalid(other.to_string()),
    }
}

/// The body of an ERDDAP response, or what went wrong.
fn erddap_body(response: HostResponse) -> Result<Vec<u8>, MapError> {
    let fetched = fetched(response).map_err(|e| invalid(e.to_string()))?;
    {
        let r = fetched.as_response();
        erddap::check_response(&r).map_err(|e| erddap_error(&e))?;
    }
    Ok(fetched.body)
}

/// What a stored block is: the layer, dataset and time asked for, and the block.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct BlockId {
    layer: String,
    dataset: String,
    /// Unix seconds, or none for the newest time.
    time: Option<i64>,
    block: Block,
}

impl BlockId {
    /// `sst/jplMURSST41/last/40/-74/1`: layer, dataset, time (Unix seconds or `last`), south,
    /// west, stride.
    fn key(&self) -> String {
        let time = self
            .time
            .map_or_else(|| "last".to_owned(), |t| t.to_string());
        format!(
            "{}/{}/{}/{}/{}/{}",
            self.layer, self.dataset, time, self.block.south, self.block.west, self.block.stride
        )
    }

    fn parse(key: &str) -> Result<(Self, &'static Layer, &'static GridSpec), MapError> {
        let bad = || invalid(format!("`{key}` is not a grid key"));
        let parts: Vec<&str> = key.split('/').collect();
        let [layer_id, dataset, time, south, west, stride] = parts.as_slice() else {
            return Err(bad());
        };
        let l = layer(layer_id)?;
        let (_, spec) = grid_spec(l, Some(dataset))?;
        let time = match *time {
            "last" => None,
            t => Some(t.parse::<i64>().map_err(|_| bad())?),
        };
        let block = Block {
            south: south.parse().map_err(|_| bad())?,
            west: west.parse().map_err(|_| bad())?,
            stride: stride.parse().map_err(|_| bad())?,
        };
        if block.stride == 0 {
            return Err(bad());
        }
        let id = Self {
            layer: l.id.to_owned(),
            dataset: spec.dataset.to_owned(),
            time,
            block,
        };
        Ok((id, l, spec))
    }
}

/// A stored block: one grid per variable of its dataset.
#[derive(Debug)]
struct Stored {
    grids: Vec<Arc<Grid>>,
    values: usize,
    fetched: UnixTime,
    /// When it was last used, by the store's counter.
    used: u64,
}

/// A grid's extent, reaching one cell spacing beyond its outermost centres.
fn extent(grid: &Grid) -> Option<Bbox> {
    let pad = |axis: &[f64]| match axis {
        [_] => grid.source.native_step_deg.abs(),
        [first, .., last] => (last - first).abs() / (axis.len() - 1) as f64,
        [] => 0.0,
    };
    let (lat_pad, lon_pad) = (pad(&grid.lats), pad(&grid.lons));
    Some(Bbox {
        west: grid.lons.first()? - lon_pad,
        south: grid.lats.first()? - lat_pad,
        east: grid.lons.last()? + lon_pad,
        north: grid.lats.last()? + lat_pad,
    })
}

const fn overlaps(a: Bbox, b: Bbox) -> bool {
    a.west <= b.east && b.west <= a.east && a.south <= b.north && b.south <= a.north
}

/// The store's state, behind the mutex.
#[derive(Debug, Default)]
struct Blocks {
    stored: BTreeMap<BlockId, Stored>,
    /// Per layer, the dataset and time the last view asked for: what is drawn and inspected.
    selection: BTreeMap<String, (String, Option<i64>)>,
    values: usize,
    clock: u64,
}

impl Blocks {
    fn tick(&mut self) -> u64 {
        self.clock += 1;
        self.clock
    }

    /// The selected blocks of `layer` whose extent overlaps `area` (any, without one), finest
    /// stride first, each as its grids; marks them used.
    fn selected(&mut self, layer: &str, area: Option<Bbox>) -> Vec<Vec<Arc<Grid>>> {
        let Some((dataset, time)) = self.selection.get(layer).cloned() else {
            return Vec::new();
        };
        let now = self.tick();
        let mut found: Vec<(u32, Vec<Arc<Grid>>)> = Vec::new();
        for (id, stored) in &mut self.stored {
            if id.layer != layer || id.dataset != dataset || id.time != time {
                continue;
            }
            let hit = match (area, stored.grids.first().and_then(|g| extent(g))) {
                (None, _) => true,
                (Some(a), Some(e)) => overlaps(a, e),
                (Some(_), None) => false,
            };
            if hit {
                stored.used = now;
                found.push((id.block.stride, stored.grids.clone()));
            }
        }
        found.sort_by_key(|(stride, _)| *stride);
        found.into_iter().map(|(_, g)| g).collect()
    }

    fn insert(&mut self, id: BlockId, grids: Vec<Grid>, fetched: UnixTime) -> Result<(), MapError> {
        let values: usize = grids
            .iter()
            .map(|g| g.values.len() + g.lats.len() + g.lons.len())
            .sum();
        if values > MAX_STORE_VALUES {
            return Err(invalid("the block is larger than the store"));
        }
        if let Some(old) = self.stored.remove(&id) {
            self.values -= old.values;
        }
        // Make room: the least recently used blocks go first.
        while self.values + values > MAX_STORE_VALUES || self.stored.len() >= MAX_STORE_BLOCKS {
            let Some(oldest) = self
                .stored
                .iter()
                .min_by_key(|(_, s)| s.used)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            if let Some(gone) = self.stored.remove(&oldest) {
                self.values -= gone.values;
            }
        }
        let used = self.tick();
        self.values += values;
        self.stored.insert(
            id,
            Stored {
                grids: grids.into_iter().map(Arc::new).collect(),
                values,
                fetched,
                used,
            },
        );
        Ok(())
    }
}

/// Gridded layers held on the device: the blocks of ERDDAP and GLERL grids fetched for the views,
/// rendered into tiles, drawn as arrows and read by the point inspector.
///
/// Shared across threads: one mutex guards the blocks, held only to look up, insert or copy out
/// a few `Arc`s; parsing and rendering happen outside it. Bounded: at most
/// [`MAX_STORE_VALUES`] values in at most [`MAX_STORE_BLOCKS`] blocks, the least recently used
/// dropped first. Nothing is written to disk; a later dataset (the NCEI elevation model) is one
/// more kind of block under the same bound.
#[derive(Debug, Default, uniffi::Object)]
pub struct GridStore {
    blocks: Mutex<Blocks>,
}

impl GridStore {
    fn lock(&self) -> MutexGuard<'_, Blocks> {
        // A panic while the lock was held leaves whole entries; keep using them.
        self.blocks.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[uniffi::export]
impl GridStore {
    /// An empty store.
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// The block requests a view of `layer` needs that the store does not already hold fresh,
    /// nearest to the view's centre first, at most [`MAX_REQUESTS_PER_VIEW`]. `dataset` picks
    /// the fallback dataset (none: the primary); `time` is one of the layer's times (none: the
    /// newest). The view is `west`, `south`, `east`, `north` in degrees at `zoom`; above the
    /// layer's shallowest zoom it needs nothing. The view's dataset and time become what the
    /// store draws and inspects for the layer.
    #[allow(clippy::too_many_arguments)] // a view is four edges and a zoom
    #[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
    pub fn grid_requests(
        &self,
        layer: String,
        dataset: Option<String>,
        time: Option<i64>,
        west: f64,
        south: f64,
        east: f64,
        north: f64,
        zoom: f64,
        now: i64,
    ) -> Result<Vec<GridRequest>, MapError> {
        let l = self::layer(&layer)?;
        let (_, spec) = grid_spec(l, dataset.as_deref())?;
        let source = spec.source();
        let view = Bbox {
            west,
            south,
            east,
            north,
        };
        let blocks = if l.min_zoom.is_some_and(|min| zoom < f64::from(min)) {
            Vec::new()
        } else {
            grid::blocks_for_view(&source, view, zoom)
        };
        let now = UnixTime(now);
        let grid_time = time.map_or(GridTime::Last, |t| GridTime::At(UnixTime(t)));
        let max_age = if time.is_some() {
            DATED_RESPONSE_AGE_S
        } else {
            LATEST_RESPONSE_AGE_S
        };
        let mut out = Vec::new();
        let mut store = self.lock();
        store
            .selection
            .insert(l.id.to_owned(), (spec.dataset.to_owned(), time));
        for block in blocks {
            if out.len() >= MAX_REQUESTS_PER_VIEW {
                break;
            }
            let id = BlockId {
                layer: l.id.to_owned(),
                dataset: spec.dataset.to_owned(),
                time,
                block,
            };
            let fresh = store.stored.get(&id).is_some_and(|s| match time {
                Some(t) => grid::block_is_fresh(spec.dataset, UnixTime(t), s.fetched, now),
                None => (0..LATEST_MAX_AGE_S).contains(&(now.0 - s.fetched.0)),
            });
            if fresh {
                continue;
            }
            let request = erddap::block_request(&source, grid_time, block)
                .map_err(|e| invalid(e.to_string()))?;
            out.push(GridRequest {
                key: id.key(),
                node: node_name(request.node),
                request: get(request.url, max_age),
            });
        }
        Ok(out)
    }

    /// Stores the response to the request under `key` (from [`GridStore::grid_requests`]),
    /// fetched at `now`. ERDDAP's failures come back as errors: [`MapError::NoData`] for a time
    /// or box without data, [`MapError::RetryLater`] for a busy server; record the status with
    /// the [`GridPacer`] too.
    #[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
    pub fn add_grid(&self, key: String, response: HostResponse, now: i64) -> Result<(), MapError> {
        let (id, l, spec) = BlockId::parse(&key)?;
        let body = erddap_body(response)?;
        let source = spec.source();
        let grids = if l.grid().map(|g| g.draw) == Some(GridDraw::Arrows) {
            let c = erddap::parse_currents(&source, &body).map_err(|e| erddap_error(&e))?;
            vec![c.u, c.v]
        } else {
            vec![erddap::parse_grid(&source, &body).map_err(|e| erddap_error(&e))?]
        };
        self.lock().insert(id, grids, UnixTime(now))
    }

    /// A 256-pixel PNG tile of a raster grid layer at `z`/`x`/`y`, coloured with `ramp` (none:
    /// the layer's own) over `low`..`high` (canonical units; none: the layer's own range); none
    /// when the store holds nothing of the layer's view there. Transparent over land, cloud and
    /// outside the data.
    #[allow(clippy::too_many_arguments)] // a tile is three numbers and a ramp is three
    #[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
    pub fn render_tile(
        &self,
        layer: String,
        ramp: Option<String>,
        low: Option<f64>,
        high: Option<f64>,
        z: u8,
        x: u32,
        y: u32,
    ) -> Result<Option<Vec<u8>>, MapError> {
        let l = self::layer(&layer)?;
        if l.grid().map(|g| g.draw) != Some(GridDraw::Raster) {
            return Err(invalid(format!("`{layer}` is not drawn as a raster")));
        }
        let (ramp, range) = ramp_and_range(l, ramp.as_deref(), low, high)?;
        let key = tile_key(z, x, y)?;
        let shared = self.lock().selected(l.id, Some(tiles::tile_bbox(key)));
        if shared.is_empty() {
            return Ok(None);
        }
        // Copied out of the lock: rendering holds no lock.
        let grids: Vec<Grid> = shared
            .iter()
            .filter_map(|g| g.first().map(|g| Grid::clone(g)))
            .collect();
        render::render_tile(&grids, ramp, range, key, TILE_PX)
            .map(Some)
            .map_err(|e| invalid(e.to_string()))
    }

    /// The current arrows of an arrow layer in a view (`west`, `south`, `east`, `north`, at
    /// `zoom`), thinned to about one per 32 points, coloured with `ramp` over `low`..`high` as
    /// [`GridStore::render_tile`]; at most 4,000.
    #[allow(clippy::too_many_arguments)] // a view is five numbers and a ramp is three
    #[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
    pub fn arrows(
        &self,
        layer: String,
        ramp: Option<String>,
        low: Option<f64>,
        high: Option<f64>,
        west: f64,
        south: f64,
        east: f64,
        north: f64,
        zoom: f64,
    ) -> Result<Vec<MapArrow>, MapError> {
        let l = self::layer(&layer)?;
        if l.grid().map(|g| g.draw) != Some(GridDraw::Arrows) {
            return Err(invalid(format!("`{layer}` is not drawn as arrows")));
        }
        let (ramp, range) = ramp_and_range(l, ramp.as_deref(), low, high)?;
        let view = Bbox {
            west,
            south,
            east,
            north,
        };
        let shared = self.lock().selected(l.id, Some(view));
        let mut out = Vec::new();
        for grids in &shared {
            let [u, v] = grids.as_slice() else {
                continue;
            };
            for a in render::arrows(u, v, view, zoom) {
                if out.len() >= MAX_ARROWS {
                    return Ok(out);
                }
                let speed = f64::from(a.speed_ms);
                out.push(MapArrow {
                    lat: a.at.lat,
                    lon: a.at.lon,
                    toward_deg: a.toward_deg,
                    fraction: range.position(speed).unwrap_or(0.0) as f32,
                    colour: ramp
                        .colour(range, speed)
                        .unwrap_or(render::Rgba::TRANSPARENT)
                        .into(),
                });
            }
        }
        Ok(out)
    }

    /// Each of `layers`' values at `lat`, `lon` from the blocks the store holds for the layer's
    /// view, in the order given, worded for the settings: the same value the tile shows there
    /// (one sampling rule). A layer that is not gridded, or has no block over the point, has no
    /// row; a block over land or cloud gives "No value here". `time_zone` (IANA) words hourly
    /// times.
    #[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
    pub fn inspect(
        &self,
        lat: f64,
        lon: f64,
        layers: Vec<String>,
        settings_json: String,
        time_zone: String,
    ) -> Result<Vec<InspectorRow>, MapError> {
        let (config, _) = load(&settings_json)?;
        let f = formatter(&config, &time_zone);
        let at = GeoPoint { lat, lon };
        let point = Bbox {
            west: lon,
            south: lat,
            east: lon,
            north: lat,
        };
        let mut rows = Vec::new();
        for id in &layers {
            let Some(l) = catalogue::find(id).filter(|l| l.grid().is_some()) else {
                continue;
            };
            let shared = self.lock().selected(l.id, Some(point));
            let Some(first) = shared.first().and_then(|g| g.first()) else {
                continue;
            };
            let arrows = l.grid().map(|g| g.draw) == Some(GridDraw::Arrows);
            let found = shared.iter().find_map(|grids| {
                let value = if arrows {
                    let (u, v) = (grids.first()?, grids.get(1)?);
                    let (east, north) = (
                        f64::from(u.sample_bilinear(at)?),
                        f64::from(v.sample_bilinear(at)?),
                    );
                    let toward = east.atan2(north).to_degrees().rem_euclid(360.0);
                    map_current(&f, east.hypot(north), toward)
                } else {
                    let quantity = MapQuantity::of_layer(l.id)?;
                    let g = grids.first()?;
                    map_value(&f, quantity, f64::from(g.sample_bilinear(at)?))
                };
                Some((value, grids.first()?.time))
            });
            let (value_text, time) =
                found.unwrap_or_else(|| (f.strings.get(Key::MapNoValue).to_owned(), first.time));
            rows.push(InspectorRow {
                layer: l.id.to_owned(),
                label: f.strings.layer_title(l.id).unwrap_or(l.id).to_owned(),
                value_text,
                source: f.strings.layer_credit(l.id).unwrap_or_default().to_owned(),
                time_text: map_time(&f, time, l.date_mode),
            });
        }
        Ok(rows)
    }

    /// Forgets every block and view.
    pub fn clear(&self) {
        *self.lock() = Blocks::default();
    }

    /// How many blocks the store holds.
    pub fn block_count(&self) -> u32 {
        u32::try_from(self.lock().stored.len()).unwrap_or(u32::MAX)
    }

    /// How many values the store holds, axes included; at most [`MAX_STORE_VALUES`].
    pub fn value_count(&self) -> u64 {
        self.lock().values as u64
    }
}

/// The request for the times a gridded layer's dataset holds (`dataset`: none for the primary):
/// the last two weeks of a daily layer, the last two days of an hourly one, before `now`.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn time_axis_request(
    layer: String,
    dataset: Option<String>,
    now: i64,
) -> Result<HostRequest, MapError> {
    let l = self::layer(&layer)?;
    let (_, spec) = grid_spec(l, dataset.as_deref())?;
    let days = if l.date_mode == DateMode::Hourly {
        HOURLY_AXIS_DAYS
    } else {
        DAILY_AXIS_DAYS
    };
    let since = UnixTime(now.saturating_sub(days * 86_400)).local_date(UtcOffset(0));
    let request =
        erddap::time_axis_request(&spec.source(), since).map_err(|e| invalid(e.to_string()))?;
    Ok(get(request.url, LATEST_RESPONSE_AGE_S))
}

/// The times in a response to [`time_axis_request`], oldest first, labelled for the settings
/// (daily layers by their UTC day; hourly ones in `time_zone`, an IANA name).
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn parse_time_axis(
    layer: String,
    dataset: Option<String>,
    response: HostResponse,
    settings_json: String,
    time_zone: String,
) -> Result<Vec<LayerTime>, MapError> {
    let l = self::layer(&layer)?;
    grid_spec(l, dataset.as_deref())?;
    let (config, _) = load(&settings_json)?;
    let f = formatter(&config, &time_zone);
    let body = erddap_body(response)?;
    let axis = erddap::parse_time_axis(&body).map_err(|e| erddap_error(&e))?;
    Ok(axis
        .times()
        .iter()
        .map(|&t| LayerTime {
            time: t.0,
            label: map_time(&f, t, l.date_mode),
        })
        .collect())
}

// ---------------------------------------------------------------------------------------------
// Pacing ERDDAP requests
// ---------------------------------------------------------------------------------------------

fn node_of(name: &str) -> Result<GridHost, MapError> {
    serde_json::from_value(serde_json::Value::String(name.trim().to_owned()))
        .map_err(|_| invalid(format!("`{name}` is not a grid server")))
}

/// Whether a request may go to a grid server now.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Record)]
pub struct PaceDecision {
    /// Send it: the request has been counted.
    pub send: bool,
    /// Otherwise, seconds until a request may go.
    pub wait_s: u32,
}

/// Keeps requests to each ERDDAP server polite: at most 10 a minute, and a pause after "busy"
/// (429 or 503) that doubles with each further one up to 15 minutes, or what the server's
/// `Retry-After` asks, up to an hour. The app keeps one across launches as JSON
/// ([`GridPacer::to_json`], [`GridPacer::from_json`]); it holds no coordinates.
#[derive(Debug, Default, uniffi::Object)]
pub struct GridPacer {
    pacer: Mutex<ErddapPacer>,
}

impl GridPacer {
    fn lock(&self) -> MutexGuard<'_, ErddapPacer> {
        self.pacer.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[uniffi::export]
impl GridPacer {
    /// A pacer with no history.
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// A pacer from its saved JSON.
    #[uniffi::constructor]
    #[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
    pub fn from_json(json: String) -> Result<Arc<Self>, MapError> {
        let pacer: ErddapPacer = serde_json::from_str(&json)
            .map_err(|e| invalid(format!("the saved pacing cannot be read: {e}")))?;
        Ok(Arc::new(Self {
            pacer: Mutex::new(pacer),
        }))
    }

    /// The pacer as JSON, to save.
    pub fn to_json(&self) -> String {
        // A map of plain records always serializes.
        serde_json::to_string(&*self.lock()).unwrap_or_else(|_| "{}".to_owned())
    }

    /// Whether a request to `node` (a [`GridRequest`]'s) may be sent at `now`; when it may, it
    /// is counted as sent.
    #[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
    pub fn may_send(&self, node: String, now: i64) -> Result<PaceDecision, MapError> {
        let node = node_of(&node)?;
        let now = UnixTime(now);
        Ok(match self.lock().try_send(node, now) {
            Ok(()) => PaceDecision {
                send: true,
                wait_s: 0,
            },
            Err(wait) => PaceDecision {
                send: false,
                wait_s: wait.after_s(now),
            },
        })
    }

    /// Records the status of a response from `node` at `now`, with the `Retry-After` it sent in
    /// seconds: 429 and 503 pause the server, an answer from it resets the pause.
    #[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
    pub fn record(
        &self,
        node: String,
        status: u16,
        retry_after_s: Option<u32>,
        now: i64,
    ) -> Result<(), MapError> {
        let node = node_of(&node)?;
        let mut response = Response::new(status, &[]);
        response.retry_after = retry_after_s;
        self.lock().on_response(node, &response, UnixTime(now));
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------
// Measuring
// ---------------------------------------------------------------------------------------------

/// A point on the map, in degrees.
#[derive(Clone, Copy, Debug, PartialEq, uniffi::Record)]
pub struct MapPoint {
    pub lat: f64,
    pub lon: f64,
}

impl MapPoint {
    fn geo(self) -> Result<GeoPoint, MapError> {
        let ok = self.lat.is_finite()
            && self.lon.is_finite()
            && (-90.0..=90.0).contains(&self.lat)
            && (-180.0..=180.0).contains(&self.lon);
        if !ok {
            return Err(invalid("a point needs a latitude and a longitude in range"));
        }
        Ok(GeoPoint {
            lat: self.lat,
            lon: self.lon,
        })
    }
}

/// A measured line, worded.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct MeasureText {
    /// The great-circle distance in the user's unit: "2.4 mi".
    pub distance: String,
    /// The initial bearing from the first point, true: "NE 45°".
    pub bearing: String,
    /// Both: "2.4 mi at NE 45°".
    pub line: String,
    /// "Not for navigation." (safety text): measuring is for planning.
    pub notice: String,
}

/// The distance and bearing from `from` to `to`, in the settings' units and voice.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn measure(
    from: MapPoint,
    to: MapPoint,
    settings_json: String,
) -> Result<MeasureText, MapError> {
    let (a, b) = (from.geo()?, to.geo()?);
    let (config, _) = load(&settings_json)?;
    let f = formatter(&config, "UTC");
    let (distance, bearing, line) =
        map_measure(&f, tiles::haversine_m(a, b), tiles::bearing_deg(a, b));
    Ok(MeasureText {
        distance,
        bearing,
        line,
        notice: f.strings.safety(SafetyKey::NotForNavigation).to_owned(),
    })
}

// ---------------------------------------------------------------------------------------------
// Radar
// ---------------------------------------------------------------------------------------------

/// How long a radar frame's tiles keep: a frame names its composite's time and never changes.
const RADAR_FRAME_AGE_S: u64 = 86_400;

/// One frame of the radar loop.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct RadarFrame {
    /// Minutes before the latest frame.
    pub minutes_ago: u32,
    /// When the composite was valid, Unix seconds.
    pub valid: i64,
    /// The frame's `{z}/{x}/{y}` tile URL template.
    pub tile_template: String,
    /// The frame's time in the settings' clock: "4:15 PM".
    pub label: String,
}

/// Whether the NEXRAD composite covers a point (the contiguous US).
#[uniffi::export]
pub fn radar_covers(lat: f64, lon: f64) -> bool {
    radar::covers(lat, lon)
}

/// The request for the latest composite's metadata.
#[uniffi::export]
pub fn radar_meta_request() -> HostRequest {
    let mut request = get(radar::meta_url(), 0);
    request.fresh = true;
    request
}

/// The latest composite's valid time (Unix seconds) from the metadata body.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn radar_latest_valid(body: Vec<u8>) -> Result<i64, MapError> {
    radar::parse_valid(&body)
        .map(|t| t.0)
        .map_err(|e| invalid(format!("the radar metadata cannot be read: {e}")))
}

/// The loop ending at `latest`, oldest first, each frame's tiles named by its composite's time,
/// labelled in the settings' clock in `time_zone` (an IANA name).
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn radar_frames(
    latest: i64,
    settings_json: String,
    time_zone: String,
) -> Result<Vec<RadarFrame>, MapError> {
    let (config, _) = load(&settings_json)?;
    let f = formatter(&config, &time_zone);
    Ok(radar::frames(UnixTime(latest))
        .into_iter()
        .map(|(minutes_ago, valid)| RadarFrame {
            minutes_ago,
            valid: valid.0,
            tile_template: radar::archive_tile_template(valid),
            label: f.clock(valid),
        })
        .collect())
}

/// The request for tile `z`/`x`/`y` of the frame valid at `valid`; none outside the radar
/// layer's zooms.
#[uniffi::export]
pub fn radar_frame_request(
    valid: i64,
    z: u8,
    x: u32,
    y: u32,
) -> Result<Option<HostRequest>, MapError> {
    let key = tile_key(z, x, y)?;
    let l = self::layer("radar")?;
    let inside = l.min_zoom.is_none_or(|min| z >= min) && l.max_zoom.is_none_or(|max| z <= max);
    if !inside {
        return Ok(None);
    }
    let url = radar::archive_tile_template(UnixTime(valid))
        .replace("{z}", &key.z.to_string())
        .replace("{x}", &key.x.to_string())
        .replace("{y}", &key.y.to_string());
    Ok(Some(get(url, RADAR_FRAME_AGE_S)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_keys_outside_their_level_are_refused() {
        assert!(tile_key(0, 0, 0).is_ok());
        assert!(tile_key(1, 2, 0).is_err());
        assert!(tile_key(25, 0, 0).is_err());
    }

    #[test]
    fn a_stored_range_narrows_the_layers_own_and_a_bad_one_is_refused() {
        let sst = layer("sst").unwrap();
        let (ramp, range) = ramp_and_range(sst, None, Some(5.0), Some(20.0)).unwrap();
        assert_eq!(ramp.id, "thermal");
        assert_eq!(range, RampRange::linear(5.0, 20.0));
        assert!(ramp_and_range(sst, None, Some(20.0), Some(5.0)).is_err());
        assert!(ramp_and_range(sst, Some("plaid"), None, None).is_err());
        assert_eq!(
            ramp_and_range(sst, Some("diverging"), None, None)
                .unwrap()
                .0
                .id,
            "diverging"
        );
        assert!(ramp_and_range(layer("radar").unwrap(), None, None, None).is_err());
    }

    #[test]
    fn the_neutral_formatter_writes_dates_without_a_voice() {
        assert_eq!(
            previous_date("2026-03-01".into()).unwrap().date,
            "2026-02-28"
        );
        assert_eq!(previous_date("2026-03-01".into()).unwrap().label, "Feb 28");
    }
}
