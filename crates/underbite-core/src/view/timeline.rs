//! The day timeline: tide, current, wind, pressure, light and solunar lanes.
//!
//! Every lane shares one time axis: the spot's local day (or days) from midnight, sampled at the
//! requested step. Steps are walked as instants, so a daylight-saving day has 23 or 25 hourly
//! steps. A lane with no data is still present, with no series, bands or markers, so rows line up.

use jiff::tz::TimeZone;

use crate::datum::{Height, VerticalDatum};
use crate::model::{
    CurrentEvent, CurrentKind, Polar, SkyDay, SolunarKind, TideKind, TideSourceKind,
};
use crate::time::{UnixTime, local_date, local_date_bounds, zone};
use crate::view::cards::current::notices as current_notices;
use crate::view::cards::flow::credits as station_credits;
use crate::view::cards::solunar::{kind_label, range_text};
use crate::view::cards::sun_moon::computed_credit;
use crate::view::cards::tide::{credits as tide_credits, notices as tide_notices};
use crate::view::chart::{self, Zoned, level_at, speed_at};
use crate::view::format::{Formatter, Phase};
use crate::view::strings::{Key, SafetyKey};
use crate::view::{
    BandKind, CardId, ChartBand, ChartMarker, ChartPoint, ChartSeries, ChartTick, ChartVm,
    CreditVm, LaneKind, MarkerKind, Notice, Quantity, SeriesRole, SeriesStyle, TimelineLane,
    TimelineVm, ViewInput,
};

/// The widest gap a curve or hourly series is interpolated across.
/// Half the span marked as slack around a slack event, seconds.
const SLACK_HALF_WINDOW_S: i64 = 15 * 60;

/// What every lane needs.
struct Ctx<'a> {
    input: &'a ViewInput,
    f: Formatter,
    local: Zoned,
    start: i64,
    end: i64,
    /// Instants, one per step, from `start` while before `end`.
    steps: Vec<i64>,
    ticks: Vec<ChartTick>,
}

/// The span: midnight of the spot's local today to `hours` later, on local midnights when the span
/// is whole days (so a daylight-saving day is 23 or 25 hours long).
fn window(input: &ViewInput, tz: &TimeZone) -> Option<(i64, i64)> {
    let hours = i64::from(input.timeline_span.hours);
    if hours == 0 {
        return None;
    }
    let today = local_date(input.now, tz).ok()?;
    let (start, mut end) = local_date_bounds(today, tz).ok()?;
    if hours % 24 != 0 {
        return Some((start.0, start.0 + hours * 3_600));
    }
    for _ in 1..hours / 24 {
        let next = local_date(end, tz).ok()?;
        end = local_date_bounds(next, tz).ok()?.1;
    }
    Some((start.0, end.0))
}

impl Ctx<'_> {
    fn in_window(&self, t: i64) -> bool {
        t >= self.start && t < self.end
    }

    /// One point per step where `value` (canonical units) is known, formatted by `show`.
    fn points(
        &self,
        value: impl Fn(i64) -> Option<f32>,
        show: impl Fn(f32) -> Quantity,
    ) -> Vec<ChartPoint> {
        self.steps
            .iter()
            .filter_map(|&t| {
                let q = show(value(t)?);
                Some(ChartPoint {
                    t,
                    value: q.value.unwrap_or_default(),
                    label: q.full(),
                    major: self.local.on_the_hour(t),
                })
            })
            .collect()
    }

    #[allow(clippy::too_many_arguments)]
    fn chart(
        &self,
        name: &str,
        credit: &str,
        unit: &str,
        series: Vec<ChartSeries>,
        bands: Vec<ChartBand>,
        markers: Vec<ChartMarker>,
        summary: &str,
    ) -> ChartVm {
        let empty = series.is_empty() && bands.is_empty() && markers.is_empty();
        let (title, accessibility) = if empty {
            (
                name.to_owned(),
                format!("{name}: {}", self.f.strings.get(Key::NoData)),
            )
        } else {
            (format!("{name} · {credit}"), format!("{name}: {summary}"))
        };
        ChartVm {
            title,
            unit: unit.to_owned(),
            x_ticks: self.ticks.clone(),
            series,
            bands,
            rules: Vec::new(),
            markers,
            now_t: (self.in_window(self.input.now.0)).then_some(self.input.now.0),
            accessibility,
        }
    }

    fn empty(&self, name: &str) -> ChartVm {
        self.chart(name, "", "", Vec::new(), Vec::new(), Vec::new(), "")
    }

    fn clip(&self, a: i64, b: i64) -> Option<(i64, i64)> {
        let (a, b) = (a.max(self.start), b.min(self.end));
        (a < b).then_some((a, b))
    }
}

/// "Low 0.0 ft at 1:34 PM" style summary of a series' extremes.
fn extremes(points: &[ChartPoint], f: &Formatter) -> String {
    let lo = points.iter().min_by(|a, b| a.value.total_cmp(&b.value));
    let hi = points.iter().max_by(|a, b| a.value.total_cmp(&b.value));
    match (lo, hi) {
        (Some(lo), Some(hi)) => f.strings.fmt(
            Key::ExtremesAt,
            &[
                ("lowest", &lo.label),
                ("lowest_time", &f.clock(UnixTime(lo.t))),
                ("highest", &hi.label),
                ("highest_time", &f.clock(UnixTime(hi.t))),
            ],
        ),
        _ => String::new(),
    }
}

// ---------------------------------------------------------------------------------------------
// Tide
// ---------------------------------------------------------------------------------------------

#[allow(clippy::too_many_lines)]
fn tide_lane(ctx: &Ctx) -> ChartVm {
    let name = ctx.f.strings.card_title(CardId::Tide);
    let Some(tide) = ctx.input.snapshot.marine.tide.as_ref() else {
        return ctx.empty(name);
    };
    let f = &ctx.f;
    let mut events = tide.events.clone();
    events.sort_by_key(|e| e.time);
    let shown =
        tide.source_kind != TideSourceKind::ModelMean && tide.datum != VerticalDatum::ModelMean;
    let levels: Vec<(i64, f32)> = ctx
        .steps
        .iter()
        .filter_map(|&t| Some((t, level_at(tide, &events, UnixTime(t))?)))
        .collect();
    if levels.is_empty() && events.is_empty() {
        return ctx.empty(name);
    }
    let series_points: Vec<ChartPoint> = if shown {
        ctx.points(
            |t| levels.iter().find(|(s, _)| *s == t).map(|(_, v)| *v),
            |m| {
                f.height(Some(Height {
                    datum: tide.datum,
                    metres: m,
                }))
            },
        )
    } else {
        // A relative curve: scaled to 0..1, labelled with the time, never with a height.
        let (lo, hi) = levels
            .iter()
            .fold((f32::MAX, f32::MIN), |(lo, hi), (_, v)| {
                (lo.min(*v), hi.max(*v))
            });
        levels
            .iter()
            .map(|(t, v)| ChartPoint {
                t: *t,
                value: if hi > lo { (v - lo) / (hi - lo) } else { 0.5 },
                label: f.clock(UnixTime(*t)),
                major: ctx.local.on_the_hour(*t),
            })
            .collect()
    };
    // Highs and lows are events at their own times, never taken from the hourly samples.
    let markers: Vec<ChartMarker> = events
        .iter()
        .filter(|e| ctx.in_window(e.time.0))
        .map(|e| {
            let word = f.strings.tide_kind(e.kind);
            let q = shown.then(|| {
                f.height(e.height_m.map(|metres| Height {
                    datum: tide.datum,
                    metres,
                }))
            });
            ChartMarker {
                kind: match e.kind {
                    TideKind::High => MarkerKind::High,
                    TideKind::Low => MarkerKind::Low,
                },
                t: e.time.0,
                value: q.as_ref().and_then(|q| q.value),
                label: match &q {
                    Some(q) => format!("{word} {}", q.full()),
                    None => format!("{word} {}", f.clock(e.time)),
                },
            }
        })
        .collect();
    let summary = if markers.is_empty() {
        extremes(&series_points, f)
    } else {
        markers
            .iter()
            .map(|m| {
                f.strings.fmt(
                    Key::EventAt,
                    &[("event", &m.label), ("time", &f.clock(UnixTime(m.t)))],
                )
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    let (label, role) = if shown {
        (f.strings.get(Key::Predicted), SeriesRole::Predicted)
    } else {
        (f.strings.get(Key::ModelledSeaLevel), SeriesRole::Modelled)
    };
    let credit = ctx.f.strings.source_name(&tide.provenance.source);
    ctx.chart(
        name,
        &credit,
        if shown {
            ctx.f.strings.height_unit(ctx.input.units.height)
        } else {
            ""
        },
        vec![ChartSeries {
            label: label.to_owned(),
            role,
            style: SeriesStyle::Line,
            points: series_points,
        }],
        Vec::new(),
        markers,
        &summary,
    )
}

// ---------------------------------------------------------------------------------------------
// Current
// ---------------------------------------------------------------------------------------------

fn current_lane(ctx: &Ctx) -> ChartVm {
    let name = ctx.f.strings.card_title(CardId::Current);
    let Some(series) = ctx.input.snapshot.marine.tidal_current.as_ref() else {
        return ctx.empty(name);
    };
    let f = &ctx.f;
    let mut events = series.events.clone();
    events.sort_by_key(|e| e.time);
    let points = ctx.points(
        |t| speed_at(series, &events, UnixTime(t)),
        |v| f.current_speed(Some(v)),
    );
    let slacks: Vec<&CurrentEvent> = events
        .iter()
        .filter(|e| e.kind == CurrentKind::Slack)
        .collect();
    let markers: Vec<ChartMarker> = slacks
        .iter()
        .filter(|e| ctx.in_window(e.time.0))
        .map(|e| ChartMarker {
            kind: MarkerKind::Slack,
            t: e.time.0,
            value: None,
            label: format!("{} {}", f.phase(Phase::Slack), f.clock(e.time)),
        })
        .collect();
    let bands: Vec<ChartBand> = slacks
        .iter()
        .filter_map(|e| {
            let (a, b) = ctx.clip(
                e.time.0 - SLACK_HALF_WINDOW_S,
                e.time.0 + SLACK_HALF_WINDOW_S,
            )?;
            Some(ChartBand {
                kind: BandKind::Slack,
                start_t: a,
                end_t: b,
                label: f.phase(Phase::Slack).to_owned(),
            })
        })
        .collect();
    if points.is_empty() && markers.is_empty() {
        return ctx.empty(name);
    }
    let summary = if markers.is_empty() {
        extremes(&points, f)
    } else {
        markers
            .iter()
            .map(|m| m.label.clone())
            .collect::<Vec<_>>()
            .join(", ")
    };
    ctx.chart(
        name,
        &ctx.f.strings.source_name(&series.provenance.source),
        ctx.f.strings.speed_unit(ctx.input.units.current_speed),
        vec![ChartSeries {
            label: f.strings.get(Key::Predicted).to_owned(),
            role: SeriesRole::Predicted,
            style: SeriesStyle::Line,
            points,
        }],
        bands,
        markers,
        &summary,
    )
}

// ---------------------------------------------------------------------------------------------
// Wind and pressure
// ---------------------------------------------------------------------------------------------

/// Linear interpolation over (time, value) pairs sorted by time; none beyond the data or across
/// a gap wider than [`chart::MAX_GAP`].
fn interpolate(points: &[(i64, f32)], t: i64) -> Option<f32> {
    let i = points.partition_point(|(s, _)| *s <= t);
    let (t0, v0) = *points.get(i.checked_sub(1)?)?;
    if t0 == t {
        return Some(v0);
    }
    let (t1, v1) = *points.get(i)?;
    if t1 - t0 > chart::MAX_GAP.as_secs().try_into().unwrap_or(i64::MAX) {
        return None;
    }
    Some(v0 + (v1 - v0) * ((t - t0) as f32 / (t1 - t0) as f32))
}

/// (time, value) pairs sorted by time.
type Pairs = Vec<(i64, f32)>;

/// Where a lane's values come from.
struct Source {
    credit: String,
    role: SeriesRole,
}

fn wind_lane(ctx: &Ctx) -> ChartVm {
    let name = ctx.f.strings.card_title(CardId::Wind);
    let snapshot = &ctx.input.snapshot;
    let f = &ctx.f;
    let (source, speed, gust): (Source, Pairs, Pairs) = match &snapshot.weather {
        Some(w) if w.hourly.iter().any(|h| h.wind_speed.is_some()) => (
            Source {
                credit: w.provider.display_name().to_owned(),
                role: SeriesRole::Forecast,
            },
            w.hourly
                .iter()
                .filter_map(|h| Some((h.time.0, h.wind_speed?.0)))
                .collect(),
            w.hourly
                .iter()
                .filter_map(|h| Some((h.time.0, h.wind_gust?.0)))
                .collect(),
        ),
        _ => match &snapshot.marine.observed_wind {
            Some(obs) if !obs.is_empty() => (
                Source {
                    credit: ctx.f.strings.source_name(&obs.provenance().source),
                    role: SeriesRole::Observed,
                },
                obs.samples()
                    .iter()
                    .map(|s| (s.time.0, s.value.speed_ms))
                    .collect(),
                obs.samples()
                    .iter()
                    .filter_map(|s| Some((s.time.0, s.value.gust_ms?)))
                    .collect(),
            ),
            _ => return ctx.empty(name),
        },
    };
    let mut series = vec![ChartSeries {
        label: name.to_owned(),
        role: source.role,
        style: SeriesStyle::Line,
        points: ctx.points(|t| interpolate(&speed, t), |v| f.wind_speed(Some(v))),
    }];
    let gusts = ctx.points(|t| interpolate(&gust, t), |v| f.wind_speed(Some(v)));
    if !gusts.is_empty() {
        series.push(ChartSeries {
            label: f.strings.get(Key::Gust).to_owned(),
            role: source.role,
            style: SeriesStyle::Dots,
            points: gusts,
        });
    }
    if series[0].points.is_empty() {
        return ctx.empty(name);
    }
    let summary = extremes(&series[0].points, f);
    ctx.chart(
        name,
        &source.credit,
        ctx.f.strings.speed_unit(ctx.input.units.wind_speed),
        series,
        Vec::new(),
        Vec::new(),
        &summary,
    )
}

fn pressure_lane(ctx: &Ctx) -> ChartVm {
    let name = ctx.f.strings.card_title(CardId::Pressure);
    let snapshot = &ctx.input.snapshot;
    let f = &ctx.f;
    let (credit, role, values): (String, SeriesRole, Vec<(i64, f32)>) = match &snapshot.weather {
        Some(w) if w.hourly.iter().any(|h| h.pressure_msl.is_some()) => (
            w.provider.display_name().to_owned(),
            SeriesRole::Forecast,
            w.hourly
                .iter()
                .filter_map(|h| Some((h.time.0, h.pressure_msl?.0)))
                .collect(),
        ),
        _ => match &snapshot.marine.observed_pressure_hpa {
            Some(obs) if !obs.is_empty() => (
                ctx.f.strings.source_name(&obs.provenance().source),
                SeriesRole::Observed,
                obs.samples().iter().map(|s| (s.time.0, s.value)).collect(),
            ),
            _ => return ctx.empty(name),
        },
    };
    let points = ctx.points(|t| interpolate(&values, t), |v| f.pressure(Some(v)));
    if points.is_empty() {
        return ctx.empty(name);
    }
    let summary = extremes(&points, f);
    ctx.chart(
        name,
        &credit,
        ctx.f.strings.pressure_unit(ctx.input.units.pressure),
        vec![ChartSeries {
            label: name.to_owned(),
            role,
            style: SeriesStyle::Line,
            points,
        }],
        Vec::new(),
        Vec::new(),
        &summary,
    )
}

// ---------------------------------------------------------------------------------------------
// Light and solunar
// ---------------------------------------------------------------------------------------------

fn band(ctx: &Ctx, kind: BandKind, a: i64, b: i64, label: &str) -> Option<ChartBand> {
    let (start_t, end_t) = ctx.clip(a, b)?;
    Some(ChartBand {
        kind,
        start_t,
        end_t,
        label: label.to_owned(),
    })
}

/// Night from each civil dusk to the next civil dawn; a polar-night day is night all day.
fn night_spans(ctx: &Ctx, days: &[SkyDay], tz: &TimeZone) -> Vec<(i64, i64)> {
    let mut dawns: Vec<i64> = days
        .iter()
        .filter_map(|d| d.sun.civil_dawn)
        .map(|t| t.0)
        .collect();
    let mut dusks: Vec<i64> = days
        .iter()
        .filter_map(|d| d.sun.civil_dusk)
        .map(|t| t.0)
        .collect();
    dawns.sort_unstable();
    dusks.sort_unstable();
    let mut spans = Vec::new();
    if let (Some(&dawn), Some(&dusk)) = (dawns.first(), dusks.first())
        && dawn < dusk
    {
        spans.push((ctx.start, dawn));
    }
    for &dusk in &dusks {
        let dawn = dawns.iter().copied().find(|d| *d > dusk).unwrap_or(ctx.end);
        spans.push((dusk, dawn));
    }
    for d in days
        .iter()
        .filter(|d| d.sun.polar == Some(Polar::AlwaysDown))
    {
        if let Ok((a, b)) = local_date_bounds(d.date, tz) {
            spans.push((a.0, b.0));
        }
    }
    spans
}

fn light_lane(ctx: &Ctx, tz: &TimeZone) -> ChartVm {
    let name = ctx.f.strings.get(Key::LaneLight);
    let days = &ctx.input.snapshot.sky.days;
    let f = &ctx.f;
    let mut bands: Vec<ChartBand> = night_spans(ctx, days, tz)
        .into_iter()
        .filter_map(|(a, b)| band(ctx, BandKind::Night, a, b, f.strings.get(Key::Night)))
        .collect();
    for d in days {
        let s = &d.sun;
        let t = |v: Option<UnixTime>| v.map(|t| t.0);
        for (a, b, label) in [
            (t(s.civil_dawn), t(s.sunrise), Key::Twilight),
            (t(s.sunrise), t(s.golden_morning_end), Key::GoldenHour),
            (t(s.golden_evening_start), t(s.sunset), Key::GoldenHour),
            (t(s.sunset), t(s.civil_dusk), Key::Twilight),
        ] {
            if let (Some(a), Some(b)) = (a, b)
                && let Some(x) = band(ctx, BandKind::Highlight, a, b, f.strings.get(label))
            {
                bands.push(x);
            }
        }
    }
    bands.sort_by_key(|b| (b.start_t, b.end_t));
    let mut markers: Vec<ChartMarker> = days
        .iter()
        .flat_map(|d| {
            [
                (
                    MarkerKind::Sunrise,
                    f.strings.get(Key::Sunrise),
                    d.sun.sunrise,
                ),
                (MarkerKind::Sunset, f.strings.get(Key::Sunset), d.sun.sunset),
            ]
        })
        .filter_map(|(kind, word, t)| {
            let t = t?;
            ctx.in_window(t.0).then(|| ChartMarker {
                kind,
                t: t.0,
                value: None,
                label: format!("{word} {}", f.clock(t)),
            })
        })
        .collect();
    markers.sort_by_key(|m| m.t);
    let polar: Vec<&str> = days
        .iter()
        .filter_map(|d| d.sun.polar)
        .map(|p| {
            f.strings.get(match p {
                Polar::AlwaysUp => Key::SunStaysUp,
                Polar::AlwaysDown => Key::SunStaysDown,
            })
        })
        .collect();
    if bands.is_empty() && markers.is_empty() && polar.is_empty() {
        return ctx.empty(name);
    }
    let mut summary: Vec<String> = markers.iter().map(|m| m.label.clone()).collect();
    summary.extend(polar.iter().map(|p| (*p).to_owned()));
    ctx.chart(
        name,
        f.strings.get(Key::ComputedOnDeviceInline),
        "",
        Vec::new(),
        bands,
        markers,
        &summary.join(", "),
    )
}

fn solunar_lane(ctx: &Ctx) -> ChartVm {
    let f = &ctx.f;
    let name = f.strings.card_title(CardId::Solunar);
    // A period belongs to every day it overlaps: list each once.
    let mut periods: Vec<_> = ctx
        .input
        .snapshot
        .sky
        .days
        .iter()
        .flat_map(|d| d.solunar.iter().copied())
        .collect();
    periods.sort_by_key(|p| (p.start, p.end, p.kind as u8, p.cause as u8));
    periods.dedup_by_key(|p| (p.start, p.end, p.kind as u8, p.cause as u8));
    let bands: Vec<ChartBand> = periods
        .iter()
        .filter_map(|p| {
            let label = format!(
                "{} {}",
                kind_label(f, p.kind),
                range_text(f, p.start, p.end)
            );
            let kind = match p.kind {
                SolunarKind::Major => BandKind::SolunarMajor,
                SolunarKind::Minor => BandKind::SolunarMinor,
            };
            band(ctx, kind, p.start.0, p.end.0, &label)
        })
        .collect();
    if bands.is_empty() {
        return ctx.empty(name);
    }
    let summary = format!(
        "{}. {}",
        bands
            .iter()
            .map(|b| b.label.clone())
            .collect::<Vec<_>>()
            .join(", "),
        f.strings.safety(SafetyKey::TraditionalCue)
    );
    ctx.chart(
        name,
        f.strings.get(Key::ComputedOnDeviceInline),
        "",
        Vec::new(),
        bands,
        Vec::new(),
        &summary,
    )
}

// ---------------------------------------------------------------------------------------------
// Credits and notices
// ---------------------------------------------------------------------------------------------

/// True when the wind (and pressure) lane takes its values from the forecast.
fn forecast_has(ctx: &Ctx, has: impl Fn(&whether_core::model::Hourly) -> bool) -> bool {
    ctx.input
        .snapshot
        .weather
        .as_ref()
        .is_some_and(|w| w.hourly.iter().any(has))
}

/// The credit of a weather forecast provider.
fn weather_credit(weather: &whether_core::model::WeatherSnapshot) -> CreditVm {
    CreditVm {
        source: weather.provider.display_name().to_owned(),
        text: weather.provider.attribution().to_owned(),
        link: None,
        licence: None,
    }
}

/// The sources and notices of a lane that has something to show, as its card gives them.
fn lane_extras(ctx: &Ctx, kind: LaneKind) -> (Vec<CreditVm>, Vec<Notice>) {
    let snapshot = &ctx.input.snapshot;
    match kind {
        LaneKind::Tide => snapshot
            .marine
            .tide
            .as_ref()
            .map_or_else(Default::default, |t| {
                (
                    tide_credits(&ctx.f, snapshot, &t.provenance.source),
                    tide_notices(&ctx.f, t),
                )
            }),
        LaneKind::Current => {
            snapshot
                .marine
                .tidal_current
                .as_ref()
                .map_or_else(Default::default, |c| {
                    (
                        tide_credits(&ctx.f, snapshot, &c.provenance.source),
                        current_notices(&ctx.f, c),
                    )
                })
        }
        LaneKind::Wind => {
            let credit = match (&snapshot.weather, &snapshot.marine.observed_wind) {
                (Some(w), _) if forecast_has(ctx, |h| h.wind_speed.is_some()) => {
                    Some(weather_credit(w))
                }
                (_, Some(obs)) => station_credits(ctx.input, &[obs.provenance()]).pop(),
                _ => None,
            };
            (credit.into_iter().collect(), Vec::new())
        }
        LaneKind::Pressure => {
            let credit = match (&snapshot.weather, &snapshot.marine.observed_pressure_hpa) {
                (Some(w), _) if forecast_has(ctx, |h| h.pressure_msl.is_some()) => {
                    Some(weather_credit(w))
                }
                (_, Some(obs)) => station_credits(ctx.input, &[obs.provenance()]).pop(),
                _ => None,
            };
            (credit.into_iter().collect(), Vec::new())
        }
        LaneKind::Light | LaneKind::Solunar => (vec![computed_credit(ctx.f.strings)], Vec::new()),
    }
}

// ---------------------------------------------------------------------------------------------
// Build
// ---------------------------------------------------------------------------------------------

/// Builds the timeline for the spot's local day (or days) from midnight. A lane with no data is
/// present and empty; the solunar lane is left out when the user hides solunar. `None` when the
/// span is empty or no lane has anything to show.
pub fn build(input: &ViewInput) -> Option<TimelineVm> {
    let tz = zone(&input.snapshot.spot.time_zone).unwrap_or(TimeZone::UTC);
    let (start, end) = window(input, &tz)?;
    let step = i64::from(input.timeline_span.step_minutes.max(1)) * 60;
    let steps: Vec<i64> = std::iter::successors(Some(start), |t| Some(t + step))
        .take_while(|t| *t < end)
        .collect();
    let f = Formatter::for_input(input);
    let local = Zoned::from_zone(tz.clone());
    let ticks = chart::ticks(&local, &f, start, end - 1);
    let ctx = Ctx {
        input,
        f,
        local,
        start,
        end,
        steps,
        ticks,
    };
    let s = ctx.f.strings;
    let mut lanes = vec![
        (LaneKind::Tide, s.card_title(CardId::Tide), tide_lane(&ctx)),
        (
            LaneKind::Current,
            s.card_title(CardId::Current),
            current_lane(&ctx),
        ),
        (LaneKind::Wind, s.card_title(CardId::Wind), wind_lane(&ctx)),
        (
            LaneKind::Pressure,
            s.card_title(CardId::Pressure),
            pressure_lane(&ctx),
        ),
        (
            LaneKind::Light,
            s.get(Key::LaneLight),
            light_lane(&ctx, &tz),
        ),
    ];
    if input.solunar.show {
        lanes.push((
            LaneKind::Solunar,
            s.card_title(CardId::Solunar),
            solunar_lane(&ctx),
        ));
    }
    let any = lanes
        .iter()
        .any(|(_, _, c)| !(c.series.is_empty() && c.bands.is_empty() && c.markers.is_empty()));
    any.then(|| TimelineVm {
        start_t: start,
        end_t: end,
        step_minutes: input.timeline_span.step_minutes.max(1),
        x_ticks: ctx.ticks.clone(),
        lanes: lanes
            .into_iter()
            .map(|(kind, label, chart)| {
                let empty =
                    chart.series.is_empty() && chart.bands.is_empty() && chart.markers.is_empty();
                let (credits, notices) = if empty {
                    (Vec::new(), Vec::new())
                } else {
                    lane_extras(&ctx, kind)
                };
                TimelineLane {
                    kind,
                    label: label.to_owned(),
                    chart,
                    credits,
                    notices,
                }
            })
            .collect(),
    })
}

#[cfg(test)]
pub(in crate::view) mod tests {
    use super::*;
    use crate::astro::solunar::{DayBounds, SolunarSettings, sky_conditions};
    use crate::datum::VerticalDatum;
    use crate::model::{TideEvent, TideKind, TideSeries};
    use crate::provenance::{Provenance, Quality, ValueKind};
    use crate::series::{Sample, Series};
    use crate::time::{LocalDate, parse_coops, parse_rfc3339};
    use crate::view::cards::sun_moon::testing::battery_input;
    use crate::view::{TimelineSpan, Voice};
    use whether_core::presentation::TimeFormat;
    use whether_core::sun::{CIVIL, GOLDEN, SunDay, sun_crossing};

    fn fixture(name: &str, key: &str) -> Vec<(UnixTime, f32, String)> {
        let path = format!(
            "{}/../../tests/fixtures/raw/coops/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
        let doc: serde_json::Value = serde_json::from_str(&text).unwrap();
        doc[key]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| {
                (
                    parse_coops(p["t"].as_str().unwrap()).unwrap(),
                    p["v"].as_str().unwrap().parse().unwrap(),
                    p["type"].as_str().unwrap_or_default().to_owned(),
                )
            })
            .collect()
    }

    /// The Battery on 2026-11-08 with the recorded predictions and the computed sky.
    pub(in crate::view) fn input() -> ViewInput {
        let mut input = battery_input();
        input.snapshot.weather = None;
        input.snapshot.marine = crate::model::MarineConditions::default();
        let sample = crate::model::snapshot::testdata::sample_marine()
            .tide
            .unwrap();
        let prov = Provenance {
            kind: ValueKind::Predicted,
            quality: Quality::Approved,
            observed_at: parse_rfc3339("2026-11-08T00:00:00Z").unwrap(),
            ..sample.provenance
        };
        let events = fixture("coops-8518750-hilo-2026-11-08.json", "predictions")
            .into_iter()
            .map(|(time, v, kind)| TideEvent {
                time,
                kind: if kind == "H" {
                    TideKind::High
                } else {
                    TideKind::Low
                },
                height_m: Some(v),
            })
            .collect();
        let samples = fixture(
            "coops-8518750-predictions-6min-2026-11-08.json",
            "predictions",
        )
        .into_iter()
        .map(|(t, v, _)| Sample::new(t, v))
        .collect();
        input.snapshot.marine.tide = Some(TideSeries {
            source_kind: TideSourceKind::HarmonicStation,
            datum: VerticalDatum::Mllw,
            events,
            curve: Some(Series::new(prov.clone(), samples)),
            observed: None,
            provenance: prov,
        });
        input
    }

    fn lane(vm: &TimelineVm, kind: LaneKind) -> &ChartVm {
        &vm.lanes.iter().find(|l| l.kind == kind).unwrap().chart
    }

    #[test]
    fn battery_day_has_24_hourly_steps_in_est() {
        let vm = build(&input()).unwrap();
        // Local midnight EST is 05:00 UTC.
        let midnight = parse_rfc3339("2026-11-08T05:00:00Z").unwrap().0;
        assert_eq!(vm.start_t, midnight);
        assert_eq!(vm.end_t, midnight + 24 * 3_600);
        assert_eq!(vm.step_minutes, 60);
        let tide = &lane(&vm, LaneKind::Tide).series[0].points;
        let times: Vec<i64> = tide.iter().map(|p| p.t).collect();
        let expected: Vec<i64> = (0..24).map(|i| midnight + i * 3_600).collect();
        assert_eq!(times, expected);
        assert!(tide.iter().all(|p| p.label.ends_with(" ft MLLW")));
        // The markers carry the datum too.
        let markers = &lane(&vm, LaneKind::Tide).markers;
        assert!(markers.iter().all(|m| m.label.ends_with(" ft MLLW")));
        assert_eq!(vm.x_ticks.len(), 24);
        assert_eq!(vm.x_ticks[0].label, "12 AM");
        assert_eq!(vm.x_ticks[15].label, "3 PM");
        assert!(vm.x_ticks[3].major && !vm.x_ticks[4].major);
        // The curve is sampled, never averaged: highs and lows are markers at their own times.
        let markers = &lane(&vm, LaneKind::Tide).markers;
        assert!(markers.iter().any(|m| m.kind == MarkerKind::High));
        assert!(markers.iter().all(|m| m.t >= vm.start_t && m.t < vm.end_t));
        assert!(markers.iter().any(|m| m.t % 3_600 != 0));
    }

    #[test]
    fn light_bands_match_the_sun_times() {
        let input = input();
        let vm = build(&input).unwrap();
        let date = LocalDate {
            year: 2026,
            month: 11,
            day: 8,
        };
        let SunDay::RiseSet {
            rise: dawn,
            set: dusk,
        } = sun_crossing(date, 40.7033, -74.0170, CIVIL)
        else {
            panic!("no twilight")
        };
        let SunDay::RiseSet {
            rise: golden_end, ..
        } = sun_crossing(date, 40.7033, -74.0170, GOLDEN)
        else {
            panic!("no golden hour")
        };
        let light = lane(&vm, LaneKind::Light);
        let nights: Vec<_> = light
            .bands
            .iter()
            .filter(|b| b.kind == BandKind::Night)
            .collect();
        assert_eq!(nights.len(), 2);
        assert_eq!((nights[0].start_t, nights[0].end_t), (vm.start_t, dawn.0));
        assert_eq!((nights[1].start_t, nights[1].end_t), (dusk.0, vm.end_t));
        let sunrise = input.snapshot.sky.days[0].sun.sunrise.unwrap().0;
        let morning_twilight = light.bands.iter().find(|b| b.label == "Twilight").unwrap();
        assert_eq!(
            (morning_twilight.start_t, morning_twilight.end_t),
            (dawn.0, sunrise)
        );
        let golden = light
            .bands
            .iter()
            .find(|b| b.label == "Golden hour")
            .unwrap();
        assert_eq!((golden.start_t, golden.end_t), (sunrise, golden_end.0));
        let kinds: Vec<_> = light.markers.iter().map(|m| m.kind).collect();
        assert_eq!(kinds, [MarkerKind::Sunrise, MarkerKind::Sunset]);
    }

    #[test]
    fn solunar_bands_come_from_the_moon_events_and_hide_with_the_setting() {
        let mut input = input();
        let vm = build(&input).unwrap();
        let bands = &lane(&vm, LaneKind::Solunar).bands;
        let majors = bands
            .iter()
            .filter(|b| b.kind == BandKind::SolunarMajor)
            .count();
        let minors = bands
            .iter()
            .filter(|b| b.kind == BandKind::SolunarMinor)
            .count();
        assert_eq!((majors, minors), (2, 2));
        let day = &input.snapshot.sky.days[0];
        let upper = day.moon.upper_transit.unwrap().0;
        let major = bands
            .iter()
            .find(|b| b.kind == BandKind::SolunarMajor)
            .unwrap();
        assert_eq!((major.start_t, major.end_t), (upper - 3_600, upper + 3_600));
        assert!(
            lane(&vm, LaneKind::Solunar)
                .accessibility
                .contains("traditional cue")
        );
        input.solunar.show = false;
        let hidden = build(&input).unwrap();
        assert!(hidden.lanes.iter().all(|l| l.kind != LaneKind::Solunar));
    }

    fn lane_of(vm: &TimelineVm, kind: LaneKind) -> &TimelineLane {
        vm.lanes.iter().find(|l| l.kind == kind).unwrap()
    }

    #[test]
    fn lanes_carry_their_credits_and_notices() {
        use crate::view::NoticeKind;
        let kinds = |l: &TimelineLane| l.notices.iter().map(|n| n.kind).collect::<Vec<_>>();
        let mut input = input();
        let vm = build(&input).unwrap();
        let tide = lane_of(&vm, LaneKind::Tide);
        assert_eq!(kinds(tide), [NoticeKind::NotForNavigation]);
        assert_ne!(tide.credits.len(), 0);
        // Lanes with nothing to show credit and warn of nothing.
        let current = lane_of(&vm, LaneKind::Current);
        assert!(current.credits.is_empty() && current.notices.is_empty());
        // Light is computed on the device and carries no notice.
        let light = lane_of(&vm, LaneKind::Light);
        assert_eq!(light.credits.len(), 1);
        assert_eq!(light.notices.len(), 0);

        // A subordinate station's tide is approximate.
        input.snapshot.marine.tide.as_mut().unwrap().source_kind =
            TideSourceKind::SubordinateStation;
        let vm = build(&input).unwrap();
        assert_eq!(
            kinds(lane_of(&vm, LaneKind::Tide)),
            [NoticeKind::NotForNavigation, NoticeKind::Approximate]
        );

        // A model-mean tide shows times only and carries the sea-level notice.
        let tide = input.snapshot.marine.tide.as_mut().unwrap();
        tide.source_kind = TideSourceKind::ModelMean;
        tide.datum = VerticalDatum::ModelMean;
        let vm = build(&input).unwrap();
        assert_eq!(
            kinds(lane_of(&vm, LaneKind::Tide)),
            [NoticeKind::NotForNavigation, NoticeKind::Modelled]
        );
    }

    #[test]
    fn lanes_without_data_are_present_and_empty() {
        let vm = build(&input()).unwrap();
        let kinds: Vec<_> = vm.lanes.iter().map(|l| l.kind).collect();
        assert_eq!(
            kinds,
            [
                LaneKind::Tide,
                LaneKind::Current,
                LaneKind::Wind,
                LaneKind::Pressure,
                LaneKind::Light,
                LaneKind::Solunar
            ]
        );
        let current = lane(&vm, LaneKind::Current);
        assert!(
            current.series.is_empty() && current.bands.is_empty() && current.markers.is_empty()
        );
        assert!(current.accessibility.starts_with("Current: "));
        assert_eq!(current.x_ticks.len(), 24);
    }

    #[test]
    fn wind_and_pressure_use_the_forecast_in_display_units() {
        let mut input = input();
        let weather = whether_core::testdata::sample_snapshot();
        let midnight = parse_rfc3339("2026-11-08T05:00:00Z").unwrap().0;
        let mut w = weather;
        for (h, i) in w.hourly.iter_mut().zip(0_i64..) {
            h.time = UnixTime(midnight + i * 3_600);
        }
        input.snapshot.weather = Some(w);
        let vm = build(&input).unwrap();
        let wind = lane(&vm, LaneKind::Wind);
        assert_eq!(wind.unit, "mph");
        assert_eq!(wind.series[0].points.len(), 24);
        assert!(wind.title.starts_with("Wind · "), "{}", wind.title);
        let pressure = lane(&vm, LaneKind::Pressure);
        assert_eq!(pressure.unit, "inHg");
        assert!(
            pressure.series[0]
                .points
                .iter()
                .all(|p| p.label.ends_with(" inHg"))
        );
    }

    fn dst_input(date: LocalDate, hours: u16) -> ViewInput {
        let mut input = battery_input();
        let tz = zone("America/New_York").unwrap();
        let (start, end) = local_date_bounds(date, &tz).unwrap();
        input.snapshot.sky = sky_conditions(
            40.7033,
            -74.0170,
            &[DayBounds { date, start, end }],
            SolunarSettings::default(),
        );
        input.now = UnixTime(start.0 + 3_600);
        input.timeline_span = TimelineSpan {
            hours,
            step_minutes: 60,
        };
        input
    }

    #[test]
    fn daylight_saving_days_have_23_or_25_hourly_steps() {
        let long = build(&dst_input(
            LocalDate {
                year: 2026,
                month: 11,
                day: 1,
            },
            24,
        ))
        .unwrap();
        assert_eq!(long.end_t - long.start_t, 25 * 3_600);
        assert_eq!(long.x_ticks.len(), 25);
        let short = build(&dst_input(
            LocalDate {
                year: 2026,
                month: 3,
                day: 8,
            },
            24,
        ))
        .unwrap();
        assert_eq!(short.end_t - short.start_t, 23 * 3_600);
        assert_eq!(short.x_ticks.len(), 23);
        // Two days on the 25-hour day end on the second local midnight.
        let two = build(&dst_input(
            LocalDate {
                year: 2026,
                month: 11,
                day: 1,
            },
            48,
        ))
        .unwrap();
        assert_eq!(two.end_t - two.start_t, 49 * 3_600);
    }

    #[test]
    fn a_polar_night_day_is_night_all_day() {
        let mut input = dst_input(
            LocalDate {
                year: 2026,
                month: 11,
                day: 8,
            },
            24,
        );
        input.snapshot.sky.days[0].sun = crate::model::SunTimes {
            polar: Some(Polar::AlwaysDown),
            ..crate::model::SunTimes::default()
        };
        let vm = build(&input).unwrap();
        let light = lane(&vm, LaneKind::Light);
        assert_eq!(light.bands.len(), 1);
        assert_eq!(
            (light.bands[0].start_t, light.bands[0].end_t),
            (vm.start_t, vm.end_t)
        );
        assert!(light.accessibility.contains("Sun stays down all day"));
    }

    #[test]
    fn nothing_to_show_is_no_timeline() {
        let mut input = battery_input();
        input.snapshot.sky.days.clear();
        input.snapshot.weather = None;
        input.snapshot.marine = crate::model::MarineConditions::default();
        assert!(build(&input).is_none());
    }

    #[test]
    fn snapshots_in_both_voices() {
        let mut input = input();
        insta::assert_json_snapshot!("timeline_luna", build(&input).unwrap());
        input.voice = Voice::Plain;
        input.clock = TimeFormat::H24;
        input.units.height = crate::units::HeightUnit::M;
        insta::assert_json_snapshot!("timeline_plain_24h_metric", build(&input).unwrap());
    }
}
