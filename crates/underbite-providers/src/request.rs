//! Requests a source plans and the responses the host hands back.
//!
//! A [`Request`] is everything the host needs to send one GET and everything the source needs to
//! parse the answer: sources keep no state, so the request echoes the station, bin, datum and
//! measurements in its [`Target`], and its [`Resource`] selects the parser.

use std::collections::BTreeSet;
use std::fmt;

use underbite_core::datum::VerticalDatum;
use underbite_core::model::{Measurement, Requirements, ZoneKind};
use underbite_core::provenance::{Relation, StationRef};
use underbite_core::time::UnixTime;

pub use whether_providers::url::{
    Coordinates, QueryBuilder, encode_path_segment, is_secret_header, redact_url, secret_values,
};

/// The most decimals of a coordinate sent to a data source: 2 places, about 1 km.
pub const MAX_COORDINATE_DECIMALS: u8 = 2;

/// Which resource of which source a request targets; selects the parser for its response.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Resource {
    /// CO-OPS data API `predictions` with `interval=hilo`: high and low tides.
    CoopsTideHighLow,
    /// CO-OPS data API `predictions` at 6 minutes: the tide curve.
    CoopsTideCurve,
    /// CO-OPS data API `water_level`: observed water level.
    CoopsWaterLevel,
    /// CO-OPS data API `water_temperature`.
    CoopsWaterTemperature,
    /// CO-OPS data API `air_temperature`.
    CoopsAirTemperature,
    /// CO-OPS data API `air_pressure`.
    CoopsAirPressure,
    /// CO-OPS data API `wind`.
    CoopsWind,
    /// CO-OPS data API `currents_predictions` with `interval=max_slack`: slack and maximum
    /// flood and ebb.
    CoopsCurrentEvents,
    /// CO-OPS data API `currents_predictions` at 6 minutes: the current curve.
    CoopsCurrentCurve,
    /// CO-OPS data API `currents`: observed currents.
    CoopsCurrents,
    /// CO-OPS metadata API station list (`stations.json?type=…`).
    CoopsStations,
    /// CO-OPS metadata API harmonic constituents (`harcon.json`).
    CoopsHarmonics,
    /// CO-OPS metadata API datums (`datums.json`).
    CoopsDatums,
    /// CO-OPS metadata API subordinate tide offsets (`tidepredoffsets.json`).
    CoopsTideOffsets,
    /// CO-OPS metadata API subordinate current offsets (`currentpredictionoffsets.json`).
    CoopsCurrentOffsets,
    /// CO-OPS metadata API current meter bins (`bins.json`).
    CoopsBins,
    /// USGS Water Data `latest-continuous`: the newest value per site and parameter.
    UsgsLatestContinuous,
    /// USGS Water Data `continuous`: instantaneous values over a window.
    UsgsContinuous,
    /// USGS Water Data `daily`: daily statistics.
    UsgsDaily,
    /// USGS Statistics API day-of-year normals (`observationNormals`).
    UsgsStatistics,
    /// USGS Water Data `monitoring-locations`.
    UsgsMonitoringLocations,
    /// USGS Water Data `time-series-metadata`.
    UsgsTimeSeriesMetadata,
    /// NDBC realtime standard meteorological file (`{id}.txt`).
    NdbcStandard,
    /// NDBC realtime spectral wave summary (`{id}.spec`).
    NdbcSpectralSummary,
    /// NDBC realtime oceanographic file (`{id}.ocean`).
    NdbcOcean,
    /// NDBC realtime current profile (`{id}.adcp`).
    NdbcAdcp,
    /// NDBC active station list (`activestations.xml`).
    NdbcActiveStations,
    /// Open-Meteo Marine forecast (`/v1/marine`).
    OpenMeteoMarine,
    /// NWS `/points/{lat},{lon}`: the point's office and zones.
    NwsPoint,
    /// NWS `/zones?point=` or `/zones/{type}?point=`: the zones containing a point.
    NwsZones,
    /// NWS `/zones/{type}/{id}`: one zone.
    NwsZone,
    /// NWS `/zones/{type}/{id}/forecast`: a zone's text forecast.
    NwsZoneForecast,
    /// NWS `/alerts/active?zone=`: active alerts for a zone.
    NwsAlerts,
    /// NWS `/products/types/{type}/locations/{location}/latest`: a marine text product.
    NwsProduct,
    /// NWPS `/gauges/{lid}`: a gauge with its flood categories.
    NwpsGauge,
    /// NWPS `/gauges/{lid}/stageflow/forecast`.
    NwpsStageFlowForecast,
    /// NWPS `/gauges?bbox…`: gauges in a box.
    NwpsGauges,
    /// A test source's resource.
    #[cfg(test)]
    Fake,
}

/// How a response body is checked before parsing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BodyFormat {
    /// JSON (including `GeoJSON`, JSON-LD and problem details); anything else is an error.
    Json,
    /// Text the parser reads itself (NDBC's column files and XML).
    Text,
}

impl Resource {
    /// The format the source answers in for this resource.
    pub fn format(self) -> BodyFormat {
        match self {
            Self::NdbcStandard
            | Self::NdbcSpectralSummary
            | Self::NdbcOcean
            | Self::NdbcAdcp
            | Self::NdbcActiveStations => BodyFormat::Text,
            _ => BodyFormat::Json,
        }
    }
}

/// A position rounded for sending to a data source.
///
/// Built only by [`RoundedPoint::new`], which rounds to at most [`MAX_COORDINATE_DECIMALS`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RoundedPoint {
    coordinates: Coordinates,
    decimals: u8,
}

impl RoundedPoint {
    /// `coordinates` rounded to `decimals` places, at most [`MAX_COORDINATE_DECIMALS`].
    pub fn new(coordinates: Coordinates, decimals: u8) -> Self {
        let decimals = decimals.min(MAX_COORDINATE_DECIMALS);
        Self {
            coordinates: coordinates.rounded(decimals),
            decimals,
        }
    }

    /// The rounded position.
    pub fn coordinates(self) -> Coordinates {
        self.coordinates
    }

    /// The number of decimals kept.
    pub fn decimals(self) -> u8 {
        self.decimals
    }

    /// `(latitude, longitude)` as query values without trailing zeros, e.g.
    /// `("40.7", "-74.02")`.
    pub fn fmt_param(self) -> (String, String) {
        self.coordinates.fmt_param(self.decimals)
    }
}

/// An NWS marine zone, as a zone lookup describes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ZoneTarget {
    /// The UGC zone id, e.g. `ANZ338`.
    pub id: String,
    /// The zone's name.
    pub name: String,
    /// Coastal, Great Lakes nearshore or open, or offshore.
    pub kind: ZoneKind,
    /// The forecast office, e.g. `OKX`.
    pub office: Option<String>,
}

/// Where a request's values come from.
#[derive(Clone, Debug, PartialEq)]
pub enum Place {
    /// A station, with how it relates to the spot (for provenance).
    Station {
        /// The station as its source lists it, with its bin for current meters.
        station: StationRef,
        /// Distance from the spot, metres.
        distance_m: Option<f32>,
        /// How the station relates to the spot.
        relation: Relation,
    },
    /// A rounded point, for gridded sources and lookups.
    Point(RoundedPoint),
    /// An NWS marine zone.
    Zone(ZoneTarget),
}

/// A span of time, both ends inclusive, UTC.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Window {
    /// Start.
    pub from: UnixTime,
    /// End.
    pub to: UnixTime,
}

impl Window {
    /// `requirements.past_hours` before `now` to `requirements.future_days` after it.
    pub fn around(now: UnixTime, requirements: &Requirements) -> Self {
        let past = i64::from(requirements.past_hours) * 3_600;
        let future = i64::from(requirements.future_days) * 86_400;
        Self {
            from: UnixTime(now.0.saturating_sub(past)),
            to: UnixTime(now.0.saturating_add(future)),
        }
    }
}

/// What a request is for: the place, the measurements wanted and the time window.
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    /// The station, rounded point or zone.
    pub place: Place,
    /// The measurements wanted from this place; one response may serve several.
    pub measurements: BTreeSet<Measurement>,
    /// The time window wanted.
    pub window: Window,
    /// The vertical datum heights are wanted in, for sources that take one (CO-OPS).
    pub datum: Option<VerticalDatum>,
}

/// How the host may cache a response.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct CacheHint {
    /// Whether the host must fetch a fresh copy, past any memory or disk cache
    /// (time-critical files).
    pub fresh: bool,
    /// Whether the host must not write the response to a disk cache. Always true for a
    /// request that carries a key (see [`Request::may_store_on_disk`]).
    pub no_store: bool,
    /// How long the host may reuse the response when the source sends no cache headers,
    /// seconds.
    pub max_age: Option<u32>,
}

/// An HTTP GET planned by a source.
#[derive(Clone, PartialEq)]
pub struct Request {
    /// Full URL including the query. May contain a key; log it only through [`redact_url`]
    /// (the `Debug` impl does).
    pub url: String,
    /// Extra request headers. Values of secret headers ([`is_secret_header`]) are never shown.
    pub headers: Vec<(&'static str, String)>,
    /// The resource, which selects the parser.
    pub resource: Resource,
    /// What the request is for; the parser reads the station, bin, datum and measurements
    /// here.
    pub target: Target,
    /// Whether the link fails without this response.
    pub required: bool,
    /// Whether the request counts against the source's budget.
    pub metered: bool,
    /// How the host may cache the response.
    pub cache: CacheHint,
}

impl Request {
    /// A required, unmetered GET of `url`, not to be stored on disk if the URL carries a key.
    pub fn new(url: String, resource: Resource, target: Target) -> Self {
        let no_store = !secret_values(&url).is_empty();
        Self {
            url,
            headers: Vec::new(),
            resource,
            target,
            required: true,
            metered: false,
            cache: CacheHint {
                no_store,
                ..CacheHint::default()
            },
        }
    }

    /// Adds a header; a secret header (a key) also marks the response not to be stored.
    #[must_use]
    pub fn header(mut self, name: &'static str, value: impl Into<String>) -> Self {
        self.cache.no_store |= is_secret_header(name);
        self.headers.push((name, value.into()));
        self
    }

    /// Marks the request optional: the link survives its failure.
    #[must_use]
    pub fn optional(mut self) -> Self {
        self.required = false;
        self
    }

    /// Marks the request as counting against the source's budget.
    #[must_use]
    pub fn metered(mut self) -> Self {
        self.metered = true;
        self
    }

    /// Asks the host for a fresh copy, past its caches.
    #[must_use]
    pub fn fresh(mut self) -> Self {
        self.cache.fresh = true;
        self
    }

    /// Sets how long the response may be reused when the source sends no cache headers.
    #[must_use]
    pub fn max_age_hint(mut self, seconds: u32) -> Self {
        self.cache.max_age = Some(seconds);
        self
    }

    /// Whether the URL or a header carries a key.
    pub fn carries_secret(&self) -> bool {
        !secret_values(&self.url).is_empty()
            || self.headers.iter().any(|(name, _)| is_secret_header(name))
    }

    /// Whether the host may write the response to a disk cache: never for a request that
    /// carries a key or says `no_store`.
    pub fn may_store_on_disk(&self) -> bool {
        !self.cache.no_store && !self.carries_secret()
    }
}

impl fmt::Debug for Request {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Request")
            .field("url", &redact_url(&self.url))
            .field("headers", &RedactedHeaders(&self.headers))
            .field("resource", &self.resource)
            .field("target", &self.target)
            .field("required", &self.required)
            .field("metered", &self.metered)
            .field("cache", &self.cache)
            .finish()
    }
}

/// `Debug` for request headers that masks the values of secret ones.
struct RedactedHeaders<'a>(&'a [(&'static str, String)]);

impl fmt::Debug for RedactedHeaders<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list()
            .entries(self.0.iter().map(|(name, value)| {
                (
                    *name,
                    if is_secret_header(name) {
                        "***"
                    } else {
                        value.as_str()
                    },
                )
            }))
            .finish()
    }
}

/// An HTTP response as the host received it.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Response<'a> {
    /// Status code.
    pub status: u16,
    /// Body bytes, decompressed and de-chunked.
    pub body: &'a [u8],
    /// The `Content-Type` header.
    pub content_type: Option<&'a str>,
    /// The `Date` header.
    pub date: Option<UnixTime>,
    /// The `Expires` header.
    pub expires: Option<UnixTime>,
    /// `max-age` of the `Cache-Control` header, seconds.
    pub cache_max_age: Option<u32>,
    /// The `Last-Modified` header.
    pub last_modified: Option<&'a str>,
    /// The `ETag` header.
    pub etag: Option<&'a str>,
    /// The `Retry-After` header, seconds from the response (the host converts a date).
    pub retry_after: Option<u32>,
    /// The `X-RateLimit-Remaining` header.
    pub rate_limit_remaining: Option<u32>,
}

impl<'a> Response<'a> {
    /// A response with just a status and body.
    pub fn new(status: u16, body: &'a [u8]) -> Self {
        Self {
            status,
            body,
            content_type: None,
            date: None,
            expires: None,
            cache_max_age: None,
            last_modified: None,
            etag: None,
            retry_after: None,
            rate_limit_remaining: None,
        }
    }
}

impl fmt::Debug for Response<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Response")
            .field("status", &self.status)
            .field("body_len", &self.body.len())
            .field("content_type", &self.content_type)
            .field("date", &self.date)
            .field("expires", &self.expires)
            .field("cache_max_age", &self.cache_max_age)
            .field("last_modified", &self.last_modified)
            .field("etag", &self.etag)
            .field("retry_after", &self.retry_after)
            .field("rate_limit_remaining", &self.rate_limit_remaining)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use underbite_core::provenance::{SourceId, StationKind};

    pub(crate) fn battery() -> StationRef {
        StationRef {
            source: SourceId::COOPS,
            id: "8518750".into(),
            name: "The Battery".into(),
            kind: StationKind::TideHarmonic,
            latitude: 40.7003,
            longitude: -74.0142,
            bin: None,
            reference_id: None,
        }
    }

    fn station_target() -> Target {
        Target {
            place: Place::Station {
                station: battery(),
                distance_m: Some(640.0),
                relation: Relation::SameWaterBody,
            },
            measurements: BTreeSet::from([Measurement::Discharge]),
            window: Window {
                from: UnixTime(1_794_000_000),
                to: UnixTime(1_794_086_400),
            },
            datum: None,
        }
    }

    #[test]
    fn request_debug_hides_key_param_and_header() {
        let url = QueryBuilder::new(
            "https://api.waterdata.usgs.gov/ogcapi/v1/collections/latest-continuous/items",
        )
        .param("monitoring_location_id", "USGS-01375000")
        .param("api_key", "SECRET-PARAM-123")
        .finish();
        let req = Request::new(url, Resource::UsgsLatestContinuous, station_target())
            .header("User-Agent", "underbite/0.1")
            .header("X-Api-Key", "SECRET-HEADER-456");
        let debug = format!("{req:?}");
        assert!(!debug.contains("SECRET-PARAM-123"), "{debug}");
        assert!(!debug.contains("SECRET-HEADER-456"), "{debug}");
        assert!(debug.contains("api_key=***"), "{debug}");
        assert!(debug.contains(r#"("X-Api-Key", "***")"#), "{debug}");
        assert!(debug.contains("underbite/0.1"), "{debug}");
        assert!(debug.contains("USGS-01375000"), "{debug}");
    }

    #[test]
    fn request_debug_snapshot() {
        let req = Request::new(
            "https://api.tidesandcurrents.noaa.gov/api/prod/datagetter?product=predictions&station=8518750&datum=MLLW&interval=hilo".into(),
            Resource::CoopsTideHighLow,
            Target {
                measurements: BTreeSet::from([Measurement::Tide]),
                datum: Some(VerticalDatum::Mllw),
                ..station_target()
            },
        )
        .optional()
        .max_age_hint(3_600);
        insta::assert_debug_snapshot!(req);
    }

    #[test]
    fn request_with_a_key_is_never_stored_on_disk() {
        let plain = Request::new(
            "https://api.example.test/a?site=1".into(),
            Resource::UsgsDaily,
            station_target(),
        );
        assert!(!plain.cache.no_store && !plain.carries_secret() && plain.may_store_on_disk());

        let in_url = Request::new(
            "https://api.example.test/a?site=1&api_key=K".into(),
            Resource::UsgsDaily,
            station_target(),
        );
        assert!(in_url.cache.no_store && !in_url.may_store_on_disk());

        let in_header = Request::new(
            "https://api.example.test/a".into(),
            Resource::UsgsDaily,
            station_target(),
        )
        .header("x-api-key", "K");
        assert!(in_header.cache.no_store && !in_header.may_store_on_disk());

        // A key added after construction still keeps the response off the disk.
        let mut late = plain.clone();
        late.url.push_str("&token=T");
        assert!(!late.cache.no_store && !late.may_store_on_disk());
    }

    #[test]
    fn request_builders_set_flags() {
        let req = Request::new(
            "https://api.weather.gov/alerts/active?zone=ANZ338".into(),
            Resource::NwsAlerts,
            station_target(),
        );
        assert!(req.required && !req.metered);
        assert_eq!(req.cache, CacheHint::default());
        let req = req.optional().metered().fresh().max_age_hint(60);
        assert!(!req.required && req.metered);
        assert_eq!(
            req.cache,
            CacheHint {
                fresh: true,
                no_store: false,
                max_age: Some(60),
            }
        );
    }

    #[test]
    fn request_point_is_rounded_to_two_decimals_at_most() {
        let battery = Coordinates {
            latitude: 40.703_3,
            longitude: -74.017,
        };
        let p = RoundedPoint::new(battery, 4);
        assert_eq!(p.decimals(), 2);
        assert_eq!(p.fmt_param(), ("40.7".to_string(), "-74.02".to_string()));
        assert!((p.coordinates().latitude - 40.70).abs() < 1e-9);
        let coarse = RoundedPoint::new(battery, 1);
        assert_eq!(coarse.fmt_param(), ("40.7".to_string(), "-74".to_string()));
    }

    #[test]
    fn request_window_spans_requirements() {
        let now = UnixTime(1_794_000_000);
        let w = Window::around(now, &Requirements::default());
        assert_eq!(w.from, UnixTime(now.0 - 7 * 86_400));
        assert_eq!(w.to, UnixTime(now.0 + 7 * 86_400));
    }

    #[test]
    fn request_resource_formats() {
        assert_eq!(Resource::NdbcStandard.format(), BodyFormat::Text);
        assert_eq!(Resource::NdbcActiveStations.format(), BodyFormat::Text);
        assert_eq!(Resource::CoopsTideHighLow.format(), BodyFormat::Json);
        assert_eq!(Resource::NwsZoneForecast.format(), BodyFormat::Json);
    }

    #[test]
    fn response_debug_shows_body_length_only() {
        let resp = Response {
            content_type: Some("application/json"),
            retry_after: Some(30),
            ..Response::new(200, br#"{"secret":"body"}"#)
        };
        let debug = format!("{resp:?}");
        assert!(
            debug.contains("body_len: 17") && !debug.contains("secret"),
            "{debug}"
        );
    }
}
