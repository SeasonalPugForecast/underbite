//! The hydrography around a spot (3DHP flowlines and waterbodies) and what it tells of the
//! stations near it: upstream, downstream, the same water or another.

use std::collections::{BTreeMap, BTreeSet};

use underbite_core::discover::hydro::{self as hydrography, HydroOptions, Point};
use underbite_core::discover::rank::{ChainSpec, StationKey, distance_m};
use underbite_core::model::{Measurement, Spot, StationCandidate, WaterType};
use underbite_core::provenance::{Relation, StationRef};
use underbite_providers::hydro;
use underbite_providers::request::{CacheHint, Coordinates, Response};
use underbite_providers::source::{FetchContext, SourceError};

use crate::host::{Call, Driver, FetchHost};

/// How far around the spot the hydrography is read, metres.
pub(crate) const HYDRO_RADIUS_M: f64 = 5_000.0;
/// River stations beyond the spot's hydrography whose own surroundings are read, per fetch.
const FAR_STATIONS: usize = 4;
/// Measurements of a river at a gauge, which only mean something on the spot's own river.
const RIVER_MEASUREMENTS: [Measurement; 4] = [
    Measurement::Discharge,
    Measurement::GageHeight,
    Measurement::FlowForecast,
    Measurement::FloodCategories,
];

/// Water types whose stations are related to the spot through the river network: rivers,
/// lakes and the estuaries and bays they open into.
pub(crate) fn uses_hydrography(water: WaterType) -> bool {
    matches!(
        water,
        WaterType::RiverStream
            | WaterType::LakeReservoir
            | WaterType::Pond
            | WaterType::Ice
            | WaterType::TidalRiverEstuary
            | WaterType::BayInshore
    )
}

/// Every page of a 3DHP query.
pub(crate) async fn hydro_pages<T, H: FetchHost>(
    host: &Driver<'_, H>,
    first: hydro::HydroQuery,
    parse: fn(&Response<'_>) -> Result<Vec<T>, SourceError>,
) -> Result<Vec<T>, SourceError> {
    let mut out = Vec::new();
    let mut query = Some(first);
    while let Some(q) = query {
        let call = Call::get(q.url.clone()).with_cache(CacheHint {
            max_age: Some(q.max_age_s),
            ..CacheHint::default()
        });
        let fetched = host
            .send(&call)
            .await
            .map_err(|e| SourceError::Transport(e.to_string()))?;
        let response = fetched.as_response();
        out.extend(parse(&response)?);
        query = hydro::next_page(&q, &response)?;
    }
    Ok(out)
}

/// What the hydrography tells of the stations around a spot.
#[derive(Debug, Default)]
pub(crate) struct Placement {
    /// Each placed station's relation to the spot, keyed without bin.
    pub(crate) relations: BTreeMap<StationKey, Relation>,
    /// Whether the spot is on the mapped river network.
    pub(crate) spot_placed: bool,
}

/// The flowlines and waterbodies loaded for a spot.
struct Network {
    flowlines: Vec<hydrography::Flowline>,
    waterbodies: Vec<hydrography::Waterbody>,
}

/// The relation of each station to the spot from the flowlines and waterbodies around it. A
/// station or spot off the mapped streams has no known relation and is left out; a `Nearby`
/// between two placed ones (a tributary, another river) is known and kept.
///
/// `river` lists the stations that serve a river measurement, nearest first. Those the spot's
/// hydrography leaves off the network get the flowlines and waterbodies around their own
/// position (at most [`FAR_STATIONS`] stations), and the relations are computed once more over
/// both.
pub(crate) async fn relations<H: FetchHost>(
    host: &Driver<'_, H>,
    ctx: &FetchContext,
    spot: &Spot,
    stations: &[StationRef],
    river: &[StationRef],
) -> Result<Placement, SourceError> {
    let at = Coordinates {
        latitude: spot.latitude,
        longitude: spot.longitude,
    };
    let mut network = Network {
        flowlines: hydro_pages(
            host,
            hydro::flowlines_query(at, HYDRO_RADIUS_M, ctx)?,
            hydro::parse_flowlines,
        )
        .await?,
        waterbodies: hydro_pages(
            host,
            hydro::waterbodies_query(at, HYDRO_RADIUS_M, ctx)?,
            hydro::parse_waterbodies,
        )
        .await?,
    };
    let here = Point::new(spot.latitude, spot.longitude);
    let relate = |network: &Network| {
        hydrography::relations(
            here,
            stations,
            &network.flowlines,
            &network.waterbodies,
            HydroOptions::default(),
        )
    };
    let mut hints = relate(&network);
    let off: Vec<&StationRef> = river
        .iter()
        .filter(|s| {
            hints
                .get(&(s.source.clone(), s.id.clone()))
                .is_some_and(off_network)
        })
        .collect();
    if far_hydrography(host, ctx, &off, &mut network).await {
        hints = relate(&network);
    }
    let spot_placed = hydrography::Hydro::new(
        &network.flowlines,
        &network.waterbodies,
        HydroOptions::default(),
    )
    .locate_spot(here)
    .stream
    .is_some();
    let placed = |hint: &hydrography::RelationHint| {
        hint.relation != Relation::Nearby
            || !hint
                .reasons
                .iter()
                .any(|r| matches!(r, hydrography::HydroReason::NotOnMappedStream { .. }))
    };
    let relations = hints
        .into_iter()
        .filter(|(_, hint)| placed(hint))
        .map(|((source, id), hint)| (StationKey::new(source, id), hint.relation))
        .collect();
    Ok(Placement {
        relations,
        spot_placed,
    })
}

/// A station the loaded network does not reach while it places the spot.
fn off_network(hint: &hydrography::RelationHint) -> bool {
    let off = |subject: hydrography::Subject| {
        hint.reasons.iter().any(|r| {
            matches!(r, hydrography::HydroReason::NotOnMappedStream { subject: s } if *s == subject)
        })
    };
    hint.relation == Relation::Nearby
        && off(hydrography::Subject::Station)
        && !off(hydrography::Subject::Spot)
}

/// Adds the flowlines and waterbodies around each of `stations` (nearest first, at most
/// [`FAR_STATIONS`] stations, each query centred on the station rounded by the context) that
/// are not loaded yet; returns whether any were added. A query that fails leaves out what it
/// would have added.
async fn far_hydrography<H: FetchHost>(
    host: &Driver<'_, H>,
    ctx: &FetchContext,
    stations: &[&StationRef],
    network: &mut Network,
) -> bool {
    let mut lines: BTreeSet<String> = network.flowlines.iter().map(|f| f.id.clone()).collect();
    let mut bodies: BTreeSet<String> = network.waterbodies.iter().map(|w| w.id.clone()).collect();
    let mut queried: BTreeSet<String> = BTreeSet::new();
    let before = (network.flowlines.len(), network.waterbodies.len());
    let failed = |station: &StationRef, e: &SourceError| {
        tracing::debug!(station = %station.id, error = %e, "hydrography around a station");
    };
    for station in stations {
        let at = Coordinates {
            latitude: station.latitude,
            longitude: station.longitude,
        };
        let pair = hydro::flowlines_query(at, hydro::MIN_RADIUS_M, ctx)
            .and_then(|f| hydro::waterbodies_query(at, hydro::MIN_RADIUS_M, ctx).map(|w| (f, w)));
        let (flowlines, waterbodies) = match pair {
            Ok(pair) => pair,
            Err(e) => {
                failed(station, &e);
                continue;
            }
        };
        // Stations in the same rounding cell share their queries.
        if queried.contains(&flowlines.url) {
            continue;
        }
        if queried.len() == FAR_STATIONS {
            tracing::debug!(station = %station.id, "hydrography around a station: limit reached");
            break;
        }
        queried.insert(flowlines.url.clone());
        match hydro_pages(host, flowlines, hydro::parse_flowlines).await {
            Ok(found) => network
                .flowlines
                .extend(found.into_iter().filter(|f| lines.insert(f.id.clone()))),
            Err(e) => failed(station, &e),
        }
        match hydro_pages(host, waterbodies, hydro::parse_waterbodies).await {
            Ok(found) => network
                .waterbodies
                .extend(found.into_iter().filter(|w| bodies.insert(w.id.clone()))),
            Err(e) => failed(station, &e),
        }
    }
    (network.flowlines.len(), network.waterbodies.len()) != before
}

/// The stations that can serve a river measurement within its chain's distance limit, nearest
/// first.
pub(crate) fn river_stations(
    spot: &Spot,
    candidates: &[StationCandidate],
    chains: &BTreeMap<Measurement, ChainSpec>,
) -> Vec<StationRef> {
    let mut out: Vec<(f64, &StationRef)> = Vec::new();
    for c in candidates {
        let d = distance_m(
            spot.latitude,
            spot.longitude,
            c.station.latitude,
            c.station.longitude,
        );
        let serves = RIVER_MEASUREMENTS.iter().any(|m| {
            c.offers.contains_key(m)
                && chains.get(m).is_some_and(|chain| {
                    chain.sources.contains(&c.station.source) && d <= chain.limits.max_distance_m
                })
        });
        let listed = out
            .iter()
            .any(|(_, s)| s.source == c.station.source && s.id == c.station.id);
        if serves && !listed {
            out.push((d, &c.station));
        }
    }
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    out.into_iter().map(|(_, s)| s.clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{MemoryHost, assert_rounded, croton, ctx, now};
    use Measurement as M;
    use underbite_core::discover::rank::{DiscoveryInput, Limits, Reason};
    use underbite_core::model::Coverage;
    use underbite_core::provenance::{SourceId, StationKind};

    /// Answers the spot's flowlines and no waterbodies, plus flowline pages around stations;
    /// any other URL has no route.
    fn hydrography(spot: &Spot, flowlines: String, around: &[(&StationRef, String)]) -> MemoryHost {
        let at = |latitude, longitude| Coordinates {
            latitude,
            longitude,
        };
        let centre = at(spot.latitude, spot.longitude);
        let query = |centre, radius| hydro::flowlines_query(centre, radius, &ctx()).expect("query");
        let mut host = MemoryHost::default()
            .answering(query(centre, HYDRO_RADIUS_M).url, flowlines)
            .answering(
                hydro::waterbodies_query(centre, HYDRO_RADIUS_M, &ctx())
                    .expect("query")
                    .url,
                r#"{"features":[]}"#,
            );
        for (station, page) in around {
            let url = query(at(station.latitude, station.longitude), hydro::MIN_RADIUS_M).url;
            host = host.answering(url, page.clone());
        }
        host
    }

    /// `host` answering the waterbodies page around `station` too.
    fn with_waterbodies(host: MemoryHost, station: &StationRef, page: String) -> MemoryHost {
        let at = Coordinates {
            latitude: station.latitude,
            longitude: station.longitude,
        };
        let url = hydro::waterbodies_query(at, hydro::MIN_RADIUS_M, &ctx())
            .expect("query")
            .url;
        host.answering(url, page)
    }

    /// A synthetic flowline: id, name, level path, hydrosequence, the hydrosequence downstream
    /// (0 at an outlet) and its ends as (latitude, longitude), upstream first.
    type Line<'a> = (&'a str, &'a str, u64, u64, u64, [(f64, f64); 2]);

    /// A layer 50 page of synthetic flowlines.
    fn flowline_page(lines: &[Line<'_>]) -> String {
        let features: Vec<serde_json::Value> = lines
            .iter()
            .map(|(id, name, level_path, hs, dn, ends)| {
                let path: Vec<[f64; 2]> = ends.iter().map(|&(lat, lon)| [lon, lat]).collect();
                serde_json::json!({
                    "attributes": {
                        "id3dhp": id,
                        "gnisidlabel": name,
                        "featuretypelabel": "StreamRiver",
                        "streamorder": 3,
                        "levelpath": level_path,
                        "hydrosequence": hs,
                        "dnhydrosequence": dn,
                        "uphydrosequence": 0,
                    },
                    "geometry": { "paths": [path] },
                })
            })
            .collect();
        serde_json::json!({ "features": features }).to_string()
    }

    /// A forecast gauge that defines flood stages.
    fn flood_gauge(id: &str, latitude: f64, longitude: f64) -> StationCandidate {
        let coverage = Coverage {
            last_value_at: None,
            primary: true,
        };
        StationCandidate {
            station: StationRef {
                source: SourceId::NWPS,
                id: id.into(),
                name: id.into(),
                kind: StationKind::NwpsGauge,
                latitude,
                longitude,
                bin: None,
                reference_id: None,
            },
            offers: BTreeMap::from([(M::FloodCategories, coverage)]),
        }
    }

    fn flood_chain() -> BTreeMap<Measurement, ChainSpec> {
        BTreeMap::from([(
            M::FloodCategories,
            ChainSpec {
                sources: vec![SourceId::NWPS],
                limits: Limits::from_config(Some(25.0), None),
            },
        )])
    }

    /// The relations `host` gives `stations` around `spot`.
    fn placed(
        host: &MemoryHost,
        spot: &Spot,
        stations: &[StationRef],
        river: &[StationRef],
    ) -> Placement {
        pollster::block_on(async {
            let driver = Driver::new(host, now(), true).await;
            relations(&driver, &ctx(), spot, stations, river)
                .await
                .expect("hydrography")
        })
    }

    /// The spot's own reach of the main river, north to south through the spot.
    const SPOT_REACH: Line<'static> = (
        "main-1",
        "Main River",
        1,
        100,
        0,
        [(41.235, -73.8588), (41.215, -73.8588)],
    );

    #[test]
    fn far_river_gauges_are_placed_by_the_hydrography_around_them() {
        let spot = croton();
        // About 17 km away, beyond the spot's flowlines: one up the same river, one on another
        // river, and one whose surroundings cannot be read (left unplaced).
        let candidates = vec![
            flood_gauge("UPPN6", 41.38, -73.8588),
            flood_gauge("OTHN6", 41.2252, -74.06),
            flood_gauge("LSTN6", 41.11, -73.97),
        ];
        let stations: Vec<StationRef> = candidates.iter().map(|c| c.station.clone()).collect();
        let upper = (
            "main-9",
            "Main River",
            1,
            200,
            150,
            [(41.39, -73.8588), (41.37, -73.8588)],
        );
        let other = (
            "other-1",
            "Other River",
            2,
            500,
            0,
            [(41.235, -74.06), (41.215, -74.06)],
        );
        let host = with_waterbodies(
            hydrography(
                &spot,
                flowline_page(&[SPOT_REACH]),
                &[
                    // The spot's reach again: loaded once.
                    (&stations[0], flowline_page(&[upper, SPOT_REACH])),
                    (&stations[1], flowline_page(&[other])),
                ],
            ),
            &stations[0],
            r#"{"features":[]}"#.into(),
        );
        let chains = flood_chain();
        let river = river_stations(&spot, &candidates, &chains);
        let nearest: Vec<&str> = river.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(nearest, ["LSTN6", "OTHN6", "UPPN6"]);

        let placement = placed(&host, &spot, &stations, &river);
        assert!(placement.spot_placed);
        let relations = placement.relations;
        let key = |id: &str| StationKey::new(SourceId::NWPS, id);
        // Same level path, higher hydrosequence: upstream, with no reach loaded in between.
        assert_eq!(relations.get(&key("UPPN6")), Some(&Relation::Upstream));
        assert_eq!(relations.get(&key("OTHN6")), Some(&Relation::Nearby));
        assert_eq!(relations.get(&key("LSTN6")), None);
        // The spot's two queries, then flowlines and waterbodies around each gauge.
        let seen = host.seen();
        assert_eq!(seen.len(), 8, "{seen:#?}");
        for url in &seen {
            assert_rounded(url);
        }

        let input = DiscoveryInput {
            latitude: spot.latitude,
            longitude: spot.longitude,
            now: now(),
            candidates,
            chains,
            relations,
            spot_placed: placement.spot_placed,
            ..DiscoveryInput::default()
        };
        let ranking = underbite_core::discover::rank::rank(&input);
        let flood = &ranking.measurements[&M::FloodCategories];
        let kept: Vec<(&str, Relation)> = flood
            .stations
            .iter()
            .map(|s| (s.station.id.as_str(), s.relation))
            .collect();
        assert_eq!(kept, [("UPPN6", Relation::Upstream)]);
        let reason = |id: &str| {
            let d = flood.dropped.iter().find(|d| d.station.id == id);
            d.expect("dropped").reason.clone()
        };
        assert_eq!(
            reason("OTHN6"),
            Reason::OtherWaterBody {
                relation: Relation::Nearby
            }
        );
        assert_eq!(reason("LSTN6"), Reason::NotPlaced);
    }

    #[test]
    fn a_far_gauge_inside_a_wide_river_is_placed_by_its_outline() {
        let spot = croton();
        // In a wide river 17 km away, 700 m from its flowline.
        let candidates = vec![flood_gauge("WIDN6", 41.39, -73.96)];
        let gauge = candidates[0].station.clone();
        let wide = (
            "wide-1",
            "Wide River",
            3,
            900,
            0,
            [(41.40, -73.9684), (41.38, -73.9684)],
        );
        let outline: Vec<[f64; 2]> = [
            (41.38, -73.975),
            (41.40, -73.975),
            (41.40, -73.959),
            (41.38, -73.959),
            (41.38, -73.975),
        ]
        .iter()
        .map(|&(lat, lon)| [lon, lat])
        .collect();
        let river = serde_json::json!({ "features": [{
            "attributes": {
                "id3dhp": "W1",
                "gnisid": 7,
                "gnisidlabel": "Wide River",
                "featuretypelabel": "River",
                "areasqkm": 3.0,
            },
            "geometry": { "rings": [outline] },
        }]});
        let dropped_for = |host: &MemoryHost| {
            let chains = flood_chain();
            let river = river_stations(&spot, &candidates, &chains);
            let stations = std::slice::from_ref(&gauge);
            let placement = placed(host, &spot, stations, &river);
            let input = DiscoveryInput {
                latitude: spot.latitude,
                longitude: spot.longitude,
                now: now(),
                candidates: candidates.clone(),
                chains,
                relations: placement.relations,
                spot_placed: placement.spot_placed,
                ..DiscoveryInput::default()
            };
            let ranking = underbite_core::discover::rank::rank(&input);
            let flood = &ranking.measurements[&M::FloodCategories];
            assert_eq!(flood.stations, []);
            flood.dropped[0].reason.clone()
        };
        let around = [(&gauge, flowline_page(&[wide]))];
        let bare = hydrography(&spot, flowline_page(&[SPOT_REACH]), &around);
        assert_eq!(dropped_for(&bare), Reason::NotPlaced);
        let outlined = with_waterbodies(
            hydrography(&spot, flowline_page(&[SPOT_REACH]), &around),
            &gauge,
            river.to_string(),
        );
        assert_eq!(
            dropped_for(&outlined),
            Reason::OtherWaterBody {
                relation: Relation::Nearby
            }
        );
    }

    #[test]
    fn far_hydrography_queries_are_bounded_and_shared_within_a_rounding_cell() {
        let spot = croton();
        let mut candidates: Vec<StationCandidate> = (0..6)
            .map(|i| {
                let latitude = 41.30 + 0.02 * f64::from(i);
                flood_gauge(&format!("G{i}"), latitude, -73.70)
            })
            .collect();
        // The nearest gauge's neighbour in the same cell shares its query.
        candidates.push(flood_gauge("TWIN", 41.3004, -73.7003));
        // A gauge on the spot's reach needs no query of its own.
        candidates.push(flood_gauge("HERE", 41.22, -73.8588));
        let stations: Vec<StationRef> = candidates.iter().map(|c| c.station.clone()).collect();
        let host = hydrography(&spot, flowline_page(&[SPOT_REACH]), &[]);
        let river = river_stations(&spot, &candidates, &flood_chain());
        let relations = placed(&host, &spot, &stations, &river).relations;
        // The failed queries leave their gauges unknown without failing the hydrography.
        assert_eq!(
            relations.keys().map(|k| k.id.as_str()).collect::<Vec<_>>(),
            ["HERE"]
        );
        let far: Vec<String> = host.seen()[2..].to_vec();
        let expected: Vec<String> = ["G0", "G1", "G2", "G3"]
            .iter()
            .flat_map(|id| {
                let g = stations.iter().find(|s| s.id == *id).expect("gauge");
                let at = Coordinates {
                    latitude: g.latitude,
                    longitude: g.longitude,
                };
                let r = hydro::MIN_RADIUS_M;
                [
                    hydro::flowlines_query(at, r, &ctx()).expect("query").url,
                    hydro::waterbodies_query(at, r, &ctx()).expect("query").url,
                ]
            })
            .collect();
        assert_eq!(far, expected);
        // Every gauge query but the answered ones went unanswered, and loudly so.
        assert_eq!(host.missed(), expected);
    }
}
