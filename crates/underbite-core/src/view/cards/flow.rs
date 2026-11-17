//! River flow and gage height against normal and the user's ranges.

use schemars::JsonSchema;
use serde::Serialize;

use crate::datum::{Height, VerticalDatum};
use crate::model::{FloodCategories, Measurement, Percentile, PercentileBasis, PercentileClass};
use crate::provenance::{Provenance, Quality, SourceId, StationRef};
use crate::series::{Sample, Series};
use crate::time::{LocalDate, UnixTime};
use crate::view::cards::wind::latest_at;
use crate::view::format::Formatter;
use crate::view::strings::{Key, SafetyKey};
use crate::view::{
    CardChrome, CardId, CardState, ChartPoint, ChartRule, ChartSeries, ChartTick, ChartVm, Compact,
    CreditVm, Notice, NoticeKind, Quantity, RiverRanges, Row, RuleKind, Section, SeriesRole,
    SeriesStyle, TrendDirection, TrendVm, ViewInput,
};

/// Seven days, the span of the sparklines and charts.
const WEEK_S: i64 = 7 * 86_400;
/// The trend compares the latest value with the one three hours earlier.
const TREND_HOURS: i64 = 3;
/// Discharge counts as moving when it changes by more than this share of its value over the trend
/// window.
const DISCHARGE_TREND_SHARE: f32 = 0.02;
/// Stage counts as moving beyond this many metres (about 0.03 ft).
const STAGE_TREND_M: f32 = 0.01;
/// Compact sparklines keep at most this many points.
const SPARK_POINTS: usize = 84;
const PROVISIONAL_STATEMENT: &str = "https://waterdata.usgs.gov/provisional-data-statement/";

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct FlowCard {
    pub chrome: CardChrome,
    pub compact: Compact,
    pub full: FlowFull,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RangeStatus {
    Wadeable,
    Best,
    Fishable,
    BlownOut,
    Unknown,
}

/// Where the discharge sits against the user's own ranges.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct RangeStatusVm {
    pub status: RangeStatus,
    /// "Best range (400 to 900 cfs)"
    pub text: String,
}

/// "Higher than 80% of Nov 14 days since 1990".
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct PercentileVm {
    pub text: String,
    pub class: Option<PercentileClass>,
    /// "Above normal"
    pub class_label: String,
    /// "Statistics API" or "Computed on this device from daily values"
    pub basis: String,
    /// "approved data only, 36 years"
    pub sample: String,
    /// Labelled thresholds, "10th percentile".
    pub thresholds: Vec<PercentileThresholdVm>,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct PercentileThresholdVm {
    pub label: String,
    pub value: Quantity,
}

/// A flood category: "Action", "Minor", "Moderate", "Major".
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct FloodCategoryVm {
    pub label: String,
    pub stage: Quantity,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct FlowFull {
    pub discharge: Option<Quantity>,
    pub trend: Option<TrendVm>,
    pub gage_height: Option<Quantity>,
    pub percentile: Option<PercentileVm>,
    pub range_status: Option<RangeStatusVm>,
    pub flood_categories: Vec<FloodCategoryVm>,
    /// Seven days of discharge, the NWPS forecast after now, user ranges and flood stages as rules.
    pub chart: Option<ChartVm>,
    /// A separate stage chart when the forecast is a stage hydrograph.
    pub stage_chart: Option<ChartVm>,
    pub sections: Vec<Section>,
}

// ---------------------------------------------------------------------------------------------
// Helpers shared by the water cards
// ---------------------------------------------------------------------------------------------

pub(super) fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    chars.next().map_or_else(String::new, |c| {
        c.to_uppercase().chain(chars).collect::<String>()
    })
}

pub(super) fn row(label: &str, value: &str) -> Row {
    Row {
        label: label.to_owned(),
        value: value.to_owned(),
        detail: None,
    }
}

/// The quality of a sample, falling back to the series' own.
pub(super) fn sample_quality<T>(series: &Series<T>, sample: &Sample<T>) -> Quality {
    sample.quality.unwrap_or(series.provenance().quality)
}

/// Ice-affected and flagged values are shown, marked unreliable.
pub(super) fn is_unreliable(q: Quality) -> bool {
    matches!(q, Quality::IceAffected | Quality::Flagged)
}

pub(super) fn unreliable_text(f: &Formatter, q: Quality) -> &'static str {
    f.strings.get(if q == Quality::IceAffected {
        Key::AffectedByIce
    } else {
        Key::FlaggedBySource
    })
}

/// Thins a sparkline to at most about [`SPARK_POINTS`] points, keeping the last.
pub(super) fn spark(values: &[f32]) -> Vec<f32> {
    if values.len() <= SPARK_POINTS {
        return values.to_vec();
    }
    let stride = values.len().div_ceil(SPARK_POINTS);
    let mut out: Vec<f32> = values.iter().copied().step_by(stride).collect();
    if !(values.len() - 1).is_multiple_of(stride)
        && let Some(&last) = values.last()
    {
        out.push(last);
    }
    out
}

/// The card state from the user's age and distance limits.
pub(super) fn card_state(
    input: &ViewInput,
    f: &Formatter,
    measurement: Measurement,
    p: &Provenance,
    latest: Option<UnixTime>,
) -> CardState {
    let Some(limit) = input.limit(measurement) else {
        return CardState::Fresh;
    };
    if let (Some(max), Some(d)) = (limit.max_distance_m, p.distance_m)
        && d > max
    {
        return CardState::Far {
            reason: f.far_reason(d, max),
        };
    }
    if let (Some(max), Some(t)) = (limit.max_age_s, latest) {
        let age = (input.now.0 - t.0).max(0);
        if age > max {
            return CardState::Stale {
                reason: f.stale_reason(age, max),
            };
        }
    }
    CardState::Fresh
}

/// Credits for the sources behind a card, from the snapshot's attributions; provisional USGS
/// values add the provisional data statement.
pub(in crate::view) fn credits(input: &ViewInput, provs: &[&Provenance]) -> Vec<CreditVm> {
    let f = Formatter::for_input(input);
    let mut out: Vec<CreditVm> = Vec::new();
    for p in provs {
        let name = f.strings.source_name(&p.source);
        let found = input
            .snapshot
            .attribution
            .iter()
            .find(|a| a.source == p.source);
        let text = match found {
            Some(a) => a.credit.clone(),
            // The credit text of a USGS value whose snapshot carries no attribution.
            None if p.source == SourceId::USGS => {
                f.strings.safety(SafetyKey::CreditUsgs).to_owned()
            }
            None => name.clone(),
        };
        let credit = CreditVm {
            source: name.clone(),
            text,
            link: found.and_then(|a| a.url.clone()),
            licence: None,
        };
        if !out.contains(&credit) {
            out.push(credit);
        }
        if p.source == SourceId::USGS && p.quality == Quality::Provisional {
            let statement = CreditVm {
                source: name,
                text: f.strings.get(Key::ProvisionalStatement).to_owned(),
                link: Some(PROVISIONAL_STATEMENT.to_owned()),
                licence: None,
            };
            if !out.contains(&statement) {
                out.push(statement);
            }
        }
    }
    out
}

/// The provisional notice when any shown value is provisional.
pub(super) fn quality_notices(f: &Formatter, qualities: &[Quality]) -> Vec<Notice> {
    if qualities.contains(&Quality::Provisional) {
        vec![f.notice(NoticeKind::Provisional)]
    } else {
        Vec::new()
    }
}

/// Day ticks along the span, on the local midnights of the spot's zone, labelled with the date.
pub(super) fn ticks(f: &Formatter, from: UnixTime, to: UnixTime) -> Vec<ChartTick> {
    f.local_midnights(from, to)
        .into_iter()
        .map(|t| ChartTick {
            t: t.0,
            label: f.date_of(t),
            major: true,
        })
        .collect()
}

/// Chart points from samples; `show` gives the display quantity of a canonical value. The first,
/// last, highest and lowest points keep their labels on narrow layouts.
pub(super) fn points(samples: &[Sample<f32>], show: &dyn Fn(f32) -> Quantity) -> Vec<ChartPoint> {
    let hi = samples
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.value.total_cmp(&b.1.value))
        .map(|(i, _)| i);
    let lo = samples
        .iter()
        .enumerate()
        .min_by(|a, b| a.1.value.total_cmp(&b.1.value))
        .map(|(i, _)| i);
    samples
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let q = show(s.value);
            ChartPoint {
                t: s.time.0,
                value: q.value.unwrap_or(0.0),
                label: q.full(),
                major: i == 0 || i + 1 == samples.len() || Some(i) == hi || Some(i) == lo,
            }
        })
        .collect()
}

pub(super) fn chart_series(
    label: &str,
    role: SeriesRole,
    samples: &[Sample<f32>],
    show: &dyn Fn(f32) -> Quantity,
) -> ChartSeries {
    ChartSeries {
        label: label.to_owned(),
        role,
        style: if role == SeriesRole::Observed {
            SeriesStyle::Line
        } else {
            SeriesStyle::Dots
        },
        points: points(samples, show),
    }
}

/// Words for a chart: its latest observed point and its extremes.
pub(super) fn describe(f: &Formatter, title: &str, series: &[ChartSeries]) -> String {
    let all: Vec<&ChartPoint> = series.iter().flat_map(|s| &s.points).collect();
    let (Some(hi), Some(lo)) = (
        all.iter().max_by(|a, b| a.value.total_cmp(&b.value)),
        all.iter().min_by(|a, b| a.value.total_cmp(&b.value)),
    ) else {
        return format!("{title}: {}", f.strings.get(Key::ChartNoData));
    };
    let last = series
        .iter()
        .find(|s| s.role == SeriesRole::Observed)
        .and_then(|s| s.points.last());
    match last {
        Some(l) => f.strings.fmt(
            Key::ChartNowHighLow,
            &[
                ("title", title),
                ("now", &l.label),
                ("high", &hi.label),
                ("low", &lo.label),
            ],
        ),
        None => f.strings.fmt(
            Key::ChartHighLow,
            &[("title", title), ("high", &hi.label), ("low", &lo.label)],
        ),
    }
}

/// The first and last time of a chart's series; now when there are none.
pub(super) fn span(series: &[ChartSeries], now: UnixTime) -> (UnixTime, UnixTime) {
    let first = series
        .iter()
        .filter_map(|s| s.points.first())
        .map(|p| p.t)
        .min();
    let last = series
        .iter()
        .filter_map(|s| s.points.last())
        .map(|p| p.t)
        .max();
    (
        UnixTime(first.unwrap_or(now.0)),
        UnixTime(last.unwrap_or(now.0)),
    )
}

/// Samples from `from` on.
pub(super) fn observed_after(series: &Series<f32>, from: UnixTime) -> &[Sample<f32>] {
    let samples = series.samples();
    &samples[samples.partition_point(|s| s.time < from)..]
}

pub(super) fn max_sample(samples: &[Sample<f32>]) -> Option<&Sample<f32>> {
    samples.iter().max_by(|a, b| a.value.total_cmp(&b.value))
}

/// The change over the last [`TREND_HOURS`] hours. `min_change` is the smallest canonical change
/// that counts as moving; `amount` formats a canonical change (always positive).
pub(super) fn trend(
    f: &Formatter,
    samples: &[Sample<f32>],
    min_change: f32,
    amount: &dyn Fn(f32) -> String,
) -> Option<TrendVm> {
    let last = samples.last()?;
    let base = samples
        .iter()
        .rev()
        .find(|s| s.time.0 <= last.time.0 - TREND_HOURS * 3_600)?;
    let hours = ((last.time.0 - base.time.0) as f32 / 3_600.0)
        .round()
        .max(1.0);
    let hours = f.strings.number(hours, 0);
    let delta = last.value - base.value;
    let (direction, text) = if delta.abs() < min_change {
        (
            TrendDirection::Steady,
            f.strings.fmt(Key::SteadyOver, &[("hours", &hours)]),
        )
    } else {
        let (d, key) = if delta > 0.0 {
            (TrendDirection::Rising, Key::Rising)
        } else {
            (TrendDirection::Falling, Key::Falling)
        };
        (
            d,
            f.strings.fmt(
                Key::TrendOver,
                &[
                    ("direction", f.strings.get(key)),
                    ("amount", &amount(delta.abs())),
                    ("hours", &hours),
                ],
            ),
        )
    };
    Some(TrendVm { direction, text })
}

// ---------------------------------------------------------------------------------------------
// The flow card
// ---------------------------------------------------------------------------------------------

fn ordinal_label(f: &Formatter, n: u8) -> String {
    f.strings.fmt(
        Key::PercentileRank,
        &[("ordinal", &f.strings.ordinal(u32::from(n)))],
    )
}

fn percentile_date(p: &Percentile) -> Option<LocalDate> {
    let (m, d) = p.day_of_year.split_once('-')?;
    // Only the month and the day are shown, so the year is a placeholder.
    Some(LocalDate {
        year: 2000,
        month: m.parse().ok()?,
        day: d.parse().ok()?,
    })
}

fn years(f: &Formatter, n: u32) -> String {
    f.strings.fmt(
        if n == 1 { Key::YearOne } else { Key::YearMany },
        &[("n", &n.to_string())],
    )
}

fn percentile_vm(
    f: &Formatter,
    p: &Percentile,
    show: &dyn Fn(f32) -> Quantity,
) -> Option<PercentileVm> {
    let on = percentile_date(p)?;
    let text = f.percentile(p.class, p.rank, p.rank_clamped, on)?;
    let class_label = match p.class? {
        PercentileClass::RecordLow => f.strings.get(Key::ClassRecordLow).to_owned(),
        PercentileClass::RecordHigh => f.strings.get(Key::ClassRecordHigh).to_owned(),
        PercentileClass::MuchBelowNormal => capitalize(f.strings.get(Key::PercentileMuchBelow)),
        PercentileClass::BelowNormal => capitalize(f.strings.get(Key::PercentileBelow)),
        PercentileClass::Normal => capitalize(f.strings.get(Key::PercentileNormal)),
        PercentileClass::AboveNormal => capitalize(f.strings.get(Key::PercentileAbove)),
        PercentileClass::MuchAboveNormal => capitalize(f.strings.get(Key::PercentileMuchAbove)),
    };
    let (basis, sample) = match p.basis {
        PercentileBasis::StatisticsApi => (
            f.strings.get(Key::BasisStatistics).to_owned(),
            if p.approved_only {
                f.strings
                    .fmt(Key::ApprovedOnly, &[("years", &years(f, p.sample_count))])
            } else {
                years(f, p.sample_count)
            },
        ),
        PercentileBasis::ComputedFromDaily { years: n } => (
            f.strings
                .fmt(Key::ComputedFromDaily, &[("years", &years(f, n))]),
            years(f, p.sample_count),
        ),
    };
    let t = &p.thresholds;
    let thresholds = [
        (5, t.p05),
        (10, t.p10),
        (25, t.p25),
        (50, t.p50),
        (75, t.p75),
        (90, t.p90),
        (95, t.p95),
    ]
    .into_iter()
    .filter_map(|(n, v)| {
        Some(PercentileThresholdVm {
            label: ordinal_label(f, n),
            value: show(v?),
        })
    })
    .collect();
    Some(PercentileVm {
        text,
        class: p.class,
        class_label,
        basis,
        sample,
        thresholds,
    })
}

#[allow(clippy::many_single_char_names)]
fn range_status(
    f: &Formatter,
    ranges: &RiverRanges,
    discharge: Option<(f32, Quality)>,
) -> Option<RangeStatusVm> {
    if ranges == &RiverRanges::default() {
        return None;
    }
    let show = |v: f32| f.discharge(Some(v)).text;
    let unit = f.discharge(None).unit;
    let Some((v, q)) = discharge else {
        return Some(RangeStatusVm {
            status: RangeStatus::Unknown,
            text: f.strings.get(Key::RangeNoDischarge).to_owned(),
        });
    };
    if is_unreliable(q) {
        return Some(RangeStatusVm {
            status: RangeStatus::Unknown,
            text: f
                .strings
                .fmt(Key::RangesNotApplied, &[("reason", unreliable_text(f, q))]),
        });
    }
    let (status, text) = if let Some(b) = ranges.blown_out_above_m3s.filter(|b| v > *b) {
        (
            RangeStatus::BlownOut,
            f.strings
                .fmt(Key::RangeBlownOut, &[("value", &show(b)), ("unit", &unit)]),
        )
    } else if ranges.best_min_m3s.is_none_or(|a| v >= a)
        && ranges.best_max_m3s.is_none_or(|b| v <= b)
        && (ranges.best_min_m3s.is_some() || ranges.best_max_m3s.is_some())
    {
        let range = match (ranges.best_min_m3s, ranges.best_max_m3s) {
            (Some(a), Some(b)) => f.strings.fmt(
                Key::RangeBetween,
                &[("from", &show(a)), ("to", &show(b)), ("unit", &unit)],
            ),
            (Some(a), None) => f
                .strings
                .fmt(Key::RangeFrom, &[("value", &show(a)), ("unit", &unit)]),
            (None, Some(b)) => f
                .strings
                .fmt(Key::RangeUpTo, &[("value", &show(b)), ("unit", &unit)]),
            (None, None) => String::new(),
        };
        (
            RangeStatus::Best,
            f.strings.fmt(Key::RangeBest, &[("range", &range)]),
        )
    } else if let Some(w) = ranges.wadeable_below_m3s.filter(|w| v < *w) {
        (
            RangeStatus::Wadeable,
            f.strings
                .fmt(Key::RangeWadeable, &[("value", &show(w)), ("unit", &unit)]),
        )
    } else {
        (
            RangeStatus::Fishable,
            f.strings.get(Key::RangeFishable).to_owned(),
        )
    };
    Some(RangeStatusVm { status, text })
}

fn range_rules(f: &Formatter, r: &RiverRanges) -> Vec<ChartRule> {
    let unit = f.discharge(None).unit;
    let rule = |v: Option<f32>, label: Key| {
        let q = f.discharge(v);
        q.value.map(|value| ChartRule {
            kind: RuleKind::UserRange,
            value,
            label: f.strings.fmt(label, &[("value", &q.text), ("unit", &unit)]),
        })
    };
    [
        rule(r.wadeable_below_m3s, Key::RuleWadeableBelow),
        rule(r.best_min_m3s, Key::RuleBestFrom),
        rule(r.best_max_m3s, Key::RuleBestUpTo),
        rule(r.blown_out_above_m3s, Key::RuleBlownOutAbove),
    ]
    .into_iter()
    .flatten()
    .collect()
}

fn flood_list(c: &FloodCategories) -> Vec<(Key, Height)> {
    [
        (Key::FloodAction, c.action),
        (Key::FloodMinor, c.minor),
        (Key::FloodModerate, c.moderate),
        (Key::FloodMajor, c.major),
    ]
    .into_iter()
    .filter_map(|(l, h)| Some((l, h?)))
    .collect()
}

/// A station id without the `USGS-` prefix some lists put in front of a site number.
fn bare_id(id: &str) -> &str {
    id.strip_prefix("USGS-").unwrap_or(id)
}

/// True when the flood gauge is the stage station itself or its forecast join: the same
/// station, or one that names the other as its reference.
fn same_gauge(flood: &StationRef, other: &StationRef) -> bool {
    let names = |a: &StationRef, b: &StationRef| {
        a.reference_id
            .as_deref()
            .is_some_and(|r| bare_id(r) == bare_id(&b.id))
    };
    (flood.source == other.source && bare_id(&flood.id) == bare_id(&other.id))
        || names(flood, other)
        || names(other, flood)
}

/// The flood stages of the spot's own gauge: those whose gauge is the gage-height station or
/// the forecast's station, never another river's. Stages without a recorded gauge are not shown.
fn own_flood_stages<'a>(
    c: Option<&'a FloodCategories>,
    gage: Option<&Provenance>,
    forecast: Option<&Provenance>,
) -> Option<(&'a FloodCategories, &'a Provenance)> {
    let c = c?;
    let p = c.provenance.as_ref()?;
    let site = p.station.as_ref()?;
    [gage, forecast]
        .into_iter()
        .flatten()
        .filter_map(|q| q.station.as_ref())
        .any(|s| same_gauge(site, s))
        .then_some((c, p))
}

fn forecast_after(series: &Series<f32>, now: UnixTime) -> &[Sample<f32>] {
    observed_after(series, now)
}

/// A chart over the given series, with rules and the day ticks.
pub(super) fn chart_vm(
    f: &Formatter,
    title: &str,
    unit: String,
    series: Vec<ChartSeries>,
    rules: Vec<ChartRule>,
    now: UnixTime,
) -> Option<ChartVm> {
    if series.is_empty() {
        return None;
    }
    let (first, last) = span(&series, now);
    Some(ChartVm {
        accessibility: describe(f, title, &series),
        title: title.to_owned(),
        unit,
        x_ticks: ticks(f, first, last),
        series,
        bands: Vec::new(),
        rules,
        markers: Vec::new(),
        now_t: (first.0 <= now.0 && now.0 <= last.0).then_some(now.0),
    })
}

#[allow(clippy::too_many_lines)]
pub fn build(input: &ViewInput) -> Option<FlowCard> {
    let w = &input.snapshot.water;
    let f = Formatter::for_input(input);
    let now = input.now;
    let from = UnixTime(now.0 - WEEK_S);

    let discharge = w.discharge_m3s.as_ref().filter(|s| !s.is_empty());
    // A lake or reservoir has a level instead of a stage.
    let lake_level = discharge.is_none() && w.gage_height.is_none();
    let (stage, stage_measurement) = if lake_level {
        (w.level.as_ref(), Measurement::LakeLevel)
    } else {
        (w.gage_height.as_ref(), Measurement::GageHeight)
    };
    let stage = stage.filter(|h| !h.series.is_empty());
    let q_forecast = w.discharge_forecast_m3s.as_ref().filter(|s| !s.is_empty());
    let s_forecast = w.stage_forecast.as_ref().filter(|h| !h.series.is_empty());
    if discharge.is_none() && stage.is_none() && q_forecast.is_none() && s_forecast.is_none() {
        return None;
    }

    let show_q = |v: f32| f.discharge(Some(v));
    let fr = &f;
    let stage_show = |datum: VerticalDatum| {
        move |v: f32| {
            let h = Some(Height { datum, metres: v });
            if lake_level {
                fr.lake_level(h)
            } else {
                fr.stage(h)
            }
        }
    };
    // A change in stage, or a level with no datum, in the group's unit.
    let stage_delta = |m: Option<f32>| {
        if lake_level {
            f.lake_level_length(m)
        } else {
            f.stage_length(m)
        }
    };

    // Values now.
    let dq = discharge.and_then(|s| s.latest().map(|l| (s, l)));
    let discharge_q = dq.map(|(_, l)| show_q(l.value));
    let dq_quality = dq.map(|(s, l)| (l.value, sample_quality(s, l)));
    let sq = stage.and_then(|h| h.series.latest().map(|l| (h, l)));
    let gage_height = sq.map(|(h, l)| stage_show(h.datum)(l.value));
    let sq_quality = sq.map(|(h, l)| sample_quality(&h.series, l));

    // Trend: discharge first, then stage.
    let trend_vm = dq
        .and_then(|(s, l)| {
            trend(
                &f,
                s.samples(),
                (l.value.abs() * DISCHARGE_TREND_SHARE).max(0.001),
                &|d| f.discharge(Some(d)).full(),
            )
        })
        .or_else(|| {
            sq.and_then(|(h, _)| {
                trend(&f, h.series.samples(), STAGE_TREND_M, &|d| {
                    stage_delta(Some(d)).full()
                })
            })
        });

    // Normal for this date. A level's datum is the percentile's own, else the level's.
    let pct_source = if discharge.is_some() {
        w.discharge_percentile_for_date.as_ref()
    } else {
        w.level_percentile_for_date.as_ref()
    };
    let pct_datum = pct_source.and_then(|p| p.datum).or(stage.map(|h| h.datum));
    let show_pct = |v: f32| match (discharge.is_some(), pct_datum) {
        (false, Some(datum)) => stage_show(datum)(v),
        (false, None) => stage_delta(Some(v)),
        (true, _) => f.discharge(Some(v)),
    };
    let percentile = pct_source.and_then(|p| percentile_vm(&f, p, &show_pct));

    let range_status = input
        .river_ranges
        .as_ref()
        .and_then(|r| range_status(&f, r, dq_quality));

    // Flood categories stay in their own (usually gage) datum.
    let own_flood = own_flood_stages(
        w.flood_categories.as_ref(),
        stage.map(|h| h.series.provenance()),
        s_forecast.map(|h| h.series.provenance()),
    );
    let flood: Vec<(Key, Height)> = own_flood.map(|(c, _)| flood_list(c)).unwrap_or_default();
    let flood_categories: Vec<FloodCategoryVm> = flood
        .iter()
        .map(|(label, h)| FloodCategoryVm {
            label: f.strings.get(*label).to_owned(),
            stage: f.stage(Some(*h)),
        })
        .collect();

    // Discharge chart: the week, the forecast, the user's ranges.
    let chart = {
        let mut series = Vec::new();
        if let Some(s) = discharge {
            let pts = observed_after(s, from);
            if !pts.is_empty() {
                series.push(chart_series(
                    f.strings.get(Key::Observed),
                    SeriesRole::Observed,
                    pts,
                    &show_q,
                ));
            }
        }
        if let Some(s) = q_forecast {
            let pts = forecast_after(s, now);
            if !pts.is_empty() {
                series.push(chart_series(
                    f.strings.get(Key::Forecast),
                    SeriesRole::Forecast,
                    pts,
                    &show_q,
                ));
            }
        }
        let rules = input
            .river_ranges
            .as_ref()
            .map(|r| range_rules(&f, r))
            .unwrap_or_default();
        chart_vm(
            &f,
            f.strings.get(Key::Discharge),
            f.discharge(None).unit,
            series,
            rules,
            now,
        )
    };

    // Stage chart: observed and forecast in one datum, flood rules only in that datum.
    let stage_chart = s_forecast
        .map(|h| h.datum)
        .or(stage.map(|h| h.datum))
        .and_then(|datum| {
            let show = stage_show(datum);
            let mut series = Vec::new();
            if let Some(h) = stage.filter(|h| h.datum == datum) {
                let pts = observed_after(&h.series, from);
                if !pts.is_empty() {
                    series.push(chart_series(
                        f.strings.get(Key::Observed),
                        SeriesRole::Observed,
                        pts,
                        &show,
                    ));
                }
            }
            if let Some(h) = s_forecast {
                let pts = forecast_after(&h.series, now);
                if !pts.is_empty() {
                    series.push(chart_series(
                        f.strings.get(Key::Forecast),
                        SeriesRole::Forecast,
                        pts,
                        &show,
                    ));
                }
            }
            let rules = flood
                .iter()
                .filter(|(_, h)| h.datum == datum)
                .filter_map(|(label, h)| {
                    let q = f.stage(Some(*h));
                    q.value.map(|value| ChartRule {
                        kind: RuleKind::FloodCategory,
                        value,
                        label: format!("{} {}", f.strings.get(*label), q.full()),
                    })
                })
                .collect();
            let title = f
                .strings
                .get(if lake_level { Key::Level } else { Key::Stage });
            chart_vm(&f, title, show(0.0).unit, series, rules, now)
        });

    // Sections.
    let mut sections = Vec::new();
    if let (Some(p), Some(src)) = (&percentile, pct_source) {
        let mut rows = vec![
            row(f.strings.get(Key::RowBasis), &p.basis),
            row(f.strings.get(Key::RowSample), &p.sample),
        ];
        if let Some(v) = src.thresholds.p50 {
            rows.push(row(f.strings.get(Key::RowMedian), &show_pct(v).full()));
        }
        if let (Some(lo), Some(hi)) = (src.min, src.max) {
            rows.push(row(
                f.strings.get(Key::RowRecordRange),
                &f.strings.fmt(
                    Key::Range,
                    &[("from", &show_pct(lo).full()), ("to", &show_pct(hi).full())],
                ),
            ));
        }
        sections.push(Section {
            title: f.strings.get(Key::SectionNormal).to_owned(),
            rows,
        });
    }
    if let Some(r) = &input.river_ranges {
        let rows: Vec<Row> = range_rules(&f, r)
            .into_iter()
            .map(|rule| row(&rule.label, ""))
            .collect();
        if !rows.is_empty() {
            sections.push(Section {
                title: f.strings.get(Key::SectionRanges).to_owned(),
                rows,
            });
        }
    }
    let crest_q = q_forecast
        .and_then(|s| max_sample(forecast_after(s, now)).map(|c| (c.time, show_q(c.value).full())));
    let crest_s = s_forecast.and_then(|h| {
        max_sample(forecast_after(&h.series, now))
            .map(|c| (c.time, stage_show(h.datum)(c.value).full()))
    });
    let rows: Vec<Row> = crest_q
        .into_iter()
        .chain(crest_s)
        .map(|(t, v)| Row {
            label: f.strings.get(Key::ForecastCrest).to_owned(),
            value: v,
            detail: Some(format!("{}, {}", f.date_of(t), f.clock(t))),
        })
        .collect();
    if !rows.is_empty() {
        sections.push(Section {
            title: f.strings.get(Key::Forecast).to_owned(),
            rows,
        });
    }
    // The gauge the flood stages belong to, named and credited.
    if let Some((_, p)) = own_flood {
        sections.push(Section {
            title: f.strings.get(Key::MeasFloodCategories).to_owned(),
            rows: vec![Row {
                label: f.strings.get(Key::Station).to_owned(),
                value: p
                    .station
                    .as_ref()
                    .map_or_else(|| f.strings.source_name(&p.source), |s| s.name.clone()),
                detail: Some(f.strings.source_name(&p.source)),
            }],
        });
    }
    let mut qualities: Vec<Quality> = Vec::new();
    qualities.extend(dq_quality.map(|(_, q)| q));
    qualities.extend(sq_quality);
    let unreliable = qualities.iter().copied().find(|q| is_unreliable(*q));
    if let Some(q) = unreliable {
        sections.push(Section {
            title: f.strings.get(Key::SectionReadingQuality).to_owned(),
            rows: vec![row(
                f.strings.get(Key::RowReliability),
                unreliable_text(&f, q),
            )],
        });
    }

    // Compact.
    let headline = discharge_q
        .clone()
        .or_else(|| gage_height.clone())
        .unwrap_or_else(|| f.discharge(None));
    let main_prov: Option<&Provenance> = dq
        .map(|(s, _)| s.provenance())
        .or_else(|| sq.map(|(h, _)| h.series.provenance()))
        .or_else(|| q_forecast.map(Series::provenance))
        .or_else(|| s_forecast.map(|h| h.series.provenance()));
    let mut lines: Vec<String> = Vec::new();
    lines.extend(unreliable.map(|q| unreliable_text(&f, q).to_owned()));
    lines.extend(percentile.as_ref().map(|p| p.text.clone()));
    lines.extend(range_status.as_ref().map(|r| r.text.clone()));
    lines.extend(main_prov.map(|p| f.station_footer(p).text));
    lines.truncate(2);
    let spark_values: Vec<f32> = if let Some(s) = discharge {
        observed_after(s, from)
            .iter()
            .map(|s| show_q(s.value).value.unwrap_or(0.0))
            .collect()
    } else if let Some(h) = stage {
        let show = stage_show(h.datum);
        observed_after(&h.series, from)
            .iter()
            .map(|s| show(s.value).value.unwrap_or(0.0))
            .collect()
    } else {
        Vec::new()
    };

    // Chrome.
    let prov_list: Vec<&Provenance> = dq
        .map(|(s, _)| s.provenance())
        .into_iter()
        .chain(sq.map(|(h, _)| h.series.provenance()))
        .chain(q_forecast.map(Series::provenance))
        .chain(s_forecast.map(|h| h.series.provenance()))
        .chain(own_flood.map(|(_, p)| p))
        .collect();
    let measurement = if discharge.is_some() {
        Measurement::Discharge
    } else {
        stage_measurement
    };
    let latest_t = dq.map(|(_, l)| l.time).or_else(|| sq.map(|(_, l)| l.time));
    let card = match main_prov {
        Some(p) if latest_t.is_some() => card_state(input, &f, measurement, p, latest_t),
        _ => CardState::Fresh,
    };
    let label = f.strings.get(if discharge.is_some() {
        Key::FlowWord
    } else if lake_level {
        Key::Level
    } else {
        Key::Stage
    });
    // The age is that of the newest sample at or before now in the series that leads.
    let shown_at = if let Some(s) = discharge {
        latest_at(s, now)
    } else if let Some(h) = stage {
        latest_at(&h.series, now)
    } else if let Some(s) = q_forecast {
        latest_at(s, now)
    } else {
        s_forecast.and_then(|h| latest_at(&h.series, now))
    }
    .map(|x| x.time);
    let footnote = main_prov
        .map(|p| f.footnote_aged(&format!("{label} {}", headline.full()), p, now, shown_at));
    let title = if lake_level {
        f.strings.get(Key::LakeLevel).to_owned()
    } else {
        f.strings.card_title(CardId::Flow).to_owned()
    };
    let mut accessibility = format!("{title}: {}", headline.full());
    if let Some(p) = &percentile {
        accessibility.push_str(", ");
        accessibility.push_str(&p.text);
    }
    if let Some(r) = &range_status {
        accessibility.push_str(". ");
        accessibility.push_str(&r.text);
    }
    if let Some(q) = unreliable {
        accessibility.push_str(". ");
        accessibility.push_str(unreliable_text(&f, q));
    }

    Some(FlowCard {
        chrome: CardChrome {
            id: CardId::Flow,
            title,
            footnote,
            credits: credits(input, &prov_list),
            state: card,
            notices: quality_notices(&f, &qualities),
            accessibility,
        },
        compact: Compact {
            headline,
            lines,
            spark: spark(&spark_values),
        },
        full: FlowFull {
            discharge: discharge_q,
            trend: trend_vm,
            gage_height,
            percentile,
            range_status,
            flood_categories,
            chart,
            stage_chart,
            sections,
        },
    })
}

/// Builders for the tests of the three water cards.
#[cfg(test)]
pub(in crate::view) mod fixtures {
    use super::*;
    use crate::model::{WaterConditions, WaterType};
    use crate::provenance::{StationKind, UsgsSiteType, ValueKind};
    use crate::units::{
        DischargeUnit, DistanceUnit, HeightUnit, PressureUnit, SpeedUnit, TempUnit,
    };
    use crate::view::{DisplayUnits, Voice};

    /// "Now" of the sample snapshot.
    pub(in crate::view) const NOW: i64 = 1_794_000_000;
    pub(in crate::view) const CFS: f32 = 0.028_316_847;
    pub(in crate::view) const FT: f32 = 0.3048;

    pub(in crate::view) fn us() -> DisplayUnits {
        DisplayUnits {
            temperature: TempUnit::F,
            air_temperature: TempUnit::F,
            height: HeightUnit::Ft,
            stage: HeightUnit::Ft,
            lake_level: HeightUnit::Ft,
            waves: HeightUnit::Ft,
            discharge: DischargeUnit::Cfs,
            current_speed: SpeedUnit::Kn,
            wind_speed: SpeedUnit::Mph,
            pressure: PressureUnit::InHg,
            distance: DistanceUnit::Mi,
        }
    }

    pub(in crate::view) fn metric() -> DisplayUnits {
        DisplayUnits {
            temperature: TempUnit::C,
            air_temperature: TempUnit::C,
            height: HeightUnit::M,
            stage: HeightUnit::M,
            lake_level: HeightUnit::M,
            waves: HeightUnit::M,
            discharge: DischargeUnit::M3s,
            current_speed: SpeedUnit::Ms,
            wind_speed: SpeedUnit::Kmh,
            pressure: PressureUnit::Hpa,
            distance: DistanceUnit::Km,
        }
    }

    pub(in crate::view) fn prov(
        source: SourceId,
        id: &str,
        name: &str,
        kind: StationKind,
        value_kind: ValueKind,
        quality: Quality,
    ) -> Provenance {
        Provenance {
            source: source.clone(),
            station: Some(crate::provenance::StationRef {
                source,
                id: id.into(),
                name: name.into(),
                kind,
                latitude: 41.2,
                longitude: -73.9,
                bin: None,
                reference_id: None,
            }),
            distance_m: Some(2_400.0),
            relation: crate::provenance::Relation::SameWaterBody,
            observed_at: UnixTime(NOW),
            kind: value_kind,
            quality,
        }
    }

    /// The Croton River USGS site.
    pub(in crate::view) fn croton(quality: Quality) -> Provenance {
        prov(
            SourceId::USGS,
            "01375000",
            "Croton River at New Croton Dam",
            StationKind::UsgsSite(UsgsSiteType::Stream),
            ValueKind::Observed,
            quality,
        )
    }

    /// Samples every `step_s` seconds ending at `end`, oldest first.
    pub(in crate::view) fn run(
        p: Provenance,
        end: i64,
        step_s: i64,
        values: &[f32],
    ) -> Series<f32> {
        let n = i64::try_from(values.len()).unwrap_or(0);
        let samples = values
            .iter()
            .enumerate()
            .map(|(i, v)| {
                Sample::new(
                    UnixTime(end - (n - 1 - i64::try_from(i).unwrap_or(0)) * step_s),
                    *v,
                )
            })
            .collect();
        Series::new(p, samples)
    }

    /// A linear ramp of `n` values.
    pub(in crate::view) fn ramp(from: f32, to: f32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| from + (to - from) * i as f32 / (n - 1) as f32)
            .collect()
    }

    /// A river spot with no water data, in the sample snapshot's moment.
    pub(in crate::view) fn input(units: DisplayUnits, voice: Voice) -> ViewInput {
        let mut input = crate::view::tests::sample_input();
        input.units = units;
        input.voice = voice;
        input.snapshot.spot.water_type = WaterType::RiverStream;
        input.snapshot.spot.name = "Croton River".into();
        input.snapshot.water = WaterConditions::default();
        input.snapshot.marine = crate::model::MarineConditions::default();
        input.snapshot.attribution = vec![crate::provenance::Attribution {
            source: SourceId::USGS,
            credit: "U.S. Geological Survey".into(),
            url: Some("https://www.usgs.gov".into()),
        }];
        input.limits = Vec::new();
        input
    }

    /// Croton 12.4 cfs against the 8 November normals (93 years, 5th percentile 0.634 cfs,
    /// median 15.0 cfs, 95th 1,180 cfs).
    pub(in crate::view) fn croton_input(units: DisplayUnits, voice: Voice) -> ViewInput {
        use crate::model::PercentileThresholds;
        let mut input = input(units, voice);
        let p = croton(Quality::Provisional);
        input.snapshot.water.discharge_m3s =
            Some(run(p.clone(), NOW, 900, &ramp(11.0 * CFS, 12.4 * CFS, 25)));
        input.snapshot.water.discharge_percentile_for_date = Some(Percentile {
            day_of_year: "11-08".into(),
            thresholds: PercentileThresholds {
                p05: Some(0.634 * CFS),
                p50: Some(15.0 * CFS),
                p95: Some(1180.0 * CFS),
                ..PercentileThresholds::default()
            },
            min: None,
            max: None,
            mean: None,
            value: 12.4 * CFS,
            rank: Some(45.0),
            rank_clamped: false,
            class: Some(PercentileClass::Normal),
            basis: PercentileBasis::StatisticsApi,
            sample_count: 93,
            approved_only: true,
            datum: None,
            provenance: Some(p),
        });
        input
    }
}

#[cfg(test)]
pub(in crate::view) mod tests {
    use super::fixtures::*;
    use super::*;
    use crate::datum::Heights;
    use crate::model::PercentileThresholds;
    use crate::provenance::{StationKind, UsgsSiteType, ValueKind};
    use crate::units::HeightUnit;
    use crate::view::Voice;

    fn card(input: &ViewInput) -> FlowCard {
        build(input).expect("a flow card")
    }

    #[test]
    fn croton_reads_normal_on_the_statistics_basis() {
        let c = card(&croton_input(us(), Voice::Plain));
        let p = c.full.percentile.as_ref().unwrap();
        assert_eq!(p.text, "normal, 45th percentile for Nov 8");
        assert_eq!(p.class, Some(PercentileClass::Normal));
        assert_eq!(p.class_label, "Normal");
        assert_eq!(p.basis, "USGS statistics (beta)");
        assert_eq!(p.sample, "approved data only, 93 years");
        let labels: Vec<&str> = p.thresholds.iter().map(|t| t.label.as_str()).collect();
        assert_eq!(
            labels,
            ["5th percentile", "50th percentile", "95th percentile"]
        );
        assert_eq!(p.thresholds[1].value.full(), "15.0 cfs");
        assert_eq!(c.compact.headline.full(), "12.4 cfs");
        assert_eq!(c.full.trend.as_ref().unwrap().text, "Rising 0.7 cfs in 3 h");
        assert_eq!(c.chrome.notices.len(), 1);
        assert_eq!(
            c.chrome.notices[0].text,
            "Provisional data, subject to revision."
        );
        assert!(c.chrome.credits.iter().any(|c| {
            c.link.as_deref() == Some("https://waterdata.usgs.gov/provisional-data-statement/")
        }));
    }

    #[test]
    fn computed_basis_is_named_and_not_called_approved() {
        let mut input = croton_input(us(), Voice::Plain);
        let p = input
            .snapshot
            .water
            .discharge_percentile_for_date
            .as_mut()
            .unwrap();
        p.basis = PercentileBasis::ComputedFromDaily { years: 16 };
        p.sample_count = 16;
        let c = card(&input);
        let p = c.full.percentile.unwrap();
        assert_eq!(p.basis, "computed from 16 years of daily values");
        assert_eq!(p.sample, "16 years");
        assert!(!p.sample.contains("approved"));
    }

    #[test]
    fn ranges_colour_the_value_in_either_unit() {
        let ranges = RiverRanges {
            wadeable_below_m3s: Some(8.0 * CFS),
            best_min_m3s: Some(10.0 * CFS),
            best_max_m3s: Some(20.0 * CFS),
            blown_out_above_m3s: Some(50.0 * CFS),
        };
        let mut input = croton_input(us(), Voice::Plain);
        input.river_ranges = Some(ranges);
        let c = card(&input);
        let r = c.full.range_status.as_ref().unwrap();
        assert_eq!(r.status, RangeStatus::Best);
        assert_eq!(r.text, "Best range (10.0 to 20.0 cfs)");
        let chart = c.full.chart.as_ref().unwrap();
        let rules: Vec<f32> = chart.rules.iter().map(|r| r.value.round()).collect();
        assert_eq!(rules, [8.0, 10.0, 20.0, 50.0]);

        // The same gauge for a user in m³/s: the comparison happens in canonical units.
        input.units = metric();
        let c = card(&input);
        assert_eq!(
            c.full.range_status.as_ref().unwrap().status,
            RangeStatus::Best
        );

        // Below the wadeable limit, above the blown-out one, outside the best range.
        input.units = us();
        input.river_ranges = Some(RiverRanges {
            wadeable_below_m3s: Some(30.0 * CFS),
            ..RiverRanges::default()
        });
        let r = card(&input).full.range_status.unwrap();
        assert_eq!(r.status, RangeStatus::Wadeable);
        assert_eq!(r.text, "Wadeable (below 30.0 cfs)");
        input.river_ranges = Some(RiverRanges {
            blown_out_above_m3s: Some(5.0 * CFS),
            ..RiverRanges::default()
        });
        let r = card(&input).full.range_status.unwrap();
        assert_eq!(r.status, RangeStatus::BlownOut);
        input.river_ranges = Some(RiverRanges {
            best_min_m3s: Some(14.0 * CFS),
            best_max_m3s: Some(20.0 * CFS),
            ..RiverRanges::default()
        });
        assert_eq!(
            card(&input).full.range_status.unwrap().status,
            RangeStatus::Fishable
        );
        input.river_ranges = Some(RiverRanges::default());
        assert!(card(&input).full.range_status.is_none());
    }

    #[test]
    fn ice_affected_values_are_shown_as_unreliable() {
        let mut input = croton_input(us(), Voice::Plain);
        input.river_ranges = Some(RiverRanges {
            blown_out_above_m3s: Some(50.0 * CFS),
            ..RiverRanges::default()
        });
        let p = croton(Quality::IceAffected);
        input.snapshot.water.discharge_m3s =
            Some(run(p, NOW, 900, &ramp(11.0 * CFS, 12.4 * CFS, 25)));
        let c = card(&input);
        assert_eq!(c.compact.headline.full(), "12.4 cfs");
        assert_eq!(c.compact.lines[0], "Affected by ice: reading unreliable");
        let r = c.full.range_status.unwrap();
        assert_eq!(r.status, RangeStatus::Unknown);
        assert!(r.text.contains("unreliable"));
        assert_eq!(c.chrome.notices.len(), 0);
    }

    fn nwps_gauge(id: &str, name: &str) -> Provenance {
        prov(
            SourceId::NWPS,
            id,
            name,
            StationKind::NwpsGauge,
            ValueKind::Modelled,
            Quality::Provisional,
        )
    }

    /// COHN6 forecast stage, flood categories at 17, 20, 21 and 22 ft above the gauge datum.
    pub(in crate::view) fn cohn6_input(units: crate::view::DisplayUnits) -> ViewInput {
        let mut input = croton_input(units, Voice::Plain);
        let nwps = prov(
            SourceId::NWPS,
            "COHN6",
            "Mohawk River at Cohoes",
            StationKind::NwpsGauge,
            ValueKind::Predicted,
            Quality::Provisional,
        );
        let ft = |v: f32| v * FT;
        input.snapshot.water.stage_forecast = Some(Heights {
            datum: VerticalDatum::Gauge,
            series: run(
                nwps,
                NOW + 6 * 3_600,
                3_600,
                &[ft(15.0), ft(18.5), ft(20.4), ft(19.0)],
            ),
        });
        let h = |v: f32| {
            Some(Height {
                datum: VerticalDatum::Gauge,
                metres: ft(v),
            })
        };
        input.snapshot.water.flood_categories = Some(FloodCategories {
            action: h(17.0),
            minor: h(20.0),
            moderate: h(21.0),
            major: h(22.0),
            provenance: Some(nwps_gauge("COHN6", "Mohawk River at Cohoes")),
        });
        // An observed stage in another datum is never merged into the forecast chart.
        input.snapshot.water.gage_height = Some(Heights {
            datum: VerticalDatum::Navd88,
            series: run(croton(Quality::Provisional), NOW, 900, &[1.2, 1.3]),
        });
        input
    }

    #[test]
    fn cohn6_forecast_carries_flood_lines_in_display_units() {
        let c = card(&cohn6_input(us()));
        let chart = c.full.stage_chart.as_ref().unwrap();
        let rules: Vec<(f32, &str)> = chart
            .rules
            .iter()
            .map(|r| ((r.value * 10.0).round() / 10.0, r.label.as_str()))
            .collect();
        assert_eq!(
            rules,
            [
                (17.0, "Action 17.0 ft gage datum"),
                (20.0, "Minor 20.0 ft gage datum"),
                (21.0, "Moderate 21.0 ft gage datum"),
                (22.0, "Major 22.0 ft gage datum"),
            ]
        );
        assert!(
            chart
                .rules
                .iter()
                .all(|r| r.kind == RuleKind::FloodCategory)
        );
        assert_eq!(chart.unit, "ft gage datum");
        assert_eq!(chart.series.len(), 1);
        assert_eq!(chart.series[0].role, SeriesRole::Forecast);
        let crest = &c
            .full
            .sections
            .iter()
            .find(|s| s.title == "Forecast")
            .unwrap()
            .rows;
        assert_eq!(crest[0].value, "20.4 ft gage datum");
        assert_eq!(c.full.flood_categories.len(), 4);

        let c = card(&cohn6_input(metric()));
        let chart = c.full.stage_chart.as_ref().unwrap();
        assert!((chart.rules[0].value - 5.2).abs() < 0.05);
        assert_eq!(chart.unit, "m gage datum");
    }

    #[test]
    fn flood_stages_name_and_credit_the_gauge_they_belong_to() {
        let mut input = cohn6_input(us());
        input
            .snapshot
            .attribution
            .push(crate::provenance::Attribution {
                source: SourceId::NWPS,
                credit: "NOAA National Water Prediction Service".into(),
                url: None,
            });
        let c = card(&input);
        let section = c
            .full
            .sections
            .iter()
            .find(|s| s.title == "Flood categories")
            .unwrap();
        assert_eq!(section.rows[0].value, "Mohawk River at Cohoes");
        assert!(
            c.chrome
                .credits
                .iter()
                .any(|cr| cr.text == "NOAA National Water Prediction Service")
        );
        // The observed gauge joined to the forecast gauge (the NWPS gauge names its USGS site).
        let mut joined = cohn6_input(us());
        joined.snapshot.water.stage_forecast = None;
        let mut gauge = nwps_gauge("COHN6", "Mohawk River at Cohoes");
        gauge.station.as_mut().unwrap().reference_id = Some("01375000".into());
        joined
            .snapshot
            .water
            .flood_categories
            .as_mut()
            .unwrap()
            .provenance = Some(gauge);
        joined.snapshot.water.gage_height = Some(Heights {
            datum: VerticalDatum::Gauge,
            series: run(croton(Quality::Provisional), NOW, 900, &[1.2, 1.3]),
        });
        let c = card(&joined);
        assert_eq!(c.full.flood_categories.len(), 4);
        assert_eq!(c.full.stage_chart.unwrap().rules.len(), 4);
    }

    #[test]
    fn another_gauges_flood_stages_are_not_shown() {
        let mut input = cohn6_input(us());
        input
            .snapshot
            .water
            .flood_categories
            .as_mut()
            .unwrap()
            .provenance = Some(nwps_gauge("WNYN6", "Hackensack River at New Milford"));
        let c = card(&input);
        assert_eq!(c.full.flood_categories.len(), 0);
        assert_eq!(c.full.stage_chart.unwrap().rules.len(), 0);
        assert!(
            c.full
                .sections
                .iter()
                .all(|s| s.title != "Flood categories")
        );
        assert!(c.chrome.credits.iter().all(|cr| cr.source != "NWPS"));
        // Stages with no recorded gauge are not shown either.
        let mut input = cohn6_input(us());
        input
            .snapshot
            .water
            .flood_categories
            .as_mut()
            .unwrap()
            .provenance = None;
        assert_eq!(card(&input).full.flood_categories.len(), 0);
    }

    #[test]
    fn flood_lines_are_not_drawn_in_another_datum() {
        let mut input = cohn6_input(us());
        input.snapshot.water.stage_forecast = None;
        input
            .snapshot
            .water
            .flood_categories
            .as_mut()
            .unwrap()
            .provenance = Some(croton(Quality::Provisional));
        let c = card(&input);
        let chart = c.full.stage_chart.unwrap();
        assert_eq!(chart.rules.len(), 0);
        assert_eq!(chart.unit, "ft NAVD88");
        assert_eq!(c.full.flood_categories.len(), 4);
    }

    #[test]
    fn lake_level_uses_its_datum_and_percentile() {
        let mut input = input(us(), Voice::Plain);
        let p = prov(
            SourceId::USGS,
            "01318500",
            "Great Sacandaga Lake",
            StationKind::UsgsSite(UsgsSiteType::Lake),
            ValueKind::Observed,
            Quality::Approved,
        );
        input.snapshot.spot.water_type = crate::model::WaterType::LakeReservoir;
        let m = |ft: f32| ft * FT;
        input.snapshot.water.level = Some(Heights {
            datum: VerticalDatum::Ngvd29,
            series: run(p.clone(), NOW, 900, &[m(755.3), m(755.49)]),
        });
        input.snapshot.water.level_percentile_for_date = Some(Percentile {
            day_of_year: "11-08".into(),
            thresholds: PercentileThresholds {
                p50: Some(m(754.97)),
                ..PercentileThresholds::default()
            },
            min: None,
            max: None,
            mean: None,
            value: m(755.49),
            rank: Some(55.0),
            rank_clamped: false,
            class: Some(PercentileClass::Normal),
            basis: PercentileBasis::ComputedFromDaily { years: 96 },
            sample_count: 96,
            approved_only: true,
            datum: Some(VerticalDatum::Ngvd29),
            provenance: Some(p),
        });
        let c = card(&input);
        assert_eq!(c.chrome.title, "Lake level");
        assert_eq!(
            c.full.gage_height.as_ref().unwrap().full(),
            "755.5 ft NGVD29"
        );
        let pct = c.full.percentile.unwrap();
        assert_eq!(pct.thresholds[0].value.full(), "755.0 ft NGVD29");
        assert!(c.full.discharge.is_none());
        assert_eq!(c.chrome.notices.len(), 0);
    }

    #[test]
    fn stage_and_lake_level_use_their_own_units_not_the_tide_unit() {
        let mut units = us();
        units.stage = HeightUnit::M;
        units.lake_level = HeightUnit::M;
        let mut input = croton_input(units, Voice::Plain);
        input.snapshot.water.gage_height = Some(Heights {
            datum: VerticalDatum::Navd88,
            series: run(croton(Quality::Provisional), NOW, 900, &[1.2, 1.3]),
        });
        let c = card(&input);
        assert_eq!(c.full.gage_height.as_ref().unwrap().full(), "1.3 m NAVD88");
        assert_eq!(
            c.full.stage_chart.as_ref().unwrap().unit,
            "m NAVD88",
            "stage chart"
        );
        // Tide heights stay in feet.
        assert_eq!(
            Formatter::for_input(&input)
                .height(Some(Height {
                    datum: VerticalDatum::Mllw,
                    metres: 0.3048,
                }))
                .full(),
            "1.0 ft MLLW"
        );

        let mut lake = croton_input(units, Voice::Plain);
        lake.snapshot.water.discharge_m3s = None;
        lake.snapshot.water.discharge_percentile_for_date = None;
        lake.snapshot.water.level = Some(Heights {
            datum: VerticalDatum::Ngvd29,
            series: run(croton(Quality::Approved), NOW, 900, &[230.0, 230.5]),
        });
        let c = card(&lake);
        assert_eq!(
            c.full.gage_height.as_ref().unwrap().full(),
            "230.5 m NGVD29"
        );
        lake.units.lake_level = HeightUnit::Ft;
        let c = card(&lake);
        assert_eq!(
            c.full.gage_height.as_ref().unwrap().full(),
            "756.2 ft NGVD29"
        );
    }

    #[test]
    fn chart_day_ticks_fall_on_local_midnights_across_the_november_change() {
        let input = croton_input(us(), Voice::Plain);
        assert_eq!(input.snapshot.spot.time_zone, "America/New_York");
        let f = Formatter::for_input(&input);
        // Friday 30 October noon EDT to Tuesday 3 November noon EST; clocks went back on the 1st.
        let ticks = ticks(&f, UnixTime(1_793_376_000), UnixTime(1_793_725_200));
        let at: Vec<i64> = ticks.iter().map(|t| t.t).collect();
        assert_eq!(
            at,
            [1_793_419_200, 1_793_505_600, 1_793_595_600, 1_793_682_000]
        );
        let labels: Vec<&str> = ticks.iter().map(|t| t.label.as_str()).collect();
        assert_eq!(labels, ["Oct 31", "Nov 1", "Nov 2", "Nov 3"]);
        // The day the clocks went back is 25 hours long.
        assert_eq!(at[2] - at[1], 25 * 3_600);
    }

    #[test]
    fn the_footnote_age_is_that_of_the_newest_sample_at_or_before_now() {
        let mut input = croton_input(us(), Voice::Plain);
        let mut p = croton(Quality::Provisional);
        p.observed_at = UnixTime(NOW - 2 * 86_400);
        // Samples from an hour ago to now, and one ahead of now.
        let mut s = run(p, NOW + 3_600, 3_600, &[1.0, 2.0, 3.0]);
        s = Series::new(s.provenance().clone(), s.samples().to_vec());
        input.snapshot.water.discharge_m3s = Some(s);
        let text = build(&input).unwrap().chrome.footnote.unwrap();
        assert!(text.ends_with("just now"), "{text}");
    }

    #[test]
    fn no_data_no_card() {
        assert!(build(&input(us(), Voice::Plain)).is_none());
    }

    #[test]
    fn far_and_stale_limits_grey_the_card() {
        let mut input = croton_input(us(), Voice::Plain);
        input.limits = vec![crate::view::MeasurementLimit {
            measurement: Measurement::Discharge,
            max_distance_m: Some(1_000.0),
            max_age_s: None,
        }];
        assert!(matches!(card(&input).chrome.state, CardState::Far { .. }));
        input.limits[0].max_distance_m = None;
        input.limits[0].max_age_s = Some(60);
        input.now = UnixTime(NOW + 3_600);
        assert!(matches!(card(&input).chrome.state, CardState::Stale { .. }));
    }

    #[test]
    fn snapshots_in_both_voices_and_unit_systems() {
        insta::assert_json_snapshot!("croton_luna_us", card(&croton_input(us(), Voice::Luna)));
        insta::assert_json_snapshot!(
            "croton_plain_metric",
            card(&croton_input(metric(), Voice::Plain))
        );
        insta::assert_json_snapshot!("cohn6_plain_us", card(&cohn6_input(us())));
    }
}
