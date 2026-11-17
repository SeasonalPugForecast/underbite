//! What worked: filters over the user's own catches.
//!
//! A filter and counts, not a model. [`summarise`] selects the live entries that match a
//! [`Filter`] and tallies them along each condition the log records: tide direction, hours from the
//! last high or low, current set, water temperature, flow percentile, pressure trend, moon phase,
//! solunar period, light and hour of day. Every [`Histogram`] carries the number of catches in the
//! selection, how many of them had that condition and how many did not, so the view can say
//! "3 of 5" and never a bare percentage.
//!
//! Counts are of catches only. The log holds no record of the time spent fishing without a bite,
//! so a count says when fish were caught, not when fishing is best. [`MIN_CATCHES_FOR_PATTERN`]
//! is the size below which a histogram ([`Histogram::enough`]) is too thin to read a pattern from.
//!
//! Buckets use each catch's own local time (the zone stored with it). The core reads no clock and
//! nothing here is text: the view words it. Deleted entries (tombstones) are never read.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::LogBook;
use super::entry::{
    CatchConditions, CatchEntry, CurrentSet, FlowCondition, Light, PressureTrend, TideDirection,
    TurnKind,
};
use crate::model::{MoonPhase, PercentileClass, SolunarKind};
use crate::time::{UnixTime, zone};
use crate::trip::SpeciesId;

/// Fewer known catches than this in a histogram is too few to call a pattern.
pub const MIN_CATCHES_FOR_PATTERN: usize = 10;

/// Water-temperature bin width, in °C.
pub const TEMPERATURE_BIN_C: i32 = 2;

/// Longest tide interval counted by hour: later than this lands in the last bucket.
const MAX_TURN_HOURS: u8 = 6;

/// Metres per degree of latitude (a spherical Earth of mean radius 6 371 km).
const METRES_PER_DEGREE: f64 = 111_194.926_644_558_74;

/// Which catches to count. Every field that is set must match; an empty filter matches all.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Filter {
    /// The catalogue species.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub species: Option<SpeciesId>,
    /// A saved spot's id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spot_id: Option<String>,
    /// A place name, compared ignoring case and surrounding space.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub area: Option<String>,
    /// Caught at or after this instant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<UnixTime>,
    /// Caught before this instant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<UnixTime>,
    /// The method, compared ignoring case and surrounding space.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
}

fn same_text(a: &str, b: &str) -> bool {
    a.trim().to_lowercase() == b.trim().to_lowercase()
}

impl Filter {
    /// Whether `e` passes the filter.
    #[must_use]
    pub fn matches(&self, e: &CatchEntry) -> bool {
        self.species
            .as_ref()
            .is_none_or(|s| e.species.as_ref() == Some(s))
            && self
                .spot_id
                .as_ref()
                .is_none_or(|s| e.location.spot_id.as_ref() == Some(s))
            && self
                .area
                .as_ref()
                .is_none_or(|a| e.location.area.as_deref().is_some_and(|x| same_text(x, a)))
            && self.from.is_none_or(|t| e.caught_at.0 >= t.0)
            && self.until.is_none_or(|t| e.caught_at.0 < t.0)
            && self
                .method
                .as_ref()
                .is_none_or(|m| e.method.as_deref().is_some_and(|x| same_text(x, m)))
    }
}

/// One value of a condition and the catches that had it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bucket<K> {
    pub key: K,
    pub count: usize,
}

/// Catches counted along one condition.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Histogram<K> {
    /// All catches in the selection.
    pub total: usize,
    /// Catches in the selection with this condition recorded; the sum of the bucket counts.
    pub known: usize,
    /// Catches without it ("unknown"): `total - known`.
    pub unknown: usize,
    /// In the condition's natural order; closed domains list every value, even at zero.
    pub buckets: Vec<Bucket<K>>,
}

impl<K> Histogram<K> {
    /// Whether enough catches have the condition to read a pattern from it.
    #[must_use]
    pub fn enough(&self) -> bool {
        self.known >= MIN_CATCHES_FOR_PATTERN
    }
}

/// How the flow compared with the date's normal, in percentile bins.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlowBin {
    /// Below the 10th percentile.
    Under10,
    /// 10th up to the 25th.
    From10To25,
    /// 25th to the 75th.
    From25To75,
    /// Above the 75th up to the 90th.
    From75To90,
    /// Above the 90th.
    Over90,
}

/// Where the last tide turn put the catch: which turn and how many whole hours after it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HoursSinceTurn {
    pub turn: TurnKind,
    /// Whole hours since the turn; the last bucket also holds anything later.
    pub hours: u8,
}

/// A water-temperature bin of [`TEMPERATURE_BIN_C`] degrees.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TemperatureBin {
    /// The bin's lower edge in °C (inclusive); the upper edge is this plus the bin width.
    pub from_c: i32,
}

/// Where the catch fell among the solunar periods.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SolunarBin {
    Major,
    Minor,
    /// The sky was worked out and no period covered the catch.
    Outside,
}

/// What the filter selected, counted along each condition.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Insights {
    /// Live catches that passed the filter.
    pub total: usize,
    pub tide_direction: Histogram<TideDirection>,
    pub since_turn: Histogram<HoursSinceTurn>,
    pub current: Histogram<CurrentSet>,
    pub water_temperature: Histogram<TemperatureBin>,
    pub flow: Histogram<FlowBin>,
    pub pressure_trend: Histogram<PressureTrend>,
    pub moon_phase: Histogram<MoonPhase>,
    pub solunar: Histogram<SolunarBin>,
    pub light: Histogram<Light>,
    /// Local hour 0 to 23.
    pub hour_of_day: Histogram<u8>,
}

impl Insights {
    /// Whether the selection is big enough to compare anything in it.
    #[must_use]
    pub fn enough(&self) -> bool {
        self.total >= MIN_CATCHES_FOR_PATTERN
    }
}

/// Tallies `values` over a closed `domain`.
fn closed<K: Copy + PartialEq>(domain: &[K], values: &[Option<K>]) -> Histogram<K> {
    let mut buckets: Vec<Bucket<K>> = domain.iter().map(|&key| Bucket { key, count: 0 }).collect();
    let mut known = 0;
    for v in values.iter().flatten() {
        if let Some(b) = buckets.iter_mut().find(|b| b.key == *v) {
            b.count += 1;
            known += 1;
        }
    }
    Histogram {
        total: values.len(),
        known,
        unknown: values.len() - known,
        buckets,
    }
}

fn flow_bin(c: &FlowCondition) -> Option<FlowBin> {
    if let Some(p) = c.percentile.filter(|p| p.is_finite()) {
        return Some(if p < 10.0 {
            FlowBin::Under10
        } else if p < 25.0 {
            FlowBin::From10To25
        } else if p <= 75.0 {
            FlowBin::From25To75
        } else if p <= 90.0 {
            FlowBin::From75To90
        } else {
            FlowBin::Over90
        });
    }
    c.percentile_class.map(|k| match k {
        PercentileClass::MuchBelowNormal | PercentileClass::RecordLow => FlowBin::Under10,
        PercentileClass::BelowNormal => FlowBin::From10To25,
        PercentileClass::Normal => FlowBin::From25To75,
        PercentileClass::AboveNormal => FlowBin::From75To90,
        PercentileClass::MuchAboveNormal | PercentileClass::RecordHigh => FlowBin::Over90,
    })
}

fn local_hour(e: &CatchEntry) -> Option<u8> {
    let tz = zone(&e.time_zone).ok()?;
    let ts = jiff::Timestamp::from_second(e.caught_at.0).ok()?;
    u8::try_from(ts.to_zoned(tz).hour()).ok()
}

fn temperature_bin(c: f32) -> Option<TemperatureBin> {
    // Beyond this a reading is a fault, not a water temperature.
    if !c.is_finite() || !(-10.0..=60.0).contains(&c) {
        return None;
    }
    let width = TEMPERATURE_BIN_C as f32;
    Some(TemperatureBin {
        from_c: (c / width).floor() as i32 * TEMPERATURE_BIN_C,
    })
}

/// Water temperature is an open domain: the buckets run unbroken from the coldest to the warmest.
fn temperature_histogram(values: &[Option<TemperatureBin>]) -> Histogram<TemperatureBin> {
    let known: Vec<i32> = values.iter().flatten().map(|b| b.from_c).collect();
    let mut buckets = Vec::new();
    if let (Some(&lo), Some(&hi)) = (known.iter().min(), known.iter().max()) {
        let mut from_c = lo;
        while from_c <= hi {
            let count = known.iter().filter(|&&k| k == from_c).count();
            buckets.push(Bucket {
                key: TemperatureBin { from_c },
                count,
            });
            from_c += TEMPERATURE_BIN_C;
        }
    }
    Histogram {
        total: values.len(),
        known: known.len(),
        unknown: values.len() - known.len(),
        buckets,
    }
}

fn cond(e: &CatchEntry) -> Option<&CatchConditions> {
    e.conditions.as_ref()
}

/// Counts the live catches that pass `filter` along each recorded condition.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn summarise(book: &LogBook, filter: &Filter) -> Insights {
    let picked: Vec<&CatchEntry> = book.live().filter(|e| filter.matches(e)).collect();

    let tide: Vec<_> = picked
        .iter()
        .map(|e| cond(e).and_then(|c| c.tide.as_ref()).map(|t| &t.value))
        .collect();
    let direction: Vec<_> = tide.iter().map(|t| t.map(|t| t.direction)).collect();
    let since: Vec<_> = tide
        .iter()
        .map(|t| {
            t.and_then(|t| {
                let h = t.hours_since_previous;
                (h.is_finite() && h >= 0.0).then(|| HoursSinceTurn {
                    turn: t.previous,
                    hours: (h.floor() as u32).min(u32::from(MAX_TURN_HOURS)) as u8,
                })
            })
        })
        .collect();
    let since_domain: Vec<HoursSinceTurn> = [TurnKind::High, TurnKind::Low]
        .into_iter()
        .flat_map(|turn| (0..=MAX_TURN_HOURS).map(move |hours| HoursSinceTurn { turn, hours }))
        .collect();
    let current: Vec<_> = picked
        .iter()
        .map(|e| {
            cond(e)
                .and_then(|c| c.current.as_ref())
                .and_then(|c| c.value.set)
        })
        .collect();
    let temps: Vec<_> = picked
        .iter()
        .map(|e| {
            cond(e)
                .and_then(|c| c.water_temperature_c.as_ref())
                .and_then(|t| temperature_bin(t.value))
        })
        .collect();
    let flow: Vec<_> = picked
        .iter()
        .map(|e| {
            cond(e)
                .and_then(|c| c.flow.as_ref())
                .and_then(|f| flow_bin(&f.value))
        })
        .collect();
    let trend: Vec<_> = picked
        .iter()
        .map(|e| {
            cond(e)
                .and_then(|c| c.pressure_trend.as_ref())
                .map(|t| t.value)
        })
        .collect();
    let moon: Vec<_> = picked
        .iter()
        .map(|e| cond(e).and_then(|c| c.moon).map(|m| m.phase))
        .collect();
    let solunar: Vec<_> = picked
        .iter()
        .map(|e| {
            cond(e).and_then(|c| match c.solunar {
                Some(s) => Some(match s.kind {
                    SolunarKind::Major => SolunarBin::Major,
                    SolunarKind::Minor => SolunarBin::Minor,
                }),
                // The moon and the light come from the same sky day as the periods.
                None => (c.moon.is_some() || c.light.is_some()).then_some(SolunarBin::Outside),
            })
        })
        .collect();
    let light: Vec<_> = picked
        .iter()
        .map(|e| cond(e).and_then(|c| c.light))
        .collect();
    let hours: Vec<_> = picked.iter().map(|e| local_hour(e)).collect();
    let hour_domain: Vec<u8> = (0..24).collect();

    Insights {
        total: picked.len(),
        tide_direction: closed(&[TideDirection::Rising, TideDirection::Falling], &direction),
        since_turn: closed(&since_domain, &since),
        current: closed(
            &[CurrentSet::Flood, CurrentSet::Ebb, CurrentSet::Slack],
            &current,
        ),
        water_temperature: temperature_histogram(&temps),
        flow: closed(
            &[
                FlowBin::Under10,
                FlowBin::From10To25,
                FlowBin::From25To75,
                FlowBin::From75To90,
                FlowBin::Over90,
            ],
            &flow,
        ),
        pressure_trend: closed(
            &[
                PressureTrend::Falling,
                PressureTrend::Steady,
                PressureTrend::Rising,
            ],
            &trend,
        ),
        moon_phase: closed(
            &[
                MoonPhase::New,
                MoonPhase::WaxingCrescent,
                MoonPhase::FirstQuarter,
                MoonPhase::WaxingGibbous,
                MoonPhase::Full,
                MoonPhase::WaningGibbous,
                MoonPhase::LastQuarter,
                MoonPhase::WaningCrescent,
            ],
            &moon,
        ),
        solunar: closed(
            &[SolunarBin::Major, SolunarBin::Minor, SolunarBin::Outside],
            &solunar,
        ),
        light: closed(
            &[Light::Dawn, Light::Day, Light::Dusk, Light::Night],
            &light,
        ),
        hour_of_day: closed(&hour_domain, &hours),
    }
}

/// A square of the local heat map and the catches in it.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Cell {
    /// Row from the equator, counted in cells of `cell_m` metres north.
    pub row: i32,
    /// Column from the prime meridian, counted in cells that are `cell_m` metres wide at the
    /// row's latitude.
    pub col: i32,
    /// The cell's centre, degrees north.
    pub latitude: f64,
    /// The cell's centre, degrees east.
    pub longitude: f64,
    pub count: usize,
    /// All catches binned, so the view can say "3 of 5".
    pub total: usize,
}

/// Bins every live catch onto a grid of `cell_m`-metre squares for the local heat map.
///
/// The grid is fixed to the Earth, not to the data: rows are `cell_m` metres tall, and the width of
/// a row's cells in degrees of longitude is set by the latitude of that row, so a cell is the
/// same size in metres at 41 degrees as at the equator and a cell never moves when more catches
/// are logged. Cells come in row, then column order. A `cell_m` that is not a positive finite
/// number gives no cells, and a catch with a coordinate off the globe gets none.
#[must_use]
pub fn catch_density(book: &LogBook, cell_m: f64) -> Vec<Cell> {
    catch_density_for(book, &Filter::default(), cell_m)
}

/// [`catch_density`] over the catches that pass `filter`.
#[must_use]
pub fn catch_density_for(book: &LogBook, filter: &Filter, cell_m: f64) -> Vec<Cell> {
    if !(cell_m.is_finite() && cell_m > 0.0) {
        return Vec::new();
    }
    let dlat = cell_m / METRES_PER_DEGREE;
    let mut cells: BTreeMap<(i32, i32), usize> = BTreeMap::new();
    let mut total = 0;
    for e in book.live().filter(|e| filter.matches(e)) {
        let (lat, lon) = (e.location.latitude, e.location.longitude);
        if !(lat.is_finite() && lon.is_finite() && lat.abs() <= 90.0 && lon.abs() <= 180.0) {
            continue;
        }
        let Some((row, col)) = grid(lat, lon, dlat) else {
            continue;
        };
        *cells.entry((row, col)).or_default() += 1;
        total += 1;
    }
    cells
        .into_iter()
        .map(|((row, col), count)| Cell {
            row,
            col,
            latitude: (f64::from(row) + 0.5) * dlat,
            longitude: (f64::from(col) + 0.5) * dlon_for(row, dlat),
            count,
            total,
        })
        .collect()
}

/// A cell's width in degrees of longitude at the centre of `row`.
fn dlon_for(row: i32, dlat: f64) -> f64 {
    let lat = ((f64::from(row) + 0.5) * dlat).clamp(-89.9, 89.9);
    // The cosine is floored so a row at a pole still has a finite width.
    dlat / lat.to_radians().cos().max(0.01)
}

fn grid(lat: f64, lon: f64, dlat: f64) -> Option<(i32, i32)> {
    let row_f = (lat / dlat).floor();
    let row = i32::try_from(row_f as i64).ok()?;
    let col_f = (lon / dlon_for(row, dlat)).floor();
    let col = i32::try_from(col_f as i64).ok()?;
    Some((row, col))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log::entry::{
        CatchConditions, CatchLocation, CurrentCondition, MoonCondition, SolunarCondition, Sourced,
        TideCondition, WindCondition,
    };
    use crate::provenance::SourceId;

    // 2026-06-10 12:00 UTC, which is 08:00 in New York.
    const NOON_UTC: i64 = 1_781_092_800;

    fn sourced<T>(value: T) -> Sourced<T> {
        Sourced {
            value,
            source: SourceId::COOPS,
            station: None,
            station_name: None,
            age_s: 0,
        }
    }

    fn entry(id: &str, caught: i64) -> CatchEntry {
        CatchEntry::new(
            id,
            UnixTime(caught),
            "America/New_York",
            CatchLocation::new(40.7003, -74.0142),
            UnixTime(caught),
        )
    }

    fn with(mut e: CatchEntry, f: impl FnOnce(&mut CatchConditions)) -> CatchEntry {
        let mut c = CatchConditions {
            at: e.caught_at,
            ..CatchConditions::default()
        };
        f(&mut c);
        e.conditions = Some(c);
        e
    }

    fn book(entries: Vec<CatchEntry>) -> LogBook {
        let mut b = LogBook::new();
        for e in entries {
            b.upsert(e);
        }
        b
    }

    fn count<K: PartialEq + Copy>(h: &Histogram<K>, key: K) -> usize {
        h.buckets
            .iter()
            .find(|b| b.key == key)
            .map_or(0, |b| b.count)
    }

    fn tide(dir: TideDirection, prev: TurnKind, since: f32) -> TideCondition {
        TideCondition {
            direction: dir,
            previous: prev,
            hours_since_previous: since,
            next: if prev == TurnKind::High {
                TurnKind::Low
            } else {
                TurnKind::High
            },
            hours_to_next: 6.0 - since,
            height: None,
        }
    }

    #[test]
    fn empty_log_has_empty_counts() {
        let i = summarise(&LogBook::new(), &Filter::default());
        assert_eq!(i.total, 0);
        assert!(!i.enough());
        assert_eq!(i.tide_direction.total, 0);
        assert_eq!(i.tide_direction.known, 0);
        assert_eq!(i.water_temperature.buckets, vec![]);
        assert_eq!(catch_density(&LogBook::new(), 100.0), vec![]);
    }

    #[test]
    fn entries_without_conditions_are_unknown_everywhere_but_the_hour() {
        let b = book(vec![entry("a", NOON_UTC), entry("b", NOON_UTC + 60)]);
        let i = summarise(&b, &Filter::default());
        assert_eq!(i.total, 2);
        for (total, known, unknown) in [
            (
                i.tide_direction.total,
                i.tide_direction.known,
                i.tide_direction.unknown,
            ),
            (i.since_turn.total, i.since_turn.known, i.since_turn.unknown),
            (i.current.total, i.current.known, i.current.unknown),
            (
                i.water_temperature.total,
                i.water_temperature.known,
                i.water_temperature.unknown,
            ),
            (i.flow.total, i.flow.known, i.flow.unknown),
            (
                i.pressure_trend.total,
                i.pressure_trend.known,
                i.pressure_trend.unknown,
            ),
            (i.moon_phase.total, i.moon_phase.known, i.moon_phase.unknown),
            (i.solunar.total, i.solunar.known, i.solunar.unknown),
            (i.light.total, i.light.known, i.light.unknown),
        ] {
            assert_eq!((total, known, unknown), (2, 0, 2));
        }
        // The hour comes from the catch time, so it is always known.
        assert_eq!(i.hour_of_day.known, 2);
        assert_eq!(count(&i.hour_of_day, 8), 2);
    }

    #[test]
    fn tide_direction_and_hours_since_a_turn() {
        let b = book(vec![
            with(entry("a", NOON_UTC), |c| {
                c.tide = Some(sourced(tide(TideDirection::Rising, TurnKind::Low, 1.4)));
            }),
            with(entry("b", NOON_UTC + 1), |c| {
                c.tide = Some(sourced(tide(TideDirection::Rising, TurnKind::Low, 1.9)));
            }),
            with(entry("c", NOON_UTC + 2), |c| {
                c.tide = Some(sourced(tide(TideDirection::Falling, TurnKind::High, 9.0)));
            }),
            entry("d", NOON_UTC + 3),
        ]);
        let i = summarise(&b, &Filter::default());
        assert_eq!((i.tide_direction.total, i.tide_direction.known), (4, 3));
        assert_eq!(i.tide_direction.unknown, 1);
        assert_eq!(count(&i.tide_direction, TideDirection::Rising), 2);
        assert_eq!(count(&i.tide_direction, TideDirection::Falling), 1);
        let at = |turn, hours| count(&i.since_turn, HoursSinceTurn { turn, hours });
        assert_eq!(at(TurnKind::Low, 1), 2);
        assert_eq!(at(TurnKind::High, MAX_TURN_HOURS), 1);
        assert_eq!(i.since_turn.buckets.len(), 14);
        assert_eq!(
            i.since_turn.buckets.iter().map(|b| b.count).sum::<usize>(),
            i.since_turn.known
        );
    }

    #[test]
    fn temperature_bins_are_two_degrees_and_unbroken() {
        let temp = |id: &str, t: f32| {
            with(entry(id, NOON_UTC), |c| {
                c.water_temperature_c = Some(sourced(t));
            })
        };
        let b = book(vec![
            temp("a", 11.9),
            temp("b", 12.0),
            temp("c", 13.99),
            temp("d", 18.0),
            temp("e", -0.5),
            temp("f", f32::NAN),
            temp("g", 900.0),
        ]);
        let h = summarise(&b, &Filter::default()).water_temperature;
        assert_eq!((h.total, h.known, h.unknown), (7, 5, 2));
        let keys: Vec<i32> = h.buckets.iter().map(|b| b.key.from_c).collect();
        assert_eq!(keys, vec![-2, 0, 2, 4, 6, 8, 10, 12, 14, 16, 18]);
        assert_eq!(count(&h, TemperatureBin { from_c: 10 }), 1);
        assert_eq!(count(&h, TemperatureBin { from_c: 12 }), 2);
        assert_eq!(count(&h, TemperatureBin { from_c: 8 }), 0);
        assert_eq!(count(&h, TemperatureBin { from_c: -2 }), 1);
    }

    #[test]
    fn flow_bins_follow_the_percentile_then_the_class() {
        let flow = |id: &str, p: Option<f32>, k: Option<PercentileClass>| {
            with(entry(id, NOON_UTC), |c| {
                c.flow = Some(sourced(FlowCondition {
                    discharge_m3s: 4.0,
                    percentile: p,
                    percentile_class: k,
                }));
            })
        };
        let b = book(vec![
            flow("a", Some(9.9), None),
            flow("b", Some(10.0), None),
            flow("c", Some(24.9), None),
            flow("d", Some(25.0), None),
            flow("e", Some(75.0), None),
            flow("f", Some(75.1), None),
            flow("g", Some(90.0), None),
            flow("h", Some(90.1), None),
            flow("i", None, Some(PercentileClass::RecordLow)),
            flow("j", None, Some(PercentileClass::Normal)),
            flow("k", None, None),
            // The number wins over a class that disagrees with it.
            flow("l", Some(50.0), Some(PercentileClass::MuchAboveNormal)),
        ]);
        let h = summarise(&b, &Filter::default()).flow;
        assert_eq!((h.total, h.known, h.unknown), (12, 11, 1));
        assert_eq!(count(&h, FlowBin::Under10), 2);
        assert_eq!(count(&h, FlowBin::From10To25), 2);
        assert_eq!(count(&h, FlowBin::From25To75), 4);
        assert_eq!(count(&h, FlowBin::From75To90), 2);
        assert_eq!(count(&h, FlowBin::Over90), 1);
        assert!(h.enough());
    }

    #[test]
    fn moon_solunar_light_pressure_and_current() {
        let b = book(vec![
            with(entry("a", NOON_UTC), |c| {
                c.moon = Some(MoonCondition {
                    phase: MoonPhase::Full,
                    illuminated_fraction: 1.0,
                    elongation_deg: 180.0,
                });
                c.solunar = Some(SolunarCondition {
                    kind: SolunarKind::Major,
                    cause: crate::model::SolunarCause::UpperTransit,
                    start: UnixTime(NOON_UTC - 3_600),
                    end: UnixTime(NOON_UTC + 3_600),
                });
                c.light = Some(Light::Day);
                c.pressure_trend = Some(sourced(PressureTrend::Falling));
                c.current = Some(sourced(CurrentCondition {
                    speed_ms: 0.8,
                    set: Some(CurrentSet::Ebb),
                    toward_deg: None,
                }));
            }),
            with(entry("b", NOON_UTC + 1), |c| {
                c.moon = Some(MoonCondition {
                    phase: MoonPhase::New,
                    illuminated_fraction: 0.0,
                    elongation_deg: 0.0,
                });
                c.light = Some(Light::Dawn);
                // A current whose set could not be told is unknown, not slack.
                c.current = Some(sourced(CurrentCondition {
                    speed_ms: 0.8,
                    set: None,
                    toward_deg: None,
                }));
                c.wind = Some(sourced(WindCondition {
                    speed_ms: 3.0,
                    gust_ms: None,
                    from_deg: None,
                }));
            }),
            // Conditions without a sky day: the solunar period is unknown, not "outside".
            with(entry("c", NOON_UTC + 2), |_| {}),
        ]);
        let i = summarise(&b, &Filter::default());
        assert_eq!(count(&i.moon_phase, MoonPhase::Full), 1);
        assert_eq!(count(&i.moon_phase, MoonPhase::New), 1);
        assert_eq!(i.moon_phase.buckets.len(), 8);
        assert_eq!(i.moon_phase.unknown, 1);
        assert_eq!(count(&i.solunar, SolunarBin::Major), 1);
        assert_eq!(count(&i.solunar, SolunarBin::Outside), 1);
        assert_eq!(i.solunar.unknown, 1);
        assert_eq!(count(&i.light, Light::Dawn), 1);
        assert_eq!(count(&i.pressure_trend, PressureTrend::Falling), 1);
        assert_eq!((i.current.known, i.current.unknown), (1, 2));
        assert_eq!(count(&i.current, CurrentSet::Ebb), 1);
        assert_eq!(count(&i.current, CurrentSet::Slack), 0);
    }

    #[test]
    fn hour_of_day_is_local_to_each_catch() {
        let mut tokyo = entry("t", NOON_UTC);
        tokyo.time_zone = "Asia/Tokyo".into();
        let mut odd = entry("z", NOON_UTC);
        odd.time_zone = "Not/AZone".into();
        let b = book(vec![entry("a", NOON_UTC), tokyo, odd]);
        let h = summarise(&b, &Filter::default()).hour_of_day;
        assert_eq!(count(&h, 8), 1);
        assert_eq!(count(&h, 21), 1);
        assert_eq!((h.total, h.known, h.unknown), (3, 2, 1));
        assert_eq!(h.buckets.len(), 24);
    }

    #[test]
    fn filter_fields_all_apply() {
        let mut a = entry("a", NOON_UTC);
        a.species = Some(SpeciesId::new("morone-saxatilis").unwrap());
        a.location.spot_id = Some("battery".into());
        a.location.area = Some("Lower Hudson".into());
        a.method = Some("Jigging".into());
        let mut b2 = entry("b", NOON_UTC + 86_400);
        b2.species = Some(SpeciesId::new("pomatomus-saltatrix").unwrap());
        b2.method = Some("casting".into());
        let book = book(vec![a, b2, entry("c", NOON_UTC + 2 * 86_400)]);
        let n = |f: Filter| summarise(&book, &f).total;
        assert_eq!(n(Filter::default()), 3);
        assert_eq!(
            n(Filter {
                species: Some(SpeciesId::new("morone-saxatilis").unwrap()),
                ..Filter::default()
            }),
            1
        );
        assert_eq!(
            n(Filter {
                spot_id: Some("battery".into()),
                ..Filter::default()
            }),
            1
        );
        assert_eq!(
            n(Filter {
                area: Some("  lower hudson ".into()),
                ..Filter::default()
            }),
            1
        );
        assert_eq!(
            n(Filter {
                method: Some("JIGGING".into()),
                ..Filter::default()
            }),
            1
        );
        // From is inclusive and until is exclusive.
        assert_eq!(
            n(Filter {
                from: Some(UnixTime(NOON_UTC + 86_400)),
                until: Some(UnixTime(NOON_UTC + 2 * 86_400)),
                ..Filter::default()
            }),
            1
        );
        assert_eq!(
            n(Filter {
                from: Some(UnixTime(NOON_UTC)),
                until: Some(UnixTime(NOON_UTC)),
                ..Filter::default()
            }),
            0
        );
        // Every set field must match.
        assert_eq!(
            n(Filter {
                species: Some(SpeciesId::new("morone-saxatilis").unwrap()),
                method: Some("casting".into()),
                ..Filter::default()
            }),
            0
        );
    }

    #[test]
    fn tombstones_are_never_counted() {
        let mut b = book(vec![entry("a", NOON_UTC), entry("b", NOON_UTC + 1)]);
        b.delete("a", UnixTime(NOON_UTC + 10));
        assert_eq!(summarise(&b, &Filter::default()).total, 1);
        let cells = catch_density(&b, 100.0);
        assert_eq!(cells.iter().map(|c| c.count).sum::<usize>(), 1);
    }

    #[test]
    fn small_samples_are_flagged() {
        let few = book(
            (0..9)
                .map(|n| entry(&format!("e{n}"), NOON_UTC + n))
                .collect(),
        );
        let i = summarise(&few, &Filter::default());
        assert_eq!(i.total, 9);
        assert!(!i.enough());
        assert!(!i.hour_of_day.enough());
        let many = book(
            (0..10)
                .map(|n| entry(&format!("e{n}"), NOON_UTC + n))
                .collect(),
        );
        let i = summarise(&many, &Filter::default());
        assert!(i.enough() && i.hour_of_day.enough());
        // Ten catches, none with a recorded tide: enough catches, nothing to compare.
        assert!(!i.tide_direction.enough());
    }

    fn at(id: &str, lat: f64, lon: f64) -> CatchEntry {
        let mut e = entry(id, NOON_UTC);
        e.location = CatchLocation::new(lat, lon);
        e
    }

    #[test]
    fn density_bins_in_metres_not_degrees() {
        // 100 m is 0.0009 degrees of latitude; at 40.7 N a 100 m column is about 0.00119 degrees
        // of longitude, so these two points 80 m apart east-west share a cell...
        let b = book(vec![
            at("a", 40.70030, -74.01420),
            at("b", 40.70030, -74.01330),
            // ...and this one, 500 m north, does not.
            at("c", 40.70480, -74.01420),
        ]);
        let cells = catch_density(&b, 100.0);
        assert_eq!(cells.iter().map(|c| c.count).sum::<usize>(), 3);
        assert!(cells.iter().all(|c| c.total == 3));
        let two = cells.iter().filter(|c| c.count == 2).count();
        let one = cells.iter().filter(|c| c.count == 1).count();
        // Whether a and b share a cell depends on where the edge falls, but c is never with them.
        assert!(cells.len() == 2 || cells.len() == 3);
        assert!(one >= 1 && two <= 1);
        let rows: Vec<i32> = cells.iter().map(|c| c.row).collect();
        assert!(rows.windows(2).all(|w| w[0] <= w[1]));
    }

    #[test]
    fn density_cell_widths_match_in_metres_at_any_latitude() {
        for lat in [0.0, 41.0, 64.0] {
            let cell_m = 250.0;
            let dlat = cell_m / METRES_PER_DEGREE;
            let row = (lat / dlat).floor() as i32;
            let width_m = dlon_for(row, dlat) * METRES_PER_DEGREE * lat.to_radians().cos();
            assert!((width_m - cell_m).abs() < cell_m * 0.01, "{lat}: {width_m}");
        }
    }

    #[test]
    fn density_cell_contains_its_catches_and_is_stable() {
        let p = at("a", 40.7003, -74.0142);
        let one = catch_density(&book(vec![p.clone()]), 200.0);
        let more = catch_density(&book(vec![p, at("b", 41.5, -73.0)]), 200.0);
        let c = one[0];
        assert!(more.iter().any(|m| m.row == c.row && m.col == c.col));
        let dlat = 200.0 / METRES_PER_DEGREE;
        assert!((c.latitude - 40.7003).abs() <= dlat);
        assert!((c.longitude - -74.0142).abs() <= 0.003);
        assert_eq!(c.count, 1);
    }

    #[test]
    fn density_ignores_bad_input() {
        let b = book(vec![
            at("a", 40.7, -74.0),
            at("b", f64::NAN, 10.0),
            at("c", 95.0, 0.0),
        ]);
        for bad in [0.0, -5.0, f64::NAN, f64::INFINITY] {
            assert_eq!(catch_density(&b, bad), vec![]);
        }
        let cells = catch_density(&b, 100.0);
        assert_eq!(cells.len(), 1);
        assert_eq!(cells[0].total, 1);
        // Tiny cells overflow the grid index rather than panic.
        assert_eq!(catch_density(&b, 1e-12), vec![]);
        // The poles and the date line stay finite.
        let edge = book(vec![at("p", 90.0, 180.0), at("q", -90.0, -180.0)]);
        assert_eq!(catch_density(&edge, 1_000.0).len(), 2);
    }

    #[test]
    fn density_follows_the_filter() {
        let mut a = at("a", 40.7, -74.0);
        a.method = Some("jigging".into());
        let b = book(vec![a, at("b", 40.7, -74.0)]);
        let f = Filter {
            method: Some("jigging".into()),
            ..Filter::default()
        };
        let cells = catch_density_for(&b, &f, 100.0);
        assert_eq!(cells.len(), 1);
        assert_eq!((cells[0].count, cells[0].total), (1, 1));
    }

    #[test]
    fn insights_serialise_deterministically() {
        let b = book(vec![with(entry("a", NOON_UTC), |c| {
            c.light = Some(Light::Dusk);
        })]);
        let i = summarise(&b, &Filter::default());
        let json = serde_json::to_string(&i).unwrap();
        assert_eq!(
            json,
            serde_json::to_string(&summarise(&b, &Filter::default())).unwrap()
        );
        assert!(json.contains(r#""light":{"total":1,"known":1,"unknown":0"#));
    }
}
