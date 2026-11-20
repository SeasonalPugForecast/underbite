//! Station pins on the map: the station lists of the visible area and the state of each pin.
//!
//! The lists are the ones discovery uses: CO-OPS's and NDBC's whole lists, and USGS sites and
//! NWPS gauges in small boxes. A list a spot's discovery stored is used as it is; the map stores
//! what it fetches under keys of its own, so it never changes what discovery reads. A whole
//! list is kept as long as the offline cache serves it ([`MetadataKind`]). Box lists come by the
//! cells of a fixed half-degree grid (each cell is the box the providers build around its
//! centre): those stored are always read, and only when the map is close in are a few missing
//! or old ones fetched per call, so panning across a region does not download it.
//!
//! A pin's state: a USGS gauge shows its latest value, from one batched `latest-continuous`
//! request for at most [`MAX_STATE_GAUGES`] visible gauges per call. That request is metered;
//! when the call ledger refuses it nothing is sent and the pins show without state. An NWPS
//! gauge shows the flood category its box listing observed. Percentiles are not fetched for
//! pins. The state is a colour role by name ([`station_pin_roles`]), never a colour, and every
//! word is the view's.
//!
//! Coordinates leave the device only as cell centres (whole or half degrees) and the box edges
//! the providers build around them; the map's own box is never sent.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use underbite_config::schema::Config;
use underbite_core::datum::Height;
use underbite_core::map::catalogue::{self, LayerKind, StationKind as LayerStation};
use underbite_core::model::{
    Measurement, Requirements, SourceError as ModelSourceError, StationCandidate,
};
use underbite_core::offline::cache::MetadataKind;
use underbite_core::provenance::{Relation, SourceId, StationKind, StationRef};
use underbite_core::time::UnixTime;
use underbite_core::view::format::Formatter;
use underbite_core::view::strings::{Key, SafetyKey};
use underbite_fetch::store::{self, STATION_LIST_TTL_S, Stored, spot_key};
use underbite_fetch::{Call, Fetched};
use underbite_providers::coops::Coops;
use underbite_providers::coops::mdapi::{self, Metadata, StationList};
use underbite_providers::ndbc::{self, Ndbc};
use underbite_providers::nwps::{self, FloodCategory, ListedGauge, Nwps};
use underbite_providers::request::{
    Coordinates, Place, Request, RoundedPoint, Target, Window, redact_url, secret_values,
};
use underbite_providers::session::usage::UsageLedger;
use underbite_providers::source::{
    FetchContext, FragmentValue, Fragments, Secrets, Source, SourceError, check_response,
};
use underbite_providers::usgs::{Usgs, ogc, sites};

use crate::fetch::{FetchHost, Keys, SourceIssue, fetched, host_request, user_agent};
use crate::map::MapError;
use crate::settings::load;

/// The most USGS gauges whose state one call asks for, in one request.
pub const MAX_STATE_GAUGES: usize = 25;

/// The side of a list cell, degrees.
const CELL_DEG: f64 = 0.5;
/// Half the side of a list cell: the half width of the USGS site search and of the NWPS gauge
/// box the providers build around a point, about 25 km.
const CELL_HALF_DEG: f64 = 0.25;
/// The most cells a view may span for its box lists to be read at all (a view about 3° across).
const MAX_AREA_CELLS: usize = 36;
/// The most cells of each box list fetched in one call, nearest the view's centre first.
const MAX_FETCHED_CELLS: usize = 4;
/// The zoom from which box lists are fetched and gauge states asked for.
const CLOSE_ZOOM: f64 = 9.0;
/// How long a stored NWPS cell answers before it is fetched again: its flood categories change
/// within hours.
const NWPS_REFRESH_S: i64 = 3_600;
/// How old an observed flood category may be and still colour a pin.
const NWPS_STATE_MAX_AGE_S: i64 = 6 * 3_600;
/// How long a cell's USGS sites answer when one of its site-type searches failed: long enough
/// that panning back and forth spends no more requests, short enough that the missing type is
/// soon asked again.
const PARTIAL_SITES_TTL_S: i64 = 3_600;
/// A USGS site whose list says its newest value is older than this is shown without a value,
/// and not asked about (the list's series ends lag by about a day at most).
const USGS_REPORTING_WITHIN_S: i64 = 30 * 86_400;

/// The parameters asked of `latest-continuous` for gauge pins: discharge, gage height, lake and
/// reservoir levels, USGS tide levels and water temperature (codes of the USGS provider's table).
const PIN_CODES: &[&str] = &[
    "00060", "00065", "62614", "62615", "62616", "00062", "62620", "62619", "00010",
];
/// What a gauge pin shows, first found first.
const PIN_MEASUREMENTS: [Measurement; 5] = [
    Measurement::Discharge,
    Measurement::GageHeight,
    Measurement::LakeLevel,
    Measurement::Tide,
    Measurement::WaterTemperature,
];

// ---------------------------------------------------------------------------------------------
// What the app receives
// ---------------------------------------------------------------------------------------------

/// One station on the map.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct StationPin {
    /// The station as the pin calls take it: `coops:8518750`, `ndbc:44025`,
    /// `usgs:USGS-01375000`, `nwps:BATN6`. A current station is one pin for all its bins.
    pub id: String,
    /// `tide`, `current`, `buoy`, `river_gauge` or `forecast_gauge`.
    pub kind: String,
    /// The map layer that shows it: `tide-stations`, `current-stations`, `buoys`, `usgs-gauges`
    /// or `nwps-gauges`.
    pub layer: String,
    /// The station's name as its source writes it.
    pub name: String,
    /// Degrees north.
    pub lat: f64,
    /// Degrees east.
    pub lon: f64,
    /// The colour role of its state, one of [`station_pin_roles`].
    pub state_role: String,
    /// The pin in words: "River gauge · 412 ft³/s, 25 min ago", "River forecast gauge · No
    /// flooding, 1 h ago", "Tide station".
    pub label: String,
}

/// The pins of a view, and the lists or states that could not be had.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct MapStations {
    /// Every pin in the view of a shown station layer, in the catalogue's layer order, then by
    /// id.
    pub pins: Vec<StationPin>,
    /// Lists or states that failed or were refused (`rate_limited` when the call budget is
    /// spent).
    pub errors: Vec<SourceIssue>,
}

/// The colour role of a pin's state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    /// The kind has no state (tide and current stations, buoys).
    None,
    /// The state was not asked for or could not be had.
    Unknown,
    /// The gauge has no recent value, or is out of service.
    NoData,
    /// The gauge reports, with nothing to grade its value by.
    Reporting,
    /// Below the gauge's low-water threshold.
    Low,
    /// Below every flood stage.
    Normal,
    /// At or above the action stage.
    Elevated,
    FloodMinor,
    FloodModerate,
    FloodMajor,
}

impl Role {
    const ALL: [Self; 10] = [
        Self::None,
        Self::Unknown,
        Self::NoData,
        Self::Reporting,
        Self::Low,
        Self::Normal,
        Self::Elevated,
        Self::FloodMinor,
        Self::FloodModerate,
        Self::FloodMajor,
    ];

    const fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Unknown => "unknown",
            Self::NoData => "no_data",
            Self::Reporting => "reporting",
            Self::Low => "low",
            Self::Normal => "normal",
            Self::Elevated => "elevated",
            Self::FloodMinor => "flood_minor",
            Self::FloodModerate => "flood_moderate",
            Self::FloodMajor => "flood_major",
        }
    }
}

/// Every colour role a pin's `state_role` may name, for the app's palette: `none` (no state
/// for its kind), `unknown`, `no_data`, `reporting`, `low`, `normal`, `elevated`,
/// `flood_minor`, `flood_moderate`, `flood_major`.
#[uniffi::export]
pub fn station_pin_roles() -> Vec<String> {
    Role::ALL.iter().map(|r| r.name().to_owned()).collect()
}

// ---------------------------------------------------------------------------------------------
// Layers and the view
// ---------------------------------------------------------------------------------------------

/// A station layer, in the catalogue's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum PinLayer {
    Tide,
    Current,
    RiverGauge,
    Buoy,
    ForecastGauge,
}

impl PinLayer {
    const fn of(kind: LayerStation) -> Self {
        match kind {
            LayerStation::Tide => Self::Tide,
            LayerStation::Current => Self::Current,
            LayerStation::RiverGauge => Self::RiverGauge,
            LayerStation::Buoy => Self::Buoy,
            LayerStation::ForecastGauge => Self::ForecastGauge,
        }
    }

    const fn kind(self) -> &'static str {
        match self {
            Self::Tide => "tide",
            Self::Current => "current",
            Self::RiverGauge => "river_gauge",
            Self::Buoy => "buoy",
            Self::ForecastGauge => "forecast_gauge",
        }
    }

    /// What a pin of the layer is, in words.
    fn word(self, f: &Formatter, station: StationKind) -> &'static str {
        f.strings.get(match (self, station) {
            (Self::Buoy, StationKind::Cman) => Key::PinCoastalStation,
            (Self::Buoy, _) => Key::PinBuoy,
            (Self::Tide, _) => Key::PinTideStation,
            (Self::Current, _) => Key::PinCurrentStation,
            (Self::RiverGauge, _) => Key::PinRiverGauge,
            (Self::ForecastGauge, _) => Key::PinForecastGauge,
        })
    }
}

/// The station layers the settings show, with their catalogue ids.
fn shown_layers(config: &Config) -> BTreeMap<PinLayer, &'static str> {
    catalogue::ids()
        .filter_map(catalogue::find)
        .filter_map(|l| match l.kind {
            LayerKind::Stations(kind) => Some((PinLayer::of(kind), l.id)),
            _ => None,
        })
        .filter(|(_, id)| {
            config
                .map
                .layers
                .iter()
                .any(|st| st.id.as_str() == *id && st.shown)
        })
        .collect()
}

/// The map's box, in degrees; `west` > `east` crosses the antimeridian.
#[derive(Clone, Copy, Debug, PartialEq)]
struct View {
    west: f64,
    south: f64,
    east: f64,
    north: f64,
}

impl View {
    fn new(west: f64, south: f64, east: f64, north: f64) -> Result<Self, MapError> {
        let lat = |v: f64| v.is_finite() && (-90.0..=90.0).contains(&v);
        let lon = |v: f64| v.is_finite() && (-180.0..=180.0).contains(&v);
        if !(lat(south) && lat(north) && lon(west) && lon(east)) || south > north {
            return Err(MapError::Invalid {
                message: "a view needs edges in range, south of its north".into(),
            });
        }
        Ok(Self {
            west,
            south,
            east,
            north,
        })
    }

    fn contains(&self, lat: f64, lon: f64) -> bool {
        let across = if self.west <= self.east {
            (self.west..=self.east).contains(&lon)
        } else {
            lon >= self.west || lon <= self.east
        };
        (self.south..=self.north).contains(&lat) && across
    }

    /// The longitude ranges the view spans: two when it crosses the antimeridian.
    fn lon_ranges(&self) -> Vec<(f64, f64)> {
        if self.west <= self.east {
            vec![(self.west, self.east)]
        } else {
            vec![(self.west, 180.0), (-180.0, self.east)]
        }
    }

    fn centre(&self) -> (f64, f64) {
        let width = (self.east - self.west).rem_euclid(360.0);
        let lon = self.west + width / 2.0;
        let lon = if lon > 180.0 { lon - 360.0 } else { lon };
        (f64::midpoint(self.south, self.north), lon)
    }

    /// How far `(lat, lon)` is from the view's centre, for ordering only (squared degrees).
    fn off_centre(&self, lat: f64, lon: f64) -> f64 {
        let (clat, clon) = self.centre();
        let dlon = (lon - clon + 540.0).rem_euclid(360.0) - 180.0;
        (lat - clat).powi(2) + dlon.powi(2)
    }
}

/// A cell of the list grid, by index: its centre is `index × CELL_DEG`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Cell {
    lat: i32,
    lon: i32,
}

impl Cell {
    fn index(degrees: f64) -> i32 {
        // INVARIANT: degrees are checked to ±180, so the index fits.
        ((degrees + CELL_HALF_DEG) / CELL_DEG).floor() as i32
    }

    fn centre(self) -> Coordinates {
        Coordinates {
            latitude: f64::from(self.lat) * CELL_DEG,
            longitude: f64::from(self.lon) * CELL_DEG,
        }
    }

    /// The centre as it is sent: whole or half degrees.
    fn target(self, now: UnixTime) -> Target {
        Target {
            place: Place::Point(RoundedPoint::new(self.centre(), 1)),
            measurements: BTreeSet::new(),
            window: Window { from: now, to: now },
            datum: None,
        }
    }

    /// The key the cell's list is stored under: a hash, so no coordinates name a stored value.
    fn key(self, list: &str) -> String {
        format!(
            "map-{list}-{}",
            spot_key(&format!("{},{}", self.lat, self.lon))
        )
    }
}

/// The cells under `view`, nearest its centre first; `None` when it spans more than
/// [`MAX_AREA_CELLS`].
fn cells(view: &View) -> Option<Vec<Cell>> {
    let lats = Cell::index(view.south)..=Cell::index(view.north);
    let mut lons = BTreeSet::new();
    for (west, east) in view.lon_ranges() {
        let (first, last) = (Cell::index(west), Cell::index(east));
        if usize::try_from(last - first).ok()? >= MAX_AREA_CELLS {
            return None;
        }
        // 180° east is 180° west.
        lons.extend((first..=last).map(|i| if i >= 360 { i - 720 } else { i }));
    }
    if lats.clone().count() * lons.len() > MAX_AREA_CELLS {
        return None;
    }
    let mut out: Vec<Cell> = lats
        .flat_map(|lat| lons.iter().map(move |&lon| Cell { lat, lon }))
        .collect();
    out.sort_by(|a, b| {
        let off = |c: &Cell| {
            let p = c.centre();
            view.off_centre(p.latitude, p.longitude)
        };
        off(a).total_cmp(&off(b)).then(a.cmp(b))
    });
    Some(out)
}

// ---------------------------------------------------------------------------------------------
// Through the host
// ---------------------------------------------------------------------------------------------

/// A station list as discovery and the map store it.
#[derive(Debug, Default, Serialize, Deserialize)]
struct StoredStations {
    #[serde(default)]
    candidates: Vec<StationCandidate>,
    /// Some of the searches behind the list failed: it is kept for
    /// [`PARTIAL_SITES_TTL_S`] only.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    partial: bool,
}

impl StoredStations {
    /// How long the list answers before it is fetched again.
    const fn ttl_s(&self) -> i64 {
        if self.partial {
            PARTIAL_SITES_TTL_S
        } else {
            STATION_LIST_TTL_S
        }
    }
}

/// A cell's NWPS gauges with their observed flood categories.
#[derive(Debug, Default, Serialize, Deserialize)]
struct StoredGauges {
    #[serde(default)]
    gauges: Vec<GaugeState>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct GaugeState {
    station: StationRef,
    flood_category: FloodCategory,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    observed_at: Option<UnixTime>,
}

impl From<ListedGauge> for GaugeState {
    fn from(g: ListedGauge) -> Self {
        Self {
            station: g.candidate.station,
            flood_category: g.flood_category,
            observed_at: g.observed_at,
        }
    }
}

/// The host for one call: requests one at a time, the stored lists, and the call ledger, which
/// counts and saves a metered request before it goes out.
struct Link<'h> {
    host: &'h dyn FetchHost,
    now: UnixTime,
    ledger: Option<UsageLedger>,
}

impl Link<'_> {
    /// A list stored under `key`, however old.
    fn load<T: DeserializeOwned>(&self, key: &str) -> Option<Stored<T>> {
        let text = self.host.load(store::STATIONS.to_owned(), key.to_owned())?;
        serde_json::from_str(&text).ok()
    }

    /// Stores `value` under `key`, stamped with the call's time.
    fn keep<T: Serialize>(&self, key: &str, value: &T) {
        let stored = Stored {
            saved_at: self.now.0,
            value,
        };
        if let Ok(json) = serde_json::to_string(&stored) {
            self.host
                .store(store::STATIONS.to_owned(), key.to_owned(), json);
        }
    }

    /// Whether a value saved at `saved_at` is at most `ttl_s` old.
    fn fresh(&self, saved_at: i64, ttl_s: i64) -> bool {
        (0..=ttl_s).contains(&(self.now.0.saturating_sub(saved_at)))
    }

    fn ledger(&mut self) -> &mut UsageLedger {
        let host = self.host;
        let now = self.now;
        self.ledger.get_or_insert_with(|| {
            let mut ledger = host
                .load(store::USAGE.to_owned(), store::LEDGER_KEY.to_owned())
                .and_then(|text| serde_json::from_str::<Stored<UsageLedger>>(&text).ok())
                .map(|stored| stored.value)
                .unwrap_or_default();
            ledger.drop_distant_pauses(now);
            ledger
        })
    }

    fn save_ledger(&mut self) -> Result<(), String> {
        let saved_at = self.now.0;
        let json = serde_json::to_string(&Stored {
            saved_at,
            value: self.ledger(),
        })
        .map_err(|e| e.to_string())?;
        self.host.save_usage(json).map_err(|e| e.to_string())
    }

    /// Sends a request of `source`; a metered one only once the ledger has counted and saved
    /// it, and not at all when the budget is spent.
    async fn get(
        &mut self,
        source: &dyn Source,
        request: &Request,
    ) -> Result<Fetched, SourceError> {
        let now = self.now;
        let id = source.id();
        let budget = if request.metered {
            let budget = source.budget().ok_or_else(|| {
                SourceError::Unsupported("a metered request without a budget".into())
            })?;
            self.ledger().try_spend(&id, budget, now).map_err(|quota| {
                SourceError::RateLimited {
                    retry_after_s: Some(quota.retry_after_s(now)),
                }
            })?;
            self.save_ledger().map_err(|e| {
                SourceError::Transport(format!("the call ledger was not saved: {e}"))
            })?;
            Some(budget)
        } else {
            None
        };
        let call = Call::from(request);
        let response = self
            .host
            .send(host_request(&call))
            .await
            .map_err(|e| SourceError::Transport(scrub(&e.to_string(), &call.url)))?;
        let fetched = fetched(response).map_err(|e| SourceError::Transport(e.to_string()))?;
        if let Some(budget) = budget {
            let ledger = self.ledger();
            if let Some(left) = fetched.rate_limit_remaining {
                ledger.observe_remaining(&id, budget, left, now);
            }
            if fetched.status == 429 {
                ledger.rate_limited(&id, fetched.retry_after, now);
            }
            // The call was counted before it went out; this only keeps what the response said.
            let _ = self.save_ledger();
        }
        Ok(fetched)
    }
}

/// `message` with `url` redacted and every secret of it masked.
fn scrub(message: &str, url: &str) -> String {
    let mut out = message.replace(url, &redact_url(url));
    for secret in secret_values(url) {
        out = out.replace(secret, "***");
    }
    out
}

/// A snapshot error for `source` about `what`.
fn issue(source: SourceId, what: &str, error: &SourceError) -> ModelSourceError {
    let mut e = error.to_model(source, None);
    e.message = format!("{what}: {}", e.message);
    e
}

// ---------------------------------------------------------------------------------------------
// The lists
// ---------------------------------------------------------------------------------------------

/// A whole station list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WholeList {
    Coops(StationList),
    Ndbc,
}

impl WholeList {
    /// The key discovery stores the list under; the map's own copy is `map-` and this.
    fn key(self) -> String {
        match self {
            Self::Coops(list) => format!("coops-{}", list.type_name()),
            Self::Ndbc => "ndbc-active".to_owned(),
        }
    }

    const fn source(self) -> SourceId {
        match self {
            Self::Coops(_) => SourceId::COOPS,
            Self::Ndbc => SourceId::NDBC,
        }
    }

    fn request(self, target: Target) -> Request {
        match self {
            Self::Coops(list) => mdapi::stations_request(list, target),
            Self::Ndbc => ndbc::stations::request(target),
        }
    }

    fn parse(
        self,
        request: &Request,
        fetched: &Fetched,
    ) -> Result<Vec<StationCandidate>, SourceError> {
        let response = fetched.as_response();
        match self {
            Self::Coops(_) => match mdapi::parse(request, &response)? {
                Metadata::Stations(candidates) => Ok(candidates),
                _ => Err(SourceError::Parse("not a station list".into())),
            },
            Self::Ndbc => {
                let body = check_response(request, &response)?;
                let text =
                    std::str::from_utf8(body).map_err(|e| SourceError::Parse(e.to_string()))?;
                ndbc::stations::parse(text)
            }
        }
    }
}

/// A whole list: a stored copy (discovery's first, then the map's) while the offline cache
/// would serve it, else fetched and stored; the youngest older copy when the fetch fails.
async fn whole_list(
    link: &mut Link<'_>,
    list: WholeList,
    target: &Target,
    errors: &mut Vec<ModelSourceError>,
) -> Vec<StationCandidate> {
    let request = list.request(target.clone());
    let ttl =
        MetadataKind::of_url(&request.url).map_or(STATION_LIST_TTL_S, MetadataKind::max_age_s);
    let ours = format!("map-{}", list.key());
    let mut stored: Option<Stored<StoredStations>> = None;
    for key in [list.key(), ours.clone()] {
        if let Some(s) = link.load::<StoredStations>(&key) {
            if link.fresh(s.saved_at, ttl) {
                return s.value.candidates;
            }
            if stored.as_ref().is_none_or(|o| o.saved_at < s.saved_at) {
                stored = Some(s);
            }
        }
    }
    let got = match list {
        WholeList::Coops(_) => link.get(&Coops, &request).await,
        WholeList::Ndbc => link.get(&Ndbc, &request).await,
    };
    match got.and_then(|f| list.parse(&request, &f)) {
        Ok(candidates) => {
            let stored = StoredStations {
                candidates,
                partial: false,
            };
            link.keep(&ours, &stored);
            stored.candidates
        }
        Err(e) => {
            errors.push(issue(list.source(), "station list", &e));
            stored.map(|s| s.value.candidates).unwrap_or_default()
        }
    }
}

/// The USGS sites of a cell with what they serve, and the site-type searches that failed.
async fn usgs_cell(
    link: &mut Link<'_>,
    cell: Cell,
    ctx: &FetchContext,
) -> Result<(Vec<StationCandidate>, Vec<SourceError>), SourceError> {
    let usgs = Usgs::new();
    let target = cell.target(link.now);
    let mut stations: Vec<StationRef> = Vec::new();
    let mut gaps = Vec::new();
    // One request per site type; a type that fails leaves the others usable, except when the
    // budget is spent.
    for request in sites::locations_requests(&target, CELL_HALF_DEG, ctx)? {
        let found = match link.get(&usgs, &request).await {
            Ok(f) => check_response(&request, &f.as_response()).and_then(sites::parse_locations),
            Err(e) => Err(e),
        };
        match found {
            Ok(found) => {
                for station in found {
                    if !stations.iter().any(|s| s.id == station.id) {
                        stations.push(station);
                    }
                }
            }
            Err(e @ SourceError::RateLimited { .. }) => return Err(e),
            Err(e) => gaps.push(e),
        }
    }
    let request = sites::metadata_request(&target, CELL_HALF_DEG, ctx)?;
    let fetched = link.get(&usgs, &request).await?;
    let offers = sites::parse_metadata(check_response(&request, &fetched.as_response())?)?;
    Ok((sites::candidates(stations, &offers), gaps))
}

/// The NWPS gauges of a cell with their observed flood categories.
async fn nwps_cell(
    link: &mut Link<'_>,
    cell: Cell,
    ctx: &FetchContext,
) -> Result<Vec<GaugeState>, SourceError> {
    let target = cell.target(link.now);
    let request = Nwps
        .plan(&target, &Requirements::default(), ctx)?
        .into_iter()
        .next()
        .ok_or_else(|| SourceError::Unsupported("no gauge list request".into()))?
        // INVARIANT: a positive constant well under u32::MAX.
        .max_age_hint(NWPS_REFRESH_S as u32);
    let fetched = link.get(&Nwps, &request).await?;
    let body = check_response(&request, &fetched.as_response())?;
    Ok(nwps::parse_gauge_list(body)?
        .into_iter()
        .map(GaugeState::from)
        .collect())
}

/// The USGS sites of the view's cells: stored ones, and up to [`MAX_FETCHED_CELLS`] missing or
/// old ones fetched when `fetch` allows. Returns whether the budget refused a request.
async fn usgs_sites(
    link: &mut Link<'_>,
    cells: &[Cell],
    fetch: bool,
    ctx: &FetchContext,
    errors: &mut Vec<ModelSourceError>,
) -> (Vec<StationCandidate>, bool) {
    let mut out = Vec::new();
    let mut fetched = 0;
    let mut refused = false;
    for &cell in cells {
        let key = cell.key("usgs");
        let stored = link.load::<StoredStations>(&key);
        let due = stored
            .as_ref()
            .is_none_or(|s| !link.fresh(s.saved_at, s.value.ttl_s()));
        if due && fetch && !refused && fetched < MAX_FETCHED_CELLS {
            fetched += 1;
            match usgs_cell(link, cell, ctx).await {
                Ok((candidates, gaps)) => {
                    // Kept even when a site type failed, for a short while, so that panning
                    // does not search the cell again and again.
                    let stored = StoredStations {
                        candidates,
                        partial: !gaps.is_empty(),
                    };
                    link.keep(&key, &stored);
                    errors.extend(
                        gaps.iter()
                            .map(|e| issue(SourceId::USGS, "station list", e)),
                    );
                    out.extend(stored.candidates);
                    continue;
                }
                Err(e) => {
                    refused = matches!(e, SourceError::RateLimited { .. });
                    errors.push(issue(SourceId::USGS, "station list", &e));
                }
            }
        }
        out.extend(stored.into_iter().flat_map(|s| s.value.candidates));
    }
    (out, refused)
}

/// The NWPS gauges of the view's cells, as [`usgs_sites`] gets sites.
async fn nwps_gauges(
    link: &mut Link<'_>,
    cells: &[Cell],
    fetch: bool,
    ctx: &FetchContext,
    errors: &mut Vec<ModelSourceError>,
) -> Vec<GaugeState> {
    let mut out = Vec::new();
    let mut fetched = 0;
    for &cell in cells {
        let key = cell.key("nwps");
        let stored = link.load::<StoredGauges>(&key);
        let due = stored
            .as_ref()
            .is_none_or(|s| !link.fresh(s.saved_at, NWPS_REFRESH_S));
        if due && fetch && fetched < MAX_FETCHED_CELLS {
            fetched += 1;
            match nwps_cell(link, cell, ctx).await {
                Ok(gauges) => {
                    let stored = StoredGauges { gauges };
                    link.keep(&key, &stored);
                    out.extend(stored.gauges);
                    continue;
                }
                Err(e) => errors.push(issue(SourceId::NWPS, "station list", &e)),
            }
        }
        out.extend(stored.into_iter().flat_map(|s| s.value.gauges));
    }
    out
}

// ---------------------------------------------------------------------------------------------
// States
// ---------------------------------------------------------------------------------------------

/// What a gauge pin says about its latest value.
#[derive(Clone, Debug, PartialEq)]
enum Reading {
    /// A value in words, and when it was read.
    Value(String, UnixTime),
    /// Nothing recent.
    NoRecent,
}

/// The newest value of the first of [`PIN_MEASUREMENTS`] a parse found, in words.
fn headline(out: &Fragments, f: &Formatter) -> Option<(String, UnixTime)> {
    PIN_MEASUREMENTS.iter().find_map(|&m| {
        out.fragments()
            .iter()
            .filter(|frag| frag.measurement == m)
            .find_map(|frag| match &frag.value {
                FragmentValue::Series(series) => {
                    let s = series.latest()?;
                    let q = match m {
                        Measurement::Discharge => f.discharge(Some(s.value)),
                        Measurement::WaterTemperature => f.temperature(Some(s.value)),
                        _ => return None,
                    };
                    Some((q.full(), s.time))
                }
                FragmentValue::Heights(h) => {
                    let s = h.series.latest()?;
                    let height = Some(Height {
                        datum: h.datum,
                        metres: s.value,
                    });
                    let q = match m {
                        Measurement::GageHeight => f.stage(height),
                        Measurement::LakeLevel => f.lake_level(height),
                        Measurement::Tide => f.height(height),
                        _ => return None,
                    };
                    Some((q.full(), s.time))
                }
                _ => None,
            })
    })
}

/// The latest values of `gauges` from one `latest-continuous` request, by station id.
async fn usgs_readings(
    link: &mut Link<'_>,
    gauges: &[&StationRef],
    ctx: &FetchContext,
    f: &Formatter,
) -> Result<BTreeMap<String, Reading>, SourceError> {
    let Some(first) = gauges.first() else {
        return Ok(BTreeMap::new());
    };
    let now = link.now;
    let station_target = |station: &StationRef| Target {
        place: Place::Station {
            station: station.clone(),
            distance_m: None,
            relation: Relation::Nearby,
        },
        measurements: PIN_MEASUREMENTS.into_iter().collect(),
        window: Window { from: now, to: now },
        datum: None,
    };
    // Sorted, so the same gauges make the same URL.
    let sites: Vec<String> = gauges
        .iter()
        .map(|s| s.id.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let request = ogc::latest_request(&sites, PIN_CODES, &station_target(first), ctx);
    let usgs = Usgs::new();
    let fetched = link.get(&usgs, &request).await?;
    let response = fetched.as_response();
    check_response(&request, &response)?;
    let mut out = BTreeMap::new();
    for station in gauges {
        let mut one = request.clone();
        one.target = station_target(station);
        let mut found = Fragments::new();
        let reading = match usgs.parse(&one, &response, ctx, &mut found) {
            Ok(()) => {
                headline(&found, f).map_or(Reading::NoRecent, |(text, at)| Reading::Value(text, at))
            }
            Err(SourceError::Stale { .. } | SourceError::NoData(_)) => Reading::NoRecent,
            Err(e) => return Err(e),
        };
        out.insert(station.id.clone(), reading);
    }
    Ok(out)
}

/// Whether a USGS site's list says it reported within [`USGS_REPORTING_WITHIN_S`].
fn reports(candidate: &StationCandidate, now: UnixTime) -> bool {
    candidate
        .offers
        .values()
        .filter_map(|c| c.last_value_at)
        .any(|t| now.0.saturating_sub(t.0) <= USGS_REPORTING_WITHIN_S)
}

/// The role and words of an NWPS gauge's observed flood category.
fn flood_state(gauge: &GaugeState, now: UnixTime, f: &Formatter) -> (Role, Option<String>) {
    let s = f.strings;
    let (role, word) = match gauge.flood_category {
        FloodCategory::NoFlooding => (Role::Normal, s.safety(SafetyKey::PinNoFlooding)),
        FloodCategory::Action => (Role::Elevated, s.safety(SafetyKey::PinActionStage)),
        FloodCategory::Minor => (Role::FloodMinor, s.safety(SafetyKey::PinMinorFlooding)),
        FloodCategory::Moderate => (
            Role::FloodModerate,
            s.safety(SafetyKey::PinModerateFlooding),
        ),
        FloodCategory::Major => (Role::FloodMajor, s.safety(SafetyKey::PinMajorFlooding)),
        FloodCategory::LowThreshold => (Role::Low, s.safety(SafetyKey::PinLowWater)),
        FloodCategory::NotDefined => (Role::Reporting, s.get(Key::PinNoFloodStages)),
        FloodCategory::ObsNotCurrent => {
            return (Role::NoData, Some(s.get(Key::PinNoRecentValue).to_owned()));
        }
        FloodCategory::OutOfService => {
            return (
                Role::NoData,
                Some(s.safety(SafetyKey::PinOutOfService).to_owned()),
            );
        }
        FloodCategory::FcstNotCurrent | FloodCategory::Unknown => return (Role::Unknown, None),
    };
    match gauge.observed_at {
        Some(at) if now.0.saturating_sub(at.0) <= NWPS_STATE_MAX_AGE_S => (
            role,
            Some(s.fmt(
                Key::PinStateAge,
                &[("state", word), ("age", &f.age(now.0.saturating_sub(at.0)))],
            )),
        ),
        // Too old to say what the river does now.
        _ => (Role::Unknown, None),
    }
}

// ---------------------------------------------------------------------------------------------
// Pins
// ---------------------------------------------------------------------------------------------

/// The pins of a call, one per station, in layer order then by id.
#[derive(Debug, Default)]
struct Pins(BTreeMap<(PinLayer, String), StationPin>);

impl Pins {
    fn add(
        &mut self,
        layer: PinLayer,
        layer_id: &str,
        station: &StationRef,
        role: Role,
        state: Option<String>,
        f: &Formatter,
    ) {
        let id = format!("{}:{}", station.source, station.id);
        let kind = layer.word(f, station.kind);
        let label = match state {
            Some(state) => f
                .strings
                .fmt(Key::PinLabel, &[("kind", kind), ("state", &state)]),
            None => kind.to_owned(),
        };
        self.0.entry((layer, id.clone())).or_insert(StationPin {
            id,
            kind: layer.kind().to_owned(),
            layer: layer_id.to_owned(),
            name: station.name.clone(),
            lat: station.latitude,
            lon: station.longitude,
            state_role: role.name().to_owned(),
            label,
        });
    }

    /// The stations of `candidates` in `view`, without state.
    fn plain(
        &mut self,
        layer: PinLayer,
        layer_id: &str,
        candidates: &[StationCandidate],
        view: &View,
        f: &Formatter,
    ) {
        for c in candidates {
            if view.contains(c.station.latitude, c.station.longitude) {
                self.add(layer, layer_id, &c.station, Role::None, None, f);
            }
        }
    }
}

/// One call: the host, the view, the words, and the pins and errors found so far.
struct Pass<'h> {
    link: Link<'h>,
    view: View,
    ctx: FetchContext,
    f: Formatter,
    /// Close enough in for box lists to be fetched and gauge states asked for.
    close: bool,
    pins: Pins,
    errors: Vec<ModelSourceError>,
}

impl Pass<'_> {
    /// The pins of the whole lists of the shown layers.
    async fn whole_lists(&mut self, shown: &BTreeMap<PinLayer, &'static str>) {
        // The whole lists name no place; their target is never sent.
        let (lat, lon) = self.view.centre();
        let anywhere = Cell {
            lat: Cell::index(lat),
            lon: Cell::index(lon),
        }
        .target(self.link.now);
        let wholes = [
            (
                PinLayer::Tide,
                WholeList::Coops(StationList::TidePredictions),
            ),
            (PinLayer::Tide, WholeList::Coops(StationList::WaterLevels)),
            (
                PinLayer::Current,
                WholeList::Coops(StationList::CurrentPredictions),
            ),
            (PinLayer::Buoy, WholeList::Ndbc),
        ];
        for (layer, list) in wholes {
            if let Some(&layer_id) = shown.get(&layer) {
                let candidates =
                    whole_list(&mut self.link, list, &anywhere, &mut self.errors).await;
                self.pins
                    .plain(layer, layer_id, &candidates, &self.view, &self.f);
            }
        }
    }

    /// The USGS gauge pins of `cells`, the reporting ones nearest the centre with their
    /// latest values.
    async fn river_gauges(&mut self, layer_id: &str, cells: &[Cell]) {
        let now = self.link.now;
        let (sites, refused) = usgs_sites(
            &mut self.link,
            cells,
            self.close,
            &self.ctx,
            &mut self.errors,
        )
        .await;
        let view = self.view;
        let mut visible: Vec<&StationCandidate> = sites
            .iter()
            .filter(|c| view.contains(c.station.latitude, c.station.longitude))
            .collect();
        visible.sort_by(|a, b| {
            let off =
                |c: &StationCandidate| view.off_centre(c.station.latitude, c.station.longitude);
            off(a)
                .total_cmp(&off(b))
                .then_with(|| a.station.id.cmp(&b.station.id))
        });
        let mut seen = BTreeSet::new();
        visible.retain(|c| seen.insert(c.station.id.clone()));
        let asked: Vec<&StationRef> = visible
            .iter()
            .filter(|c| reports(c, now))
            .map(|c| &c.station)
            .take(MAX_STATE_GAUGES)
            .collect();
        let readings = if self.close && !refused {
            match usgs_readings(&mut self.link, &asked, &self.ctx, &self.f).await {
                Ok(readings) => readings,
                Err(e) => {
                    self.errors.push(issue(SourceId::USGS, "gauge values", &e));
                    BTreeMap::new()
                }
            }
        } else {
            BTreeMap::new()
        };
        let s = self.f.strings;
        for c in visible {
            let (role, state) = match readings.get(&c.station.id) {
                Some(Reading::Value(text, at)) => (
                    Role::Reporting,
                    Some(s.fmt(
                        Key::PinStateAge,
                        &[
                            ("state", text.as_str()),
                            ("age", &self.f.age(now.0.saturating_sub(at.0))),
                        ],
                    )),
                ),
                Some(Reading::NoRecent) => {
                    (Role::NoData, Some(s.get(Key::PinNoRecentValue).to_owned()))
                }
                None if !reports(c, now) => {
                    (Role::NoData, Some(s.get(Key::PinNoRecentValue).to_owned()))
                }
                None => (Role::Unknown, None),
            };
            self.pins.add(
                PinLayer::RiverGauge,
                layer_id,
                &c.station,
                role,
                state,
                &self.f,
            );
        }
    }

    /// The NWPS gauge pins of `cells` with their observed flood categories.
    async fn forecast_gauges(&mut self, layer_id: &str, cells: &[Cell]) {
        let gauges = nwps_gauges(
            &mut self.link,
            cells,
            self.close,
            &self.ctx,
            &mut self.errors,
        )
        .await;
        for gauge in gauges
            .iter()
            .filter(|g| self.view.contains(g.station.latitude, g.station.longitude))
        {
            let (role, state) = flood_state(gauge, self.link.now, &self.f);
            self.pins.add(
                PinLayer::ForecastGauge,
                layer_id,
                &gauge.station,
                role,
                state,
                &self.f,
            );
        }
    }

    fn finish(self) -> MapStations {
        let mut issues: Vec<ModelSourceError> = Vec::new();
        for e in self.errors {
            if !issues.contains(&e) {
                issues.push(e);
            }
        }
        MapStations {
            pins: self.pins.0.into_values().collect(),
            errors: issues.iter().map(SourceIssue::from).collect(),
        }
    }
}

/// Every station of the shown station layers in the view `west`, `south`, `east`, `north`
/// (degrees; `west` > `east` crosses the antimeridian) at map zoom `zoom`, through `host`, at
/// Unix time `now`, with each pin's state worded in the settings' units and voice.
///
/// Lists come from what the host stores, else are fetched (see the module notes); from zoom 9
/// on, the box lists of a few cells are fetched and the latest values of at most
/// [`MAX_STATE_GAUGES`] USGS gauges asked for in one metered request, which the call ledger
/// may refuse. `keys` carries the USGS key for those requests. A list or state that fails is
/// listed in the errors; the call fails only on a bad view or unreadable settings.
#[uniffi::export]
#[allow(clippy::too_many_arguments)] // a view is four edges and a zoom
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub async fn map_stations(
    west: f64,
    south: f64,
    east: f64,
    north: f64,
    zoom: f64,
    settings_json: String,
    keys: Keys,
    now: i64,
    host: Arc<dyn FetchHost>,
) -> Result<MapStations, MapError> {
    let view = View::new(west, south, east, north)?;
    let (config, _) = load(&settings_json)?;
    let shown = shown_layers(&config);
    let now = UnixTime(now);
    let secrets = Secrets {
        usgs_api_key: keys.usgs.filter(|k| !k.trim().is_empty()),
    };
    let mut pass = Pass {
        link: Link {
            host: host.as_ref(),
            now,
            ledger: None,
        },
        view,
        ctx: FetchContext::new(now, user_agent())
            .with_coordinate_precision(config.sources.coordinate_precision)
            .with_secrets(secrets),
        f: Formatter::new(
            config.units.display_units(),
            config.presentation.voice,
            config.presentation.clock,
            "UTC",
        ),
        close: zoom.is_finite() && zoom >= CLOSE_ZOOM,
        pins: Pins::default(),
        errors: Vec::new(),
    };
    pass.whole_lists(&shown).await;
    let gauges = shown.get(&PinLayer::RiverGauge).copied();
    let forecasts = shown.get(&PinLayer::ForecastGauge).copied();
    let cells = if gauges.is_some() || forecasts.is_some() {
        cells(&view).unwrap_or_default()
    } else {
        Vec::new()
    };
    if let Some(layer_id) = gauges {
        pass.river_gauges(layer_id, &cells).await;
    }
    if let Some(layer_id) = forecasts {
        pass.forecast_gauges(layer_id, &cells).await;
    }
    Ok(pass.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use underbite_core::view::Voice;
    use whether_core::presentation::TimeFormat;

    fn view(west: f64, south: f64, east: f64, north: f64) -> View {
        View::new(west, south, east, north).expect("view")
    }

    fn formatter() -> Formatter {
        Formatter::new(
            Config::default().units.display_units(),
            Voice::Plain,
            TimeFormat::H24,
            "UTC",
        )
    }

    #[test]
    fn views_are_checked_and_may_cross_the_antimeridian() {
        assert!(View::new(-74.1, 40.6, -73.9, 40.8).is_ok());
        for (w, s, e, n) in [
            (f64::NAN, 40.6, -73.9, 40.8),
            (-74.1, 40.8, -73.9, 40.6),
            (-74.1, -91.0, -73.9, 40.8),
            (-181.0, 40.6, -73.9, 40.8),
            (-74.1, 40.6, f64::INFINITY, 40.8),
        ] {
            assert!(View::new(w, s, e, n).is_err(), "{w} {s} {e} {n}");
        }
        let pacific = view(179.0, 50.0, -179.0, 52.0);
        assert!(pacific.contains(51.0, 179.5) && pacific.contains(51.0, -179.5));
        assert!(!pacific.contains(51.0, 0.0));
        let (_, lon) = pacific.centre();
        assert!((lon.abs() - 180.0).abs() < 1e-9, "{lon}");
    }

    #[test]
    fn cells_cover_the_view_nearest_first_and_name_no_place() {
        // The Battery: two cells, centred on 40.5 and 41.0 N at 74 W.
        let battery = view(-74.10, 40.62, -73.95, 40.78);
        let found = cells(&battery).expect("cells");
        let centres: Vec<(f64, f64)> = found
            .iter()
            .map(|c| (c.centre().latitude, c.centre().longitude))
            .collect();
        assert_eq!(centres, [(40.5, -74.0), (41.0, -74.0)]);
        let key = found[0].key("usgs");
        assert!(key.starts_with("map-usgs-") && !key.contains('.') && !key.contains(','));
        let target = found[0].target(UnixTime(0));
        let Place::Point(point) = target.place else {
            panic!("a point");
        };
        assert_eq!(point.fmt_param(), ("40.5".to_owned(), "-74".to_owned()));
        // Across the antimeridian 180° E and 180° W are one cell.
        let pacific = cells(&view(179.6, 51.0, -179.6, 51.1)).expect("cells");
        assert_eq!(pacific.len(), 3, "{pacific:?}");
        assert!(pacific.iter().all(|c| (-360..360).contains(&c.lon)));
        // Too far out for box lists.
        assert_eq!(cells(&view(-80.0, 38.0, -70.0, 44.0)), None);
        assert_eq!(cells(&view(-180.0, 0.0, 180.0, 1.0)), None);
    }

    #[test]
    fn every_role_has_a_name() {
        let roles = station_pin_roles();
        assert_eq!(roles.len(), 10);
        assert_eq!(roles.iter().collect::<BTreeSet<_>>().len(), 10);
        assert!(
            roles
                .iter()
                .all(|r| !r.contains('#') && !r.starts_with("rgb"))
        );
    }

    fn gauge(category: FloodCategory, age_s: Option<i64>) -> GaugeState {
        GaugeState {
            station: StationRef {
                source: SourceId::NWPS,
                id: "PMTN6".into(),
                name: "Hudson River at Piermont (IN MLLW)".into(),
                kind: StationKind::NwpsGauge,
                latitude: 41.04305,
                longitude: -73.8961,
                bin: None,
                reference_id: None,
            },
            flood_category: category,
            observed_at: age_s.map(|s| UnixTime(1_000_000 - s)),
        }
    }

    /// A host that answers every request 429 with the longest `Retry-After`.
    struct Limited;

    #[async_trait::async_trait]
    impl FetchHost for Limited {
        async fn send(
            &self,
            _request: crate::fetch::HostRequest,
        ) -> Result<crate::fetch::HostResponse, crate::fetch::HostError> {
            Ok(crate::fetch::HostResponse {
                status: 429,
                body: Vec::new(),
                content_type: None,
                date: None,
                expires: None,
                last_modified: None,
                etag: None,
                retry_after: Some("4294967295".into()),
                rate_limit_remaining: None,
                from_cache: false,
            })
        }

        fn load(&self, _namespace: String, _key: String) -> Option<String> {
            None
        }

        fn store(&self, _namespace: String, _key: String, _value: String) {}

        fn save_usage(&self, _ledger_json: String) -> Result<(), crate::fetch::HostError> {
            Ok(())
        }
    }

    #[test]
    fn a_429_on_the_gauge_request_pauses_the_pool_for_a_day_at_most() {
        let now = UnixTime(1_794_780_000);
        let host = Limited;
        let mut link = Link {
            host: &host,
            now,
            ledger: None,
        };
        let station = StationRef {
            source: SourceId::USGS,
            id: "01375000".into(),
            name: "Croton River".into(),
            kind: StationKind::UsgsSite(underbite_core::provenance::UsgsSiteType::Stream),
            latitude: 41.2,
            longitude: -73.8,
            bin: None,
            reference_id: None,
        };
        let ctx = FetchContext::new(now, "underbite/test");
        let f = formatter();
        let _ = pollster::block_on(usgs_readings(&mut link, &[&station], &ctx, &f));
        let until = link.ledger().paused_until(&SourceId::USGS, now);
        assert_eq!(until, Some(UnixTime(now.0 + 86_400)));
    }

    #[test]
    fn times_from_any_clock_never_overflow_an_age() {
        let f = formatter();
        let mut long_ago = gauge(FloodCategory::Action, Some(0));
        long_ago.observed_at = Some(UnixTime(i64::MIN));
        assert_eq!(
            flood_state(&long_ago, UnixTime(i64::MAX), &f),
            (Role::Unknown, None)
        );
    }

    #[test]
    fn flood_categories_take_roles_and_words() {
        let f = formatter();
        let now = UnixTime(1_000_000);
        let state = |c, age| flood_state(&gauge(c, age), now, &f);
        assert_eq!(
            state(FloodCategory::NoFlooding, Some(1_800)),
            (Role::Normal, Some("No flooding, 30 min ago".into()))
        );
        assert_eq!(
            state(FloodCategory::Action, Some(60)),
            (Role::Elevated, Some("Action stage, 1 min ago".into()))
        );
        assert_eq!(state(FloodCategory::Major, Some(0)).0, Role::FloodMajor);
        assert_eq!(
            state(FloodCategory::Moderate, Some(0)).0,
            Role::FloodModerate
        );
        assert_eq!(state(FloodCategory::Minor, Some(0)).0, Role::FloodMinor);
        assert_eq!(state(FloodCategory::LowThreshold, Some(0)).0, Role::Low);
        assert_eq!(state(FloodCategory::NotDefined, Some(0)).0, Role::Reporting);
        assert_eq!(
            state(FloodCategory::OutOfService, None),
            (Role::NoData, Some("Out of service".into()))
        );
        assert_eq!(state(FloodCategory::ObsNotCurrent, None).0, Role::NoData);
        assert_eq!(
            state(FloodCategory::Unknown, Some(0)),
            (Role::Unknown, None)
        );
        // A category from hours ago says nothing about the river now.
        assert_eq!(
            state(FloodCategory::Action, Some(NWPS_STATE_MAX_AGE_S + 1)),
            (Role::Unknown, None)
        );
    }

    #[test]
    fn pins_are_worded_one_per_station() {
        let f = formatter();
        let g = gauge(FloodCategory::NoFlooding, Some(0));
        let mut pins = Pins::default();
        pins.add(
            PinLayer::ForecastGauge,
            "nwps-gauges",
            &g.station,
            Role::Normal,
            Some("No flooding, just now".into()),
            &f,
        );
        pins.add(
            PinLayer::ForecastGauge,
            "nwps-gauges",
            &g.station,
            Role::Unknown,
            None,
            &f,
        );
        let pin = pins.0.values().next().expect("pin");
        assert_eq!(pins.0.len(), 1);
        assert_eq!(pin.id, "nwps:PMTN6");
        assert_eq!(pin.kind, "forecast_gauge");
        assert_eq!(pin.state_role, "normal");
        assert_eq!(pin.label, "River forecast gauge · No flooding, just now");
    }

    #[test]
    fn host_messages_never_quote_a_key() {
        let url = "https://api.example.test/a?site=1&api_key=SECRET-1";
        let text = scrub(&format!("could not reach {url} (SECRET-1)"), url);
        assert!(!text.contains("SECRET-1"), "{text}");
    }
}
