//! The beach forecast of the view: rip current risk and surf by shore zone, from the National
//! Weather Service's marine beach forecast service.
//!
//! [`plan_beach_forecast`] asks in two rounds, and as few requests as the state's caches allow:
//!
//! 1. the service's layer list (which layer holds each forecast office's Day 1 zones; kept for a
//!    week) and the forecast office of the view's centre and corners (`api.weather.gov` `/points`
//!    at two decimals; an office does not move, so kept for a month);
//! 2. for each office found, the zones of its Day 1 layer that meet the view, with their outlines.
//!
//! No coordinate leaves the phone finer than two decimals (or the settings' precision, if
//! coarser). A zone the office has issued nothing for is shown as "No forecast issued", never as a
//! low risk. Every word is the view's.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use underbite_core::map::GeoPoint;
use underbite_core::model::SourceErrorKind;
use underbite_core::provenance::SourceId;
use underbite_core::time::UnixTime;
use underbite_core::view::beach::{self, BeachFacts, RipCategory};
use underbite_core::view::strings::Key;
use underbite_providers::beach_forecast::{self as bf, RipRisk};
use underbite_providers::nws_marine::zones::point_url;
use underbite_providers::request::{Coordinates, Response, RoundedPoint};

use super::{
    Area, Inner, MapPoint, NearbyArea, NearbyError, NearbyPlan, NearbyState, Pending, Want, get,
    kind_of_status, push_unique,
};
use crate::fetch::user_agent;
use crate::settings::load;

/// How long an office of a point is kept, seconds: offices change on a scale of years.
const OFFICE_TTL_S: i64 = 30 * 86_400;
/// Most points kept with their office.
const OFFICE_ENTRIES: usize = 64;

/// The layer list, as the state keeps it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct LayerCache {
    fetched_at: i64,
    /// Office id to its Day 1 layer and, when it has one, its Day 2 layer.
    offices: BTreeMap<String, (u32, Option<u32>)>,
}

/// The forecast office of a rounded point, as the state keeps it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct OfficeCache {
    fetched_at: i64,
    /// None when the service has no office for the point (the open sea).
    office: Option<String>,
}

impl NearbyState {
    /// Drops the beach forecast caches that have expired and keeps the newest points.
    pub(super) fn prune_beaches(&mut self, now: UnixTime) {
        if self
            .beach_layers
            .as_ref()
            .is_some_and(|l| now.0 - l.fetched_at > i64::from(bf::LAYER_LIST_MAX_AGE_S))
        {
            self.beach_layers = None;
        }
        self.beach_offices
            .retain(|_, o| now.0 - o.fetched_at <= OFFICE_TTL_S);
        if self.beach_offices.len() > OFFICE_ENTRIES {
            let mut ages: Vec<(i64, String)> = self
                .beach_offices
                .iter()
                .map(|(k, o)| (o.fetched_at, k.clone()))
                .collect();
            ages.sort_by(|a, b| b.cmp(a));
            for (_, key) in ages.into_iter().skip(OFFICE_ENTRIES) {
                self.beach_offices.remove(&key);
            }
        }
    }
}

/// What the lookup has done so far.
#[derive(Clone, Debug, Default)]
pub(super) struct Run {
    /// The rounded points whose office decides the layers asked, by `lat,lon`.
    points: Vec<String>,
    /// Whether the zones of the offices have been asked for.
    queried: bool,
}

/// One outline of a zone: rings, the first the outer boundary and the others holes.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct BeachPolygon {
    pub rings: Vec<Vec<MapPoint>>,
}

/// One line of a zone's card.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct BeachRowInfo {
    /// "Surf".
    pub label: String,
    /// "Around 2 feet.", "about 11 °C".
    pub value: String,
}

/// The card of a zone, every word the view's.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct BeachCardInfo {
    /// The zone's name as the office writes it.
    pub title: String,
    /// `low`, `moderate`, `high` or `none`: also the colour role.
    pub role: String,
    /// The risk in words: "Moderate", "No forecast issued".
    pub risk: String,
    /// The risk, then the fields the office issued.
    pub rows: Vec<BeachRowInfo>,
    /// The period the forecast is for, as issued: "Rest of today".
    pub period: Option<String>,
    /// "Issued by NWS BOX, 409 PM EST 11/20/2026".
    pub issued: String,
    /// "3 h ago".
    pub age: Option<String>,
    /// The rip current safety line, shown with every card.
    pub safety: String,
    /// What a screen reader says for the zone.
    pub accessibility: String,
}

/// A shore zone of the forecast, in the view.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct BeachZonePin {
    /// Stable for the zone: the office and the zone id, `okx:nyz178`.
    pub id: String,
    pub name: String,
    /// `low`, `moderate`, `high` or `none`: the colour role of the zone's fill.
    pub role: String,
    /// Where to put the zone's marker: the mean of the first outline's outer ring.
    pub lat: f64,
    pub lon: f64,
    pub polygons: Vec<BeachPolygon>,
    pub card: BeachCardInfo,
}

/// One entry of the legend.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct BeachLegendItem {
    /// The colour role.
    pub role: String,
    /// "Low", "No forecast issued".
    pub label: String,
}

/// Plans the beach forecast lookup for the view at Unix time `now`: the zones that meet the view,
/// of the forecast offices at its centre and corners. Call it only from the handler of a user's
/// tap. `state_json` is the last [`super::NearbyResults::state_json`] (empty for none).
///
/// # Errors
/// [`NearbyError::ZoomIn`] for a view wider than [`bf::MAX_BOX_SPAN_DEG`];
/// [`NearbyError::Invalid`] for a view out of range or unreadable settings or state.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn plan_beach_forecast(
    area: NearbyArea,
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
    if !inner.area.fits(bf::MAX_BOX_SPAN_DEG) {
        return Err(NearbyError::ZoomIn {
            message: inner.f.strings.get(Key::NearbyZoomInBeaches).to_owned(),
        });
    }
    inner.beach = Some(Run::default());
    let mut first = Vec::new();
    inner.beach_discover(&mut first);
    // With everything in the state's caches, the zones are asked for at once.
    inner.beach_after_round(&mut first);
    inner.round = first;
    Ok(Arc::new(NearbyPlan {
        inner: Mutex::new(inner),
    }))
}

fn key_of(point: RoundedPoint) -> String {
    let (lat, lon) = point.fmt_param();
    format!("{lat},{lon}")
}

impl Inner {
    /// The centre and the corners of the view's boxes, rounded, once each.
    fn beach_points(&self) -> Vec<RoundedPoint> {
        let decimals = self.decimals();
        let mut points: Vec<RoundedPoint> = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        for b in self.area.rounded_parts(decimals) {
            let lat = f64::midpoint(b.south, b.north);
            let lon = f64::midpoint(b.west, b.east);
            for (latitude, longitude) in [
                (lat, lon),
                (b.south, b.west),
                (b.south, b.east),
                (b.north, b.west),
                (b.north, b.east),
            ] {
                let p = RoundedPoint::new(
                    Coordinates {
                        latitude,
                        longitude,
                    },
                    decimals,
                );
                if seen.insert(key_of(p)) {
                    points.push(p);
                }
            }
        }
        points
    }

    /// Asks for what the state does not hold: the layer list and the offices of the points.
    pub(super) fn beach_discover(&mut self, out: &mut Vec<Pending>) {
        let source = SourceId::NWS_BEACH_FORECAST;
        if self.state.beach_layers.is_none() {
            let r = bf::layer_list_request();
            let p = self.make(
                source.clone(),
                "beach-layers",
                0,
                Want::BeachLayers,
                get(r.url, None, bf::LAYER_LIST_MAX_AGE_S),
            );
            out.push(p);
        }
        let ua = user_agent();
        let mut keys = Vec::new();
        for point in self.beach_points() {
            let key = key_of(point);
            keys.push(key.clone());
            if self.state.beach_offices.contains_key(&key) {
                continue;
            }
            let p = self.make(
                source.clone(),
                "beach-points",
                0,
                Want::BeachOffice { key },
                get(point_url(point), Some(&ua), 30 * 86_400),
            );
            out.push(p);
        }
        if let Some(run) = &mut self.beach {
            run.points = keys;
        }
    }

    /// Once nothing is outstanding, asks for the zones of the offices found.
    pub(super) fn beach_after_round(&mut self, next: &mut Vec<Pending>) {
        let Some(run) = &self.beach else {
            return;
        };
        if run.queried || !next.is_empty() {
            return;
        }
        let keys = run.points.clone();
        if let Some(run) = &mut self.beach {
            run.queried = true;
        }
        let Some(layers) = self.state.beach_layers.clone() else {
            // The list could not be had: the failure is already said.
            return;
        };
        let mut offices: Vec<String> = keys
            .iter()
            .filter_map(|k| self.state.beach_offices.get(k)?.office.clone())
            .collect();
        offices.sort();
        offices.dedup();
        for office in offices {
            let Some(&(day1, _)) = layers.offices.get(&office) else {
                continue;
            };
            for bbox in self.area.rounded_parts(self.decimals()) {
                match bf::box_request(bbox, day1) {
                    Ok(r) => {
                        let p = self.make(
                            SourceId::NWS_BEACH_FORECAST,
                            "beach-zones",
                            0,
                            Want::BeachZones,
                            get(r.url, None, bf::MAX_AGE_S),
                        );
                        next.push(p);
                    }
                    Err(e) => self.issue(
                        &SourceId::NWS_BEACH_FORECAST,
                        SourceErrorKind::Other,
                        e.to_string(),
                    ),
                }
            }
        }
    }

    pub(super) fn read_beach_layers(&mut self, response: &Response<'_>) {
        match bf::parse_layer_list(response) {
            Ok(list) => {
                self.state.beach_layers = Some(LayerCache {
                    fetched_at: self.now.0,
                    offices: list
                        .offices
                        .into_iter()
                        .map(|(k, o)| (k, (o.day1, o.day2)))
                        .collect(),
                });
            }
            Err(e) => self.beach_failed(&e),
        }
    }

    pub(super) fn read_beach_office(&mut self, key: &str, response: &Response<'_>) {
        let source = SourceId::NWS_BEACH_FORECAST;
        // A point the service has no data for is not an error: it is open water.
        let office = if response.status == 404 {
            Ok(None)
        } else if (200..300).contains(&response.status) {
            bf::office_of_point(response.body).map_err(|e| e.to_string())
        } else {
            self.failed(
                &source,
                kind_of_status(response.status),
                format!("HTTP {}", response.status),
            );
            return;
        };
        match office {
            Ok(office) => {
                self.state.beach_offices.insert(
                    key.to_owned(),
                    OfficeCache {
                        fetched_at: self.now.0,
                        office,
                    },
                );
            }
            Err(message) => self.failed(&source, SourceErrorKind::Parse, message),
        }
    }

    pub(super) fn read_beach_zones(&mut self, response: &Response<'_>) {
        match bf::parse_zones(response) {
            Ok(got) => {
                if got.more {
                    self.notice(Key::NearbyMoreThanShown, &SourceId::NWS_BEACH_FORECAST);
                }
                for z in got.zones {
                    if !self.beach_zones.iter().any(|k| k.id == z.id) {
                        self.beach_zones.push(z);
                    }
                }
            }
            Err(e) => self.beach_failed(&e),
        }
    }

    fn beach_failed(&mut self, e: &bf::BeachError) {
        let kind = match e {
            bf::BeachError::Status { status } => kind_of_status(*status),
            bf::BeachError::Parse(_) => SourceErrorKind::Parse,
            _ => SourceErrorKind::Other,
        };
        self.failed(&SourceId::NWS_BEACH_FORECAST, kind, e.to_string());
    }

    /// "No beach forecast zones in this view", when the lookup is done and found none.
    pub(super) fn beach_notices(&self, notices: &mut Vec<String>) {
        let done = self.beach.as_ref().is_some_and(|r| r.queried);
        if done && self.round.is_empty() && self.beach_zones.is_empty() && self.issues.is_empty() {
            push_unique(
                notices,
                self.f.strings.get(Key::NearbyBeachesNone).to_owned(),
            );
        }
    }

    /// The zones with an outline, in the order of their ids, and the legend.
    pub(super) fn beach_results(&self) -> (Vec<BeachZonePin>, Vec<BeachLegendItem>) {
        if self.beach.is_none() {
            return (Vec::new(), Vec::new());
        }
        let f = &self.f;
        let mut zones: Vec<&bf::BeachZone> = self.beach_zones.iter().collect();
        zones.sort_by(|a, b| a.id.cmp(&b.id));
        let pins = zones
            .into_iter()
            .filter_map(|z| zone_pin(f, z, self.now))
            .collect();
        let legend = beach::legend(f)
            .into_iter()
            .map(|e| BeachLegendItem {
                role: e.role,
                label: e.label,
            })
            .collect();
        (pins, legend)
    }
}

const fn category(risk: RipRisk) -> RipCategory {
    match risk {
        RipRisk::Low => RipCategory::Low,
        RipRisk::Moderate => RipCategory::Moderate,
        RipRisk::High => RipCategory::High,
    }
}

fn map_point(p: GeoPoint) -> MapPoint {
    MapPoint {
        lat: p.lat,
        lon: p.lon,
    }
}

fn zone_pin(
    f: &underbite_core::view::format::Formatter,
    z: &bf::BeachZone,
    now: UnixTime,
) -> Option<BeachZonePin> {
    let outer = z
        .polygons
        .first()
        .and_then(|p| p.first())
        .filter(|r| !r.is_empty())?;
    let n = outer.len() as f64;
    let facts = BeachFacts {
        name: &z.name,
        office: z.office.as_deref(),
        rip: z.rip.map(category),
        rip_text: z.rip_text.as_deref(),
        surf: z.surf.as_deref(),
        water_temperature: z.water_temperature.as_deref(),
        uv: z.uv.as_deref(),
        weather: z.weather.as_deref(),
        winds: z.winds.as_deref(),
        period: z.period.as_deref(),
        product_date: z.product_date.as_deref(),
        product_time: z.product_time.as_deref(),
        issued: z.issued,
    };
    let card = beach::card(f, &facts, now);
    Some(BeachZonePin {
        id: format!(
            "{}:{}",
            z.office.as_deref().unwrap_or("nws").to_ascii_lowercase(),
            z.id
        ),
        name: z.name.clone(),
        role: card.role.clone(),
        lat: outer.iter().map(|p| p.lat).sum::<f64>() / n,
        lon: outer.iter().map(|p| p.lon).sum::<f64>() / n,
        polygons: z
            .polygons
            .iter()
            .map(|rings| BeachPolygon {
                rings: rings
                    .iter()
                    .map(|r| r.iter().copied().map(map_point).collect())
                    .collect(),
            })
            .collect(),
        card: BeachCardInfo {
            title: card.title,
            role: card.role,
            risk: card.risk,
            rows: card
                .rows
                .into_iter()
                .map(|r| BeachRowInfo {
                    label: r.label,
                    value: r.value,
                })
                .collect(),
            period: card.period,
            issued: card.issued,
            age: card.age,
            safety: card.safety,
            accessibility: card.accessibility,
        },
    })
}
