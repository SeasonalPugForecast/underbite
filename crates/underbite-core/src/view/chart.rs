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

/// An axis tick on every local hour, major every third. The first tick of each local day
/// is major and names the weekday: "Sat" at midnight, "Sat 9 PM" where the chart starts
/// part-way through a day. A chart inside one local day has no day names.
pub(super) fn ticks(z: &Zoned, f: &Formatter, start: i64, end: i64) -> Vec<ChartTick> {
    let hours: Vec<i64> = grid(start, end, 15)
        .into_iter()
        .filter(|t| z.on_the_hour(*t))
        .collect();
    // A chart inside one local day needs no day names.
    let several_days = hours.first().map(|t| z.local(UnixTime(*t)).date)
        != hours.last().map(|t| z.local(UnixTime(*t)).date);
    let mut last_day = None;
    hours
        .into_iter()
        .map(|t| {
            let local = z.local(UnixTime(t));
            let first_of_day = several_days && last_day != Some(local.date);
            last_day = Some(local.date);
            if first_of_day {
                let day = f.strings.weekday(local.weekday);
                let label = if local.hour == 0 {
                    day.to_owned()
                } else {
                    format!("{day} {}", f.hour_label(UnixTime(t)))
                };
                return ChartTick {
                    t,
                    label,
                    major: true,
                };
            }
            ChartTick {
                t,
                label: f.hour_label(UnixTime(t)),
                major: local.hour.is_multiple_of(3),
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::tests::sample_input;

    #[test]
    fn the_first_tick_of_each_local_day_names_the_weekday_and_is_major() {
        let input = sample_input();
        let f = Formatter::for_input(&input);
        let z = Zoned::new(&input.snapshot.spot.time_zone);
        let (start, end) = (input.now.0, input.now.0 + 72 * 3_600);
        let ticks = ticks(&z, &f, start, end);
        let days = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
        let named: Vec<&ChartTick> = ticks
            .iter()
            .filter(|t| days.iter().any(|d| t.label.starts_with(d)))
            .collect();
        // The window's first part-day, then one per midnight.
        assert_eq!(named.len(), 4, "{ticks:?}");
        assert!(named.iter().all(|t| t.major));
        for t in &named[1..] {
            assert_eq!(z.local(UnixTime(t.t)).hour, 0);
            assert!(!t.label.contains(' '), "{}", t.label);
        }
        // The hours between carry the hour alone.
        assert!(ticks.iter().any(|t| t.label.ends_with(" AM") && !t.major));
    }
}
