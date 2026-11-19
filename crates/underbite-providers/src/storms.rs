//! The storm-ahead check and nearby hurricanes, through whether's storm and NHC requests.
//!
//! [`StormWatchPlan`] is sans-IO, like the fetch session: the host takes
//! [`StormWatchPlan::next_request`], sends its whether [`HttpRequest`], reports the answer with
//! [`StormWatchPlan::on_response`] or [`StormWatchPlan::on_transport_error`], and ends with
//! [`StormWatchPlan::finish`], which returns the spot's [`StormWatch`].
//!
//! - The models' hours at the spot (one Open-Meteo point request, metered and counted with
//!   Open-Meteo Marine) are fetched when the previous watch has none for the spot or they are
//!   [`POINT_MAX_AGE_SECS`] old, and earlier when `meta.json` reports a newer run. The run
//!   times are read only around a point request, or once a newer run could be out.
//! - NHC's active storms are read on every plan. They carry no coordinates and are the same
//!   for everyone. A storm's a-deck (forecast tracks, gzip) is read when its position or known
//!   forecast comes near, or it is close enough that its forecast might; its b-deck (observed
//!   track) only when the forecast does. Tracks of the storm's current forecast advisory are
//!   reused. A storm is near when its position or its official track comes within the
//!   configured distance, or its forecast cone covers the spot. Storms outside NHC's basins
//!   are not listed, so elsewhere the list is simply empty.
//! - Every failure becomes a [`ModelSourceError`]; the watch keeps what it had.

use std::collections::{BTreeMap, VecDeque};

use underbite_core::model::{SourceError as ModelSourceError, Spot, StormWatch};
use underbite_core::provenance::{Attribution, SourceId};
use underbite_core::time::UnixTime;
use whether_core::model::ResolvedLocation;
use whether_core::storm::analysis::distance_km;
use whether_core::storm::hurricane::{
    DRAWN_AIDS, KM_PER_NMI, OFFICIAL_LEAD_SECS, hurricane_reaches,
};
use whether_core::storm::{DETERMINISTIC_MODELS, Hurricane, ModelRun, StormOutlook, TrackFix};
use whether_providers::nhc::{self, ActiveStorm, FRESH_COPY, ForecastTracks};
use whether_providers::open_meteo::storm::{self as om, PointForecast};
use whether_providers::{HttpRequest, HttpResponse, ProviderError, check_status};

use crate::request::{CacheHint, Coordinates, Response};
use crate::session::usage::SharedUsage;
use crate::source::{Budget, FetchContext, Period, SourceError};

/// The storm point and run-time requests; counted in the Open-Meteo pool.
pub const OPEN_METEO_STORM: SourceId = SourceId::from_static("open-meteo-storm");
/// The US National Hurricane Center.
pub const NHC: SourceId = SourceId::from_static("nhc");
/// How long the models' hours at a spot are used before they are fetched again, seconds.
pub const POINT_MAX_AGE_SECS: i64 = 3 * 3_600;
/// The point request's budget, checked against the Open-Meteo pool like Open-Meteo Marine's.
pub const POINT_BUDGET: Budget = Budget {
    period: Period::Day,
    limit: 200,
};
/// How far from the spot a storm is followed unless configured otherwise, km.
pub const DEFAULT_HURRICANE_DISTANCE_KM: f64 = 3_000.0;
/// How much further a storm not known to come near has its forecast read: five days at
/// 25 kt, about as far as an official forecast travels.
const SEARCH_BEYOND_KM: f64 = 120.0 * 25.0 * KM_PER_NMI;
/// How far ahead a cone over the spot counts, seconds.
const CONE_HORIZON_SECS: i64 = 7 * 86_400;
/// How long the host may reuse a `meta.json`, which is the same for every spot, seconds.
const META_MAX_AGE_S: u32 = 600;
/// Degrees beyond which a previous point series belongs to another place.
const SAME_PLACE_DEG: f64 = 0.01;

/// What a [`StormRequest`] asks for; selects its parser.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StormResource {
    /// One model domain's `meta.json` (latest run).
    RunMeta(&'static str),
    /// The deterministic models' hours at the spot.
    Point,
    /// NHC's `CurrentStorms.json`.
    CurrentStorms,
    /// The a-deck of an NHC storm id.
    ADeck(String),
    /// The b-deck of an NHC storm id.
    BDeck(String),
}

/// One request for the host: whether's request plus how to count and cache it.
#[derive(Clone, Debug, PartialEq)]
pub struct StormRequest {
    /// The request to send, as whether builds it.
    pub http: HttpRequest,
    /// What it asks for.
    pub resource: StormResource,
    /// The source it counts and fails under.
    pub source: SourceId,
    /// How the host may cache the response; NHC files ask for a fresh copy.
    pub cache: CacheHint,
}

/// Settings and host capabilities for one plan.
#[derive(Clone, Debug, PartialEq)]
pub struct StormOptions {
    /// How close a storm's position, official track or cone must come, km.
    pub hurricane_distance_km: f64,
    /// Whether this fetch may spend metered calls (the point request).
    pub allow_metered: bool,
}

impl Default for StormOptions {
    fn default() -> Self {
        Self {
            hurricane_distance_km: DEFAULT_HURRICANE_DISTANCE_KM,
            allow_metered: true,
        }
    }
}

/// The watch with its failures and credits.
#[derive(Clone, Debug, PartialEq)]
pub struct StormOutcome {
    /// The watch for `SpotSnapshot.storm`.
    pub watch: StormWatch,
    /// What failed, for `SpotSnapshot.errors`.
    pub errors: Vec<ModelSourceError>,
    /// Credits of the parts the watch holds.
    pub attribution: Vec<Attribution>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PointPlan {
    Keep,
    CheckRuns,
    Fetch,
}

/// Where one active storm stands.
#[derive(Clone, Debug)]
enum Tracking {
    ADeck,
    BDeck(Box<Hurricane>),
    Near(Box<Hurricane>),
    Gone,
}

/// One spot's storm requests, planned and parsed without I/O.
#[derive(Debug)]
pub struct StormWatchPlan {
    name: String,
    place: (f64, f64),
    now: UnixTime,
    query: Coordinates,
    distance_km: f64,
    previous: Option<StormOutlook>,
    known: Vec<Hurricane>,
    usage: Option<SharedUsage>,
    metered: bool,
    point_plan: PointPlan,
    queue: VecDeque<StormRequest>,
    in_flight: Option<StormRequest>,
    metas_left: usize,
    runs: BTreeMap<&'static str, ModelRun>,
    point: Option<PointForecast>,
    storms: Option<Vec<(ActiveStorm, Tracking)>>,
    errors: Vec<ModelSourceError>,
}

impl StormWatchPlan {
    /// Plans `spot`'s requests at `ctx.now`, given its `previous` watch. The point request is
    /// metered: it goes out only with `usage` and `options.allow_metered`.
    pub fn new(
        spot: &Spot,
        previous: Option<&StormWatch>,
        options: &StormOptions,
        ctx: &FetchContext,
        usage: Option<SharedUsage>,
    ) -> Self {
        let place = (spot.latitude, spot.longitude);
        let now = ctx.now;
        let previous_point = previous.and_then(|w| w.point.clone()).filter(|o| {
            !o.points.is_empty()
                && (o.place.latitude - place.0).abs() <= SAME_PLACE_DEG
                && (o.place.longitude - place.1).abs() <= SAME_PLACE_DEG
        });
        let metered = options.allow_metered && usage.is_some();
        let point_plan = match &previous_point {
            _ if !metered => PointPlan::Keep,
            None => PointPlan::Fetch,
            Some(o) if !(0..POINT_MAX_AGE_SECS).contains(&(now.0 - o.fetched_at.0)) => {
                PointPlan::Fetch
            }
            Some(o)
                if o.points
                    .iter()
                    .any(|p| p.run.as_ref().is_some_and(|r| next_run_due(r, now))) =>
            {
                PointPlan::CheckRuns
            }
            Some(_) => PointPlan::Keep,
        };
        let mut plan = Self {
            name: spot.name.clone(),
            place,
            now,
            query: ctx
                .round(Coordinates {
                    latitude: place.0,
                    longitude: place.1,
                })
                .coordinates(),
            distance_km: options.hurricane_distance_km,
            previous: previous_point,
            known: previous.map(|w| w.hurricanes.clone()).unwrap_or_default(),
            usage,
            metered,
            point_plan,
            queue: VecDeque::new(),
            in_flight: None,
            metas_left: 0,
            runs: BTreeMap::new(),
            point: None,
            storms: None,
            errors: Vec::new(),
        };
        if point_plan != PointPlan::Keep {
            for (_, domain) in om::RUN_DOMAINS {
                plan.queue.push_back(StormRequest {
                    http: optional(om::meta_request(domain)),
                    resource: StormResource::RunMeta(domain),
                    source: OPEN_METEO_STORM,
                    cache: CacheHint {
                        max_age: Some(META_MAX_AGE_S),
                        ..CacheHint::default()
                    },
                });
                plan.metas_left += 1;
            }
        }
        if point_plan == PointPlan::Fetch {
            plan.queue.push_back(plan.point_request());
        }
        plan.queue.push_back(nhc_request(
            nhc::current_storms_request(),
            StormResource::CurrentStorms,
        ));
        plan
    }

    /// The next request to send, or `None` when the plan is done. A point request over the
    /// budget is not handed out and is recorded as rate limited.
    pub fn next_request(&mut self) -> Option<&StormRequest> {
        if let Some(unanswered) = self.in_flight.take() {
            self.settle(
                &unanswered,
                Err(SourceError::Transport("no response".into())),
            );
        }
        while let Some(request) = self.queue.pop_front() {
            if request.resource == StormResource::Point {
                match self.admit() {
                    Ok(()) => {}
                    Err(None) => continue,
                    Err(Some(error)) => {
                        self.errors.push(error.to_model(OPEN_METEO_STORM, None));
                        continue;
                    }
                }
            }
            self.in_flight = Some(request);
            return self.in_flight.as_ref();
        }
        None
    }

    /// Gives back the call counted for the point request last returned by
    /// [`Self::next_request`]: a cache answered it, so it is not a call to Open-Meteo.
    pub fn refund(&mut self) {
        if let (Some(request), Some(usage)) = (&self.in_flight, &self.usage)
            && request.resource == StormResource::Point
        {
            usage.with(|ledger| ledger.refund(&OPEN_METEO_STORM, self.now));
        }
    }

    /// Reports the response to the request last returned by [`Self::next_request`].
    pub fn on_response(&mut self, response: &Response<'_>) {
        let Some(request) = self.in_flight.take() else {
            return;
        };
        if request.source == OPEN_METEO_STORM
            && let Some(usage) = &self.usage
        {
            usage.with(|ledger| {
                if let Some(left) = response.rate_limit_remaining {
                    ledger.observe_remaining(&OPEN_METEO_STORM, POINT_BUDGET, left, self.now);
                }
                if response.status == 429 {
                    ledger.rate_limited(&OPEN_METEO_STORM, response.retry_after, self.now);
                }
            });
        }
        self.settle(&request, classify(response));
    }

    /// Reports that the request last returned by [`Self::next_request`] got no response.
    pub fn on_transport_error(&mut self, message: &str) {
        if let Some(request) = self.in_flight.take() {
            self.settle(&request, Err(SourceError::Transport(message.to_string())));
        }
    }

    /// Skips the metered point request if it is still waiting; NHC's still go out.
    pub fn cancel(&mut self) {
        self.metered = false;
        self.queue.retain(|r| r.resource != StormResource::Point);
    }

    /// The spot's watch at `now`: fresh parts where they arrived, the previous ones otherwise.
    pub fn finish(mut self) -> StormOutcome {
        if let Some(unanswered) = self.in_flight.take() {
            let error = SourceError::Transport("no response".into());
            self.errors.push(error.to_model(unanswered.source, None));
        }
        let point = match self.point.take() {
            Some(p) => Some(self.outlook(p)),
            None => self.previous.take(),
        };
        let hurricanes: Vec<Hurricane> = match self.storms.take() {
            None => self
                .known
                .iter()
                .filter(|h| self.near(h))
                .cloned()
                .collect(),
            Some(storms) => storms
                .into_iter()
                .filter_map(|(storm, tracking)| match tracking {
                    Tracking::Near(h) => Some(*h),
                    Tracking::BDeck(h) => Some(Hurricane {
                        past: self.earlier(&storm.id).map(|e| e.past).unwrap_or_default(),
                        ..*h
                    }),
                    Tracking::ADeck => {
                        let earlier = self.earlier(&storm.id);
                        let forecast = earlier.as_ref().map(forecast_of).unwrap_or_default();
                        let past = earlier.map(|e| e.past).unwrap_or_default();
                        Some(storm.hurricane(forecast, past)).filter(|h| self.near(h))
                    }
                    Tracking::Gone => None,
                })
                .collect(),
        };
        let mut attribution = Vec::new();
        if point.is_some() {
            attribution.push(Attribution {
                source: OPEN_METEO_STORM,
                credit: "Weather data by Open-Meteo.com (CC BY 4.0); ECMWF data CC BY 4.0.".into(),
                url: Some("https://open-meteo.com/".into()),
            });
        }
        if !hurricanes.is_empty() {
            attribution.push(Attribution {
                source: NHC,
                credit: "National Hurricane Center (NOAA). Not for navigation or as a sole \
                         safety source."
                    .into(),
                url: Some("https://www.nhc.noaa.gov/".into()),
            });
        }
        StormOutcome {
            watch: StormWatch {
                checked_at: self.now,
                point,
                hurricanes,
            },
            errors: self.errors,
            attribution,
        }
    }

    fn point_request(&self) -> StormRequest {
        StormRequest {
            http: HttpRequest {
                metered: true,
                ..optional(om::point_request(&self.query, &DETERMINISTIC_MODELS))
            },
            resource: StormResource::Point,
            source: OPEN_METEO_STORM,
            cache: CacheHint::default(),
        }
    }

    /// Spends one call for the point request: `Err(None)` when metered calls are off.
    fn admit(&self) -> Result<(), Option<SourceError>> {
        let Some(usage) = self.usage.as_ref().filter(|_| self.metered) else {
            return Err(None);
        };
        usage
            .with(|ledger| ledger.try_spend(&OPEN_METEO_STORM, POINT_BUDGET, self.now))
            .map_err(|quota| {
                Some(SourceError::RateLimited {
                    retry_after_s: Some(quota.retry_after_s(self.now)),
                })
            })
    }

    /// Parses `body` for `request`, or records why there is none, and moves the plan on.
    fn settle(&mut self, request: &StormRequest, body: Result<&[u8], SourceError>) {
        match &request.resource {
            StormResource::RunMeta(domain) => {
                let run = body.and_then(|b| parsed(om::parse_run_meta(domain, b)));
                if let Some(run) = record(&mut self.errors, &request.source, run) {
                    self.runs.insert(domain, run);
                }
                self.meta_done();
            }
            StormResource::Point => {
                let point = body.and_then(|b| parsed(om::parse_points(b, &DETERMINISTIC_MODELS)));
                self.point = record(&mut self.errors, &request.source, point);
            }
            StormResource::CurrentStorms => {
                let active = body.and_then(|b| parsed(nhc::parse_current_storms(b)));
                if let Some(active) = record(&mut self.errors, &request.source, active) {
                    let storms = active
                        .into_iter()
                        .map(|storm| {
                            let tracking = self.start(&storm);
                            (storm, tracking)
                        })
                        .collect();
                    self.storms = Some(storms);
                }
            }
            StormResource::ADeck(id) => {
                let read = body.and_then(|b| parsed(nhc::parse_adeck(b, &DRAWN_AIDS)));
                let read = record(&mut self.errors, &request.source, read);
                if let Some(storm) = self.storm(id) {
                    let earlier = self.earlier(id);
                    let forecast = match (read, earlier) {
                        (Some(read), Some(e)) if e.official_init > read.official_init => {
                            forecast_of(&e)
                        }
                        (Some(read), _) => read,
                        (None, e) => e.as_ref().map(forecast_of).unwrap_or_default(),
                    };
                    let tracking = self.decide(&storm, forecast);
                    self.set(id, tracking);
                }
            }
            StormResource::BDeck(id) => {
                let read = body.and_then(|b| parsed(nhc::parse_bdeck(b)));
                let read = record(&mut self.errors, &request.source, read);
                let past = read
                    .or_else(|| self.earlier(id).map(|e| e.past))
                    .unwrap_or_default();
                if let Some(Tracking::BDeck(h)) = self.tracking(id) {
                    self.set(id, Tracking::Near(Box::new(Hurricane { past, ..*h })));
                }
            }
        }
    }

    /// Counts one `meta.json` done; after the last, asks for the point if a run is newer.
    fn meta_done(&mut self) {
        self.metas_left = self.metas_left.saturating_sub(1);
        if self.metas_left > 0 || self.point_plan != PointPlan::CheckRuns {
            return;
        }
        let newer = self.previous.iter().flat_map(|o| &o.points).any(|p| {
            p.run.as_ref().is_some_and(|run| {
                self.runs
                    .get(run.model.as_str())
                    .is_some_and(|latest| latest.init > run.init)
            })
        });
        if newer && self.metered {
            self.queue.push_front(self.point_request());
        }
    }

    /// The first step for an active storm: reuse its current advisory's tracks, read its
    /// a-deck, or leave it out as too far.
    fn start(&mut self, storm: &ActiveStorm) -> Tracking {
        let earlier = self.earlier(&storm.id);
        if let Some(e) = earlier.as_ref().filter(|e| {
            e.forecast_advisory_time == Some(storm.forecast_time()) && e.official_is_current()
        }) {
            return self.decide(storm, forecast_of(e));
        }
        let known = storm.hurricane(
            earlier.as_ref().map(forecast_of).unwrap_or_default(),
            Vec::new(),
        );
        let position = (storm.latitude, storm.longitude);
        if !self.near(&known) && !self.within(position, self.distance_km + SEARCH_BEYOND_KM) {
            return Tracking::Gone;
        }
        match nhc::adeck_request(&storm.id) {
            Ok(http) => {
                let resource = StormResource::ADeck(storm.id.clone());
                self.queue.push_back(nhc_request(http, resource));
                Tracking::ADeck
            }
            Err(e) => {
                self.errors.push(provider_error(e, 0).to_model(NHC, None));
                self.decide(storm, forecast_of(&known))
            }
        }
    }

    /// Keeps a storm whose `forecast` comes near, reading its b-deck unless the known
    /// observed track is current.
    fn decide(&mut self, storm: &ActiveStorm, forecast: ForecastTracks) -> Tracking {
        let h = storm.hurricane(forecast, Vec::new());
        if !self.near(&h) {
            return Tracking::Gone;
        }
        let earlier_past = self.earlier(&storm.id).map(|e| e.past);
        if let Some(past) = earlier_past
            .as_ref()
            .filter(|past| past_is_current(past, storm.forecast_time()))
        {
            return Tracking::Near(Box::new(Hurricane {
                past: past.clone(),
                ..h
            }));
        }
        match nhc::bdeck_request(&storm.id) {
            Ok(http) => {
                let resource = StormResource::BDeck(storm.id.clone());
                self.queue.push_back(nhc_request(http, resource));
                Tracking::BDeck(Box::new(h))
            }
            Err(e) => {
                self.errors.push(provider_error(e, 0).to_model(NHC, None));
                let past = earlier_past.unwrap_or_default();
                Tracking::Near(Box::new(Hurricane { past, ..h }))
            }
        }
    }

    /// Whether `h` is near: its position or official track within the distance, or its cone
    /// over the spot within [`CONE_HORIZON_SECS`].
    fn near(&self, h: &Hurricane) -> bool {
        self.within((h.latitude, h.longitude), self.distance_km)
            || h.official
                .iter()
                .any(|f| self.within(f.position(), self.distance_km))
            || hurricane_reaches(h, self.place, self.now, CONE_HORIZON_SECS).is_some()
    }

    fn within(&self, (latitude, longitude): (f64, f64), km: f64) -> bool {
        distance_km(self.place.0, self.place.1, latitude, longitude) <= km
    }

    fn earlier(&self, id: &str) -> Option<Hurricane> {
        self.known.iter().find(|h| h.id == id).cloned()
    }

    fn storm(&self, id: &str) -> Option<ActiveStorm> {
        let storms = self.storms.as_ref()?;
        storms
            .iter()
            .find(|(s, _)| s.id == id)
            .map(|(s, _)| s.clone())
    }

    fn tracking(&self, id: &str) -> Option<Tracking> {
        let storms = self.storms.as_ref()?;
        storms
            .iter()
            .find(|(s, _)| s.id == id)
            .map(|(_, t)| t.clone())
    }

    fn set(&mut self, id: &str, tracking: Tracking) {
        if let Some(entry) = self
            .storms
            .as_mut()
            .and_then(|storms| storms.iter_mut().find(|(s, _)| s.id == id))
        {
            entry.1 = tracking;
        }
    }

    /// The point forecast as the watch keeps it: the spot's name and coordinates, Open-Meteo's
    /// zone, each model tagged with the run read from `meta.json`.
    fn outlook(&self, point: PointForecast) -> StormOutlook {
        let PointForecast {
            location,
            series: mut points,
        } = point;
        for p in &mut points {
            p.run = om::run_domain(&p.model)
                .and_then(|d| self.runs.get(d))
                .cloned();
        }
        StormOutlook {
            place: ResolvedLocation {
                name: self.name.clone(),
                latitude: self.place.0,
                longitude: self.place.1,
                ..location
            },
            points,
            fetched_at: self.now,
            ..StormOutlook::default()
        }
    }
}

/// The value of `result`, or `None` with its error recorded under `source`.
fn record<T>(
    errors: &mut Vec<ModelSourceError>,
    source: &SourceId,
    result: Result<T, SourceError>,
) -> Option<T> {
    result
        .map_err(|e| errors.push(e.to_model(source.clone(), None)))
        .ok()
}

/// whether's parse result, its error as a source error.
fn parsed<T>(result: Result<T, ProviderError>) -> Result<T, SourceError> {
    result.map_err(|e| provider_error(e, 200))
}

/// `request` marked optional: storms never fail a spot's fetch.
fn optional(request: HttpRequest) -> HttpRequest {
    HttpRequest {
        required: false,
        ..request
    }
}

/// An NHC request asking every cache on the way, the host's included, for a fresh copy.
fn nhc_request(request: HttpRequest, resource: StormResource) -> StormRequest {
    let mut http = optional(request);
    if !http.headers.iter().any(|(name, _)| *name == FRESH_COPY.0) {
        http.headers.push((FRESH_COPY.0, FRESH_COPY.1.to_string()));
    }
    StormRequest {
        http,
        resource,
        source: NHC,
        cache: CacheHint {
            fresh: true,
            ..CacheHint::default()
        },
    }
}

/// Whether a newer run than `run` may be out at `now`: one update interval after it was.
fn next_run_due(run: &ModelRun, now: UnixTime) -> bool {
    let interval = if run.model.starts_with("cmc_gem") {
        12 * 3_600
    } else {
        6 * 3_600
    };
    now.0 >= run.available.0 + interval
}

fn forecast_of(h: &Hurricane) -> ForecastTracks {
    ForecastTracks {
        official: h.official.clone(),
        official_init: h.official_init,
        aids: h.aids.clone(),
    }
}

/// Whether the observed track reaches the synoptic hour before the forecast advisory.
fn past_is_current(past: &[TrackFix], forecast_time: UnixTime) -> bool {
    past.last()
        .is_some_and(|f| f.time.0 + OFFICIAL_LEAD_SECS >= forecast_time.0)
}

/// The body of a 2xx response, or the error the status means.
fn classify<'a>(response: &Response<'a>) -> Result<&'a [u8], SourceError> {
    match response.status {
        200..=299 if response.body.trim_ascii().is_empty() => {
            Err(SourceError::NoData(Some("empty response".into())))
        }
        200..=299 => Ok(response.body),
        404 | 410 => Err(SourceError::NotFound(None)),
        429 => Err(SourceError::RateLimited {
            retry_after_s: response.retry_after,
        }),
        status => Err(
            match check_status(&HttpResponse::new(status, response.body)) {
                Err(e) => provider_error(e, status),
                Ok(()) => SourceError::Status {
                    status,
                    message: None,
                },
            },
        ),
    }
}

/// whether's error as a source error; `status` is the response's (0 before sending).
fn provider_error(error: ProviderError, status: u16) -> SourceError {
    match error {
        ProviderError::Http { status, message } => SourceError::Status { status, message },
        ProviderError::Rejected(m) | ProviderError::Unauthorized(m) => SourceError::Status {
            status,
            message: Some(m),
        },
        ProviderError::RateLimited { retry_after_s } => SourceError::RateLimited { retry_after_s },
        ProviderError::Malformed(m) => SourceError::Parse(m),
        ProviderError::Config(m) => SourceError::Unsupported(m),
        ProviderError::Transport(m) => SourceError::Transport(m),
        other => SourceError::Status {
            status,
            message: Some(other.to_string()),
        },
    }
}

#[cfg(test)]
mod tests {
    use underbite_core::model::{SourceErrorKind, WaterType};
    use whether_core::storm::analysis::{all_events, significant_event};

    use super::*;
    use crate::session::usage::{OPEN_METEO, UsageLedger};
    use crate::testutil::fixture_bytes;

    /// 2026-11-15 22:00 UTC, after the recorded runs were out.
    const NOW: i64 = 1_794_780_000;
    /// An hour after Josephine's advisory 5 (2026-10-22 21:00 UTC).
    const JOSEPHINE_NOW: i64 = 1_792_702_800 + 3_600;
    const HOUR: i64 = 3_600;
    const QUIET: &str = "nhc-current-storms-2026-11-15.json";
    const JOSEPHINE: &str = "whether-nhc-current-storms-josephine.synthetic.json";

    fn spot(latitude: f64, longitude: f64) -> Spot {
        Spot {
            id: "battery".into(),
            name: "The Battery".into(),
            latitude,
            longitude,
            water_type: WaterType::TidalRiverEstuary,
            water_body: None,
            time_zone: "America/New_York".into(),
        }
    }

    fn battery() -> Spot {
        spot(40.7003, -74.0142)
    }

    /// Serves the recorded storm files; `storms` names the active storms file.
    struct Net {
        storms: &'static str,
        icon_meta: Vec<u8>,
        down: Vec<&'static str>,
    }

    impl Net {
        fn new(storms: &'static str) -> Self {
            Self {
                storms,
                icon_meta: fixture_bytes("storms", "open-meteo-meta-dwd_icon-2026-11-15.json"),
                down: Vec::new(),
            }
        }

        fn answer(&self, url: &str) -> (u16, Vec<u8>) {
            if self.down.iter().any(|part| url.contains(part)) {
                return (503, b"Service Unavailable".to_vec());
            }
            let name = if url.contains("/dwd_icon/") {
                return (200, self.icon_meta.clone());
            } else if let Some(domain) = url.strip_suffix("/static/meta.json") {
                let domain = domain.rsplit('/').next().unwrap();
                format!("open-meteo-meta-{domain}-2026-11-15.json")
            } else if url.starts_with(om::FORECAST_URL) {
                "whether-open-meteo-storm-point-10001.json".into()
            } else if url == nhc::CURRENT_STORMS_URL {
                self.storms.into()
            } else if url.ends_with("aal102026.dat.gz") {
                "whether-nhc-aal102026.dat.gz".into()
            } else if url.ends_with("bal102026.dat") {
                "whether-nhc-bal102026.dat".into()
            } else {
                return (404, Vec::new());
            };
            (200, fixture_bytes("storms", &name))
        }
    }

    /// Runs a plan against `net`; the requests sent and the outcome.
    fn run(mut plan: StormWatchPlan, net: &Net) -> (Vec<StormRequest>, StormOutcome) {
        let mut sent = Vec::new();
        while let Some(request) = plan.next_request() {
            let request = request.clone();
            let (status, body) = net.answer(&request.http.url);
            plan.on_response(&Response::new(status, &body));
            sent.push(request);
        }
        (sent, plan.finish())
    }

    fn plan_at(
        now: i64,
        spot: &Spot,
        previous: Option<&StormWatch>,
        options: &StormOptions,
        usage: &SharedUsage,
    ) -> StormWatchPlan {
        let ctx = FetchContext::new(UnixTime(now), "underbite/test");
        StormWatchPlan::new(spot, previous, options, &ctx, Some(usage.clone()))
    }

    fn check(
        now: i64,
        previous: Option<&StormWatch>,
        net: &Net,
    ) -> (Vec<StormRequest>, StormOutcome) {
        let usage = SharedUsage::default();
        run(
            plan_at(now, &battery(), previous, &StormOptions::default(), &usage),
            net,
        )
    }

    fn resources(sent: &[StormRequest]) -> Vec<StormResource> {
        sent.iter().map(|r| r.resource.clone()).collect()
    }

    fn metas() -> Vec<StormResource> {
        om::RUN_DOMAINS
            .iter()
            .map(|(_, d)| StormResource::RunMeta(d))
            .collect()
    }

    #[test]
    fn a_quiet_day_brings_the_point_series_and_no_hurricanes() {
        let usage = SharedUsage::default();
        let plan = plan_at(NOW, &battery(), None, &StormOptions::default(), &usage);
        let (sent, outcome) = run(plan, &Net::new(QUIET));
        let listed: Vec<String> = sent
            .iter()
            .map(|r| {
                let (fresh, metered) = (r.cache.fresh, r.http.metered);
                format!(
                    "{} {} fresh={fresh} metered={metered}",
                    r.source, r.http.url
                )
            })
            .collect();
        insta::assert_yaml_snapshot!(listed);
        assert!(sent.iter().all(|r| !r.http.required));
        assert_eq!(outcome.errors, []);
        assert_eq!(usage.snapshot().used(&OPEN_METEO, UnixTime(NOW)), 1);

        let watch = outcome.watch;
        assert_eq!(watch.checked_at, UnixTime(NOW));
        assert_eq!(watch.hurricanes, []);
        let point = watch.point.unwrap();
        assert_eq!(point.points.len(), 4);
        assert!(point.points.iter().all(|p| p.run.is_some()));
        assert_eq!(point.place.name, "The Battery");
        assert_eq!(point.place.tz_name.as_deref(), Some("America/New_York"));
        assert!(point.ensembles.is_empty() && point.grids.is_empty());
        // The recorded ten days in New York are dry and calm: no storm ahead.
        assert_eq!(all_events(&point), []);
        assert_eq!(significant_event(&point, UnixTime(NOW)), None);
        let credited: Vec<&SourceId> = outcome.attribution.iter().map(|a| &a.source).collect();
        assert_eq!(credited, [&OPEN_METEO_STORM]);
    }

    #[test]
    fn a_storm_429_pauses_the_pool_for_a_day_at_most() {
        let usage = SharedUsage::default();
        let mut plan = plan_at(NOW, &battery(), None, &StormOptions::default(), &usage);
        let mut hit = false;
        while let Some(request) = plan.next_request() {
            if request.source == OPEN_METEO_STORM && !hit {
                hit = true;
                let mut response = Response::new(429, b"");
                response.retry_after = Some(u32::MAX);
                plan.on_response(&response);
            } else {
                let net = Net::new(QUIET);
                let (status, body) = net.answer(&request.http.url);
                plan.on_response(&Response::new(status, &body));
            }
        }
        assert!(hit);
        let until = usage.snapshot().paused_until(&OPEN_METEO, UnixTime(NOW));
        assert_eq!(until, Some(UnixTime(NOW + 86_400)));
    }

    #[test]
    fn a_second_plan_within_three_hours_makes_no_point_request() {
        let net = Net::new(QUIET);
        let (_, first) = check(NOW, None, &net);
        let (sent, again) = check(NOW + HOUR / 2, Some(&first.watch), &net);
        assert_eq!(resources(&sent), [StormResource::CurrentStorms]);
        assert_eq!(again.watch.point, first.watch.point);
        assert_eq!(again.watch.checked_at, UnixTime(NOW + HOUR / 2));

        let (sent, _) = check(NOW + POINT_MAX_AGE_SECS, Some(&first.watch), &net);
        assert!(resources(&sent).contains(&StormResource::Point));
    }

    #[test]
    fn a_newer_run_brings_the_point_early() {
        let net = Net::new(QUIET);
        let (_, first) = check(NOW, None, &net);
        // Two hours on, GFS's next run may be out: the run times are read, none is newer.
        let (sent, same) = check(NOW + 2 * HOUR, Some(&first.watch), &net);
        let mut expected = metas();
        expected.push(StormResource::CurrentStorms);
        assert_eq!(resources(&sent), expected);
        assert_eq!(same.watch.point, first.watch.point);

        let mut newer = Net::new(QUIET);
        newer.icon_meta = String::from_utf8(newer.icon_meta)
            .unwrap()
            .replace("1794765600", "1794787200")
            .replace("1794777781", "1794794400")
            .into_bytes();
        let (sent, fresh) = check(NOW + 2 * HOUR, Some(&first.watch), &newer);
        let mut expected = metas();
        expected.extend([StormResource::Point, StormResource::CurrentStorms]);
        assert_eq!(resources(&sent), expected);
        let point = fresh.watch.point.unwrap();
        assert_eq!(point.fetched_at, UnixTime(NOW + 2 * HOUR));
        let icon = point
            .points
            .iter()
            .find(|p| p.model == "icon_seamless")
            .unwrap();
        assert_eq!(icon.run.as_ref().unwrap().init, UnixTime(1_794_787_200));
    }

    #[test]
    fn josephine_within_the_distance_brings_her_tracks() {
        let net = Net::new(JOSEPHINE);
        let (sent, outcome) = check(JOSEPHINE_NOW, None, &net);
        let id = "al102026".to_string();
        assert_eq!(
            resources(&sent)[5..],
            [
                StormResource::CurrentStorms,
                StormResource::ADeck(id.clone()),
                StormResource::BDeck(id.clone())
            ]
        );
        assert_eq!(outcome.errors, []);
        let [h] = outcome.watch.hurricanes.as_slice() else {
            panic!("one hurricane expected");
        };
        assert_eq!((h.id.as_str(), h.name.as_str()), ("al102026", "Josephine"));
        assert!(!h.official.is_empty() && !h.aids.is_empty() && !h.past.is_empty());
        assert!(outcome.attribution.iter().any(|a| a.source == NHC));

        // The same advisory an hour later: her tracks are kept, no deck is read.
        let (sent, again) = check(JOSEPHINE_NOW + HOUR, Some(&outcome.watch), &net);
        assert_eq!(resources(&sent), [StormResource::CurrentStorms]);
        assert_eq!(again.watch.hurricanes, outcome.watch.hurricanes);
    }

    #[test]
    fn storms_beyond_the_distance_are_left_out() {
        let net = Net::new(JOSEPHINE);
        let usage = SharedUsage::default();
        let near_only = StormOptions {
            hurricane_distance_km: 300.0,
            ..StormOptions::default()
        };
        // About 620 km from the Battery and moving away: her forecast is read, then dropped.
        let plan = plan_at(JOSEPHINE_NOW, &battery(), None, &near_only, &usage);
        let (sent, outcome) = run(plan, &net);
        assert_eq!(
            resources(&sent)[5..],
            [
                StormResource::CurrentStorms,
                StormResource::ADeck("al102026".into())
            ]
        );
        assert!(outcome.watch.hurricanes.is_empty() && outcome.errors.is_empty());

        // Far outside NHC's basins: nothing more to read, an empty list, no error.
        let far = spot(20.0, 140.0);
        let plan = plan_at(JOSEPHINE_NOW, &far, None, &StormOptions::default(), &usage);
        let (sent, outcome) = run(plan, &net);
        assert_eq!(resources(&sent).last(), Some(&StormResource::CurrentStorms));
        assert!(outcome.watch.hurricanes.is_empty() && outcome.errors.is_empty());
    }

    #[test]
    fn a_cone_over_the_spot_counts_beyond_the_track_distance() {
        let net = Net::new(JOSEPHINE);
        let usage = SharedUsage::default();
        // Half a degree north of her 36-hour position: inside the cone, off the track.
        let beside = spot(44.0, -55.9);
        let options = StormOptions {
            hurricane_distance_km: 1.0,
            ..StormOptions::default()
        };
        let (_, outcome) = run(
            plan_at(JOSEPHINE_NOW, &beside, None, &options, &usage),
            &net,
        );
        let [h] = outcome.watch.hurricanes.as_slice() else {
            panic!("one hurricane expected");
        };
        assert!(
            h.official
                .iter()
                .all(|f| distance_km(44.0, -55.9, f.latitude, f.longitude) > 1.0)
        );
    }

    #[test]
    fn nhc_requests_carry_no_coordinates_and_ask_for_a_fresh_copy() {
        let net = Net::new(JOSEPHINE);
        let nhc_requests = |spot: &Spot| -> Vec<StormRequest> {
            let usage = SharedUsage::default();
            let plan = plan_at(JOSEPHINE_NOW, spot, None, &StormOptions::default(), &usage);
            run(plan, &net)
                .0
                .into_iter()
                .filter(|r| r.source == NHC)
                .collect()
        };
        let here = nhc_requests(&battery());
        assert_eq!(here.len(), 3);
        assert_eq!(here, nhc_requests(&spot(43.5, -55.9)));
        for r in &here {
            let url = &r.http.url;
            assert!(
                url.starts_with("https://www.nhc.noaa.gov/")
                    || url.starts_with("https://ftp.nhc.noaa.gov/")
            );
            assert!(!url.contains('?') && !url.contains("40.7") && !url.contains("74.0"));
            assert!(r.cache.fresh);
            let fresh = (FRESH_COPY.0, FRESH_COPY.1.to_string());
            assert!(r.http.headers.contains(&fresh));
        }
    }

    #[test]
    fn failures_become_source_errors() {
        let mut net = Net::new(JOSEPHINE);
        net.down = vec!["api.open-meteo.com/v1", "aid_public"];
        let usage = SharedUsage::default();
        let plan = plan_at(
            JOSEPHINE_NOW,
            &battery(),
            None,
            &StormOptions::default(),
            &usage,
        );
        let (_, outcome) = run(plan, &net);
        let failed: Vec<(&str, SourceErrorKind)> = outcome
            .errors
            .iter()
            .map(|e| (e.source.as_str(), e.kind))
            .collect();
        assert_eq!(
            failed,
            [
                ("open-meteo-storm", SourceErrorKind::Other),
                ("nhc", SourceErrorKind::Other)
            ]
        );
        // No series, but Josephine is still listed by her position, without a cone.
        assert_eq!(outcome.watch.point, None);
        let [h] = outcome.watch.hurricanes.as_slice() else {
            panic!("one hurricane expected");
        };
        assert_eq!(h.official, []);

        // A spent budget sends no point request.
        let mut ledger = UsageLedger::new();
        for _ in 0..POINT_BUDGET.limit {
            ledger.count(&OPEN_METEO, Period::Day, UnixTime(NOW));
        }
        let exhausted = SharedUsage::new(ledger);
        let plan = plan_at(NOW, &battery(), None, &StormOptions::default(), &exhausted);
        let (sent, outcome) = run(plan, &Net::new(QUIET));
        assert!(!resources(&sent).contains(&StormResource::Point));
        assert_eq!(outcome.errors[0].kind, SourceErrorKind::RateLimited);
    }
}
