//! The water at a dropped marker: what the hydrography says is there.
//!
//! The tapped point is snapped on the device to the 3DHP flowlines and waterbodies around it
//! with the tolerances of [`HydroOptions`]; nothing about the exact point is looked up
//! elsewhere. The water's name is the source's own: a feature without one stays unnamed, and a
//! lake or pond never takes the name of the stream through it. The kind of water is the app's
//! water type for the place ([`infer_water_type`]), refined into the word an angler would use
//! (stream, lake, pond, reservoir, tidal river, estuary, bay or harbor).
//!
//! 3DHP types tidal rivers and estuarine water as plain rivers, so tide is decided from what
//! the caller knows locally: a tide station on the water nearby (see [`TIDE_STATION_M`]).

use super::hydro::{
    Flowline, Hydro, HydroOptions, HydroPosition, Point, SnapBasis, Waterbody, WaterbodyKind,
    infer_water_type,
};
use super::rank::distance_m;
use crate::model::WaterType;
use crate::provenance::UsgsSiteType;

/// A tide station this close to the point is on its water, which is then tidal, metres. A lake
/// or reservoir is never tidal, whatever stands beside it.
pub const TIDE_STATION_M: f64 = 2_000.0;

/// A point this near the centre line of a mapped stream is on it, metres: the accuracy of a
/// finger on a map, not of the mapping.
pub const ON_STREAM_M: f64 = 15.0;

/// What the water is called.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum WaterKind {
    /// A mapped stream, creek or brook.
    Stream,
    /// A river (a wide one is mapped as a waterbody).
    River,
    Lake,
    Pond,
    Reservoir,
    /// A river with tide in it.
    TidalRiver,
    Estuary,
    /// A bay, harbor, sound or the like, with tide.
    Bay,
    /// A waterbody the source gives no more specific type.
    Waterbody,
}

impl WaterKind {
    /// A stable code for the app: `stream`, `tidal_river`, ...
    pub const fn code(self) -> &'static str {
        match self {
            Self::Stream => "stream",
            Self::River => "river",
            Self::Lake => "lake",
            Self::Pond => "pond",
            Self::Reservoir => "reservoir",
            Self::TidalRiver => "tidal_river",
            Self::Estuary => "estuary",
            Self::Bay => "bay",
            Self::Waterbody => "waterbody",
        }
    }
}

/// Where the point is against the water.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Reach {
    /// In the waterbody, or on the stream.
    On,
    /// Beside it, metres away.
    Away { distance_m: f64 },
}

/// The water at a point.
#[derive(Clone, Debug, PartialEq)]
pub struct WaterHere {
    /// The source's name for it; `None` when the source gives none.
    pub name: Option<String>,
    pub kind: WaterKind,
    pub reach: Reach,
    /// The app's water type for a spot here (the type "Add as spot" carries).
    pub water_type: WaterType,
    /// The stream's order, for a stream.
    pub stream_order: Option<u8>,
    /// Waterbody area, km², for a lake, pond or reservoir.
    pub area_km2: Option<f64>,
}

/// The water at `tap`, from the flowlines and waterbodies around it; `None` when none is within
/// reach (open land, or water the mapping does not hold). `tide_stations` are the places of
/// tide stations known on the device.
pub fn water_here(
    tap: Point,
    flowlines: &[Flowline],
    waterbodies: &[Waterbody],
    tide_stations: &[Point],
    options: HydroOptions,
) -> Option<WaterHere> {
    let position = Hydro::new(flowlines, waterbodies, options).locate_spot(tap);
    describe(&position, tap, tide_stations, options)
}

fn describe(
    position: &HydroPosition,
    tap: Point,
    tide_stations: &[Point],
    options: HydroOptions,
) -> Option<WaterHere> {
    if position.waterbody.is_none() && position.stream.is_none() {
        return None;
    }
    let tide_near = tide_stations.iter().any(|s| {
        distance_m(tap.latitude, tap.longitude, s.latitude, s.longitude) <= TIDE_STATION_M
    });
    // A tide station on the water is what a tidal USGS site is to the inference.
    let site = tide_near.then_some(UsgsSiteType::TidalStream);
    let water_type = infer_water_type(position, site, None, options)?.water_type;
    let stream = position.stream.as_ref();
    if let Some(w) = &position.waterbody {
        let kind = match (w.kind, water_type) {
            (_, WaterType::Pond) => WaterKind::Pond,
            (_, WaterType::BayInshore) => WaterKind::Bay,
            (WaterbodyKind::Lake, _) => WaterKind::Lake,
            (WaterbodyKind::Reservoir, _) => WaterKind::Reservoir,
            (WaterbodyKind::Estuary, _) => WaterKind::Estuary,
            (_, WaterType::TidalRiverEstuary) => WaterKind::TidalRiver,
            (WaterbodyKind::River, _) => WaterKind::River,
            (WaterbodyKind::Other, _) => WaterKind::Waterbody,
        };
        let river_like = matches!(
            w.kind,
            WaterbodyKind::River | WaterbodyKind::Estuary | WaterbodyKind::Other
        );
        // A wide river's polygon often has no name of its own while the reach through it does.
        let through = stream
            .filter(|s| river_like && s.basis == SnapBasis::WaterbodyPath)
            .and_then(|s| s.name.clone());
        return Some(WaterHere {
            name: w.name.clone().or(through),
            kind,
            reach: if w.inside {
                Reach::On
            } else {
                Reach::Away {
                    distance_m: w.distance_m,
                }
            },
            water_type,
            stream_order: None,
            area_km2: matches!(
                kind,
                WaterKind::Lake | WaterKind::Pond | WaterKind::Reservoir
            )
            .then_some(w.area_km2),
        });
    }
    let s = stream?;
    Some(WaterHere {
        name: s.name.clone(),
        kind: if water_type == WaterType::TidalRiverEstuary {
            WaterKind::TidalRiver
        } else {
            WaterKind::Stream
        },
        reach: if s.offset_m <= ON_STREAM_M {
            Reach::On
        } else {
            Reach::Away {
                distance_m: s.offset_m,
            }
        },
        water_type,
        stream_order: s.stream_order,
        area_km2: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discover::hydro::FlowlineKind;

    fn line(name: Option<&str>, order: u8, points: &[(f64, f64)]) -> Flowline {
        Flowline {
            id: "f1".into(),
            name: name.map(str::to_owned),
            kind: FlowlineKind::ChannelLine,
            stream_order: Some(order),
            level_path: 1,
            hydrosequence: 1,
            dn_hydrosequence: 0,
            up_hydrosequence: 0,
            length_km: None,
            paths: vec![points.iter().map(|&(la, lo)| Point::new(la, lo)).collect()],
        }
    }

    fn lake(name: Option<&str>, kind: WaterbodyKind, area_km2: f64) -> Waterbody {
        let ring = [
            (41.00, -73.00),
            (41.00, -72.99),
            (41.01, -72.99),
            (41.01, -73.00),
        ];
        Waterbody {
            id: "w1".into(),
            gnis_id: None,
            name: name.map(str::to_owned),
            kind,
            area_km2,
            rings: vec![ring.iter().map(|&(la, lo)| Point::new(la, lo)).collect()],
        }
    }

    fn here(
        tap: Point,
        flow: &[Flowline],
        bodies: &[Waterbody],
        stations: &[Point],
    ) -> Option<WaterHere> {
        water_here(tap, flow, bodies, stations, HydroOptions::default())
    }

    #[test]
    fn a_tap_on_a_named_stream_is_on_a_stream() {
        let flow = [line(
            Some("Mill Creek"),
            3,
            &[(41.0, -73.1), (41.01, -73.1)],
        )];
        let found = here(Point::new(41.005, -73.1), &flow, &[], &[]).unwrap();
        assert_eq!(found.name.as_deref(), Some("Mill Creek"));
        assert_eq!(found.kind, WaterKind::Stream);
        assert_eq!(found.reach, Reach::On);
        assert_eq!(found.water_type, WaterType::RiverStream);
        assert_eq!(found.stream_order, Some(3));
    }

    #[test]
    fn a_tap_beside_a_stream_says_how_far() {
        let flow = [line(None, 1, &[(41.0, -73.1), (41.01, -73.1)])];
        // About 0.0009 degrees of longitude, 75 m, east of the line.
        let found = here(Point::new(41.005, -73.099_1), &flow, &[], &[]).unwrap();
        assert_eq!(found.name, None);
        let Reach::Away { distance_m } = found.reach else {
            panic!("on the stream")
        };
        assert!((60.0..90.0).contains(&distance_m), "{distance_m}");
    }

    #[test]
    fn a_lake_is_a_lake_a_small_one_a_pond_and_neither_takes_a_stream_name() {
        let flow = [line(
            Some("Inlet Brook"),
            1,
            &[(41.005, -73.01), (41.005, -72.995)],
        )];
        let tap = Point::new(41.005, -72.995);
        let big = here(
            tap,
            &flow,
            &[lake(Some("Round Lake"), WaterbodyKind::Lake, 1.2)],
            &[],
        )
        .unwrap();
        assert_eq!((big.kind, big.reach), (WaterKind::Lake, Reach::On));
        assert_eq!(big.name.as_deref(), Some("Round Lake"));
        assert_eq!(big.water_type, WaterType::LakeReservoir);
        let unnamed = here(
            tap,
            &flow,
            &[lake(None, WaterbodyKind::Reservoir, 1.2)],
            &[],
        )
        .unwrap();
        assert_eq!(unnamed.name, None);
        assert_eq!(unnamed.kind, WaterKind::Reservoir);
        let small = here(tap, &flow, &[lake(None, WaterbodyKind::Lake, 0.01)], &[]).unwrap();
        assert_eq!(
            (small.kind, small.water_type),
            (WaterKind::Pond, WaterType::Pond)
        );
    }

    #[test]
    fn a_river_polygon_takes_the_name_of_the_reach_through_it() {
        let flow = [line(
            Some("Big River"),
            8,
            &[(41.0, -72.995), (41.01, -72.995)],
        )];
        let found = here(
            Point::new(41.005, -72.9975),
            &flow,
            &[lake(None, WaterbodyKind::River, 20.0)],
            &[],
        )
        .unwrap();
        assert_eq!(found.name.as_deref(), Some("Big River"));
        assert_eq!(found.kind, WaterKind::River);
    }

    #[test]
    fn a_tide_station_on_the_water_makes_a_river_tidal_but_never_a_lake() {
        let station = [Point::new(41.006, -72.9975)];
        let tap = Point::new(41.005, -72.9975);
        let river = here(
            tap,
            &[],
            &[lake(None, WaterbodyKind::River, 20.0)],
            &station,
        )
        .unwrap();
        assert_eq!(river.kind, WaterKind::TidalRiver);
        assert_eq!(river.water_type, WaterType::TidalRiverEstuary);
        let still = here(tap, &[], &[lake(None, WaterbodyKind::Lake, 20.0)], &station).unwrap();
        assert_eq!(still.kind, WaterKind::Lake);
        let bay = here(
            tap,
            &[],
            &[lake(Some("Gull Harbor"), WaterbodyKind::River, 20.0)],
            &station,
        )
        .unwrap();
        assert_eq!(
            (bay.kind, bay.water_type),
            (WaterKind::Bay, WaterType::BayInshore)
        );
    }

    #[test]
    fn open_land_has_no_water() {
        let flow = [line(
            Some("Mill Creek"),
            3,
            &[(41.0, -73.1), (41.01, -73.1)],
        )];
        let far = Point::new(41.2, -73.4);
        assert_eq!(
            here(far, &flow, &[lake(None, WaterbodyKind::Lake, 1.0)], &[far]),
            None
        );
    }
}
