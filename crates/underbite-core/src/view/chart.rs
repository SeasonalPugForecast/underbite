//! Helpers the chart-drawing cards and the day timeline share: the local clock of a spot, time
//! grids and axis ticks, and the level or speed at an instant interpolated between events.

use std::time::Duration;

use jiff::Timestamp;
use jiff::tz::TimeZone;
use whether_core::time::LocalDateTime;

use crate::model::{CurrentEvent, CurrentSeries, TideEvent, TideSeries};
use crate::time::{UnixTime, UtcOffset, zone};
use crate::view::format::Formatter;
use crate::view::{ChartTick, ViewInput};

/// How much past a chart shows before now.
pub(super) const LOOKBACK_S: i64 = 3 * 3_600;
/// The widest gap a curve is interpolated across.
pub(super) const MAX_GAP: Duration = Duration::from_hours(3);

/// A spot's zone, for the hour-of-day questions the formatter does not answer.
pub(super) struct Zoned(TimeZone);

impl Zoned {
    pub(super) fn new(name: &str) -> Self {
        Self(zone(name).unwrap_or(TimeZone::UTC))
    }

    pub(super) const fn from_zone(zone: TimeZone) -> Self {
        Self(zone)
    }

    pub(super) fn local(&self, t: UnixTime) -> LocalDateTime {
        let offset = Timestamp::from_second(t.0).map_or(0, |ts| self.0.to_offset(ts).seconds());
        t.to_local(UtcOffset(offset))
    }

    pub(super) fn on_the_hour(&self, t: i64) -> bool {
        self.local(UnixTime(t)).minute == 0
    }
}

/// The span a chart covers: a little of the past and the requested hours ahead.
pub(super) fn window(input: &ViewInput) -> (i64, i64) {
    (
        input.now.0 - LOOKBACK_S,
        input.now.0 + i64::from(input.timeline_span.hours) * 3_600,
    )
}

/// Instants on a grid of `step_minutes` (aligned to UTC) from `start` to `end`.
pub(super) fn grid(start: i64, end: i64, step_minutes: u16) -> Vec<i64> {
    let step = i64::from(step_minutes.max(1)) * 60;
    let first = (start + step - 1).div_euclid(step) * step;
    // INVARIANT: the step is at least 60 and at most 65,535 minutes in seconds, so it fits usize.
    (first..=end)
        .step_by(usize::try_from(step).unwrap_or(60))
        .collect()
}

/// An axis tick on every local hour, major every third.
pub(super) fn ticks(z: &Zoned, f: &Formatter, start: i64, end: i64) -> Vec<ChartTick> {
    grid(start, end, 15)
        .into_iter()
        .filter(|t| z.on_the_hour(*t))
        .map(|t| ChartTick {
            t,
            label: f.hour_label(UnixTime(t)),
            major: z.local(UnixTime(t)).hour.is_multiple_of(3),
        })
        .collect()
}

fn cosine(a: f32, b: f32, fraction: f32) -> f32 {
    a + (b - a) * (1.0 - (std::f32::consts::PI * fraction).cos()) / 2.0
}

/// The events either side of `t` in time-sorted `events`, with how far along `t` is.
fn neighbours<E>(
    events: &[E],
    time: impl Fn(&E) -> UnixTime,
    t: UnixTime,
) -> Option<(&E, &E, f32)> {
    let before = events.iter().rfind(|e| time(e) <= t)?;
    let after = events.iter().find(|e| time(e) > t)?;
    let fraction = (t.0 - time(before).0) as f32 / (time(after).0 - time(before).0) as f32;
    Some((before, after, fraction))
}

/// The tide level at `t` in metres: the curve, else a cosine between the neighbouring events
/// (`events` sorted by time).
pub(super) fn level_at(tide: &TideSeries, events: &[TideEvent], t: UnixTime) -> Option<f32> {
    if let Some(v) = tide.curve.as_ref().and_then(|c| c.at(t, MAX_GAP)) {
        return Some(v);
    }
    let (before, after, fraction) = neighbours(events, |e| e.time, t)?;
    Some(cosine(before.height_m?, after.height_m?, fraction))
}

/// The signed current speed at `t`: the curve, else a cosine between the neighbouring events
/// (`events` sorted by time).
pub(super) fn speed_at(
    series: &CurrentSeries,
    events: &[CurrentEvent],
    t: UnixTime,
) -> Option<f32> {
    if let Some(v) = series.curve.as_ref().and_then(|c| c.at(t, MAX_GAP)) {
        return Some(v);
    }
    let (before, after, fraction) = neighbours(events, |e| e.time, t)?;
    Some(cosine(before.speed_ms, after.speed_ms, fraction))
}
