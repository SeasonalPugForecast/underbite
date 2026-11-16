//! Wind and gusts.

use std::fmt::Write;

use schemars::JsonSchema;
use serde::Serialize;
use whether_core::derive::current_hour_index;
use whether_core::model::WeatherSnapshot;

use crate::model::{Measurement, SpotSnapshot, Wind};
use crate::provenance::{Provenance, SourceId};
use crate::series::{Sample, Series};
use crate::time::UnixTime;
use crate::view::cards::tide::state;
use crate::view::chart::{Zoned, ticks, window};
use crate::view::format::Formatter;
use crate::view::strings::{Key, source_name};
use crate::view::{
    CardChrome, CardId, CardState, ChartPoint, ChartSeries, ChartVm, Compact, CreditVm, NoticeKind,
    Quantity, Row, Section, SeriesRole, SeriesStyle, ViewInput,
};

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct WindCard {
    pub chrome: CardChrome,
    pub compact: Compact,
    pub full: WindFull,
}

/// Where the wind figure comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WindBasis {
    /// A station or buoy.
    Observed,
    /// The forecast model.
    Forecast,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct WindHourVm {
    pub t: i64,
    pub time: String,
    pub speed: Quantity,
    pub gust: Option<Quantity>,
    /// "NW"
    pub direction: Option<String>,
    /// Degrees the wind blows from, for drawing arrows.
    pub from_deg: Option<f32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct WindFull {
    pub basis: WindBasis,
    pub speed: Option<Quantity>,
    pub gust: Option<Quantity>,
    pub direction: Option<String>,
    pub from_deg: Option<f32>,
    pub hours: Vec<WindHourVm>,
    pub chart: Option<ChartVm>,
    pub sections: Vec<Section>,
}

/// An observation older than this is not "now" unless the user set another limit.
pub(super) const DEFAULT_MAX_AGE_S: i64 = 3 * 3_600;

/// The newest sample at or before `now`.
pub(super) fn latest_at<T>(series: &Series<T>, now: UnixTime) -> Option<&Sample<T>> {
    series.samples().iter().rev().find(|s| s.time <= now)
}

/// The age beyond which a station value is no longer current.
pub(super) fn max_age(input: &ViewInput, measurement: Measurement) -> i64 {
    input
        .limit(measurement)
        .and_then(|l| l.max_age_s)
        .unwrap_or(DEFAULT_MAX_AGE_S)
}

/// Fresh or far as for any station, or stale when the newest value is older than the limit.
pub(super) fn aged_state(
    input: &ViewInput,
    f: &Formatter,
    p: &Provenance,
    measurement: Measurement,
    age_s: i64,
) -> CardState {
    let limit = max_age(input, measurement);
    if age_s > limit {
        CardState::Stale {
            reason: f.stale_reason(age_s, limit),
        }
    } else {
        state(input, f, p, measurement)
    }
}

/// The standard credit lines of the station sources.
const CREDIT_NDBC: &str = "Observations: NOAA National Data Buoy Center";
const CREDIT_COOPS: &str = "Observations: NOAA CO-OPS";

/// The credit a station source is owed: the snapshot's own text when it has one, else `fallback`,
/// else a standard line for the source.
pub(super) fn station_credit(
    snapshot: &SpotSnapshot,
    id: &SourceId,
    fallback: Option<&str>,
) -> CreditVm {
    let found = snapshot.attribution.iter().find(|a| a.source == *id);
    let standard = match id.as_str() {
        "ndbc" => CREDIT_NDBC,
        "coops" => CREDIT_COOPS,
        _ => "",
    };
    let text = found.map_or_else(
        || {
            let text = fallback.unwrap_or(standard);
            if text.is_empty() {
                source_name(id)
            } else {
                text.to_owned()
            }
        },
        |a| a.credit.clone(),
    );
    CreditVm {
        source: source_name(id),
        text,
        link: found.and_then(|a| a.url.clone()),
        licence: None,
    }
}

/// The credit whether's weather provider is owed.
pub(super) fn weather_credit(weather: &WeatherSnapshot) -> CreditVm {
    CreditVm {
        source: weather.provider.display_name().to_owned(),
        text: weather.provider.attribution().to_owned(),
        link: None,
        licence: None,
    }
}

/// "Wind 12 mph · Open-Meteo · modelled · 6 min ago"; the age is that of the forecast value
/// shown (`valid_at`), not of the download.
pub(super) fn weather_footnote(
    f: &Formatter,
    value: &str,
    weather: &WeatherSnapshot,
    now: UnixTime,
    valid_at: UnixTime,
) -> String {
    format!(
        "{value} · {} · {} · {}",
        weather.provider.display_name(),
        f.strings.get(Key::Modelled),
        f.age(now.0 - valid_at.0),
    )
}

/// whether's current conditions are used only this close to `now`.
pub(super) const CURRENT_WINDOW_S: i64 = 3_600;

/// Whether's current conditions when they are for about now.
pub(super) fn current_near(
    weather: &WeatherSnapshot,
    now: UnixTime,
) -> Option<&whether_core::model::Current> {
    weather
        .current
        .as_ref()
        .filter(|c| (now.0 - c.time.0).abs() <= CURRENT_WINDOW_S)
}

/// Whether the station lies beyond the user's distance limit.
pub(super) fn is_far(input: &ViewInput, p: &Provenance, measurement: Measurement) -> bool {
    matches!(
        (p.distance_m, input.limit(measurement).and_then(|l| l.max_distance_m)),
        (Some(d), Some(max)) if d > max
    )
}

/// Fresh, or stale when the forecast value's own time is older than the age limit.
pub(super) fn forecast_state(
    input: &ViewInput,
    f: &Formatter,
    measurement: Measurement,
    valid_at: UnixTime,
) -> CardState {
    let limit = max_age(input, measurement);
    let age = (input.now.0 - valid_at.0).max(0);
    if age > limit {
        CardState::Stale {
            reason: f.stale_reason(age, limit),
        }
    } else {
        CardState::Fresh
    }
}

/// Chart points from (time, quantity) pairs; pairs without a value are skipped.
pub(super) fn points(z: &Zoned, rows: Vec<(i64, Quantity)>) -> Vec<ChartPoint> {
    rows.into_iter()
        .filter_map(|(t, q)| {
            Some(ChartPoint {
                t,
                value: q.value?,
                label: format!("{} {}", q.text, q.unit).trim_end().to_owned(),
                major: z.on_the_hour(t),
            })
        })
        .collect()
}

/// A line series, or nothing when it has no points.
pub(super) fn line_series(
    label: &str,
    role: SeriesRole,
    points: Vec<ChartPoint>,
) -> Option<ChartSeries> {
    (!points.is_empty()).then(|| ChartSeries {
        label: label.to_owned(),
        role,
        style: SeriesStyle::Line,
        points,
    })
}

/// Speed, gust and "from" degrees, all canonical.
type Fix = (f32, Option<f32>, Option<f32>);

/// The forecast figure that applies now and the time it is for: whether's current conditions
/// when they are within an hour of `now`, else the hourly step containing `now`, else nothing.
fn forecast_now(weather: &WeatherSnapshot, now: UnixTime) -> Option<(Fix, UnixTime)> {
    if let Some(c) = current_near(weather, now)
        && let Some(speed) = c.wind_speed
    {
        return Some((
            (speed.0, c.wind_gust.map(|g| g.0), c.wind_dir.map(|d| d.0)),
            c.time,
        ));
    }
    let h = weather
        .hourly
        .get(current_hour_index(&weather.hourly, now)?)?;
    Some((
        (
            h.wind_speed?.0,
            h.wind_gust.map(|g| g.0),
            h.wind_dir.map(|d| d.0),
        ),
        h.time,
    ))
}

fn hour_vm(f: &Formatter, t: UnixTime, fix: Fix) -> WindHourVm {
    WindHourVm {
        t: t.0,
        time: f.clock(t),
        speed: f.wind_speed(Some(fix.0)),
        gust: fix.1.map(|g| f.wind_speed(Some(g))),
        direction: fix.2.map(|d| f.direction(Some(d))),
        from_deg: fix.2,
    }
}

fn forecast_hours(weather: &WeatherSnapshot, from: i64, to: i64) -> Vec<(UnixTime, Fix)> {
    weather
        .hourly
        .iter()
        .filter(|h| h.time.0 >= from && h.time.0 <= to)
        .filter_map(|h| {
            Some((
                h.time,
                (
                    h.wind_speed?.0,
                    h.wind_gust.map(|g| g.0),
                    h.wind_dir.map(|d| d.0),
                ),
            ))
        })
        .collect()
}

fn observed_fix(w: &Wind) -> Fix {
    (w.speed_ms, w.gust_ms, w.from_deg)
}

#[allow(clippy::too_many_lines)]
pub fn build(input: &ViewInput) -> Option<WindCard> {
    let snapshot = &input.snapshot;
    let now = input.now;
    let f = Formatter::for_input(input);
    let z = Zoned::new(&snapshot.spot.time_zone);
    let (start, end) = window(input);

    let observed = snapshot.marine.observed_wind.as_ref();
    let newest = observed.and_then(|s| latest_at(s, now));
    // A station leads only while it is recent and not too far away.
    let observed_fresh = observed.zip(newest).is_some_and(|(o, s)| {
        now.0 - s.time.0 <= max_age(input, Measurement::ObservedWind)
            && !is_far(input, o.provenance(), Measurement::ObservedWind)
    });
    let weather = snapshot.weather.as_ref();
    let forecast = weather.and_then(|w| forecast_now(w, now));
    // A fresh station first, else the forecast, else a stale station shown as such.
    let basis = if observed_fresh || (forecast.is_none() && newest.is_some()) {
        WindBasis::Observed
    } else if forecast.is_some() {
        WindBasis::Forecast
    } else {
        return None;
    };

    // The headline figure and the hours list come from one basis only.
    let (fix, hours, card_state, station) = match basis {
        WindBasis::Observed => {
            let (series, sample) = (observed?, newest?);
            let hours: Vec<WindHourVm> = series
                .samples()
                .iter()
                .filter(|s| s.time.0 >= start && s.time <= now)
                .map(|s| hour_vm(&f, s.time, observed_fix(&s.value)))
                .collect();
            (
                observed_fix(&sample.value),
                hours,
                aged_state(
                    input,
                    &f,
                    series.provenance(),
                    Measurement::ObservedWind,
                    now.0 - sample.time.0,
                ),
                Some((series.provenance(), sample.time)),
            )
        }
        WindBasis::Forecast => {
            let hours = forecast_hours(weather?, now.0 - 3_600, end)
                .into_iter()
                .map(|(t, v)| hour_vm(&f, t, v))
                .collect();
            let (fix, valid_at) = forecast?;
            (
                fix,
                hours,
                forecast_state(input, &f, Measurement::ObservedWind, valid_at),
                None,
            )
        }
    };
    let speed = f.wind_speed(Some(fix.0));
    let gust = fix.1.map(|g| f.wind_speed(Some(g)));
    let direction = fix.2.map(|d| f.direction(Some(d)));
    let headline = format!("{} {}", f.strings.card_title(CardId::Wind), speed.full());
    let forecast_time = forecast.map(|(_, t)| t);
    let footnote = match (station, weather, forecast_time) {
        (Some((p, t)), _, _) => f.footnote_aged(&headline, p, now, Some(t)),
        (None, Some(w), Some(t)) => weather_footnote(&f, &headline, w, now, t),
        _ => headline.clone(),
    };

    // The chart: past observations and the forecast ahead, as separate series.
    let mut series = Vec::new();
    let mut chart_credits = Vec::new();
    if let Some(o) = observed {
        let rows = |pick: fn(&Wind) -> Option<f32>| {
            points(
                &z,
                o.samples()
                    .iter()
                    .filter(|s| s.time.0 >= start && s.time <= now)
                    .map(|s| (s.time.0, f.wind_speed(pick(&s.value))))
                    .collect(),
            )
        };
        let before = series.len();
        series.extend(line_series(
            f.strings.get(Key::ObservedWind),
            SeriesRole::Observed,
            rows(|w| Some(w.speed_ms)),
        ));
        series.extend(line_series(
            f.strings.get(Key::ObservedGusts),
            SeriesRole::Observed,
            rows(|w| w.gust_ms),
        ));
        if series.len() > before || basis == WindBasis::Observed {
            chart_credits.push(station_credit(snapshot, &o.provenance().source, None));
        }
    }
    if let Some(w) = weather {
        let steps = forecast_hours(w, start, end);
        let rows = |pick: fn(&Fix) -> Option<f32>| {
            points(
                &z,
                steps
                    .iter()
                    .map(|(t, v)| (t.0, f.wind_speed(pick(v))))
                    .collect(),
            )
        };
        let before = series.len();
        series.extend(line_series(
            f.strings.get(Key::ForecastWind),
            SeriesRole::Forecast,
            rows(|v| Some(v.0)),
        ));
        series.extend(line_series(
            f.strings.get(Key::ForecastGusts),
            SeriesRole::Forecast,
            rows(|v| v.1),
        ));
        if series.len() > before || basis == WindBasis::Forecast {
            chart_credits.push(weather_credit(w));
        }
    }
    let chart = (!series.is_empty()).then(|| ChartVm {
        title: f.strings.card_title(CardId::Wind).to_owned(),
        unit: input.units.wind_speed.symbol().to_owned(),
        x_ticks: ticks(&z, &f, start, end),
        series,
        bands: Vec::new(),
        rules: Vec::new(),
        markers: Vec::new(),
        now_t: Some(now.0),
        accessibility: f.strings.fmt(
            Key::ChartNowAcc,
            &[
                ("title", f.strings.card_title(CardId::Wind)),
                ("value", &speed.full()),
                ("extra", ""),
            ],
        ),
    });

    let mut lines = Vec::new();
    if let Some(d) = &direction {
        lines.push(f.strings.fmt(Key::FromCap, &[("direction", d)]));
    }
    if let Some(g) = &gust {
        lines.push(f.strings.fmt(Key::GustsCap, &[("value", &g.full())]));
    }
    let spark = chart
        .as_ref()
        .and_then(|c| c.series.first())
        .map(|s| s.points.iter().map(|p| p.value).collect())
        .unwrap_or_default();

    let source_row = match (basis, observed.zip(newest), weather) {
        (WindBasis::Observed, Some((o, s)), _) => Some(Row {
            label: f.strings.get(Key::Station).to_owned(),
            value: o
                .provenance()
                .station
                .as_ref()
                .map_or_else(|| source_name(&o.provenance().source), |st| st.name.clone()),
            detail: Some(
                f.strings
                    .fmt(Key::ObservedAt, &[("time", &f.clock(s.time))]),
            ),
        }),
        (_, _, Some(w)) => Some(Row {
            label: f.strings.get(Key::Forecast).to_owned(),
            value: w.provider.display_name().to_owned(),
            detail: Some(f.strings.get(Key::Modelled).to_owned()),
        }),
        _ => None,
    };

    let basis_word = f.strings.get(match basis {
        WindBasis::Observed => Key::BasisObserved,
        WindBasis::Forecast => Key::BasisForecast,
    });
    let mut accessibility = format!(
        "{} {basis_word}: {}",
        f.strings.card_title(CardId::Wind),
        speed.full()
    );
    if let Some(d) = &direction {
        let _ = write!(
            accessibility,
            " {}",
            f.strings.fmt(Key::FromLower, &[("direction", d)])
        );
    }
    if let Some(g) = &gust {
        let _ = write!(
            accessibility,
            ", {}",
            f.strings.fmt(Key::GustsLower, &[("value", &g.full())])
        );
    }
    accessibility.push('.');

    Some(WindCard {
        chrome: CardChrome {
            id: CardId::Wind,
            title: f.strings.card_title(CardId::Wind).to_owned(),
            footnote: Some(footnote),
            credits: chart_credits,
            state: card_state,
            notices: if basis == WindBasis::Forecast {
                vec![f.notice(NoticeKind::Modelled)]
            } else {
                Vec::new()
            },
            accessibility,
        },
        compact: Compact {
            headline: speed.clone(),
            lines,
            spark,
        },
        full: WindFull {
            basis,
            speed: Some(speed),
            gust,
            direction,
            from_deg: fix.2,
            hours,
            chart,
            sections: vec![Section {
                title: f.strings.get(Key::SectionSource).to_owned(),
                rows: source_row.into_iter().collect(),
            }],
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provenance::{Quality, Relation, SourceId, ValueKind};
    use crate::view::tests::sample_input;
    use whether_core::testdata::SAMPLE_NOW;

    /// New York Harbor with whether's sample forecast and no station wind (buoy 44025 reports
    /// waves and pressure but not wind).
    fn forecast_only() -> ViewInput {
        let mut input = sample_input();
        input.now = SAMPLE_NOW;
        input.snapshot.marine = crate::model::MarineConditions::default();
        input
    }

    fn station_wind(input: &ViewInput, age_s: i64) -> Series<Wind> {
        let prov = Provenance {
            source: SourceId::NDBC,
            station: None,
            distance_m: Some(4_000.0),
            relation: Relation::Nearby,
            observed_at: UnixTime(input.now.0 - age_s),
            kind: ValueKind::Observed,
            quality: Quality::Provisional,
        };
        let sample = |minutes: i64, speed: f32| {
            Sample::new(
                UnixTime(input.now.0 - age_s - minutes * 60),
                Wind {
                    speed_ms: speed,
                    gust_ms: Some(speed + 3.0),
                    from_deg: Some(250.0),
                },
            )
        };
        Series::new(prov, vec![sample(60, 5.0), sample(30, 5.5), sample(0, 6.0)])
    }

    #[test]
    fn wind_comes_from_the_forecast_when_no_station_reports_it() {
        let input = forecast_only();
        let card = build(&input).unwrap();
        assert_eq!(card.full.basis, WindBasis::Forecast);
        // Whether's sample blows from the south-west (225 degrees), not toward it.
        assert_eq!(card.full.from_deg, Some(225.0));
        assert_eq!(card.full.direction.as_deref(), Some("SW 225°"));
        assert_eq!(card.full.speed.as_ref().unwrap().text, "8");
        assert_eq!(card.full.gust.as_ref().unwrap().text, "16");
        let chart = card.full.chart.as_ref().unwrap();
        assert!(chart.series.iter().all(|s| s.role == SeriesRole::Forecast));
        assert!(
            card.chrome
                .footnote
                .as_deref()
                .unwrap()
                .contains("Open-Meteo")
        );
        assert_eq!(card.chrome.credits.len(), 1);
        assert_eq!(card.chrome.state, CardState::Fresh);
        insta::assert_json_snapshot!("wind_forecast_us_luna", card);
    }

    #[test]
    fn a_fresh_station_leads_and_is_never_mixed_with_the_forecast() {
        let mut input = forecast_only();
        input.snapshot.marine.observed_wind = Some(station_wind(&input, 600));
        let card = build(&input).unwrap();
        assert_eq!(card.full.basis, WindBasis::Observed);
        assert_eq!(card.full.from_deg, Some(250.0));
        // 6 m/s is 13 mph.
        assert_eq!(card.full.speed.as_ref().unwrap().text, "13");
        assert_eq!(card.full.hours.len(), 3);
        let roles: Vec<_> = card
            .full
            .chart
            .as_ref()
            .unwrap()
            .series
            .iter()
            .map(|s| (s.label.as_str(), s.role))
            .collect();
        assert_eq!(
            roles,
            [
                ("Observed wind", SeriesRole::Observed),
                ("Observed gusts", SeriesRole::Observed),
                ("Forecast wind", SeriesRole::Forecast),
                ("Forecast gusts", SeriesRole::Forecast),
            ]
        );
        assert_eq!(card.chrome.credits.len(), 2);
        insta::assert_json_snapshot!("wind_observed_us_luna", card);
    }

    #[test]
    fn a_stale_station_gives_way_to_the_forecast() {
        let mut input = forecast_only();
        input.snapshot.marine.observed_wind = Some(station_wind(&input, 5 * 3_600));
        let card = build(&input).unwrap();
        assert_eq!(card.full.basis, WindBasis::Forecast);
        assert!(card.full.hours.iter().all(|h| h.t >= input.now.0 - 3_600));
    }

    #[test]
    fn a_stale_station_alone_is_shown_greyed() {
        let mut input = forecast_only();
        input.snapshot.weather = None;
        input.snapshot.marine.observed_wind = Some(station_wind(&input, 5 * 3_600));
        let card = build(&input).unwrap();
        assert_eq!(card.full.basis, WindBasis::Observed);
        assert!(matches!(card.chrome.state, CardState::Stale { .. }));
    }

    #[test]
    fn no_wind_anywhere_means_no_card() {
        let mut input = forecast_only();
        input.snapshot.weather = None;
        assert!(build(&input).is_none());
        // A forecast with neither wind nor a current step is no wind either.
        let mut input = forecast_only();
        if let Some(w) = input.snapshot.weather.as_mut() {
            w.current = None;
            w.hourly.clear();
        }
        assert!(build(&input).is_none());
    }

    #[test]
    fn an_old_current_step_gives_way_to_the_hour_now() {
        let mut input = forecast_only();
        let w = input.snapshot.weather.as_mut().unwrap();
        let c = w.current.as_mut().unwrap();
        c.time = UnixTime(SAMPLE_NOW.0 - 3 * 3_600);
        c.wind_speed = Some(whether_core::units::Mps(20.0));
        let card = build(&input).unwrap();
        assert_eq!(card.full.speed.as_ref().unwrap().text, "9");
        assert_eq!(card.chrome.state, CardState::Fresh);
        assert_eq!(card.chrome.notices.len(), 1);
        assert_eq!(card.chrome.notices[0].kind, NoticeKind::Modelled);
        // A current step within the hour is used, with its own age.
        let w = input.snapshot.weather.as_mut().unwrap();
        w.current.as_mut().unwrap().time = UnixTime(SAMPLE_NOW.0 - 1_800);
        let card = build(&input).unwrap();
        assert_eq!(card.full.speed.as_ref().unwrap().text, "45");
        assert!(
            card.chrome
                .footnote
                .as_deref()
                .unwrap()
                .ends_with("30 min ago")
        );
    }

    #[test]
    fn no_forecast_value_beyond_the_hourly_data() {
        let mut input = forecast_only();
        input.now = UnixTime(SAMPLE_NOW.0 + 10 * 86_400);
        assert!(build(&input).is_none());
        // An old station is then shown greyed, not an old forecast.
        input.snapshot.marine.observed_wind = Some(station_wind(&input, 5 * 3_600));
        let card = build(&input).unwrap();
        assert_eq!(card.full.basis, WindBasis::Observed);
        assert!(matches!(card.chrome.state, CardState::Stale { .. }));
    }

    #[test]
    fn a_far_station_does_not_beat_the_forecast() {
        let mut input = forecast_only();
        input.snapshot.marine.observed_wind = Some(station_wind(&input, 600));
        input.limits.push(crate::view::MeasurementLimit {
            measurement: Measurement::ObservedWind,
            max_distance_m: Some(1_000.0),
            max_age_s: None,
        });
        let card = build(&input).unwrap();
        assert_eq!(card.full.basis, WindBasis::Forecast);
        assert_eq!(card.full.speed.as_ref().unwrap().text, "8");
        // Alone, it is still shown, marked far.
        input.snapshot.weather = None;
        let card = build(&input).unwrap();
        assert_eq!(card.full.basis, WindBasis::Observed);
        assert!(matches!(card.chrome.state, CardState::Far { .. }));
    }

    #[test]
    fn a_station_with_only_samples_ahead_has_no_age() {
        let mut input = forecast_only();
        input.snapshot.weather = None;
        input.snapshot.marine.observed_wind = Some(station_wind(&input, -7_200));
        assert!(build(&input).is_none());
        let f = Formatter::for_input(&input);
        let p = input
            .snapshot
            .marine
            .observed_wind
            .as_ref()
            .unwrap()
            .provenance();
        let text = f.footnote_aged("Wind 13 mph", p, input.now, None);
        assert!(
            !text.contains("ago") && !text.contains("just now"),
            "{text}"
        );
    }
}
