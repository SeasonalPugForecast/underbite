//! Call budgets and rate-limit pauses per source, kept by the host between fetches.
//!
//! A metered request is sent only after [`UsageLedger::try_spend`] succeeds. A budget covers one
//! UTC hour, day or calendar month; calls are counted when sent and never refunded. Sources on
//! one service share a counter ([`pool`]): Open-Meteo limits calls per IP address across all of
//! its APIs. A source's `X-RateLimit-Remaining` can only lower what is left, and a `Retry-After`
//! pauses every request to the pool until then. The ledger cannot see calls other apps make from
//! the same address, so a 429 is always possible; the session handles it.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use serde::{Deserialize, Serialize};
use underbite_core::provenance::SourceId;
use underbite_core::time::UnixTime;
use whether_core::time::{civil_from_days, days_from_civil};

use crate::source::{Budget, Period};

const HOUR: i64 = 3_600;
const DAY: i64 = 86_400;

/// How long a pool is paused after a 429 that names no `Retry-After`, seconds. USGS answers
/// an anonymous caller that asks too much this way, with no `X-RateLimit-*` headers either;
/// ten minutes is long enough for its limiter to forget a burst and short enough that a
/// river spot is not left without data for the rest of the hour.
pub const DEFAULT_RATE_LIMIT_PAUSE_S: u32 = 600;

/// The counter every Open-Meteo API counts against.
pub const OPEN_METEO: SourceId = SourceId::from_static("open-meteo");

/// The counter `source` counts against: `open-meteo` for `open-meteo` and every
/// `open-meteo-…` source (Marine, storm requests, and whether's calls the host counts), the
/// source itself otherwise.
pub fn pool(source: &SourceId) -> SourceId {
    let id = source.as_str();
    if id == OPEN_METEO.as_str() || id.starts_with("open-meteo-") {
        OPEN_METEO
    } else {
        source.clone()
    }
}

/// Why a metered call was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum QuotaError {
    /// The period's budget is used up.
    #[error("the {} limit of {limit} calls is used up", adjective(*period))]
    Exhausted {
        /// The limit.
        limit: u32,
        /// The period the limit covers.
        period: Period,
        /// When the next period starts.
        resets: UnixTime,
    },
    /// The source asked for no requests until `until` (`Retry-After`).
    #[error("the source asked to wait")]
    Paused {
        /// When requests may resume.
        until: UnixTime,
    },
}

impl QuotaError {
    /// Seconds from `now` until a call may be made again.
    pub fn retry_after_s(&self, now: UnixTime) -> u32 {
        let (Self::Exhausted { resets: t, .. } | Self::Paused { until: t }) = *self;
        u32::try_from(t.0.saturating_sub(now.0).max(0)).unwrap_or(u32::MAX)
    }
}

fn adjective(period: Period) -> &'static str {
    match period {
        Period::Hour => "hourly",
        Period::Day => "daily",
        Period::Month => "monthly",
    }
}

/// [`Period`] as saved.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Span {
    Hour,
    Day,
    Month,
}

impl From<Period> for Span {
    fn from(period: Period) -> Self {
        match period {
            Period::Hour => Self::Hour,
            Period::Day => Self::Day,
            Period::Month => Self::Month,
        }
    }
}

/// The start of the period that contains `now`, and of the one after it, Unix seconds.
fn bounds(period: Period, now: UnixTime) -> (i64, i64) {
    match period {
        Period::Hour => {
            let start = now.0 - now.0.rem_euclid(HOUR);
            (start, start + HOUR)
        }
        Period::Day => {
            let start = now.0 - now.0.rem_euclid(DAY);
            (start, start + DAY)
        }
        Period::Month => {
            let (year, month, _) = civil_from_days(now.0.div_euclid(DAY));
            let (next_year, next_month) = if month == 12 {
                (year + 1, 1)
            } else {
                (year, month + 1)
            };
            (
                days_from_civil(year, month, 1) * DAY,
                days_from_civil(next_year, next_month, 1) * DAY,
            )
        }
    }
}

/// What one pool has used in its current period.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Counter {
    period: Span,
    /// Start of the period, Unix seconds.
    start: i64,
    used: u32,
}

/// What a response said about the rate limit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ResponseLimits {
    /// The HTTP status.
    pub status: u16,
    /// `Retry-After`, seconds.
    pub retry_after_s: Option<u32>,
    /// `X-RateLimit-Remaining`.
    pub remaining: Option<u32>,
}

/// Calls spent per pool, each counted in its own period, and pauses sources asked for.
///
/// Serialized as `{"counters":{"open-meteo":{"period":"day","start":1794009600,"used":3}},
/// "paused":{"usgs":1794013200}}`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageLedger {
    #[serde(default)]
    counters: BTreeMap<SourceId, Counter>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    paused: BTreeMap<SourceId, UnixTime>,
}

impl UsageLedger {
    /// An empty ledger.
    pub fn new() -> Self {
        Self::default()
    }

    /// Calls `source`'s pool has used in its current period. A clock that went back to an
    /// earlier period still sees the later period's count, so it cannot unlock a budget.
    pub fn used(&self, source: &SourceId, now: UnixTime) -> u32 {
        self.counters.get(&pool(source)).map_or(0, |c| {
            let period = match c.period {
                Span::Hour => Period::Hour,
                Span::Day => Period::Day,
                Span::Month => Period::Month,
            };
            if bounds(period, now).0 > c.start {
                0
            } else {
                c.used
            }
        })
    }

    /// Calls left under `budget` for `source`'s pool.
    pub fn remaining(&self, source: &SourceId, budget: Budget, now: UnixTime) -> u32 {
        budget.limit.saturating_sub(self.used(source, now))
    }

    /// The counter of `pool` for `period` at `now`: a new period or a changed period starts
    /// at zero.
    fn counter(&mut self, pool: SourceId, period: Period, now: UnixTime) -> &mut Counter {
        let start = bounds(period, now).0;
        let fresh = Counter {
            period: period.into(),
            start,
            used: 0,
        };
        let counter = self.counters.entry(pool).or_insert(fresh);
        if counter.period != fresh.period || start > counter.start {
            *counter = fresh;
        }
        counter
    }

    /// Counts one call for `source` if its pool is not paused and the call fits `budget`.
    pub fn try_spend(
        &mut self,
        source: &SourceId,
        budget: Budget,
        now: UnixTime,
    ) -> Result<(), QuotaError> {
        if let Some(until) = self.paused_until(source, now) {
            return Err(QuotaError::Paused { until });
        }
        let counter = self.counter(pool(source), budget.period, now);
        if counter.used >= budget.limit {
            return Err(QuotaError::Exhausted {
                limit: budget.limit,
                period: budget.period,
                resets: UnixTime(bounds(budget.period, now).1),
            });
        }
        counter.used += 1;
        Ok(())
    }

    /// Gives back one call counted by [`Self::try_spend`] when a cache answered the request
    /// after all. Nothing changes if the counter has moved on to a new period.
    pub fn refund(&mut self, source: &SourceId, now: UnixTime) {
        if let Some(counter) = self.counters.get_mut(&pool(source)) {
            let period = match counter.period {
                Span::Hour => Period::Hour,
                Span::Day => Period::Day,
                Span::Month => Period::Month,
            };
            if bounds(period, now).0 <= counter.start {
                counter.used = counter.used.saturating_sub(1);
            }
        }
    }

    /// Counts one call sent outside a session (whether's Open-Meteo requests), budget or not.
    pub fn count(&mut self, source: &SourceId, period: Period, now: UnixTime) {
        let counter = self.counter(pool(source), period, now);
        counter.used = counter.used.saturating_add(1);
    }

    /// Lowers what is left for `source`'s pool to the `remaining` calls a response reported.
    pub fn observe_remaining(
        &mut self,
        source: &SourceId,
        budget: Budget,
        remaining: u32,
        now: UnixTime,
    ) {
        let counter = self.counter(pool(source), budget.period, now);
        counter.used = counter.used.max(budget.limit.saturating_sub(remaining));
    }

    /// Pauses every request to `source`'s pool until `until`.
    pub fn pause_until(&mut self, source: &SourceId, until: UnixTime) {
        let at = self.paused.entry(pool(source)).or_insert(until);
        *at = (*at).max(until);
    }

    /// Records a 429 from `source`: its pool is paused for the response's `Retry-After`, or
    /// for [`DEFAULT_RATE_LIMIT_PAUSE_S`] when it names none.
    pub fn rate_limited(&mut self, source: &SourceId, retry_after_s: Option<u32>, now: UnixTime) {
        let wait = retry_after_s.unwrap_or(DEFAULT_RATE_LIMIT_PAUSE_S);
        self.pause_until(source, UnixTime(now.0.saturating_add(i64::from(wait))));
    }

    /// Takes in what a response said about the limit: the `remaining` calls it reported
    /// (under `budget`, when the source has one) and a 429, which pauses the pool.
    pub fn observe_response(
        &mut self,
        source: &SourceId,
        budget: Option<Budget>,
        response: ResponseLimits,
        now: UnixTime,
    ) {
        if let (Some(budget), Some(left)) = (budget, response.remaining) {
            self.observe_remaining(source, budget, left, now);
        }
        if response.status == 429 {
            self.rate_limited(source, response.retry_after_s, now);
        }
    }

    /// When `source`'s pool may be asked again, if it is paused at `now`.
    pub fn paused_until(&self, source: &SourceId, now: UnixTime) -> Option<UnixTime> {
        self.paused.get(&pool(source)).copied().filter(|&t| t > now)
    }
}

/// One ledger shared by the sessions of spots fetched side by side, so they count together.
#[derive(Clone, Debug, Default)]
pub struct SharedUsage(Arc<Mutex<UsageLedger>>);

impl SharedUsage {
    /// Shares `ledger`.
    pub fn new(ledger: UsageLedger) -> Self {
        Self(Arc::new(Mutex::new(ledger)))
    }

    /// Runs `f` on the ledger.
    pub fn with<R>(&self, f: impl FnOnce(&mut UsageLedger) -> R) -> R {
        // A panic while holding the lock cannot leave the counts half-written.
        let mut ledger = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        f(&mut ledger)
    }

    /// A copy of the ledger, for the host to save.
    pub fn snapshot(&self) -> UsageLedger {
        self.with(|ledger| ledger.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const USGS: SourceId = SourceId::USGS;
    /// 2026-11-15 12:00:00 UTC.
    const NOON: UnixTime = UnixTime(1_794_744_000);

    fn hourly(limit: u32) -> Budget {
        Budget {
            period: Period::Hour,
            limit,
        }
    }

    fn daily(limit: u32) -> Budget {
        Budget {
            period: Period::Day,
            limit,
        }
    }

    #[test]
    fn a_refund_gives_back_one_call_of_this_period_only() {
        let mut ledger = UsageLedger::new();
        ledger.try_spend(&USGS, hourly(2), NOON).unwrap();
        ledger.try_spend(&USGS, hourly(2), NOON).unwrap();
        ledger.refund(&USGS, NOON);
        assert_eq!(ledger.used(&USGS, NOON), 1);
        // Never below zero, and a new period is not touched.
        ledger.refund(&USGS, NOON);
        ledger.refund(&USGS, NOON);
        assert_eq!(ledger.used(&USGS, NOON), 0);
        ledger.try_spend(&USGS, hourly(2), NOON).unwrap();
        ledger.refund(&USGS, UnixTime(NOON.0 + HOUR));
        assert_eq!(ledger.used(&USGS, NOON), 1);
    }

    #[test]
    fn usage_spends_up_to_the_limit_then_resets_next_hour() {
        let mut ledger = UsageLedger::new();
        for _ in 0..2 {
            ledger.try_spend(&USGS, hourly(2), NOON).unwrap();
        }
        let refused = ledger.try_spend(&USGS, hourly(2), NOON).unwrap_err();
        assert_eq!(
            refused,
            QuotaError::Exhausted {
                limit: 2,
                period: Period::Hour,
                resets: UnixTime(NOON.0 + HOUR),
            }
        );
        assert_eq!(refused.retry_after_s(UnixTime(NOON.0 + 600)), 3_000);
        assert_eq!(
            refused.to_string(),
            "the hourly limit of 2 calls is used up"
        );
        assert_eq!(ledger.remaining(&USGS, hourly(2), NOON), 0);
        let later = UnixTime(NOON.0 + HOUR);
        assert_eq!(ledger.used(&USGS, later), 0);
        ledger.try_spend(&USGS, hourly(2), later).unwrap();
        // A clock set back to the earlier hour sees the later count.
        assert_eq!(ledger.used(&USGS, NOON), 1);
    }

    #[test]
    fn usage_months_follow_the_calendar() {
        let budget = Budget {
            period: Period::Month,
            limit: 1,
        };
        let mut ledger = UsageLedger::new();
        let december = UnixTime(days_from_civil(2026, 12, 31) * DAY + 3_600);
        ledger.try_spend(&USGS, budget, december).unwrap();
        let refused = ledger.try_spend(&USGS, budget, december).unwrap_err();
        let january = UnixTime(days_from_civil(2027, 1, 1) * DAY);
        assert_eq!(refused.retry_after_s(december), 82_800);
        assert_eq!(ledger.used(&USGS, january), 0);
        ledger.try_spend(&USGS, budget, january).unwrap();
    }

    #[test]
    fn usage_open_meteo_sources_share_one_counter() {
        let mut ledger = UsageLedger::new();
        let marine = SourceId::OPEN_METEO_MARINE;
        let storm = SourceId::new("open-meteo-storm");
        ledger.try_spend(&marine, daily(3), NOON).unwrap();
        ledger.try_spend(&storm, daily(3), NOON).unwrap();
        ledger.count(&OPEN_METEO, Period::Day, NOON);
        assert_eq!(ledger.used(&marine, NOON), 3);
        assert!(ledger.try_spend(&storm, daily(3), NOON).is_err());
        assert_eq!(ledger.used(&USGS, NOON), 0);
        assert_eq!(
            pool(&SourceId::new("open-meteorology")).as_str(),
            "open-meteorology"
        );
    }

    #[test]
    fn usage_reported_remaining_only_lowers_what_is_left() {
        let mut ledger = UsageLedger::new();
        ledger.try_spend(&USGS, daily(1_000), NOON).unwrap();
        ledger.observe_remaining(&USGS, daily(1_000), 998, NOON);
        assert_eq!(ledger.used(&USGS, NOON), 2);
        ledger.observe_remaining(&USGS, daily(1_000), 999, NOON);
        assert_eq!(ledger.used(&USGS, NOON), 2);
        ledger.observe_remaining(&USGS, daily(1_000), 0, NOON);
        assert!(ledger.try_spend(&USGS, daily(1_000), NOON).is_err());
    }

    #[test]
    fn a_429_without_retry_after_pauses_the_pool_by_default() {
        let mut ledger = UsageLedger::new();
        let limits = ResponseLimits {
            status: 429,
            ..ResponseLimits::default()
        };
        ledger.observe_response(&USGS, Some(hourly(1_000)), limits, NOON);
        let until = UnixTime(NOON.0 + i64::from(DEFAULT_RATE_LIMIT_PAUSE_S));
        assert_eq!(ledger.paused_until(&USGS, NOON), Some(until));
        assert_eq!(ledger.paused_until(&USGS, until), None);
        assert!(matches!(
            ledger.try_spend(&USGS, hourly(1_000), NOON),
            Err(QuotaError::Paused { .. })
        ));
    }

    #[test]
    fn a_429_with_retry_after_pauses_for_that_long_and_other_statuses_do_not_pause() {
        let mut ledger = UsageLedger::new();
        let ok = ResponseLimits {
            status: 200,
            retry_after_s: Some(900),
            remaining: Some(990),
        };
        ledger.observe_response(&USGS, Some(hourly(1_000)), ok, NOON);
        assert_eq!(ledger.paused_until(&USGS, NOON), None);
        assert_eq!(ledger.used(&USGS, NOON), 10);
        let limited = ResponseLimits {
            status: 429,
            retry_after_s: Some(900),
            remaining: None,
        };
        ledger.observe_response(&USGS, Some(hourly(1_000)), limited, NOON);
        assert_eq!(
            ledger.paused_until(&USGS, NOON),
            Some(UnixTime(NOON.0 + 900))
        );
    }

    #[test]
    fn usage_retry_after_pauses_the_pool() {
        let mut ledger = UsageLedger::new();
        let until = UnixTime(NOON.0 + 5_760);
        ledger.pause_until(&SourceId::new("open-meteo-storm"), until);
        ledger.pause_until(&OPEN_METEO, UnixTime(NOON.0 + 60));
        let marine = SourceId::OPEN_METEO_MARINE;
        assert_eq!(ledger.paused_until(&marine, NOON), Some(until));
        assert_eq!(
            ledger.try_spend(&marine, daily(10), NOON),
            Err(QuotaError::Paused { until })
        );
        assert_eq!(ledger.paused_until(&marine, until), None);
        ledger.try_spend(&marine, daily(10), until).unwrap();
        assert_eq!(ledger.paused_until(&USGS, NOON), None);
    }

    #[test]
    fn usage_round_trips_through_json() {
        let mut ledger = UsageLedger::new();
        ledger
            .try_spend(&SourceId::OPEN_METEO_MARINE, daily(10), NOON)
            .unwrap();
        ledger.pause_until(&USGS, UnixTime(NOON.0 + HOUR));
        let text = serde_json::to_string(&ledger).unwrap();
        assert_eq!(
            text,
            r#"{"counters":{"open-meteo":{"period":"day","start":1794700800,"used":1}},"paused":{"usgs":1794747600}}"#
        );
        assert_eq!(serde_json::from_str::<UsageLedger>(&text).unwrap(), ledger);
        assert_eq!(
            serde_json::from_str::<UsageLedger>("{}").unwrap(),
            UsageLedger::new()
        );
        let shared = SharedUsage::new(ledger.clone());
        shared.clone().with(|l| l.count(&USGS, Period::Hour, NOON));
        assert_eq!(shared.snapshot().used(&USGS, NOON), 1);
    }
}
