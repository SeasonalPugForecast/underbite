//! The spot view, fully formatted for the app.
//!
//! The view is `underbite-core`'s `SpotView`, passed through as JSON without reshaping. The
//! settings become a `ViewInput` through `underbite-config`, the command line's own.

use std::collections::BTreeMap;

use serde::Serialize;
use underbite_config::schema::Config;
use underbite_config::view_input::view_input;
use underbite_core::discover::rank::MeasurementRanking;
use underbite_core::model::{Measurement, SpotSnapshot, WaterType};
use underbite_core::time::UnixTime;
use underbite_core::view::format::Formatter;
use underbite_core::view::stations;
use underbite_core::view::{Card, CardId, SpotView, build_view};

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

/// One station of a spot's station choices, every word written by the core.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct StationRow {
    /// The station as `pin_station` and the others take it: `coops:8518750`,
    /// `coops:NYH1927:13`.
    pub key: String,
    /// The station's name.
    pub name: String,
    /// The source's id (`coops`).
    pub source: String,
    /// The source's name: "NOAA CO-OPS".
    pub source_name: String,
    /// The station's id with its bin: "8518750", "NYH1927 · Bin 13".
    pub station: String,
    /// "0.4 mi", in the settings' unit.
    pub distance: String,
    /// "Same water body", "Nearby", ...; none for a station left out.
    pub relation: Option<String>,
    /// The line under the name: "NOAA CO-OPS 8518750 · 0.4 mi · Nearby".
    pub detail: String,
    /// Why it has its place, or why it was left out: "Latest value 6 min ago".
    pub reasons: Vec<String>,
    /// Always used for the spot.
    pub pinned: bool,
    /// Never used for the spot.
    pub excluded: bool,
    /// The ranking uses it.
    pub used: bool,
}

/// The stations of one measurement.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct StationGroup {
    /// The measurement's id (`"water_temperature"`).
    pub measurement: String,
    /// "Water temperature".
    pub title: String,
    /// The stations used, best first, then the ones the user excluded.
    pub rows: Vec<StationRow>,
    /// "3 not used", when other stations were left out.
    pub left_out_title: Option<String>,
    /// Those stations, each with why.
    pub left_out: Vec<StationRow>,
}

impl From<stations::StationRow> for StationRow {
    fn from(r: stations::StationRow) -> Self {
        Self {
            key: r.key,
            name: r.name,
            source: r.source,
            source_name: r.source_name,
            station: r.station,
            distance: r.distance,
            relation: r.relation,
            detail: r.detail,
            reasons: r.reasons,
            pinned: r.pinned,
            excluded: r.excluded,
            used: r.used,
        }
    }
}

/// The station choices of a spot as rows, from `discover_stations`'s `stations_json`: per
/// measurement (in the order lists of them read) the stations used with their distance in the
/// settings' unit, their relation and the reasons for their place, then the ones the user
/// excluded, and the others left out with why. `now` is Unix seconds.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn station_rows(
    stations_json: String,
    settings_json: String,
    now: i64,
) -> Result<Vec<StationGroup>, ViewError> {
    let (config, _) = load(&settings_json)?;
    let rankings: BTreeMap<Measurement, MeasurementRanking> =
        serde_json::from_str(&stations_json).map_err(|e| invalid("stations", &e))?;
    let f = Formatter::new(
        config.units.display_units(),
        config.presentation.voice,
        config.presentation.clock,
        "UTC",
    );
    Ok(stations::station_rows(&rankings, &f, UnixTime(now))
        .into_iter()
        .map(|g| StationGroup {
            measurement: serde_json::to_value(g.measurement)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_default(),
            title: g.title,
            rows: g.rows.into_iter().map(StationRow::from).collect(),
            left_out_title: g.left_out_title,
            left_out: g.left_out.into_iter().map(StationRow::from).collect(),
        })
        .collect())
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
