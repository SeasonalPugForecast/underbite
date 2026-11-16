//! Waves and swell.

use std::fmt::Write;

use schemars::JsonSchema;
use serde::Serialize;

use crate::model::{Measurement, WaveDirection, WaveSample, WaveSeries, WaveTrain};
use crate::series::Sample;
use crate::time::UnixTime;
use crate::view::cards::wind::{
    aged_state, latest_at, line_series, max_age, points, station_credit,
};
use crate::view::chart::{Zoned, ticks, window};
use crate::view::format::Formatter;
use crate::view::strings::{Key, source_name};
use crate::view::{
    CardChrome, CardId, CardState, ChartPoint, ChartVm, Compact, Quantity, Row, Section,
    SeriesRole, ViewInput,
};

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct WavesCard {
    pub chrome: CardChrome,
    pub compact: Compact,
    pub full: WavesFull,
}

/// One wave train (wind waves or a swell).
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct WaveTrainVm {
    /// "Wind waves", "Swell 1"
    pub label: String,
    pub height: Option<Quantity>,
    pub period: Option<Quantity>,
    /// "SE 135°", the direction the waves come from.
    pub direction: Option<String>,
}

/// Waves at one moment: the latest observation or the forecast for a time.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct WaveNowVm {
    pub t: i64,
    pub time: String,
    pub significant_height: Option<Quantity>,
    pub dominant_period: Option<Quantity>,
    pub mean_period: Option<Quantity>,
    pub direction: Option<String>,
    pub wind_waves: Option<WaveTrainVm>,
    pub swells: Vec<WaveTrainVm>,
    /// "Steep", when the source says.
    pub steepness: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct WavesFull {
    pub observed: Option<WaveNowVm>,
    /// The forecast for now (or the next hour).
    pub forecast: Option<WaveNowVm>,
    /// Observed and forecast height on one axis.
    pub height_chart: Option<ChartVm>,
    pub period_chart: Option<ChartVm>,
    pub sections: Vec<Section>,
}

/// The credit of the wave model when the snapshot carries none.
const CREDIT_WAVE_MODEL: &str = "Wave model: Open-Meteo.com";

fn has_content(s: &WaveSample) -> bool {
    s.significant_height_m.is_some()
        || s.dominant_period_s.is_some()
        || s.mean_period_s.is_some()
        || s.wind_waves.is_some()
        || !s.swells.is_empty()
}

/// `VERY_STEEP` as "Very steep"; empty or not-available text is no steepness.
fn steepness_text(raw: &str) -> Option<String> {
    let text = raw.trim().replace('_', " ").to_lowercase();
    if text.is_empty() || text == "n/a" {
        return None;
    }
    let mut chars = text.chars();
    let first = chars.next()?;
    Some(first.to_uppercase().chain(chars).collect())
}

fn seconds(s: Option<f32>) -> Option<Quantity> {
    s.map(|v| Quantity {
        value: Some(v),
        text: format!("{v:.1}"),
        unit: "s".to_owned(),
    })
}

/// A direction the waves come from: degrees get a compass name, compass text stays as given.
fn direction_text(f: &Formatter, d: &WaveDirection) -> String {
    match d {
        WaveDirection::Degrees(v) => f.direction(Some(*v)),
        WaveDirection::Compass(text) => text.clone(),
    }
}

fn train_vm(f: &Formatter, label: String, t: &WaveTrain) -> WaveTrainVm {
    WaveTrainVm {
        label,
        height: t.height_m.map(|h| f.length(Some(h))),
        period: seconds(t.period_s),
        direction: t.direction_from.as_ref().map(|d| direction_text(f, d)),
    }
}

fn now_vm(f: &Formatter, t: UnixTime, s: &WaveSample) -> WaveNowVm {
    WaveNowVm {
        t: t.0,
        time: f.clock(t),
        significant_height: s.significant_height_m.map(|h| f.length(Some(h))),
        dominant_period: seconds(s.dominant_period_s),
        mean_period: seconds(s.mean_period_s),
        direction: s.direction_from_deg.map(|d| f.direction(Some(d))),
        wind_waves: s
            .wind_waves
            .as_ref()
            .map(|t| train_vm(f, f.strings.get(Key::WindWaves).to_owned(), t)),
        swells: s
            .swells
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let label = f.strings.fmt(Key::SwellN, &[("n", &(i + 1).to_string())]);
                train_vm(f, label, t)
            })
            .collect(),
        steepness: s.steepness.as_deref().and_then(steepness_text),
    }
}

/// The forecast sample that applies now: the newest at or before it, else the first after.
fn forecast_pick(series: &WaveSeries, now: UnixTime) -> Option<&Sample<WaveSample>> {
    let samples = series.samples();
    samples
        .iter()
        .rev()
        .find(|s| s.time <= now && has_content(&s.value))
        .or_else(|| {
            samples
                .iter()
                .find(|s| s.time > now && has_content(&s.value))
        })
}

fn chart_points(
    z: &Zoned,
    series: &WaveSeries,
    from: i64,
    to: i64,
    pick: impl Fn(&WaveSample) -> Option<Quantity>,
) -> Vec<ChartPoint> {
    points(
        z,
        series
            .samples()
            .iter()
            .filter(|s| s.time.0 >= from && s.time.0 <= to)
            .filter_map(|s| Some((s.time.0, pick(&s.value)?)))
            .collect(),
    )
}

fn row_for(f: &Formatter, label: &str, series: &WaveSeries) -> Row {
    let p = series.provenance();
    Row {
        label: label.to_owned(),
        value: p
            .station
            .as_ref()
            .map_or_else(|| source_name(&p.source), |s| s.name.clone()),
        detail: p.distance_m.map(|m| f.distance(Some(m)).full()),
    }
}

#[allow(clippy::too_many_lines)]
pub fn build(input: &ViewInput) -> Option<WavesCard> {
    let snapshot = &input.snapshot;
    let now = input.now;
    let f = Formatter::for_input(input);
    let z = Zoned::new(&snapshot.spot.time_zone);
    let (start, end) = window(input);

    let observed = snapshot.marine.waves.as_ref();
    let newest = observed
        .and_then(|s| latest_at(s, now))
        .filter(|s| has_content(&s.value));
    let forecast_series = snapshot.marine.waves_forecast.as_ref();
    let forecast = forecast_series.and_then(|s| forecast_pick(s, now));
    let fresh = newest.is_some_and(|s| now.0 - s.time.0 <= max_age(input, Measurement::Waves));
    // A fresh buoy first; a stale one is shown (greyed) only when nothing else is.
    let show_observed = newest.is_some() && (fresh || forecast.is_none());
    if !show_observed && forecast.is_none() {
        return None;
    }

    let observed_vm = newest
        .filter(|_| show_observed)
        .map(|s| now_vm(&f, s.time, &s.value));
    let forecast_vm = forecast.map(|s| now_vm(&f, s.time, &s.value));
    let observed_shown = observed.filter(|_| observed_vm.is_some());
    let forecast_shown = forecast_series.filter(|_| forecast_vm.is_some());

    // Observed and modelled stay in separate series on the same axis.
    let height = |s: &WaveSample| s.significant_height_m.map(|h| f.length(Some(h)));
    let period = |s: &WaveSample| seconds(s.dominant_period_s);
    let mut height_series = Vec::new();
    let mut period_series = Vec::new();
    if let Some(o) = observed_shown {
        height_series.extend(line_series(
            f.strings.get(Key::Observed),
            SeriesRole::Observed,
            chart_points(&z, o, start, now.0, height),
        ));
        period_series.extend(line_series(
            f.strings.get(Key::Observed),
            SeriesRole::Observed,
            chart_points(&z, o, start, now.0, period),
        ));
    }
    if let Some(m) = forecast_shown {
        height_series.extend(line_series(
            f.strings.get(Key::ModelledCap),
            SeriesRole::Modelled,
            chart_points(&z, m, start, end, height),
        ));
        period_series.extend(line_series(
            f.strings.get(Key::ModelledCap),
            SeriesRole::Modelled,
            chart_points(&z, m, start, end, period),
        ));
    }
    let chart = |title: &str, unit: &str, series: Vec<_>| {
        (!series.is_empty()).then(|| ChartVm {
            title: title.to_owned(),
            unit: unit.to_owned(),
            x_ticks: ticks(&z, &f, start, end),
            series,
            bands: Vec::new(),
            rules: Vec::new(),
            markers: Vec::new(),
            now_t: Some(now.0),
            accessibility: f.strings.fmt(Key::ChartOnly, &[("title", title)]),
        })
    };
    let height_chart = chart(
        f.strings.get(Key::WaveHeight),
        input.units.height.symbol(),
        height_series,
    );
    let period_chart = chart(f.strings.get(Key::WavePeriod), "s", period_series);

    // The headline: the buoy when shown, else the model.
    let (lead, lead_series, is_observed) =
        if let (Some(o), Some(s)) = (&observed_vm, observed_shown) {
            (o, s, true)
        } else if let (Some(m), Some(s)) = (&forecast_vm, forecast_shown) {
            (m, s, false)
        } else {
            return None;
        };
    let headline = lead.significant_height.clone().unwrap_or_default();
    let mut lines = Vec::new();
    let mut details = Vec::new();
    if let Some(p) = &lead.dominant_period {
        details.push(format!("{} {}", p.text, p.unit));
    }
    if let Some(d) = &lead.direction {
        details.push(f.strings.fmt(Key::FromLower, &[("direction", d)]));
    }
    if !details.is_empty() {
        lines.push(details.join(" · "));
    }
    match (is_observed, &forecast_vm) {
        (true, Some(m)) => {
            if let Some(h) = &m.significant_height {
                lines.push(f.strings.fmt(Key::ModelledHeight, &[("height", &h.full())]));
            }
        }
        (false, _) => lines.push(f.strings.get(Key::Modelled).to_owned()),
        _ => {}
    }
    let spark = height_chart
        .as_ref()
        .and_then(|c| c.series.first())
        .map(|s| s.points.iter().map(|p| p.value).collect())
        .unwrap_or_default();

    let label = if is_observed {
        f.strings.card_title(CardId::Waves)
    } else {
        f.strings.get(Key::ModelledWaves)
    };
    let value = format!("{label} {}", headline.full());
    let p = lead_series.provenance();
    let card_state = match newest {
        Some(s) if is_observed => aged_state(input, &f, p, Measurement::Waves, now.0 - s.time.0),
        _ => CardState::Fresh,
    };

    let mut card_credits = Vec::new();
    if let Some(o) = observed_shown {
        card_credits.push(station_credit(snapshot, &o.provenance().source, None));
    }
    if let Some(m) = forecast_shown {
        card_credits.push(station_credit(
            snapshot,
            &m.provenance().source,
            Some(CREDIT_WAVE_MODEL),
        ));
    }

    let mut rows = Vec::new();
    if let Some(o) = observed_shown {
        rows.push(row_for(&f, f.strings.get(Key::Buoy), o));
    }
    if let Some(m) = forecast_shown {
        let mut row = row_for(&f, f.strings.get(Key::ModelWord), m);
        row.detail = Some(f.strings.get(Key::Modelled).to_owned());
        rows.push(row);
    }

    let mut accessibility = value.clone();
    if let Some(p) = &lead.dominant_period {
        let _ = write!(
            accessibility,
            ", {}",
            f.strings.fmt(Key::PeriodSeconds, &[("value", &p.text)])
        );
    }
    if let Some(d) = &lead.direction {
        let _ = write!(
            accessibility,
            ", {}",
            f.strings.fmt(Key::FromLower, &[("direction", d)])
        );
    }
    accessibility.push('.');

    Some(WavesCard {
        chrome: CardChrome {
            id: CardId::Waves,
            title: f.strings.card_title(CardId::Waves).to_owned(),
            footnote: Some(f.footnote(&value, p, now)),
            credits: card_credits,
            state: card_state,
            notices: Vec::new(),
            accessibility,
        },
        compact: Compact {
            headline,
            lines,
            spark,
        },
        full: WavesFull {
            observed: observed_vm,
            forecast: forecast_vm,
            height_chart,
            period_chart,
            sections: vec![Section {
                title: f.strings.card_title(CardId::Sources).to_owned(),
                rows,
            }],
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provenance::{Provenance, Quality, Relation, SourceId, ValueKind};
    use crate::series::Series;
    use crate::view::tests::sample_input;

    fn prov(
        source: SourceId,
        kind: ValueKind,
        relation: Relation,
        age_s: i64,
        now: i64,
    ) -> Provenance {
        Provenance {
            source,
            station: None,
            distance_m: Some(30_000.0),
            relation,
            observed_at: UnixTime(now - age_s),
            kind,
            quality: Quality::Provisional,
        }
    }

    fn train(height: f32, period: Option<f32>, dir: Option<WaveDirection>) -> WaveTrain {
        WaveTrain {
            height_m: Some(height),
            period_s: period,
            direction_from: dir,
        }
    }

    /// Buoy 44025-like waves: wind waves and one swell with compass text directions, one swell
    /// without a period, no mean period; no wind, no model.
    fn buoy_input() -> ViewInput {
        let mut input = sample_input();
        let now = input.now.0;
        input.snapshot.marine = crate::model::MarineConditions::default();
        let sample = |minutes: i64, h: f32| {
            Sample::new(
                UnixTime(now - 900 - minutes * 60),
                WaveSample {
                    significant_height_m: Some(h),
                    dominant_period_s: Some(9.0),
                    mean_period_s: None,
                    direction_from_deg: None,
                    wind_waves: Some(train(
                        0.6,
                        Some(4.0),
                        Some(WaveDirection::Compass("ESE".into())),
                    )),
                    swells: vec![
                        train(0.9, Some(9.1), Some(WaveDirection::Compass("SE".into()))),
                        train(0.2, None, None),
                    ],
                    steepness: Some("VERY_STEEP".into()),
                },
            )
        };
        input.snapshot.marine.waves = Some(Series::new(
            prov(
                SourceId::NDBC,
                ValueKind::Observed,
                Relation::Nearby,
                900,
                now,
            ),
            vec![sample(60, 1.0), sample(30, 1.1), sample(0, 1.2)],
        ));
        input
    }

    fn model_series(input: &ViewInput) -> WaveSeries {
        let now = input.now.0;
        let sample = |hours: i64, h: f32| {
            Sample::new(
                UnixTime(now + hours * 3_600),
                WaveSample {
                    significant_height_m: Some(h),
                    dominant_period_s: Some(7.0),
                    direction_from_deg: Some(135.0),
                    swells: vec![train(0.8, Some(7.5), Some(WaveDirection::Degrees(135.0)))],
                    ..WaveSample::default()
                },
            )
        };
        Series::new(
            prov(
                SourceId::OPEN_METEO_MARINE,
                ValueKind::Modelled,
                Relation::Gridded,
                600,
                now,
            ),
            vec![
                sample(-1, 0.9),
                sample(0, 1.0),
                sample(1, 1.3),
                sample(2, 1.5),
            ],
        )
    }

    #[test]
    fn a_buoy_gives_waves_with_text_directions_kept_and_optional_parts_absent() {
        let input = buoy_input();
        let card = build(&input).unwrap();
        let observed = card.full.observed.as_ref().unwrap();
        assert!(card.full.forecast.is_none());
        // 1.2 m is 3.9 ft; the period is one decimal.
        assert_eq!(
            observed.significant_height.as_ref().unwrap().full(),
            "3.9 ft"
        );
        assert_eq!(observed.dominant_period.as_ref().unwrap().full(), "9.0 s");
        assert!(observed.mean_period.is_none());
        assert!(observed.direction.is_none());
        // Compass text from the buoy stays as given; a degree bearing gets the name.
        assert_eq!(
            observed.wind_waves.as_ref().unwrap().direction.as_deref(),
            Some("ESE")
        );
        assert_eq!(observed.swells[0].direction.as_deref(), Some("SE"));
        assert_eq!(observed.swells[1].label, "Swell 2");
        assert!(observed.swells[1].period.is_none());
        assert!(observed.swells[1].direction.is_none());
        assert_eq!(observed.steepness.as_deref(), Some("Very steep"));
        let chart = card.full.height_chart.as_ref().unwrap();
        assert_eq!(chart.series.len(), 1);
        assert_eq!(chart.series[0].role, SeriesRole::Observed);
        assert_eq!(chart.series[0].points.len(), 3);
        assert_eq!(chart.unit, "ft");
        assert_eq!(card.chrome.credits.len(), 1);
        assert_eq!(card.chrome.state, CardState::Fresh);
        insta::assert_json_snapshot!("waves_buoy_us_luna", card);
    }

    #[test]
    fn observed_and_modelled_waves_stay_in_separate_series() {
        let mut input = buoy_input();
        input.snapshot.marine.waves_forecast = Some(model_series(&input));
        let card = build(&input).unwrap();
        let forecast = card.full.forecast.as_ref().unwrap();
        // The sample at "now", with a degree bearing named: waves come from 135 degrees.
        assert_eq!(
            forecast.significant_height.as_ref().unwrap().full(),
            "3.3 ft"
        );
        assert_eq!(forecast.direction.as_deref(), Some("SE 135°"));
        assert_eq!(forecast.swells[0].direction.as_deref(), Some("SE 135°"));
        let chart = card.full.height_chart.as_ref().unwrap();
        let roles: Vec<_> = chart
            .series
            .iter()
            .map(|s| (s.label.as_str(), s.role))
            .collect();
        assert_eq!(
            roles,
            [
                ("Observed", SeriesRole::Observed),
                ("Modelled", SeriesRole::Modelled)
            ]
        );
        // Neither series carries the other's points.
        assert!(chart.series[0].points.iter().all(|p| p.t <= input.now.0));
        assert_eq!(chart.series[1].points.len(), 4);
        assert_eq!(card.chrome.credits.len(), 2);
        assert!(card.compact.lines.iter().any(|l| l.starts_with("Modelled")));
        insta::assert_json_snapshot!("waves_buoy_and_model_us_luna", card);
    }

    #[test]
    fn a_stale_buoy_gives_way_to_the_model() {
        let mut input = buoy_input();
        let now = input.now.0;
        input.snapshot.marine.waves = input.snapshot.marine.waves.take().map(|s| {
            let moved = s
                .samples()
                .iter()
                .map(|x| Sample::new(UnixTime(x.time.0 - 6 * 3_600), x.value.clone()))
                .collect();
            Series::new(s.provenance().clone(), moved)
        });
        input.snapshot.marine.waves_forecast = Some(model_series(&input));
        let card = build(&input).unwrap();
        assert!(card.full.observed.is_none());
        assert!(card.full.forecast.is_some());
        assert_eq!(card.compact.headline.text, "3.3");
        assert!(
            card.chrome
                .footnote
                .as_deref()
                .unwrap()
                .starts_with("Modelled waves")
        );
        assert_eq!(card.chrome.state, CardState::Fresh);
        assert!(
            card.full
                .height_chart
                .unwrap()
                .series
                .iter()
                .all(|s| s.role == SeriesRole::Modelled)
        );
        let _ = now;
    }

    #[test]
    fn a_stale_buoy_alone_is_shown_greyed() {
        let mut input = buoy_input();
        input.snapshot.marine.waves = input.snapshot.marine.waves.take().map(|s| {
            let moved = s
                .samples()
                .iter()
                .map(|x| Sample::new(UnixTime(x.time.0 - 6 * 3_600), x.value.clone()))
                .collect();
            Series::new(s.provenance().clone(), moved)
        });
        let card = build(&input).unwrap();
        assert!(matches!(card.chrome.state, CardState::Stale { .. }));
    }

    #[test]
    fn steepness_text_is_readable_or_absent() {
        assert_eq!(steepness_text("AVERAGE").as_deref(), Some("Average"));
        assert_eq!(steepness_text("N/A"), None);
        assert_eq!(steepness_text(""), None);
    }

    #[test]
    fn nothing_to_show_means_no_card() {
        let mut input = sample_input();
        input.snapshot.marine = crate::model::MarineConditions::default();
        assert!(build(&input).is_none());
        // A sample with no content is no waves either.
        let empty = Sample::new(input.now, WaveSample::default());
        input.snapshot.marine.waves = Some(Series::new(
            prov(
                SourceId::NDBC,
                ValueKind::Observed,
                Relation::Nearby,
                0,
                input.now.0,
            ),
            vec![empty],
        ));
        assert!(build(&input).is_none());
    }
}
