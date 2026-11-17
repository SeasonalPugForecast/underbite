//! The map across the FFI: the layer catalogue, tile and grid requests, rendering, the point
//! inspector and measuring.
//!
//! Engine-neutral: the app draws what these calls describe. Every word and value the map shows is
//! written by the core's view (titles, credits, legend labels, inspector values, distances);
//! numbers cross only where the app draws with them (coordinates, opacity, colours, fractions of
//! a tile or a legend bar). Times are Unix seconds; dates are `YYYY-MM-DD` UTC days.

use underbite_config::schema::Config;
use underbite_core::map::catalogue::{
    self, AppleBase, DateMode, GridDraw, Group, Layer, LayerKind, PRESETS, resolve_preset,
};
use underbite_core::map::render::{self, Ramp, RampRange};
use underbite_core::map::tiles::{
    MAX_ZOOM, TILE_PX, TileOutcome as CoreTileOutcome, cache_lifetime_s, classify, over_zoom,
    previous_day,
};
use underbite_core::map::{TileKey, tiles};
use underbite_core::time::{LocalDate, UnixTime, parse_date};
use underbite_core::view::format::Formatter;
use underbite_core::view::strings::{MapQuantity, SafetyKey, Strings, map_value};

use crate::fetch::{HostMethod, HostRequest};
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
