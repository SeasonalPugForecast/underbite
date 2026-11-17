//! The source trait: what a data source offers, how it plans requests and parses responses.
//!
//! A [`Source`] holds only settings. It never does I/O and never reads the clock: the host
//! sends the [`Request`]s it plans and hands each [`Response`] back to [`Source::parse`],
//! which adds [`Fragment`]s of a spot snapshot to [`Fragments`].

use std::fmt;

use serde::Deserialize;
use underbite_core::datum::Heights;
use underbite_core::model::SourceError as ModelSourceError;
use underbite_core::model::{
    Coverage, CurrentSeries, FloodCategories, MarineAlert, Measurement, Percentile, Requirements,
    SourceErrorKind, TideSeries, Turbidity, WaveSeries, Wind, ZoneForecast,
};
use underbite_core::provenance::{Attribution, SourceId};
use underbite_core::series::{Series, Vector};
use underbite_core::time::UnixTime;

use crate::request::{
    BodyFormat, Coordinates, MAX_COORDINATE_DECIMALS, Request, Response, RoundedPoint, Target,
    is_secret_header, secret_values,
};

/// Follow-ups one link may make unless its source allows more ([`Source::max_follow_ups`]).
pub const DEFAULT_MAX_FOLLOW_UPS: u32 = 4;

/// Longest message kept from an error body, in characters.
const MAX_MESSAGE: usize = 200;

/// How long a [`Budget`] lasts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Period {
    /// One clock hour, UTC.
    Hour,
    /// One UTC day.
    Day,
    /// One UTC calendar month.
    Month,
}

/// How many metered requests a source allows in one period.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Budget {
    /// The period the limit covers.
    pub period: Period,
    /// The most metered requests in one period.
    pub limit: u32,
}

/// Secrets a source may add to its requests. `Debug` never shows them.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Secrets {
    /// The USGS Water Data API key (`api.data.gov`), raising the anonymous limit.
    pub usgs_api_key: Option<String>,
}

impl fmt::Debug for Secrets {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Secrets")
            .field("usgs_api_key", &self.usgs_api_key.as_ref().map(|_| "***"))
            .finish()
    }
}

/// Per-fetch context. The caller reads the clock and the keychain, never a source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FetchContext {
    /// The current time.
    pub now: UnixTime,
    /// `underbite/<version>`, sent where a source asks for one.
    pub user_agent: String,
    coordinate_precision: u8,
    secrets: Secrets,
}

impl FetchContext {
    /// A context with coordinates at [`MAX_COORDINATE_DECIMALS`] and no secrets.
    pub fn new(now: UnixTime, user_agent: impl Into<String>) -> Self {
        Self {
            now,
            user_agent: user_agent.into(),
            coordinate_precision: MAX_COORDINATE_DECIMALS,
            secrets: Secrets::default(),
        }
    }

    /// Sends coordinates with `decimals` places, clamped to [`MAX_COORDINATE_DECIMALS`].
    #[must_use]
    pub fn with_coordinate_precision(mut self, decimals: u8) -> Self {
        self.coordinate_precision = decimals.min(MAX_COORDINATE_DECIMALS);
        self
    }

    /// Adds the secrets sources may send.
    #[must_use]
    pub fn with_secrets(mut self, secrets: Secrets) -> Self {
        self.secrets = secrets;
        self
    }

    /// Decimals of a coordinate sent to a source, at most [`MAX_COORDINATE_DECIMALS`].
    pub fn coordinate_precision(&self) -> u8 {
        self.coordinate_precision
    }

    /// The secrets sources may send.
    pub fn secrets(&self) -> &Secrets {
        &self.secrets
    }

    /// `point` rounded to this context's precision, ready to leave the device.
    pub fn round(&self, point: Coordinates) -> RoundedPoint {
        RoundedPoint::new(point, self.coordinate_precision)
    }
}

/// Why a source did not deliver.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SourceError {
    /// The source answered with an error: a status other than 2xx, 404 and 429, or an error
    /// body under 2xx. `message` is the source's text, trimmed, shortened and without secrets.
    #[error("HTTP {status}{}", suffix(message.as_deref()))]
    Status {
        /// Status code.
        status: u16,
        /// The source's message, if any.
        message: Option<String>,
    },
    /// 429 Too Many Requests, or a metered request not sent because the budget is spent.
    #[error("rate limited")]
    RateLimited {
        /// Seconds to wait, when known.
        retry_after_s: Option<u32>,
    },
    /// The source has nothing for this station and time.
    #[error("no data{}", suffix(.0.as_deref()))]
    NoData(Option<String>),
    /// The station, zone or resource does not exist (404).
    #[error("not found{}", suffix(.0.as_deref()))]
    NotFound(Option<String>),
    /// The body did not have the expected shape.
    #[error("unexpected response: {0}")]
    Parse(String),
    /// The source cannot serve this target or measurement.
    #[error("not supported: {0}")]
    Unsupported(String),
    /// The newest value is older than the measurement allows.
    #[error("no recent data")]
    Stale {
        /// Time of the newest value.
        latest: UnixTime,
    },
    /// The request never got a response (DNS, TLS, timeout, connection reset).
    #[error("network: {0}")]
    Transport(String),
}

/// `": message"`, or nothing.
fn suffix(message: Option<&str>) -> String {
    message.map(|m| format!(": {m}")).unwrap_or_default()
}

impl SourceError {
    /// The snapshot's kind for this error.
    pub fn kind(&self) -> SourceErrorKind {
        match self {
            Self::RateLimited { .. } => SourceErrorKind::RateLimited,
            Self::NotFound(_) => SourceErrorKind::NotFound,
            Self::NoData(_) => SourceErrorKind::NoData,
            Self::Stale { .. } => SourceErrorKind::Stale,
            Self::Parse(_) => SourceErrorKind::Parse,
            Self::Transport(_) => SourceErrorKind::Transport,
            Self::Status { .. } | Self::Unsupported(_) => SourceErrorKind::Other,
        }
    }

    /// The error as a spot snapshot records it.
    pub fn to_model(&self, source: SourceId, measurement: Option<Measurement>) -> ModelSourceError {
        ModelSourceError {
            source,
            measurement,
            kind: self.kind(),
            message: self.to_string(),
        }
    }
}

/// A parsed value and the shape it has. Values are in canonical units with provenance.
#[derive(Clone, Debug, PartialEq)]
pub enum FragmentValue {
    /// Plain values: temperatures, discharge, water quality, pressure, a flow forecast.
    Series(Series<f32>),
    /// Heights above a datum: gage height, lake level, observed tide level, a stage forecast.
    Heights(Heights),
    /// Turbidity with the unit it was measured in.
    Turbidity(Series<Turbidity>),
    /// Tide events and curve.
    TideSeries(TideSeries),
    /// Tidal current events and curve.
    CurrentSeries(CurrentSeries),
    /// Flow vectors: an ocean current, or observed tidal currents.
    Vectors(Series<Vector>),
    /// Waves, observed or forecast.
    WaveSeries(WaveSeries),
    /// Wind observed at a station.
    Wind(Series<Wind>),
    /// A marine zone's text forecast.
    ZoneForecast(ZoneForecast),
    /// Marine alerts in effect.
    Alerts(Vec<MarineAlert>),
    /// A gauge's flood stages.
    FloodCategories(FloodCategories),
    /// A value among the normals for its day of year.
    Percentile(Percentile),
    /// What a station currently reports for a measurement, for discovery.
    Coverage {
        /// The station id as the source writes it.
        station_id: String,
        /// The station's coverage of the measurement.
        coverage: Coverage,
    },
}

impl FragmentValue {
    /// Whether this value can stand for `measurement`.
    pub fn fits(&self, measurement: Measurement) -> bool {
        use FragmentValue as V;
        use Measurement as M;
        if matches!(self, V::Coverage { .. }) {
            return true;
        }
        match measurement {
            M::WaterTemperature
            | M::DissolvedOxygen
            | M::Conductivity
            | M::Salinity
            | M::Ph
            | M::Chlorophyll
            | M::Clarity
            | M::Cyanobacteria
            | M::SeaSurfaceTemp
            | M::SeaSurfaceTempForecast
            | M::ObservedPressure => matches!(self, V::Series(_)),
            M::Discharge => matches!(self, V::Series(_) | V::Percentile(_)),
            M::GageHeight => matches!(self, V::Heights(_)),
            M::LakeLevel => matches!(self, V::Heights(_) | V::Percentile(_)),
            M::Turbidity => matches!(self, V::Turbidity(_)),
            M::Tide => matches!(self, V::TideSeries(_) | V::Heights(_)),
            M::TidalCurrent => matches!(self, V::CurrentSeries(_) | V::Vectors(_)),
            M::OceanCurrent => matches!(self, V::Vectors(_)),
            M::Waves | M::WavesForecast => matches!(self, V::WaveSeries(_)),
            M::MarineForecast => matches!(self, V::ZoneForecast(_)),
            M::MarineAlerts => matches!(self, V::Alerts(_)),
            M::FloodCategories => matches!(self, V::FloodCategories(_)),
            M::FlowForecast => matches!(self, V::Series(_) | V::Heights(_)),
            M::ObservedWind => matches!(self, V::Wind(_)),
            // No value shape carries these yet.
            M::Reservoir | M::Bloom => false,
        }
    }

    fn shape(&self) -> &'static str {
        match self {
            Self::Series(_) => "series",
            Self::Heights(_) => "heights",
            Self::Turbidity(_) => "turbidity",
            Self::TideSeries(_) => "tide series",
            Self::CurrentSeries(_) => "current series",
            Self::Vectors(_) => "vectors",
            Self::WaveSeries(_) => "wave series",
            Self::Wind(_) => "wind",
            Self::ZoneForecast(_) => "zone forecast",
            Self::Alerts(_) => "alerts",
            Self::FloodCategories(_) => "flood categories",
            Self::Percentile(_) => "percentile",
            Self::Coverage { .. } => "coverage",
        }
    }
}

/// One measurement's value from one response.
#[derive(Clone, Debug, PartialEq)]
pub struct Fragment {
    /// The measurement the value is for.
    pub measurement: Measurement,
    /// The value.
    pub value: FragmentValue,
}

/// What parsing a link's responses produced: fragments and the credits they owe.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Fragments {
    fragments: Vec<Fragment>,
    attributions: Vec<Attribution>,
}

impl Fragments {
    /// Nothing yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds `value` for `measurement`; an error if the value's shape cannot stand for it.
    pub fn push(
        &mut self,
        measurement: Measurement,
        value: FragmentValue,
    ) -> Result<(), SourceError> {
        if !value.fits(measurement) {
            return Err(SourceError::Unsupported(format!(
                "{} for {measurement:?}",
                value.shape()
            )));
        }
        self.fragments.push(Fragment { measurement, value });
        Ok(())
    }

    /// Adds a credit a response named (NWPS `dataAttribution`), once.
    pub fn credit(&mut self, attribution: Attribution) {
        if !self.attributions.contains(&attribution) {
            self.attributions.push(attribution);
        }
    }

    /// The fragments, in the order they were added.
    pub fn fragments(&self) -> &[Fragment] {
        &self.fragments
    }

    /// Credits named by responses, beyond [`Source::attribution`].
    pub fn attributions(&self) -> &[Attribution] {
        &self.attributions
    }

    /// True when nothing was added.
    pub fn is_empty(&self) -> bool {
        self.fragments.is_empty() && self.attributions.is_empty()
    }

    /// The fragments and credits.
    pub fn into_parts(self) -> (Vec<Fragment>, Vec<Attribution>) {
        (self.fragments, self.attributions)
    }
}

/// A data source: plans requests and parses responses. Implementations hold only settings;
/// they never do I/O or read the clock, and keep no state between calls.
pub trait Source: Send + Sync {
    /// Which source this is.
    fn id(&self) -> SourceId;
    /// The credit every display of its data shows.
    fn attribution(&self) -> Attribution;
    /// The measurements it can supply.
    fn measurements(&self) -> &[Measurement];
    /// How many metered requests ([`Request::metered`]) it allows, or `None` when it sends
    /// none.
    fn budget(&self) -> Option<Budget> {
        None
    }
    /// The most follow-ups one link may make (paging needs more than the default).
    fn max_follow_ups(&self) -> u32 {
        DEFAULT_MAX_FOLLOW_UPS
    }
    /// The requests for `target` within `requirements`, in the order to send them;
    /// [`Request::required`] marks those whose failure fails the link.
    fn plan(
        &self,
        target: &Target,
        requirements: &Requirements,
        ctx: &FetchContext,
    ) -> Result<Vec<Request>, SourceError>;
    /// Requests a successful `response` to `request` calls for next, from the response alone:
    /// the next page, or the NWS `/points` lookup's zone forecast. A follow-up of a required
    /// request is required. None by default.
    fn follow_up(
        &self,
        _request: &Request,
        _response: &Response<'_>,
        _ctx: &FetchContext,
    ) -> Result<Vec<Request>, SourceError> {
        Ok(Vec::new())
    }
    /// Parses `response` to `request` into `out`. Starts with [`check_response`].
    fn parse(
        &self,
        request: &Request,
        response: &Response<'_>,
        ctx: &FetchContext,
        out: &mut Fragments,
    ) -> Result<(), SourceError>;
}

/// The error fields of a JSON body, as the tier-one sources write them.
#[derive(Deserialize)]
struct ErrorProbe {
    /// CO-OPS `{"error":{"message":…}}`, Open-Meteo `"error": true`.
    error: Option<serde_json::Value>,
    /// Open-Meteo.
    reason: Option<serde_json::Value>,
    /// USGS (OGC API).
    description: Option<serde_json::Value>,
    /// NWS problem details.
    detail: Option<serde_json::Value>,
    /// NWPS.
    message: Option<serde_json::Value>,
    /// NWS problem details.
    title: Option<serde_json::Value>,
}

impl ErrorProbe {
    /// Whether the body says it is an error.
    fn is_error(&self) -> bool {
        match &self.error {
            None | Some(serde_json::Value::Null | serde_json::Value::Bool(false)) => false,
            Some(_) => true,
        }
    }

    /// The most specific message in the body.
    fn message(&self) -> Option<&str> {
        [
            self.error.as_ref().and_then(|e| e.get("message")),
            self.error.as_ref(),
            self.reason.as_ref(),
            self.description.as_ref(),
            self.detail.as_ref(),
            self.message.as_ref(),
            self.title.as_ref(),
        ]
        .into_iter()
        .flatten()
        .find_map(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|m| !m.is_empty())
    }
}

/// Whether a message reads as "nothing here" (CO-OPS: "No data was found…", "No Predictions
/// data was found…").
fn says_no_data(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.starts_with("no ") && lower.contains(" data")
}

fn looks_like_html(body: &[u8], content_type: Option<&str>) -> bool {
    body.first() == Some(&b'<')
        || content_type.is_some_and(|ct| ct.to_ascii_lowercase().contains("html"))
}

/// A message from an error body: the JSON message field, else the text unless it is HTML.
fn body_message(request: &Request, body: &[u8], content_type: Option<&str>) -> Option<String> {
    let probe = match body.first() {
        Some(b'{') => serde_json::from_slice::<ErrorProbe>(body).ok(),
        _ => None,
    };
    match &probe {
        Some(p) => tidy(p.message()?, request),
        None if looks_like_html(body, content_type) => None,
        None => tidy(&String::from_utf8_lossy(body), request),
    }
}

/// `text` trimmed, with the request's secrets masked, cut to [`MAX_MESSAGE`] characters;
/// `None` when nothing is left.
fn tidy(text: &str, request: &Request) -> Option<String> {
    let text = scrub(text.trim(), request);
    let cut = text
        .char_indices()
        .nth(MAX_MESSAGE)
        .map_or(text.len(), |(i, _)| i);
    (cut > 0).then(|| text[..cut].to_string())
}

/// `text` with every secret of `request` (key query values, secret header values) masked.
fn scrub(text: &str, request: &Request) -> String {
    let header_secrets = request
        .headers
        .iter()
        .filter(|(name, _)| is_secret_header(name))
        .map(|(_, value)| value.as_str());
    let mut out = text.to_string();
    for secret in secret_values(&request.url)
        .into_iter()
        .chain(header_secrets)
    {
        if !secret.is_empty() {
            out = out.replace(secret, "***");
        }
    }
    out
}

/// Checks a response before parsing and returns its trimmed body.
///
/// Status first: 2xx goes on, 404 and 410 are [`SourceError::NotFound`], 429 is
/// [`SourceError::RateLimited`] with `Retry-After`, anything else is [`SourceError::Status`]
/// with the body's message. A binary resource's body comes back as it arrived, untrimmed, and
/// only an empty one is [`SourceError::NoData`]. Any other body is trimmed: empty is
/// [`SourceError::NoData`]. For a JSON resource the body must be a JSON object or array, else it is the source's error text
/// ([`SourceError::Status`]; CO-OPS answers some errors in plain text under a JSON content
/// type); an object with a truthy `error` is an error even under 200, [`SourceError::NoData`]
/// when it says there is no data; malformed JSON is [`SourceError::Parse`].
pub fn check_response<'a>(
    request: &Request,
    response: &Response<'a>,
) -> Result<&'a [u8], SourceError> {
    let body = response.body.trim_ascii();
    let status = response.status;
    match status {
        200..=299 => {}
        404 | 410 => {
            return Err(SourceError::NotFound(body_message(
                request,
                body,
                response.content_type,
            )));
        }
        429 => {
            return Err(SourceError::RateLimited {
                retry_after_s: response.retry_after,
            });
        }
        _ => {
            return Err(SourceError::Status {
                status,
                message: body_message(request, body, response.content_type),
            });
        }
    }
    let format = request.resource.format();
    if format == BodyFormat::Binary {
        // Bytes that look like whitespace at either end belong to the archive.
        if response.body.is_empty() {
            return Err(SourceError::NoData(Some("empty response".to_string())));
        }
        return Ok(response.body);
    }
    if body.is_empty() {
        return Err(SourceError::NoData(Some("empty response".to_string())));
    }
    if format == BodyFormat::Text {
        return Ok(body);
    }
    match body[0] {
        b'{' => {
            let probe: ErrorProbe = serde_json::from_slice(body)
                .map_err(|e| SourceError::Parse(format!("invalid JSON: {e}")))?;
            if probe.is_error() {
                let message = probe.message().and_then(|m| tidy(m, request));
                return Err(match message {
                    Some(m) if says_no_data(&m) => SourceError::NoData(Some(m)),
                    message => SourceError::Status { status, message },
                });
            }
        }
        b'[' => {
            serde_json::from_slice::<serde::de::IgnoredAny>(body)
                .map_err(|e| SourceError::Parse(format!("invalid JSON: {e}")))?;
        }
        _ if looks_like_html(body, response.content_type) => {
            return Err(SourceError::Parse("HTML instead of JSON".to_string()));
        }
        _ => {
            return Err(SourceError::Status {
                status,
                message: body_message(request, body, response.content_type),
            });
        }
    }
    Ok(body)
}

/// A scripted source for tests of the session and of this module.
#[cfg(test)]
pub(crate) mod fake {
    use std::collections::{BTreeMap, BTreeSet};

    use underbite_core::datum::VerticalDatum;
    use underbite_core::provenance::{
        Provenance, Quality, Relation, StationKind, StationRef, ValueKind,
    };
    use underbite_core::series::Sample;

    use super::*;
    use crate::request::{Place, Resource, Window};

    /// A request the fake plans: its URL and flags; the target is the planned one. A
    /// follow-up that `carries` takes the earlier response's values along in its carry.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub(crate) struct FakeRequest {
        pub(crate) url: String,
        pub(crate) required: bool,
        pub(crate) metered: bool,
        pub(crate) carries: bool,
    }

    impl FakeRequest {
        pub(crate) fn required(url: &str) -> Self {
            Self {
                url: url.to_string(),
                required: true,
                metered: false,
                carries: false,
            }
        }

        pub(crate) fn optional(url: &str) -> Self {
            Self {
                required: false,
                ..Self::required(url)
            }
        }

        #[must_use]
        pub(crate) fn metered(mut self) -> Self {
            self.metered = true;
            self
        }

        #[must_use]
        pub(crate) fn carrying(mut self) -> Self {
            self.carries = true;
            self
        }

        fn build(&self, target: &Target) -> Request {
            let mut request = Request::new(self.url.clone(), Resource::Fake, target.clone());
            request.required = self.required;
            request.metered = self.metered;
            request
        }
    }

    /// A source that plans fixed URLs, follows up by URL, and parses bodies of the form
    /// `{"water_temperature": [[unix_seconds, value], …], …}` into series (heights in the
    /// gauge datum for gage height, lake level and tide) for the target's measurements,
    /// together with the values a carrying follow-up brought along.
    #[derive(Clone, Debug)]
    pub(crate) struct FakeSource {
        pub(crate) id: SourceId,
        pub(crate) measurements: Vec<Measurement>,
        pub(crate) budget: Option<Budget>,
        pub(crate) max_follow_ups: u32,
        pub(crate) plan: Vec<FakeRequest>,
        pub(crate) follow_ups: BTreeMap<String, Vec<FakeRequest>>,
    }

    impl FakeSource {
        pub(crate) fn new(id: &str, measurements: &[Measurement]) -> Self {
            Self {
                id: SourceId::new(id),
                measurements: measurements.to_vec(),
                budget: None,
                max_follow_ups: DEFAULT_MAX_FOLLOW_UPS,
                plan: Vec::new(),
                follow_ups: BTreeMap::new(),
            }
        }

        #[must_use]
        pub(crate) fn plans(mut self, request: FakeRequest) -> Self {
            self.plan.push(request);
            self
        }

        #[must_use]
        pub(crate) fn follows(mut self, url: &str, request: FakeRequest) -> Self {
            self.follow_ups
                .entry(url.to_string())
                .or_default()
                .push(request);
            self
        }

        #[must_use]
        pub(crate) fn with_budget(mut self, budget: Budget) -> Self {
            self.budget = Some(budget);
            self
        }

        fn provenance(&self, target: &Target, observed_at: UnixTime) -> Provenance {
            let (station, distance_m, relation) = match &target.place {
                Place::Station {
                    station,
                    distance_m,
                    relation,
                } => (Some(station.clone()), *distance_m, *relation),
                Place::Point(_) | Place::Zone(_) => (None, None, Relation::Gridded),
            };
            Provenance {
                source: self.id.clone(),
                station,
                distance_m,
                relation,
                observed_at,
                kind: ValueKind::Observed,
                quality: Quality::Provisional,
            }
        }
    }

    impl Source for FakeSource {
        fn id(&self) -> SourceId {
            self.id.clone()
        }

        fn attribution(&self) -> Attribution {
            Attribution {
                source: self.id.clone(),
                credit: format!("Test data from {}", self.id),
                url: None,
            }
        }

        fn measurements(&self) -> &[Measurement] {
            &self.measurements
        }

        fn budget(&self) -> Option<Budget> {
            self.budget
        }

        fn max_follow_ups(&self) -> u32 {
            self.max_follow_ups
        }

        fn plan(
            &self,
            target: &Target,
            _requirements: &Requirements,
            _ctx: &FetchContext,
        ) -> Result<Vec<Request>, SourceError> {
            if !target
                .measurements
                .iter()
                .any(|m| self.measurements.contains(m))
            {
                return Err(SourceError::Unsupported(format!(
                    "{} has none of {:?}",
                    self.id, target.measurements
                )));
            }
            Ok(self.plan.iter().map(|r| r.build(target)).collect())
        }

        fn follow_up(
            &self,
            request: &Request,
            response: &Response<'_>,
            _ctx: &FetchContext,
        ) -> Result<Vec<Request>, SourceError> {
            let Some(next) = self.follow_ups.get(&request.url) else {
                return Ok(Vec::new());
            };
            let mut requests = Vec::with_capacity(next.len());
            for r in next {
                let built = r.build(&request.target);
                requests.push(if r.carries {
                    let body = check_response(request, response)?;
                    built.with_carry(String::from_utf8_lossy(body))
                } else {
                    built
                });
            }
            Ok(requests)
        }

        fn parse(
            &self,
            request: &Request,
            response: &Response<'_>,
            ctx: &FetchContext,
            out: &mut Fragments,
        ) -> Result<(), SourceError> {
            let body = check_response(request, response)?;
            let decode = |bytes: &[u8]| {
                serde_json::from_slice::<BTreeMap<Measurement, Vec<(i64, f32)>>>(bytes)
                    .map_err(|e| SourceError::Parse(e.to_string()))
            };
            let mut values = decode(body)?;
            if let Some(carry) = request.carry() {
                for (measurement, points) in decode(carry.as_bytes())? {
                    values.entry(measurement).or_default().extend(points);
                }
            }
            for (measurement, points) in values {
                if !request.target.measurements.contains(&measurement) {
                    continue;
                }
                let latest = points.iter().map(|p| p.0).max().map_or(ctx.now, UnixTime);
                let samples = points
                    .into_iter()
                    .map(|(t, v)| Sample::new(UnixTime(t), v))
                    .collect();
                let series = Series::new(self.provenance(&request.target, latest), samples);
                let value = match measurement {
                    Measurement::GageHeight | Measurement::LakeLevel | Measurement::Tide => {
                        FragmentValue::Heights(Heights {
                            datum: VerticalDatum::Gauge,
                            series,
                        })
                    }
                    _ => FragmentValue::Series(series),
                };
                out.push(measurement, value)?;
            }
            Ok(())
        }
    }

    /// A target at a test station for `measurements`, over a week before `now`.
    pub(crate) fn target(station_id: &str, measurements: &[Measurement], now: UnixTime) -> Target {
        Target {
            place: Place::Station {
                station: StationRef {
                    source: SourceId::new("fake"),
                    id: station_id.to_string(),
                    name: format!("Test station {station_id}"),
                    kind: StationKind::WaterLevel,
                    latitude: 40.7,
                    longitude: -74.0,
                    bin: None,
                    reference_id: None,
                },
                distance_m: Some(500.0),
                relation: Relation::SameWaterBody,
            },
            measurements: measurements.iter().copied().collect::<BTreeSet<_>>(),
            window: Window {
                from: UnixTime(now.0 - 7 * 86_400),
                to: now,
            },
            datum: None,
        }
    }

    /// A context at `now` with no secrets.
    pub(crate) fn ctx(now: UnixTime) -> FetchContext {
        FetchContext::new(now, "underbite/test")
    }
}

#[cfg(test)]
mod tests {
    use super::fake::{FakeRequest, FakeSource};
    use super::*;
    use crate::request::{Place, Resource, Window};
    use crate::testutil::fixture_bytes;
    use std::collections::BTreeSet;
    use underbite_core::model::{SourceErrorKind, ZoneKind, ZoneProduct};
    use underbite_core::provenance::{Provenance, Quality, Relation, ValueKind};

    const NOW: UnixTime = UnixTime(1_794_000_000);

    fn prov() -> Provenance {
        Provenance {
            source: SourceId::NDBC,
            station: None,
            distance_m: None,
            relation: Relation::Nearby,
            observed_at: NOW,
            kind: ValueKind::Observed,
            quality: Quality::Provisional,
        }
    }

    fn request(resource: Resource) -> Request {
        Request::new(
            "https://api.example.test/data?site=1".into(),
            resource,
            fake::target("1", &[Measurement::WaterTemperature], NOW),
        )
    }

    fn check(resource: Resource, status: u16, body: &[u8]) -> Result<Vec<u8>, SourceError> {
        check_response(&request(resource), &Response::new(status, body)).map(<[u8]>::to_vec)
    }

    fn check_fixture(
        resource: Resource,
        source: &str,
        name: &str,
        status: u16,
    ) -> Result<Vec<u8>, SourceError> {
        check(resource, status, &fixture_bytes(source, name))
    }

    #[test]
    fn source_check_classifies_coops_plain_text_400() {
        let err = check_fixture(
            Resource::CoopsTideHighLow,
            "coops",
            "coops-error-no-product.txt",
            400,
        )
        .unwrap_err();
        assert_eq!(
            err,
            SourceError::Status {
                status: 400,
                message: Some("Wrong Product : Product cannot be null or empty".into()),
            }
        );
        assert_eq!(err.kind(), SourceErrorKind::Other);
    }

    #[test]
    fn source_check_classifies_coops_error_object_under_200_as_no_data() {
        let err = check_fixture(
            Resource::CoopsWind,
            "coops",
            "coops-8518750-wind-2026-11-07-error-nodata.json",
            200,
        )
        .unwrap_err();
        assert_eq!(
            err,
            SourceError::NoData(Some(
                "No data was found. This product may not be offered at this station at the requested time."
                    .into()
            ))
        );
        assert_eq!(err.kind(), SourceErrorKind::NoData);
    }

    #[test]
    fn source_check_classifies_usgs_limit_exceeded() {
        let err = check_fixture(
            Resource::UsgsContinuous,
            "usgs",
            "usgs-error-limit-exceeded.json",
            400,
        )
        .unwrap_err();
        assert_eq!(
            err,
            SourceError::Status {
                status: 400,
                message: Some("Limit of 50000 exceeded".into()),
            }
        );
    }

    /// Every recorded error body with the status it was recorded with.
    #[test]
    fn source_check_classifies_recorded_errors() {
        let cases: &[(Resource, &str, &str, u16)] = &[
            (
                Resource::CoopsTideHighLow,
                "coops",
                "coops-error-bad-bin.json",
                400,
            ),
            (
                Resource::CoopsTideHighLow,
                "coops",
                "coops-error-end-before-begin.json",
                400,
            ),
            (
                Resource::CoopsTideHighLow,
                "coops",
                "coops-error-no-datum.json",
                400,
            ),
            (
                Resource::CoopsTideHighLow,
                "coops",
                "coops-error-no-product.txt",
                400,
            ),
            (
                Resource::CoopsTideHighLow,
                "coops",
                "coops-error-range-limit-hilo.json",
                400,
            ),
            (
                Resource::CoopsCurrents,
                "coops",
                "coops-error-nodata-currents-n03020.json",
                200,
            ),
            (
                Resource::CoopsTideCurve,
                "coops",
                "coops-error-subordinate-interval-6.json",
                200,
            ),
            (
                Resource::CoopsWind,
                "coops",
                "coops-8518750-wind-2026-11-07-error-nodata.json",
                200,
            ),
            (
                Resource::UsgsContinuous,
                "usgs",
                "usgs-error-limit-exceeded.json",
                400,
            ),
            (
                Resource::OpenMeteoMarine,
                "open-meteo-marine",
                "marine-error-forecast-days.json",
                400,
            ),
            (
                Resource::OpenMeteoMarine,
                "open-meteo-marine",
                "marine-error-invalid-variable.json",
                400,
            ),
            (
                Resource::NwsPoint,
                "nws-marine",
                "points-45.0_-30.0-invalid-point.problem.json",
                404,
            ),
            (
                Resource::NwsZoneForecast,
                "nws-marine",
                "zone-forecast-LEZ146-not-found.problem.json",
                404,
            ),
            (
                Resource::NwsZoneForecast,
                "nws-marine",
                "zone-forecast-ANZ820-offshore-unsupported.problem.json",
                404,
            ),
            (
                Resource::NdbcOcean,
                "ndbc",
                "ndbc-44025.ocean-404.html",
                404,
            ),
        ];
        let results: Vec<String> = cases
            .iter()
            .map(|&(resource, source, name, status)| {
                let err = check_fixture(resource, source, name, status).unwrap_err();
                format!("{name} ({status}): {err:?} => {:?}: {err}", err.kind())
            })
            .collect();
        insta::assert_debug_snapshot!(results);
    }

    #[test]
    fn source_check_passes_recorded_data() {
        let hilo = fixture_bytes("coops", "coops-8518750-hilo-2026-11-08.json");
        let body = check(Resource::CoopsTideHighLow, 200, &hilo).unwrap();
        assert_eq!(body, hilo.trim_ascii());
        let txt = fixture_bytes("ndbc", "ndbc-44025.txt");
        assert_eq!(
            check(Resource::NdbcStandard, 200, &txt).unwrap(),
            txt.trim_ascii()
        );
        let alerts = fixture_bytes("nws-marine", "alerts-active-zone-ANZ338.geo.json");
        assert!(check(Resource::NwsAlerts, 200, &alerts).is_ok());
        assert_eq!(
            check(Resource::Fake, 200, b" [1, 2] \n").unwrap(),
            b"[1, 2]"
        );
        // `"error": false` and `null` are not errors.
        assert!(check(Resource::Fake, 200, br#"{"error":false,"data":[]}"#).is_ok());
        assert!(check(Resource::Fake, 200, br#"{"error":null}"#).is_ok());
    }

    #[test]
    fn source_check_passes_a_binary_body_untrimmed() {
        let zip = fixture_bytes("hab", "noaa-hab-gulf-rif-currentmodelrun-2026-11-17.zip");
        assert_eq!(check(Resource::HabGulfZip, 200, &zip).unwrap(), zip);
        // Bytes that look like whitespace or JSON are the archive's own.
        let edges = b" PK\x03\x04 {\n";
        assert_eq!(check(Resource::HabGulfZip, 200, edges).unwrap(), edges);
        assert_eq!(
            check(Resource::HabGulfZip, 200, b""),
            Err(SourceError::NoData(Some("empty response".into())))
        );
        let missing =
            br#"<?xml version="1.0" encoding="utf-8"?><Error><Code>BlobNotFound</Code></Error>"#;
        assert_eq!(
            check(Resource::HabGulfZip, 404, missing),
            Err(SourceError::NotFound(None))
        );
    }

    #[test]
    fn source_check_rejects_empty_html_and_broken_bodies() {
        assert_eq!(
            check(Resource::OpenMeteoMarine, 200, b"  \n"),
            Err(SourceError::NoData(Some("empty response".into())))
        );
        assert_eq!(
            check(Resource::Fake, 200, b"<html><body>Sign in</body></html>"),
            Err(SourceError::Parse("HTML instead of JSON".into()))
        );
        assert!(matches!(
            check(Resource::Fake, 200, br#"{"data": [1, 2"#),
            Err(SourceError::Parse(m)) if m.starts_with("invalid JSON")
        ));
        assert!(matches!(
            check(Resource::Fake, 200, b"[1, 2"),
            Err(SourceError::Parse(_))
        ));
        assert_eq!(
            check(Resource::Fake, 200, b"\n Wrong Product : nothing \n"),
            Err(SourceError::Status {
                status: 200,
                message: Some("Wrong Product : nothing".into())
            })
        );
        assert_eq!(
            check(
                Resource::Fake,
                200,
                br#"{"error":true,"reason":"bad variable"}"#
            ),
            Err(SourceError::Status {
                status: 200,
                message: Some("bad variable".into())
            })
        );
    }

    #[test]
    fn source_check_maps_statuses() {
        let resp = Response {
            retry_after: Some(30),
            ..Response::new(429, b"slow down")
        };
        assert_eq!(
            check_response(&request(Resource::UsgsDaily), &resp),
            Err(SourceError::RateLimited {
                retry_after_s: Some(30)
            })
        );
        assert_eq!(
            check(
                Resource::NwpsGauge,
                404,
                br#"{"code":5,"message":"[ZZZZ9] could not find gauge ID","details":[]}"#
            ),
            Err(SourceError::NotFound(Some(
                "[ZZZZ9] could not find gauge ID".into()
            )))
        );
        assert_eq!(
            check(Resource::Fake, 503, b""),
            Err(SourceError::Status {
                status: 503,
                message: None
            })
        );
        let html = Response {
            content_type: Some("text/html; charset=iso-8859-1"),
            ..Response::new(504, b"Gateway Time-out")
        };
        assert_eq!(
            check_response(&request(Resource::CoopsTideCurve), &html),
            Err(SourceError::Status {
                status: 504,
                message: None
            })
        );
    }

    #[test]
    fn source_error_messages_hide_secrets_and_stay_short() {
        let keyed = Request::new(
            "https://api.example.test/data?site=1&api_key=SECRET-PARAM".into(),
            Resource::UsgsDaily,
            fake::target("1", &[Measurement::Discharge], NOW),
        )
        .header("X-Api-Key", "SECRET-HEADER");
        let body = br#"{"description":"key SECRET-PARAM or SECRET-HEADER rejected"}"#;
        let err = check_response(&keyed, &Response::new(403, body)).unwrap_err();
        let text = format!("{err} {err:?}");
        assert!(!text.contains("SECRET"), "{text}");
        assert!(text.contains("key *** or *** rejected"), "{text}");

        // A key that straddles the cut is masked before the message is shortened.
        let straddling = format!(
            r#"{{"error":{{"message":"{}SECRET-PARAM"}}}}"#,
            "x".repeat(195)
        );
        let err = check_response(&keyed, &Response::new(200, straddling.as_bytes())).unwrap_err();
        let SourceError::Status {
            status: 200,
            message: Some(m),
        } = &err
        else {
            panic!("expected a status error, got {err:?}");
        };
        assert!(!m.contains("SECRET") && m.ends_with("***"), "{m}");
        assert_eq!(m.chars().count(), 198);

        let long = "x".repeat(1_000);
        for (status, body) in [
            (500, long.clone()),
            (200, format!(r#"{{"error":"{long}"}}"#)),
        ] {
            let Err(SourceError::Status {
                message: Some(m), ..
            }) = check(Resource::Fake, status, body.as_bytes())
            else {
                panic!("expected a status error");
            };
            assert_eq!(m.chars().count(), MAX_MESSAGE);
        }
    }

    #[test]
    fn source_errors_map_to_the_model() {
        let cases = [
            (
                SourceError::RateLimited {
                    retry_after_s: None,
                },
                SourceErrorKind::RateLimited,
            ),
            (SourceError::NotFound(None), SourceErrorKind::NotFound),
            (SourceError::NoData(None), SourceErrorKind::NoData),
            (SourceError::Stale { latest: NOW }, SourceErrorKind::Stale),
            (SourceError::Parse("x".into()), SourceErrorKind::Parse),
            (
                SourceError::Transport("timeout".into()),
                SourceErrorKind::Transport,
            ),
            (SourceError::Unsupported("x".into()), SourceErrorKind::Other),
            (
                SourceError::Status {
                    status: 500,
                    message: None,
                },
                SourceErrorKind::Other,
            ),
        ];
        for (err, kind) in cases {
            assert_eq!(err.kind(), kind, "{err:?}");
        }
        let model = SourceError::RateLimited {
            retry_after_s: Some(60),
        }
        .to_model(SourceId::USGS, Some(Measurement::Discharge));
        assert_eq!(model.source, SourceId::USGS);
        assert_eq!(model.measurement, Some(Measurement::Discharge));
        assert_eq!(model.kind, SourceErrorKind::RateLimited);
        assert_eq!(model.message, "rate limited");
        assert_eq!(
            SourceError::Status {
                status: 400,
                message: Some("Limit of 50000 exceeded".into())
            }
            .to_string(),
            "HTTP 400: Limit of 50000 exceeded"
        );
        assert_eq!(SourceError::NotFound(None).to_string(), "not found");
    }

    #[test]
    fn source_context_clamps_precision_and_hides_secrets() {
        let ctx = FetchContext::new(NOW, "underbite/0.1").with_coordinate_precision(5);
        assert_eq!(ctx.coordinate_precision(), 2);
        let coarse = ctx.clone().with_coordinate_precision(1);
        assert_eq!(coarse.coordinate_precision(), 1);
        let battery = Coordinates {
            latitude: 40.703_3,
            longitude: -74.017,
        };
        assert_eq!(
            ctx.round(battery).fmt_param(),
            ("40.7".to_string(), "-74.02".to_string())
        );
        let ctx = ctx.with_secrets(Secrets {
            usgs_api_key: Some("SECRET-KEY".into()),
        });
        assert_eq!(ctx.secrets().usgs_api_key.as_deref(), Some("SECRET-KEY"));
        let debug = format!("{ctx:?}");
        assert!(
            !debug.contains("SECRET-KEY") && debug.contains("***"),
            "{debug}"
        );
    }

    #[test]
    fn source_fragments_accept_only_fitting_shapes() {
        let series = || FragmentValue::Series(Series::new(prov(), Vec::new()));
        let mut out = Fragments::new();
        assert!(out.is_empty());
        out.push(Measurement::WaterTemperature, series()).unwrap();
        assert_eq!(
            out.push(Measurement::Tide, series()),
            Err(SourceError::Unsupported("series for Tide".into()))
        );
        out.push(
            Measurement::Waves,
            FragmentValue::Coverage {
                station_id: "44025".into(),
                coverage: Coverage {
                    last_value_at: Some(NOW),
                    primary: true,
                },
            },
        )
        .unwrap();
        let credit = Attribution {
            source: SourceId::NWPS,
            credit: "US Geological Survey".into(),
            url: None,
        };
        out.credit(credit.clone());
        out.credit(credit);
        assert_eq!(out.fragments().len(), 2);
        assert_eq!(out.attributions().len(), 1);
        let (fragments, credits) = out.into_parts();
        assert_eq!(fragments[0].measurement, Measurement::WaterTemperature);
        assert_eq!(credits[0].source, SourceId::NWPS);
    }

    #[test]
    fn source_every_measurement_has_a_shape() {
        use underbite_core::datum::VerticalDatum;
        let prov = prov();
        let shapes = [
            FragmentValue::Series(Series::new(prov.clone(), Vec::new())),
            FragmentValue::Heights(Heights {
                datum: VerticalDatum::Mllw,
                series: Series::new(prov.clone(), Vec::new()),
            }),
            FragmentValue::Turbidity(Series::new(prov.clone(), Vec::new())),
            FragmentValue::Vectors(Series::new(prov.clone(), Vec::new())),
            FragmentValue::WaveSeries(Series::new(prov.clone(), Vec::new())),
            FragmentValue::Wind(Series::new(prov.clone(), Vec::new())),
            FragmentValue::Alerts(Vec::new()),
            FragmentValue::ZoneForecast(ZoneForecast {
                id: "ANZ338".into(),
                name: "New York Harbor".into(),
                kind: ZoneKind::Coastal,
                issued: NOW,
                product: ZoneProduct::ZoneApi,
                periods: Vec::new(),
            }),
            FragmentValue::FloodCategories(FloodCategories::default()),
        ];
        let all: BTreeSet<Measurement> = serde_json::from_str(
            r#"["water_temperature","discharge","gage_height","lake_level","turbidity",
                "dissolved_oxygen","conductivity","salinity","ph","chlorophyll","clarity",
                "cyanobacteria","tide","tidal_current","ocean_current","waves","waves_forecast",
                "sea_surface_temp","sea_surface_temp_forecast","marine_forecast","marine_alerts",
                "flood_categories","flow_forecast","observed_wind","observed_pressure"]"#,
        )
        .unwrap();
        assert_eq!(all.len(), 25);
        for m in all {
            assert!(shapes.iter().any(|s| s.fits(m)), "{m:?}");
        }
    }

    #[test]
    fn source_fake_chain_carries_a_value_into_the_follow_up_parse() {
        let src = FakeSource::new("fake-chain", &[Measurement::WaterTemperature])
            .plans(FakeRequest::required("https://fake.test/first"))
            .follows(
                "https://fake.test/first",
                FakeRequest::required("https://fake.test/second").carrying(),
            );
        let source: &dyn Source = &src;
        let ctx = fake::ctx(NOW);
        let target = fake::target("1", &[Measurement::WaterTemperature], NOW);
        let first = source
            .plan(&target, &Requirements::default(), &ctx)
            .unwrap()
            .remove(0);
        assert_eq!(first.carry(), None);

        // The first response holds a value the second does not.
        let first_body = br#" {"water_temperature":[[1793999000,14.5]]} "#;
        let next = source
            .follow_up(&first, &Response::new(200, first_body), &ctx)
            .unwrap();
        assert_eq!(next.len(), 1);
        let second = &next[0];
        assert_eq!(second.url, "https://fake.test/second");
        assert_eq!(
            second.carry(),
            Some(r#"{"water_temperature":[[1793999000,14.5]]}"#)
        );
        assert!(!second.url.contains("14.5") && second.headers.is_empty());

        let mut out = Fragments::new();
        let second_body = br#"{"water_temperature":[[1793999600,14.6]]}"#;
        source
            .parse(second, &Response::new(200, second_body), &ctx, &mut out)
            .unwrap();
        let [fragment] = out.fragments() else {
            panic!("one fragment, got {:?}", out.fragments());
        };
        let FragmentValue::Series(series) = &fragment.value else {
            panic!("temperature is a series");
        };
        let values: Vec<(i64, f32)> = series
            .samples()
            .iter()
            .map(|s| (s.time.0, s.value))
            .collect();
        assert_eq!(values, [(1_793_999_000, 14.5), (1_793_999_600, 14.6)]);

        // A follow-up that does not carry gets nothing from the first response.
        let src = src.follows(
            "https://fake.test/first",
            FakeRequest::required("https://fake.test/other"),
        );
        let next = src
            .follow_up(&first, &Response::new(200, first_body), &ctx)
            .unwrap();
        assert_eq!(next[1].url, "https://fake.test/other");
        assert_eq!(next[1].carry(), None);
    }

    #[test]
    fn source_fake_plans_follows_and_parses() {
        let src = FakeSource::new(
            "fake-a",
            &[Measurement::WaterTemperature, Measurement::GageHeight],
        )
        .plans(FakeRequest::required("https://fake.test/a"))
        .plans(FakeRequest::optional("https://fake.test/b").metered())
        .follows(
            "https://fake.test/a",
            FakeRequest::required("https://fake.test/a?page=2"),
        )
        .with_budget(Budget {
            period: Period::Hour,
            limit: 10,
        });
        let source: &dyn Source = &src;
        let ctx = fake::ctx(NOW);
        let target = fake::target(
            "1",
            &[Measurement::WaterTemperature, Measurement::GageHeight],
            NOW,
        );
        let plan = source
            .plan(&target, &Requirements::default(), &ctx)
            .unwrap();
        assert_eq!(plan.len(), 2);
        assert!(plan[0].required && !plan[0].metered);
        assert!(!plan[1].required && plan[1].metered);
        assert_eq!(plan[0].target, target);
        assert_eq!(source.budget().map(|b| b.limit), Some(10));
        assert_eq!(source.max_follow_ups(), DEFAULT_MAX_FOLLOW_UPS);

        let body = br#"{"water_temperature":[[1793999000,14.5],[1793999600,14.6]],
                        "gage_height":[[1793999600,1.2]],"discharge":[[1793999600,3.0]]}"#;
        let resp = Response::new(200, body);
        let next = source.follow_up(&plan[0], &resp, &ctx).unwrap();
        assert_eq!(next.len(), 1);
        assert_eq!(next[0].url, "https://fake.test/a?page=2");
        assert_eq!(source.follow_up(&plan[1], &resp, &ctx).unwrap(), []);

        let mut out = Fragments::new();
        source.parse(&plan[0], &resp, &ctx, &mut out).unwrap();
        let measurements: Vec<Measurement> =
            out.fragments().iter().map(|f| f.measurement).collect();
        // Discharge was not wanted and is left out.
        assert_eq!(
            measurements,
            [Measurement::WaterTemperature, Measurement::GageHeight]
        );
        let FragmentValue::Series(temperature) = &out.fragments()[0].value else {
            panic!("temperature is a series");
        };
        assert_eq!(temperature.samples().len(), 2);
        assert_eq!(
            temperature.provenance().observed_at,
            UnixTime(1_793_999_600)
        );
        assert!(matches!(
            out.fragments()[1].value,
            FragmentValue::Heights(_)
        ));

        let err = source
            .parse(
                &plan[0],
                &Response::new(503, b""),
                &ctx,
                &mut Fragments::new(),
            )
            .unwrap_err();
        assert_eq!(err.kind(), SourceErrorKind::Other);

        let elsewhere = Target {
            place: Place::Point(ctx.round(Coordinates {
                latitude: 40.0,
                longitude: -73.0,
            })),
            measurements: BTreeSet::from([Measurement::Waves]),
            window: Window { from: NOW, to: NOW },
            datum: None,
        };
        assert!(matches!(
            source.plan(&elsewhere, &Requirements::default(), &ctx),
            Err(SourceError::Unsupported(_))
        ));
    }
}
