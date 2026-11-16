//! Drives one spot's requests across source chains, supplements and gap filling.
//!
//! Every wanted measurement has a [`Chain`] of [`Link`]s (a source and the station, point or
//! zone to ask), best first. The session plans the current link of every chain, merging links
//! that ask one source about one place, so a response that serves several measurements (one
//! USGS `latest-continuous`, one NDBC `.txt`) is sent and parsed once. The caller loops: take
//! [`Session::next_request`], perform it, report it with [`Session::on_response`] or
//! [`Session::on_transport_error`], and end with [`Session::finish`].
//!
//! A link that has no value for a measurement, or only values older than the chain's
//! `max_age_s`, hands that measurement to its next link and records why. A required request that
//! fails does the same for the measurements it serves, and only for those: the link's other
//! measurements keep their requests and the values already received. An optional request's
//! failure only explains a value the link then lacks; a follow-up is required only when its
//! source makes it so, whatever its parent was. Each measurement takes all
//! its values from one link: one series never mixes sources. Within a link, fragments of one
//! measurement merge by time (pages, chunks, an events response with a curve response).
//!
//! Metered requests spend from a [`SharedUsage`] ledger. A 429 stops the source's pool (every
//! Open-Meteo API shares one) for the rest of the fetch, and its `Retry-After` pauses the pool in
//! the ledger for later fetches too.

pub mod usage;

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;
use std::sync::Arc;

use underbite_core::datum::Heights;
use underbite_core::model::SourceError as ModelSourceError;
use underbite_core::model::{
    CurrentSeries, FloodCategories, MarineConditions, Measurement, Requirements, TideSeries,
    WaterConditions, WaveSample,
};
use underbite_core::provenance::{Attribution, Quality, SourceId, StationRef, ValueKind};
use underbite_core::series::Series;
use underbite_core::time::UnixTime;

use crate::request::{Place, Request, Resource, Response, Target, redact_url};
use crate::source::{FetchContext, Fragment, FragmentValue, Fragments, Source, SourceError};
use usage::{SharedUsage, pool};

/// One source asked about one place.
#[derive(Clone)]
pub struct Link {
    /// The source.
    pub source: Arc<dyn Source>,
    /// The station, point or zone, and the measurements wanted from it.
    pub target: Target,
}

impl fmt::Debug for Link {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Link")
            .field("source", &self.source.id())
            .field("target", &self.target)
            .finish()
    }
}

/// A measurement's links, best first, and the oldest value it accepts.
#[derive(Clone, Debug, Default)]
pub struct Chain {
    /// The links in order of preference.
    pub links: Vec<Link>,
    /// The oldest latest value accepted, seconds before `now`; `None` accepts any age.
    pub max_age_s: Option<i64>,
}

/// What became of one request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Sent and parsed.
    Ok,
    /// Sent; the source failed or its response did not parse.
    Failed(SourceError),
    /// Not sent: budget spent, source paused or stopped, or too many follow-ups.
    NotSent(SourceError),
    /// Not sent: a metered request while metered calls are off or the fetch is cancelled.
    Skipped,
}

/// One request the session handed out or held back, for logs and partial-failure displays.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attempt {
    /// The source.
    pub source: SourceId,
    /// The resource.
    pub resource: Resource,
    /// The URL with keys masked.
    pub url: String,
    /// The measurements the request served.
    pub measurements: BTreeSet<Measurement>,
    /// Whether the link needed it.
    pub required: bool,
    /// What happened.
    pub outcome: Outcome,
}

/// What a fetch produced, for the host to put into a `SpotSnapshot`.
#[derive(Clone, Debug, PartialEq)]
pub struct SessionOutcome {
    /// When the fetch ended.
    pub fetched_at: UnixTime,
    /// Water values.
    pub water: WaterConditions,
    /// Marine values.
    pub marine: MarineConditions,
    /// The source each measurement came from.
    pub sources: BTreeMap<Measurement, SourceId>,
    /// The stations values came from.
    pub stations: Vec<StationRef>,
    /// Credits of every source used.
    pub attribution: Vec<Attribution>,
    /// Why links were passed over, per measurement ("usgs: rate limited, used nwps").
    pub errors: Vec<ModelSourceError>,
    /// Every request, in the order handed out.
    pub attempts: Vec<Attempt>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    /// Its current link is to be planned.
    Waiting,
    /// Its current link runs in this group.
    Running(usize),
    Done,
}

#[derive(Debug)]
struct Track {
    chain: Chain,
    at: usize,
    state: State,
    failures: Vec<(SourceId, SourceError)>,
}

/// One source asked about one place for one or more measurements.
struct Group {
    source: Arc<dyn Source>,
    target: Target,
    pending: VecDeque<Request>,
    /// Whether a request went out or failed; others may join only before.
    sent: bool,
    open: bool,
    follow_ups: u32,
    fragments: Vec<Fragment>,
    credits: Vec<Attribution>,
    /// The first error of an optional request, per measurement it served: the reason given
    /// when the link ends up without a value for it.
    missed: BTreeMap<Measurement, SourceError>,
}

/// Where a value of a measurement goes; values of one slot merge.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Slot {
    Main,
    /// Observations attached to the main series (tide level, observed currents).
    Observed,
    Percentile,
    /// A flow forecast's stage.
    Stage,
}

/// The values a measurement took from its winning link.
#[derive(Debug)]
struct Won {
    source: SourceId,
    station: Option<StationRef>,
    credits: Vec<Attribution>,
    values: BTreeMap<Slot, FragmentValue>,
}

/// The fetch state machine for one spot's water and marine measurements.
pub struct Session {
    tracks: BTreeMap<Measurement, Track>,
    requirements: Requirements,
    ctx: FetchContext,
    groups: Vec<Group>,
    in_flight: Option<(usize, Request)>,
    usage: Option<SharedUsage>,
    metered: bool,
    /// Pools rate-limited during this fetch.
    stopped: BTreeMap<SourceId, SourceError>,
    won: BTreeMap<Measurement, Won>,
    attempts: Vec<Attempt>,
}

impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session")
            .field("tracks", &self.tracks)
            .field("groups", &self.groups.len())
            .field("attempts", &self.attempts)
            .finish_non_exhaustive()
    }
}

impl Session {
    /// A session over the chains of the measurements in `requirements`; other chains are
    /// ignored. Metered requests are skipped until a ledger is attached.
    pub fn new(
        chains: BTreeMap<Measurement, Chain>,
        requirements: Requirements,
        ctx: FetchContext,
    ) -> Self {
        let tracks = chains
            .into_iter()
            .filter(|(m, _)| requirements.measurements.contains(m))
            .map(|(m, chain)| {
                let track = Track {
                    chain,
                    at: 0,
                    state: State::Waiting,
                    failures: Vec::new(),
                };
                (m, track)
            })
            .collect();
        Self {
            tracks,
            requirements,
            ctx,
            groups: Vec::new(),
            in_flight: None,
            usage: None,
            metered: false,
            stopped: BTreeMap::new(),
            won: BTreeMap::new(),
            attempts: Vec::new(),
        }
    }

    /// Attaches the call counts and pauses (shared with spots fetched alongside) and allows
    /// metered requests. The host saves the ledger before sending each metered request.
    #[must_use]
    pub fn with_usage(mut self, usage: SharedUsage) -> Self {
        self.usage = Some(usage);
        self.metered = true;
        self
    }

    /// Whether metered requests may be sent (with a ledger attached).
    #[must_use]
    pub fn allow_metered(mut self, allowed: bool) -> Self {
        self.metered = allowed && self.usage.is_some();
        self
    }

    /// Skips every metered request still waiting; free ones still go out.
    pub fn cancel(&mut self) {
        self.metered = false;
    }

    /// The next request to perform, or `None` when the session is finished. Until the
    /// request is reported, the same request is returned again.
    pub fn next_request(&mut self) -> Option<&Request> {
        if self.in_flight.is_none() {
            self.dispatch();
        }
        self.in_flight.as_ref().map(|(_, request)| request)
    }

    /// Reports the response to the request last returned by [`Self::next_request`].
    pub fn on_response(&mut self, response: &Response<'_>) {
        let Some((g, request)) = self.in_flight.take() else {
            return;
        };
        let source = Arc::clone(&self.groups[g].source);
        let id = source.id();
        let now = self.ctx.now;
        if let Some(usage) = &self.usage {
            usage.with(|ledger| {
                if let (Some(budget), Some(left)) = (source.budget(), response.rate_limit_remaining)
                {
                    ledger.observe_remaining(&id, budget, left, now);
                }
                if let (429, Some(wait)) = (response.status, response.retry_after) {
                    ledger.pause_until(&id, UnixTime(now.0.saturating_add(i64::from(wait))));
                }
            });
        }
        let mut out = Fragments::new();
        let result = if response.status == 429 {
            Err(SourceError::RateLimited {
                retry_after_s: response.retry_after,
            })
        } else {
            source
                .parse(&request, response, &self.ctx, &mut out)
                .and_then(|()| source.follow_up(&request, response, &self.ctx))
        };
        match result {
            Ok(next) => {
                self.record(g, &request, Outcome::Ok);
                let (fragments, credits) = out.into_parts();
                let group = &mut self.groups[g];
                group.fragments.extend(fragments);
                for credit in credits {
                    if !group.credits.contains(&credit) {
                        group.credits.push(credit);
                    }
                }
                self.queue_follow_ups(g, next);
            }
            Err(error) => self.failed(g, &request, error),
        }
    }

    /// Reports that the request last returned by [`Self::next_request`] got no response.
    pub fn on_transport_error(&mut self, message: &str) {
        if let Some((g, request)) = self.in_flight.take() {
            self.failed(g, &request, SourceError::Transport(message.to_string()));
        }
    }

    /// The values, sources, credits and errors, in measurement order.
    pub fn finish(self, now: UnixTime) -> SessionOutcome {
        let mut out = SessionOutcome {
            fetched_at: now,
            water: WaterConditions::default(),
            marine: MarineConditions::default(),
            sources: BTreeMap::new(),
            stations: Vec::new(),
            attribution: Vec::new(),
            errors: Vec::new(),
            attempts: self.attempts,
        };
        for (m, won) in self.won {
            out.sources.insert(m, won.source);
            if let Some(station) = won.station
                && !out.stations.contains(&station)
            {
                out.stations.push(station);
            }
            for credit in won.credits {
                if !out.attribution.contains(&credit) {
                    out.attribution.push(credit);
                }
            }
            for value in won.values.into_values() {
                place(m, value, &mut out.water, &mut out.marine);
            }
        }
        for (m, track) in &self.tracks {
            for (source, error) in &track.failures {
                let mut e = error.to_model(source.clone(), Some(*m));
                e.message = match out.sources.get(m) {
                    Some(used) => format!("{source}: {}, used {used}", e.message),
                    None => format!("{source}: {}", e.message),
                };
                out.errors.push(e);
            }
        }
        out
    }

    /// Hands out the next sendable request into `in_flight`, planning and closing links as
    /// they come due.
    fn dispatch(&mut self) {
        loop {
            self.plan_waiting();
            let Some(g) = self
                .groups
                .iter()
                .position(|g| g.open && !g.pending.is_empty())
            else {
                let open: Vec<usize> = (0..self.groups.len())
                    .filter(|&g| self.groups[g].open)
                    .collect();
                if open.is_empty() {
                    return;
                }
                for g in open {
                    self.complete(g);
                }
                continue;
            };
            let Some(request) = self.groups[g].pending.pop_front() else {
                continue;
            };
            let source = Arc::clone(&self.groups[g].source);
            match self.admit(source.as_ref(), &request) {
                Ok(()) => {
                    self.groups[g].sent = true;
                    self.in_flight = Some((g, request));
                    return;
                }
                Err(outcome) => {
                    let error = match &outcome {
                        Outcome::NotSent(e) => Some(e.clone()),
                        _ => None,
                    };
                    self.record(g, &request, outcome);
                    if request.required {
                        self.fail(g, Some(&request.target.measurements), error.as_ref());
                    } else if let Some(error) = error {
                        self.missed(g, &request, &error);
                    }
                }
            }
        }
    }

    /// Puts every waiting measurement into a group for its current link: an unsent group of
    /// the same source and place, or a new one; then plans the groups that changed.
    fn plan_waiting(&mut self) {
        loop {
            let mut changed = BTreeSet::new();
            for (&m, track) in &mut self.tracks {
                if track.state != State::Waiting {
                    continue;
                }
                let Some(link) = track.chain.links.get(track.at) else {
                    track.state = State::Done;
                    continue;
                };
                let id = link.source.id();
                let joinable = self.groups.iter().position(|g| {
                    g.open && !g.sent && g.source.id() == id && same_place(&g.target, &link.target)
                });
                let g = joinable.unwrap_or_else(|| {
                    self.groups.push(Group {
                        source: Arc::clone(&link.source),
                        target: link.target.clone(),
                        pending: VecDeque::new(),
                        sent: false,
                        open: true,
                        follow_ups: 0,
                        fragments: Vec::new(),
                        credits: Vec::new(),
                        missed: BTreeMap::new(),
                    });
                    self.groups.len() - 1
                });
                let target = &mut self.groups[g].target;
                target.measurements.extend(link.target.measurements.iter());
                target.measurements.insert(m);
                track.state = State::Running(g);
                changed.insert(g);
            }
            if changed.is_empty() {
                return;
            }
            for g in changed {
                let group = &self.groups[g];
                match group
                    .source
                    .plan(&group.target, &self.requirements, &self.ctx)
                {
                    Ok(requests) => self.groups[g].pending = requests.into(),
                    Err(error) => self.fail(g, None, Some(&error)),
                }
            }
        }
    }

    /// Whether `request` may go out now, or what became of it instead.
    fn admit(&mut self, source: &dyn Source, request: &Request) -> Result<(), Outcome> {
        let id = source.id();
        if let Some(error) = self.stopped.get(&pool(&id)) {
            return Err(Outcome::NotSent(error.clone()));
        }
        let now = self.ctx.now;
        let paused = self
            .usage
            .as_ref()
            .and_then(|u| u.with(|ledger| ledger.paused_until(&id, now)));
        if let Some(until) = paused {
            return Err(Outcome::NotSent(SourceError::RateLimited {
                retry_after_s: u32::try_from(until.0 - now.0).ok(),
            }));
        }
        if !request.metered {
            return Ok(());
        }
        let (Some(usage), true) = (&self.usage, self.metered) else {
            return Err(Outcome::Skipped);
        };
        let Some(budget) = source.budget() else {
            return Err(Outcome::NotSent(SourceError::Unsupported(
                "a metered request without a budget".into(),
            )));
        };
        usage
            .with(|ledger| ledger.try_spend(&id, budget, now))
            .map_err(|quota| {
                Outcome::NotSent(SourceError::RateLimited {
                    retry_after_s: Some(quota.retry_after_s(now)),
                })
            })
    }

    /// Queues the follow-ups a response called for, right after it and within the source's
    /// cap. Each keeps the `required` its source gave it.
    fn queue_follow_ups(&mut self, g: usize, requests: Vec<Request>) {
        let cap = self.groups[g].source.max_follow_ups();
        let mut queued = Vec::new();
        for request in requests {
            if self.groups[g].follow_ups >= cap {
                let error = SourceError::Parse(format!("more than {cap} follow-ups"));
                self.record(g, &request, Outcome::NotSent(error));
                continue;
            }
            self.groups[g].follow_ups += 1;
            queued.push(request);
        }
        for request in queued.into_iter().rev() {
            self.groups[g].pending.push_front(request);
        }
    }

    /// A sent request failed: a rate limit stops its pool, and a required one fails the
    /// measurements it serves.
    fn failed(&mut self, g: usize, request: &Request, error: SourceError) {
        if matches!(error, SourceError::RateLimited { .. }) {
            let id = self.groups[g].source.id();
            self.stopped.insert(pool(&id), error.clone());
        }
        if request.required {
            self.fail(g, Some(&request.target.measurements), Some(&error));
        } else {
            self.missed(g, request, &error);
        }
        self.record(g, request, Outcome::Failed(error));
    }

    /// Keeps an optional request's error for the measurements it served.
    fn missed(&mut self, g: usize, request: &Request, error: &SourceError) {
        let group = &mut self.groups[g];
        let served = if request.target.measurements.is_empty() {
            &group.target.measurements
        } else {
            &request.target.measurements
        };
        for &m in served {
            group.missed.entry(m).or_insert_with(|| error.clone());
        }
    }

    /// Moves the link's measurements among `served` (all of them for `None` or an empty set)
    /// to their next links. The link keeps the requests that still serve a measurement left
    /// on it, and closes when none is left.
    fn fail(
        &mut self,
        g: usize,
        served: Option<&BTreeSet<Measurement>>,
        error: Option<&SourceError>,
    ) {
        let served = served.filter(|s| !s.is_empty());
        let id = self.groups[g].source.id();
        let mut left = BTreeSet::new();
        for (m, track) in &mut self.tracks {
            if track.state != State::Running(g) {
                continue;
            }
            if served.is_some_and(|s| !s.contains(m)) {
                left.insert(*m);
                continue;
            }
            if let Some(error) = error {
                track.failures.push((id.clone(), error.clone()));
            }
            track.at += 1;
            track.state = State::Waiting;
        }
        let group = &mut self.groups[g];
        if left.is_empty() {
            group.open = false;
            group.pending.clear();
        } else {
            // Its plan is settled: a measurement joining now would plan the failed request again.
            group.sent = true;
            (group.pending).retain(|r| {
                r.target.measurements.is_empty() || !r.target.measurements.is_disjoint(&left)
            });
        }
    }

    /// Closes a link whose requests all ran: each measurement with a usable value is done,
    /// the others move on.
    fn complete(&mut self, g: usize) {
        let group = &mut self.groups[g];
        group.open = false;
        let id = group.source.id();
        let station = match &group.target.place {
            Place::Station { station, .. } => Some(station.clone()),
            Place::Point(_) | Place::Zone(_) => None,
        };
        let own = group.source.attribution();
        let mut credits: Vec<Attribution> = group
            .credits
            .iter()
            .filter(|c| **c != own)
            .cloned()
            .collect();
        credits.insert(0, own);
        let fragments = std::mem::take(&mut group.fragments);
        let mut missed = std::mem::take(&mut group.missed);
        for (&m, track) in &mut self.tracks {
            if track.state != State::Running(g) {
                continue;
            }
            match accept(m, &fragments, self.ctx.now, track.chain.max_age_s) {
                Ok(values) => {
                    track.state = State::Done;
                    let won = Won {
                        source: id.clone(),
                        station: station.clone(),
                        credits: credits.clone(),
                        values,
                    };
                    self.won.insert(m, won);
                }
                Err(error) => {
                    // "No data" from a link whose optional request failed says why.
                    let error = match (error, missed.remove(&m)) {
                        (SourceError::NoData(None), Some(why)) => why,
                        (error, _) => error,
                    };
                    track.failures.push((id.clone(), error));
                    track.at += 1;
                    track.state = State::Waiting;
                }
            }
        }
    }

    fn record(&mut self, g: usize, request: &Request, outcome: Outcome) {
        self.attempts.push(Attempt {
            source: self.groups[g].source.id(),
            resource: request.resource,
            url: redact_url(&request.url),
            measurements: request.target.measurements.clone(),
            required: request.required,
            outcome,
        });
    }
}

/// Whether two targets ask about the same place, window and datum.
fn same_place(a: &Target, b: &Target) -> bool {
    a.place == b.place && a.window == b.window && a.datum == b.datum
}

/// `m`'s values among a link's fragments, merged per slot, without empty or stale ones; an
/// error when none is left.
fn accept(
    m: Measurement,
    fragments: &[Fragment],
    now: UnixTime,
    max_age_s: Option<i64>,
) -> Result<BTreeMap<Slot, FragmentValue>, SourceError> {
    let mut values: BTreeMap<Slot, FragmentValue> = BTreeMap::new();
    for f in fragments.iter().filter(|f| f.measurement == m) {
        if matches!(f.value, FragmentValue::Coverage { .. }) {
            continue;
        }
        let slot = slot(m, &f.value);
        let value = match values.remove(&slot) {
            Some(old) => merge(old, f.value.clone()),
            None => f.value.clone(),
        };
        values.insert(slot, value);
    }
    let mut stale: Option<UnixTime> = None;
    values.retain(|_, v| {
        if is_empty(v) {
            return false;
        }
        match (latest(v), max_age_s) {
            (Some(t), Some(max)) if now.0.saturating_sub(t.0) > max => {
                stale = Some(stale.map_or(t, |s| s.max(t)));
                false
            }
            _ => true,
        }
    });
    // Observations attach to a main series in their datum, never alone.
    let attaches = match (values.get(&Slot::Main), values.get(&Slot::Observed)) {
        (Some(FragmentValue::TideSeries(t)), Some(FragmentValue::Heights(h))) => t.datum == h.datum,
        (Some(_), _) => true,
        (None, _) => false,
    };
    if !attaches {
        values.remove(&Slot::Observed);
    }
    if values.is_empty() {
        return Err(
            stale.map_or(SourceError::NoData(None), |latest| SourceError::Stale {
                latest,
            }),
        );
    }
    Ok(values)
}

fn slot(m: Measurement, value: &FragmentValue) -> Slot {
    use FragmentValue as V;
    use Measurement as M;
    match (m, value) {
        (_, V::Percentile(_)) => Slot::Percentile,
        (M::Tide, V::Heights(_)) | (M::TidalCurrent, V::Vectors(_)) => Slot::Observed,
        (M::FlowForecast, V::Heights(_)) => Slot::Stage,
        _ => Slot::Main,
    }
}

/// Two values of one slot from one link as one: series by time, the newer sample winning a
/// shared time (a later page carries the earlier ones), except wave samples, which merge field
/// by field (a standard file and a spectral file describe one sea state); events from both; a
/// predicted curve over an estimated one. Heights in different datums never merge: the first stays.
fn merge(old: FragmentValue, new: FragmentValue) -> FragmentValue {
    use FragmentValue as V;
    match (old, new) {
        (V::Series(a), V::Series(b)) => V::Series(concat(a, b)),
        (V::Heights(a), V::Heights(b)) if a.datum == b.datum => V::Heights(Heights {
            datum: a.datum,
            series: concat(a.series, b.series),
        }),
        (V::Turbidity(a), V::Turbidity(b)) => V::Turbidity(concat(a, b)),
        (V::Vectors(a), V::Vectors(b)) => V::Vectors(concat(a, b)),
        (V::WaveSeries(a), V::WaveSeries(b)) => V::WaveSeries(concat_with(a, b, merge_waves)),
        (V::Wind(a), V::Wind(b)) => V::Wind(concat(a, b)),
        (V::TideSeries(a), V::TideSeries(b)) if a.datum == b.datum => {
            let mut events = [a.events, b.events].concat();
            events.sort_by_key(|e| e.time);
            events.dedup_by_key(|e| e.time);
            V::TideSeries(TideSeries {
                events,
                curve: merge_curve(a.curve, b.curve),
                observed: merge_option(a.observed, b.observed),
                ..a
            })
        }
        (V::CurrentSeries(a), V::CurrentSeries(b)) => {
            let mut events = [a.events, b.events].concat();
            events.sort_by_key(|e| e.time);
            events.dedup_by_key(|e| e.time);
            V::CurrentSeries(CurrentSeries {
                events,
                curve: merge_curve(a.curve, b.curve),
                observed: merge_option(a.observed, b.observed),
                flood_dir_deg: a.flood_dir_deg.or(b.flood_dir_deg),
                ebb_dir_deg: a.ebb_dir_deg.or(b.ebb_dir_deg),
                bin: a.bin.or(b.bin),
                depth_m: a.depth_m.or(b.depth_m),
                provenance: a.provenance,
            })
        }
        (V::Alerts(mut a), V::Alerts(b)) => {
            for alert in b {
                if !a.iter().any(|x| x.id == alert.id) {
                    a.push(alert);
                }
            }
            V::Alerts(a)
        }
        (old @ (V::Heights(_) | V::TideSeries(_)), _) => old,
        (_, new) => new,
    }
}

/// Both series by time, `b`'s sample winning a shared time, with the provenance of the one
/// that reaches later. A many-sample series replaces a single latest value, unless that value
/// is newer than all of the series (a history that ends before the latest reading).
fn concat<T: Clone>(a: Series<T>, b: Series<T>) -> Series<T> {
    concat_with(a, b, |_, new| new.clone())
}

/// [`concat`], with `join(a's value, b's value)` at a time both series share.
fn concat_with<T: Clone>(a: Series<T>, b: Series<T>, join: impl Fn(&T, &T) -> T) -> Series<T> {
    let last = |s: &Series<T>| s.latest().map(|x| x.time);
    match (a.samples().len(), b.samples().len()) {
        (1, n) if n > 1 && last(&a) <= last(&b) => return b,
        (n, 1) if n > 1 && last(&b) <= last(&a) => return a,
        _ => {}
    }
    let provenance = if last(&b) >= last(&a) {
        b.provenance().clone()
    } else {
        a.provenance().clone()
    };
    let mut samples = b.samples().to_vec();
    for sample in &mut samples {
        if let Ok(i) = (a.samples()).binary_search_by_key(&sample.time, |x| x.time) {
            let old = &a.samples()[i];
            sample.value = join(&old.value, &sample.value);
            sample.quality = sample.quality.or(old.quality);
        }
    }
    // `Series::new` keeps the first of a shared time: the joined one.
    samples.extend_from_slice(a.samples());
    Series::new(provenance, samples)
}

/// One sea state from two readings at one time: `new`'s fields where it has them, `old`'s
/// elsewhere (the standard file's dominant period with the spectral file's trains).
fn merge_waves(old: &WaveSample, new: &WaveSample) -> WaveSample {
    WaveSample {
        significant_height_m: new.significant_height_m.or(old.significant_height_m),
        dominant_period_s: new.dominant_period_s.or(old.dominant_period_s),
        mean_period_s: new.mean_period_s.or(old.mean_period_s),
        direction_from_deg: new.direction_from_deg.or(old.direction_from_deg),
        wind_waves: new.wind_waves.clone().or_else(|| old.wind_waves.clone()),
        swells: if new.swells.is_empty() {
            old.swells.clone()
        } else {
            new.swells.clone()
        },
        steepness: new.steepness.clone().or_else(|| old.steepness.clone()),
    }
}

fn merge_option<T: Clone>(a: Option<Series<T>>, b: Option<Series<T>>) -> Option<Series<T>> {
    match (a, b) {
        (Some(a), Some(b)) => Some(concat(a, b)),
        (a, b) => a.or(b),
    }
}

/// Curves merged by time; where a predicted curve covers a time, an estimated or computed one
/// (interpolated between events) gives way.
fn merge_curve(a: Option<Series<f32>>, b: Option<Series<f32>>) -> Option<Series<f32>> {
    let estimated = |s: &Series<f32>| {
        let p = s.provenance();
        p.kind == ValueKind::Computed || p.quality == Quality::Estimated
    };
    let (best, rest) = match (a, b) {
        (Some(a), Some(b)) if estimated(&a) && !estimated(&b) => (b, a),
        (Some(a), Some(b)) if estimated(&b) && !estimated(&a) => (a, b),
        (a, b) => return merge_option(a, b),
    };
    let (Some(from), Some(to)) = (best.samples().first(), best.latest()) else {
        return Some(rest);
    };
    let (from, to) = (from.time, to.time);
    let outside = rest
        .samples()
        .iter()
        .filter(|s| s.time < from || s.time > to);
    let samples = best.samples().iter().chain(outside).copied().collect();
    Some(Series::new(best.provenance().clone(), samples))
}

/// The time of a value's newest sample, issue or observation, if it has one.
fn latest(value: &FragmentValue) -> Option<UnixTime> {
    use FragmentValue as V;
    fn last<T>(s: &Series<T>) -> Option<UnixTime> {
        s.latest().map(|x| x.time)
    }
    match value {
        V::Series(s) => last(s),
        V::Heights(h) => last(&h.series),
        V::Turbidity(s) => last(s),
        V::Vectors(s) => last(s),
        V::WaveSeries(s) => last(s),
        V::Wind(s) => last(s),
        V::TideSeries(t) => (t.events.iter().map(|e| e.time))
            .chain(t.curve.as_ref().and_then(last))
            .chain(t.observed.as_ref().and_then(last))
            .max(),
        V::CurrentSeries(c) => (c.events.iter().map(|e| e.time))
            .chain(c.curve.as_ref().and_then(last))
            .chain(c.observed.as_ref().and_then(last))
            .max(),
        V::ZoneForecast(z) => Some(z.issued),
        V::Percentile(p) => p.provenance.as_ref().map(|p| p.observed_at),
        V::Alerts(_) | V::FloodCategories(_) | V::Coverage { .. } => None,
    }
}

/// Whether a value holds nothing to show. No alerts is news, not a gap.
fn is_empty(value: &FragmentValue) -> bool {
    use FragmentValue as V;
    match value {
        V::Series(s) => s.is_empty(),
        V::Heights(h) => h.series.is_empty(),
        V::Turbidity(s) => s.is_empty(),
        V::Vectors(s) => s.is_empty(),
        V::WaveSeries(s) => s.is_empty(),
        V::Wind(s) => s.is_empty(),
        V::TideSeries(t) => {
            t.events.is_empty()
                && t.curve.as_ref().is_none_or(Series::is_empty)
                && t.observed.as_ref().is_none_or(Series::is_empty)
        }
        V::CurrentSeries(c) => {
            c.events.is_empty()
                && c.curve.as_ref().is_none_or(Series::is_empty)
                && c.observed.as_ref().is_none_or(Series::is_empty)
        }
        V::ZoneForecast(z) => z.periods.is_empty(),
        // The gauge's provenance alone is not a threshold.
        V::FloodCategories(f) => {
            let FloodCategories {
                action,
                minor,
                moderate,
                major,
                provenance: _,
            } = f;
            [action, minor, moderate, major].iter().all(|h| h.is_none())
        }
        V::Alerts(_) | V::Percentile(_) | V::Coverage { .. } => false,
    }
}

/// Puts `m`'s value into its field. Main values come before the observations attached to them.
fn place(m: Measurement, value: FragmentValue, w: &mut WaterConditions, s: &mut MarineConditions) {
    use FragmentValue as V;
    use Measurement as M;
    match (m, value) {
        (M::WaterTemperature, V::Series(v)) => w.temperature_c = Some(v),
        (M::Discharge, V::Series(v)) => w.discharge_m3s = Some(v),
        (M::Discharge, V::Percentile(v)) => w.discharge_percentile_for_date = Some(v),
        (M::GageHeight, V::Heights(v)) => w.gage_height = Some(v),
        (M::LakeLevel, V::Heights(v)) => w.level = Some(v),
        (M::LakeLevel, V::Percentile(v)) => w.level_percentile_for_date = Some(v),
        (M::Turbidity, V::Turbidity(v)) => w.turbidity = Some(v),
        (M::DissolvedOxygen, V::Series(v)) => w.dissolved_oxygen_mg_l = Some(v),
        (M::Conductivity, V::Series(v)) => w.conductivity_us_cm = Some(v),
        (M::Salinity, V::Series(v)) => w.salinity_psu = Some(v),
        (M::Ph, V::Series(v)) => w.ph = Some(v),
        (M::Chlorophyll, V::Series(v)) => w.chlorophyll_ug_l = Some(v),
        (M::Clarity, V::Series(v)) => w.clarity_kd490_per_m = Some(v),
        (M::Cyanobacteria, V::Series(v)) => w.cyanobacteria_cells_ml = Some(v),
        (M::FlowForecast, V::Series(v)) => w.discharge_forecast_m3s = Some(v),
        (M::FlowForecast, V::Heights(v)) => w.stage_forecast = Some(v),
        (M::FloodCategories, V::FloodCategories(v)) => w.flood_categories = Some(v),
        (M::Tide, V::TideSeries(v)) => s.tide = Some(v),
        (M::Tide, V::Heights(v)) => {
            if let Some(tide) = &mut s.tide {
                tide.observed = Some(v.series);
            }
        }
        (M::TidalCurrent, V::CurrentSeries(v)) => s.tidal_current = Some(v),
        (M::TidalCurrent, V::Vectors(v)) => {
            if let Some(current) = &mut s.tidal_current {
                current.observed = Some(v);
            }
        }
        (M::OceanCurrent, V::Vectors(v)) => s.ocean_current = Some(v),
        (M::Waves, V::WaveSeries(v)) => s.waves = Some(v),
        (M::WavesForecast, V::WaveSeries(v)) => s.waves_forecast = Some(v),
        (M::SeaSurfaceTemp, V::Series(v)) => s.sea_surface_temp_c = Some(v),
        (M::SeaSurfaceTempForecast, V::Series(v)) => s.sea_surface_temp_forecast_c = Some(v),
        (M::MarineForecast, V::ZoneForecast(v)) => s.marine_forecast = Some(v),
        (M::MarineAlerts, V::Alerts(v)) => s.marine_alerts = v,
        (M::ObservedWind, V::Wind(v)) => s.observed_wind = Some(v),
        (M::ObservedPressure, V::Series(v)) => s.observed_pressure_hpa = Some(v),
        // Values that cannot stand for their measurement never get here (`Fragments::push`).
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::fake::{self, FakeRequest, FakeSource};
    use crate::source::{Budget, Period};
    use underbite_core::datum::VerticalDatum;
    use underbite_core::model::{SourceErrorKind, TideEvent, TideKind, TideSourceKind};
    use underbite_core::provenance::{Provenance, Relation};
    use underbite_core::series::Sample;
    use usage::UsageLedger;

    use Measurement::{
        Discharge, ObservedPressure, SeaSurfaceTemp, SeaSurfaceTempForecast, Tide, WaterTemperature,
    };

    /// 2026-11-15 12:00:00 UTC.
    const NOW: UnixTime = UnixTime(1_794_744_000);

    fn source(id: &str, measurements: &[Measurement], plan: &[FakeRequest]) -> FakeSource {
        (plan.iter().cloned()).fold(FakeSource::new(id, measurements), FakeSource::plans)
    }

    fn link(source: &Arc<FakeSource>, station: &str, measurements: &[Measurement]) -> Link {
        Link {
            source: Arc::clone(source) as Arc<dyn Source>,
            target: fake::target(station, measurements, NOW),
        }
    }

    fn chain(links: Vec<Link>) -> Chain {
        Chain {
            links,
            max_age_s: Some(3_600),
        }
    }

    fn session(chains: BTreeMap<Measurement, Chain>) -> Session {
        let requirements = Requirements {
            measurements: chains.keys().copied().collect(),
            ..Requirements::default()
        };
        Session::new(chains, requirements, fake::ctx(NOW))
    }

    /// `{"<measurement>":[[NOW - age_s, value]]}`.
    fn body(measurement: &str, age_s: i64, value: f32) -> String {
        format!(r#"{{"{measurement}":[[{},{value}]]}}"#, NOW.0 - age_s)
    }

    /// Runs `s`, answering each URL with `reply` (`None` is a transport error; a 429 asks to
    /// retry in an hour). Returns the outcome and the URLs sent.
    fn drive(
        mut s: Session,
        reply: impl Fn(&str) -> Option<(u16, String)>,
    ) -> (SessionOutcome, Vec<String>) {
        let mut urls = Vec::new();
        while let Some(request) = s.next_request() {
            let url = request.url.clone();
            match reply(&url) {
                Some((status, body)) => s.on_response(&Response {
                    retry_after: (status == 429).then_some(3_600),
                    ..Response::new(status, body.as_bytes())
                }),
                None => s.on_transport_error("connection reset"),
            }
            urls.push(url);
        }
        (s.finish(NOW), urls)
    }

    fn usgs_and_coops() -> (Arc<FakeSource>, Arc<FakeSource>) {
        let usgs = source(
            "usgs",
            &[WaterTemperature, Discharge],
            &[FakeRequest::required("https://usgs.test/latest")],
        );
        let coops = source(
            "coops",
            &[WaterTemperature, Tide],
            &[FakeRequest::required("https://coops.test/temp")],
        );
        (Arc::new(usgs), Arc::new(coops))
    }

    fn temperature_chain(usgs: &Arc<FakeSource>, coops: &Arc<FakeSource>) -> Chain {
        chain(vec![link(usgs, "01", &[]), link(coops, "8518750", &[])])
    }

    #[test]
    fn session_falls_back_when_a_required_request_fails() {
        let (usgs, coops) = usgs_and_coops();
        let chains = BTreeMap::from([(WaterTemperature, temperature_chain(&usgs, &coops))]);
        let (out, urls) = drive(session(chains), |url| {
            Some(if url.contains("usgs") {
                (500, String::new())
            } else {
                (200, body("water_temperature", 60, 12.5))
            })
        });
        assert_eq!(
            urls,
            ["https://usgs.test/latest", "https://coops.test/temp"]
        );
        assert_eq!(out.sources[&WaterTemperature], SourceId::COOPS);
        assert_eq!(out.water.temperature_c.unwrap().samples()[0].value, 12.5);
        assert_eq!(out.errors.len(), 1);
        assert_eq!(out.errors[0].message, "usgs: HTTP 500, used coops");
        assert_eq!(out.stations[0].id, "8518750");
        assert_eq!(out.attribution[0].credit, "Test data from coops");
    }

    /// A fake that plans one required request per measurement, at `<base>/<measurement>` and
    /// serving only that measurement, as CO-OPS asks for one product at a time.
    struct PerProduct {
        fake: FakeSource,
        base: &'static str,
    }

    impl Source for PerProduct {
        fn id(&self) -> SourceId {
            self.fake.id()
        }

        fn attribution(&self) -> Attribution {
            self.fake.attribution()
        }

        fn measurements(&self) -> &[Measurement] {
            self.fake.measurements()
        }

        fn plan(
            &self,
            target: &Target,
            _requirements: &Requirements,
            _ctx: &FetchContext,
        ) -> Result<Vec<Request>, SourceError> {
            let served = (target.measurements.iter()).filter(|m| self.measurements().contains(m));
            let requests = served.map(|&m| {
                let name = serde_json::to_value(m).unwrap();
                let url = format!("{}/{}", self.base, name.as_str().unwrap());
                let target = Target {
                    measurements: BTreeSet::from([m]),
                    ..target.clone()
                };
                Request::new(url, Resource::Fake, target)
            });
            Ok(requests.collect())
        }

        fn parse(
            &self,
            request: &Request,
            response: &Response<'_>,
            ctx: &FetchContext,
            out: &mut Fragments,
        ) -> Result<(), SourceError> {
            self.fake.parse(request, response, ctx, out)
        }
    }

    const NO_DATA: &str = r#"{"error":{"message":"No data was found. This product may not be offered at this station at the requested time."}}"#;

    #[test]
    fn session_fails_only_the_measurements_of_a_failed_request() {
        let coops = Arc::new(PerProduct {
            fake: FakeSource::new("coops", &[WaterTemperature, ObservedPressure]),
            base: "https://coops.test",
        });
        let coops_link = |m: Measurement| Link {
            source: Arc::clone(&coops) as Arc<dyn Source>,
            target: fake::target("8516945", &[m], NOW),
        };
        let ndbc = Arc::new(source(
            "ndbc",
            &[ObservedPressure],
            &[FakeRequest::required("https://ndbc.test/KPTN6.txt")],
        ));
        let pressure_links = vec![
            coops_link(ObservedPressure),
            link(&ndbc, "KPTN6", &[ObservedPressure]),
        ];
        let chains = BTreeMap::from([
            (WaterTemperature, chain(vec![coops_link(WaterTemperature)])),
            (ObservedPressure, chain(pressure_links)),
        ]);
        let (out, urls) = drive(session(chains), |url| {
            Some(match url.rsplit('/').next() {
                Some("water_temperature") => (200, body("water_temperature", 60, 11.5)),
                Some("observed_pressure") => (200, NO_DATA.to_string()),
                _ => (200, body("observed_pressure", 600, 1_013.0)),
            })
        });
        // Both products go to one link; the pressure's "no data" leaves the temperature on it.
        assert_eq!(
            urls,
            [
                "https://coops.test/water_temperature",
                "https://coops.test/observed_pressure",
                "https://ndbc.test/KPTN6.txt",
            ]
        );
        assert_eq!(out.sources[&WaterTemperature], SourceId::COOPS);
        assert_eq!(out.water.temperature_c.unwrap().samples()[0].value, 11.5);
        assert_eq!(out.sources[&ObservedPressure], SourceId::NDBC);
        let pressure = out.marine.observed_pressure_hpa.unwrap();
        assert_eq!(pressure.samples()[0].value, 1_013.0);
        assert_eq!(out.errors.len(), 1, "{:?}", out.errors);
        assert_eq!(out.errors[0].measurement, Some(ObservedPressure));
        assert_eq!(out.errors[0].kind, SourceErrorKind::NoData);
        assert!(
            out.errors[0].message.starts_with("coops: ")
                && out.errors[0].message.ends_with(", used ndbc"),
            "{}",
            out.errors[0].message
        );
        let stations: Vec<&str> = out.stations.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(stations, ["8516945", "KPTN6"]);
    }

    #[test]
    fn session_falls_back_for_every_measurement_a_failed_request_serves() {
        let (usgs, coops) = usgs_and_coops();
        let temperature_links = vec![
            link(&usgs, "01", &[WaterTemperature]),
            link(&coops, "8518750", &[WaterTemperature]),
        ];
        let chains = BTreeMap::from([
            (WaterTemperature, chain(temperature_links)),
            (Discharge, chain(vec![link(&usgs, "01", &[Discharge])])),
        ]);
        let (out, urls) = drive(session(chains), |url| {
            Some(if url.contains("usgs") {
                (503, String::new())
            } else {
                (200, body("water_temperature", 60, 12.0))
            })
        });
        assert_eq!(
            urls,
            ["https://usgs.test/latest", "https://coops.test/temp"]
        );
        assert_eq!(out.sources[&WaterTemperature], SourceId::COOPS);
        assert!(!out.sources.contains_key(&Discharge));
        let failed: Vec<_> = (out.errors.iter()).map(|e| e.measurement).collect();
        assert_eq!(failed, [Some(WaterTemperature), Some(Discharge)]);
        assert_eq!(out.attempts[0].measurements.len(), 2);
    }

    /// The Croton at New Croton Dam: the gage-height history (00065, series `1d9a…`) answers
    /// 404, an optional follow-up of the required latest values, so both measurements stay.
    #[test]
    fn session_keeps_a_link_whose_optional_follow_up_fails() {
        use crate::testutil::fixture_str;
        use underbite_core::model::Measurement::GageHeight;
        use underbite_core::provenance::{StationKind, UsgsSiteType};

        // 2026-11-08 14:30:00 UTC, when the Croton fixtures were recorded.
        let now = UnixTime(1_794_148_200);
        let station = StationRef {
            source: SourceId::USGS,
            id: "USGS-01375000".into(),
            name: "Croton River at New Croton Dam near Croton-on-Hudson NY".into(),
            kind: StationKind::UsgsSite(UsgsSiteType::Stream),
            latitude: 41.2247,
            longitude: -73.8592,
            bin: None,
            reference_id: None,
        };
        let target = Target {
            place: Place::Station {
                station,
                distance_m: Some(300.0),
                relation: Relation::SameWaterBody,
            },
            ..fake::target("01375000", &[Discharge, GageHeight], now)
        };
        let usgs: Arc<dyn Source> = Arc::new(crate::usgs::Usgs::new());
        let usgs_chain = || Chain {
            links: vec![Link {
                source: Arc::clone(&usgs),
                target: target.clone(),
            }],
            max_age_s: None,
        };
        let chains = BTreeMap::from([(Discharge, usgs_chain()), (GageHeight, usgs_chain())]);
        let requirements = Requirements {
            measurements: chains.keys().copied().collect(),
            ..Requirements::default()
        };
        let mut s =
            Session::new(chains, requirements, fake::ctx(now)).with_usage(SharedUsage::default());
        let latest = fixture_str("usgs", "usgs-latest-continuous-croton-01375000.json");
        let discharge = fixture_str(
            "usgs",
            "usgs-continuous-croton-01375000-00060-2026-11-01_2026-11-08.json",
        );
        let mut urls = Vec::new();
        while let Some(request) = s.next_request() {
            let url = request.url.clone();
            let (status, body) = if url.contains("/latest-continuous/") {
                (200, latest.as_str())
            } else if url.contains("1d9a520f9f5d468099e5726040bfe317") {
                (404, r#"{"code":"NotFound","description":"Not found"}"#)
            } else {
                (200, discharge.as_str())
            };
            s.on_response(&Response::new(status, body.as_bytes()));
            urls.push(url);
        }
        let out = s.finish(now);
        assert_eq!(urls.len(), 3, "{urls:#?}");
        assert!(
            (out.attempts.iter()).any(|a| !a.required && matches!(a.outcome, Outcome::Failed(_))),
            "{:#?}",
            out.attempts
        );
        assert_eq!(out.errors, Vec::<ModelSourceError>::new());
        assert_eq!(out.sources[&Discharge], SourceId::USGS);
        assert_eq!(out.sources[&GageHeight], SourceId::USGS);
        // The week's record ends at 8 November 00:00; the newest value (13:45, 12.4 ft³/s)
        // follows it.
        let q = out.water.discharge_m3s.unwrap();
        assert_eq!(q.samples().len(), 674);
        assert_eq!(q.latest().unwrap().time, UnixTime(1_794_145_500));
        let q = q.latest().unwrap().value;
        assert!((q - 0.351).abs() < 0.001, "{q}");
        let stage = out.water.gage_height.unwrap();
        let stage = stage.series.latest().unwrap().value;
        assert!((stage - 0.335).abs() < 0.001, "{stage}");
    }

    #[test]
    fn session_keeps_the_link_when_an_optional_request_fails() {
        let usgs = Arc::new(source(
            "usgs",
            &[WaterTemperature],
            &[
                FakeRequest::required("https://usgs.test/latest"),
                FakeRequest::optional("https://usgs.test/extra"),
            ],
        ));
        let chains = BTreeMap::from([(WaterTemperature, chain(vec![link(&usgs, "01", &[])]))]);
        let (out, urls) = drive(session(chains), |url| {
            (url.ends_with("latest")).then(|| (200, body("water_temperature", 60, 9.0)))
        });
        assert_eq!(urls.len(), 2);
        assert_eq!(out.sources[&WaterTemperature], SourceId::USGS);
        assert_eq!(out.errors, Vec::<ModelSourceError>::new());
        assert_eq!(
            out.attempts[1].outcome,
            Outcome::Failed(SourceError::Transport("connection reset".into()))
        );
    }

    #[test]
    fn session_sends_a_shared_url_once_and_parses_it_once() {
        let usgs = FakeSource::new("usgs", &[WaterTemperature, Discharge])
            .plans(FakeRequest::required("https://usgs.test/latest"))
            .follows(
                "https://usgs.test/latest",
                FakeRequest::required("https://usgs.test/page2").carrying(),
            );
        let usgs = Arc::new(usgs);
        let chains = BTreeMap::from([
            (
                WaterTemperature,
                chain(vec![link(&usgs, "01", &[WaterTemperature])]),
            ),
            (Discharge, chain(vec![link(&usgs, "01", &[Discharge])])),
        ]);
        let (out, urls) = drive(session(chains), |url| {
            let t = NOW.0 - if url.ends_with("page2") { 60 } else { 960 };
            let both = format!(r#"{{"water_temperature":[[{t},11.0]],"discharge":[[{t},3.5]]}}"#);
            Some((200, both))
        });
        assert_eq!(
            urls,
            ["https://usgs.test/latest", "https://usgs.test/page2"]
        );
        let both = BTreeSet::from([WaterTemperature, Discharge]);
        assert!(out.attempts.iter().all(|a| a.measurements == both));
        assert_eq!(out.water.temperature_c.unwrap().samples().len(), 2);
        assert_eq!(out.water.discharge_m3s.unwrap().samples().len(), 2);
        assert_eq!(out.stations.len(), 1);
    }

    #[test]
    fn session_holds_back_a_metered_request_beyond_the_budget() {
        let marine = FakeSource::new("open-meteo-marine", &[WaterTemperature])
            .plans(FakeRequest::required("https://marine.test/v1").metered())
            .with_budget(Budget {
                period: Period::Day,
                limit: 1,
            });
        let (marine, (_, coops)) = (Arc::new(marine), usgs_and_coops());
        let links = vec![link(&marine, "om", &[]), link(&coops, "8518750", &[])];
        let chains = BTreeMap::from([(WaterTemperature, chain(links))]);
        // Another Open-Meteo call (whether's) spent the day's one call.
        let mut ledger = UsageLedger::new();
        ledger.count(&usage::OPEN_METEO, Period::Day, NOW);
        let shared = SharedUsage::new(ledger);
        let s = session(chains).with_usage(shared.clone());
        let (out, urls) = drive(s, |_| Some((200, body("water_temperature", 60, 8.0))));
        assert_eq!(urls, ["https://coops.test/temp"]);
        assert_eq!(out.sources[&WaterTemperature], SourceId::COOPS);
        assert_eq!(out.errors[0].kind, SourceErrorKind::RateLimited);
        assert_eq!(
            out.errors[0].message,
            "open-meteo-marine: rate limited, used coops"
        );
        assert!(matches!(out.attempts[0].outcome, Outcome::NotSent(_)));
        assert_eq!(shared.snapshot().used(&SourceId::OPEN_METEO_MARINE, NOW), 1);
    }

    #[test]
    fn session_stops_and_pauses_a_pool_on_429() {
        let om = |id: &str, url: &str| {
            let measurements = [SeaSurfaceTemp, SeaSurfaceTempForecast];
            Arc::new(source(id, &measurements, &[FakeRequest::required(url)]))
        };
        let marine = om("open-meteo-marine", "https://marine.test/v1");
        let storm = om("open-meteo-storm", "https://storm.test/v1");
        let ndbc = Arc::new(source(
            "ndbc",
            &[SeaSurfaceTemp],
            &[FakeRequest::required("https://ndbc.test/44025.txt")],
        ));
        let sst = chain(vec![link(&storm, "b", &[]), link(&ndbc, "44025", &[])]);
        let chains = BTreeMap::from([
            (SeaSurfaceTemp, sst),
            (SeaSurfaceTempForecast, chain(vec![link(&marine, "a", &[])])),
        ]);
        let shared = SharedUsage::default();
        let s = session(chains).with_usage(shared.clone());
        let (out, urls) = drive(s, |url| {
            Some(if url.contains("ndbc") {
                (200, body("sea_surface_temp", 600, 13.0))
            } else {
                let limit = r#"{"reason":"Daily API request limit exceeded.","error":true}"#;
                (429, limit.to_string())
            })
        });
        // The storm request's 429 stops the marine request too, which shares its pool.
        assert_eq!(
            urls,
            ["https://storm.test/v1", "https://ndbc.test/44025.txt"]
        );
        assert_eq!(out.sources[&SeaSurfaceTemp], SourceId::NDBC);
        assert!(!out.sources.contains_key(&SeaSurfaceTempForecast));
        assert_eq!(out.errors.len(), 2);
        assert!(
            (out.errors.iter()).all(|e| e.kind == SourceErrorKind::RateLimited),
            "{:?}",
            out.errors
        );
        assert_eq!(
            shared
                .snapshot()
                .paused_until(&SourceId::OPEN_METEO_MARINE, NOW),
            Some(UnixTime(NOW.0 + 3_600))
        );
    }

    #[test]
    fn session_passes_over_a_stale_series() {
        let (usgs, coops) = usgs_and_coops();
        let chains = BTreeMap::from([(WaterTemperature, temperature_chain(&usgs, &coops))]);
        let (out, _) = drive(session(chains), |url| {
            let age = if url.contains("usgs") { 7_200 } else { 60 };
            Some((200, body("water_temperature", age, 10.0)))
        });
        assert_eq!(out.sources[&WaterTemperature], SourceId::COOPS);
        assert_eq!(out.errors[0].kind, SourceErrorKind::Stale);
        assert_eq!(out.errors[0].message, "usgs: no recent data, used coops");
    }

    #[test]
    fn session_refuses_follow_ups_beyond_the_cap() {
        let mut pages = FakeSource::new("usgs", &[WaterTemperature])
            .plans(FakeRequest::required("https://usgs.test/p0"));
        for i in 0..3 {
            let next = FakeRequest::required(&format!("https://usgs.test/p{}", i + 1));
            pages = pages.follows(&format!("https://usgs.test/p{i}"), next);
        }
        pages.max_follow_ups = 2;
        let links = vec![link(&Arc::new(pages), "01", &[])];
        let chains = BTreeMap::from([(WaterTemperature, chain(links))]);
        let (out, urls) = drive(session(chains), |_| {
            Some((200, body("water_temperature", 60, 7.0)))
        });
        assert_eq!(urls.len(), 3);
        assert_eq!(
            out.attempts[3].outcome,
            Outcome::NotSent(SourceError::Parse("more than 2 follow-ups".into()))
        );
        assert_eq!(out.sources[&WaterTemperature], SourceId::USGS);
    }

    #[test]
    fn session_skips_metered_requests_once_cancelled() {
        let usgs = FakeSource::new("usgs", &[WaterTemperature])
            .plans(FakeRequest::required("https://usgs.test/free"))
            .plans(FakeRequest::optional("https://usgs.test/paid").metered())
            .with_budget(Budget {
                period: Period::Hour,
                limit: 10,
            });
        let links = vec![link(&Arc::new(usgs), "01", &[])];
        let chains = BTreeMap::from([(WaterTemperature, chain(links))]);
        let shared = SharedUsage::default();
        let mut s = session(chains).with_usage(shared.clone());
        s.cancel();
        let (out, urls) = drive(s, |_| Some((200, body("water_temperature", 60, 7.0))));
        assert_eq!(urls, ["https://usgs.test/free"]);
        assert_eq!(out.attempts[1].outcome, Outcome::Skipped);
        assert_eq!(out.sources[&WaterTemperature], SourceId::USGS);
        assert_eq!(shared.snapshot(), UsageLedger::new());
    }

    /// A chain never borrows another chain's links: the Open-Meteo sea level is not a tide
    /// fallback unless discovery put it in the tide chain.
    #[test]
    fn session_never_invents_links() {
        let (_, coops) = usgs_and_coops();
        let marine = Arc::new(source(
            "open-meteo-marine",
            &[SeaSurfaceTemp, Tide],
            &[FakeRequest::required("https://marine.test/v1")],
        ));
        let chains = BTreeMap::from([
            (Tide, chain(vec![link(&coops, "8518750", &[])])),
            (SeaSurfaceTemp, chain(vec![link(&marine, "a", &[])])),
        ]);
        let (out, urls) = drive(session(chains), |url| {
            (url.contains("marine")).then(|| (200, body("sea_surface_temp", 60, 12.0)))
        });
        assert_eq!(urls, ["https://coops.test/temp", "https://marine.test/v1"]);
        assert!(out.marine.tide.is_none());
        assert_eq!(out.errors.len(), 1);
        assert_eq!(out.errors[0].measurement, Some(Tide));
    }

    #[test]
    fn session_finish_is_deterministic() {
        let run = |reversed: bool| {
            let (usgs, coops) = usgs_and_coops();
            let mut entries = vec![
                (WaterTemperature, temperature_chain(&usgs, &coops)),
                (Discharge, chain(vec![link(&usgs, "02", &[])])),
                (Tide, chain(vec![link(&coops, "8518750", &[])])),
            ];
            if reversed {
                entries.reverse();
            }
            let s = session(entries.into_iter().collect());
            let (out, _) = drive(s, |url| {
                let both = format!(r#"{{"water_temperature":[[{}, 12.5]],"tide":[]}}"#, NOW.0);
                (url.contains("coops")).then_some((200, both))
            });
            out
        };
        let out = run(false);
        assert_eq!(out, run(true));
        insta::assert_debug_snapshot!((&out.sources, &out.errors, &out.attempts));
    }

    fn provenance(kind: ValueKind, quality: Quality) -> Provenance {
        Provenance {
            source: SourceId::COOPS,
            station: None,
            distance_m: None,
            relation: Relation::SameWaterBody,
            observed_at: NOW,
            kind,
            quality,
        }
    }

    /// A series with a sample `minutes` after `NOW` for each entry.
    fn series(minutes: &[i64], kind: ValueKind, quality: Quality) -> Series<f32> {
        let samples = (minutes.iter())
            .map(|&m| Sample::new(UnixTime(NOW.0 + m * 60), m as f32))
            .collect();
        Series::new(provenance(kind, quality), samples)
    }

    fn minutes(s: &Series<f32>) -> Vec<i64> {
        s.samples()
            .iter()
            .map(|x| (x.time.0 - NOW.0) / 60)
            .collect()
    }

    #[test]
    fn session_merges_an_events_response_with_a_predicted_curve() {
        let tide = |events: Vec<TideEvent>, curve: Series<f32>| Fragment {
            measurement: Tide,
            value: FragmentValue::TideSeries(TideSeries {
                source_kind: TideSourceKind::HarmonicStation,
                datum: VerticalDatum::Mllw,
                events,
                curve: Some(curve),
                observed: None,
                provenance: provenance(ValueKind::Predicted, Quality::Provisional),
            }),
        };
        let high = TideEvent {
            time: UnixTime(NOW.0 + 3_600),
            kind: TideKind::High,
            height_m: Some(1.7),
        };
        let interpolated = series(&[0, 30, 60, 90], ValueKind::Computed, Quality::Estimated);
        let hilo = tide(vec![high], interpolated);
        let predicted = series(&[0, 6, 12, 60], ValueKind::Predicted, Quality::Provisional);
        let six_minute = tide(Vec::new(), predicted);
        let mut values = accept(Tide, &[hilo, six_minute], NOW, Some(3_600)).unwrap();
        assert_eq!(values.len(), 1);
        let Some(FragmentValue::TideSeries(merged)) = values.remove(&Slot::Main) else {
            panic!("one tide series expected");
        };
        assert_eq!(merged.events, [high]);
        let curve = merged.curve.unwrap();
        assert_eq!(minutes(&curve), [0, 6, 12, 60, 90]);
        assert_eq!(curve.provenance().kind, ValueKind::Predicted);
    }

    #[test]
    fn session_lets_pages_and_a_series_replace_a_latest_value() {
        let fragment = |m: &[i64]| Fragment {
            measurement: Discharge,
            value: FragmentValue::Series(series(m, ValueKind::Observed, Quality::Provisional)),
        };
        let merged = |fragments: &[Fragment]| {
            let mut values = accept(Discharge, fragments, NOW, None).unwrap();
            let Some(FragmentValue::Series(s)) = values.remove(&Slot::Main) else {
                panic!("a discharge series expected");
            };
            minutes(&s)
        };
        // A series reaching past the latest value replaces it.
        let covered = [fragment(&[-20]), fragment(&[-60, -45, -30, -15])];
        assert_eq!(merged(&covered), [-60, -45, -30, -15]);
        // A latest value newer than every page stays after them.
        let newer = [
            fragment(&[-5]),
            fragment(&[-60, -45]),
            fragment(&[-60, -45, -30, -15]),
        ];
        assert_eq!(merged(&newer), [-60, -45, -30, -15, -5]);
    }

    /// The buoy's standard file and spectral file report one sea state at 13:40Z: the merged
    /// sample keeps the standard file's dominant period and the spectral file's steepness.
    #[test]
    fn session_merges_wave_samples_of_one_time_field_by_field() {
        use crate::ndbc::Ndbc;
        use crate::request::Window;
        use crate::testutil::fixture_bytes;
        use underbite_core::provenance::StationKind;

        // 2026-11-08 14:00:00 UTC.
        let now = UnixTime(1_794_146_400);
        let target = Target {
            place: Place::Station {
                station: StationRef {
                    source: SourceId::NDBC,
                    id: "44025".into(),
                    name: "Long Island 30 NM south of Islip".into(),
                    kind: StationKind::Buoy,
                    latitude: 40.25,
                    longitude: -73.16,
                    bin: None,
                    reference_id: None,
                },
                distance_m: Some(60_000.0),
                relation: Relation::Nearby,
            },
            measurements: BTreeSet::from([Measurement::Waves]),
            window: Window {
                from: UnixTime(now.0 - 86_400),
                to: now,
            },
            datum: None,
        };
        let link = Link {
            source: Arc::new(Ndbc),
            target,
        };
        let chains = BTreeMap::from([(
            Measurement::Waves,
            Chain {
                links: vec![link],
                max_age_s: Some(3_600),
            },
        )]);
        let requirements = Requirements {
            measurements: BTreeSet::from([Measurement::Waves]),
            ..Requirements::default()
        };
        let mut s = Session::new(chains, requirements, fake::ctx(now));
        let mut urls = Vec::new();
        while let Some(request) = s.next_request() {
            let url = request.url.clone();
            let file = if request.resource == Resource::NdbcSpectralSummary {
                "ndbc-44025.spec"
            } else {
                "ndbc-44025.txt"
            };
            s.on_response(&Response::new(200, &fixture_bytes("ndbc", file)));
            urls.push(url);
        }
        assert_eq!(urls.len(), 2, "{urls:?}");
        let out = s.finish(now);
        let waves = out.marine.waves.unwrap();
        let newest = waves.latest().unwrap();
        assert_eq!(newest.time, UnixTime(now.0 - 20 * 60));
        assert_eq!(newest.value.significant_height_m, Some(1.8));
        assert_eq!(newest.value.dominant_period_s, Some(6.0));
        assert_eq!(newest.value.mean_period_s, Some(4.6));
        assert_eq!(newest.value.direction_from_deg, Some(281.0));
        assert_eq!(newest.value.steepness.as_deref(), Some("VERY_STEEP"));
        let periods = waves
            .samples()
            .iter()
            .filter(|x| x.value.dominant_period_s.is_some());
        assert_eq!(periods.count(), waves.samples().len());
    }

    #[test]
    fn flood_categories_without_thresholds_are_empty_whatever_their_provenance() {
        let gauge = FloodCategories {
            provenance: Some(provenance(ValueKind::Observed, Quality::Provisional)),
            ..FloodCategories::default()
        };
        assert!(is_empty(&FragmentValue::FloodCategories(gauge.clone())));
        let action = FloodCategories {
            action: Some(underbite_core::datum::Height {
                datum: VerticalDatum::Gauge,
                metres: 3.0,
            }),
            ..gauge
        };
        assert!(!is_empty(&FragmentValue::FloodCategories(action)));
    }

    #[test]
    fn wave_fields_from_a_later_reading_win_and_absent_ones_keep_the_earlier() {
        let old = WaveSample {
            significant_height_m: Some(1.0),
            dominant_period_s: Some(6.0),
            ..WaveSample::default()
        };
        let new = WaveSample {
            significant_height_m: Some(1.1),
            steepness: Some("STEEP".into()),
            ..WaveSample::default()
        };
        let merged = merge_waves(&old, &new);
        assert_eq!(merged.significant_height_m, Some(1.1));
        assert_eq!(merged.dominant_period_s, Some(6.0));
        assert_eq!(merged.steepness.as_deref(), Some("STEEP"));
    }
}
