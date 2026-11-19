//! Trips across the FFI: checking a trip, the requests its history and tides need, and its view.
//!
//! The app keeps a trip as the JSON of the core's `Trip` (the `.underbite` payload) and passes it
//! through unchanged. A trip's view takes three steps, none of which does any I/O here:
//!
//! 1. [`trip_spots`]: the places to fetch with `fetch_spot` for their forecasts, and whether a
//!    forecast can reach any of their days.
//! 2. [`plan_trip`]: with those snapshots, the requests for what the forecasts do not reach:
//!    past years' values for the trip's dates (USGS normals for river flow and water
//!    temperature, Open-Meteo Marine sea surface temperature from 2023 on) and the tide
//!    station's harmonic constants, so tides are computed for every day. The app sends them
//!    (`URLSessionHost`) and hands the responses to [`TripPlan::results`]: each place's history
//!    as JSON to keep (past years do not change), and the constants as metadata to keep with
//!    `fetch_spot`'s `cacheable` responses.
//! 3. [`trip_view`]: the `TripView` as JSON, from the trip, the snapshots, the histories and the
//!    kept metadata.
//!
//! Coordinates leave the phone only rounded to the configured `sources.coordinate_precision`
//! (at most 2 decimals); station requests name the station alone. Keys arrive in [`Keys`] and
//! travel only as the USGS `X-Api-Key` header of a request marked never to be stored on disk.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use serde::Serialize;
use underbite_config::view_input::{river_ranges, view_input};
use underbite_core::astro::tide::TideHarmonics;
use underbite_core::datum::VerticalDatum;
use underbite_core::model::{Measurement, Spot, SpotSnapshot, WaterType};
use underbite_core::offline::cache::{CachedMetadata, MetadataKind, cache_key, serve_offline};
use underbite_core::provenance::{
    Provenance, Quality, SourceId, StationKind, StationRef, ValueKind,
};
use underbite_core::series::Series;
use underbite_core::time::{self, LocalDate, UnixTime, day_number};
use underbite_core::trip::grid::{self, GridConfig, PlaceInputs, PlaceKey, TideConstants};
use underbite_core::trip::history::{
    DayThresholds, GriddedInput, HistoryInputs, HistoryKind, SeriesInput, TripHistory, daily_means,
};
use underbite_core::trip::{MAX_TRIP_DAYS, Trip, TripError as Invalidity, TripPlace};
use underbite_core::view::trip::{OfflinePack, Sharing, TripAppState, build_trip_view};
use underbite_fetch::Call;
use underbite_providers::coops::mdapi;
use underbite_providers::open_meteo_marine::{HistoryKind as MarineHistoryKind, OpenMeteoMarine};
use underbite_providers::request::{Coordinates, Place, Request, Response, Target, Window};
use underbite_providers::source::{FetchContext, Secrets, SourceError};
use underbite_providers::usgs::Usgs;

use crate::fetch::{
    CachedResponse, HostRequest, HostResponse, Keys, SourceIssue, fetched, host_request,
    read_metadata, station_metadata, user_agent,
};
use crate::settings::{SettingsError, load};
use crate::view::SpotInfo;

/// The first year Open-Meteo's best-match sea surface temperature covers whole (it starts
/// between June and December 2022).
pub const SST_FIRST_YEAR: i32 = 2023;
/// The fewest years a date needs for a typical sea surface temperature.
pub const SST_MIN_YEARS: u32 = 2;
/// The fewest years the daily-record fallback of the normals needs.
const NORMALS_MIN_YEARS: u32 = 10;
/// How many days ahead any forecast a spot fetch gets can reach (Open-Meteo's 16).
pub const FORECAST_REACH_DAYS: i64 = 16;

// ---------------------------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------------------------

/// Why a trip call could not be done.
#[derive(Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum TripError {
    /// The settings could not be read.
    #[error("settings: {message}")]
    Settings {
        /// Why.
        message: String,
    },
    /// A trip, snapshot or history could not be read or written.
    #[error("invalid {what}: {message}")]
    Invalid {
        /// What was being read or written.
        what: String,
        /// The parser's message.
        message: String,
    },
}

impl From<SettingsError> for TripError {
    fn from(e: SettingsError) -> Self {
        let SettingsError::Invalid { message } = e;
        Self::Settings { message }
    }
}

fn invalid(what: &str, message: &impl ToString) -> TripError {
    TripError::Invalid {
        what: what.to_owned(),
        message: message.to_string(),
    }
}

fn read_trip(json: &str) -> Result<Trip, TripError> {
    serde_json::from_str(json).map_err(|e| invalid("trip", &e))
}

// ---------------------------------------------------------------------------------------------
// Checking a trip
// ---------------------------------------------------------------------------------------------

/// Checks a trip (its JSON): `None` when it can be planned, else why not, as a sentence: "The
/// last day is before the first.", "The trip is longer than 31 days.", "The Battery: unknown
/// time zone.".
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn trip_problem(trip_json: String) -> Option<String> {
    let Ok(trip) = serde_json::from_str::<Trip>(&trip_json) else {
        return Some(sentence("that is not a trip this app can read"));
    };
    trip.validate().err().map(|e| match e {
        Invalidity::Place { index, problem } => {
            let name = trip.places.get(index).map_or("", |p| p.name.as_str());
            sentence(&format!("{name}: {problem}"))
        }
        other => sentence(&other.to_string()),
    })
}

/// `text` with a capital first letter and a full stop.
fn sentence(text: &str) -> String {
    let mut chars = text.chars();
    let mut out: String = chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default();
    if !out.ends_with('.') {
        out.push('.');
    }
    out
}

// ---------------------------------------------------------------------------------------------
// The forecasts
// ---------------------------------------------------------------------------------------------

/// A place of a trip as a spot to fetch with `fetch_spot`.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct TripSpot {
    /// The place's key in the trip (its spot id).
    pub key: String,
    /// The spot to fetch; none for a place shared by area only, which has no coordinates.
    pub spot: Option<SpotInfo>,
    /// Whether a forecast can reach one of the place's days that is not over yet (within 16
    /// days of `now` in the place's zone). When it cannot, its last snapshot will do: only
    /// computed tides, astronomy and past years' values apply.
    pub forecast: bool,
}

/// The places of a trip (its JSON) as spots to fetch for their forecasts at Unix time `now`.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn trip_spots(trip_json: String, now: i64) -> Result<Vec<TripSpot>, TripError> {
    let trip = read_trip(&trip_json)?;
    let now = UnixTime(now);
    Ok(trip
        .places
        .iter()
        .map(|place| {
            let forecast = time::zone(&place.time_zone).is_ok_and(|tz| {
                time::local_date(now, &tz).is_ok_and(|today| {
                    let today = day_number(today);
                    dates(trip.first, trip.last).iter().any(|&d| {
                        let d = day_number(d);
                        d >= today && d <= today + FORECAST_REACH_DAYS
                    })
                })
            });
            TripSpot {
                key: PlaceKey::of(place).0,
                spot: spot_info(place),
                forecast,
            }
        })
        .collect())
}

fn spot_info(place: &TripPlace) -> Option<SpotInfo> {
    Some(SpotInfo {
        id: place.spot_id.clone(),
        name: place.name.clone(),
        latitude: place.latitude?,
        longitude: place.longitude?,
        water_type: serde_name(&place.water_type),
        time_zone: place.time_zone.clone(),
    })
}

/// A serde enum's name (`"river_stream"`).
fn serde_name(value: &impl Serialize) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(s)) => s,
        _ => String::new(),
    }
}

/// The dates from `first` to `last`; none when reversed or longer than a trip may be.
fn dates(first: LocalDate, last: LocalDate) -> Vec<LocalDate> {
    let (a, b) = (day_number(first), day_number(last));
    if b < a || b - a >= MAX_TRIP_DAYS {
        return Vec::new();
    }
    (a..=b).map(date_of).collect()
}

fn date_of(n: i64) -> LocalDate {
    let (year, month, day) = whether_core::time::civil_from_days(n);
    LocalDate { year, month, day }
}

// ---------------------------------------------------------------------------------------------
// The plan
// ---------------------------------------------------------------------------------------------

/// What the app holds for one place of a trip.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct TripPlaceData {
    /// The place's key in the trip (its spot id).
    pub key: String,
    /// The place's latest snapshot (`fetch_spot`'s `snapshot_json`), if any.
    pub snapshot_json: Option<String>,
    /// The place's history as [`TripPlan::results`] gave it, if kept.
    pub history_json: Option<String>,
}

/// One request of a [`TripPlan`], for the app to send.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct TripRequest {
    /// Unique within the plan: hand the response back under it.
    pub id: String,
    /// The key of the place it is for.
    pub place: String,
    /// The source's id (`"usgs"`, `"open-meteo-marine"`, `"coops"`).
    pub source: String,
    /// Whether it counts against the source's budget. The plan holds metered requests only
    /// when metered calls are allowed.
    pub metered: bool,
    /// The request itself.
    pub request: HostRequest,
}

/// The response to one [`TripRequest`]. A request with no response is left out.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct TripResponse {
    /// The request's id.
    pub id: String,
    /// What came back, any status.
    pub response: HostResponse,
}

/// The history of one place, to keep and pass back in [`TripPlaceData::history_json`].
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct TripPlaceHistory {
    /// The place's key.
    pub key: String,
    /// The core's `TripHistory` as JSON.
    pub history_json: String,
}

/// What a plan's responses gave.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct TripResults {
    /// One per place whose history was asked for.
    pub histories: Vec<TripPlaceHistory>,
    /// Station metadata (harmonic constants, datums, offsets) to keep with `fetch_spot`'s
    /// `cacheable` responses and pass to [`trip_view`] and `fetch_spot` as `cached_metadata`.
    pub cacheable: Vec<CachedResponse>,
    /// Requests that failed or had nothing; the cells they would have filled say
    /// "unavailable".
    pub errors: Vec<SourceIssue>,
}

/// What a planned request asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Wanted {
    Datums,
    Harcon,
    Offsets,
    Normals(Measurement),
    Sst,
}

impl Wanted {
    /// The tide metadata of `kind`.
    fn of(kind: MetadataKind) -> Option<Self> {
        match kind {
            MetadataKind::Datums => Some(Self::Datums),
            MetadataKind::Harmonics => Some(Self::Harcon),
            MetadataKind::TideOffsets => Some(Self::Offsets),
            _ => None,
        }
    }

    fn source(self) -> SourceId {
        match self {
            Self::Datums | Self::Harcon | Self::Offsets => SourceId::COOPS,
            Self::Normals(_) => SourceId::USGS,
            Self::Sst => SourceId::OPEN_METEO_MARINE,
        }
    }

    fn measurement(self) -> Measurement {
        match self {
            Self::Datums | Self::Harcon | Self::Offsets => Measurement::Tide,
            Self::Normals(m) => m,
            Self::Sst => Measurement::SeaSurfaceTempForecast,
        }
    }
}

#[derive(Debug)]
struct Planned {
    id: String,
    place: usize,
    wanted: Wanted,
    request: Request,
}

/// Which histories a place asks for.
#[derive(Clone, Debug, Default)]
struct PlaceAsk {
    key: String,
    /// The place's IANA zone.
    zone: String,
    flow: bool,
    water_temp: bool,
    sst: bool,
}

impl PlaceAsk {
    fn any(&self) -> bool {
        self.flow || self.water_temp || self.sst
    }
}

/// A trip's requests, kept on the Rust side: read them with [`TripPlan::requests`], send them,
/// and give the responses to [`TripPlan::results`].
#[derive(uniffi::Object)]
pub struct TripPlan {
    trip: Trip,
    places: Vec<PlaceAsk>,
    planned: Vec<Planned>,
    ctx: FetchContext,
}

impl fmt::Debug for TripPlan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TripPlan")
            .field("trip", &self.trip.id)
            .field("places", &self.places.len())
            .field("requests", &self.planned.len())
            .finish_non_exhaustive()
    }
}

/// Plans what a trip (its JSON) needs beyond its places' snapshots at Unix time `now`:
///
/// - the harmonic constants of each place's CO-OPS tide station (from its snapshot), unless
///   `cached_metadata` holds a copy recent enough, so tides are computed for every date;
/// - for each quantity no forecast reaches on some day not over yet: the USGS normals of the
///   snapshot's discharge and water temperature gauges for the trip's dates (a range across the
///   year end is two requests), and, on salt and tidal water, Open-Meteo Marine's sea surface
///   temperature for the same dates in each past year from 2023 on.
///
/// Histories are metered and planned only when `allow_metered`; a place whose kept history
/// (`history_json`) already has every quantity it needs is not asked again. A place with no
/// snapshot gets no history: its stations are not known.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn plan_trip(
    trip_json: String,
    places: Vec<TripPlaceData>,
    cached_metadata: Vec<CachedResponse>,
    settings_json: String,
    keys: Keys,
    allow_metered: bool,
    now: i64,
) -> Result<Arc<TripPlan>, TripError> {
    let (config, _) = load(&settings_json)?;
    let trip = read_trip(&trip_json)?;
    let now = UnixTime(now);
    let ctx = FetchContext::new(now, user_agent())
        .with_coordinate_precision(config.sources.coordinate_precision)
        .with_secrets(Secrets {
            usgs_api_key: keys.usgs.clone().filter(|k| !k.trim().is_empty()),
        });
    let cached = cached_map(cached_metadata);
    let data = data_by_key(&places);
    let mut asks = Vec::new();
    let mut planned = Vec::new();
    for (index, place) in trip.places.iter().enumerate() {
        let key = PlaceKey::of(place).0;
        let given = data.get(key.as_str());
        let snapshot = given
            .and_then(|d| d.snapshot_json.as_deref())
            .map(|json| {
                serde_json::from_str::<SpotSnapshot>(json).map_err(|e| invalid("snapshot", &e))
            })
            .transpose()?;
        let kept = given
            .and_then(|d| d.history_json.as_deref())
            .and_then(|json| serde_json::from_str::<TripHistory>(json).ok());
        let mut ask = PlaceAsk {
            key,
            zone: place.time_zone.clone(),
            ..PlaceAsk::default()
        };
        let Some(snapshot) = snapshot else {
            asks.push(ask);
            continue;
        };
        let mut add = |wanted: Wanted, request: Request, tag: String| {
            planned.push(Planned {
                id: format!("{index}:{tag}"),
                place: index,
                wanted,
                request,
            });
        };
        for (wanted, request) in tide_requests(&snapshot) {
            let fresh =
                serve_offline(&request.url, cached.get(&cache_key(&request.url)), now).is_ok();
            if !fresh {
                let tag = format!("{wanted:?}:{}", station_of(&request).unwrap_or_default());
                add(wanted, request, tag.to_ascii_lowercase());
            }
        }
        if allow_metered {
            plan_history(
                &trip,
                place,
                &snapshot,
                kept.as_ref(),
                &ctx,
                &mut ask,
                &mut add,
            );
        }
        asks.push(ask);
    }
    Ok(Arc::new(TripPlan {
        trip,
        places: asks,
        planned,
        ctx,
    }))
}

/// The history requests of one place: the normals of its gauges and the past years' sea surface
/// temperature, for the quantities some day needs and the kept history lacks.
fn plan_history(
    trip: &Trip,
    place: &TripPlace,
    snapshot: &SpotSnapshot,
    kept: Option<&TripHistory>,
    ctx: &FetchContext,
    ask: &mut PlaceAsk,
    add: &mut impl FnMut(Wanted, Request, String),
) {
    let Some(needs) = needs(trip, snapshot, &ask.zone, ctx.now) else {
        return;
    };
    let usgs = Usgs::new();
    for (measurement, wanted, series) in [
        (
            Measurement::Discharge,
            needs.flow,
            snapshot.water.discharge_m3s.as_ref(),
        ),
        (
            Measurement::WaterTemperature,
            needs.water_temp,
            snapshot.water.temperature_c.as_ref(),
        ),
    ] {
        let Some(target) = series.and_then(|s| usgs_target(s, measurement, ctx.now)) else {
            continue;
        };
        if !wanted || has(kept, trip, measurement) {
            continue;
        }
        let Ok(requests) =
            usgs.statistics_range_requests(&target, measurement, None, trip.first, trip.last, ctx)
        else {
            continue;
        };
        match measurement {
            Measurement::Discharge => ask.flow = true,
            _ => ask.water_temp = true,
        }
        for (n, request) in requests.into_iter().enumerate() {
            add(
                Wanted::Normals(measurement),
                request,
                format!("{}:{n}", serde_name(&measurement)),
            );
        }
    }
    if needs.water_temp
        && is_salt(place.water_type)
        && !has(kept, trip, Measurement::SeaSurfaceTempForecast)
        && let Some(target) = point_target(place, snapshot, ctx)
    {
        for (year, request) in sst_requests(trip, &target, ctx) {
            ask.sst = true;
            add(Wanted::Sst, request, format!("sst:{year}"));
        }
    }
}

#[uniffi::export]
impl TripPlan {
    /// Every request of the plan, in a fixed order.
    pub fn requests(&self) -> Vec<TripRequest> {
        self.planned
            .iter()
            .map(|p| TripRequest {
                id: p.id.clone(),
                place: self.places[p.place].key.clone(),
                source: p.wanted.source().as_str().to_owned(),
                metered: p.request.metered,
                request: host_request(&Call::from(&p.request)),
            })
            .collect()
    }

    /// Reads the responses: the history of each place that asked for one, the station metadata
    /// worth keeping, and what failed. Responses with an id the plan does not know are ignored;
    /// a request without a response counts as failed.
    #[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
    pub fn results(&self, responses: Vec<TripResponse>) -> TripResults {
        let by_id: BTreeMap<&str, &HostResponse> = responses
            .iter()
            .map(|r| (r.id.as_str(), &r.response))
            .collect();
        let mut gathered: Vec<Gathered> = self.places.iter().map(|_| Gathered::default()).collect();
        let mut cacheable: Vec<CachedResponse> = Vec::new();
        let mut errors = Vec::new();
        for p in &self.planned {
            let issue = |e: &SourceError| {
                SourceIssue::from(&e.to_model(p.wanted.source(), Some(p.wanted.measurement())))
            };
            let Some(response) = by_id.get(p.id.as_str()) else {
                errors.push(issue(&SourceError::Transport("no response".into())));
                continue;
            };
            let fetched = match fetched((*response).clone()) {
                Ok(f) => f,
                Err(e) => {
                    errors.push(issue(&SourceError::Transport(e.to_string())));
                    continue;
                }
            };
            let response = fetched.as_response();
            match self.read(p, &response, &mut gathered[p.place]) {
                Ok(()) => {
                    if matches!(p.wanted, Wanted::Datums | Wanted::Harcon | Wanted::Offsets)
                        && let Some(entry) =
                            CachedMetadata::new(&p.request.url, fetched.body.clone(), self.ctx.now)
                        && !cacheable.iter().any(|c| c.key == entry.key)
                    {
                        cacheable.push(CachedResponse {
                            key: entry.key,
                            body: entry.body,
                            fetched_at: entry.fetched_at.0,
                        });
                    }
                }
                Err(e) => errors.push(issue(&e)),
            }
        }
        let histories = self
            .places
            .iter()
            .zip(&gathered)
            .filter(|(ask, _)| ask.any())
            .filter_map(|(ask, got)| {
                let history = got.history(&self.trip, ask)?;
                Some(TripPlaceHistory {
                    key: ask.key.clone(),
                    history_json: serde_json::to_string(&history).ok()?,
                })
            })
            .collect();
        TripResults {
            histories,
            cacheable,
            errors,
        }
    }
}

impl TripPlan {
    /// Parses one response into `got`.
    fn read(
        &self,
        p: &Planned,
        response: &Response<'_>,
        got: &mut Gathered,
    ) -> Result<(), SourceError> {
        let body = || {
            if (200..300).contains(&response.status) {
                Ok(response.body)
            } else {
                Err(SourceError::Status {
                    status: response.status,
                    message: None,
                })
            }
        };
        match p.wanted {
            // Read as a spot fetch reads them before keeping them.
            Wanted::Datums => read_metadata(MetadataKind::Datums, &p.request.url, body()?),
            Wanted::Harcon => read_metadata(MetadataKind::Harmonics, &p.request.url, body()?),
            Wanted::Offsets => match mdapi::parse_tide_offsets(body()?)? {
                Some(_) => Ok(()),
                None => Err(SourceError::NoData(Some("no tide offsets".into()))),
            },
            Wanted::Normals(measurement) => {
                let range = Usgs::new().parse_statistics_range(&p.request, response, &self.ctx)?;
                let days = range.days.into_iter().map(|d| DayThresholds {
                    day_of_year: d.day_of_year,
                    thresholds: d.thresholds,
                    sample_count: d.sample_count,
                });
                match measurement {
                    Measurement::Discharge => got.flow.extend(days),
                    _ => got.water_temp.extend(days),
                }
                Ok(())
            }
            Wanted::Sst => {
                let history = OpenMeteoMarine.parse_history(&p.request, response, &self.ctx)?;
                match history.sea_surface_temp {
                    Some(series) => {
                        got.sst.push(series);
                        Ok(())
                    }
                    None => Err(SourceError::NoData(Some(
                        "no sea surface temperature for these dates".into(),
                    ))),
                }
            }
        }
    }
}

/// What the responses of one place gave.
#[derive(Debug, Default)]
struct Gathered {
    flow: Vec<DayThresholds>,
    water_temp: Vec<DayThresholds>,
    sst: Vec<Series<f32>>,
}

impl Gathered {
    /// The place's history: every quantity asked for is covered, empty where nothing came.
    fn history(&self, trip: &Trip, ask: &PlaceAsk) -> Option<TripHistory> {
        let sst_daily = match time::zone(&ask.zone) {
            Ok(tz) if ask.sst => daily_means(&self.sst.iter().collect::<Vec<_>>(), &tz),
            _ => Vec::new(),
        };
        let (usgs, marine) = (SourceId::USGS, SourceId::OPEN_METEO_MARINE);
        let series = |normals| SeriesInput {
            normals,
            normals_source: &usgs,
            daily: &[],
            daily_source: &usgs,
            min_years: NORMALS_MIN_YEARS,
        };
        let inputs = HistoryInputs {
            flow: ask.flow.then(|| series(&self.flow)),
            water_temp: ask.water_temp.then(|| series(&self.water_temp)),
            sst: ask.sst.then(|| GriddedInput {
                daily: &sst_daily,
                source: &marine,
                kind: HistoryKind::Model,
                min_years: SST_MIN_YEARS,
            }),
            waves: None,
            weather: None,
        };
        TripHistory::build(trip.first, trip.last, &inputs).ok()
    }
}

/// Which quantities some day of the trip needs past years' values for at a place: a day not
/// over yet (in the place's zone) that its forecast does not reach.
struct Needs {
    flow: bool,
    water_temp: bool,
}

fn needs(trip: &Trip, snapshot: &SpotSnapshot, zone: &str, now: UnixTime) -> Option<Needs> {
    let tz = time::zone(zone).ok()?;
    let today = day_number(time::local_date(now, &tz).ok()?);
    let starts: Vec<UnixTime> = dates(trip.first, trip.last)
        .into_iter()
        .filter(|&d| day_number(d) >= today)
        .filter_map(|d| time::local_date_bounds(d, &tz).ok().map(|(start, _)| start))
        .collect();
    let beyond = |series: Option<&Series<f32>>| {
        let end = series.and_then(|s| s.latest()).map(|s| s.time);
        starts
            .iter()
            .any(|&start| end.is_none_or(|end| end < start))
    };
    Some(Needs {
        flow: beyond(snapshot.water.discharge_forecast_m3s.as_ref()),
        water_temp: beyond(snapshot.marine.sea_surface_temp_forecast_c.as_ref()),
    })
}

/// Whether a kept history already has `measurement` for every date of the trip.
fn has(kept: Option<&TripHistory>, trip: &Trip, measurement: Measurement) -> bool {
    use underbite_core::trip::history::Coverage;
    let Some(h) = kept else { return false };
    let covered = match measurement {
        Measurement::Discharge => h.flow,
        Measurement::WaterTemperature => h.water_temp,
        _ => h.sst,
    };
    covered == Coverage::Available
        && dates(trip.first, trip.last)
            .iter()
            .all(|d| h.days.contains_key(d))
}

fn is_salt(water: WaterType) -> bool {
    matches!(
        water,
        WaterType::TidalRiverEstuary
            | WaterType::BayInshore
            | WaterType::Surf
            | WaterType::Nearshore
            | WaterType::Offshore
    )
}

/// The target of the normals of the USGS gauge `series` comes from.
fn usgs_target(series: &Series<f32>, measurement: Measurement, now: UnixTime) -> Option<Target> {
    let provenance = series.provenance();
    let station = provenance
        .station
        .as_ref()
        .filter(|s| s.source == SourceId::USGS)?;
    Some(Target {
        place: Place::Station {
            station: station.clone(),
            distance_m: provenance.distance_m,
            relation: provenance.relation,
        },
        measurements: std::iter::once(measurement).collect(),
        window: Window { from: now, to: now },
        datum: None,
    })
}

/// The rounded point of a place (its own coordinates, else its snapshot's).
fn point_target(place: &TripPlace, snapshot: &SpotSnapshot, ctx: &FetchContext) -> Option<Target> {
    let (latitude, longitude) = match (place.latitude, place.longitude) {
        (Some(lat), Some(lon)) => (lat, lon),
        _ => (snapshot.spot.latitude, snapshot.spot.longitude),
    };
    if !latitude.is_finite() || !longitude.is_finite() {
        return None;
    }
    Some(Target {
        place: Place::Point(ctx.round(Coordinates {
            latitude,
            longitude,
        })),
        measurements: std::iter::once(Measurement::SeaSurfaceTempForecast).collect(),
        window: Window {
            from: ctx.now,
            to: ctx.now,
        },
        datum: None,
    })
}

/// One request per past year from [`SST_FIRST_YEAR`] whose copy of the trip's dates is over.
fn sst_requests(trip: &Trip, target: &Target, ctx: &FetchContext) -> Vec<(i32, Request)> {
    let today = day_number(ctx.now.to_local(time::UtcOffset(0)).date);
    let span = trip.last.year - trip.first.year;
    (SST_FIRST_YEAR..=trip.last.year.max(SST_FIRST_YEAR))
        .filter_map(|year| {
            let from = in_year(trip.first, year);
            let to = in_year(trip.last, year + span);
            if day_number(to) >= today {
                return None;
            }
            OpenMeteoMarine
                .history_request(target, MarineHistoryKind::SeaSurfaceTemp, from, to, ctx)
                .ok()
                .map(|r| (year, r))
        })
        .collect()
}

/// `date` in `year`; 29 February is 28 February in a year without one.
fn in_year(date: LocalDate, year: i32) -> LocalDate {
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let day = if date.month == 2 && date.day == 29 && !leap {
        28
    } else {
        date.day
    };
    LocalDate {
        year,
        month: date.month,
        day,
    }
}

// ---------------------------------------------------------------------------------------------
// Tides from the kept station metadata
// ---------------------------------------------------------------------------------------------

/// The CO-OPS tide station of a snapshot's tide.
fn tide_station(snapshot: &SpotSnapshot) -> Option<(&StationRef, &Provenance)> {
    let tide = snapshot.marine.tide.as_ref()?;
    let station = tide
        .provenance
        .station
        .as_ref()
        .filter(|s| s.source == SourceId::COOPS)?;
    matches!(
        station.kind,
        StationKind::TideHarmonic | StationKind::TideSubordinate
    )
    .then_some((station, &tide.provenance))
}

/// The metadata requests the tides of a snapshot's station are computed from: a harmonic
/// station's datums and constants, or a subordinate station's offsets and its reference's
/// datums and constants, as a spot fetch keeps them.
fn tide_requests(snapshot: &SpotSnapshot) -> Vec<(Wanted, Request)> {
    let Some((station, _)) = tide_station(snapshot) else {
        return Vec::new();
    };
    station_metadata(station, None)
        .into_iter()
        .filter_map(|(kind, request)| Some((Wanted::of(kind)?, request)))
        .collect()
}

fn station_of(request: &Request) -> Option<String> {
    match &request.target.place {
        Place::Station { station, .. } => Some(station.id.clone()),
        _ => None,
    }
}

/// The harmonic constants of a snapshot's tide station from kept metadata (any age: constants
/// change only with a new tidal epoch), heights above MLLW.
fn tide_constants(
    snapshot: &SpotSnapshot,
    cached: &BTreeMap<String, CachedMetadata>,
    now: UnixTime,
) -> Option<TideConstants> {
    let (station, provenance) = tide_station(snapshot)?;
    let requests = tide_requests(snapshot);
    let body = |wanted: Wanted| {
        let (_, request) = requests.iter().find(|(w, _)| *w == wanted)?;
        cached
            .get(&cache_key(&request.url))
            .map(|c| (c.body.as_slice(), station_of(request).unwrap_or_default()))
    };
    let harmonics = || -> Option<TideHarmonics> {
        let (datums, id) = body(Wanted::Datums)?;
        let table = mdapi::parse_datums(&id, datums).ok()?;
        let (harcon, _) = body(Wanted::Harcon)?;
        mdapi::parse_tide_harmonics(&id, harcon, &table, VerticalDatum::Mllw).ok()
    };
    let base = Provenance {
        source: SourceId::COOPS_HARMONICS,
        station: Some(station.clone()),
        distance_m: provenance.distance_m,
        relation: provenance.relation,
        observed_at: now,
        kind: ValueKind::Computed,
        quality: Quality::Approved,
    };
    match station.kind {
        StationKind::TideSubordinate => {
            let (offsets, _) = body(Wanted::Offsets)?;
            let offsets = mdapi::parse_tide_offsets(offsets).ok()??;
            Some(TideConstants::Subordinate {
                reference: harmonics()?,
                offsets,
                base,
            })
        }
        _ => Some(TideConstants::Harmonic {
            harmonics: harmonics()?,
            base,
        }),
    }
}

fn cached_map(cached: Vec<CachedResponse>) -> BTreeMap<String, CachedMetadata> {
    cached
        .into_iter()
        .map(|c| {
            let entry = CachedMetadata {
                key: c.key.clone(),
                body: c.body,
                fetched_at: UnixTime(c.fetched_at),
            };
            (c.key, entry)
        })
        .collect()
}

fn data_by_key(places: &[TripPlaceData]) -> BTreeMap<&str, &TripPlaceData> {
    places.iter().map(|d| (d.key.as_str(), d)).collect()
}

// ---------------------------------------------------------------------------------------------
// The view
// ---------------------------------------------------------------------------------------------

/// The trip's offline pack, as the app knows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum TripPack {
    /// No pack.
    NotDownloaded,
    /// Downloaded: its size and when it was saved (Unix seconds).
    Ready {
        /// Bytes on disk.
        bytes: u64,
        /// When it was saved.
        saved_at: i64,
    },
}

/// Whether the trip is shared live.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum TripSharing {
    /// Not shared.
    NotShared,
    /// Shared through iCloud.
    Live {
        /// Others can only read it.
        read_only: bool,
    },
}

/// What the app knows about a trip besides its data; the view words it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Record)]
pub struct TripAppStatus {
    /// The offline pack.
    pub pack: TripPack,
    /// The live share.
    pub sharing: TripSharing,
}

impl From<TripAppStatus> for TripAppState {
    fn from(s: TripAppStatus) -> Self {
        Self {
            offline: match s.pack {
                TripPack::NotDownloaded => OfflinePack::NotDownloaded,
                TripPack::Ready { bytes, saved_at } => OfflinePack::Ready {
                    bytes,
                    saved_at: UnixTime(saved_at),
                },
            },
            sharing: match s.sharing {
                TripSharing::NotShared => Sharing::NotShared,
                TripSharing::Live { read_only } => Sharing::Live { read_only },
            },
        }
    }
}

/// The view of a trip (its JSON) at Unix time `now`: the core's `TripView` as JSON, every word
/// and value formatted in the settings' units and voice.
///
/// `places` holds each place's latest snapshot and kept history; a place without them still
/// gets its sun, moon and solunar days, and its tides when its station's constants are in
/// `cached_metadata`. Each day's cell says what it rests on: certain, a forecast with its issue
/// time, past years with their years, or unavailable and why. The trip's own limits judge its
/// days, else the settings' first limits profile.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn trip_view(
    trip_json: String,
    places: Vec<TripPlaceData>,
    cached_metadata: Vec<CachedResponse>,
    app: TripAppStatus,
    settings_json: String,
    now: i64,
) -> Result<String, TripError> {
    let (config, _) = load(&settings_json)?;
    let trip = read_trip(&trip_json)?;
    let now = UnixTime(now);
    let cached = cached_map(cached_metadata);
    let data = data_by_key(&places);
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
        let tide_constants = snapshot
            .as_ref()
            .and_then(|s| tide_constants(s, &cached, now));
        let river = snapshot
            .as_ref()
            .and_then(|s| river_ranges(&config.rivers, s))
            .unwrap_or_default();
        inputs.insert(
            key,
            PlaceInputs {
                snapshot,
                tide_constants,
                history,
                river,
                species_water_temp_c: None,
            },
        );
    }
    let fallback = trip
        .places
        .first()
        .and_then(|p| inputs.get(&PlaceKey::of(p)))
        .and_then(|i| i.snapshot.clone())
        .unwrap_or_else(|| SpotSnapshot::new(placeholder_spot(trip.places.first()), now));
    let input = view_input(&config, fallback, now);
    let cfg = GridConfig {
        windows: input.windows,
        limits: if trip.limits.is_some() {
            None
        } else {
            input.trip_limits.clone()
        },
    };
    let grid = grid::build(&trip, &inputs, &cfg, now);
    let view = build_trip_view(&trip, &grid, &inputs, &app.into(), &input);
    serde_json::to_string(&view).map_err(|e| invalid("trip view", &e))
}

/// The spot the view falls back to for its credits when no place has a snapshot.
fn placeholder_spot(place: Option<&TripPlace>) -> Spot {
    Spot {
        id: place.map(|p| p.spot_id.clone()).unwrap_or_default(),
        name: place.map(|p| p.name.clone()).unwrap_or_default(),
        latitude: place.and_then(|p| p.latitude).unwrap_or_default(),
        longitude: place.and_then(|p| p.longitude).unwrap_or_default(),
        water_type: place.map_or(WaterType::RiverStream, |p| p.water_type),
        water_body: None,
        time_zone: place.map(|p| p.time_zone.clone()).unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(year: i32, month: u8, day: u8) -> LocalDate {
        LocalDate { year, month, day }
    }

    #[test]
    fn sentences_start_with_a_capital_and_end_with_a_stop() {
        assert_eq!(sentence("the trip has no place"), "The trip has no place.");
        assert_eq!(sentence("Done."), "Done.");
        assert_eq!(sentence(""), ".");
    }

    #[test]
    fn dates_run_first_to_last_and_refuse_odd_ranges() {
        let d = dates(date(2026, 12, 30), date(2027, 1, 2));
        assert_eq!(d.len(), 4);
        assert_eq!(d[3], date(2027, 1, 2));
        assert_eq!(dates(date(2026, 11, 10), date(2026, 11, 8)), Vec::new());
        assert_eq!(dates(date(2026, 1, 1), date(2026, 3, 1)), Vec::new());
    }

    #[test]
    fn a_leap_day_moves_to_the_28th_in_other_years() {
        assert_eq!(in_year(date(2028, 2, 29), 2025), date(2025, 2, 28));
        assert_eq!(in_year(date(2028, 2, 29), 2024), date(2024, 2, 29));
        assert_eq!(in_year(date(2026, 11, 8), 2023), date(2023, 11, 8));
    }
}
