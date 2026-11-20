//! Nearby queries the user asks for on the map: access areas, species records and stocking
//! points, planned and parsed for the Swift host.
//!
//! Nothing here runs by itself: each `plan_*` function is called from the handler of a user's
//! tap and returns a [`NearbyPlan`]. The app sends the plan's [`NearbyPlan::requests`] through
//! its `URLSession` host, hands the answers to [`NearbyPlan::read`], and repeats while the plan
//! holds requests (a next page of public land, a retry after a busy answer); with none left,
//! [`NearbyPlan::results`] gives the same pins. No I/O happens here.
//!
//! - **Access** ([`plan_access`]): public land from PAD-US, and boat ramps, piers, fishing
//!   spots and (when asked) parking from OpenStreetMap through Overpass. Overpass cells are
//!   cached for 30 days and its daily cap and pauses kept in a state the app stores as JSON
//!   (`state_json` in, [`NearbyResults::state_json`] out).
//! - **Species records** ([`plan_records`]): one page of GBIF and iNaturalist records of a
//!   species in the view, and USGS NAS records of the state the app names, kept to the view.
//!   iNaturalist's pacing (a request a second, a daily cap per phone) and both services'
//!   pauses after HTTP 429 live in the same state; each request carries the delay to wait and
//!   the time its answer may be reused.
//! - **Stocking** ([`plan_stocking`]): the bundled catalogue's stocked waters in the view, and
//!   their New York stocking lists, each worded as the view's stocking card.
//!
//! A view wider than a source's limit is refused with a "zoom in" message rather than paged
//! through ([`NearbyError::ZoomIn`]). Boxes leave the phone rounded outward to the settings'
//! `sources.coordinate_precision` (at most 2 decimals); none of these services takes a point,
//! and stocking requests name only a county and its waters. No observer, identifier or
//! rights-holder name is read, so none crosses here; every record carries its licence, its
//! page and its platform's credit, and every result lists the credits of the sources it used.
//! Every word comes from the view's strings.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};

use serde::{Deserialize, Serialize};
use underbite_config::schema::Config;
use underbite_config::view_input::view_input;
use underbite_core::map::{Bbox, GeoPoint};
use underbite_core::model::nearby::{
    AccessFeature, AccessGeometry, AccessKind, AccessLevel, SpeciesRecord,
};
use underbite_core::model::{
    HabLevel, SourceError as ModelSourceError, SourceErrorKind, Spot, SpotSnapshot, StockingEvent,
    WaterType,
};
use underbite_core::provenance::{Attribution, SourceId};
use underbite_core::time::{LocalDate, UnixTime, UtcOffset};
use underbite_core::view::cards::stocking::{self, StockingCard};
use underbite_core::view::format::Formatter;
use underbite_core::view::strings::{
    Key, SafetyKey, record_link_title, record_position, record_seen,
};
use underbite_providers::access::{overpass, padus};
use underbite_providers::beach_forecast;
use underbite_providers::blooms::hab;
use underbite_providers::request::{Coordinates, MAX_COORDINATE_DECIMALS, Response};
use underbite_providers::species::{Licence, gbif, inat, nas, without_inaturalist_duplicates};
use underbite_providers::stocking::catalogue::{
    Catalogue, Kind as WaterKind, Placed, canonical_county, canonical_waterbody,
};
use underbite_providers::stocking::socrata_ny::{self, Dataset};

use crate::fetch::{Header, HostMethod, HostRequest, HostResponse, SourceIssue, fetched};
use crate::map::{MapPoint, formatter};
use crate::settings::{SettingsError, load};

mod beaches;
pub use beaches::{
    BeachCardInfo, BeachLegendItem, BeachPolygon, BeachRowInfo, BeachZonePin, plan_beach_forecast,
};

/// The widest view, in degrees of latitude or longitude, access is looked up for (PAD-US; the
/// Overpass part is limited to [`overpass::MAX_AREAS`] cells of 0.1 degree).
pub const ACCESS_MAX_SPAN_DEG: f64 = 0.5;
/// The most PAD-US pages read for one view.
pub const PADUS_MAX_PAGES: u32 = 4;
/// The widest view, degrees, species records are looked up for.
pub const RECORDS_MAX_SPAN_DEG: f64 = 1.0;
/// The most NAS pages read for one view (the service has no box search, so pages are of the
/// state or the hydrologic unit).
pub const NAS_MAX_PAGES: u32 = 4;
/// iNaturalist requests one phone sends in a UTC day: a hundredth of the budget the service
/// gives the app for all its users ([`inat::MAX_REQUESTS_PER_DAY`]).
pub const INATURALIST_DAILY_CAP: u32 = 100;
/// The widest view, degrees, stocked waters are looked up for.
pub const STOCKING_MAX_SPAN_DEG: f64 = 1.0;
/// The most stocked waters one view asks the lists about, nearest the view's centre first.
pub const STOCKING_MAX_WATERBODIES: usize = 50;

/// Overpass cells kept in the state, newest first.
const OVERPASS_CACHE_CELLS: usize = 64;
/// How long a PAD-US, NAS or stocking-list answer may be reused, seconds.
const DAY_S: u32 = 86_400;
const STATE_VERSION: u32 = 1;

// ---------------------------------------------------------------------------------------------
// What the app sends and receives
// ---------------------------------------------------------------------------------------------

/// Why a nearby lookup could not be planned.
#[derive(Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum NearbyError {
    /// The view, settings, state or query cannot be read.
    #[error("{message}")]
    Invalid {
        /// What is wrong.
        message: String,
    },
    /// The view is too wide for the lookup: show `message` ("Zoom in to look for species
    /// records") and ask nothing.
    #[error("{message}")]
    ZoomIn {
        /// The words to show.
        message: String,
    },
}

impl From<SettingsError> for NearbyError {
    fn from(e: SettingsError) -> Self {
        invalid(e.to_string())
    }
}

fn invalid(message: impl Into<String>) -> NearbyError {
    NearbyError::Invalid {
        message: message.into(),
    }
}

/// The visible map region, degrees. `west` greater than `east` crosses the antimeridian.
#[derive(Clone, Copy, Debug, PartialEq, uniffi::Record)]
pub struct NearbyArea {
    pub west: f64,
    pub south: f64,
    pub east: f64,
    pub north: f64,
}

/// What to look up in [`plan_records`].
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct RecordsQuery {
    /// The species' scientific name, e.g. `Morone saxatilis`.
    pub scientific_name: String,
    /// The page, from 1; the next one only when [`NearbyResults::more`] said so and the user
    /// asked for more.
    pub page: u32,
    /// Source ids to ask (`gbif`, `inaturalist`, `usgs-nas`); empty asks every one.
    pub sources: Vec<String>,
    /// The two-letter state whose NAS records to read, e.g. `NY`. NAS is asked only with a
    /// state or a hydrologic unit, and only for page 1.
    pub nas_state: Option<String>,
    /// The 8-digit hydrologic unit whose NAS records to read.
    pub nas_huc8: Option<String>,
}

/// One request of a [`NearbyPlan`].
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct NearbyRequest {
    /// Unique within the plan: hand the response back under it.
    pub id: String,
    /// The source's id (`padus`, `overpass`, `gbif`, `inaturalist`, `usgs-nas`,
    /// `ny-dec-stocking`).
    pub source: String,
    /// Requests of one queue are sent one at a time, in the order given; different queues may
    /// run side by side.
    pub queue: String,
    /// How long to wait before sending, milliseconds: from the moment the round is sent for the
    /// first request of a queue, from the end of the previous one for the others. It keeps
    /// iNaturalist's pace and Overpass's pause after a busy answer.
    pub delay_ms: u32,
    /// The request; its `max_age_s` is how long the answer may be reused.
    pub request: HostRequest,
}

/// The answer to one [`NearbyRequest`]. A request with no answer counts as failed.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct NearbyResponse {
    /// The request's id.
    pub id: String,
    /// What came back, any status.
    pub response: HostResponse,
}

/// The shape of an access feature.
#[derive(Clone, Debug, PartialEq, uniffi::Enum)]
pub enum AccessShape {
    /// A ramp, pier, fishing spot or parking area (ways arrive as their centre).
    Point,
    /// A line.
    Line { points: Vec<MapPoint> },
    /// Rings, the first the outer boundary; each ring closes implicitly.
    Polygon { rings: Vec<Vec<MapPoint>> },
}

/// A place to fish from, on the map.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct AccessPin {
    /// Stable for the same feature: source, kind and marker position.
    pub id: String,
    /// `padus` or `overpass`.
    pub source: String,
    /// The source's name: "USGS PAD-US", "OpenStreetMap".
    pub source_name: String,
    /// `ramp`, `pier`, `fishing_spot`, `parking` or `public_land`.
    pub kind: String,
    /// "Boat ramp", "Public land".
    pub kind_label: String,
    /// The feature's name as the source writes it, else the kind's label.
    pub title: String,
    /// The access the source records, `open`, `restricted`, `closed` or `unknown`: also the
    /// colour role.
    pub access: String,
    /// What that means: "Open to the public, as the source records it".
    pub access_label: String,
    /// Where to put the marker: the point, a line's middle vertex or the mean of a polygon's
    /// outer ring.
    pub lat: f64,
    pub lon: f64,
    pub shape: AccessShape,
    /// The pin read aloud.
    pub accessibility: String,
}

/// A record's photo, with its own licence.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct RecordPhoto {
    pub url: String,
    /// "CC BY".
    pub licence: String,
    /// "Photo: CC BY".
    pub caption: String,
}

/// Where a species was seen, on the map. It names no observer.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct RecordPin {
    /// `<source>:<record id>`.
    pub id: String,
    /// `gbif`, `inaturalist` or `usgs-nas`.
    pub source: String,
    /// "iNaturalist".
    pub source_name: String,
    /// The scientific name as the source writes it.
    pub taxon: String,
    /// The common name, when the source gives one.
    pub common_name: Option<String>,
    /// The common name, else the scientific one.
    pub title: String,
    pub lat: f64,
    pub lon: f64,
    /// The radius the position is good to, metres, for a circle; for a blurred record, the
    /// blurring cell's.
    pub radius_m: Option<f64>,
    /// The source blurred the position on purpose.
    pub obscured: bool,
    /// The record is of a species outside its native range: every USGS NAS record.
    pub introduced: bool,
    /// "Introduced species" for an introduced record, else `None`.
    pub introduced_label: Option<String>,
    /// "Seen Nov 3 2026", "Date not recorded".
    pub seen: String,
    /// "Position good to 98 ft", "Position blurred by the source, within 17 mi".
    pub position: Option<String>,
    /// The record's licence: "CC BY", "CC0", "Public domain".
    pub licence: String,
    /// The licence's page, when the source names one.
    pub licence_url: Option<String>,
    /// The record's page at the source: with the credit, the attribution its licence asks for.
    pub link: Option<String>,
    /// The words of the link to that page, "Record on iNaturalist"; `None` without a page.
    pub link_title: Option<String>,
    /// The platform's credit: "iNaturalist", "GBIF.org", or the USGS citation for NAS.
    pub credit: String,
    /// The platform's page.
    pub credit_url: Option<String>,
    pub photo: Option<RecordPhoto>,
    /// The pin read aloud.
    pub accessibility: String,
}

/// A stocked water on the map, with its stocking lists as the view's stocking card.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct StockingPin {
    /// `ny-dec-stocking:<county>:<waterbody>`.
    pub id: String,
    /// The waterbody as the state's lists write it.
    pub waterbody: String,
    pub county: String,
    /// The matched map feature's name.
    pub name: String,
    /// A representative point; for a river, a point on its line, not where fish were stocked.
    pub lat: f64,
    pub lon: f64,
    /// `lake`, `river` or `pond`.
    pub kind: String,
    /// The view's `StockingCard` as JSON, once a list answered; `None` before, or when both
    /// lists failed.
    pub card_json: Option<String>,
    /// The card's first line ("Brown Trout · Apr 2025 · about 8,920 · 7 in"), else the
    /// waterbody.
    pub label: String,
}

/// A source to credit beside the pins.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct NearbyCredit {
    /// The source's id.
    pub source: String,
    /// The source's name.
    pub name: String,
    /// The credit as the source asks for it.
    pub text: String,
    pub url: Option<String>,
    /// The data's licence, when one covers the whole source (`ODbL 1.0`, "Public domain");
    /// records carry their own.
    pub licence: Option<String>,
}

/// A beach the Gulf respiratory irritation forecast lists, in the view.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct BloomPin {
    /// Stable for the beach: source, name and place.
    pub id: String,
    /// The beach's name as NOAA writes it.
    pub name: String,
    pub lat: f64,
    pub lon: f64,
    /// `very_low`, `low`, `moderate`, `high` or `unknown`.
    pub level: String,
    /// The level in words ("Low"), as the bloom card words it.
    pub level_label: String,
    /// The bloom card's words: the level, with the confidence when the source gives one.
    pub detail: String,
    /// When the forecast is valid ("Nov 17, 3:00 AM"), in the beach's own time zone.
    pub forecast_for: String,
    /// The region's forecast page.
    pub link: Option<String>,
    /// What a screen reader says for the pin.
    pub accessibility: String,
}

/// What a plan has found so far.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct NearbyResults {
    pub access: Vec<AccessPin>,
    pub records: Vec<RecordPin>,
    pub stocking: Vec<StockingPin>,
    /// Beach bloom forecasts ([`plan_blooms`]).
    pub blooms: Vec<BloomPin>,
    /// Beach forecast zones with their outlines and cards ([`plan_beach_forecast`]).
    pub beach_zones: Vec<BeachZonePin>,
    /// The categories of the beach forecast zones' colours, in words.
    pub beach_legend: Vec<BeachLegendItem>,
    /// Every source the pins come from.
    pub credits: Vec<NearbyCredit>,
    /// What happened, in words: "OpenStreetMap is busy; try again later", "Zoom in to look
    /// for ramps, piers and parking from OpenStreetMap".
    pub notices: Vec<String>,
    /// Standing notes to show with the pins: what access levels mean, what records are, the
    /// NAS disclaimer.
    pub notes: Vec<String>,
    /// Requests that failed or were refused.
    pub issues: Vec<SourceIssue>,
    /// Another page of records can be asked for.
    pub more: bool,
    /// The lookup state to keep and pass to the next plan; `None` when the plan keeps none
    /// (stocking).
    pub state_json: Option<String>,
}

/// The words of the buttons that start the lookups, in the settings' voice.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct NearbyWords {
    pub find_access: String,
    pub finding_access: String,
    pub find_records: String,
    pub finding_records: String,
    pub find_blooms: String,
    pub finding_blooms: String,
    pub find_beaches: String,
    pub finding_beaches: String,
    /// In place of a lookup that is off while the view is wider than its limit, the words its
    /// plan refuses with ([`NearbyError::ZoomIn`]): "Zoom in to look for ramps, piers and
    /// public land", "Zoom in to look for species records", "Zoom in to look for stocked
    /// waters".
    pub zoom_in_access: String,
    pub zoom_in_records: String,
    pub zoom_in_stocking: String,
    pub zoom_in_beaches: String,
}

/// The words of the lookup buttons.
///
/// # Errors
/// [`NearbyError::Invalid`] for unreadable settings.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn nearby_words(settings_json: String) -> Result<NearbyWords, NearbyError> {
    let (config, _) = load(&settings_json)?;
    let f = formatter(&config, "UTC");
    let w = |key| f.strings.get(key).to_owned();
    Ok(NearbyWords {
        find_access: w(Key::NearbyFindAccess),
        finding_access: w(Key::NearbyFindingAccess),
        find_records: w(Key::NearbyFindRecords),
        finding_records: w(Key::NearbyFindingRecords),
        find_blooms: w(Key::NearbyFindBlooms),
        finding_blooms: w(Key::NearbyFindingBlooms),
        find_beaches: w(Key::NearbyFindBeaches),
        finding_beaches: w(Key::NearbyFindingBeaches),
        zoom_in_access: w(Key::NearbyZoomInAccess),
        zoom_in_records: w(Key::NearbyZoomInRecords),
        zoom_in_stocking: w(Key::NearbyZoomInStocking),
        zoom_in_beaches: w(Key::NearbyZoomInBeaches),
    })
}

/// The limits the plans keep, for the app's own caching and messages.
#[derive(Clone, Copy, Debug, PartialEq, uniffi::Record)]
pub struct NearbyLimits {
    pub access_max_span_deg: f64,
    pub records_max_span_deg: f64,
    pub stocking_max_span_deg: f64,
    pub beach_max_span_deg: f64,
    pub overpass_max_cells: u32,
    pub overpass_daily_cap: u32,
    pub overpass_cache_days: u32,
    pub inaturalist_daily_cap: u32,
    pub inaturalist_min_interval_ms: u32,
    pub inaturalist_max_age_s: u32,
    pub gbif_max_age_s: u32,
}

/// The limits the plans keep.
#[uniffi::export]
pub fn nearby_limits() -> NearbyLimits {
    NearbyLimits {
        access_max_span_deg: ACCESS_MAX_SPAN_DEG,
        records_max_span_deg: RECORDS_MAX_SPAN_DEG,
        stocking_max_span_deg: STOCKING_MAX_SPAN_DEG,
        beach_max_span_deg: beach_forecast::MAX_BOX_SPAN_DEG,
        overpass_max_cells: u32::try_from(overpass::MAX_AREAS).unwrap_or(u32::MAX),
        overpass_daily_cap: overpass::DEFAULT_DAILY_CAP,
        overpass_cache_days: u32::try_from(overpass::CACHE_DAYS).unwrap_or(u32::MAX),
        inaturalist_daily_cap: INATURALIST_DAILY_CAP,
        inaturalist_min_interval_ms: inat::MIN_INTERVAL_MS,
        inaturalist_max_age_s: inat::MAX_AGE_S,
        gbif_max_age_s: gbif::MAX_AGE_S,
    }
}

// ---------------------------------------------------------------------------------------------
// The view
// ---------------------------------------------------------------------------------------------

/// A checked view, split at the antimeridian.
#[derive(Clone, Debug)]
struct Area {
    west: f64,
    south: f64,
    east: f64,
    north: f64,
}

impl Area {
    fn new(a: NearbyArea) -> Result<Self, NearbyError> {
        let lat = |v: f64| v.is_finite() && (-90.0..=90.0).contains(&v);
        let lon = |v: f64| v.is_finite() && (-180.0..=180.0).contains(&v);
        if !(lat(a.south) && lat(a.north) && lon(a.west) && lon(a.east)) || a.south > a.north {
            return Err(invalid("a view needs edges in range, south of its north"));
        }
        Ok(Self {
            west: a.west,
            south: a.south,
            east: a.east,
            north: a.north,
        })
    }

    fn width(&self) -> f64 {
        if self.west <= self.east {
            self.east - self.west
        } else {
            self.east - self.west + 360.0
        }
    }

    fn fits(&self, max_deg: f64) -> bool {
        self.width() <= max_deg && self.north - self.south <= max_deg
    }

    /// The boxes the view spans: two when it crosses the antimeridian.
    fn parts(&self) -> Vec<Bbox> {
        let part = |west, east| Bbox {
            west,
            south: self.south,
            east,
            north: self.north,
        };
        if self.west <= self.east {
            vec![part(self.west, self.east)]
        } else {
            vec![part(self.west, 180.0), part(-180.0, self.east)]
        }
    }

    /// [`Self::parts`] rounded outward to `decimals` (at most 2).
    fn rounded_parts(&self, decimals: u8) -> Vec<Bbox> {
        self.parts()
            .into_iter()
            .map(|b| outward(b, decimals))
            .collect()
    }

    fn contains(&self, lat: f64, lon: f64) -> bool {
        let across = if self.west <= self.east {
            (self.west..=self.east).contains(&lon)
        } else {
            lon >= self.west || lon <= self.east
        };
        (self.south..=self.north).contains(&lat) && across
    }

    fn centre(&self) -> (f64, f64) {
        let lon = self.west + self.width() / 2.0;
        let lon = if lon > 180.0 { lon - 360.0 } else { lon };
        (f64::midpoint(self.south, self.north), lon)
    }

    /// How far a point is from the centre, for ordering only (squared degrees).
    fn off_centre(&self, lat: f64, lon: f64) -> f64 {
        let (clat, clon) = self.centre();
        let dlon = (lon - clon + 540.0).rem_euclid(360.0) - 180.0;
        (lat - clat).powi(2) + dlon.powi(2)
    }
}

/// A box rounded outward to `decimals` (at most [`MAX_COORDINATE_DECIMALS`]) so it still covers
/// the view, inside the world.
fn outward(b: Bbox, decimals: u8) -> Bbox {
    let scale = 10f64.powi(i32::from(decimals.min(MAX_COORDINATE_DECIMALS)));
    // A value already on the grid stays put (`-74.4 * 100` is `-7440.000000000001`).
    let down = |v: f64| (v * scale + 1e-6).floor() / scale;
    let up = |v: f64| (v * scale - 1e-6).ceil() / scale;
    Bbox {
        west: down(b.west).max(-180.0),
        south: down(b.south).max(-90.0),
        east: up(b.east).min(180.0),
        north: up(b.north).min(90.0),
    }
}

// ---------------------------------------------------------------------------------------------
// The state the app keeps
// ---------------------------------------------------------------------------------------------

/// One Overpass cell's answer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct CachedCell {
    fetched_at: i64,
    #[serde(default)]
    truncated: bool,
    #[serde(default)]
    features: Vec<AccessFeature>,
}

/// The pace of a service: a daily count, the earliest next request and a pause.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
struct Pacer {
    day: i64,
    used: u32,
    next_at_ms: i64,
    pause_until: i64,
}

/// Why a request may not be sent now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Refusal {
    DailyCap,
    Paused { wait_s: u32 },
}

impl From<overpass::Refused> for Refusal {
    fn from(r: overpass::Refused) -> Self {
        match r {
            overpass::Refused::DailyCap => Self::DailyCap,
            overpass::Refused::Paused { wait_secs } => Self::Paused { wait_s: wait_secs },
        }
    }
}

impl Pacer {
    /// Counts a request to be sent now or later and gives the delay before it, milliseconds:
    /// `interval_ms` after the previous one.
    fn begin(&mut self, now: UnixTime, cap: Option<u32>, interval_ms: u32) -> Result<u32, Refusal> {
        if now.0 < self.pause_until {
            let wait = self.pause_until.saturating_sub(now.0);
            return Err(Refusal::Paused {
                wait_s: u32::try_from(wait).unwrap_or(u32::MAX),
            });
        }
        let today = now.0.div_euclid(i64::from(DAY_S));
        if self.day != today {
            self.day = today;
            self.used = 0;
        }
        if cap.is_some_and(|cap| self.used >= cap) {
            return Err(Refusal::DailyCap);
        }
        let now_ms = now.0.saturating_mul(1_000);
        let at = self.next_at_ms.max(now_ms);
        self.next_at_ms = at.saturating_add(i64::from(interval_ms));
        self.used = self.used.saturating_add(1);
        Ok(u32::try_from(at - now_ms).unwrap_or(u32::MAX))
    }

    fn pause(&mut self, now: UnixTime, secs: u32) {
        self.pause_until = self.pause_until.max(now.0.saturating_add(i64::from(secs)));
    }
}

/// Everything the lookups keep between plans.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
struct NearbyState {
    version: u32,
    overpass_gate: overpass::RequestGate,
    /// By [`overpass::CacheKey::id`].
    overpass_cache: BTreeMap<String, CachedCell>,
    inaturalist: Pacer,
    gbif: Pacer,
    /// The beach forecast service's layer list.
    beach_layers: Option<beaches::LayerCache>,
    /// The forecast office of a rounded point, by `lat,lon`.
    beach_offices: BTreeMap<String, beaches::OfficeCache>,
}

impl Default for NearbyState {
    fn default() -> Self {
        Self {
            version: STATE_VERSION,
            overpass_gate: overpass::RequestGate::default(),
            overpass_cache: BTreeMap::new(),
            inaturalist: Pacer::default(),
            gbif: Pacer::default(),
            beach_layers: None,
            beach_offices: BTreeMap::new(),
        }
    }
}

impl NearbyState {
    /// The state in `json`; empty text is a fresh state.
    fn read(json: &str) -> Result<Self, NearbyError> {
        if json.trim().is_empty() {
            return Ok(Self::default());
        }
        serde_json::from_str(json).map_err(|e| invalid(format!("the nearby state: {e}")))
    }

    /// Drops cells that expired at `now` and keeps the newest [`OVERPASS_CACHE_CELLS`].
    fn prune(&mut self, now: UnixTime) {
        self.prune_beaches(now);
        self.overpass_cache
            .retain(|_, c| overpass::cache_valid(UnixTime(c.fetched_at), now));
        if self.overpass_cache.len() > OVERPASS_CACHE_CELLS {
            let mut ages: Vec<(i64, String)> = self
                .overpass_cache
                .iter()
                .map(|(k, c)| (c.fetched_at, k.clone()))
                .collect();
            ages.sort_by(|a, b| b.cmp(a));
            for (_, key) in ages.into_iter().skip(OVERPASS_CACHE_CELLS) {
                self.overpass_cache.remove(&key);
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The plan
// ---------------------------------------------------------------------------------------------

/// What a request asks for.
#[derive(Clone, Debug)]
enum Want {
    Padus {
        bbox: Bbox,
        offset: u32,
        page: u32,
    },
    Overpass {
        key: overpass::CacheKey,
    },
    Gbif,
    Inat,
    Nas {
        offset: u32,
        page: u32,
    },
    Stocking {
        dataset: Dataset,
        points: Vec<usize>,
    },
    Blooms,
    /// The beach forecast service's layer list.
    BeachLayers,
    /// The forecast office of the rounded point `key`.
    BeachOffice {
        key: String,
    },
    /// The zones of an office in the view.
    BeachZones,
}

#[derive(Clone, Debug)]
struct Pending {
    id: String,
    source: SourceId,
    queue: &'static str,
    delay_ms: u32,
    want: Want,
    request: HostRequest,
}

/// A stocked water of the view.
#[derive(Clone, Debug)]
struct Point {
    placed: Placed,
    key: (String, String),
    events: Vec<StockingEvent>,
    answered: bool,
}

/// What the plan holds between rounds.
struct Inner {
    config: Config,
    f: Formatter,
    now: UnixTime,
    area: Area,
    /// Whether the plan keeps the state (access and records do).
    stateful: bool,
    state: NearbyState,
    round: Vec<Pending>,
    seq: u32,
    access: Vec<AccessFeature>,
    inat: Vec<inat::InatRecord>,
    gbif: Vec<gbif::GbifRecord>,
    nas: Vec<nas::NasRecord>,
    nas_query: Option<nas::NasQuery>,
    points: Vec<Point>,
    blooms: Vec<hab::HabBeach>,
    /// The beach forecast lookup, when this plan is one.
    beach: Option<beaches::Run>,
    beach_zones: Vec<beach_forecast::BeachZone>,
    /// When the oldest Overpass cell answered from the state was fetched.
    cached_oldest: Option<i64>,
    used: BTreeSet<SourceId>,
    notices: Vec<String>,
    notes: Vec<String>,
    issues: Vec<ModelSourceError>,
    more: bool,
}

/// A nearby lookup, kept on the Rust side: send [`Self::requests`], give the answers to
/// [`Self::read`], and repeat while it holds requests.
#[derive(uniffi::Object)]
pub struct NearbyPlan {
    inner: Mutex<Inner>,
}

impl fmt::Debug for NearbyPlan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        f.debug_struct("NearbyPlan")
            .field("requests", &inner.round.len())
            .field("access", &inner.access.len())
            .field("points", &inner.points.len())
            .finish_non_exhaustive()
    }
}

pub(crate) fn get(url: String, user_agent: Option<&str>, max_age_s: u32) -> HostRequest {
    HostRequest {
        url,
        headers: user_agent
            .map(|ua| Header {
                name: "User-Agent".to_owned(),
                value: ua.to_owned(),
            })
            .into_iter()
            .collect(),
        method: HostMethod::Get,
        body: None,
        fresh: false,
        no_store: false,
        max_age_s: Some(max_age_s),
    }
}

fn push_unique<T: PartialEq>(list: &mut Vec<T>, item: T) {
    if !list.contains(&item) {
        list.push(item);
    }
}

impl Inner {
    fn new(config: Config, area: Area, state: NearbyState, stateful: bool, now: UnixTime) -> Self {
        Self {
            f: formatter(&config, "UTC"),
            config,
            now,
            area,
            stateful,
            state,
            round: Vec::new(),
            seq: 0,
            access: Vec::new(),
            inat: Vec::new(),
            gbif: Vec::new(),
            nas: Vec::new(),
            nas_query: None,
            points: Vec::new(),
            blooms: Vec::new(),
            beach: None,
            beach_zones: Vec::new(),
            cached_oldest: None,
            used: BTreeSet::new(),
            notices: Vec::new(),
            notes: Vec::new(),
            issues: Vec::new(),
            more: false,
        }
    }

    fn decimals(&self) -> u8 {
        self.config.sources.coordinate_precision
    }

    fn push(
        &mut self,
        source: SourceId,
        queue: &'static str,
        delay_ms: u32,
        want: Want,
        request: HostRequest,
    ) {
        let p = self.make(source, queue, delay_ms, want, request);
        self.round.push(p);
    }

    /// A request for the round, which the caller places.
    fn make(
        &mut self,
        source: SourceId,
        queue: &'static str,
        delay_ms: u32,
        want: Want,
        request: HostRequest,
    ) -> Pending {
        self.seq += 1;
        self.used.insert(source.clone());
        Pending {
            id: format!("{}:{}", self.seq, source.as_str()),
            source,
            queue,
            delay_ms,
            want,
            request,
        }
    }

    fn name(&self, source: &SourceId) -> String {
        self.f.strings.source_name(source)
    }

    fn notice(&mut self, key: Key, source: &SourceId) {
        let name = self.name(source);
        let text = self.f.strings.fmt(key, &[("source", &name)]);
        push_unique(&mut self.notices, text);
    }

    fn refused(&mut self, source: &SourceId, refusal: Refusal) {
        match refusal {
            Refusal::DailyCap => self.notice(Key::NearbyDailyCap, source),
            Refusal::Paused { wait_s } => {
                // Whole minutes, rounded up, so a short pause never reads "0 min".
                let wait = i64::from(wait_s.div_ceil(60).max(1)) * 60;
                let name = self.name(source);
                let text = self.f.strings.fmt(
                    Key::NearbyPaused,
                    &[("source", &name), ("wait", &self.f.duration(wait))],
                );
                push_unique(&mut self.notices, text);
            }
        }
    }

    fn issue(&mut self, source: &SourceId, kind: SourceErrorKind, message: String) {
        push_unique(
            &mut self.issues,
            ModelSourceError {
                source: source.clone(),
                measurement: None,
                kind,
                message,
            },
        );
    }

    fn note(&mut self, text: String) {
        push_unique(&mut self.notes, text);
    }
}

// ---------------------------------------------------------------------------------------------
// Access
// ---------------------------------------------------------------------------------------------

/// The Overpass cells of the view, nearest its centre first; `None` above
/// [`overpass::MAX_AREAS`].
fn overpass_cells(area: &Area) -> Option<Vec<overpass::AreaKey>> {
    let mut cells = Vec::new();
    for part in area.parts() {
        cells.extend(overpass::area_keys(part).ok()?);
    }
    cells.sort();
    cells.dedup();
    if cells.len() > overpass::MAX_AREAS {
        return None;
    }
    let off = |k: &overpass::AreaKey| {
        let b = k.bbox();
        area.off_centre(
            f64::midpoint(b.south, b.north),
            f64::midpoint(b.west, b.east),
        )
    };
    cells.sort_by(|a, b| off(a).total_cmp(&off(b)).then(a.cmp(b)));
    Some(cells)
}

/// The PAD-US simplification for a view: about a thousandth of its width.
fn padus_tolerance(area: &Area) -> f64 {
    (area.width() / 1_000.0).clamp(0.000_05, 0.005)
}

/// Plans the access lookup for the view at Unix time `now`: PAD-US public land (up to
/// [`PADUS_MAX_PAGES`] pages) and, in a view of at most [`overpass::MAX_AREAS`] Overpass cells,
/// OpenStreetMap ramps, piers and fishing spots, and parking when `include_parking`. Cells the
/// state holds from the last 30 days are answered from it; the others are counted against the
/// phone's daily Overpass cap, nearest the view's centre first, and refused with a notice once
/// it is spent or while a pause runs.
///
/// Call it only from the handler of a user's tap. `state_json` is the last
/// [`NearbyResults::state_json`] (empty for none).
///
/// # Errors
/// [`NearbyError::ZoomIn`] for a view wider than [`ACCESS_MAX_SPAN_DEG`];
/// [`NearbyError::Invalid`] for a view out of range or unreadable settings or state.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn plan_access(
    area: NearbyArea,
    include_parking: bool,
    state_json: String,
    settings_json: String,
    now: i64,
) -> Result<Arc<NearbyPlan>, NearbyError> {
    let (config, _) = load(&settings_json)?;
    let area = Area::new(area)?;
    let mut state = NearbyState::read(&state_json)?;
    let now = UnixTime(now);
    state.prune(now);
    let mut inner = Inner::new(config, area, state, true, now);
    if !inner.area.fits(ACCESS_MAX_SPAN_DEG) {
        return Err(NearbyError::ZoomIn {
            message: inner.f.strings.get(Key::NearbyZoomInAccess).to_owned(),
        });
    }

    let tolerance = padus_tolerance(&inner.area);
    for bbox in inner.area.rounded_parts(inner.decimals()) {
        match padus::query_request(bbox, 0, tolerance) {
            Ok(r) => inner.push(
                SourceId::PADUS,
                "padus",
                0,
                Want::Padus {
                    bbox,
                    offset: 0,
                    page: 1,
                },
                get(r.url, None, DAY_S),
            ),
            Err(e) => inner.issue(&SourceId::PADUS, SourceErrorKind::Other, e.to_string()),
        }
    }

    if let Some(cells) = overpass_cells(&inner.area) {
        plan_overpass(&mut inner, &cells, include_parking);
    } else {
        let text = inner.f.strings.get(Key::NearbyZoomInOsm).to_owned();
        push_unique(&mut inner.notices, text);
    }
    let note = inner
        .f
        .strings
        .safety(SafetyKey::AccessNotPermission)
        .to_owned();
    inner.note(note);
    Ok(Arc::new(NearbyPlan {
        inner: Mutex::new(inner),
    }))
}

fn plan_overpass(inner: &mut Inner, cells: &[overpass::AreaKey], include_parking: bool) {
    let mut kinds = vec![overpass::QueryKind::Places];
    if include_parking {
        kinds.push(overpass::QueryKind::Parking);
    }
    let mut refused = false;
    for &area in cells {
        for &kind in &kinds {
            let key = overpass::CacheKey { area, kind };
            if let Some(cell) = inner.state.overpass_cache.get(&key.id()).cloned() {
                inner.used.insert(SourceId::OVERPASS);
                inner.cached_oldest = Some(
                    inner
                        .cached_oldest
                        .map_or(cell.fetched_at, |t| t.min(cell.fetched_at)),
                );
                inner.access.extend(cell.features);
                if cell.truncated {
                    inner.notice(Key::NearbyMoreThanShown, &SourceId::OVERPASS);
                }
                continue;
            }
            if refused {
                continue;
            }
            if let Err(r) = inner
                .state
                .overpass_gate
                .begin(inner.now, overpass::UserAction::requested())
            {
                refused = true;
                inner.refused(&SourceId::OVERPASS, r.into());
                continue;
            }
            match overpass::query_request(overpass::DEFAULT_ENDPOINT, key) {
                Ok(r) => inner.push(
                    SourceId::OVERPASS,
                    "overpass",
                    0,
                    Want::Overpass { key },
                    get(
                        r.url,
                        Some(r.user_agent),
                        u32::try_from(overpass::CACHE_DAYS).unwrap_or(1) * DAY_S,
                    ),
                ),
                Err(e) => inner.issue(&SourceId::OVERPASS, SourceErrorKind::Other, e.to_string()),
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Species records
// ---------------------------------------------------------------------------------------------

const RECORD_SOURCES: [SourceId; 3] = [SourceId::INATURALIST, SourceId::GBIF, SourceId::USGS_NAS];

/// The NAS taxon of a scientific name: its genus and epithet, or a genus alone.
fn nas_taxon(name: &str) -> nas::NasTaxon {
    let mut words = name.split_whitespace();
    match (words.next(), words.next()) {
        (Some(genus), Some(species)) => nas::NasTaxon::Species {
            genus: genus.to_owned(),
            species: species.to_owned(),
        },
        (Some(genus), None) => nas::NasTaxon::Genus(genus.to_owned()),
        _ => nas::NasTaxon::Any,
    }
}

/// Plans one page of species records of `query.scientific_name` in the view at Unix time `now`:
/// GBIF and iNaturalist records in the view's box, and on page 1 the NAS records of the state or
/// hydrologic unit named, kept to the view (up to [`NAS_MAX_PAGES`] pages). iNaturalist is
/// paced (one request a second, [`INATURALIST_DAILY_CAP`] a day on this phone) and both it and
/// GBIF pause after HTTP 429; a source that may not be asked now gets a notice instead.
///
/// Call it only from the handler of a user's tap (opening the layer, asking for more).
/// `state_json` is the last [`NearbyResults::state_json`] (empty for none).
///
/// # Errors
/// [`NearbyError::ZoomIn`] for a view wider than [`RECORDS_MAX_SPAN_DEG`];
/// [`NearbyError::Invalid`] for an empty name, an unknown source, a state that is not two
/// letters or a unit that is not eight digits, a view out of range or unreadable settings or
/// state.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn plan_records(
    area: NearbyArea,
    query: RecordsQuery,
    state_json: String,
    settings_json: String,
    now: i64,
) -> Result<Arc<NearbyPlan>, NearbyError> {
    let (config, _) = load(&settings_json)?;
    let area = Area::new(area)?;
    let name = query.scientific_name.trim().to_owned();
    if name.is_empty() {
        return Err(invalid("a species needs its scientific name"));
    }
    let known = RECORD_SOURCES;
    let mut wanted: BTreeSet<SourceId> = BTreeSet::new();
    for id in &query.sources {
        let source = known
            .iter()
            .find(|s| s.as_str() == id.trim())
            .ok_or_else(|| invalid(format!("`{id}` is not a source of species records")))?;
        wanted.insert(source.clone());
    }
    if wanted.is_empty() {
        wanted.extend(known);
    }
    let state = NearbyState::read(&state_json)?;
    let now = UnixTime(now);
    let mut inner = Inner::new(config, area, state, true, now);
    if !inner.area.fits(RECORDS_MAX_SPAN_DEG) {
        return Err(NearbyError::ZoomIn {
            message: inner.f.strings.get(Key::NearbyZoomInRecords).to_owned(),
        });
    }
    let page = query.page.max(1);
    let parts = inner.area.rounded_parts(inner.decimals());

    if wanted.contains(&SourceId::INATURALIST) {
        plan_inat(&mut inner, &name, &parts, page);
    }
    if wanted.contains(&SourceId::GBIF) {
        plan_gbif(&mut inner, &name, &parts, page);
    }
    if wanted.contains(&SourceId::USGS_NAS)
        && page == 1
        && (query.nas_state.is_some() || query.nas_huc8.is_some())
    {
        let q = nas::NasQuery {
            state: query.nas_state.clone(),
            huc8: query.nas_huc8.clone(),
            taxon: nas_taxon(&name),
            years: None,
        };
        let r = nas::search_request(&q, 0, nas::PAGE_SIZE).map_err(|e| invalid(e.to_string()))?;
        inner.nas_query = Some(q);
        inner.push(
            SourceId::USGS_NAS,
            "usgs-nas",
            0,
            Want::Nas { offset: 0, page: 1 },
            get(r.url, None, DAY_S),
        );
        inner.note(nas::DISCLAIMER.to_owned());
    }
    let note = inner.f.strings.get(Key::RecordsWhereSeen).to_owned();
    inner.note(note);
    Ok(Arc::new(NearbyPlan {
        inner: Mutex::new(inner),
    }))
}

fn plan_inat(inner: &mut Inner, name: &str, parts: &[Bbox], page: u32) {
    for &bbox in parts {
        let Ok(r) = inat::observations_request(name, bbox, page, inat::PAGE_SIZE) else {
            // A page past the service's window: there is nothing more to ask for.
            continue;
        };
        match inner.state.inaturalist.begin(
            inner.now,
            Some(INATURALIST_DAILY_CAP),
            inat::MIN_INTERVAL_MS,
        ) {
            Ok(delay_ms) => inner.push(
                SourceId::INATURALIST,
                "inaturalist",
                delay_ms,
                Want::Inat,
                get(r.url, Some(r.user_agent.as_str()), r.max_age_s),
            ),
            Err(refusal) => {
                inner.refused(&SourceId::INATURALIST, refusal);
                return;
            }
        }
    }
}

fn plan_gbif(inner: &mut Inner, name: &str, parts: &[Bbox], page: u32) {
    let offset = (page - 1).saturating_mul(gbif::PAGE_SIZE);
    for &bbox in parts {
        let Ok(r) = gbif::occurrence_request(name, bbox, offset, gbif::PAGE_SIZE) else {
            continue;
        };
        match inner.state.gbif.begin(inner.now, None, 0) {
            Ok(delay_ms) => inner.push(
                SourceId::GBIF,
                "gbif",
                delay_ms,
                Want::Gbif,
                get(r.url, Some(r.user_agent.as_str()), r.max_age_s),
            ),
            Err(refusal) => {
                inner.refused(&SourceId::GBIF, refusal);
                return;
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Stocking
// ---------------------------------------------------------------------------------------------

/// Plans the stocking lookup for the view at Unix time `now`: the bundled catalogue's stocked
/// waters in it (at most [`STOCKING_MAX_WATERBODIES`], nearest its centre first), and the New
/// York lists of what was stocked there and what is planned. The pins are there before any
/// answer, without a card.
///
/// Call it only from the handler of a user's tap.
///
/// # Errors
/// [`NearbyError::ZoomIn`] for a view wider than [`STOCKING_MAX_SPAN_DEG`];
/// [`NearbyError::Invalid`] for a view out of range or unreadable settings.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn plan_stocking(
    area: NearbyArea,
    settings_json: String,
    now: i64,
) -> Result<Arc<NearbyPlan>, NearbyError> {
    let (config, _) = load(&settings_json)?;
    let area = Area::new(area)?;
    let now = UnixTime(now);
    let mut inner = Inner::new(config, area, NearbyState::default(), false, now);
    if !inner.area.fits(STOCKING_MAX_SPAN_DEG) {
        return Err(NearbyError::ZoomIn {
            message: inner.f.strings.get(Key::NearbyZoomInStocking).to_owned(),
        });
    }
    let catalogue = Catalogue::bundled().map_err(|e| invalid(e.to_string()))?;
    let mut placed: Vec<&Placed> = catalogue
        .file()
        .placed
        .iter()
        .filter(|p| inner.area.contains(p.latitude, p.longitude))
        .collect();
    placed.sort_by(|a, b| {
        inner
            .area
            .off_centre(a.latitude, a.longitude)
            .total_cmp(&inner.area.off_centre(b.latitude, b.longitude))
            .then_with(|| a.county.cmp(&b.county))
            .then_with(|| a.waterbody.cmp(&b.waterbody))
    });
    if placed.len() > STOCKING_MAX_WATERBODIES {
        placed.truncate(STOCKING_MAX_WATERBODIES);
        inner.notice(Key::NearbyMoreThanShown, &SourceId::NY_DEC_STOCKING);
    }
    inner.points = placed
        .into_iter()
        .map(|p| Point {
            key: (
                canonical_county(&p.county),
                canonical_waterbody(&p.waterbody),
            ),
            placed: p.clone(),
            events: Vec::new(),
            answered: false,
        })
        .collect();
    if inner.points.is_empty() {
        let text = inner.f.strings.get(Key::StockingNone).to_owned();
        push_unique(&mut inner.notices, text);
    } else {
        inner.used.insert(SourceId::NY_DEC_STOCKING);
    }

    // By county, in the order of each county's nearest water, a list request per
    // `MAX_WATERBODIES` waters.
    let mut counties: Vec<(String, Vec<usize>)> = Vec::new();
    for (i, p) in inner.points.iter().enumerate() {
        match counties.iter_mut().find(|(c, _)| *c == p.key.0) {
            Some((_, points)) => points.push(i),
            None => counties.push((p.key.0.clone(), vec![i])),
        }
    }
    for (county, points) in counties {
        for chunk in points.chunks(socrata_ny::MAX_WATERBODIES) {
            let requests = {
                let names: Vec<&str> = chunk
                    .iter()
                    .map(|&i| inner.points[i].placed.waterbody.as_str())
                    .collect();
                socrata_ny::requests(&county, &names)
            };
            for r in requests {
                let mut request = get(r.url, None, DAY_S);
                request.headers = r
                    .headers
                    .iter()
                    .map(|(name, value)| Header {
                        name: (*name).to_owned(),
                        value: value.clone(),
                    })
                    .collect();
                inner.push(
                    SourceId::NY_DEC_STOCKING,
                    "ny-dec-stocking",
                    0,
                    Want::Stocking {
                        dataset: r.dataset,
                        points: chunk.to_vec(),
                    },
                    request,
                );
            }
        }
    }
    Ok(Arc::new(NearbyPlan {
        inner: Mutex::new(inner),
    }))
}

// ---------------------------------------------------------------------------------------------
// Beach bloom forecasts
// ---------------------------------------------------------------------------------------------

/// Plans the Gulf respiratory irritation forecast for the beaches in the view at Unix time
/// `now`. It is a plan of its own rather than a part of the others: it needs no species, state
/// or paging, and its one request is the same region-wide file for every view, so the view is
/// applied to the answer and no coordinate leaves the phone. A view outside the Florida and
/// Texas forecast area gets no request and a notice that the forecast covers the Gulf only.
/// The answer may be reused for an hour (the provider's age).
///
/// # Errors
/// [`NearbyError::Invalid`] for a view out of range or unreadable settings.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn plan_blooms(
    area: NearbyArea,
    settings_json: String,
    now: i64,
) -> Result<Arc<NearbyPlan>, NearbyError> {
    let (config, _) = load(&settings_json)?;
    let area = Area::new(area)?;
    let mut inner = Inner::new(config, area, NearbyState::default(), false, UnixTime(now));
    let covered = inner
        .area
        .parts()
        .iter()
        .any(|b| hab::overlaps_gulf_area(b.south, b.west, b.north, b.east));
    if covered {
        inner.push(
            SourceId::NOAA_HAB,
            "noaa-hab",
            0,
            Want::Blooms,
            get(hab::GULF_ZIP_URL.to_owned(), None, hab::MAX_AGE_S),
        );
    } else {
        let text = inner.f.strings.get(Key::NearbyBloomsGulfOnly).to_owned();
        push_unique(&mut inner.notices, text);
    }
    Ok(Arc::new(NearbyPlan {
        inner: Mutex::new(inner),
    }))
}

// ---------------------------------------------------------------------------------------------
// Reading the answers
// ---------------------------------------------------------------------------------------------

/// A failure as the issue list names it.
fn kind_of_status(status: u16) -> SourceErrorKind {
    match status {
        404 => SourceErrorKind::NotFound,
        429 => SourceErrorKind::RateLimited,
        _ => SourceErrorKind::Other,
    }
}

impl Inner {
    /// Reads one answer; follow-ups go to `next`.
    fn read_one(&mut self, p: &Pending, answer: Option<&HostResponse>, next: &mut Vec<Pending>) {
        let got = match answer.map(|r| fetched(r.clone())) {
            Some(Ok(f)) => Ok(f),
            Some(Err(e)) => Err(e.to_string()),
            None => Err("no response".to_owned()),
        };
        match &p.want {
            Want::Overpass { key } => self.read_overpass(p, *key, &got, next),
            want => {
                let fetched = match got {
                    Ok(f) => f,
                    Err(message) => {
                        self.issue(&p.source, SourceErrorKind::Transport, message);
                        self.notice(Key::NearbyFailed, &p.source);
                        return;
                    }
                };
                let response = fetched.as_response();
                match want {
                    Want::Padus { bbox, offset, page } => {
                        self.read_padus(&response, *bbox, *offset, *page, next);
                    }
                    Want::Gbif => self.read_gbif(&response),
                    Want::Inat => self.read_inat(&response),
                    Want::Nas { offset, page } => self.read_nas(&response, *offset, *page, next),
                    Want::Stocking { dataset, points } => {
                        self.read_stocking(&response, *dataset, points);
                    }
                    Want::Blooms => self.read_blooms(&response),
                    Want::BeachLayers => self.read_beach_layers(&response),
                    Want::BeachOffice { key } => self.read_beach_office(key, &response),
                    Want::BeachZones => self.read_beach_zones(&response),
                    Want::Overpass { .. } => {}
                }
            }
        }
    }

    fn failed(&mut self, source: &SourceId, kind: SourceErrorKind, message: String) {
        self.issue(source, kind, message);
        self.notice(Key::NearbyFailed, source);
    }

    fn read_padus(
        &mut self,
        response: &Response<'_>,
        bbox: Bbox,
        offset: u32,
        page: u32,
        next: &mut Vec<Pending>,
    ) {
        match padus::parse_page(response) {
            Ok(got) => {
                self.access.extend(got.features);
                if !got.more {
                    return;
                }
                let offset = offset.saturating_add(padus::PAGE_SIZE);
                if page >= PADUS_MAX_PAGES {
                    self.notice(Key::NearbyMoreThanShown, &SourceId::PADUS);
                    return;
                }
                if let Ok(r) = padus::query_request(bbox, offset, padus_tolerance(&self.area)) {
                    self.seq += 1;
                    next.push(Pending {
                        id: format!("{}:{}", self.seq, SourceId::PADUS.as_str()),
                        source: SourceId::PADUS,
                        queue: "padus",
                        delay_ms: 0,
                        want: Want::Padus {
                            bbox,
                            offset,
                            page: page + 1,
                        },
                        request: get(r.url, None, DAY_S),
                    });
                }
            }
            Err(e) => {
                let kind = match &e {
                    padus::PadusError::Status { status } => kind_of_status(*status),
                    padus::PadusError::Parse(_) => SourceErrorKind::Parse,
                    _ => SourceErrorKind::Other,
                };
                self.failed(&SourceId::PADUS, kind, e.to_string());
            }
        }
    }

    fn read_overpass(
        &mut self,
        p: &Pending,
        key: overpass::CacheKey,
        got: &Result<underbite_fetch::Fetched, String>,
        next: &mut Vec<Pending>,
    ) {
        let parsed = match got {
            Ok(f) => overpass::parse_page(&f.as_response(), key.kind),
            // No answer at all counts as a busy server.
            Err(_) => Err(overpass::OverpassError::Busy {
                retry_after_secs: 0,
            }),
        };
        let error = match parsed {
            Ok(page) => {
                self.state.overpass_gate.finish(self.now, None);
                if page.truncated {
                    self.notice(Key::NearbyMoreThanShown, &SourceId::OVERPASS);
                }
                self.access.extend(page.features.iter().cloned());
                self.state.overpass_cache.insert(
                    key.id(),
                    CachedCell {
                        fetched_at: self.now.0,
                        truncated: page.truncated,
                        features: page.features,
                    },
                );
                return;
            }
            Err(e) => e,
        };
        let kind = match &error {
            overpass::OverpassError::RateLimited { .. } => SourceErrorKind::RateLimited,
            overpass::OverpassError::Status { status } => kind_of_status(*status),
            overpass::OverpassError::Parse(_) => SourceErrorKind::Parse,
            overpass::OverpassError::Busy { .. } if got.is_err() => SourceErrorKind::Transport,
            _ => SourceErrorKind::Other,
        };
        let message = match got {
            Err(m) => m.clone(),
            Ok(_) => error.to_string(),
        };
        self.issue(&SourceId::OVERPASS, kind, message);
        match self.state.overpass_gate.finish(self.now, Some(&error)) {
            overpass::Next::Retry { after_secs } => {
                let at = UnixTime(self.now.0.saturating_add(i64::from(after_secs)));
                match self
                    .state
                    .overpass_gate
                    .begin(at, overpass::UserAction::requested())
                {
                    Ok(()) => {
                        self.seq += 1;
                        next.push(Pending {
                            id: format!("{}:{}", self.seq, SourceId::OVERPASS.as_str()),
                            delay_ms: after_secs.saturating_mul(1_000),
                            ..p.clone()
                        });
                    }
                    Err(r) => self.refused(&SourceId::OVERPASS, r.into()),
                }
            }
            overpass::Next::Stop => {
                let key = if error.retryable() {
                    Key::NearbyBusy
                } else {
                    Key::NearbyFailed
                };
                self.notice(key, &SourceId::OVERPASS);
            }
        }
    }

    fn read_gbif(&mut self, response: &Response<'_>) {
        match gbif::parse_page(response) {
            Ok(page) => {
                self.more |= page.next_offset.is_some();
                self.gbif.extend(page.records);
            }
            Err(gbif::GbifError::RateLimited { retry_after_s }) => {
                self.state.gbif.pause(self.now, retry_after_s);
                self.issue(
                    &SourceId::GBIF,
                    SourceErrorKind::RateLimited,
                    gbif::GbifError::RateLimited { retry_after_s }.to_string(),
                );
                self.refused(
                    &SourceId::GBIF,
                    Refusal::Paused {
                        wait_s: retry_after_s,
                    },
                );
            }
            Err(e) => {
                let kind = match &e {
                    gbif::GbifError::Status { status } => kind_of_status(*status),
                    gbif::GbifError::Parse(_) => SourceErrorKind::Parse,
                    _ => SourceErrorKind::Other,
                };
                self.failed(&SourceId::GBIF, kind, e.to_string());
            }
        }
    }

    fn read_inat(&mut self, response: &Response<'_>) {
        match inat::parse_page(response) {
            Ok(page) => {
                self.more |= page.next_page.is_some();
                self.inat.extend(page.records);
            }
            Err(inat::InatError::RateLimited { retry_after_s }) => {
                self.state.inaturalist.pause(self.now, retry_after_s);
                self.issue(
                    &SourceId::INATURALIST,
                    SourceErrorKind::RateLimited,
                    inat::InatError::RateLimited { retry_after_s }.to_string(),
                );
                self.refused(
                    &SourceId::INATURALIST,
                    Refusal::Paused {
                        wait_s: retry_after_s,
                    },
                );
            }
            Err(e) => {
                let kind = match &e {
                    inat::InatError::Status { status } => kind_of_status(*status),
                    inat::InatError::Parse(_) => SourceErrorKind::Parse,
                    _ => SourceErrorKind::Other,
                };
                self.failed(&SourceId::INATURALIST, kind, e.to_string());
            }
        }
    }

    fn read_nas(
        &mut self,
        response: &Response<'_>,
        offset: u32,
        page: u32,
        next: &mut Vec<Pending>,
    ) {
        match nas::parse_page(response) {
            Ok(got) => {
                self.nas.extend(got.records);
                let Some(next_offset) = got.next_offset else {
                    return;
                };
                if page >= NAS_MAX_PAGES {
                    self.notice(Key::NearbyMoreThanShown, &SourceId::USGS_NAS);
                    return;
                }
                let Some(q) = &self.nas_query else {
                    return;
                };
                if let Ok(r) = nas::search_request(q, next_offset.max(offset), nas::PAGE_SIZE) {
                    self.seq += 1;
                    next.push(Pending {
                        id: format!("{}:{}", self.seq, SourceId::USGS_NAS.as_str()),
                        source: SourceId::USGS_NAS,
                        queue: "usgs-nas",
                        delay_ms: 0,
                        want: Want::Nas {
                            offset: next_offset,
                            page: page + 1,
                        },
                        request: get(r.url, None, DAY_S),
                    });
                }
            }
            Err(e) => {
                let kind = match &e {
                    nas::NasError::Status { status } => kind_of_status(*status),
                    nas::NasError::Parse(_) => SourceErrorKind::Parse,
                    nas::NasError::InvalidRequest(_) => SourceErrorKind::Other,
                };
                self.failed(&SourceId::USGS_NAS, kind, e.to_string());
            }
        }
    }

    fn read_blooms(&mut self, response: &Response<'_>) {
        let source = SourceId::NOAA_HAB;
        if !(200..300).contains(&response.status) {
            self.failed(
                &source,
                kind_of_status(response.status),
                format!("HTTP {}", response.status),
            );
            return;
        }
        let area = self.area.clone();
        match hab::parse_gulf_beaches(response.body, self.now, |lat, lon| area.contains(lat, lon)) {
            Ok(beaches) => {
                if beaches.is_empty() {
                    let text = self.f.strings.get(Key::NearbyBloomsNone).to_owned();
                    push_unique(&mut self.notices, text);
                }
                self.blooms = beaches;
            }
            Err(underbite_providers::source::SourceError::Stale { .. }) => {
                // An old run is never shown as "no bloom".
                let text = self
                    .f
                    .strings
                    .safety(SafetyKey::BloomNoRecentForecast)
                    .to_owned();
                push_unique(&mut self.notices, text.clone());
                self.issue(&source, SourceErrorKind::Other, text);
            }
            Err(e) => self.failed(&source, SourceErrorKind::Parse, e.to_string()),
        }
    }

    fn read_stocking(&mut self, response: &Response<'_>, dataset: Dataset, points: &[usize]) {
        if !(200..300).contains(&response.status) {
            self.failed(
                &SourceId::NY_DEC_STOCKING,
                kind_of_status(response.status),
                format!("HTTP {}", response.status),
            );
            return;
        }
        let events = match dataset.parse(response.body) {
            Ok(events) => events,
            Err(e) => {
                self.failed(
                    &SourceId::NY_DEC_STOCKING,
                    SourceErrorKind::Parse,
                    e.to_string(),
                );
                return;
            }
        };
        for &i in points {
            if let Some(p) = self.points.get_mut(i) {
                p.answered = true;
            }
        }
        for e in events {
            let county = e.county.as_deref().map(canonical_county);
            let water = canonical_waterbody(&e.waterbody);
            let found = points.iter().copied().find(|&i| {
                self.points.get(i).is_some_and(|p| {
                    p.key.1 == water && county.as_ref().is_none_or(|c| *c == p.key.0)
                })
            });
            if let Some(p) = found.and_then(|i| self.points.get_mut(i)) {
                p.events.push(e);
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The results
// ---------------------------------------------------------------------------------------------

const fn hab_code(level: HabLevel) -> &'static str {
    match level {
        HabLevel::VeryLow => "very_low",
        HabLevel::Low => "low",
        HabLevel::Moderate => "moderate",
        HabLevel::High => "high",
        HabLevel::Unknown => "unknown",
    }
}

const fn hab_key(level: HabLevel) -> SafetyKey {
    match level {
        HabLevel::VeryLow => SafetyKey::BloomLevelVeryLow,
        HabLevel::Low => SafetyKey::BloomLevelLow,
        HabLevel::Moderate => SafetyKey::BloomLevelModerate,
        HabLevel::High => SafetyKey::BloomLevelHigh,
        HabLevel::Unknown => SafetyKey::BloomLevelUnknown,
    }
}

/// The pin of a beach: its words are the bloom card's.
fn bloom_pin(config: &Config, beach: &hab::HabBeach) -> BloomPin {
    // The forecast's times are the beach's own: Central west of 85 degrees West, else Eastern.
    let zone = if beach.longitude < -85.0 {
        "America/Chicago"
    } else {
        "America/New_York"
    };
    let f = formatter(config, zone);
    let s = f.strings;
    let word = s.safety(hab_key(beach.level));
    let mut label = word.to_owned();
    if let Some(first) = label.get_mut(..1) {
        first.make_ascii_uppercase();
    }
    let mut detail = format!("{}: {word}", s.get(Key::BloomRowLevel));
    if let Some(c) = beach.confidence {
        let value = s.safety_fmt(SafetyKey::BloomConfidence, &[("value", &s.number(c, 1))]);
        detail = format!("{detail}; {}: {value}", s.get(Key::BloomRowConfidence));
    }
    let forecast_for = format!("{}, {}", f.date_of(beach.valid_at), f.clock(beach.valid_at));
    let link = hab::links_near(Coordinates {
        latitude: beach.latitude,
        longitude: beach.longitude,
    })
    .into_iter()
    .next()
    .map(|l| l.url);
    BloomPin {
        id: format!(
            "{}:{}:{:.3}:{:.3}",
            SourceId::NOAA_HAB.as_str(),
            beach.location,
            beach.latitude,
            beach.longitude
        ),
        accessibility: format!(
            "{}. {detail}. {}: {forecast_for}",
            beach.location,
            s.get(Key::BloomRowValid)
        ),
        name: beach.location.clone(),
        lat: beach.latitude,
        lon: beach.longitude,
        level: hab_code(beach.level).to_owned(),
        level_label: label,
        detail,
        forecast_for,
        link,
    }
}

const fn kind_code(kind: AccessKind) -> &'static str {
    match kind {
        AccessKind::Ramp => "ramp",
        AccessKind::Pier => "pier",
        AccessKind::FishingSpot => "fishing_spot",
        AccessKind::Parking => "parking",
        AccessKind::PublicLand => "public_land",
    }
}

const fn level_code(level: AccessLevel) -> &'static str {
    match level {
        AccessLevel::Open => "open",
        AccessLevel::Restricted => "restricted",
        AccessLevel::Closed => "closed",
        AccessLevel::Unknown => "unknown",
    }
}

fn point(g: GeoPoint) -> MapPoint {
    MapPoint {
        lat: g.lat,
        lon: g.lon,
    }
}

/// The marker position and the shape of a feature; `None` for an empty geometry.
fn shape(g: &AccessGeometry) -> Option<(GeoPoint, AccessShape)> {
    match g {
        AccessGeometry::Point(p) => Some((*p, AccessShape::Point)),
        AccessGeometry::Line(points) => {
            let middle = *points.get(points.len() / 2)?;
            Some((
                middle,
                AccessShape::Line {
                    points: points.iter().copied().map(point).collect(),
                },
            ))
        }
        AccessGeometry::Polygon(rings) => {
            let outer = rings.first().filter(|r| !r.is_empty())?;
            let n = outer.len() as f64;
            let marker = GeoPoint {
                lat: outer.iter().map(|p| p.lat).sum::<f64>() / n,
                lon: outer.iter().map(|p| p.lon).sum::<f64>() / n,
            };
            Some((
                marker,
                AccessShape::Polygon {
                    rings: rings
                        .iter()
                        .map(|r| r.iter().copied().map(point).collect())
                        .collect(),
                },
            ))
        }
    }
}

fn access_pin(f: &Formatter, feature: &AccessFeature) -> Option<AccessPin> {
    let (at, shape) = shape(&feature.geometry)?;
    let kind_label = f.strings.access_kind(feature.kind).to_owned();
    let title = feature.name.clone().unwrap_or_else(|| kind_label.clone());
    let access_label = f.strings.access_level(feature.access).to_owned();
    let accessibility = f.strings.fmt(
        Key::AccessAccessibility,
        &[
            ("title", &title),
            ("kind", &kind_label),
            ("access", &access_label),
        ],
    );
    Some(AccessPin {
        id: format!(
            "{}:{}:{:.5}:{:.5}",
            feature.source.as_str(),
            kind_code(feature.kind),
            at.lat,
            at.lon
        ),
        source: feature.source.as_str().to_owned(),
        source_name: f.strings.source_name(&feature.source),
        kind: kind_code(feature.kind).to_owned(),
        kind_label,
        title,
        access: level_code(feature.access).to_owned(),
        access_label,
        lat: at.lat,
        lon: at.lon,
        shape,
        accessibility,
    })
}

fn licence_words(f: &Formatter, licence: Licence) -> String {
    f.strings
        .safety(match licence {
            Licence::Cc0 => SafetyKey::LicenceCc0,
            Licence::CcBy => SafetyKey::LicenceCcBy,
            Licence::CcByNc => SafetyKey::LicenceCcByNc,
            Licence::PublicDomain => SafetyKey::LicencePublicDomain,
        })
        .to_owned()
}

fn web_url(text: Option<&str>) -> Option<String> {
    text.map(str::trim)
        .filter(|t| t.starts_with("https://") || t.starts_with("http://"))
        .map(str::to_owned)
}

fn record_pin(
    f: &Formatter,
    record: &SpeciesRecord,
    common_name: Option<&str>,
    credit: &Attribution,
) -> Option<RecordPin> {
    let licence = Licence::of(record)?;
    let common_name = common_name
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(str::to_owned);
    let title = common_name.clone().unwrap_or_else(|| record.taxon.clone());
    let seen = record_seen(f, record.observed_on);
    let source_name = f.strings.source_name(&record.source);
    let link = web_url(record.url.as_deref());
    let introduced = record.source == SourceId::USGS_NAS;
    let photo = record.photo.as_ref().and_then(|p| {
        let words = licence_words(f, Licence::classify(p.licence.as_deref()?)?);
        Some(RecordPhoto {
            url: web_url(Some(&p.url))?,
            caption: f.strings.fmt(Key::RecordPhoto, &[("licence", &words)]),
            licence: words,
        })
    });
    Some(RecordPin {
        id: format!("{}:{}", record.source.as_str(), record.id),
        source: record.source.as_str().to_owned(),
        accessibility: f.strings.fmt(
            Key::RecordAccessibility,
            &[("title", &title), ("seen", &seen), ("source", &source_name)],
        ),
        source_name,
        taxon: record.taxon.clone(),
        common_name,
        title,
        lat: record.latitude,
        lon: record.longitude,
        radius_m: record
            .uncertainty_m
            .filter(|m| m.is_finite() && *m >= 0.0)
            .map(f64::from),
        obscured: record.obscured,
        introduced,
        introduced_label: introduced.then(|| f.strings.get(Key::RecordIntroduced).to_owned()),
        position: record_position(f, record.obscured, record.uncertainty_m),
        seen,
        licence: licence_words(f, licence),
        licence_url: web_url(record.licence.as_deref()),
        link_title: link
            .as_ref()
            .map(|_| record_link_title(f.strings, &record.source)),
        link,
        credit: credit.credit.clone(),
        credit_url: credit.url.clone(),
        photo,
    })
}

fn water_kind(kind: WaterKind) -> (&'static str, WaterType) {
    match kind {
        WaterKind::Lake => ("lake", WaterType::LakeReservoir),
        WaterKind::River => ("river", WaterType::RiverStream),
        WaterKind::Pond => ("pond", WaterType::Pond),
    }
}

/// The view's stocking card of one water's events.
fn stocking_card(config: &Config, id: &str, p: &Point, now: UnixTime) -> Option<StockingCard> {
    let spot = Spot {
        id: id.to_owned(),
        name: p.placed.waterbody.clone(),
        latitude: p.placed.latitude,
        longitude: p.placed.longitude,
        water_type: water_kind(p.placed.kind).1,
        water_body: None,
        time_zone: "UTC".to_owned(),
    };
    let mut snapshot = SpotSnapshot::new(spot, now);
    snapshot.stocking.clone_from(&p.events);
    snapshot.attribution = vec![socrata_ny::attribution()];
    stocking::build(&view_input(config, snapshot, now))
}

impl Inner {
    fn credits(&self) -> Vec<NearbyCredit> {
        let s = self.f.strings;
        let accessed = self.now.to_local(UtcOffset(0)).date;
        self.used
            .iter()
            .filter_map(|source| {
                let (attribution, licence) = credit_of(source, accessed)?;
                Some(NearbyCredit {
                    source: source.as_str().to_owned(),
                    name: s.source_name(source),
                    text: attribution.credit,
                    url: attribution.url,
                    licence: licence.map(|k| s.safety(k).to_owned()),
                })
            })
            .collect()
    }

    #[allow(clippy::too_many_lines)]
    fn results(&self) -> NearbyResults {
        let f = &self.f;
        let mut access: Vec<AccessPin> = Vec::new();
        let mut ids = BTreeSet::new();
        for pin in self.access.iter().filter_map(|a| access_pin(f, a)) {
            if ids.insert(pin.id.clone()) {
                access.push(pin);
            }
        }

        let accessed = self.now.to_local(UtcOffset(0)).date;
        let mut records: Vec<RecordPin> = Vec::new();
        let mut seen = BTreeSet::new();
        let inat_records: Vec<SpeciesRecord> = self.inat.iter().map(|r| r.record.clone()).collect();
        let inat_credit = inat::attribution();
        let gbif_credit = gbif::attribution();
        let nas_credit = nas::attribution(accessed);
        let pins = self
            .inat
            .iter()
            .filter_map(|r| record_pin(f, &r.record, r.common_name.as_deref(), &inat_credit))
            .chain(
                without_inaturalist_duplicates(self.gbif.clone(), &inat_records)
                    .into_iter()
                    .filter_map(|r| record_pin(f, &r.record, None, &gbif_credit)),
            )
            .chain(
                self.nas
                    .iter()
                    .filter(|r| self.area.contains(r.record.latitude, r.record.longitude))
                    .filter_map(|r| {
                        record_pin(f, &r.record, r.common_name.as_deref(), &nas_credit)
                    }),
            );
        for pin in pins {
            if seen.insert(pin.id.clone()) {
                records.push(pin);
            }
        }

        let stocking = self
            .points
            .iter()
            .map(|p| {
                let id = format!(
                    "{}:{}:{}",
                    SourceId::NY_DEC_STOCKING.as_str(),
                    p.placed.county,
                    p.placed.waterbody
                );
                let card = p
                    .answered
                    .then(|| stocking_card(&self.config, &id, p, self.now))
                    .flatten();
                StockingPin {
                    label: card
                        .as_ref()
                        .and_then(|c| c.compact.lines.first().cloned())
                        .unwrap_or_else(|| p.placed.waterbody.clone()),
                    card_json: card.and_then(|c| serde_json::to_string(&c).ok()),
                    id,
                    waterbody: p.placed.waterbody.clone(),
                    county: p.placed.county.clone(),
                    name: p.placed.name.clone(),
                    lat: p.placed.latitude,
                    lon: p.placed.longitude,
                    kind: water_kind(p.placed.kind).0.to_owned(),
                }
            })
            .collect();

        let blooms: Vec<BloomPin> = self
            .blooms
            .iter()
            .map(|b| bloom_pin(&self.config, b))
            .collect();
        let (beach_zones, beach_legend) = self.beach_results();
        let mut notes = self.notes.clone();
        if !beach_zones.is_empty() {
            push_unique(
                &mut notes,
                f.strings.safety(SafetyKey::BeachSafety).to_owned(),
            );
        }
        if !blooms.is_empty() {
            push_unique(
                &mut notes,
                f.strings.safety(SafetyKey::BloomAdvisory).to_owned(),
            );
        }
        let mut notices = self.notices.clone();
        self.beach_notices(&mut notices);
        if let Some(fetched) = self.cached_oldest {
            let age = f.age(self.now.0 - fetched);
            let name = self.name(&SourceId::OVERPASS);
            push_unique(
                &mut notices,
                f.strings
                    .fmt(Key::NearbyCached, &[("source", &name), ("age", &age)]),
            );
        }

        let state_json = self.stateful.then(|| {
            let mut state = self.state.clone();
            state.prune(self.now);
            // INVARIANT: the state holds only numbers, strings and the core's serializable
            // access features, so it always serializes.
            serde_json::to_string(&state).unwrap_or_default()
        });
        NearbyResults {
            access,
            records,
            stocking,
            blooms,
            beach_zones,
            beach_legend,
            credits: self.credits(),
            notices,
            notes,
            issues: self.issues.iter().map(SourceIssue::from).collect(),
            more: self.more,
            state_json,
        }
    }
}

/// A source's credit and, when one licence covers all its data, that licence.
fn credit_of(source: &SourceId, accessed: LocalDate) -> Option<(Attribution, Option<SafetyKey>)> {
    Some(match source.as_str() {
        "padus" => (
            Attribution {
                source: SourceId::PADUS,
                credit: padus::PADUS_CREDIT.to_owned(),
                url: None,
            },
            Some(SafetyKey::LicencePublicDomain),
        ),
        "overpass" => (
            Attribution {
                source: SourceId::OVERPASS,
                credit: overpass::OVERPASS_CREDIT.to_owned(),
                url: Some(overpass::OVERPASS_COPYRIGHT_URL.to_owned()),
            },
            Some(SafetyKey::LicenceOdbl),
        ),
        "gbif" => (gbif::attribution(), None),
        "inaturalist" => (inat::attribution(), None),
        "usgs-nas" => (
            nas::attribution(accessed),
            Some(SafetyKey::LicencePublicDomain),
        ),
        "ny-dec-stocking" => (socrata_ny::attribution(), None),
        "noaa-hab" => (hab::attribution(), Some(SafetyKey::LicencePublicDomain)),
        "nws-beach-forecast" => (
            beach_forecast::attribution(),
            Some(SafetyKey::LicencePublicDomain),
        ),
        _ => return None,
    })
}

#[uniffi::export]
impl NearbyPlan {
    /// The requests of the current round, in a fixed order.
    pub fn requests(&self) -> Vec<NearbyRequest> {
        let inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        inner
            .round
            .iter()
            .map(|p| NearbyRequest {
                id: p.id.clone(),
                source: p.source.as_str().to_owned(),
                queue: p.queue.to_owned(),
                delay_ms: p.delay_ms,
                request: p.request.clone(),
            })
            .collect()
    }

    /// Reads the answers to the current round at Unix time `now` and plans the next round (a
    /// next page, a retry), which [`Self::requests`] then gives. Answers with an id the round
    /// does not hold are ignored; a request without an answer counts as failed. Returns
    /// everything found so far, with the state to keep.
    #[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
    pub fn read(&self, responses: Vec<NearbyResponse>, now: i64) -> NearbyResults {
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        inner.now = UnixTime(now.max(inner.now.0));
        let by_id: BTreeMap<&str, &HostResponse> = responses
            .iter()
            .map(|r| (r.id.as_str(), &r.response))
            .collect();
        let round = std::mem::take(&mut inner.round);
        let mut next = Vec::new();
        for p in &round {
            inner.read_one(p, by_id.get(p.id.as_str()).copied(), &mut next);
        }
        inner.beach_after_round(&mut next);
        inner.round = next;
        inner.results()
    }

    /// Everything found so far, with the state to keep: before any answer, what the state's
    /// caches held and the stocking pins without cards.
    pub fn results(&self) -> NearbyResults {
        self.inner
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .results()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(west: f64, south: f64, east: f64, north: f64) -> Area {
        Area::new(NearbyArea {
            west,
            south,
            east,
            north,
        })
        .unwrap()
    }

    #[test]
    fn boxes_round_outward_to_the_chosen_precision() {
        let b = Bbox {
            west: -74.0571,
            south: 40.6801,
            east: -73.9912,
            north: 40.7409,
        };
        let two = outward(b, 2);
        assert_eq!(
            (two.west, two.south, two.east, two.north),
            (-74.06, 40.68, -73.99, 40.75)
        );
        let one = outward(b, 1);
        assert_eq!(
            (one.west, one.south, one.east, one.north),
            (-74.1, 40.6, -73.9, 40.8)
        );
        // Never finer than 2 decimals, and a value on the grid stays put.
        let five = outward(b, 5);
        assert_eq!(five, two);
        let on = outward(
            Bbox {
                west: -74.4,
                south: 40.5,
                east: -73.7,
                north: 40.9,
            },
            2,
        );
        assert_eq!((on.west, on.east), (-74.4, -73.7));
    }

    #[test]
    fn a_view_across_the_antimeridian_is_two_boxes() {
        let a = area(179.9, 51.0, -179.9, 51.1);
        assert!((a.width() - 0.2).abs() < 1e-9);
        assert_eq!(a.parts().len(), 2);
        assert!(a.contains(51.05, 179.95) && a.contains(51.05, -179.95));
        assert!(!a.contains(51.05, 0.0));
        // One row of cells, one on each side of the antimeridian.
        let cells = overpass_cells(&a).unwrap();
        assert_eq!(cells.len(), 2);
        assert_eq!(
            cells.iter().map(|c| c.west_tenths).collect::<BTreeSet<_>>(),
            BTreeSet::from([-1800, 1799])
        );
        // A wide one is refused rather than paged.
        assert!(!area(170.0, 50.0, -170.0, 51.0).fits(RECORDS_MAX_SPAN_DEG));
        assert!(
            Area::new(NearbyArea {
                west: f64::NAN,
                south: 0.0,
                east: 1.0,
                north: 1.0
            })
            .is_err()
        );
    }

    #[test]
    fn the_pacer_spaces_caps_and_pauses() {
        let mut p = Pacer::default();
        let now = UnixTime(1_763_400_000);
        assert_eq!(p.begin(now, Some(2), 1_000), Ok(0));
        assert_eq!(p.begin(now, Some(2), 1_000), Ok(1_000));
        assert_eq!(p.begin(now, Some(2), 1_000), Err(Refusal::DailyCap));
        // A new UTC day starts a new count.
        let tomorrow = UnixTime(now.0 + 86_400);
        assert_eq!(p.begin(tomorrow, Some(2), 1_000), Ok(0));
        p.pause(tomorrow, 60);
        assert_eq!(
            p.begin(UnixTime(tomorrow.0 + 20), Some(2), 1_000),
            Err(Refusal::Paused { wait_s: 40 })
        );
    }

    #[test]
    fn the_state_drops_expired_cells_and_keeps_the_newest() {
        let mut state = NearbyState::default();
        let now = UnixTime(1_763_400_000);
        for i in 0..70 {
            state.overpass_cache.insert(
                format!("overpass:places:{i}:0"),
                CachedCell {
                    fetched_at: now.0 - i64::from(i),
                    truncated: false,
                    features: Vec::new(),
                },
            );
        }
        state.overpass_cache.insert(
            "overpass:places:old:0".into(),
            CachedCell {
                fetched_at: now.0 - 31 * 86_400,
                truncated: false,
                features: Vec::new(),
            },
        );
        state.prune(now);
        assert_eq!(state.overpass_cache.len(), OVERPASS_CACHE_CELLS);
        assert!(state.overpass_cache.contains_key("overpass:places:0:0"));
        assert!(!state.overpass_cache.contains_key("overpass:places:69:0"));
        assert!(!state.overpass_cache.contains_key("overpass:places:old:0"));
        assert_eq!(NearbyState::read("").unwrap(), NearbyState::default());
        assert!(NearbyState::read("not json").is_err());
    }

    #[test]
    fn nas_is_asked_by_genus_and_epithet() {
        assert_eq!(
            nas_taxon("Channa argus"),
            nas::NasTaxon::Species {
                genus: "Channa".into(),
                species: "argus".into()
            }
        );
        assert_eq!(nas_taxon("Channa"), nas::NasTaxon::Genus("Channa".into()));
    }
}
