//! Station lists: CO-OPS's and NDBC's whole lists, and USGS sites and NWPS gauges in a box
//! around the rounded spot, each stored for a while; and the reservoir projects of the bundled
//! CWMS and RISE tables, found on the device.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use underbite_core::discover::rank::{ChainSpec, StationKey};
use underbite_core::model::{
    Coverage, Measurement, Requirements, SourceError as ModelSourceError, StationCandidate,
};
use underbite_core::provenance::{SourceId, StationKind, StationRef};
use underbite_core::time::UnixTime;
use underbite_core::units::{Canonical, to_canonical};
use underbite_providers::coops::Coops;
use underbite_providers::coops::mdapi::{self, Metadata, StationList};
use underbite_providers::ndbc::{self, Ndbc};
use underbite_providers::nwps::{self, Nwps};
use underbite_providers::request::{Coordinates, Place, RoundedPoint, Target, Window};
use underbite_providers::reservoirs::catalogue::{self as cwms_catalogue, Near};
use underbite_providers::reservoirs::rise;
use underbite_providers::source::{FetchContext, Source, SourceError, check_response};
use underbite_providers::usgs::{Usgs, sites};

use crate::host::{Driver, FetchHost};
use crate::source_error;
use crate::store::{self, COOPS_STATION_LIST_TTL_S, STATION_LIST_TTL_S, spot_key};

/// Half the side of the USGS site search box, degrees. The box is centred on the spot's cell of
/// [`USGS_CELL_DECIMALS`] decimals, up to 0.05 degrees from the spot, so it reaches at least
/// 25 km every way.
const USGS_HALF_WIDTH_DEG: f64 = 0.3;

/// Decimals of the point the USGS site box is centred on: spots in the same tenth of a degree
/// share one list, so a few spots in a row ask USGS once.
const USGS_CELL_DECIMALS: u8 = 1;

/// The centre of the USGS site box (and the key of its list) for a spot at `point`.
pub(crate) fn usgs_cell(point: RoundedPoint) -> RoundedPoint {
    RoundedPoint::new(
        point.coordinates(),
        USGS_CELL_DECIMALS.min(point.decimals()),
    )
}

/// A station list as stored.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct StoredList {
    pub(crate) candidates: Vec<StationCandidate>,
    /// Depth of current bins, metres.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    bin_depths_m: Vec<(StationKey, f32)>,
    /// Parts that failed: the list is used but not stored, so the next fetch asks again.
    #[serde(skip)]
    gaps: Vec<SourceError>,
}

/// Every station near the spot from the lists of the sources in its chains.
#[derive(Debug, Default)]
pub(crate) struct Lists {
    pub(crate) candidates: Vec<StationCandidate>,
    pub(crate) bin_depths_m: BTreeMap<StationKey, f32>,
    /// Whether a list came from the network (rankings are redone).
    pub(crate) refreshed: bool,
    pub(crate) errors: Vec<ModelSourceError>,
}

impl Lists {
    fn add(&mut self, list: StoredList) {
        self.candidates.extend(list.candidates);
        self.bin_depths_m.extend(list.bin_depths_m);
    }
}

/// A target for list and lookup requests: the rounded spot, no measurement.
pub(crate) fn point_target(point: RoundedPoint, now: UnixTime) -> Target {
    Target {
        place: Place::Point(point),
        measurements: BTreeSet::new(),
        window: Window { from: now, to: now },
        datum: None,
    }
}

/// The key of a list around a rounded point: a hash, so no coordinates name a stored value.
pub(crate) fn area_key(list: &str, point: RoundedPoint) -> String {
    let (lat, lon) = point.fmt_param();
    format!("{list}-{}", spot_key(&format!("{lat},{lon}")))
}

/// The depth of each current bin in a CO-OPS `currentpredictions` list, metres. The list gives
/// feet (its `units`); the station parser keeps the bins but not their depths.
fn bin_depths(body: &[u8]) -> Result<Vec<(StationKey, f32)>, SourceError> {
    #[derive(Deserialize)]
    struct Body {
        units: Option<String>,
        stations: Vec<Entry>,
    }
    #[derive(Deserialize)]
    struct Entry {
        id: String,
        currbin: Option<u16>,
        depth: Option<f32>,
    }
    let body: Body = serde_json::from_slice(body).map_err(|e| SourceError::Parse(e.to_string()))?;
    let unit = match body.units.as_deref() {
        Some("metric") => "m",
        _ => "ft",
    };
    let mut out = Vec::new();
    for entry in body.stations {
        let (Some(bin), Some(depth)) = (entry.currbin, entry.depth) else {
            continue;
        };
        let Ok(Canonical::Length(metres)) = to_canonical(depth, unit) else {
            return Err(SourceError::Parse(format!("bin depth unit {unit}")));
        };
        out.push((
            StationKey::new(SourceId::COOPS, entry.id).with_bin(bin),
            metres,
        ));
    }
    Ok(out)
}

/// One CO-OPS list, with bin depths for the current list.
async fn coops_list<H: FetchHost>(
    host: &Driver<'_, H>,
    list: StationList,
    target: &Target,
) -> Result<StoredList, SourceError> {
    let request = mdapi::stations_request(list, target.clone());
    let fetched = host.get(&Coops, &request).await?;
    let response = fetched.as_response();
    let Metadata::Stations(candidates) = mdapi::parse(&request, &response)? else {
        return Err(SourceError::Parse("not a station list".into()));
    };
    let bin_depths_m = if list == StationList::CurrentPredictions {
        bin_depths(&fetched.body)?
    } else {
        Vec::new()
    };
    Ok(StoredList {
        candidates,
        bin_depths_m,
        gaps: Vec::new(),
    })
}

/// NDBC's active stations.
async fn ndbc_list<H: FetchHost>(
    host: &Driver<'_, H>,
    target: &Target,
) -> Result<StoredList, SourceError> {
    let request = ndbc::stations::request(target.clone());
    let fetched = host.get(&Ndbc, &request).await?;
    let body = check_response(&request, &fetched.as_response())?;
    let text = std::str::from_utf8(body).map_err(|e| SourceError::Parse(e.to_string()))?;
    Ok(StoredList {
        candidates: ndbc::stations::parse(text)?,
        bin_depths_m: Vec::new(),
        gaps: Vec::new(),
    })
}

/// USGS sites in the box around the spot with the series they serve.
pub(crate) async fn usgs_list<H: FetchHost>(
    host: &Driver<'_, H>,
    target: &Target,
    ctx: &FetchContext,
) -> Result<StoredList, SourceError> {
    let usgs = Usgs::new();
    let mut stations: Vec<StationRef> = Vec::new();
    let mut gaps = Vec::new();
    // One request per site type; a type that fails leaves the others usable.
    for request in sites::locations_requests(target, USGS_HALF_WIDTH_DEG, ctx)? {
        let found = match host.get(&usgs, &request).await {
            Ok(fetched) => {
                let response = fetched.as_response();
                check_response(&request, &response).and_then(sites::parse_locations)
            }
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
            // Asked too much: the other requests would only be refused too.
            Err(e @ SourceError::RateLimited { .. }) => return Err(e),
            Err(e) => gaps.push(e),
        }
    }
    let request = sites::metadata_request(target, USGS_HALF_WIDTH_DEG, ctx)?;
    let fetched = host.get(&usgs, &request).await?;
    let offers = sites::parse_metadata(check_response(&request, &fetched.as_response())?)?;
    Ok(StoredList {
        candidates: sites::candidates(stations, &offers),
        bin_depths_m: Vec::new(),
        gaps,
    })
}

/// NWPS gauges in the box around the spot.
async fn nwps_list<H: FetchHost>(
    host: &Driver<'_, H>,
    target: &Target,
    ctx: &FetchContext,
) -> Result<StoredList, SourceError> {
    let request = Nwps
        .plan(target, &Requirements::default(), ctx)?
        .into_iter()
        .next()
        .ok_or_else(|| SourceError::Unsupported("no gauge list request".into()))?;
    let fetched = host.get(&Nwps, &request).await?;
    let body = check_response(&request, &fetched.as_response())?;
    Ok(StoredList {
        candidates: nwps::parse_gauges(body)?,
        bin_depths_m: Vec::new(),
        gaps: Vec::new(),
    })
}

/// A list from the host's store while it is younger than `ttl_s`, else from `fetch` (which
/// does not run when the stored list is used).
async fn list<H: FetchHost>(
    host: &Driver<'_, H>,
    lists: &mut Lists,
    source: SourceId,
    key: &str,
    ttl_s: i64,
    fetch: impl Future<Output = Result<StoredList, SourceError>>,
) {
    if let Some(stored) = host
        .stored_within::<StoredList>(store::STATIONS, key, ttl_s)
        .await
    {
        lists.add(stored);
        return;
    }
    match fetch.await {
        Ok(mut stored) => {
            for gap in std::mem::take(&mut stored.gaps) {
                tracing::warn!(%source, error = %gap, "station list incomplete");
                lists
                    .errors
                    .push(source_error(source.clone(), "station list", &gap));
            }
            if lists.errors.iter().all(|e| e.source != source) {
                host.keep(store::STATIONS, key, &stored).await;
            }
            lists.refreshed = true;
            lists.add(stored);
        }
        Err(e) => {
            tracing::warn!(%source, error = %e, "station list");
            lists.errors.push(source_error(source, "station list", &e));
        }
    }
}

/// CO-OPS water-level stations on tidal water usually carry a water temperature sensor and a
/// weather station, which their list does not say: they are offered as secondary, unconfirmed
/// products, so a fetch tries them and records why when they have nothing recent.
fn with_station_observations(candidates: &mut [StationCandidate]) {
    for c in candidates.iter_mut().filter(|c| {
        c.station.source == SourceId::COOPS
            && c.station.kind == StationKind::WaterLevel
            && c.offers.contains_key(&Measurement::Tide)
    }) {
        for m in [
            Measurement::WaterTemperature,
            Measurement::ObservedWind,
            Measurement::ObservedPressure,
        ] {
            c.offers.entry(m).or_insert(Coverage {
                last_value_at: None,
                primary: false,
            });
        }
    }
}

/// A reservoir project as a station: the pool as the lake level, a release or inflow as the
/// reservoir block. Listed, not confirmed: the bundled table names the series, not their
/// latest values.
fn reservoir_candidate(
    source: SourceId,
    id: String,
    name: &str,
    at: (f64, f64),
    pool: bool,
    flows: bool,
) -> StationCandidate {
    let listed = Coverage {
        last_value_at: None,
        primary: true,
    };
    let mut offers = BTreeMap::new();
    if pool {
        offers.insert(Measurement::LakeLevel, listed);
    }
    if flows {
        offers.insert(Measurement::Reservoir, listed);
    }
    StationCandidate {
        station: StationRef {
            source,
            id,
            name: name.to_owned(),
            kind: StationKind::WaterLevel,
            latitude: at.0,
            longitude: at.1,
            bin: None,
            reference_id: None,
        },
        offers,
    }
}

/// The farthest any chain naming `source` reaches, metres; none when no chain names it.
fn reach_of(chains: &BTreeMap<Measurement, ChainSpec>, source: &SourceId) -> Option<f64> {
    chains
        .values()
        .filter(|c| c.sources.contains(source))
        .map(|c| c.limits.max_distance_m)
        .reduce(f64::max)
}

/// The reservoir projects of the bundled CWMS and RISE tables within the reach of the chains
/// that name their source. Found on the device from the exact spot: no request is made and
/// nothing about the spot leaves the device. Link-only projects carry no values and are left
/// out.
fn reservoir_candidates(
    spot: Coordinates,
    chains: &BTreeMap<Measurement, ChainSpec>,
) -> Vec<StationCandidate> {
    let mut out = Vec::new();
    if let Some(reach) = reach_of(chains, &SourceId::CWMS) {
        let near = cwms_catalogue::nearby(spot.latitude, spot.longitude, reach as f32);
        out.extend(near.into_iter().filter_map(|n| match n.project {
            Near::Cwms(p) => Some(reservoir_candidate(
                SourceId::CWMS,
                p.station_id(),
                p.name,
                (p.latitude, p.longitude),
                p.pool.is_some(),
                p.outflow.is_some() || p.inflow.is_some(),
            )),
            Near::Link(_) => None,
        }));
    }
    if let Some(reach) = reach_of(chains, &SourceId::RISE) {
        let near = rise::nearby(spot.latitude, spot.longitude, reach as f32);
        out.extend(near.into_iter().map(|(p, _)| {
            reservoir_candidate(
                SourceId::RISE,
                p.station_id(),
                p.name,
                (p.latitude, p.longitude),
                p.pool.is_some(),
                p.outflow.is_some() || p.inflow.is_some(),
            )
        }));
    }
    out
}

/// The lists of every station source in `chains`: those asked of the sources around the
/// rounded `point`, and the reservoir projects of the bundled tables around the exact `spot`.
pub(crate) async fn station_lists<H: FetchHost>(
    host: &Driver<'_, H>,
    ctx: &FetchContext,
    spot: Coordinates,
    point: RoundedPoint,
    chains: &BTreeMap<Measurement, ChainSpec>,
) -> Lists {
    let sources: BTreeSet<&SourceId> = chains.values().flat_map(|c| &c.sources).collect();
    let target = point_target(point, host.now);
    let mut lists = Lists::default();
    lists.candidates.extend(reservoir_candidates(spot, chains));
    if sources.contains(&SourceId::COOPS) || sources.contains(&SourceId::COOPS_HARMONICS) {
        for kind in [
            StationList::TidePredictions,
            StationList::CurrentPredictions,
            StationList::WaterLevels,
        ] {
            let key = format!("coops-{}", kind.type_name());
            list(
                host,
                &mut lists,
                SourceId::COOPS,
                &key,
                COOPS_STATION_LIST_TTL_S,
                coops_list(host, kind, &target),
            )
            .await;
        }
        with_station_observations(&mut lists.candidates);
    }
    if sources.contains(&SourceId::NDBC) {
        list(
            host,
            &mut lists,
            SourceId::NDBC,
            "ndbc-active",
            STATION_LIST_TTL_S,
            ndbc_list(host, &target),
        )
        .await;
    }
    if sources.contains(&SourceId::USGS) {
        let cell = usgs_cell(point);
        let key = area_key("usgs", cell);
        let cell_target = point_target(cell, host.now);
        list(
            host,
            &mut lists,
            SourceId::USGS,
            &key,
            STATION_LIST_TTL_S,
            usgs_list(host, &cell_target, ctx),
        )
        .await;
    }
    if sources.contains(&SourceId::NWPS) {
        let key = area_key("nwps", point);
        list(
            host,
            &mut lists,
            SourceId::NWPS,
            &key,
            STATION_LIST_TTL_S,
            nwps_list(host, &target, ctx),
        )
        .await;
    }
    lists
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::fixtures;
    use underbite_core::discover::rank::Limits;

    /// On Beltzville Lake, a little over a kilometre from the project's point.
    const BELTZVILLE: Coordinates = Coordinates {
        latitude: 40.855,
        longitude: -75.627,
    };

    fn lake_chains(sources: &[SourceId], km: Option<f64>) -> BTreeMap<Measurement, ChainSpec> {
        [Measurement::LakeLevel, Measurement::Reservoir]
            .into_iter()
            .map(|m| {
                let spec = ChainSpec {
                    sources: sources.to_vec(),
                    limits: Limits::from_config(km, None),
                };
                (m, spec)
            })
            .collect()
    }

    #[test]
    fn a_reservoir_spot_lists_its_project_from_the_bundled_table() {
        let chains = lake_chains(&[SourceId::USGS, SourceId::CWMS, SourceId::RISE], None);
        let found = reservoir_candidates(BELTZVILLE, &chains);
        let ids: Vec<(&str, &str)> = found
            .iter()
            .map(|c| (c.station.source.as_str(), c.station.id.as_str()))
            .collect();
        assert_eq!(ids, [("cwms", "NAP/Beltzville")]);
        let beltzville = &found[0];
        assert_eq!(beltzville.station.name, "Beltzville Lake");
        assert_eq!(beltzville.station.kind, StationKind::WaterLevel);
        // The pool and the inflow, listed but unconfirmed.
        assert_eq!(
            beltzville.offers.keys().copied().collect::<Vec<_>>(),
            [Measurement::LakeLevel, Measurement::Reservoir]
        );
        assert!(
            beltzville
                .offers
                .values()
                .all(|c| c.primary && c.last_value_at.is_none())
        );
    }

    #[test]
    fn reservoir_projects_stay_within_their_chains_reach() {
        // A 1 km limit leaves the project out; so does a chain without its source.
        let near = lake_chains(&[SourceId::CWMS], Some(1.0));
        assert_eq!(reservoir_candidates(BELTZVILLE, &near), []);
        let other = lake_chains(&[SourceId::USGS, SourceId::RISE], None);
        assert_eq!(reservoir_candidates(BELTZVILLE, &other), []);
        // Nothing within the default 50 km of the Battery.
        let battery = Coordinates {
            latitude: 40.7033,
            longitude: -74.0170,
        };
        let all = lake_chains(&[SourceId::CWMS, SourceId::RISE], None);
        assert_eq!(reservoir_candidates(battery, &all), []);
        // Reclamation's projects come from their own table.
        let powell = Coordinates {
            latitude: 37.07,
            longitude: -111.32,
        };
        let found = reservoir_candidates(powell, &all);
        assert_eq!(found.len(), 1, "{found:#?}");
        assert_eq!(
            (
                found[0].station.source.as_str(),
                found[0].station.id.as_str()
            ),
            ("rise", "393")
        );
    }

    #[test]
    fn bin_depths_are_read_in_metres() {
        let body = std::fs::read(
            fixtures()
                .join("raw/coops/coops-mdapi-stations-currentpredictions-newyork-trimmed.json"),
        )
        .expect("fixture");
        let depths: BTreeMap<StationKey, f32> =
            bin_depths(&body).expect("depths").into_iter().collect();
        let key = StationKey::new(SourceId::COOPS, "NYH1927").with_bin(13);
        assert!(
            (depths[&key] - 7.0 * 0.3048).abs() < 1e-4,
            "{}",
            depths[&key]
        );
        assert!(bin_depths(b"{}").is_err());
    }
}
