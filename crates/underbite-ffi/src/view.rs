//! The spot view, fully formatted for the app.
//!
//! The view is `underbite-core`'s `SpotView`, passed through as JSON without reshaping. The
//! settings become a `ViewInput` here exactly as the command line does it.

use serde::Serialize;
use underbite_config::rivers::RiverRange;
use underbite_config::schema::Config;
use underbite_config::sources::{SourceGroup, SourcesConfig};
use underbite_core::model::{Measurement, SpotSnapshot, WaterType};
use underbite_core::provenance::SourceId;
use underbite_core::time::UnixTime;
use underbite_core::view::format::Formatter;
use underbite_core::view::{
    Card, CardId, MeasurementLimit, RiverRanges, SolunarSettings, SpotView, ViewInput, build_view,
};

use crate::settings::{SettingsError, load};

/// Why a view could not be built.
#[derive(Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum ViewError {
    /// The settings could not be read.
    #[error("settings: {message}")]
    Settings {
        /// Why.
        message: String,
    },
    /// A cached snapshot or a spot could not be read.
    #[error("invalid {what}: {message}")]
    Invalid {
        /// What was being read.
        what: String,
        /// The parser's message.
        message: String,
    },
}

impl From<SettingsError> for ViewError {
    fn from(e: SettingsError) -> Self {
        let SettingsError::Invalid { message } = e;
        Self::Settings { message }
    }
}

fn invalid(what: &str, message: &impl ToString) -> ViewError {
    ViewError::Invalid {
        what: what.to_owned(),
        message: message.to_string(),
    }
}

/// A spot as the app holds it.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct SpotInfo {
    /// The spot's id.
    pub id: String,
    /// The name the user gave it.
    pub name: String,
    /// Latitude, decimal degrees.
    pub latitude: f64,
    /// Longitude, decimal degrees.
    pub longitude: f64,
    /// The kind of water the user chose, as the settings and snapshots write it
    /// (`"river_stream"`, `"tidal_river_estuary"`, ...).
    pub water_type: String,
    /// The IANA time zone, empty when not known yet.
    pub time_zone: String,
}

/// One spot of the list, as the app holds it.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct SpotEntry {
    /// The spot.
    pub spot: SpotInfo,
    /// The cached snapshot (a fetch's `snapshot_json`), if any.
    pub snapshot_json: Option<String>,
    /// When the last fetch succeeded (Unix seconds).
    pub last_success: Option<i64>,
    /// The last fetch's error, if it failed.
    pub error: Option<String>,
    /// A fetch is running.
    pub refreshing: bool,
}

/// How an entry's data stands, for the spot list and the page header.
#[derive(Debug, Serialize)]
struct Status {
    last_success: Option<i64>,
    /// "updated 6 min ago", written by the core.
    updated: Option<String>,
    error: Option<String>,
    refreshing: bool,
}

/// One row of the spot list.
#[derive(Debug, Serialize)]
struct SpotSummary {
    id: String,
    name: String,
    water_type: WaterType,
    /// "River or stream", ...
    water_type_label: String,
    status: Status,
}

/// What `view_model` returns: the list, which spot is shown and its page, unchanged from the
/// core. A spot with no snapshot yet has no page.
#[derive(Debug, Serialize)]
struct Page {
    active: u32,
    spots: Vec<SpotSummary>,
    view: Option<SpotView>,
}

fn water_type(text: &str) -> Result<WaterType, ViewError> {
    serde_json::from_value(serde_json::Value::String(text.to_owned()))
        .map_err(|_| invalid("water type", &format!("`{text}` is not a water type")))
}

/// The snapshot with the spot as the user has it now: a rename or a changed water type shows
/// at once, before the next fetch.
fn with_spot(mut snapshot: SpotSnapshot, info: &SpotInfo) -> Result<SpotSnapshot, ViewError> {
    snapshot.spot.name.clone_from(&info.name);
    snapshot.spot.water_type = water_type(&info.water_type)?;
    if !info.time_zone.is_empty() {
        snapshot.spot.time_zone.clone_from(&info.time_zone);
    }
    Ok(snapshot)
}

/// The page of the active spot and the list of all, as JSON: the core's `SpotView` for
/// `active` (cards in compact and full form, the day timeline and the notices) with every value
/// formatted for the settings' units, clock and voice, and for each spot its name and status.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn view_model(
    spots: Vec<SpotEntry>,
    active: u32,
    settings_json: String,
    now: i64,
) -> Result<String, ViewError> {
    let (config, _) = load(&settings_json)?;
    let now = UnixTime(now);
    let formatter = |zone: &str| {
        Formatter::new(
            config.units.display_units(),
            config.presentation.voice,
            config.presentation.clock,
            zone,
        )
    };
    let summaries = spots
        .iter()
        .map(|e| {
            let f = formatter(&e.spot.time_zone);
            Ok(SpotSummary {
                id: e.spot.id.clone(),
                name: e.spot.name.clone(),
                water_type: water_type(&e.spot.water_type)?,
                water_type_label: f.water_type_label(water_type(&e.spot.water_type)?),
                status: Status {
                    last_success: e.last_success,
                    updated: e.last_success.map(|t| f.updated(UnixTime(t), now)),
                    error: e.error.clone(),
                    refreshing: e.refreshing,
                },
            })
        })
        .collect::<Result<Vec<_>, ViewError>>()?;
    let view = match spots.get(usize::try_from(active).unwrap_or(usize::MAX)) {
        Some(SpotEntry {
            spot,
            snapshot_json: Some(json),
            ..
        }) => {
            let snapshot: SpotSnapshot =
                serde_json::from_str(json).map_err(|e| invalid("snapshot", &e))?;
            Some(build_view(&view_input(
                &config,
                with_spot(snapshot, spot)?,
                now,
            )))
        }
        _ => None,
    };
    let page = Page {
        active,
        spots: summaries,
        view,
    };
    serde_json::to_string(&page).map_err(|e| invalid("view", &e))
}

/// A credit a display owes.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct Credit {
    /// The source's name.
    pub source: String,
    /// The credit line.
    pub text: String,
    /// A link to the source or its terms.
    pub link: Option<String>,
    /// The licence, when the source states one.
    pub licence: Option<String>,
}

/// The credits a snapshot's values owe, as the sources card lists them, for screens that show
/// data apart from the card (the map, widgets, About).
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn attributions(snapshot_json: String) -> Result<Vec<Credit>, ViewError> {
    let snapshot: SpotSnapshot =
        serde_json::from_str(&snapshot_json).map_err(|e| invalid("snapshot", &e))?;
    let now = snapshot.fetched_at;
    let mut input = view_input(&Config::default(), snapshot, now);
    input.card_order = vec![CardId::Sources];
    let view = build_view(&input);
    Ok(view
        .cards
        .into_iter()
        .find_map(|card| match card {
            Card::Sources(c) => Some(c.full.credits),
            _ => None,
        })
        .unwrap_or_default()
        .into_iter()
        .map(|c| Credit {
            source: c.source,
            text: c.text,
            link: c.link,
            licence: c.licence,
        })
        .collect())
}

// ---------------------------------------------------------------------------------------------
// The settings as a view input (as the command line builds it)
// ---------------------------------------------------------------------------------------------

/// What `build_view` takes, from the configuration. Mirrors `view_input` in the command line's
/// `commands/spot.rs`.
pub(crate) fn view_input(config: &Config, snapshot: SpotSnapshot, now: UnixTime) -> ViewInput {
    let river_ranges = river_ranges(&config.rivers, &snapshot);
    ViewInput {
        card_order: config.cards.order_for(snapshot.spot.water_type),
        snapshot,
        now,
        units: config.units.display_units(),
        voice: config.presentation.voice,
        clock: config.presentation.clock,
        timeline_span: config.presentation.timeline_span(),
        limits: measurement_limits(&config.sources),
        river_ranges,
        trout_stress_c: Some(config.thresholds.trout_stress_c as f32),
        solunar: SolunarSettings {
            show: config.windows.show_solunar,
        },
    }
}

/// Mirrors `river_ranges` in `commands/spot.rs`. The user's ranges for the gauge the spot's discharge comes from: the entry for that USGS
/// site, and none when the spot has no USGS discharge or no entry names its gauge.
fn river_ranges(rivers: &[RiverRange], snapshot: &SpotSnapshot) -> Option<RiverRanges> {
    let station = snapshot
        .water
        .discharge_m3s
        .as_ref()?
        .provenance()
        .station
        .as_ref()
        .filter(|s| s.source == SourceId::USGS)?;
    RiverRange::find(rivers, &station.id).map(RiverRange::to_ranges)
}

/// Station observations of wind and pressure have no `[sources]` group (as in the command
/// line's `runtime/fetch.rs`).
const OBSERVATION_MAX_AGE_MINUTES: u32 = 180;

/// Mirrors `group_name` in `runtime/fetch.rs`: the `[sources]` table that configures
/// `measurement`, by name.
fn group_name(measurement: Measurement) -> Option<&'static str> {
    use Measurement as M;
    Some(match measurement {
        M::WaterTemperature => "water_temperature",
        M::Tide => "tide",
        M::TidalCurrent | M::OceanCurrent => "currents",
        M::Discharge | M::GageHeight | M::FlowForecast | M::FloodCategories => "flow",
        M::LakeLevel => "lake_level",
        M::Waves | M::WavesForecast => "waves",
        M::SeaSurfaceTemp | M::SeaSurfaceTempForecast => "sst",
        M::Clarity | M::Turbidity => "clarity",
        M::DissolvedOxygen
        | M::Conductivity
        | M::Salinity
        | M::Ph
        | M::Chlorophyll
        | M::Cyanobacteria => "water_quality",
        M::MarineForecast | M::MarineAlerts => "marine_forecast",
        M::ObservedWind | M::ObservedPressure => return None,
    })
}

/// The distance and age limits of `measurement`'s group, or the fixed ones for observations.
fn limits_of(sources: &SourcesConfig, measurement: Measurement) -> (Option<f64>, Option<u32>) {
    let group: Option<&SourceGroup> = group_name(measurement).and_then(|name| {
        sources
            .groups()
            .into_iter()
            .find(|(n, _)| *n == name)
            .map(|(_, g)| g)
    });
    match group {
        Some(g) => (g.max_distance_km, g.max_age_minutes),
        None => (None, Some(OBSERVATION_MAX_AGE_MINUTES)),
    }
}

/// Mirrors `measurement_limits` in `runtime/fetch.rs`: the user's distance and age limits per
/// measurement, for greying values in the view.
fn measurement_limits(sources: &SourcesConfig) -> Vec<MeasurementLimit> {
    use Measurement as M;
    [
        M::WaterTemperature,
        M::Discharge,
        M::GageHeight,
        M::LakeLevel,
        M::Turbidity,
        M::Tide,
        M::TidalCurrent,
        M::Waves,
        M::SeaSurfaceTemp,
        M::ObservedWind,
        M::ObservedPressure,
    ]
    .into_iter()
    .filter_map(|m| {
        let (km, minutes) = limits_of(sources, m);
        let limit = MeasurementLimit {
            measurement: m,
            max_distance_m: km.map(|km| (km * 1_000.0) as f32),
            max_age_s: minutes.map(|min| i64::from(min) * 60),
        };
        (limit.max_distance_m.is_some() || limit.max_age_s.is_some()).then_some(limit)
    })
    .collect()
}
