//! The water at a dropped marker, planned and read for the Swift host.
//!
//! [`plan_water_here`] is called when the point card opens. The app sends the plan's
//! [`WaterHerePlan::requests`] through its `URLSession` host, hands the answers to
//! [`WaterHerePlan::read`] and repeats while the plan holds requests, like the nearby plans.
//! No I/O happens here.
//!
//! - **The water**: one flowline query and one waterbody query from 3DHP (a next page of each
//!   at most once). Their boxes are centred on the tapped point rounded to the settings'
//!   `sources.coordinate_precision` (at most 2 decimals) and reach 1.5 km plus the rounding
//!   cell, so they cover the point whatever the rounding; the exact point never leaves the
//!   phone, and the snapping to the water happens here, on the phone. The answers may be
//!   reused for 30 days.
//! - **The regions** of the point: the region the app names, if any, then every state the
//!   point is in or beside, from the boundaries bundled with the app (`discover::states`). The
//!   lookup is local; nothing about the exact point is asked of anyone.
//! - **Stocking**, only when New York is among the regions and the water's name and place
//!   match a stocked water of the bundled catalogue: the agency's lists for that county and water, as
//!   the nearby stocking plan asks them. Those requests name a county and a water, no point.
//! - **Regulation and licence links** come from the bundled catalogue for each of the regions,
//!   as links with the date each was checked.
//!
//! Every word is the view's.

use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};

use underbite_config::schema::Config;
use underbite_config::view_input::view_input;
use underbite_core::discover::hydro::{Flowline, HydroOptions, Point, Waterbody};
use underbite_core::discover::states::states_at;
use underbite_core::discover::tidal::Tide;
use underbite_core::discover::water_here::{WaterHere, WaterKind, water_here};
use underbite_core::model::{Spot, SpotSnapshot, StockingEvent};
use underbite_core::provenance::SourceId;
use underbite_core::time::UnixTime;
use underbite_core::view::cards::stocking::{self, StockingVm};
use underbite_core::view::format::Formatter;
use underbite_core::view::water_here as view;
use underbite_core::view::water_here::{WaterHereInput, WaterHereVm, WaterHereWords};
use underbite_providers::hydro::{self, HydroQuery};
use underbite_providers::request::Coordinates;
use underbite_providers::source::FetchContext;
use underbite_providers::stocking::catalogue::{
    Catalogue, Kind as CatalogueKind, Placed, canonical_county, canonical_waterbody, name_matches,
};
use underbite_providers::stocking::socrata_ny::{self, Dataset};

use crate::fetch::{Header, HostRequest, HostResponse, fetched};
use crate::map::{MapPoint, formatter};
use crate::nearby::{NearbyError, NearbyRequest, NearbyResponse, get};
use crate::settings::load;

/// How far around the point the water is looked for, metres (before the rounding cell is
/// added): well beyond the snap and shore tolerances of [`HydroOptions`].
pub const SEARCH_RADIUS_M: f64 = hydro::MIN_RADIUS_M;

/// The region whose stocking lists are bundled.
const NY: &str = "US-NY";
/// How long a stocking list may be reused, seconds.
const DAY_S: u32 = 86_400;
/// How far from the tapped point the representative point of a stocked lake or pond may be, and
/// of a stocked river or creek (a line, so its point can lie far along it), metres.
const STOCKED_STILL_M: f64 = 10_000.0;
const STOCKED_RUNNING_M: f64 = 40_000.0;

/// Where a lookup stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum WaterHereStatus {
    /// No answer yet: show the loading words.
    Looking,
    /// There is water here: show the card.
    Found,
    /// No water within reach: show no section.
    Nothing,
    /// The lookup failed: show the failure words.
    Failed,
}

/// A link to an official page.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct WaterHereLink {
    /// "Freshwater regulations".
    pub title: String,
    /// "NYS DEC, link checked 2026-03-01".
    pub detail: String,
    pub url: String,
}

/// A source to credit.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct WaterHereCredit {
    pub name: String,
    pub text: String,
    pub url: Option<String>,
    pub licence: Option<String>,
}

/// The section for the water at the marker.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct WaterHereCard {
    /// The water's name, or "Unnamed stream".
    pub title: String,
    /// The source gave the water a name.
    pub named: bool,
    /// `stream`, `river`, `lake`, `pond`, `reservoir`, `tidal_river`, `estuary`, `bay` or
    /// `waterbody`.
    pub kind: String,
    /// "Stream", "Tidal river".
    pub kind_text: String,
    /// "The marker is on this water" or "120 ft from the marker".
    pub position: String,
    /// The water type id a new spot here carries (`river_stream`).
    pub water_type: String,
    /// The water type in words: "River or stream"; empty when the kind already says it.
    pub water_type_text: String,
    /// The name a new spot starts with; none for unnamed water.
    pub spot_name: Option<String>,
    pub stocking_title: Option<String>,
    /// The agency's stocking lines for this water.
    pub stocking: Vec<String>,
    /// "Show all (6)": the disclosure for the lines beyond the first two; none when all fit.
    pub stocking_show_all: Option<String>,
    pub links_title: Option<String>,
    /// Said when the links are those of several states: "On the New York–New Jersey line: …".
    pub links_note: Option<String>,
    pub links: Vec<WaterHereLink>,
    /// The regulations disclaimer, shown with the links.
    pub disclaimer: Option<String>,
    pub credits: Vec<WaterHereCredit>,
}

/// What the lookup has so far.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct WaterHereResult {
    pub status: WaterHereStatus,
    /// The section, with [`WaterHereStatus::Found`].
    pub card: Option<WaterHereCard>,
    /// The failure words, with [`WaterHereStatus::Failed`].
    pub message: Option<String>,
}

/// The words around the section that do not depend on the water.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct WaterHereTexts {
    /// "Water here".
    pub title: String,
    /// "Looking up the water here…".
    pub loading: String,
    /// "USGS 3DHP could not be reached".
    pub failed: String,
    /// "Add as spot".
    pub add_as_spot: String,
}

impl From<WaterHereWords> for WaterHereTexts {
    fn from(w: WaterHereWords) -> Self {
        Self {
            title: w.title,
            loading: w.loading,
            failed: w.failed,
            add_as_spot: w.add_as_spot,
        }
    }
}

/// The words of the section, in the settings' voice and language.
///
/// # Errors
/// [`NearbyError::Invalid`] for unreadable settings.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn water_here_words(settings_json: String) -> Result<WaterHereTexts, NearbyError> {
    let (config, _) = load(&settings_json)?;
    Ok(view::words(&formatter(&config, "UTC")).into())
}

#[derive(Clone, Debug)]
enum Want {
    Flowlines(HydroQuery),
    Waterbodies(HydroQuery),
    Stocking(Dataset),
}

#[derive(Clone, Debug)]
struct Pending {
    id: String,
    queue: &'static str,
    want: Want,
    request: HostRequest,
}

struct Inner {
    config: Config,
    f: Formatter,
    now: UnixTime,
    tap: Point,
    /// The regions of the point: the one the app named, then the states the point is in or beside.
    regions: Vec<String>,
    /// A state of the point's own was added to the regions.
    from_state_lines: bool,
    tide_stations: Vec<Point>,
    round: Vec<Pending>,
    seq: u32,
    flowlines: Vec<Flowline>,
    waterbodies: Vec<Waterbody>,
    /// Whether a 3DHP answer was missing or unreadable.
    hydro_failed: bool,
    /// Whether the water has been placed (the 3DHP round is over).
    placed: bool,
    here: Option<WaterHere>,
    /// The stocked water of the catalogue this water is.
    stocked: Option<Placed>,
    events: Vec<StockingEvent>,
}

/// A water lookup, kept on the Rust side: send [`Self::requests`], give the answers to
/// [`Self::read`], and repeat while it holds requests.
#[derive(uniffi::Object)]
pub struct WaterHerePlan {
    inner: Mutex<Inner>,
}

impl fmt::Debug for WaterHerePlan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        f.debug_struct("WaterHerePlan")
            .field("requests", &inner.round.len())
            .field("placed", &inner.placed)
            .finish_non_exhaustive()
    }
}

/// Plans the lookup of the water at `point` at Unix time `now`. `region` is the ISO 3166-2
/// region the app knows the user fishes (`US-NY`), none when it is not known; the states the
/// point is in or beside, from the bundled boundaries, are added to it for the regulation links
/// and the stocking lists, so the section needs no saved region. `tide_stations` are the places of the tide stations on the
/// map, which decide whether a river here is tidal.
///
/// Call it only when the point card opens. The two 3DHP requests carry the point rounded to the
/// settings' precision, at most two decimals.
///
/// # Errors
/// [`NearbyError::Invalid`] for a point out of range or unreadable settings.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn plan_water_here(
    point: MapPoint,
    region: Option<String>,
    tide_stations: Vec<MapPoint>,
    settings_json: String,
    now: i64,
) -> Result<Arc<WaterHerePlan>, NearbyError> {
    let (config, _) = load(&settings_json)?;
    let in_range = |p: &MapPoint| {
        p.lat.is_finite()
            && p.lon.is_finite()
            && (-90.0..=90.0).contains(&p.lat)
            && (-180.0..=180.0).contains(&p.lon)
    };
    if !in_range(&point) {
        return Err(NearbyError::Invalid {
            message: "a point needs a latitude and a longitude in range".to_owned(),
        });
    }
    let now = UnixTime(now);
    let ctx = FetchContext::new(now, crate::fetch::user_agent())
        .with_coordinate_precision(config.sources.coordinate_precision);
    let at = Coordinates {
        latitude: point.lat,
        longitude: point.lon,
    };
    let invalid = |e: underbite_providers::source::SourceError| NearbyError::Invalid {
        message: e.to_string(),
    };
    let flowlines = hydro::flowlines_query(at, SEARCH_RADIUS_M, &ctx).map_err(invalid)?;
    let waterbodies = hydro::waterbodies_query(at, SEARCH_RADIUS_M, &ctx).map_err(invalid)?;
    let (regions, from_state_lines) = regions_of(region, Point::new(point.lat, point.lon));
    let mut inner = Inner {
        f: formatter(&config, "UTC"),
        config,
        now,
        tap: Point::new(point.lat, point.lon),
        regions,
        from_state_lines,
        tide_stations: tide_stations
            .iter()
            .filter(|p| in_range(p))
            .map(|p| Point::new(p.lat, p.lon))
            .collect(),
        round: Vec::new(),
        seq: 0,
        flowlines: Vec::new(),
        waterbodies: Vec::new(),
        hydro_failed: false,
        placed: false,
        here: None,
        stocked: None,
        events: Vec::new(),
    };
    inner.push_hydro("usgs-3dhp-flowlines", Want::Flowlines(flowlines));
    inner.push_hydro("usgs-3dhp-waterbodies", Want::Waterbodies(waterbodies));
    Ok(Arc::new(WaterHerePlan {
        inner: Mutex::new(inner),
    }))
}

impl Inner {
    /// Queues a 3DHP query.
    fn push_hydro(&mut self, queue: &'static str, want: Want) {
        let request = match &want {
            Want::Flowlines(q) | Want::Waterbodies(q) => get(q.url.clone(), None, q.max_age_s),
            Want::Stocking(_) => return,
        };
        self.push_request(queue, want, request);
    }

    fn push_request(&mut self, queue: &'static str, want: Want, request: HostRequest) {
        self.seq += 1;
        let source = match want {
            Want::Stocking(_) => SourceId::NY_DEC_STOCKING,
            _ => SourceId::USGS_3DHP,
        };
        self.round.push(Pending {
            id: format!("{}:{}", self.seq, source.as_str()),
            queue,
            want,
            request,
        });
    }

    fn read_round(&mut self, round: &[Pending], answers: &[(&str, &HostResponse)]) {
        let mut next: Vec<(&'static str, Want)> = Vec::new();
        for p in round {
            let answer = answers.iter().find(|(id, _)| *id == p.id).map(|(_, r)| *r);
            let fetched = answer.and_then(|r| fetched(r.clone()).ok());
            match &p.want {
                Want::Flowlines(q) => {
                    let Some(f) = &fetched else {
                        self.hydro_failed = true;
                        continue;
                    };
                    let response = f.as_response();
                    match hydro::parse_flowlines(&response) {
                        Ok(lines) => {
                            self.flowlines.extend(lines);
                            // One more page at most.
                            if q.offset == 0
                                && let Ok(Some(more)) = hydro::next_page(q, &response)
                            {
                                next.push(("usgs-3dhp-flowlines", Want::Flowlines(more)));
                            }
                        }
                        Err(_) => self.hydro_failed = true,
                    }
                }
                Want::Waterbodies(q) => {
                    let Some(f) = &fetched else {
                        self.hydro_failed = true;
                        continue;
                    };
                    let response = f.as_response();
                    match hydro::parse_waterbodies(&response) {
                        Ok(bodies) => {
                            self.waterbodies.extend(bodies);
                            if q.offset == 0
                                && let Ok(Some(more)) = hydro::next_page(q, &response)
                            {
                                next.push(("usgs-3dhp-waterbodies", Want::Waterbodies(more)));
                            }
                        }
                        Err(_) => self.hydro_failed = true,
                    }
                }
                Want::Stocking(dataset) => {
                    // A list that did not answer leaves the water without stocking lines.
                    if let Some(f) = &fetched
                        && (200..300).contains(&f.status)
                        && let Ok(events) = dataset.parse(f.as_response().body)
                    {
                        self.read_stocking(events);
                    }
                }
            }
        }
        for (queue, want) in next {
            self.push_hydro(queue, want);
        }
        if !self.placed && self.round.is_empty() {
            self.place();
        }
    }

    /// The 3DHP answers are all in: place the water, and ask the stocking lists when it is a
    /// stocked water of New York.
    fn place(&mut self) {
        self.placed = true;
        if self.hydro_failed {
            return;
        }
        self.here = water_here(
            self.tap,
            &self.flowlines,
            &self.waterbodies,
            Tide::Unknown,
            &self.tide_stations,
            HydroOptions::default(),
        );
        let Some(here) = &self.here else {
            return;
        };
        if !self.regions.iter().any(|r| r == NY) {
            return;
        }
        let Ok(catalogue) = Catalogue::bundled() else {
            return;
        };
        self.stocked = stocked_water(&catalogue, here, self.tap).cloned();
        let Some(placed) = &self.stocked else {
            return;
        };
        for r in socrata_ny::requests(&placed.county, &[placed.waterbody.as_str()]) {
            let mut request = get(r.url, None, DAY_S);
            request.headers = r
                .headers
                .iter()
                .map(|(name, value)| Header {
                    name: (*name).to_owned(),
                    value: value.clone(),
                })
                .collect();
            self.push_request("ny-dec-stocking", Want::Stocking(r.dataset), request);
        }
    }

    fn read_stocking(&mut self, events: Vec<StockingEvent>) {
        let Some(placed) = &self.stocked else {
            return;
        };
        let (county, water) = (
            canonical_county(&placed.county),
            canonical_waterbody(&placed.waterbody),
        );
        self.events.extend(events.into_iter().filter(|e| {
            canonical_waterbody(&e.waterbody) == water
                && e.county
                    .as_deref()
                    .is_none_or(|c| canonical_county(c) == county)
        }));
    }

    /// The stocking lines of the water: the stocking card's rows, actual first.
    fn stocking_lines(&self, here: &WaterHere) -> Vec<StockingVm> {
        if self.events.is_empty() {
            return Vec::new();
        }
        let Some(placed) = &self.stocked else {
            return Vec::new();
        };
        let spot = Spot {
            id: "water-here".to_owned(),
            name: placed.waterbody.clone(),
            latitude: placed.latitude,
            longitude: placed.longitude,
            water_type: here.water_type,
            water_body: None,
            time_zone: "UTC".to_owned(),
        };
        let mut snapshot = SpotSnapshot::new(spot, self.now);
        snapshot.stocking.clone_from(&self.events);
        snapshot.attribution = vec![socrata_ny::attribution()];
        stocking::build(&view_input(&self.config, snapshot, self.now))
            .map(|card| {
                let mut lines = card.full.recent;
                lines.extend(card.full.planned);
                lines
            })
            .unwrap_or_default()
    }

    fn result(&self) -> WaterHereResult {
        let looking = WaterHereResult {
            status: WaterHereStatus::Looking,
            card: None,
            message: None,
        };
        if !self.placed {
            return looking;
        }
        if self.hydro_failed {
            return WaterHereResult {
                status: WaterHereStatus::Failed,
                card: None,
                message: Some(view::words(&self.f).failed),
            };
        }
        let Some(here) = &self.here else {
            return WaterHereResult {
                status: WaterHereStatus::Nothing,
                card: None,
                message: None,
            };
        };
        let lines = self.stocking_lines(here);
        let vm = view::build(
            &self.f,
            &WaterHereInput {
                here,
                regions: &self.regions,
                from_state_lines: self.from_state_lines,
                stocking: &lines,
            },
        );
        WaterHereResult {
            status: WaterHereStatus::Found,
            card: Some(card(vm)),
            message: None,
        }
    }
}

/// The regions the links and the stocking are for: the region the app names first, then every
/// other state the point is in or beside; whether any of those was added.
fn regions_of(named: Option<String>, tap: Point) -> (Vec<String>, bool) {
    let mut regions: Vec<String> = named
        .map(|r| r.trim().to_ascii_uppercase())
        .filter(|r| !r.is_empty())
        .into_iter()
        .collect();
    let mut added = false;
    for state in states_at(tap) {
        let region = state.region();
        if !regions.contains(&region) {
            regions.push(region);
            added = true;
        }
    }
    (regions, added)
}

/// The stocked water of the catalogue that is `here`: its map name is the water's name and its
/// representative point lies near the tapped point. A river or creek is a line, so its point
/// may lie far along it; the nearest of several waters of that name wins.
fn stocked_water<'a>(catalogue: &'a Catalogue, here: &WaterHere, tap: Point) -> Option<&'a Placed> {
    let name = here.name.as_deref()?;
    let running = matches!(
        here.kind,
        WaterKind::Stream | WaterKind::River | WaterKind::TidalRiver
    );
    let limit = if running {
        STOCKED_RUNNING_M
    } else {
        STOCKED_STILL_M
    };
    catalogue
        .near(tap.latitude, tap.longitude, limit)
        .into_iter()
        .find(|(p, _)| {
            name_matches(&p.name, name)
                && match p.kind {
                    CatalogueKind::Lake | CatalogueKind::Pond => !running,
                    CatalogueKind::River => running,
                }
        })
        .map(|(p, _)| p)
}

fn water_type_id(vm: &WaterHereVm) -> String {
    serde_json::to_value(vm.water_type)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn card(vm: WaterHereVm) -> WaterHereCard {
    WaterHereCard {
        water_type: water_type_id(&vm),
        title: vm.title,
        named: vm.named,
        kind: vm.kind,
        kind_text: vm.kind_text,
        position: vm.position,
        water_type_text: vm.water_type_text,
        spot_name: vm.spot_name,
        stocking_title: vm.stocking_title,
        stocking: vm.stocking,
        stocking_show_all: vm.stocking_show_all,
        links_title: vm.links_title,
        links_note: vm.links_note,
        links: vm
            .links
            .into_iter()
            .map(|l| WaterHereLink {
                title: l.title,
                detail: l.detail,
                url: l.url,
            })
            .collect(),
        disclaimer: vm.disclaimer,
        credits: vm
            .credits
            .into_iter()
            .map(|c| WaterHereCredit {
                name: c.source,
                text: c.text,
                url: c.link,
                licence: c.licence,
            })
            .collect(),
    }
}

#[uniffi::export]
impl WaterHerePlan {
    /// The requests of the current round, in a fixed order; none once the lookup is complete.
    pub fn requests(&self) -> Vec<NearbyRequest> {
        let inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        inner
            .round
            .iter()
            .map(|p| NearbyRequest {
                id: p.id.clone(),
                source: match p.want {
                    Want::Stocking(_) => SourceId::NY_DEC_STOCKING,
                    _ => SourceId::USGS_3DHP,
                }
                .as_str()
                .to_owned(),
                queue: p.queue.to_owned(),
                delay_ms: 0,
                request: p.request.clone(),
            })
            .collect()
    }

    /// Reads the answers to the current round at Unix time `now` and plans the next (a next
    /// page, the stocking lists), which [`Self::requests`] then gives. Answers with an id the
    /// round does not hold are ignored; a request without an answer counts as failed.
    #[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
    pub fn read(&self, responses: Vec<NearbyResponse>, now: i64) -> WaterHereResult {
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        inner.now = UnixTime(now.max(inner.now.0));
        let answers: Vec<(&str, &HostResponse)> = responses
            .iter()
            .map(|r| (r.id.as_str(), &r.response))
            .collect();
        let round = std::mem::take(&mut inner.round);
        inner.read_round(&round, &answers);
        inner.result()
    }

    /// What the lookup has so far.
    pub fn result(&self) -> WaterHereResult {
        self.inner
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .result()
    }
}
