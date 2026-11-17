//! Which country's data a spot is served from, decided from its coordinates.
//!
//! The outlines are coarse (tens of kilometres at the border and in the Great Lakes), enough to
//! choose default source chains. Near the border the stations found decide instead
//! ([`Region::of_spot`]). The device's own region never enters here: it only picks display
//! units.

use serde::{Deserialize, Serialize};

use super::rank::distance_m;
use crate::model::StationCandidate;
use crate::provenance::SourceId;

/// The country whose sources serve a spot by default.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Region {
    /// The United States and its coastal waters, territories included.
    Us,
    /// Canada and its coastal waters.
    Canada,
    /// Anywhere else: only gridded and satellite sources serve it.
    Elsewhere,
}

/// Stations nearer than this decide the country near a border, metres.
const BORDER_STATION_RADIUS_M: f64 = 30_000.0;

type Outline = [(f64, f64)]; // (longitude, latitude)

/// Canada, counter-clockwise from the north-west corner: the 141st meridian, the Pacific, the
/// 49th parallel, the middle of the Great Lakes and the St. Lawrence, the Maine border, the
/// Atlantic and Davis Strait.
const CANADA: [(f64, f64); 49] = [
    (-141.0, 83.0),
    (-141.0, 60.3),
    (-135.0, 52.0),
    (-131.0, 48.4),
    (-124.7, 48.4),
    (-123.3, 48.3),
    (-123.1, 48.7),
    (-123.0, 49.0),
    (-95.15, 49.0),
    (-94.6, 48.7),
    (-90.0, 48.0),
    (-89.5, 48.0),
    (-84.9, 46.5),
    (-83.9, 46.0),
    (-82.9, 45.3),
    (-82.4, 43.5),
    (-82.5, 42.6),
    (-83.1, 42.3),
    (-83.1, 42.0),
    (-82.5, 41.8),
    (-81.0, 42.2),
    (-79.0, 42.85),
    (-79.05, 43.25),
    (-79.0, 43.55),
    (-76.5, 43.6),
    (-76.3, 44.1),
    (-75.3, 44.8),
    (-74.7, 45.0),
    (-71.5, 45.0),
    (-70.8, 45.4),
    (-70.4, 46.1),
    (-70.0, 46.7),
    (-69.2, 47.45),
    (-68.3, 47.35),
    (-67.8, 47.07),
    (-67.8, 45.7),
    (-67.4, 45.15),
    (-67.0, 44.8),
    (-66.8, 44.4),
    (-65.5, 42.5),
    (-50.0, 42.5),
    (-50.0, 62.0),
    (-55.0, 64.0),
    (-57.0, 67.0),
    (-65.0, 72.0),
    (-75.0, 77.0),
    (-72.0, 79.0),
    (-68.0, 80.5),
    (-60.0, 83.0),
];

/// Alaska, checked before Canada: its panhandle lies inside the Canadian outline.
const ALASKA: [(f64, f64); 14] = [
    (-180.0, 50.0),
    (-180.0, 72.5),
    (-141.0, 72.5),
    (-141.0, 60.3),
    (-137.5, 59.0),
    (-135.0, 59.7),
    (-133.0, 57.2),
    (-131.4, 56.0),
    (-130.1, 55.9),
    (-130.0, 54.9),
    (-131.0, 54.7),
    (-133.5, 53.0),
    (-140.0, 50.0),
    (-165.0, 50.0),
];

/// The contiguous states and their coastal waters, checked after Canada.
const CONUS: [(f64, f64); 29] = [
    (-125.2, 48.4),
    (-125.2, 40.0),
    (-121.0, 33.5),
    (-118.0, 32.4),
    (-117.12, 32.5),
    (-114.8, 32.5),
    (-111.0, 31.33),
    (-108.2, 31.33),
    (-108.2, 31.78),
    (-106.5, 31.78),
    (-104.9, 30.6),
    (-104.5, 29.6),
    (-102.4, 29.8),
    (-101.0, 29.4),
    (-99.5, 27.5),
    (-97.15, 25.95),
    (-96.0, 26.0),
    (-91.0, 25.0),
    (-83.0, 24.3),
    (-80.5, 24.2),
    (-79.8, 25.5),
    (-79.5, 30.0),
    (-75.0, 35.0),
    (-70.0, 40.0),
    (-66.0, 42.0),
    (-65.5, 42.5),
    (-65.0, 50.0),
    (-125.2, 50.0),
    (-125.2, 48.4),
];

/// Hawaii and Puerto Rico with the Virgin Islands, as boxes: (west, south, east, north).
const US_BOXES: [(f64, f64, f64, f64); 2] =
    [(-161.0, 18.5, -154.5, 22.5), (-68.0, 17.5, -64.5, 18.8)];

/// Ray casting on longitude and latitude; the outlines never cross the antimeridian.
fn inside(outline: &Outline, lon: f64, lat: f64) -> bool {
    let mut in_poly = false;
    let mut j = outline.len().saturating_sub(1);
    for (i, &(xi, yi)) in outline.iter().enumerate() {
        let (xj, yj) = outline[j];
        if (yi > lat) != (yj > lat) && lon < (xj - xi) * (lat - yi) / (yj - yi) + xi {
            in_poly = !in_poly;
        }
        j = i;
    }
    in_poly
}

impl Region {
    /// The region a point lies in by the coarse outlines. A point that is not a coordinate is
    /// [`Region::Elsewhere`].
    pub fn of_point(latitude: f64, longitude: f64) -> Self {
        if !((-90.0..=90.0).contains(&latitude) && (-180.0..=180.0).contains(&longitude)) {
            return Self::Elsewhere;
        }
        let in_box = US_BOXES
            .iter()
            .any(|&(w, s, e, n)| (w..=e).contains(&longitude) && (s..=n).contains(&latitude));
        if in_box || inside(&ALASKA, longitude, latitude) {
            Self::Us
        } else if inside(&CANADA, longitude, latitude) {
            Self::Canada
        } else if inside(&CONUS, longitude, latitude) {
            Self::Us
        } else {
            Self::Elsewhere
        }
    }

    /// The region of a spot: its outline region, except that inside the outlines the nearest
    /// station of a country's own sources within 30 km decides, so a spot on the wrong side of a
    /// coarse border follows the stations that serve it. NDBC buoys belong to neither country
    /// and do not count.
    pub fn of_spot(latitude: f64, longitude: f64, candidates: &[StationCandidate]) -> Self {
        let outline = Self::of_point(latitude, longitude);
        if outline == Self::Elsewhere {
            return outline;
        }
        candidates
            .iter()
            .filter_map(|c| {
                let country = Self::of_source(&c.station.source)?;
                let d = distance_m(latitude, longitude, c.station.latitude, c.station.longitude);
                (d <= BORDER_STATION_RADIUS_M).then_some((d, country))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map_or(outline, |(_, country)| country)
    }

    /// The country a source's stations belong to, if it is a national one.
    fn of_source(source: &SourceId) -> Option<Self> {
        if *source == SourceId::DFO_IWLS || *source == SourceId::ECCC_HYDROMETRIC {
            Some(Self::Canada)
        } else if [
            SourceId::USGS,
            SourceId::COOPS,
            SourceId::COOPS_HARMONICS,
            SourceId::NWPS,
        ]
        .contains(source)
        {
            Some(Self::Us)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::discover::rank::{ChainSpec, DiscoveryInput, Limits, LinkPlace, Plan, rank};
    use crate::model::{Coverage, Measurement as M};
    use crate::provenance::{Relation, StationKind, StationRef};
    use crate::time::parse_rfc3339;

    fn fixture(path: &str) -> serde_json::Value {
        let path = format!(
            "{}/../../tests/fixtures/raw/{path}",
            env!("CARGO_MANIFEST_DIR")
        );
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    }

    fn chain(sources: &[&str], km: Option<f64>) -> ChainSpec {
        ChainSpec {
            sources: sources.iter().map(|s| SourceId::new(*s)).collect(),
            limits: Limits::from_config(km, None),
        }
    }

    fn offers(measurements: &[(M, bool)]) -> BTreeMap<M, Coverage> {
        measurements
            .iter()
            .map(|&(m, primary)| {
                (
                    m,
                    Coverage {
                        last_value_at: None,
                        primary,
                    },
                )
            })
            .collect()
    }

    fn station_ref(
        source: SourceId,
        id: &str,
        name: &str,
        kind: StationKind,
        latitude: f64,
        longitude: f64,
    ) -> StationRef {
        StationRef {
            source,
            id: id.into(),
            name: name.into(),
            kind,
            latitude,
            longitude,
            bin: None,
            reference_id: None,
        }
    }

    /// The recorded IWLS list as the planner sees it: an operating station that predicts tides
    /// or observes water level offers the tide.
    fn dfo_candidates() -> Vec<StationCandidate> {
        fixture("dfo-iwls/stations-all-2026-11-17.json")
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|s| {
                let has = |code: &str| {
                    s["timeSeries"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|t| t["code"] == code)
                };
                let predicts = has("wlp") || has("wlp-hilo");
                if s["operating"] != true || !(predicts || has("wlo")) {
                    return None;
                }
                let kind = if predicts {
                    StationKind::TideHarmonic
                } else {
                    StationKind::WaterLevel
                };
                Some(StationCandidate {
                    station: station_ref(
                        SourceId::DFO_IWLS,
                        s["id"].as_str().unwrap(),
                        &format!(
                            "{} ({})",
                            s["officialName"].as_str().unwrap(),
                            s["code"].as_str().unwrap()
                        ),
                        kind,
                        s["latitude"].as_f64().unwrap(),
                        s["longitude"].as_f64().unwrap(),
                    ),
                    offers: offers(&[(M::Tide, true)]),
                })
            })
            .collect()
    }

    /// The real-time stations of the recorded Calgary page.
    fn eccc_candidates() -> Vec<StationCandidate> {
        fixture("eccc/stations-bbox-calgary-limit10.json")["features"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|f| f["properties"]["REAL_TIME"] == 1)
            .map(|f| {
                let c = &f["geometry"]["coordinates"];
                let p = &f["properties"];
                StationCandidate {
                    station: station_ref(
                        SourceId::ECCC_HYDROMETRIC,
                        p["STATION_NUMBER"].as_str().unwrap(),
                        p["STATION_NAME"].as_str().unwrap(),
                        StationKind::WaterLevel,
                        c[1].as_f64().unwrap(),
                        c[0].as_f64().unwrap(),
                    ),
                    offers: offers(&[(M::Discharge, true), (M::GageHeight, false)]),
                }
            })
            .collect()
    }

    fn plan(
        lat: f64,
        lon: f64,
        candidates: Vec<StationCandidate>,
        chains: &[(M, &[&str])],
    ) -> Plan {
        rank(&DiscoveryInput {
            latitude: lat,
            longitude: lon,
            now: parse_rfc3339("2026-11-17T10:00:00Z").unwrap(),
            candidates,
            chains: chains
                .iter()
                .map(|&(m, s)| (m, chain(s, Some(50.0))))
                .collect(),
            ..DiscoveryInput::default()
        })
        .plan(3)
    }

    fn station_links(plan: &Plan, m: M) -> Vec<(&str, &str)> {
        plan.chains[&m]
            .links
            .iter()
            .filter_map(|l| match &l.place {
                LinkPlace::Station { station, .. } => {
                    Some((l.source.as_str(), station.name.as_str()))
                }
                LinkPlace::Point { .. } => None,
            })
            .collect()
    }

    #[test]
    fn the_outlines_place_known_cities() {
        use Region::*;
        let cases = [
            (49.2827, -123.1207, Canada, "Vancouver"),
            (48.4284, -123.3656, Canada, "Victoria"),
            (51.0447, -114.0719, Canada, "Calgary"),
            (43.6532, -79.3832, Canada, "Toronto"),
            (45.5017, -73.5673, Canada, "Montreal"),
            (44.6488, -63.5752, Canada, "Halifax"),
            (47.5615, -52.7126, Canada, "St. John's"),
            (62.454, -114.3718, Canada, "Yellowknife"),
            (60.7212, -135.0568, Canada, "Whitehorse"),
            (54.3150, -130.3208, Canada, "Prince Rupert"),
            (40.7033, -74.0170, Us, "The Battery"),
            (47.6062, -122.3321, Us, "Seattle"),
            (48.1198, -123.4307, Us, "Port Angeles"),
            (42.8864, -78.8784, Us, "Buffalo"),
            (41.8781, -87.6298, Us, "Chicago"),
            (46.8625, -68.0120, Us, "Caribou"),
            (43.6591, -70.2568, Us, "Portland, Maine"),
            (61.2181, -149.9003, Us, "Anchorage"),
            (58.3019, -134.4197, Us, "Juneau"),
            (55.3422, -131.6461, Us, "Ketchikan"),
            (21.3069, -157.8583, Us, "Honolulu"),
            (18.4655, -66.1057, Us, "San Juan"),
            (25.7617, -80.1918, Us, "Miami"),
            (32.7157, -117.1611, Us, "San Diego"),
            (32.4, -116.9, Elsewhere, "south of Tijuana"),
            (19.4326, -99.1332, Elsewhere, "Mexico City"),
            (38.7223, -9.1393, Elsewhere, "Lisbon"),
            (38.0, -12.0, Elsewhere, "off Portugal"),
            (0.0, 0.0, Elsewhere, "null island"),
            (f64::NAN, 0.0, Elsewhere, "not a number"),
        ];
        for (lat, lon, want, name) in cases {
            assert_eq!(Region::of_point(lat, lon), want, "{name}");
        }
    }

    #[test]
    fn stations_found_decide_near_the_border() {
        // The Detroit River lies on the Canadian side of the coarse outline.
        let (lat, lon) = (42.30, -83.0);
        assert_eq!(Region::of_point(lat, lon), Region::Canada);
        let near = |source: SourceId, dlat: f64| StationCandidate {
            station: station_ref(source, "x", "x", StationKind::WaterLevel, lat + dlat, lon),
            offers: BTreeMap::new(),
        };
        let both = [
            near(SourceId::COOPS, 0.1),
            near(SourceId::ECCC_HYDROMETRIC, 0.02),
        ];
        assert_eq!(Region::of_spot(lat, lon, &both), Region::Canada);
        assert_eq!(Region::of_spot(lat, lon, &both[..1]), Region::Us);
        assert_eq!(Region::of_spot(lat, lon, &[]), Region::Canada);
        // Offshore stays elsewhere whatever stations are listed.
        assert_eq!(
            Region::of_spot(38.0, -12.0, &[near(SourceId::COOPS, 0.0)]),
            Region::Elsewhere
        );
    }

    #[test]
    fn a_vancouver_spot_ranks_the_vancouver_gauge() {
        let plan = plan(
            49.2863,
            -123.1,
            dfo_candidates(),
            &[(M::Tide, &["dfo-iwls", "open-meteo-marine"])],
        );
        let links = station_links(&plan, M::Tide);
        assert_eq!(links[0], ("dfo-iwls", "Vancouver (07735)"));
        assert!(links.iter().all(|(s, _)| *s == "dfo-iwls"));
        // The gridded fallback follows as a point link.
        let last = plan.chains[&M::Tide].links.last().unwrap();
        assert_eq!(last.source, SourceId::OPEN_METEO_MARINE);
        assert!(matches!(
            last.place,
            LinkPlace::Point {
                relation: Relation::Gridded,
                ..
            }
        ));
    }

    #[test]
    fn a_calgary_spot_ranks_the_bow_river_gauge() {
        let plan = plan(
            51.0505,
            -114.06,
            eccc_candidates(),
            &[
                (M::Discharge, &["eccc-hydrometric"]),
                (M::GageHeight, &["eccc-hydrometric"]),
            ],
        );
        for m in [M::Discharge, M::GageHeight] {
            match &plan.chains[&m].links[0] {
                crate::discover::rank::PlanLink {
                    source,
                    place: LinkPlace::Station { station, .. },
                } => {
                    assert_eq!(*source, SourceId::ECCC_HYDROMETRIC);
                    assert_eq!(station.id, "05BH004");
                }
                other => panic!("{other:?}"),
            }
        }
    }

    #[test]
    fn a_spot_off_portugal_gets_only_gridded_and_satellite_links() {
        let mut candidates = dfo_candidates();
        candidates.extend(eccc_candidates());
        let plan = plan(
            38.0,
            -12.0,
            candidates,
            &[
                (M::Tide, &["open-meteo-marine"]),
                (M::WaterTemperature, &["open-meteo-marine", "mur-sst"]),
                (M::SeaSurfaceTemp, &["mur-sst", "open-meteo-marine"]),
            ],
        );
        for (m, chain) in &plan.chains {
            assert!(!chain.links.is_empty(), "{m:?}");
            for link in &chain.links {
                assert!(
                    matches!(
                        link.place,
                        LinkPlace::Point {
                            relation: Relation::Gridded | Relation::Satellite,
                            ..
                        }
                    ),
                    "{m:?} {link:?}"
                );
            }
        }
    }

    #[test]
    fn border_waters_rank_by_distance_and_keep_each_sources_own_links() {
        // Lake Ontario near the Niagara mouth: a Canadian and a US station in one ranking.
        let at = |source: SourceId, name: &str, lat: f64, lon: f64| StationCandidate {
            station: station_ref(source, name, name, StationKind::TideHarmonic, lat, lon),
            offers: offers(&[(M::Tide, true)]),
        };
        let candidates = vec![
            at(SourceId::COOPS, "us-near", 43.26, -79.06),
            at(SourceId::DFO_IWLS, "ca-near", 43.25, -79.07),
            at(SourceId::DFO_IWLS, "ca-far", 43.65, -79.38),
            at(SourceId::COOPS, "us-far", 43.2, -78.2),
        ];
        let input = DiscoveryInput {
            latitude: 43.255,
            longitude: -79.065,
            now: parse_rfc3339("2026-11-17T10:00:00Z").unwrap(),
            candidates,
            chains: BTreeMap::from([(M::Tide, chain(&["dfo-iwls", "coops"], Some(100.0)))]),
            ..DiscoveryInput::default()
        };
        let ranking = rank(&input);
        let order: Vec<&str> = ranking.measurements[&M::Tide]
            .stations
            .iter()
            .map(|s| s.station.name.as_str())
            .collect();
        assert_eq!(order, ["us-near", "ca-near", "ca-far", "us-far"]);
        // The plan follows the chain, each source drawing only its own stations.
        let plan = ranking.plan(2);
        assert_eq!(
            station_links(&plan, M::Tide),
            [
                ("dfo-iwls", "ca-near"),
                ("dfo-iwls", "ca-far"),
                ("coops", "us-near"),
                ("coops", "us-far"),
            ]
        );
    }
}
