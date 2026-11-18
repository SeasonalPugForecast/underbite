//! Widget and Live Activity payloads over the FFI, from the latest snapshots: every value and
//! word ready, for the app to write to the shared app group store and the widgets to read.
//!
//! A payload is the core's `WidgetPayload` (or `TripDayActivity`) as JSON, passed through
//! unchanged: entries for now and each of the next 12 local hours, each part with its credit
//! line. Values are in the settings' units and voice, so a payload is built again whenever the
//! settings change. Payloads carry no snapshot and no coordinates.

use std::collections::BTreeMap;

use underbite_config::schema::Config;
use underbite_config::view_input::{river_ranges, view_input};
use underbite_core::model::{Spot, SpotSnapshot, WaterType};
use underbite_core::time::{self, UnixTime, UtcOffset, day_number};
use underbite_core::trip::Trip;
use underbite_core::trip::grid::{self, GridConfig, PlaceInputs, PlaceKey};
use underbite_core::trip::history::TripHistory;
use underbite_core::view::ViewInput;
use underbite_core::view::widget::{spot_payload, trip_day_activity, trip_payload};

use crate::settings::{SettingsError, load};
use crate::trip::TripPlaceData;
use crate::view::SpotInfo;

/// Why a payload could not be built.
#[derive(Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum WidgetError {
    /// The settings could not be read.
    #[error("settings: {message}")]
    Settings {
        /// Why.
        message: String,
    },
    /// A spot, snapshot, trip or history could not be read, or the payload written.
    #[error("invalid {what}: {message}")]
    Invalid {
        /// What was being read or written.
        what: String,
        /// The parser's message.
        message: String,
    },
}

impl From<SettingsError> for WidgetError {
    fn from(e: SettingsError) -> Self {
        let SettingsError::Invalid { message } = e;
        Self::Settings { message }
    }
}

fn invalid(what: &str, message: &impl ToString) -> WidgetError {
    WidgetError::Invalid {
        what: what.to_owned(),
        message: message.to_string(),
    }
}

fn to_json(what: &str, value: &impl serde::Serialize) -> Result<String, WidgetError> {
    serde_json::to_string(value).map_err(|e| invalid(what, &e))
}

fn water_type(text: &str) -> Result<WaterType, WidgetError> {
    serde_json::from_value(serde_json::Value::String(text.to_owned()))
        .map_err(|_| invalid("water type", &format!("`{text}` is not a water type")))
}

/// The payload of a spot's widgets (tide curve, river flow with its trend, next snack time) as
/// JSON, from its latest snapshot (`fetch_spot`'s `snapshot_json`) at Unix time `now`: the
/// spot as the app holds it (its name, water type and zone win over the snapshot's), the
/// settings' units, clock and voice.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn spot_widget(
    spot: SpotInfo,
    snapshot_json: String,
    settings_json: String,
    now: i64,
) -> Result<String, WidgetError> {
    let (config, _) = load(&settings_json)?;
    let mut snapshot: SpotSnapshot =
        serde_json::from_str(&snapshot_json).map_err(|e| invalid("snapshot", &e))?;
    snapshot.spot.name.clone_from(&spot.name);
    snapshot.spot.water_type = water_type(&spot.water_type)?;
    if !spot.time_zone.is_empty() {
        snapshot.spot.time_zone.clone_from(&spot.time_zone);
    }
    let input = view_input(&config, snapshot, UnixTime(now));
    to_json("payload", &spot_payload(&input))
}

/// A trip read with what the app holds for its places, as the trip view takes them.
struct TripParts {
    trip: Trip,
    inputs: BTreeMap<PlaceKey, PlaceInputs>,
    /// The settings, `now`, and the first place's snapshot for the credits.
    input: ViewInput,
}

/// The trip (its JSON) and its places' inputs: each place's latest snapshot and kept history,
/// with the user's river ranges. Tides come from the snapshots.
fn trip_parts(
    config: &Config,
    trip_json: &str,
    places: &[TripPlaceData],
    now: UnixTime,
) -> Result<TripParts, WidgetError> {
    let trip: Trip = serde_json::from_str(trip_json).map_err(|e| invalid("trip", &e))?;
    let data: BTreeMap<&str, &TripPlaceData> = places.iter().map(|d| (d.key.as_str(), d)).collect();
    let mut inputs = BTreeMap::new();
    for place in &trip.places {
        let key = PlaceKey::of(place);
        let given = data.get(key.0.as_str());
        let snapshot = given
            .and_then(|d| d.snapshot_json.as_deref())
            .map(|json| {
                serde_json::from_str::<SpotSnapshot>(json).map_err(|e| invalid("snapshot", &e))
            })
            .transpose()?;
        let history = given
            .and_then(|d| d.history_json.as_deref())
            .map(|json| {
                serde_json::from_str::<TripHistory>(json).map_err(|e| invalid("history", &e))
            })
            .transpose()?;
        let river = snapshot
            .as_ref()
            .and_then(|s| river_ranges(&config.rivers, s))
            .unwrap_or_default();
        inputs.insert(
            key,
            PlaceInputs {
                snapshot,
                tide_constants: None,
                history,
                river,
                species_water_temp_c: None,
            },
        );
    }
    let fallback = trip
        .places
        .iter()
        .find_map(|p| {
            inputs
                .get(&PlaceKey::of(p))
                .and_then(|i| i.snapshot.clone())
        })
        .unwrap_or_else(|| {
            let first = trip.places.first();
            let spot = Spot {
                id: first.map(|p| p.spot_id.clone()).unwrap_or_default(),
                name: first.map(|p| p.name.clone()).unwrap_or_default(),
                latitude: first.and_then(|p| p.latitude).unwrap_or_default(),
                longitude: first.and_then(|p| p.longitude).unwrap_or_default(),
                water_type: first.map_or(WaterType::RiverStream, |p| p.water_type),
                water_body: None,
                time_zone: first.map(|p| p.time_zone.clone()).unwrap_or_default(),
            };
            SpotSnapshot::new(spot, now)
        });
    let input = view_input(config, fallback, now);
    Ok(TripParts {
        trip,
        inputs,
        input,
    })
}

/// The payload of a trip's widget (its go/no-go at each entry, and its first two places with a
/// snapshot for the largest sizes) as JSON, from the trip (its JSON) and what the app holds for
/// its places (`trip_view`'s `places`) at Unix time `now`. Its days are judged with the trip's
/// own limits, else the settings' first limits profile, as the trip view does.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn trip_widget(
    trip_json: String,
    places: Vec<TripPlaceData>,
    settings_json: String,
    now: i64,
) -> Result<String, WidgetError> {
    let (config, _) = load(&settings_json)?;
    let now = UnixTime(now);
    let parts = trip_parts(&config, &trip_json, &places, now)?;
    let cfg = GridConfig {
        windows: parts.input.windows,
        limits: if parts.trip.limits.is_some() {
            None
        } else {
            parts.input.trip_limits.clone()
        },
    };
    let grid = grid::build(&parts.trip, &parts.inputs, &cfg, now);
    to_json(
        "payload",
        &trip_payload(&parts.trip, &grid, &parts.inputs, &parts.input),
    )
}

/// The trip-day Live Activity's content as JSON (the next tide change, the slack, the sunset,
/// each a target instant for the system timer with its words, and when it goes stale and
/// ends), from the trip's first place with a tide, else its first place with a snapshot, at
/// Unix time `now`. Whether today is a trip day is the trip view's to say.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn trip_activity(
    trip_json: String,
    places: Vec<TripPlaceData>,
    settings_json: String,
    now: i64,
) -> Result<String, WidgetError> {
    let (config, _) = load(&settings_json)?;
    let parts = trip_parts(&config, &trip_json, &places, UnixTime(now))?;
    to_json(
        "activity",
        &trip_day_activity(&parts.trip, &parts.inputs, &parts.input),
    )
}

/// Whether every place of the trip is past its last day, each in its own zone (UTC when none
/// is known), as the trip view says "over".
fn is_over(trip: &Trip, now: UnixTime) -> bool {
    let mut todays: Vec<i64> = trip
        .places
        .iter()
        .filter_map(|p| {
            let tz = time::zone(&p.time_zone).ok()?;
            time::local_date(now, &tz).ok()
        })
        .map(day_number)
        .collect();
    if todays.is_empty() {
        todays.push(day_number(now.to_local(UtcOffset(0)).date));
    }
    let last = day_number(trip.last);
    todays.iter().all(|&t| t > last)
}

/// The id of the trip the "next trip" widget shows at Unix time `now`: the soonest one (by its
/// first day) that is not over, of `trips` (their JSON); none when every trip is over.
/// Unreadable trips are passed over.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn next_trip(trips: Vec<String>, now: i64) -> Option<String> {
    let now = UnixTime(now);
    trips
        .iter()
        .filter_map(|json| serde_json::from_str::<Trip>(json).ok())
        .filter(|t| !is_over(t, now))
        .min_by(|a, b| (day_number(a.first), &a.id).cmp(&(day_number(b.first), &b.id)))
        .map(|t| t.id)
}
