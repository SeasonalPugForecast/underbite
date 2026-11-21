//! Barometric pressure and its trend.

use std::time::Duration;

use schemars::JsonSchema;
use serde::Serialize;
use whether_core::derive::current_hour_index;
use whether_core::model::WeatherSnapshot;

use crate::derive::pressure::{Trend, forecast_trend, observed_trend};
use crate::model::Measurement;
use crate::time::UnixTime;
use crate::units::{PressureUnit, pressure_from_hpa};
use crate::view::cards::wind::{
    aged_state, current_near, forecast_state, latest_at, line_series, max_age, points,
    station_credit, weather_credit, weather_footnote,
};
use crate::view::chart::{Zoned, ticks, window};
use crate::view::format::Formatter;
use crate::view::strings::Key;
use crate::view::{
    CardChrome, CardId, ChartVm, Compact, NoticeKind, Quantity, Row, Section, SeriesRole,
    TrendDirection, TrendVm, ViewInput,
};

/// The trend window, hours.
const TREND_HOURS: i64 = 3;
/// Neighbouring observations further apart than this are not interpolated across.
const MAX_GAP: Duration = Duration::from_mins(90);

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct PressureCard {
    pub chrome: CardChrome,
    pub compact: Compact,
    pub full: PressureFull,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct PressureFull {
    pub value: Option<Quantity>,
    pub trend: Option<TrendVm>,
    /// The change over the trend window, signed.
    pub change: Option<Quantity>,
    /// Past observed and forecast pressure on one axis.
    pub chart: Option<ChartVm>,
    pub sections: Vec<Section>,
}

/// Which figure the card leads with.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Basis {
    Observed,
    Forecast,
}

/// The forecast pressure now, the change to three hours later and the time the value is for:
/// whether's current conditions when within an hour of `now`, else the hourly step containing
/// `now`, else nothing.
fn forecast_now(weather: &WeatherSnapshot, now: UnixTime) -> Option<(f32, Option<f32>, UnixTime)> {
    let step = current_hour_index(&weather.hourly, now).and_then(|i| {
        let start = weather.hourly[i].pressure_msl?.0;
        let target = weather.hourly[i].time.0 + TREND_HOURS * 3_600;
        let later = weather.hourly[i..]
            .iter()
            .find(|h| h.time.0 >= target)
            .and_then(|h| h.pressure_msl)
            .map(|p| p.0 - start);
        Some((start, later, weather.hourly[i].time))
    });
    if let Some(c) = current_near(weather, now)
        && let Some(p) = c.pressure_msl
    {
        return Some((p.0, step.and_then(|s| s.1), c.time));
    }
    step
}

fn trend_vm(f: &Formatter, trend: Trend, change: Option<&Quantity>) -> TrendVm {
    let (direction, key) = match trend {
        Trend::Rising => (TrendDirection::Rising, Key::Rising),
        Trend::Falling => (TrendDirection::Falling, Key::Falling),
        Trend::Steady => (TrendDirection::Steady, Key::Steady),
    };
    let word = f.strings.get(key);
    let text = match change {
        Some(c) => {
            let amount = format!("{} {}", c.text.trim_start_matches(['+', '-']), c.unit);
            f.strings.fmt(
                Key::TrendOver,
                &[
                    ("direction", word),
                    ("amount", &amount),
                    ("hours", &TREND_HOURS.to_string()),
                ],
            )
        }
        None => word.to_owned(),
    };
    TrendVm { direction, text }
}

/// A signed change in the display unit.
fn change_quantity(f: &Formatter, unit: PressureUnit, delta_hpa: f32, base_hpa: f32) -> Quantity {
    let value = pressure_from_hpa(unit, base_hpa + delta_hpa) - pressure_from_hpa(unit, base_hpa);
    let decimals = match unit {
        PressureUnit::Hpa | PressureUnit::MmHg => 1,
        PressureUnit::InHg | PressureUnit::Kpa => 2,
    };
    Quantity {
        value: Some(value),
        text: f
            .strings
            .locale
            .with_decimal_mark(&format!("{value:+.decimals$}")),
        unit: f.strings.pressure_unit(unit).to_owned(),
    }
}

#[allow(clippy::too_many_lines, clippy::many_single_char_names)]
pub fn build(input: &ViewInput) -> Option<PressureCard> {
    let snapshot = &input.snapshot;
    let now = input.now;
    let f = Formatter::for_input(input);
    let z = Zoned::new(&snapshot.spot.time_zone);
    let (start, end) = window(input);

    let observed = snapshot.marine.observed_pressure_hpa.as_ref();
    let newest = observed.and_then(|s| latest_at(s, now));
    let fresh =
        newest.is_some_and(|s| now.0 - s.time.0 <= max_age(input, Measurement::ObservedPressure));
    let weather = snapshot.weather.as_ref();
    let forecast = weather.and_then(|w| forecast_now(w, now));
    let basis = if fresh || (forecast.is_none() && newest.is_some()) {
        Basis::Observed
    } else if forecast.is_some() {
        Basis::Forecast
    } else {
        return None;
    };

    // The value, trend and change come from one basis only.
    let (hpa, trend, delta, card_state) = match basis {
        Basis::Observed => {
            let (series, end_sample) = (observed?, newest?);
            let delta = series
                .at(UnixTime(end_sample.time.0 - TREND_HOURS * 3_600), MAX_GAP)
                .map(|before| end_sample.value - before);
            (
                end_sample.value,
                observed_trend(series, now),
                delta,
                aged_state(
                    input,
                    &f,
                    series.provenance(),
                    Measurement::ObservedPressure,
                    now.0 - end_sample.time.0,
                ),
            )
        }
        Basis::Forecast => {
            let w = weather?;
            let (v, d, valid_at) = forecast?;
            (
                v,
                forecast_trend(w, now),
                d,
                forecast_state(input, &f, Measurement::ObservedPressure, valid_at),
            )
        }
    };
    let value = f.pressure(Some(hpa));
    let change = delta.map(|d| change_quantity(&f, input.units.pressure, d, hpa));
    let trend = trend.map(|t| trend_vm(&f, t, change.as_ref()));

    let footnote = match (basis, observed, weather) {
        (Basis::Observed, Some(o), _) => {
            Some(f.footnote_aged(o.provenance(), now, newest.map(|s| s.time)))
        }
        (_, _, Some(w)) => forecast.map(|(_, _, t)| weather_footnote(&f, w, now, t)),
        _ => None,
    };

    // The chart: observed past and forecast, as separate series.
    let mut series = Vec::new();
    let mut chart_credits = Vec::new();
    if let Some(o) = observed {
        let pts = points(
            &z,
            o.samples()
                .iter()
                .filter(|s| s.time.0 >= start && s.time <= now)
                .map(|s| (s.time.0, f.pressure(Some(s.value))))
                .collect(),
        );
        let any = !pts.is_empty();
        series.extend(line_series(
            f.strings.get(Key::ObservedPressure),
            SeriesRole::Observed,
            pts,
        ));
        if any || basis == Basis::Observed {
            chart_credits.push(station_credit(&f, snapshot, &o.provenance().source, None));
        }
    }
    if let Some(w) = weather {
        let pts = points(
            &z,
            w.hourly
                .iter()
                .filter(|h| h.time.0 >= start && h.time.0 <= end)
                .map(|h| (h.time.0, f.pressure(h.pressure_msl.map(|p| p.0))))
                .collect(),
        );
        let any = !pts.is_empty();
        series.extend(line_series(
            f.strings.get(Key::ForecastPressure),
            SeriesRole::Forecast,
            pts,
        ));
        if any || basis == Basis::Forecast {
            chart_credits.push(weather_credit(w));
        }
    }
    let chart = (!series.is_empty()).then(|| ChartVm {
        title: f.strings.card_title(CardId::Pressure).to_owned(),
        unit: f.strings.pressure_unit(input.units.pressure).to_owned(),
        x_ticks: ticks(&z, &f, start, end),
        series,
        bands: Vec::new(),
        rules: Vec::new(),
        markers: Vec::new(),
        now_t: Some(now.0),
        accessibility: f.strings.fmt(
            Key::ChartNowAcc,
            &[
                ("title", f.strings.card_title(CardId::Pressure)),
                ("value", &value.full()),
                (
                    "extra",
                    &trend
                        .as_ref()
                        .map_or(String::new(), |t| format!(", {}", t.text)),
                ),
            ],
        ),
    });

    let spark = chart
        .as_ref()
        .and_then(|c| {
            c.series
                .iter()
                .find(|s| s.role == SeriesRole::Observed)
                .or(c.series.first())
        })
        .map(|s| s.points.iter().map(|p| p.value).collect())
        .unwrap_or_default();

    let source_row = match (basis, observed.zip(newest), weather) {
        (Basis::Observed, Some((o, s)), _) => Row {
            label: f.strings.get(Key::Station).to_owned(),
            value: o.provenance().station.as_ref().map_or_else(
                || f.strings.source_name(&o.provenance().source),
                |st| st.name.clone(),
            ),
            detail: Some(
                f.strings
                    .fmt(Key::ObservedAt, &[("time", &f.clock(s.time))]),
            ),
        },
        (_, _, Some(w)) => Row {
            label: f.strings.get(Key::Forecast).to_owned(),
            value: w.provider.display_name().to_owned(),
            detail: Some(f.strings.get(Key::Modelled).to_owned()),
        },
        _ => return None,
    };

    let basis_word = f.strings.get(if basis == Basis::Observed {
        Key::BasisObserved
    } else {
        Key::BasisForecast
    });
    let title = f.strings.card_title(CardId::Pressure);
    let accessibility = match &trend {
        Some(t) => format!("{title} {basis_word}: {}, {}.", value.full(), t.text),
        None => format!("{title} {basis_word}: {}.", value.full()),
    };

    Some(PressureCard {
        chrome: CardChrome {
            id: CardId::Pressure,
            title: f.strings.card_title(CardId::Pressure).to_owned(),
            footnote,
            credits: chart_credits,
            state: card_state,
            notices: if basis == Basis::Forecast {
                vec![f.notice(NoticeKind::Modelled)]
            } else {
                Vec::new()
            },
            accessibility,
        },
        compact: Compact {
            headline: value.clone(),
            lines: trend.iter().map(|t| t.text.clone()).collect(),
            spark,
        },
        full: PressureFull {
            value: Some(value),
            trend,
            change,
            chart,
            sections: vec![Section {
                title: f.strings.get(Key::SectionSource).to_owned(),
                rows: vec![source_row],
            }],
        },
    })
}

#[cfg(test)]
pub(in crate::view) mod tests {
    use super::*;
    use crate::provenance::{Provenance, Quality, Relation, SourceId, ValueKind};
    use crate::series::{Sample, Series};
    use crate::view::CardState;
    use crate::view::tests::sample_input;
    use whether_core::testdata::SAMPLE_NOW;

    /// Whether's sample forecast and nothing from stations.
    pub(in crate::view) fn forecast_only() -> ViewInput {
        let mut input = sample_input();
        input.now = SAMPLE_NOW;
        input.snapshot.marine = crate::model::MarineConditions::default();
        input
    }

    /// A buoy's pressure every 30 minutes for four hours ending `age_s` before now, from `from`
    /// to `to` hPa.
    fn buoy(input: &ViewInput, age_s: i64, from: f32, to: f32) -> Series<f32> {
        let prov = Provenance {
            source: SourceId::NDBC,
            station: None,
            distance_m: Some(4_000.0),
            relation: Relation::Nearby,
            observed_at: UnixTime(input.now.0 - age_s),
            kind: ValueKind::Observed,
            quality: Quality::Provisional,
        };
        let samples = (0..=8)
            .map(|i| {
                let t = input.now.0 - age_s - (8 - i) * 1_800;
                Sample::new(UnixTime(t), from + (to - from) * i as f32 / 8.0)
            })
            .collect();
        Series::new(prov, samples)
    }

    #[test]
    fn pressure_comes_from_the_forecast_without_a_station() {
        let input = forecast_only();
        let card = build(&input).unwrap();
        // 1019.8 hPa in inHg.
        assert_eq!(card.full.value.as_ref().unwrap().text, "30.11");
        let trend = card.full.trend.as_ref().unwrap();
        assert_eq!(trend.direction, TrendDirection::Steady);
        let chart = card.full.chart.as_ref().unwrap();
        assert!(chart.series.iter().all(|s| s.role == SeriesRole::Forecast));
        assert!(
            card.chrome
                .footnote
                .as_deref()
                .unwrap()
                .contains("Open-Meteo")
        );
        insta::assert_json_snapshot!("pressure_forecast_us_luna", card);
    }

    #[test]
    fn a_fresh_station_leads_and_its_trend_is_observed() {
        let mut input = forecast_only();
        input.units.pressure = PressureUnit::Hpa;
        input.snapshot.marine.observed_pressure_hpa = Some(buoy(&input, 600, 1_016.0, 1_012.0));
        let card = build(&input).unwrap();
        assert_eq!(card.full.value.as_ref().unwrap().text, "1012");
        let trend = card.full.trend.as_ref().unwrap();
        assert_eq!(trend.direction, TrendDirection::Falling);
        // 4 hPa over four hours, so 3 hPa over the last three.
        assert_eq!(trend.text, "Falling 3.0 hPa in 3 h");
        assert_eq!(card.full.change.as_ref().unwrap().text, "-3.0");
        let roles: Vec<_> = card
            .full
            .chart
            .as_ref()
            .unwrap()
            .series
            .iter()
            .map(|s| s.role)
            .collect();
        assert_eq!(roles, [SeriesRole::Observed, SeriesRole::Forecast]);
        assert_eq!(card.chrome.credits.len(), 2);
        insta::assert_json_snapshot!("pressure_observed_us_luna", card);
    }

    #[test]
    fn a_stale_station_gives_way_to_the_forecast() {
        let mut input = forecast_only();
        input.snapshot.marine.observed_pressure_hpa =
            Some(buoy(&input, 5 * 3_600, 1_016.0, 1_012.0));
        let card = build(&input).unwrap();
        assert_eq!(card.full.value.as_ref().unwrap().text, "30.11");
        assert_eq!(card.chrome.state, CardState::Fresh);
    }

    #[test]
    fn a_stale_station_alone_is_shown_greyed() {
        let mut input = forecast_only();
        input.snapshot.weather = None;
        input.snapshot.marine.observed_pressure_hpa =
            Some(buoy(&input, 5 * 3_600, 1_016.0, 1_012.0));
        let card = build(&input).unwrap();
        assert!(matches!(card.chrome.state, CardState::Stale { .. }));
        // No trend "now" from a stale series.
        assert!(card.full.trend.is_none());
    }

    #[test]
    fn no_pressure_anywhere_means_no_card() {
        let mut input = forecast_only();
        input.snapshot.weather = None;
        assert!(build(&input).is_none());
    }

    #[test]
    fn an_old_current_step_gives_way_to_the_hour_now() {
        let mut input = forecast_only();
        let w = input.snapshot.weather.as_mut().unwrap();
        let c = w.current.as_mut().unwrap();
        c.time = UnixTime(SAMPLE_NOW.0 - 3 * 3_600);
        c.pressure_msl = Some(whether_core::units::Hpa(990.0));
        let card = build(&input).unwrap();
        assert_eq!(card.full.value.as_ref().unwrap().text, "30.09");
        assert_eq!(card.chrome.notices.len(), 1);
        // A current step within the hour is used, and its own age is shown.
        let w = input.snapshot.weather.as_mut().unwrap();
        w.current.as_mut().unwrap().time = UnixTime(SAMPLE_NOW.0 - 1_800);
        let card = build(&input).unwrap();
        assert_eq!(card.full.value.as_ref().unwrap().text, "29.23");
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
        // A station that is old is then shown, greyed, rather than the old forecast.
        input.snapshot.marine.observed_pressure_hpa =
            Some(buoy(&input, 5 * 3_600, 1_016.0, 1_012.0));
        let card = build(&input).unwrap();
        assert!(matches!(card.chrome.state, CardState::Stale { .. }));
    }
}
