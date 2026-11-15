//! Open-Meteo Marine: waves, sea surface temperature, currents and modelled sea level.
//!
//! [`OpenMeteoMarine`] plans one request per spot: hourly waves and sea surface temperature,
//! 15-minute ocean currents and sea level. The values are a model's forecast on a 0.08° grid,
//! so they are [`ValueKind::Modelled`], carry [`Relation::Gridded`] and the distance from the spot
//! to the grid cell the response names, and go to the forecast measurements only
//! ([`Measurement::WavesForecast`], [`Measurement::SeaSurfaceTempForecast`]).
//!
//! Sea level is not a tide table. It becomes a [`TideSeries`] of kind
//! [`TideSourceKind::ModelMean`]: the curve, and high and low events found in it with times
//! only. A session uses it only where no CO-OPS station is within range.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;
use underbite_core::datum::VerticalDatum;
use underbite_core::model::{
    Measurement, Requirements, TideEvent, TideKind, TideSeries, TideSourceKind, WaveDirection,
    WaveSample, WaveSeries, WaveTrain,
};
use underbite_core::provenance::{Attribution, Provenance, Quality, Relation, SourceId, ValueKind};
use underbite_core::series::{Sample, Series, Vector};
use underbite_core::time::{UnixTime, UtcOffset, parse_local_iso};
use underbite_core::units::{Canonical, to_canonical};

use crate::request::{Place, QueryBuilder, Request, Resource, Response, RoundedPoint, Target};
use crate::source::{
    Budget, FetchContext, FragmentValue, Fragments, Period, Source, SourceError, check_response,
};

const BASE: &str = "https://marine-api.open-meteo.com/v1/marine";

/// Calls a day this source may make. Open-Meteo's free limit is per IP address and shared with
/// the weather and storm requests, so the share is small.
const DAILY_BUDGET: u32 = 200;

/// Seconds a response may be reused.
const MAX_AGE_S: u32 = 1_800;

/// Most days Open-Meteo returns ahead, and behind.
const MAX_FORECAST_DAYS: i64 = 16;
const MAX_PAST_DAYS: i64 = 92;

const HOURLY_WAVES: &str = "wave_height,wave_direction,wave_period,wind_wave_height,\
wind_wave_direction,wind_wave_period,swell_wave_height,swell_wave_direction,swell_wave_period,\
secondary_swell_wave_height,secondary_swell_wave_period,secondary_swell_wave_direction";
const SST: &str = "sea_surface_temperature";
const CURRENT: &str = "ocean_current_velocity,ocean_current_direction";
const SEA_LEVEL: &str = "sea_level_height_msl";

const MEASUREMENTS: [Measurement; 4] = [
    Measurement::WavesForecast,
    Measurement::SeaSurfaceTempForecast,
    Measurement::OceanCurrent,
    Measurement::Tide,
];

/// The Open-Meteo Marine API as a source. It holds no settings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OpenMeteoMarine;

fn point(target: &Target) -> Result<RoundedPoint, SourceError> {
    match target.place {
        Place::Point(p) => Ok(p),
        _ => Err(SourceError::Unsupported(
            "Open-Meteo Marine needs a point".into(),
        )),
    }
}

/// Whole days in `seconds`, rounded up and clamped to `max`.
fn days(seconds: i64, max: i64) -> i64 {
    ((seconds.max(0) + 86_399) / 86_400).min(max)
}

impl Source for OpenMeteoMarine {
    fn id(&self) -> SourceId {
        SourceId::OPEN_METEO_MARINE
    }

    fn attribution(&self) -> Attribution {
        Attribution {
            source: SourceId::OPEN_METEO_MARINE,
            credit: "Weather data by Open-Meteo.com. Marine models: Copernicus Marine Service, \
                     ECMWF, NOAA NCEP, DWD, Copernicus Climate Change Service. Modelled, not \
                     suitable for coastal navigation."
                .into(),
            url: Some("https://open-meteo.com/".into()),
        }
    }

    fn measurements(&self) -> &[Measurement] {
        &MEASUREMENTS
    }

    fn budget(&self) -> Option<Budget> {
        Some(Budget {
            period: Period::Day,
            limit: DAILY_BUDGET,
        })
    }

    fn plan(
        &self,
        target: &Target,
        requirements: &Requirements,
        ctx: &FetchContext,
    ) -> Result<Vec<Request>, SourceError> {
        let point = point(target)?;
        let wants = |m: Measurement| {
            target.measurements.contains(&m)
                && (requirements.measurements.is_empty() || requirements.measurements.contains(&m))
        };
        let mut hourly = Vec::new();
        let mut minutely = Vec::new();
        let mut served = BTreeSet::new();
        for (m, columns, fine) in [
            (Measurement::WavesForecast, HOURLY_WAVES, false),
            (Measurement::SeaSurfaceTempForecast, SST, false),
            (Measurement::OceanCurrent, CURRENT, true),
            (Measurement::Tide, SEA_LEVEL, true),
        ] {
            if wants(m) {
                served.insert(m);
                if fine { &mut minutely } else { &mut hourly }.push(columns);
            }
        }
        if served.is_empty() {
            return Err(SourceError::Unsupported(format!(
                "Open-Meteo Marine offers none of {:?}",
                target.measurements
            )));
        }
        let (lat, lon) = point.fmt_param();
        let mut query = QueryBuilder::new(BASE)
            .param("latitude", lat)
            .param("longitude", lon);
        if !hourly.is_empty() {
            query = query.param("hourly", hourly.join(","));
        }
        if !minutely.is_empty() {
            query = query.param("minutely_15", minutely.join(","));
        }
        let forecast = days(target.window.to.0 - ctx.now.0, MAX_FORECAST_DAYS).max(1);
        let past = days(ctx.now.0 - target.window.from.0, MAX_PAST_DAYS);
        query = query.param("forecast_days", forecast.to_string());
        if past > 0 {
            query = query.param("past_days", past.to_string());
        }
        // `wind_speed_unit` is the parameter that sets the current's unit. Lengths and
        // temperatures are metric and Celsius by default.
        let url = query
            .param("timezone", "GMT")
            .param("wind_speed_unit", "ms")
            .finish();
        let target = Target {
            measurements: served,
            ..target.clone()
        };
        Ok(vec![
            Request::new(url, Resource::OpenMeteoMarine, target)
                .metered()
                .max_age_hint(MAX_AGE_S),
        ])
    }

    fn parse(
        &self,
        request: &Request,
        response: &Response<'_>,
        ctx: &FetchContext,
        out: &mut Fragments,
    ) -> Result<(), SourceError> {
        let body = check_response(request, response)?;
        let envelope: Envelope = serde_json::from_slice(body)
            .map_err(|e| parse_error(format!("not an Open-Meteo response: {e}")))?;
        let spot = point(&request.target)?.coordinates();
        let distance_m = match (envelope.latitude, envelope.longitude) {
            (Some(lat), Some(lon)) => Some(distance_m(spot.latitude, spot.longitude, lat, lon)),
            _ => None,
        };
        let provenance = Provenance {
            source: SourceId::OPEN_METEO_MARINE,
            station: None,
            distance_m,
            relation: Relation::Gridded,
            observed_at: ctx.now,
            kind: ValueKind::Modelled,
            quality: Quality::Provisional,
        };
        let hourly = envelope
            .hourly
            .map(|c| Block::new(c, envelope.hourly_units.unwrap_or_default()))
            .transpose()?;
        let fast = envelope
            .minutely_15
            .map(|c| Block::new(c, envelope.minutely_15_units.unwrap_or_default()))
            .transpose()?;
        if hourly.is_none() && fast.is_none() {
            return Err(parse_error("no hourly or minutely_15 block"));
        }
        // Currents and sea level come every 15 minutes where the request asked for it.
        let fine = fast.as_ref().or(hourly.as_ref());
        let mut any = false;
        for &m in &MEASUREMENTS {
            if !request.target.measurements.contains(&m) {
                continue;
            }
            let value = match m {
                Measurement::WavesForecast => hourly
                    .as_ref()
                    .map(|b| waves(b, &provenance))
                    .transpose()?
                    .flatten()
                    .map(FragmentValue::WaveSeries),
                Measurement::SeaSurfaceTempForecast => hourly
                    .as_ref()
                    .map(|b| b.scalar(SST, Kind::Temperature))
                    .transpose()?
                    .flatten()
                    .filter(|s| !s.is_empty())
                    .map(|s| FragmentValue::Series(Series::new(provenance.clone(), s))),
                Measurement::OceanCurrent => fine
                    .map(|b| current(b, &provenance))
                    .transpose()?
                    .flatten()
                    .map(FragmentValue::Vectors),
                _ => fine
                    .map(|b| sea_level(b, &provenance))
                    .transpose()?
                    .flatten()
                    .map(FragmentValue::TideSeries),
            };
            if let Some(value) = value {
                out.push(m, value)?;
                any = true;
            }
        }
        if any {
            Ok(())
        } else {
            // Land and out-of-coverage points answer 200 with every value null.
            Err(SourceError::NoData(Some(
                "the model has no values for this point".into(),
            )))
        }
    }
}

/// Great-circle distance in metres between two points.
fn distance_m(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f32 {
    const EARTH_RADIUS_M: f64 = 6_371_008.8;
    let (p1, p2) = (lat1.to_radians(), lat2.to_radians());
    let dlat = p2 - p1;
    let dlon = (lon2 - lon1).to_radians();
    let a = (dlat / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dlon / 2.0).sin().powi(2);
    (2.0 * EARTH_RADIUS_M * a.sqrt().min(1.0).asin()) as f32
}

#[derive(serde::Deserialize)]
struct Envelope {
    latitude: Option<f64>,
    longitude: Option<f64>,
    hourly: Option<BTreeMap<String, Vec<Value>>>,
    hourly_units: Option<BTreeMap<String, String>>,
    minutely_15: Option<BTreeMap<String, Vec<Value>>>,
    minutely_15_units: Option<BTreeMap<String, String>>,
}

/// What a column measures, to check its unit string and convert.
#[derive(Clone, Copy)]
enum Kind {
    Length,
    Speed,
    Temperature,
    Degrees,
    Seconds,
}

type Column = Option<Vec<Option<f32>>>;

fn parse_error(message: impl Into<String>) -> SourceError {
    SourceError::Parse(message.into())
}

/// A block of columns sharing one time axis.
struct Block {
    times: Vec<UnixTime>,
    columns: BTreeMap<String, Vec<Value>>,
    units: BTreeMap<String, String>,
}

impl Block {
    fn new(
        mut columns: BTreeMap<String, Vec<Value>>,
        units: BTreeMap<String, String>,
    ) -> Result<Self, SourceError> {
        let times = columns
            .remove("time")
            .ok_or_else(|| parse_error("no time column"))?
            .iter()
            .map(|t| {
                t.as_str()
                    .ok_or_else(|| parse_error("time is not text"))
                    .and_then(|s| {
                        parse_local_iso(s, UtcOffset(0)).map_err(|e| parse_error(e.to_string()))
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        if let Some((name, _)) = columns.iter().find(|(_, c)| c.len() != times.len()) {
            return Err(parse_error(format!("column {name} has the wrong length")));
        }
        Ok(Self {
            times,
            columns,
            units,
        })
    }

    /// A column in canonical units. `None` when the column is missing, `undefined` (the model
    /// does not provide it) or all null.
    fn column(&self, name: &str, kind: Kind) -> Result<Column, SourceError> {
        let Some(values) = self.columns.get(name) else {
            return Ok(None);
        };
        if values.iter().all(Value::is_null) {
            return Ok(None);
        }
        let unit = self
            .units
            .get(name)
            .map(String::as_str)
            .ok_or_else(|| parse_error(format!("no unit for {name}")))?;
        values
            .iter()
            .map(|v| match v {
                Value::Null => Ok(None),
                Value::Number(n) => {
                    let n = n
                        .as_f64()
                        .ok_or_else(|| parse_error("number out of range"))?;
                    convert(n as f32, unit, kind).map(Some)
                }
                _ => Err(parse_error(format!("{name} holds a non-number"))),
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Some)
    }

    /// A single-value column as samples, nulls left out.
    fn scalar(&self, name: &str, kind: Kind) -> Result<Option<Vec<Sample<f32>>>, SourceError> {
        Ok(self.column(name, kind)?.map(|c| {
            self.times
                .iter()
                .zip(c)
                .filter_map(|(t, v)| v.map(|v| Sample::new(*t, v)))
                .collect()
        }))
    }
}

/// `value` in `unit` as the canonical unit of `kind`. Degrees and seconds are not in the unit
/// table, so their exact strings are checked here.
fn convert(value: f32, unit: &str, kind: Kind) -> Result<f32, SourceError> {
    let wrong = || parse_error(format!("unexpected unit {unit:?}"));
    let exact = |expected: &str| {
        if unit == expected {
            Ok(value)
        } else {
            Err(wrong())
        }
    };
    match kind {
        Kind::Degrees => exact("°"),
        Kind::Seconds => exact("s"),
        Kind::Length | Kind::Speed | Kind::Temperature => {
            let canonical = to_canonical(value, unit).map_err(|e| parse_error(e.to_string()))?;
            match (canonical, kind) {
                (Canonical::Length(v), Kind::Length)
                | (Canonical::Speed(v), Kind::Speed)
                | (Canonical::Temperature(v), Kind::Temperature) => Ok(v),
                _ => Err(wrong()),
            }
        }
    }
}

fn at(column: &Column, i: usize) -> Option<f32> {
    column.as_ref().and_then(|c| c[i])
}

/// The columns of one wave train.
struct TrainColumns {
    height: Column,
    period: Column,
    direction: Column,
}

impl TrainColumns {
    fn read(block: &Block, prefix: &str) -> Result<Self, SourceError> {
        Ok(Self {
            height: block.column(&format!("{prefix}_height"), Kind::Length)?,
            period: block.column(&format!("{prefix}_period"), Kind::Seconds)?,
            direction: block.column(&format!("{prefix}_direction"), Kind::Degrees)?,
        })
    }

    /// The train at `i`. A train with no height and no period is not there: the GFS wave model
    /// fills the third swell with zeros where there is none.
    fn at(&self, i: usize) -> Option<WaveTrain> {
        let (h, p) = (at(&self.height, i), at(&self.period, i));
        if h.is_none_or(|h| h == 0.0) && p.is_none_or(|p| p == 0.0) {
            return None;
        }
        Some(WaveTrain {
            height_m: h,
            period_s: p,
            direction_from: at(&self.direction, i).map(WaveDirection::Degrees),
        })
    }
}

fn waves(block: &Block, provenance: &Provenance) -> Result<Option<WaveSeries>, SourceError> {
    let total = TrainColumns::read(block, "wave")?;
    let wind = TrainColumns::read(block, "wind_wave")?;
    let swells = [
        TrainColumns::read(block, "swell_wave")?,
        TrainColumns::read(block, "secondary_swell_wave")?,
        TrainColumns::read(block, "tertiary_swell_wave")?,
    ];
    let mut samples = Vec::new();
    for (i, time) in block.times.iter().enumerate() {
        let sample = WaveSample {
            significant_height_m: at(&total.height, i),
            mean_period_s: at(&total.period, i),
            direction_from_deg: at(&total.direction, i),
            wind_waves: wind.at(i),
            swells: swells.iter().filter_map(|s| s.at(i)).collect(),
            ..WaveSample::default()
        };
        if sample != WaveSample::default() {
            samples.push(Sample::new(*time, sample));
        }
    }
    Ok((!samples.is_empty()).then(|| Series::new(provenance.clone(), samples)))
}

fn current(block: &Block, provenance: &Provenance) -> Result<Option<Series<Vector>>, SourceError> {
    let speed = block.column("ocean_current_velocity", Kind::Speed)?;
    let toward = block.column("ocean_current_direction", Kind::Degrees)?;
    let samples: Vec<_> = block
        .times
        .iter()
        .enumerate()
        .filter_map(|(i, t)| {
            let value = Vector {
                speed_ms: at(&speed, i)?,
                toward_deg: at(&toward, i)?,
            };
            Some(Sample::new(*t, value))
        })
        .collect();
    Ok((!samples.is_empty()).then(|| Series::new(provenance.clone(), samples)))
}

fn sea_level(block: &Block, provenance: &Provenance) -> Result<Option<TideSeries>, SourceError> {
    let Some(samples) = block.scalar("sea_level_height_msl", Kind::Length)? else {
        return Ok(None);
    };
    if samples.is_empty() {
        return Ok(None);
    }
    Ok(Some(TideSeries {
        source_kind: TideSourceKind::ModelMean,
        datum: VerticalDatum::ModelMean,
        events: extremes(&samples),
        curve: Some(Series::new(provenance.clone(), samples)),
        observed: None,
        provenance: provenance.clone(),
    }))
}

/// High and low times of a curve: its turning points, without heights. A flat top counts once,
/// at its middle; a single-sample peak is placed by fitting a parabola through its neighbours.
fn extremes(samples: &[Sample<f32>]) -> Vec<TideEvent> {
    let mut events = Vec::new();
    let mut rising = 0i8;
    let mut plateau = 0usize;
    for i in 1..samples.len() {
        let step = match samples[i].value.partial_cmp(&samples[i - 1].value) {
            Some(std::cmp::Ordering::Greater) => 1,
            Some(std::cmp::Ordering::Less) => -1,
            _ => continue,
        };
        if rising != 0 && step != rising {
            events.push(TideEvent {
                time: turning_time(samples, plateau, i - 1),
                kind: if rising > 0 {
                    TideKind::High
                } else {
                    TideKind::Low
                },
                height_m: None,
            });
        }
        rising = step;
        plateau = i;
    }
    events
}

/// When the curve turns within `first..=last`, which hold the same value.
fn turning_time(samples: &[Sample<f32>], first: usize, last: usize) -> UnixTime {
    let (t0, t1) = (samples[first].time.0, samples[last].time.0);
    if first == last && first > 0 && last + 1 < samples.len() {
        let (a, b, c) = (
            f64::from(samples[first - 1].value),
            f64::from(samples[first].value),
            f64::from(samples[first + 1].value),
        );
        let denominator = a - 2.0 * b + c;
        if denominator != 0.0 {
            let offset = (0.5 * (a - c) / denominator).clamp(-0.5, 0.5);
            let step = (samples[first + 1].time.0 - samples[first - 1].time.0) as f64 / 2.0;
            return UnixTime(t0 + (offset * step).round() as i64);
        }
    }
    UnixTime(t0 + (t1 - t0) / 2)
}

#[cfg(test)]
mod tests {
    use underbite_core::provenance::Relation;

    use super::*;
    use crate::request::{Coordinates, Window};
    use crate::source::Fragment;
    use crate::testutil::fixture_bytes;

    const NOW: i64 = 1_794_096_000;

    fn ctx() -> FetchContext {
        FetchContext::new(UnixTime(NOW), "underbite/test")
    }

    fn target(lat: f64, lon: f64, ms: &[Measurement]) -> Target {
        Target {
            place: Place::Point(ctx().round(Coordinates {
                latitude: lat,
                longitude: lon,
            })),
            measurements: ms.iter().copied().collect(),
            window: Window {
                from: UnixTime(NOW),
                to: UnixTime(NOW + 2 * 86_400),
            },
            datum: None,
        }
    }

    fn all() -> [Measurement; 4] {
        MEASUREMENTS
    }

    fn run(fixture: &str, status: u16, ms: &[Measurement]) -> Result<Vec<Fragment>, SourceError> {
        run_at(40.70, -74.02, fixture, status, ms)
    }

    fn run_at(
        lat: f64,
        lon: f64,
        fixture: &str,
        status: u16,
        ms: &[Measurement],
    ) -> Result<Vec<Fragment>, SourceError> {
        let request = OpenMeteoMarine
            .plan(&target(lat, lon, ms), &Requirements::default(), &ctx())
            .unwrap()
            .remove(0);
        let bytes = fixture_bytes("open-meteo-marine", fixture);
        let mut out = Fragments::new();
        OpenMeteoMarine.parse(&request, &Response::new(status, &bytes), &ctx(), &mut out)?;
        Ok(out.into_parts().0)
    }

    fn find(fragments: &[Fragment], m: Measurement) -> &FragmentValue {
        &fragments.iter().find(|f| f.measurement == m).unwrap().value
    }

    fn close(a: f32, b: f32) {
        assert!((a - b).abs() < 0.005, "{a} vs {b}");
    }

    #[test]
    fn plans_one_metered_request() {
        let requests = OpenMeteoMarine
            .plan(
                &target(40.7033, -74.017, &all()),
                &Requirements::default(),
                &ctx(),
            )
            .unwrap();
        assert_eq!(requests.len(), 1);
        let r = &requests[0];
        assert_eq!(
            r.url,
            format!(
                "{BASE}?latitude=40.7&longitude=-74.02&hourly={HOURLY_WAVES},sea_surface_temperature\
                 &minutely_15={CURRENT},{SEA_LEVEL}&forecast_days=2&timezone=GMT&wind_speed_unit=ms"
            )
        );
        assert!(r.metered && r.required);
        assert_eq!(
            OpenMeteoMarine.budget(),
            Some(Budget {
                period: Period::Day,
                limit: DAILY_BUDGET
            })
        );
    }

    #[test]
    fn plan_asks_only_for_what_is_wanted_and_past_days() {
        let mut t = target(40.70, -74.02, &[Measurement::Tide]);
        t.window.from = UnixTime(NOW - 3 * 86_400 + 1);
        let r = OpenMeteoMarine
            .plan(&t, &Requirements::default(), &ctx())
            .unwrap()
            .remove(0);
        assert!(!r.url.contains("hourly="));
        assert!(r.url.contains("minutely_15=sea_level_height_msl"));
        assert!(r.url.contains("past_days=3"));
        let none = target(40.70, -74.02, &[Measurement::Salinity]);
        assert!(matches!(
            OpenMeteoMarine.plan(&none, &Requirements::default(), &ctx()),
            Err(SourceError::Unsupported(_))
        ));
    }

    #[test]
    fn new_york_hourly() {
        let frags = run("marine-40.70_-74.02-hourly-2d.json", 200, &all()).unwrap();
        let FragmentValue::WaveSeries(w) = find(&frags, Measurement::WavesForecast) else {
            panic!("waves")
        };
        assert_eq!(w.samples().len(), 48);
        let prov = w.provenance();
        assert_eq!(prov.relation, Relation::Gridded);
        assert_eq!(prov.kind, ValueKind::Modelled);
        assert_eq!(prov.source, SourceId::OPEN_METEO_MARINE);
        // The spot is about 8.5 km north of the cell 40.625, -74.04166.
        let dist = prov.distance_m.unwrap();
        assert!((8_000.0..9_000.0).contains(&dist), "{dist}");
        let first = &w.samples()[0];
        assert_eq!(first.time, UnixTime(NOW));
        assert!(first.value.significant_height_m.is_some());
        assert!(first.value.mean_period_s.is_some());
        assert!(first.value.wind_waves.is_some());
        assert_eq!(first.value.swells.len(), 2);

        let FragmentValue::Series(sst) = find(&frags, Measurement::SeaSurfaceTempForecast) else {
            panic!("sst")
        };
        assert_eq!(sst.samples().len(), 48);
        assert!((5.0..30.0).contains(&sst.samples()[0].value));

        // 0.2 to 0.7 km/h in the lower bay.
        let FragmentValue::Vectors(cur) = find(&frags, Measurement::OceanCurrent) else {
            panic!("current")
        };

        assert!(cur.samples().iter().all(|s| s.value.speed_ms < 0.2));

        let FragmentValue::TideSeries(tide) = find(&frags, Measurement::Tide) else {
            panic!("tide")
        };
        assert_eq!(tide.source_kind, TideSourceKind::ModelMean);
        assert_eq!(tide.datum, VerticalDatum::ModelMean);
        assert!(tide.events.len() >= 3);
        assert!(tide.events.iter().all(|e| e.height_m.is_none()));
        assert!(tide.events.windows(2).all(|w| w[0].kind != w[1].kind));
    }

    #[test]
    fn currents_are_converted_from_kmh() {
        let f = run(
            "marine-40.25_-73.16-hourly-2d.json",
            200,
            &[Measurement::OceanCurrent],
        )
        .unwrap();
        let FragmentValue::Vectors(c) = find(&f, Measurement::OceanCurrent) else {
            panic!("current")
        };
        // 1.0 to 1.5 km/h offshore is 0.28 to 0.42 m/s.
        let max = c
            .samples()
            .iter()
            .map(|s| s.value.speed_ms)
            .fold(0.0, f32::max);
        assert!((0.2..0.6).contains(&max), "{max}");
        assert!(
            c.samples()
                .iter()
                .all(|s| (0.0..360.0).contains(&s.value.toward_deg))
        );
    }

    #[test]
    fn offshore_44025_cell() {
        let f = run_at(
            40.25,
            -73.16,
            "marine-40.25_-73.16-hourly-2d.json",
            200,
            &all(),
        )
        .unwrap();
        let FragmentValue::WaveSeries(w) = find(&f, Measurement::WavesForecast) else {
            panic!("waves")
        };
        // The cell 40.291664, -73.12499 is about 5.5 km from the spot.
        let d = w.provenance().distance_m.unwrap();
        assert!((5_000.0..6_000.0).contains(&d), "{d}");
        assert_eq!(w.samples().len(), 48);
    }

    #[test]
    fn minutely_currents_and_sea_level() {
        let f = run(
            "marine-40.70_-74.02-minutely15-1d.json",
            200,
            &[Measurement::OceanCurrent, Measurement::Tide],
        )
        .unwrap();
        let FragmentValue::TideSeries(t) = find(&f, Measurement::Tide) else {
            panic!("tide")
        };
        let curve = t.curve.as_ref().unwrap();
        assert_eq!(curve.samples().len(), 96);
        assert_eq!(curve.samples()[1].time.0 - curve.samples()[0].time.0, 900);
        let FragmentValue::Vectors(c) = find(&f, Measurement::OceanCurrent) else {
            panic!("current")
        };
        assert_eq!(c.samples().len(), 96);
        insta::assert_json_snapshot!(
            "sea_level_events",
            t.events
                .iter()
                .map(|e| (e.time.0, e.kind))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn imperial_units_are_converted() {
        let f = run(
            "marine-40.25_-73.16-imperial-knots-1d.json",
            200,
            &[
                Measurement::WavesForecast,
                Measurement::SeaSurfaceTempForecast,
                Measurement::OceanCurrent,
            ],
        );
        // Direction columns are absent from that file, so the current has no samples.
        let f = f.unwrap();
        let FragmentValue::WaveSeries(w) = find(&f, Measurement::WavesForecast) else {
            panic!("waves")
        };
        let h = w.samples()[0].value.significant_height_m.unwrap();
        assert!((0.3..6.0).contains(&h), "{h}");
        let FragmentValue::Series(sst) = find(&f, Measurement::SeaSurfaceTempForecast) else {
            panic!("sst")
        };
        // 61.2 °F.
        close(sst.samples()[0].value, 16.222);
    }

    #[test]
    fn third_swell_zeros_are_not_a_train() {
        let f = run(
            "marine-40.25_-73.16-gfswave025-1d.json",
            200,
            &[Measurement::WavesForecast],
        )
        .unwrap();
        let FragmentValue::WaveSeries(w) = find(&f, Measurement::WavesForecast) else {
            panic!("waves")
        };
        // Early hours have two zero trains, dropped; later ones have a real third train.
        assert_eq!(w.samples()[0].value.swells.len(), 1);
        assert!(w.samples().iter().any(|s| s.value.swells.len() == 3));
    }

    #[test]
    fn land_is_no_data() {
        let e = run("marine-39.0_-98.0-land-hourly-1d.json", 200, &all()).unwrap_err();
        assert!(matches!(e, SourceError::NoData(_)), "{e:?}");
    }

    #[test]
    fn lake_erie_has_waves_only() {
        let f = run("marine-42.0_-81.5-lake-erie-hourly-1d.json", 200, &all()).unwrap();
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].measurement, Measurement::WavesForecast);
    }

    #[test]
    fn error_bodies_are_statuses() {
        for (name, text) in [
            (
                "marine-error-forecast-days.json",
                "Forecast days is invalid",
            ),
            ("marine-error-invalid-variable.json", "Invalid value"),
        ] {
            let e = run(name, 400, &all()).unwrap_err();
            let SourceError::Status { status, message } = e else {
                panic!("{e:?}")
            };
            assert_eq!(status, 400);
            assert!(message.unwrap().contains(text));
        }
        // An error body under 200 is still an error.
        let e = run("marine-error-forecast-days.json", 200, &all()).unwrap_err();
        assert!(matches!(e, SourceError::Status { .. }), "{e:?}");
    }

    #[test]
    fn empty_body_and_unknown_unit_fail() {
        let request = OpenMeteoMarine
            .plan(
                &target(40.70, -74.02, &all()),
                &Requirements::default(),
                &ctx(),
            )
            .unwrap()
            .remove(0);
        let mut out = Fragments::new();
        let e = OpenMeteoMarine
            .parse(&request, &Response::new(200, b""), &ctx(), &mut out)
            .unwrap_err();
        assert!(matches!(e, SourceError::NoData(_)), "{e:?}");
        let body = br#"{"latitude":40.6,"longitude":-74.0,"hourly_units":{"time":"iso8601","wave_height":"fathoms"},"hourly":{"time":["2026-11-08T00:00"],"wave_height":[1.0]}}"#;
        let e = OpenMeteoMarine
            .parse(&request, &Response::new(200, body), &ctx(), &mut out)
            .unwrap_err();
        assert!(matches!(e, SourceError::Parse(_)), "{e:?}");
    }

    #[test]
    fn extremes_handle_flat_tops_and_parabolas() {
        let s = |v: &[f32]| -> Vec<Sample<f32>> {
            v.iter()
                .enumerate()
                .map(|(i, v)| Sample::new(UnixTime(i64::try_from(i).unwrap() * 900), *v))
                .collect()
        };
        let events = extremes(&s(&[0.0, 1.0, 1.0, 1.0, 0.0, -1.0, 0.0]));
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].time, UnixTime(2 * 900));
        assert_eq!(events[0].kind, TideKind::High);
        assert_eq!(events[1].kind, TideKind::Low);
        // A peak that is closer to the later sample is placed after its own.
        let events = extremes(&s(&[0.0, 1.0, 0.8, 0.0]));
        assert_eq!(events.len(), 1);
        assert!(events[0].time.0 > 900 && events[0].time.0 < 1_350);
        assert_eq!(extremes(&s(&[0.0, 1.0, 2.0])).len(), 0);
    }

    #[test]
    fn attribution_names_open_meteo_and_the_models() {
        let a = OpenMeteoMarine.attribution();
        assert!(a.credit.starts_with("Weather data by Open-Meteo.com"));
        assert!(a.credit.contains("Copernicus Marine Service"));
        assert!(a.credit.contains("not suitable for coastal navigation"));
        assert_eq!(a.url.as_deref(), Some("https://open-meteo.com/"));
    }
}
