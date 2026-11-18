//! The trip day grid: for each date and place, the verdict, the best windows, tides, flow and water
//! temperature, the sun, the moon and the solunar periods.
//!
//! A cell is data, never words: an optional value and the [`Basis`] it rests on, so the view can
//! never present history as a forecast.
//!
//! - [`Basis::Certain`]: astronomy (sun, moon, solunar) and tide predictions from harmonic
//!   constants (or the agency's published predictions). They hold for any date.
//! - [`Basis::Forecast`]: a model or service forecast, with the time the snapshot was issued. Each
//!   measurement has its own horizon: the weather's hourly steps for the verdict, the discharge or
//!   sea-surface forecast series for flow and water temperature. A day past a series' end has no
//!   forecast from it.
//! - [`Basis::Typical`]: past years' values for that date from [`TripHistory`], with the years
//!   behind them. Never a forecast, never an input to go/no-go.
//! - [`Basis::Unavailable`]: nothing is claimed; the reason says why.
//!
//! Windows beyond the forecast use only the certain factors (tide stage, tidal current, low light
//! and solunar), and say so ([`DayWindows::certain_only`]). Where the only tide source is a model's
//! sea level (outside CO-OPS coverage, ADR-012) the times are approximate and carry no heights.
//!
//! A day is a local calendar day of the place's own zone, so a trip with places in two zones has two
//! "today"s ([`PlaceGrid::today`]). The module reads no clock: `now` is a parameter. Observed and
//! forecast series are never spliced: flow and water temperature for a future day come from the
//! forecast series alone.

use std::collections::BTreeMap;
use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use whether_core::history::YearRange;

use super::gonogo::{Check, DayInputs, Reason, ReasonKind, Verdict, assess};
use super::history::{Band, Coverage, TripHistory};
use super::model::{MAX_TRIP_DAYS, Trip, TripLimits, TripPlace};
use super::windows::{FishingWindow, Ranges, WindowInputs, WindowParams, compute};
use crate::astro::solunar::{DayBounds, SolunarSettings, sky_conditions};
use crate::astro::tide::{SubordinateTideOffsets, TideHarmonics};
use crate::datum::VerticalDatum;
use crate::model::{
    MoonDay, SolunarPeriod, SpotSnapshot, SunTimes, TideEvent, TideSeries, TideSourceKind,
};
use crate::offline::tables::{TideStation, tide_table};
use crate::provenance::{Provenance, SourceId};
use crate::series::Series;
use crate::time::{self, LocalDate, UnixTime};
use crate::view::RiverRanges;

/// Seconds between the points of a computed tide curve.
const TIDE_STEP: Duration = Duration::from_mins(6);
const DAY_S: i64 = 86_400;
const HOUR_S: i64 = 3_600;
/// Longest gap between hourly steps that still counts as the last step's length.
const MAX_STEP_S: i64 = 3 * HOUR_S;

/// Names a place of a trip in the grid and in the inputs: the host's id of the saved spot.
#[derive(
    Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct PlaceKey(pub String);

impl PlaceKey {
    /// The key of a trip place.
    pub fn of(place: &TripPlace) -> Self {
        Self(place.spot_id.clone())
    }
}

/// The settings the grid is built with, taken from the configuration by the caller.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GridConfig {
    /// The `[windows]` settings.
    pub windows: WindowParams,
    /// Limits to judge the days with; the trip's own limits when absent, and none at all when
    /// both are (only storms and the forecast's presence are then checked).
    pub limits: Option<TripLimits>,
}

/// Harmonic constants a place's tides are computed from (offline tables).
#[derive(Clone, Debug, PartialEq)]
pub enum TideConstants {
    /// A harmonic (reference) station.
    Harmonic {
        harmonics: TideHarmonics,
        /// Where the station is and how it relates to the spot.
        base: Provenance,
    },
    /// A subordinate station: its reference's harmonics and its own offsets. Highs and lows only.
    Subordinate {
        reference: TideHarmonics,
        offsets: SubordinateTideOffsets,
        base: Provenance,
    },
}

/// What the grid knows about one place; everything is optional.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlaceInputs {
    /// The latest snapshot: forecasts, storm watch and, if the constants are absent, published
    /// tide predictions or the model sea-level fallback.
    pub snapshot: Option<SpotSnapshot>,
    /// Harmonic constants of the tide station for this place. Preferred over the snapshot's tide
    /// series because they cover any date.
    pub tide_constants: Option<TideConstants>,
    /// Past years' values for the trip's dates.
    pub history: Option<TripHistory>,
    /// The user's ranges for the spot's gauge, m³/s.
    pub river: RiverRanges,
    /// The species' preferred water temperature, °C `[min, max]` (filled once profiles exist).
    pub species_water_temp_c: Option<[f64; 2]>,
}

/// Why a cell holds nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Why {
    /// The place has no coordinates (shared at area-only precision) and the snapshot no spot.
    NoLocation,
    /// No source was set up or asked for this quantity here.
    NoSource,
    /// No snapshot, or one without the data this needs.
    NoForecast,
    /// The date is past the end of the forecast, and there is no history for it either.
    BeyondForecast,
    /// History was asked for and has nothing for this date.
    NoHistory,
    /// The day is over.
    Past,
    /// Hidden by the user's settings.
    Disabled,
    /// No tide predictions cover the day.
    NoCoverage,
}

/// What a cell rests on.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "basis", rename_all = "snake_case")]
pub enum Basis {
    /// Astronomy or a harmonic prediction: holds for any date.
    Certain,
    /// A forecast issued at `issued` (the snapshot's fetch time).
    Forecast { issued: UnixTime },
    /// Past years' values for the date.
    Typical {
        /// First and last year of the sample; absent when the source gives only a count.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        years: Option<YearRange>,
        years_sampled: u32,
    },
    /// Nothing is claimed.
    Unavailable { reason: Why },
}

/// One cell: its value (absent exactly when the basis is unavailable), what it rests on and who
/// supplied it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(bound(serialize = "T: Serialize", deserialize = "T: Deserialize<'de>"))]
pub struct Cell<T> {
    pub basis: Basis,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<T>,
    /// The source credited, where there is one (computed astronomy has none).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceId>,
}

impl<T> Cell<T> {
    fn known(basis: Basis, value: T, source: Option<SourceId>) -> Self {
        Self {
            basis,
            value: Some(value),
            source,
        }
    }

    fn certain(value: T) -> Self {
        Self::known(Basis::Certain, value, None)
    }

    fn missing(reason: Why) -> Self {
        Self {
            basis: Basis::Unavailable { reason },
            value: None,
            source: None,
        }
    }
}

/// The go/no-go of a day: the verdict and why, with storms apart.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct GridVerdict {
    pub verdict: Verdict,
    /// The findings other than storms: one per check and outcome, worst first.
    pub reasons: Vec<Reason>,
    /// A storm ahead or a hurricane cone over the place on this day: the day's storm marker, and
    /// the reason it is a no-go.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub storm: Vec<Reason>,
}

/// The best windows of a day.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DayWindows {
    pub windows: Vec<FishingWindow>,
    /// Only tide, current, light and solunar took part: no forecast went into these windows.
    pub certain_only: bool,
}

/// Tide highs and lows of a day, in the place's local day.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TideDay {
    pub kind: TideSourceKind,
    /// The datum of the heights; absent when there are none (model sea level).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub datum: Option<VerticalDatum>,
    /// In time order; heights are absent for a model sea level.
    pub events: Vec<TideEvent>,
}

/// Where a flow falls against the user's best range.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FlowStatus {
    /// Above the user's blown-out level.
    BlownOut,
    AboveBest,
    InBest,
    BelowBest,
    /// The user set no best range or blown-out level.
    Unclassified,
}

/// The day's river discharge against the user's ranges.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FlowReading {
    /// The day's mean for a forecast, the median for a typical day; m³/s.
    pub m3s: f32,
    /// The 10th and 90th percentile of past years; only for a typical day.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p10_m3s: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p90_m3s: Option<f32>,
    /// Where `m3s` falls against the user's best and blown-out levels.
    pub status: FlowStatus,
    /// Whether `m3s` is at or below the user's wadeable level; absent if none is set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wadeable: Option<bool>,
}

/// Where a temperature falls against the species' range.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TempStatus {
    Below,
    Within,
    Above,
}

/// The day's water temperature against the species' range.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WaterTempReading {
    /// The day's mean for a forecast, the median for a typical day; °C.
    pub celsius: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p10_c: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p90_c: Option<f32>,
    /// Against the species' preferred range; absent if no species range is known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<TempStatus>,
}

/// One local day at one place.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TripDay {
    #[serde(with = "super::model::date_string")]
    #[schemars(with = "String")]
    pub date: LocalDate,
    pub verdict: Cell<GridVerdict>,
    pub windows: Cell<DayWindows>,
    pub tides: Cell<TideDay>,
    pub flow: Cell<FlowReading>,
    pub water_temp: Cell<WaterTempReading>,
    pub sun: Cell<SunTimes>,
    pub moon: Cell<MoonDay>,
    pub solunar: Cell<Vec<SolunarPeriod>>,
}

/// Why a place has no days.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GridProblem {
    /// The last day is before the first, or the trip is longer than the limit.
    Dates,
    /// The place's time zone is not in the IANA database.
    TimeZone,
}

/// The days of one place.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PlaceGrid {
    /// Position of the place in the trip's list.
    pub index: usize,
    pub name: String,
    pub time_zone: String,
    /// The local date of `now` in this place's zone.
    #[serde(default, with = "opt_date", skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<String>")]
    pub today: Option<LocalDate>,
    /// One per local date, first to last; empty when `problem` is set.
    pub days: Vec<TripDay>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<GridProblem>,
}

/// The grid of a trip: its places, by key.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TripGrid {
    pub places: BTreeMap<PlaceKey, PlaceGrid>,
}

mod opt_date {
    use serde::{Deserialize, Deserializer, Serializer};

    use crate::time::{LocalDate, parse_date};

    // serde's `with` calls this with a reference to the field.
    #[allow(clippy::ref_option)]
    pub(super) fn serialize<S: Serializer>(
        date: &Option<LocalDate>,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        match date {
            Some(d) => s.collect_str(&format_args!("{:04}-{:02}-{:02}", d.year, d.month, d.day)),
            None => s.serialize_none(),
        }
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        d: D,
    ) -> Result<Option<LocalDate>, D::Error> {
        Option::<String>::deserialize(d)?
            .map(|s| parse_date(&s).map_err(serde::de::Error::custom))
            .transpose()
    }
}

/// Builds the grid of `trip`: each place for each of its local days.
///
/// `inputs` holds what is known per place; a place without an entry gets certain astronomy only.
/// `limits` come from `cfg`, else from the trip. Nothing panics on odd input: a place with an
/// unknown zone, or a trip with reversed or too many dates, yields a [`PlaceGrid`] with `problem`
/// set and no days.
pub fn build(
    trip: &Trip,
    inputs: &BTreeMap<PlaceKey, PlaceInputs>,
    cfg: &GridConfig,
    now: UnixTime,
) -> TripGrid {
    let empty = PlaceInputs::default();
    let mut places = BTreeMap::new();
    for (index, place) in trip.places.iter().enumerate() {
        let key = PlaceKey::of(place);
        let input = inputs.get(&key).unwrap_or(&empty);
        let grid = build_place(trip, index, place, input, cfg, now);
        places.insert(key, grid);
    }
    TripGrid { places }
}

/// The local dates from `first` to `last`, or `None` if reversed or too long.
fn dates(first: LocalDate, last: LocalDate) -> Option<Vec<LocalDate>> {
    let (a, b) = (time::day_number(first), time::day_number(last));
    if b < a || b - a >= MAX_TRIP_DAYS {
        return None;
    }
    Some(
        (a..=b)
            .map(|n| {
                let (year, month, day) = whether_core::time::civil_from_days(n);
                LocalDate { year, month, day }
            })
            .collect(),
    )
}

fn build_place(
    trip: &Trip,
    index: usize,
    place: &TripPlace,
    input: &PlaceInputs,
    cfg: &GridConfig,
    now: UnixTime,
) -> PlaceGrid {
    let mut grid = PlaceGrid {
        index,
        name: place.name.clone(),
        time_zone: place.time_zone.clone(),
        today: None,
        days: Vec::new(),
        problem: None,
    };
    let Ok(tz) = time::zone(&place.time_zone) else {
        grid.problem = Some(GridProblem::TimeZone);
        return grid;
    };
    grid.today = time::local_date(now, &tz).ok();
    let Some(list) = dates(trip.first, trip.last) else {
        grid.problem = Some(GridProblem::Dates);
        return grid;
    };
    let mut bounds = Vec::with_capacity(list.len());
    for &date in &list {
        let Ok((start, end)) = time::local_date_bounds(date, &tz) else {
            grid.problem = Some(GridProblem::TimeZone);
            return grid;
        };
        bounds.push(DayBounds { date, start, end });
    }
    let ctx = PlaceCtx::new(trip, place, input, cfg, &bounds, now);
    grid.days = bounds
        .iter()
        .enumerate()
        .map(|(i, b)| ctx.day(i, b))
        .collect();
    grid
}

/// Everything shared by the days of one place.
struct PlaceCtx<'a> {
    input: &'a PlaceInputs,
    cfg: &'a GridConfig,
    limits: TripLimits,
    now: UnixTime,
    sky: Option<crate::model::SkyConditions>,
    /// The tide source for the whole trip: computed from constants, else the snapshot's.
    tide: Option<TideSeries>,
    /// Windows inputs from the snapshot, before per-day trimming.
    base_windows: Option<WindowInputs>,
}

impl<'a> PlaceCtx<'a> {
    fn new(
        trip: &'a Trip,
        place: &'a TripPlace,
        input: &'a PlaceInputs,
        cfg: &'a GridConfig,
        bounds: &[DayBounds],
        now: UnixTime,
    ) -> Self {
        let location = match (place.latitude, place.longitude) {
            (Some(lat), Some(lon)) => Some((lat, lon)),
            _ => input
                .snapshot
                .as_ref()
                .map(|s| (s.spot.latitude, s.spot.longitude)),
        };
        let params = &cfg.windows;
        let sky = location.map(|(lat, lon)| {
            sky_conditions(
                lat,
                lon,
                bounds,
                SolunarSettings {
                    major_minutes: params.solunar_major_minutes,
                    minor_minutes: params.solunar_minor_minutes,
                },
            )
        });
        let tide = computed_tides(input, bounds)
            .or_else(|| input.snapshot.as_ref().and_then(|s| s.marine.tide.clone()));
        let ranges = Ranges {
            water_temp_c: input.species_water_temp_c,
            flow_m3s: match (input.river.best_min_m3s, input.river.best_max_m3s) {
                (Some(a), Some(b)) => Some([f64::from(a), f64::from(b)]),
                _ => None,
            },
        };
        let (first, last) = (trip.first, trip.last);
        let base_windows = location.map(|(lat, lon)| {
            let mut w = match &input.snapshot {
                Some(s) => WindowInputs::from_snapshot(s, first, last, ranges),
                None => WindowInputs {
                    latitude: lat,
                    longitude: lon,
                    time_zone: place.time_zone.clone(),
                    first,
                    last,
                    tide: None,
                    current: None,
                    pressure_hpa: Vec::new(),
                    water_temp_c: None,
                    discharge_m3s: None,
                    ranges,
                },
            };
            w.latitude = lat;
            w.longitude = lon;
            w.time_zone.clone_from(&place.time_zone);
            w.tide.clone_from(&tide);
            w
        });
        Self {
            input,
            cfg,
            limits: cfg
                .limits
                .clone()
                .or_else(|| trip.limits.clone())
                .unwrap_or_default(),
            now,
            sky,
            tide,
            base_windows,
        }
    }

    fn issued(&self) -> Option<UnixTime> {
        self.input.snapshot.as_ref().map(|s| s.fetched_at)
    }

    fn day(&self, i: usize, b: &DayBounds) -> TripDay {
        let sky = self.sky.as_ref().and_then(|s| s.days.get(i));
        let past = b.end <= self.now;
        let solunar = match (sky, self.cfg.windows.show_solunar) {
            (_, false) => Cell::missing(Why::Disabled),
            (Some(s), true) => Cell::certain(s.solunar.clone()),
            (None, true) => Cell::missing(Why::NoLocation),
        };
        TripDay {
            date: b.date,
            verdict: self.verdict(b, past),
            windows: self.windows(b, past),
            tides: self.tides(b),
            flow: self.flow(b),
            water_temp: self.water_temp(b),
            sun: sky.map_or_else(|| Cell::missing(Why::NoLocation), |s| Cell::certain(s.sun)),
            moon: sky.map_or_else(|| Cell::missing(Why::NoLocation), |s| Cell::certain(s.moon)),
            solunar,
        }
    }

    fn verdict(&self, b: &DayBounds, past: bool) -> Cell<GridVerdict> {
        let Some(snapshot) = &self.input.snapshot else {
            return Cell::missing(Why::NoForecast);
        };
        if past {
            return Cell::missing(Why::Past);
        }
        let day = assess(
            b.date,
            &DayInputs::new(snapshot).with_river(self.input.river),
            &self.limits,
            self.now,
        );
        let (storm, reasons): (Vec<Reason>, Vec<Reason>) = day.reasons.into_iter().partition(|r| {
            matches!(r.check, Check::StormAhead | Check::Hurricane) && r.kind == ReasonKind::Fails
        });
        let horizon = weather_horizon(snapshot);
        if storm.is_empty() && horizon.is_none_or(|h| h <= b.start) {
            // Beyond the models' horizon nothing is claimed.
            return Cell::missing(if horizon.is_some() {
                Why::BeyondForecast
            } else {
                Why::NoForecast
            });
        }
        Cell::known(
            Basis::Forecast {
                issued: snapshot.fetched_at,
            },
            GridVerdict {
                verdict: day.verdict,
                reasons,
                storm,
            },
            snapshot
                .weather
                .as_ref()
                .map(|w| SourceId::from_static(w.provider.id())),
        )
    }

    fn windows(&self, b: &DayBounds, past: bool) -> Cell<DayWindows> {
        let Some(base) = &self.base_windows else {
            return Cell::missing(Why::NoLocation);
        };
        if past {
            return Cell::missing(Why::Past);
        }
        let mut inputs = base.clone();
        inputs.first = b.date;
        inputs.last = b.date;
        // A forecast series takes part only on days it reaches.
        let reaches = |last: Option<UnixTime>| last.is_some_and(|t| t >= b.start);
        if !reaches(inputs.pressure_hpa.last().map(|s| s.time)) {
            inputs.pressure_hpa.clear();
        }
        if !reaches(
            inputs
                .water_temp_c
                .as_ref()
                .and_then(|s| s.latest())
                .map(|s| s.time),
        ) {
            inputs.water_temp_c = None;
        }
        if !reaches(
            inputs
                .discharge_m3s
                .as_ref()
                .and_then(|s| s.latest())
                .map(|s| s.time),
        ) {
            inputs.discharge_m3s = None;
        }
        let model_tide = self.tide.as_ref().is_some_and(|t| {
            t.source_kind == TideSourceKind::ModelMean
                && t.events.iter().any(|e| e.time >= b.start && e.time < b.end)
        });
        let certain_only = inputs.pressure_hpa.is_empty()
            && inputs.water_temp_c.is_none()
            && inputs.discharge_m3s.is_none()
            && !model_tide;
        let windows = compute(&inputs, &self.cfg.windows, self.now);
        let value = DayWindows {
            windows,
            certain_only,
        };
        let basis = match (certain_only, self.issued()) {
            (false, Some(issued)) => Basis::Forecast { issued },
            _ => Basis::Certain,
        };
        Cell::known(basis, value, None)
    }

    fn tides(&self, b: &DayBounds) -> Cell<TideDay> {
        let Some(series) = &self.tide else {
            return Cell::missing(Why::NoSource);
        };
        let events: Vec<TideEvent> = series
            .events
            .iter()
            .filter(|e| e.time >= b.start && e.time < b.end)
            .map(|e| TideEvent {
                height_m: if series.source_kind == TideSourceKind::ModelMean {
                    None
                } else {
                    e.height_m
                },
                ..*e
            })
            .collect();
        if events.is_empty() {
            let reason = if series.source_kind == TideSourceKind::ModelMean {
                Why::BeyondForecast
            } else {
                Why::NoCoverage
            };
            return Cell::missing(reason);
        }
        let model = series.source_kind == TideSourceKind::ModelMean;
        let basis = match (model, self.issued()) {
            (true, Some(issued)) => Basis::Forecast { issued },
            (true, None) => return Cell::missing(Why::NoForecast),
            (false, _) => Basis::Certain,
        };
        Cell::known(
            basis,
            TideDay {
                kind: series.source_kind,
                datum: (!model).then_some(series.datum),
                events,
            },
            Some(series.provenance.source.clone()),
        )
    }

    /// Why history has nothing for a quantity on a date.
    fn no_history(&self, coverage: impl Fn(&TripHistory) -> Coverage) -> Why {
        match &self.input.history {
            Some(h) if coverage(h) != Coverage::NotRequested => Why::NoHistory,
            _ => Why::NoSource,
        }
    }

    fn flow(&self, b: &DayBounds) -> Cell<FlowReading> {
        let river = &self.input.river;
        let forecast = self
            .input
            .snapshot
            .as_ref()
            .and_then(|s| s.water.discharge_forecast_m3s.as_ref())
            .and_then(|s| day_mean(s, b));
        if let (Some(mean), Some(issued)) = (forecast, self.issued()) {
            let source = self
                .input
                .snapshot
                .as_ref()
                .and_then(|s| s.water.discharge_forecast_m3s.as_ref())
                .map(|s| s.provenance().source.clone());
            return Cell::known(
                Basis::Forecast { issued },
                FlowReading {
                    m3s: mean,
                    p10_m3s: None,
                    p90_m3s: None,
                    status: flow_status(mean, river),
                    wadeable: river.wadeable_below_m3s.map(|w| mean <= w),
                },
                source,
            );
        }
        let band = self
            .input
            .history
            .as_ref()
            .and_then(|h| h.days.get(&b.date))
            .and_then(|d| d.flow.as_ref());
        match band {
            Some(band) => Cell::known(
                typical(band),
                FlowReading {
                    m3s: band.median,
                    p10_m3s: Some(band.p10),
                    p90_m3s: Some(band.p90),
                    status: flow_status(band.median, river),
                    wadeable: river.wadeable_below_m3s.map(|w| band.median <= w),
                },
                Some(band.basis.source.clone()),
            ),
            None => Cell::missing(self.no_history(|h| h.flow)),
        }
    }

    fn water_temp(&self, b: &DayBounds) -> Cell<WaterTempReading> {
        let range = self.input.species_water_temp_c;
        let status = |c: f32| {
            range.map(|[lo, hi]| {
                let c = f64::from(c);
                if c < lo {
                    TempStatus::Below
                } else if c > hi {
                    TempStatus::Above
                } else {
                    TempStatus::Within
                }
            })
        };
        let forecast_series = self
            .input
            .snapshot
            .as_ref()
            .and_then(|s| s.marine.sea_surface_temp_forecast_c.as_ref());
        if let (Some(series), Some(issued)) = (forecast_series, self.issued())
            && let Some(mean) = day_mean(series, b)
        {
            return Cell::known(
                Basis::Forecast { issued },
                WaterTempReading {
                    celsius: mean,
                    p10_c: None,
                    p90_c: None,
                    status: status(mean),
                },
                Some(series.provenance().source.clone()),
            );
        }
        let typical_day = self
            .input
            .history
            .as_ref()
            .and_then(|h| h.days.get(&b.date));
        match typical_day.and_then(|d| d.water_temp.as_ref().or(d.sst.as_ref())) {
            Some(band) => Cell::known(
                typical(band),
                WaterTempReading {
                    celsius: band.median,
                    p10_c: Some(band.p10),
                    p90_c: Some(band.p90),
                    status: status(band.median),
                },
                Some(band.basis.source.clone()),
            ),
            None => Cell::missing(self.no_history(|h| {
                if h.water_temp == Coverage::NotRequested {
                    h.sst
                } else {
                    h.water_temp
                }
            })),
        }
    }
}

fn typical(band: &Band) -> Basis {
    Basis::Typical {
        years: band.basis.years,
        years_sampled: band.basis.years_sampled,
    }
}

/// The end of the weather forecast: the last hourly step and its length.
fn weather_horizon(snapshot: &SpotSnapshot) -> Option<UnixTime> {
    let hourly = &snapshot.weather.as_ref()?.hourly;
    let step = match hourly.as_slice() {
        [.., a, b] if b.time.0 > a.time.0 => (b.time.0 - a.time.0).min(MAX_STEP_S),
        _ => HOUR_S,
    };
    hourly.last().map(|h| UnixTime(h.time.0 + step))
}

/// The mean of the finite samples in the local day, if any.
fn day_mean(series: &Series<f32>, b: &DayBounds) -> Option<f32> {
    let values: Vec<f32> = series
        .window(b.start, b.end)
        .iter()
        .map(|s| s.value)
        .filter(|v| v.is_finite())
        .collect();
    if values.is_empty() {
        return None;
    }
    Some(values.iter().sum::<f32>() / values.len() as f32)
}

fn flow_status(m3s: f32, r: &RiverRanges) -> FlowStatus {
    if r.blown_out_above_m3s.is_some_and(|t| m3s > t) {
        return FlowStatus::BlownOut;
    }
    match (r.best_min_m3s, r.best_max_m3s) {
        (Some(lo), _) if m3s < lo => FlowStatus::BelowBest,
        (_, Some(hi)) if m3s > hi => FlowStatus::AboveBest,
        (None, None) => FlowStatus::Unclassified,
        _ => FlowStatus::InBest,
    }
}

/// Tides for the whole trip from the harmonic constants, padded a day each side so the stage at
/// the edges is known. `None` without constants or if the predictor refuses them.
fn computed_tides(input: &PlaceInputs, bounds: &[DayBounds]) -> Option<TideSeries> {
    let constants = input.tide_constants.as_ref()?;
    let first = bounds.first()?.start;
    let last = bounds.last()?.end;
    let (from, to) = (UnixTime(first.0 - DAY_S), UnixTime(last.0 + DAY_S - 1));
    let (station, base) = match constants {
        TideConstants::Harmonic { harmonics, base } => (TideStation::Harmonic(harmonics), base),
        TideConstants::Subordinate {
            reference,
            offsets,
            base,
        } => (TideStation::Subordinate { reference, offsets }, base),
    };
    tide_table(station, base.clone(), from, to, TIDE_STEP).ok()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::assert_is_empty, clippy::format_push_string)]

    use std::collections::BTreeMap;

    use serde_json::Value;
    use whether_core::time::days_from_civil;

    use super::*;
    use crate::astro::tide::{HarmonicConstituent, HeightAdjustment};
    use crate::datum::DatumTable;
    use crate::model::snapshot::testdata as sample;
    use crate::model::{StormWatch, TideKind};
    use crate::provenance::{Quality, Relation, StationKind, StationRef, ValueKind};
    use crate::series::Sample;
    use crate::trip::history::{HistoryBasis, HistoryKind, TypicalDay};
    use crate::trip::model::tests::{battery as trip_battery, trip as sample_trip};
    use whether_core::builder::SnapshotBuilder;
    use whether_core::condition::Condition;
    use whether_core::model::{Hourly, ProviderId};
    use whether_core::storm::testdata as storm_data;
    use whether_core::units::{Celsius, Meters, Mps};

    macro_rules! fixture {
        ($file:literal) => {
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../tests/fixtures/raw/coops/",
                $file
            ))
        };
    }

    const MINUTE: i64 = 60;

    fn date(year: i32, month: u8, day: u8) -> LocalDate {
        LocalDate { year, month, day }
    }

    fn utc(y: i32, m: u8, d: u8) -> UnixTime {
        UnixTime(days_from_civil(y, m, d) * DAY_S)
    }

    fn ny_start(d: LocalDate) -> UnixTime {
        let tz = time::zone("America/New_York").unwrap();
        time::local_date_bounds(d, &tz).unwrap().0
    }

    fn coops_time(s: &str) -> UnixTime {
        let n = |r: std::ops::Range<usize>| s[r].parse::<i64>().unwrap();
        let day = days_from_civil(n(0..4) as i32, n(5..7) as u8, n(8..10) as u8);
        UnixTime(day * DAY_S + n(11..13) * 3600 + n(14..16) * MINUTE)
    }

    fn battery_harmonics() -> TideHarmonics {
        let v: Value =
            serde_json::from_str(fixture!("coops-mdapi-8518750-harcon-metric.json")).unwrap();
        let constituents = v["HarmonicConstituents"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| HarmonicConstituent {
                name: c["name"].as_str().unwrap().to_owned(),
                amplitude_m: c["amplitude"].as_f64().unwrap() as f32,
                phase_gmt_deg: c["phase_GMT"].as_f64().unwrap() as f32,
            })
            .collect();
        let d: Value =
            serde_json::from_str(fixture!("coops-mdapi-8518750-datums-metric.json")).unwrap();
        let values: BTreeMap<String, f32> = d["datums"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|d| Some((d["name"].as_str()?.to_owned(), d["value"].as_f64()? as f32)))
            .collect();
        TideHarmonics::from_datum_table(
            "8518750",
            constituents,
            &DatumTable::new("8518750", values),
            VerticalDatum::Mllw,
        )
        .unwrap()
    }

    fn tarrytown_offsets() -> SubordinateTideOffsets {
        let v: Value =
            serde_json::from_str(fixture!("coops-mdapi-8518919-tidepredoffsets.json")).unwrap();
        let ratio = |key: &str| HeightAdjustment::Ratio(v[key].as_f64().unwrap() as f32);
        let minutes = |key: &str| i32::try_from(v[key].as_i64().unwrap()).unwrap();
        SubordinateTideOffsets {
            reference_id: v["refStationId"].as_str().unwrap().to_owned(),
            high_time_min: minutes("timeOffsetHighTide"),
            low_time_min: minutes("timeOffsetLowTide"),
            high_height: ratio("heightOffsetHighTide"),
            low_height: ratio("heightOffsetLowTide"),
        }
    }

    fn hilo(json: &str) -> Vec<TideEvent> {
        let v: Value = serde_json::from_str(json).unwrap();
        v["predictions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| TideEvent {
                time: coops_time(p["t"].as_str().unwrap()),
                kind: match p["type"].as_str().unwrap() {
                    "H" => TideKind::High,
                    _ => TideKind::Low,
                },
                height_m: Some(p["v"].as_str().unwrap().parse().unwrap()),
            })
            .collect()
    }

    fn base(id: &str, kind: StationKind) -> Provenance {
        Provenance {
            source: SourceId::COOPS,
            station: Some(StationRef {
                source: SourceId::COOPS,
                id: id.into(),
                name: id.into(),
                kind,
                latitude: 40.7,
                longitude: -74.0,
                bin: None,
                reference_id: None,
            }),
            distance_m: None,
            relation: Relation::SameWaterBody,
            observed_at: utc(2026, 11, 8),
            kind: ValueKind::Predicted,
            quality: Quality::Provisional,
        }
    }

    fn trip_on(first: LocalDate, last: LocalDate) -> Trip {
        Trip {
            first,
            last,
            limits: None,
            ..sample_trip()
        }
    }

    fn inputs_for(trip: &Trip, input: PlaceInputs) -> BTreeMap<PlaceKey, PlaceInputs> {
        BTreeMap::from([(PlaceKey::of(&trip.places[0]), input)])
    }

    fn weather(from: i64, hours: usize) -> whether_core::model::WeatherSnapshot {
        let mut b = SnapshotBuilder::new(ProviderId::OpenMeteo);
        b.set_location(whether_core::testdata::sample_location());
        b.set_hourly(
            (0..hours)
                .map(|i| Hourly {
                    time: UnixTime(from + i64::try_from(i).unwrap() * HOUR_S),
                    condition: Condition::PartlyCloudy,
                    temperature: Some(Celsius(12.0)),
                    wind_speed: Some(Mps(3.0)),
                    wind_gust: Some(Mps(4.5)),
                    visibility: Some(Meters(20_000.0)),
                    ..Hourly::default()
                })
                .collect(),
        );
        b.finish(UnixTime(from)).unwrap()
    }

    fn series(
        source: SourceId,
        kind: ValueKind,
        from: i64,
        step: i64,
        values: &[f32],
    ) -> Series<f32> {
        let prov = Provenance {
            source,
            station: None,
            distance_m: None,
            relation: Relation::SameWaterBody,
            observed_at: UnixTime(from),
            kind,
            quality: Quality::Provisional,
        };
        Series::new(
            prov,
            values
                .iter()
                .enumerate()
                .map(|(i, v)| Sample::new(UnixTime(from + step * i64::try_from(i).unwrap()), *v))
                .collect(),
        )
    }

    fn band(median: f32) -> Band {
        Band {
            p10: median * 0.5,
            median,
            p90: median * 2.0,
            basis: HistoryBasis {
                source: SourceId::USGS,
                kind: HistoryKind::Station,
                years: Some(YearRange {
                    first: 1991,
                    last: 2020,
                }),
                years_sampled: 29,
                percentiles: None,
            },
        }
    }

    fn history(first: LocalDate, days: u8) -> TripHistory {
        let mut h = TripHistory {
            flow: Coverage::Available,
            water_temp: Coverage::Available,
            ..TripHistory::default()
        };
        for n in 0..i64::from(days) {
            let (year, month, day) =
                whether_core::time::civil_from_days(time::day_number(first) + n);
            h.days.insert(
                LocalDate { year, month, day },
                TypicalDay {
                    flow: Some(band(20.0)),
                    water_temp: Some(band(9.0)),
                    ..TypicalDay::default()
                },
            );
        }
        h
    }

    fn assert_events(label: &str, ours: &[TideEvent], theirs: &[TideEvent]) {
        assert_eq!(ours.len(), theirs.len(), "{label}: count");
        for (a, b) in ours.iter().zip(theirs) {
            assert_eq!(a.kind, b.kind, "{label}: kind at {:?}", b.time);
            let dt = (a.time.0 - b.time.0).abs() / MINUTE;
            let dh = (a.height_m.unwrap() - b.height_m.unwrap()).abs();
            assert!(
                dt <= 6 && dh <= 0.04,
                "{label}: {dt} min, {dh} m at {:?}",
                b.time
            );
        }
    }

    fn all_events(grid: &PlaceGrid) -> Vec<TideEvent> {
        grid.days
            .iter()
            .flat_map(|d| d.tides.value.iter().flat_map(|t| t.events.clone()))
            .collect()
    }

    fn harmonic_input() -> PlaceInputs {
        PlaceInputs {
            tide_constants: Some(TideConstants::Harmonic {
                harmonics: battery_harmonics(),
                base: base("8518750", StationKind::TideHarmonic),
            }),
            ..PlaceInputs::default()
        }
    }

    fn only(grid: &TripGrid) -> &PlaceGrid {
        grid.places.values().next().unwrap()
    }

    #[test]
    fn a_three_day_battery_trip_matches_the_published_highs_and_lows() {
        let trip = trip_on(date(2026, 11, 8), date(2026, 11, 10));
        let now = UnixTime(ny_start(date(2026, 11, 8)).0 - HOUR_S);
        let grid = build(
            &trip,
            &inputs_for(&trip, harmonic_input()),
            &GridConfig::default(),
            now,
        );
        let place = only(&grid);
        assert_eq!(place.days.len(), 3);
        assert_eq!(place.today, Some(date(2026, 11, 7)));
        let (from, to) = (ny_start(date(2026, 11, 8)), ny_start(date(2026, 11, 11)));
        let theirs: Vec<_> = hilo(fixture!("coops-8518750-hilo-2026-11-08.json"))
            .into_iter()
            .filter(|e| e.time >= from && e.time < to)
            .collect();
        assert!(theirs.len() >= 10);
        assert_events("8518750", &all_events(place), &theirs);
        for day in &place.days {
            let tides = &day.tides;
            assert_eq!(tides.basis, Basis::Certain);
            let t = tides.value.as_ref().unwrap();
            assert_eq!(t.datum, Some(VerticalDatum::Mllw));
            assert_eq!(t.kind, TideSourceKind::Computed);
            assert_eq!(tides.source, Some(SourceId::COOPS_HARMONICS));
            assert_eq!(day.sun.basis, Basis::Certain);
            assert_eq!(day.moon.basis, Basis::Certain);
            assert_eq!(day.solunar.basis, Basis::Certain);
            // Without a snapshot there is no forecast, history or flow to claim.
            assert_eq!(
                day.verdict.basis,
                Basis::Unavailable {
                    reason: Why::NoForecast
                }
            );
            assert_eq!(
                day.flow.basis,
                Basis::Unavailable {
                    reason: Why::NoSource
                }
            );
            // Windows run on tide, light and solunar alone, and say so.
            let w = day.windows.value.as_ref().unwrap();
            assert!(w.certain_only);
            assert_eq!(day.windows.basis, Basis::Certain);
            assert!(!w.windows.is_empty());
        }
    }

    #[test]
    fn a_subordinate_station_applies_its_offsets() {
        let trip = trip_on(date(2026, 11, 8), date(2026, 11, 10));
        let input = PlaceInputs {
            tide_constants: Some(TideConstants::Subordinate {
                reference: battery_harmonics(),
                offsets: tarrytown_offsets(),
                base: base("8518919", StationKind::TideSubordinate),
            }),
            ..PlaceInputs::default()
        };
        let now = utc(2026, 11, 1);
        let grid = build(
            &trip,
            &inputs_for(&trip, input),
            &GridConfig::default(),
            now,
        );
        let place = only(&grid);
        let (from, to) = (ny_start(date(2026, 11, 8)), ny_start(date(2026, 11, 11)));
        let theirs: Vec<_> = hilo(fixture!("coops-8518919-hilo-2026-11-08.json"))
            .into_iter()
            .filter(|e| e.time >= from && e.time < to)
            .collect();
        assert_events("8518919", &all_events(place), &theirs);
        let t = place.days[0].tides.value.as_ref().unwrap();
        assert_eq!(t.kind, TideSourceKind::SubordinateStation);
        assert_eq!(t.datum, Some(VerticalDatum::Mllw));
    }

    fn forecast_snapshot(now: UnixTime) -> SpotSnapshot {
        let mut s = SpotSnapshot::new(sample::spot(), now);
        s.weather = Some(weather(now.0, 96));
        s
    }

    #[test]
    fn sixty_days_out_has_certain_tides_typical_flow_and_no_verdict() {
        let first = date(2027, 1, 7);
        let trip = trip_on(first, first);
        let now = utc(2026, 11, 8);
        let input = PlaceInputs {
            snapshot: Some(forecast_snapshot(now)),
            history: Some(history(first, 1)),
            river: RiverRanges {
                best_min_m3s: Some(10.0),
                best_max_m3s: Some(30.0),
                wadeable_below_m3s: Some(15.0),
                blown_out_above_m3s: Some(100.0),
            },
            species_water_temp_c: Some([12.0, 20.0]),
            ..harmonic_input()
        };
        let grid = build(
            &trip,
            &inputs_for(&trip, input),
            &GridConfig::default(),
            now,
        );
        let day = &only(&grid).days[0];
        assert_eq!(day.tides.basis, Basis::Certain);
        assert!(day.tides.value.is_some());
        assert_eq!(
            day.flow.basis,
            Basis::Typical {
                years: Some(YearRange {
                    first: 1991,
                    last: 2020
                }),
                years_sampled: 29
            }
        );
        let flow = day.flow.value.as_ref().unwrap();
        assert_eq!(
            (flow.m3s, flow.p10_m3s, flow.p90_m3s),
            (20.0, Some(10.0), Some(40.0))
        );
        assert_eq!(flow.status, FlowStatus::InBest);
        assert_eq!(flow.wadeable, Some(false));
        let temp = day.water_temp.value.as_ref().unwrap();
        assert_eq!((temp.celsius, temp.status), (9.0, Some(TempStatus::Below)));
        assert!(matches!(day.water_temp.basis, Basis::Typical { .. }));
        assert_eq!(
            day.verdict.basis,
            Basis::Unavailable {
                reason: Why::BeyondForecast
            }
        );
        assert!(day.verdict.value.is_none());
        let w = day.windows.value.as_ref().unwrap();
        assert!(w.certain_only);
        assert_eq!(day.windows.basis, Basis::Certain);
        assert!(w.windows.iter().all(|w| w.matched.iter().all(|m| matches!(
            m.factor,
            super::super::windows::Factor::TideStage
                | super::super::windows::Factor::CurrentStrength
                | super::super::windows::Factor::LowLight
                | super::super::windows::Factor::Solunar
        ))));
    }

    #[test]
    fn a_day_inside_the_forecast_is_a_forecast_and_flow_comes_from_its_series_alone() {
        let first = date(2026, 11, 8);
        let trip = trip_on(first, date(2026, 11, 12));
        let start = ny_start(first);
        let now = UnixTime(start.0 - HOUR_S);
        let mut snap = SpotSnapshot::new(sample::spot(), now);
        snap.weather = Some(weather(start.0, 48));
        // Observed discharge must not leak into the forecast cell; the forecast covers day one only.
        snap.water.discharge_m3s = Some(series(
            SourceId::USGS,
            ValueKind::Observed,
            start.0 - 6 * HOUR_S,
            HOUR_S,
            &[500.0; 6],
        ));
        snap.water.discharge_forecast_m3s = Some(series(
            SourceId::NWPS,
            ValueKind::Modelled,
            start.0,
            6 * HOUR_S,
            &[10.0, 20.0, 30.0, 40.0],
        ));
        snap.marine.sea_surface_temp_forecast_c = Some(series(
            SourceId::OPEN_METEO_MARINE,
            ValueKind::Modelled,
            start.0,
            3 * HOUR_S,
            &[10.0, 12.0, 14.0, 16.0],
        ));
        let input = PlaceInputs {
            snapshot: Some(snap),
            history: Some(history(first, 5)),
            species_water_temp_c: Some([11.0, 14.0]),
            ..harmonic_input()
        };
        let grid = build(
            &trip,
            &inputs_for(&trip, input),
            &GridConfig::default(),
            now,
        );
        let days = &only(&grid).days;
        assert_eq!(days.len(), 5);
        let issued = Basis::Forecast { issued: now };
        assert_eq!(days[0].verdict.basis, issued);
        assert_eq!(days[0].verdict.value.as_ref().unwrap().verdict, Verdict::Go);
        assert_eq!(days[0].flow.basis, issued);
        assert_eq!(days[0].flow.value.as_ref().unwrap().m3s, 25.0);
        assert_eq!(days[0].flow.source, Some(SourceId::NWPS));
        assert_eq!(days[0].water_temp.basis, issued);
        let t = days[0].water_temp.value.as_ref().unwrap();
        assert_eq!((t.celsius, t.status), (13.0, Some(TempStatus::Within)));
        // Day two: the 48-hour weather ends with the day, and the forecast series are over.
        assert_eq!(days[1].verdict.basis, issued);
        assert!(matches!(days[1].flow.basis, Basis::Typical { .. }));
        assert!(matches!(days[1].water_temp.basis, Basis::Typical { .. }));
        // Day three is past the weather: nothing claimed, history stays labelled as history.
        assert_eq!(
            days[2].verdict.basis,
            Basis::Unavailable {
                reason: Why::BeyondForecast
            }
        );
        assert!(matches!(days[2].flow.basis, Basis::Typical { .. }));
        // Forecast factors take part only where their series reach.
        assert!(!days[0].windows.value.as_ref().unwrap().certain_only);
        assert_eq!(days[0].windows.basis, issued);
        assert!(days[4].windows.value.as_ref().unwrap().certain_only);
    }

    #[test]
    fn no_history_and_no_source_are_told_apart() {
        let first = date(2027, 1, 7);
        let trip = trip_on(first, first);
        let now = utc(2026, 11, 8);
        let mut h = history(first, 1);
        h.days.clear();
        h.water_temp = Coverage::NotRequested;
        h.sst = Coverage::NotRequested;
        let input = PlaceInputs {
            history: Some(h),
            ..PlaceInputs::default()
        };
        let grid = build(
            &trip,
            &inputs_for(&trip, input),
            &GridConfig::default(),
            now,
        );
        let day = &only(&grid).days[0];
        assert_eq!(
            day.flow.basis,
            Basis::Unavailable {
                reason: Why::NoHistory
            }
        );
        assert_eq!(
            day.water_temp.basis,
            Basis::Unavailable {
                reason: Why::NoSource
            }
        );
        assert!(day.flow.value.is_none() && day.water_temp.value.is_none());
    }

    #[test]
    fn model_sea_level_has_times_and_no_heights() {
        let first = date(2026, 11, 8);
        let trip = trip_on(first, first);
        let start = ny_start(first);
        let now = UnixTime(start.0 - HOUR_S);
        let mut snap = forecast_snapshot(now);
        snap.marine.tide = Some(TideSeries {
            source_kind: TideSourceKind::ModelMean,
            datum: VerticalDatum::Mllw,
            events: vec![
                TideEvent {
                    time: UnixTime(start.0 + 3 * HOUR_S),
                    kind: TideKind::High,
                    height_m: Some(0.4),
                },
                TideEvent {
                    time: UnixTime(start.0 + 9 * HOUR_S),
                    kind: TideKind::Low,
                    height_m: None,
                },
            ],
            curve: None,
            observed: None,
            provenance: base("model", StationKind::TideHarmonic),
        });
        let input = PlaceInputs {
            snapshot: Some(snap),
            ..PlaceInputs::default()
        };
        let grid = build(
            &trip,
            &inputs_for(&trip, input),
            &GridConfig::default(),
            now,
        );
        let day = &only(&grid).days[0];
        assert_eq!(day.tides.basis, Basis::Forecast { issued: now });
        let t = day.tides.value.as_ref().unwrap();
        assert_eq!(t.datum, None);
        assert_eq!(t.kind, TideSourceKind::ModelMean);
        assert!(t.events.iter().all(|e| e.height_m.is_none()));
        assert!(!day.windows.value.as_ref().unwrap().certain_only);
    }

    #[test]
    fn storm_days_carry_a_marker_and_the_no_go_even_past_the_weather() {
        let outlook = storm_data::noreaster();
        let event = whether_core::storm::analysis::all_events(&outlook)
            .into_iter()
            .find(whether_core::storm::analysis::is_significant)
            .unwrap();
        let now = storm_data::STORM_NOW;
        let tz = time::zone("America/New_York").unwrap();
        let mid = UnixTime(i64::midpoint(event.start.0, event.end.0));
        let storm_day = time::local_date(mid, &tz).unwrap();
        let mut snap = SpotSnapshot::new(sample::spot(), now);
        snap.weather = Some(weather(now.0, 12));
        snap.storm = Some(StormWatch::from_outlook(now, outlook, Vec::new()));
        let trip = trip_on(time::local_date(now, &tz).unwrap(), storm_day);
        let input = PlaceInputs {
            snapshot: Some(snap),
            ..PlaceInputs::default()
        };
        let grid = build(
            &trip,
            &inputs_for(&trip, input),
            &GridConfig::default(),
            now,
        );
        let day = only(&grid).days.last().unwrap();
        assert_eq!(day.date, storm_day);
        assert_eq!(day.verdict.basis, Basis::Forecast { issued: now });
        let v = day.verdict.value.as_ref().unwrap();
        assert_eq!(v.verdict, Verdict::NoGo);
        assert!(v.storm.iter().all(|r| r.check == Check::StormAhead));
        assert!(!v.storm.is_empty());
        assert!(v.reasons.iter().all(|r| r.check != Check::StormAhead));
    }

    #[test]
    fn two_time_zones_have_two_todays_and_past_days_claim_nothing() {
        let mut trip = trip_on(date(2026, 11, 7), date(2026, 11, 8));
        let mut la = trip_battery();
        la.spot_id = "spot-la".into();
        la.name = "Santa Monica".into();
        la.time_zone = "America/Los_Angeles".into();
        la.latitude = Some(34.0);
        la.longitude = Some(-118.5);
        trip.places.push(la);
        // 06:00 UTC on the 8th: already the 8th in New York, still the 7th in Los Angeles.
        let now = UnixTime(utc(2026, 11, 8).0 + 6 * HOUR_S);
        let snap = |k: &TripPlace| {
            let mut s = SpotSnapshot::new(sample::spot(), now);
            s.spot.time_zone.clone_from(&k.time_zone);
            s.weather = Some(weather(now.0 - 30 * HOUR_S, 96));
            PlaceInputs {
                snapshot: Some(s),
                ..PlaceInputs::default()
            }
        };
        let inputs: BTreeMap<_, _> = trip
            .places
            .iter()
            .map(|p| (PlaceKey::of(p), snap(p)))
            .collect();
        let grid = build(&trip, &inputs, &GridConfig::default(), now);
        let ny = &grid.places[&PlaceKey("spot-battery".into())];
        let la = &grid.places[&PlaceKey("spot-la".into())];
        assert_eq!(ny.today, Some(date(2026, 11, 8)));
        assert_eq!(la.today, Some(date(2026, 11, 7)));
        assert_eq!((ny.index, la.index), (0, 1));
        // The 7th is over in New York and still running in Los Angeles.
        assert_eq!(
            ny.days[0].verdict.basis,
            Basis::Unavailable { reason: Why::Past }
        );
        assert_eq!(
            ny.days[0].windows.basis,
            Basis::Unavailable { reason: Why::Past }
        );
        assert!(matches!(la.days[0].verdict.basis, Basis::Forecast { .. }));
        assert!(matches!(
            la.days[0].windows.basis,
            Basis::Certain | Basis::Forecast { .. }
        ));
        assert!(la.days[0].sun.value.is_some());
    }

    #[test]
    fn area_only_places_have_no_sky_and_bad_places_no_days() {
        let mut trip = trip_on(date(2026, 11, 8), date(2026, 11, 8));
        trip.places[0].latitude = None;
        trip.places[0].longitude = None;
        let now = utc(2026, 11, 1);
        let grid = build(
            &trip,
            &inputs_for(&trip, harmonic_input()),
            &GridConfig::default(),
            now,
        );
        let day = &only(&grid).days[0];
        for why in [
            &day.sun.basis,
            &day.moon.basis,
            &day.solunar.basis,
            &day.windows.basis,
        ] {
            assert_eq!(
                *why,
                Basis::Unavailable {
                    reason: Why::NoLocation
                }
            );
        }
        assert_eq!(day.tides.basis, Basis::Certain);

        let mut bad = trip_on(date(2026, 11, 9), date(2026, 11, 8));
        let grid = build(&bad, &BTreeMap::new(), &GridConfig::default(), now);
        assert_eq!(only(&grid).problem, Some(GridProblem::Dates));
        assert!(only(&grid).days.is_empty());
        bad.last = date(2026, 12, 20);
        bad.first = date(2026, 11, 8);
        let grid = build(&bad, &BTreeMap::new(), &GridConfig::default(), now);
        assert_eq!(only(&grid).problem, Some(GridProblem::Dates));
        bad.last = bad.first;
        bad.places[0].time_zone = "Mars/Olympus".into();
        let grid = build(&bad, &BTreeMap::new(), &GridConfig::default(), now);
        assert_eq!(only(&grid).problem, Some(GridProblem::TimeZone));
    }

    #[test]
    fn hidden_solunar_is_disabled_not_empty() {
        let trip = trip_on(date(2026, 11, 8), date(2026, 11, 8));
        let cfg = GridConfig {
            windows: WindowParams {
                show_solunar: false,
                ..WindowParams::default()
            },
            ..GridConfig::default()
        };
        let grid = build(&trip, &BTreeMap::new(), &cfg, utc(2026, 11, 1));
        assert_eq!(
            only(&grid).days[0].solunar.basis,
            Basis::Unavailable {
                reason: Why::Disabled
            }
        );
    }

    #[test]
    fn flow_status_classifies_against_the_users_ranges() {
        let r = RiverRanges {
            wadeable_below_m3s: None,
            best_min_m3s: Some(10.0),
            best_max_m3s: Some(30.0),
            blown_out_above_m3s: Some(100.0),
        };
        assert_eq!(flow_status(5.0, &r), FlowStatus::BelowBest);
        assert_eq!(flow_status(20.0, &r), FlowStatus::InBest);
        assert_eq!(flow_status(50.0, &r), FlowStatus::AboveBest);
        assert_eq!(flow_status(150.0, &r), FlowStatus::BlownOut);
        assert_eq!(
            flow_status(5.0, &RiverRanges::default()),
            FlowStatus::Unclassified
        );
        let only_max = RiverRanges {
            best_max_m3s: Some(30.0),
            ..RiverRanges::default()
        };
        assert_eq!(flow_status(5.0, &only_max), FlowStatus::InBest);
    }

    /// One line per day and cell: what it is and what it rests on.
    fn render(grid: &TripGrid) -> String {
        let basis = |b: &Basis| match b {
            Basis::Certain => "certain".to_owned(),
            Basis::Forecast { .. } => "forecast".to_owned(),
            Basis::Typical { years_sampled, .. } => format!("typical({years_sampled} years)"),
            Basis::Unavailable { reason } => format!("unavailable({reason:?})"),
        };
        let mut out = String::new();
        for (key, place) in &grid.places {
            out += &format!("{} {} today {:?}\n", key.0, place.time_zone, place.today);
            for d in &place.days {
                out += &format!("{}-{:02}-{:02}\n", d.date.year, d.date.month, d.date.day);
                out += &format!("  verdict {}", basis(&d.verdict.basis));
                if let Some(v) = &d.verdict.value {
                    out += &format!(" {:?} reasons {}", v.verdict, v.reasons.len());
                }
                out += "\n";
                out += &format!("  windows {}", basis(&d.windows.basis));
                if let Some(w) = &d.windows.value {
                    out += &format!(" certain_only {} count {}", w.certain_only, w.windows.len());
                }
                out += "\n";
                out += &format!("  tides {}", basis(&d.tides.basis));
                if let Some(t) = &d.tides.value {
                    out += &format!(" {:?} {:?}", t.kind, t.datum);
                    for e in &t.events {
                        out += &format!(
                            " {:?}@{} {:.2}",
                            e.kind,
                            e.time.0,
                            e.height_m.unwrap_or(f32::NAN)
                        );
                    }
                }
                out += "\n";
                out += &format!("  flow {}", basis(&d.flow.basis));
                if let Some(f) = &d.flow.value {
                    out += &format!(" {:.1} {:?} wadeable {:?}", f.m3s, f.status, f.wadeable);
                }
                out += "\n";
                out += &format!("  water_temp {}", basis(&d.water_temp.basis));
                if let Some(t) = &d.water_temp.value {
                    out += &format!(" {:.1} {:?}", t.celsius, t.status);
                }
                out += "\n";
                out += &format!(
                    "  sun {} moon {} solunar {}",
                    basis(&d.sun.basis),
                    basis(&d.moon.basis),
                    basis(&d.solunar.basis)
                );
                if let Some(m) = &d.moon.value {
                    out += &format!(" {:?}", m.phase);
                }
                out += "\n";
            }
        }
        out
    }

    #[test]
    fn the_battery_week_grid() {
        let first = date(2026, 11, 8);
        let trip = trip_on(first, date(2026, 11, 10));
        let start = ny_start(first);
        let now = UnixTime(start.0 - HOUR_S);
        let mut snap = SpotSnapshot::new(sample::spot(), now);
        snap.weather = Some(weather(start.0, 48));
        snap.water.discharge_forecast_m3s = Some(series(
            SourceId::NWPS,
            ValueKind::Modelled,
            start.0,
            HOUR_S,
            &(0..24).map(|i| 10.0 + i as f32).collect::<Vec<_>>(),
        ));
        let input = PlaceInputs {
            snapshot: Some(snap),
            history: Some(history(first, 3)),
            river: RiverRanges {
                best_min_m3s: Some(10.0),
                best_max_m3s: Some(30.0),
                wadeable_below_m3s: Some(15.0),
                ..RiverRanges::default()
            },
            species_water_temp_c: Some([12.0, 20.0]),
            ..harmonic_input()
        };
        let grid = build(
            &trip,
            &inputs_for(&trip, input),
            &GridConfig::default(),
            now,
        );
        insta::assert_snapshot!("battery_three_days", render(&grid));
        let json = serde_json::to_string(&grid).unwrap();
        assert_eq!(serde_json::from_str::<TripGrid>(&json).unwrap(), grid);
    }
}
