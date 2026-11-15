//! Same water body, upstream and downstream relations from the hydrography network.
//!
//! Inputs are 3DHP flowlines (layer 50) and waterbodies (layer 60) as plain WGS84 geometry.
//! `hydrosequence` grows going upstream: a reach's downstream neighbour is the reach whose
//! `hydrosequence` equals its `dn_hydrosequence` (0 at an outlet), and a level path's id is the
//! `hydrosequence` of its downstream-most reach. Reaches are digitised in the flow direction, so
//! the first vertex of a reach is its upstream end.
//!
//! Distances use a sphere of the mean Earth radius: haversine for lengths along a line, and a local
//! equirectangular projection around the query point for the distance to a segment, which is
//! enough within a few tens of kilometres. Distances along a river follow the source's reach
//! lengths (`lengthkm`) spread over the geometry, because coordinates rounded to 4 decimals make a
//! polyline a few percent longer. Estuary connectors carry no tidal information, so nothing here
//! infers ebb or flood from them.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::model::{WaterType, ZoneKind};
use crate::provenance::{Relation, SourceId, StationKind, StationRef, UsgsSiteType};

/// Mean Earth radius (IUGG), metres.
const EARTH_RADIUS_M: f64 = 6_371_008.8;

/// A WGS84 position in degrees.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub latitude: f64,
    pub longitude: f64,
}

impl Point {
    pub const fn new(latitude: f64, longitude: f64) -> Self {
        Self {
            latitude,
            longitude,
        }
    }
}

/// 3DHP flowline type (`featuretypelabel`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlowlineKind {
    ChannelLine,
    /// A path through a lake or an estuary.
    WaterbodyConnector,
    SurfaceConnector,
    Other,
}

impl FlowlineKind {
    pub fn from_label(label: &str) -> Self {
        match label {
            "Channel Line" => Self::ChannelLine,
            "Waterbody Connector" => Self::WaterbodyConnector,
            "Surface Connector" => Self::SurfaceConnector,
            _ => Self::Other,
        }
    }
}

/// 3DHP waterbody type (`featuretypelabel`). Tidal river and estuary water is typed `River`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WaterbodyKind {
    Lake,
    Reservoir,
    River,
    Estuary,
    Other,
}

impl WaterbodyKind {
    pub fn from_label(label: &str) -> Self {
        match label {
            "Lake" | "Lake/Pond" => Self::Lake,
            "Reservoir" => Self::Reservoir,
            "River" => Self::River,
            "Estuary" => Self::Estuary,
            _ => Self::Other,
        }
    }
}

/// One flowline reach (layer 50).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Flowline {
    /// `id3dhp`.
    pub id: String,
    /// `gnisidlabel`; absent for most small streams.
    pub name: Option<String>,
    pub kind: FlowlineKind,
    /// Strahler order.
    pub stream_order: Option<u8>,
    pub level_path: u64,
    pub hydrosequence: u64,
    /// 0 at an outlet.
    pub dn_hydrosequence: u64,
    /// 0 at a headwater.
    pub up_hydrosequence: u64,
    /// `lengthkm`; along-river distances follow it rather than the generalised geometry.
    pub length_km: Option<f64>,
    /// The polyline's paths in flow direction.
    pub paths: Vec<Vec<Point>>,
}

/// One waterbody polygon (layer 60). A lake may arrive as several polygons sharing a GNIS id.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Waterbody {
    /// `id3dhp`.
    pub id: String,
    pub gnis_id: Option<u64>,
    pub name: Option<String>,
    pub kind: WaterbodyKind,
    pub area_km2: f64,
    /// Outer rings, holes and further parts in any order and orientation (even-odd rule).
    pub rings: Vec<Vec<Point>>,
}

impl Waterbody {
    /// Whether the point is inside the polygon; islands (holes) are outside.
    pub fn contains(&self, p: Point) -> bool {
        let mut inside = false;
        for ring in &self.rings {
            for (a, b) in edges(ring) {
                if (a.latitude > p.latitude) != (b.latitude > p.latitude) {
                    let x = a.longitude
                        + (p.latitude - a.latitude) * (b.longitude - a.longitude)
                            / (b.latitude - a.latitude);
                    if p.longitude < x {
                        inside = !inside;
                    }
                }
            }
        }
        inside
    }

    /// Distance from the point to the nearest edge, metres (0 inside).
    pub fn distance_m(&self, p: Point) -> f64 {
        if self.contains(p) {
            return 0.0;
        }
        self.rings
            .iter()
            .flat_map(|r| edges(r))
            .map(|(a, b)| segment_distance(p, a, b).0)
            .fold(f64::INFINITY, f64::min)
    }
}

/// Tolerances for snapping and inference.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct HydroOptions {
    /// A point farther than this from every flowline is not on a mapped stream.
    pub snap_tolerance_m: f64,
    /// A station this close to a waterbody measures it from the shore or a dam.
    pub shore_tolerance_m: f64,
    /// Lakes smaller than this (all parts together) are ponds.
    pub pond_max_area_km2: f64,
}

impl Default for HydroOptions {
    fn default() -> Self {
        Self {
            snap_tolerance_m: 200.0,
            shore_tolerance_m: 200.0,
            pond_max_area_km2: 0.04,
        }
    }
}

/// A waterbody as one lake: polygons sharing a GNIS id, or a single unnamed polygon.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WaterbodyKey {
    Gnis(u64),
    Feature(String),
}

/// The waterbody a point is in or beside.
#[derive(Clone, Debug, PartialEq)]
pub struct WaterbodyHit {
    pub key: WaterbodyKey,
    pub name: Option<String>,
    /// Type of the part the point is in or nearest to.
    pub kind: WaterbodyKind,
    /// Sum over all parts.
    pub area_km2: f64,
    pub inside: bool,
    pub distance_m: f64,
}

/// How a point was placed on a flowline.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapBasis {
    /// The nearest flowline.
    Nearest,
    /// A named flowline within tolerance over a closer unnamed one.
    PreferredNamed,
    /// The main flow path through the waterbody the point is on.
    WaterbodyPath,
}

/// A point placed on the network.
#[derive(Clone, Debug, PartialEq)]
pub struct StreamPosition {
    index: usize,
    pub flowline_id: String,
    pub name: Option<String>,
    pub kind: FlowlineKind,
    pub stream_order: Option<u8>,
    pub level_path: u64,
    pub hydrosequence: u64,
    /// Along the reach from its upstream end, metres, scaled to the source's reach length.
    pub measure_m: f64,
    /// From the point to the flowline, metres.
    pub offset_m: f64,
    pub basis: SnapBasis,
}

/// Where a spot or station sits in the hydrography.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HydroPosition {
    pub waterbody: Option<WaterbodyHit>,
    pub stream: Option<StreamPosition>,
}

/// Which of the two places a reason is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Subject {
    Spot,
    Station,
}

/// Why a relation was chosen.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum HydroReason {
    /// The station is inside the spot's waterbody.
    InsideWaterbody { name: Option<String> },
    /// The station measures the spot's waterbody from its shore or dam.
    WaterbodyShore {
        name: Option<String>,
        distance_m: f64,
    },
    /// Both are on the same river (level path).
    SameRiver { name: Option<String> },
    /// One stream flows into the other's river.
    Tributary {
        on_tributary: Subject,
        tributary: Option<String>,
        river: Option<String>,
    },
    /// Both are on mapped streams that do not join within the loaded network.
    NotConnected,
    /// Farther than the snap tolerance from every flowline.
    NotOnMappedStream { subject: Subject },
    /// How a place was put on a flowline, when not simply the nearest one.
    Snapped {
        subject: Subject,
        basis: SnapBasis,
        name: Option<String>,
    },
}

/// The relation of one station to the spot, with the distance along the river where known.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RelationHint {
    pub relation: Relation,
    /// Along the flowlines, metres, rounded to the metre.
    pub along_river_m: Option<f64>,
    pub reasons: Vec<HydroReason>,
}

/// A station's identity in the relation map.
pub type StationKey = (SourceId, String);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Spot,
    WaterbodyStation,
    StreamGauge,
}

/// The hydrography around a spot, indexed for relation queries.
#[derive(Debug)]
pub struct Hydro<'a> {
    flowlines: &'a [Flowline],
    waterbodies: &'a [Waterbody],
    options: HydroOptions,
    by_hydrosequence: BTreeMap<u64, usize>,
    groups: BTreeMap<WaterbodyKey, Vec<usize>>,
    lengths_m: Vec<f64>,
    /// Source length over geometry length, per flowline.
    scales: Vec<f64>,
}

impl<'a> Hydro<'a> {
    pub fn new(
        flowlines: &'a [Flowline],
        waterbodies: &'a [Waterbody],
        options: HydroOptions,
    ) -> Self {
        let by_hydrosequence = flowlines
            .iter()
            .enumerate()
            .map(|(i, f)| (f.hydrosequence, i))
            .collect();
        let mut groups: BTreeMap<WaterbodyKey, Vec<usize>> = BTreeMap::new();
        for (i, w) in waterbodies.iter().enumerate() {
            let key = w
                .gnis_id
                .map_or_else(|| WaterbodyKey::Feature(w.id.clone()), WaterbodyKey::Gnis);
            groups.entry(key).or_default().push(i);
        }
        let geometry_m: Vec<f64> = flowlines
            .iter()
            .map(|f| {
                f.paths
                    .iter()
                    .flat_map(|p| p.windows(2))
                    .map(|s| haversine_m(s[0], s[1]))
                    .sum()
            })
            .collect();
        let scales: Vec<f64> = flowlines
            .iter()
            .zip(&geometry_m)
            .map(|(f, &g)| match f.length_km {
                Some(km) if km > 0.0 && g > 0.0 => km * 1000.0 / g,
                _ => 1.0,
            })
            .collect();
        let lengths_m = geometry_m.iter().zip(&scales).map(|(g, k)| g * k).collect();
        Self {
            flowlines,
            waterbodies,
            options,
            by_hydrosequence,
            groups,
            lengths_m,
            scales,
        }
    }

    /// Places the spot: in or at the shore of a waterbody (then on its main flow path), or on the
    /// nearest flowline within the snap tolerance, preferring named ones.
    pub fn locate_spot(&self, spot: Point) -> HydroPosition {
        self.locate(spot, Mode::Spot)
    }

    /// The nearest flowline within the snap tolerance, a named one over a closer unnamed one.
    pub fn snap(&self, p: Point) -> Option<StreamPosition> {
        let mut nearest: Option<StreamPosition> = None;
        let mut named: Option<StreamPosition> = None;
        for i in 0..self.flowlines.len() {
            let pos = self.position_on(i, p, SnapBasis::Nearest);
            if pos.offset_m > self.options.snap_tolerance_m {
                continue;
            }
            if pos.name.is_some() && closer(&pos, named.as_ref()) {
                named = Some(pos.clone());
            }
            if closer(&pos, nearest.as_ref()) {
                nearest = Some(pos);
            }
        }
        match (named, nearest) {
            (Some(mut n), Some(a)) if n.index != a.index => {
                n.basis = SnapBasis::PreferredNamed;
                Some(n)
            }
            (_, a) => a,
        }
    }

    /// How one candidate station relates to a spot placed with [`Hydro::locate_spot`].
    ///
    /// `SameWaterBody`: the station is inside the spot's waterbody, or measures it from within
    /// the shore tolerance (lake, tide and marine stations; stream gauges are placed on the
    /// network instead, so a gauge below a dam is `Downstream` of a spot on the reservoir).
    /// `Upstream` / `Downstream`: both on the same level path. `Nearby`: a tributary, streams that
    /// do not join in the loaded network, or a place off the network.
    pub fn relation(&self, spot: &HydroPosition, station: &StationRef) -> RelationHint {
        let p = Point::new(station.latitude, station.longitude);
        let stream_gauge = matches!(
            station.kind,
            StationKind::UsgsSite(UsgsSiteType::Stream) | StationKind::NwpsGauge
        );
        let hint = |relation, along_river_m, reasons| RelationHint {
            relation,
            along_river_m,
            reasons,
        };
        let shared = spot
            .waterbody
            .as_ref()
            .map(|w| (w, self.group_hit(&w.key, p)));
        if let Some((w, Some(hit))) = &shared {
            let name = w.name.clone();
            if hit.inside && !stream_gauge {
                return hint(
                    Relation::SameWaterBody,
                    None,
                    vec![HydroReason::InsideWaterbody { name }],
                );
            }
            if hit.distance_m <= self.options.shore_tolerance_m && !stream_gauge {
                let distance_m = hit.distance_m.round();
                return hint(
                    Relation::SameWaterBody,
                    None,
                    vec![HydroReason::WaterbodyShore { name, distance_m }],
                );
            }
        }
        let mode = if stream_gauge {
            Mode::StreamGauge
        } else {
            Mode::WaterbodyStation
        };
        let at = self.locate(p, mode);
        let (Some(a), Some(b)) = (&spot.stream, &at.stream) else {
            let mut reasons = Vec::new();
            for (pos, subject) in [
                (&spot.stream, Subject::Spot),
                (&at.stream, Subject::Station),
            ] {
                if pos.is_none() {
                    reasons.push(HydroReason::NotOnMappedStream { subject });
                }
            }
            // A stream gauge off the network but inside the spot's waterbody still shares it.
            if let Some((w, Some(hit))) = &shared
                && hit.inside
            {
                let name = w.name.clone();
                reasons.insert(0, HydroReason::InsideWaterbody { name });
                return hint(Relation::SameWaterBody, None, reasons);
            }
            return hint(Relation::Nearby, None, reasons);
        };
        let (relation, along, reasons) = self.compare(a, b);
        hint(relation, along.map(f64::round), reasons)
    }

    /// Up, down or across the network from the spot's position `a` to the station's `b`.
    fn compare(
        &self,
        a: &StreamPosition,
        b: &StreamPosition,
    ) -> (Relation, Option<f64>, Vec<HydroReason>) {
        let mut reasons = Vec::new();
        let (relation, along) = if a.level_path == b.level_path {
            reasons.push(HydroReason::SameRiver {
                name: a.name.clone().or_else(|| b.name.clone()),
            });
            if a.index == b.index {
                let d = b.measure_m - a.measure_m;
                let relation = match d.partial_cmp(&0.0) {
                    Some(Ordering::Less) => Relation::Upstream,
                    Some(Ordering::Greater) => Relation::Downstream,
                    _ => Relation::SameWaterBody,
                };
                (relation, Some(d.abs()))
            } else if b.hydrosequence > a.hydrosequence {
                (Relation::Upstream, self.along_down(b, a))
            } else {
                (Relation::Downstream, self.along_down(a, b))
            }
        } else {
            let tributary = if self.joins(b, a.level_path) {
                Some((Subject::Station, b, a))
            } else if self.joins(a, b.level_path) {
                Some((Subject::Spot, a, b))
            } else {
                None
            };
            reasons.push(
                tributary.map_or(HydroReason::NotConnected, |(on_tributary, t, r)| {
                    HydroReason::Tributary {
                        on_tributary,
                        tributary: t.name.clone(),
                        river: r.name.clone(),
                    }
                }),
            );
            (Relation::Nearby, None)
        };
        for (pos, subject) in [(a, Subject::Spot), (b, Subject::Station)] {
            if pos.basis != SnapBasis::Nearest {
                reasons.push(HydroReason::Snapped {
                    subject,
                    basis: pos.basis,
                    name: pos.name.clone(),
                });
            }
        }
        (relation, along, reasons)
    }

    /// Relation hints for every station, keyed by source and station id.
    pub fn relations(
        &self,
        spot: Point,
        stations: &[StationRef],
    ) -> BTreeMap<StationKey, RelationHint> {
        let at = self.locate_spot(spot);
        stations
            .iter()
            .map(|s| ((s.source.clone(), s.id.clone()), self.relation(&at, s)))
            .collect()
    }

    fn locate(&self, p: Point, mode: Mode) -> HydroPosition {
        let snapped = self.snap(p);
        let mut hits: Vec<WaterbodyHit> = self
            .groups
            .keys()
            .filter_map(|k| self.group_hit(k, p))
            .collect();
        // Inside first, smallest (most specific) first; otherwise nearest.
        hits.sort_by(|x, y| {
            y.inside
                .cmp(&x.inside)
                .then(x.distance_m.total_cmp(&y.distance_m))
                .then(x.area_km2.total_cmp(&y.area_km2))
                .then(x.key.cmp(&y.key))
        });
        let waterbody = hits.into_iter().next().filter(|h| {
            h.inside
                || match mode {
                    Mode::StreamGauge => false,
                    Mode::WaterbodyStation => h.distance_m <= self.options.shore_tolerance_m,
                    Mode::Spot => {
                        h.distance_m <= self.options.shore_tolerance_m
                            && snapped.as_ref().is_none_or(|s| s.offset_m > h.distance_m)
                    }
                }
        });
        let stream = match (&waterbody, mode) {
            (Some(w), Mode::Spot | Mode::WaterbodyStation) => {
                self.waterbody_path(&w.key, p).or(snapped)
            }
            _ => snapped,
        };
        HydroPosition { waterbody, stream }
    }

    fn group_hit(&self, key: &WaterbodyKey, p: Point) -> Option<WaterbodyHit> {
        let parts = self.groups.get(key)?;
        let (part, distance_m) = parts
            .iter()
            .map(|&i| (i, self.waterbodies[i].distance_m(p)))
            .min_by(|x, y| x.1.total_cmp(&y.1))?;
        let w = &self.waterbodies[part];
        Some(WaterbodyHit {
            key: key.clone(),
            name: parts.iter().find_map(|&i| self.waterbodies[i].name.clone()),
            kind: w.kind,
            area_km2: parts.iter().map(|&i| self.waterbodies[i].area_km2).sum(),
            inside: w.contains(p),
            distance_m,
        })
    }

    /// The highest-order flowline through the waterbody (named first), nearest the point.
    fn waterbody_path(&self, key: &WaterbodyKey, p: Point) -> Option<StreamPosition> {
        let parts = self.groups.get(key)?;
        let inside = |q: Point| parts.iter().any(|&i| self.waterbodies[i].contains(q));
        let through: Vec<usize> = (0..self.flowlines.len())
            .filter(|&i| {
                self.flowlines[i]
                    .paths
                    .iter()
                    .flat_map(|l| l.windows(2))
                    .any(|s| {
                        inside(s[0])
                            || inside(Point::new(
                                f64::midpoint(s[0].latitude, s[1].latitude),
                                f64::midpoint(s[0].longitude, s[1].longitude),
                            ))
                    })
            })
            .collect();
        let rank = |i: usize| {
            (
                self.flowlines[i].stream_order,
                self.flowlines[i].name.is_some(),
            )
        };
        let best = through.iter().map(|&i| rank(i)).max()?;
        through
            .into_iter()
            .filter(|&i| rank(i) == best)
            .map(|i| self.position_on(i, p, SnapBasis::WaterbodyPath))
            .fold(None, |acc, pos| {
                if closer(&pos, acc.as_ref()) {
                    Some(pos)
                } else {
                    acc
                }
            })
    }

    fn position_on(&self, index: usize, p: Point, basis: SnapBasis) -> StreamPosition {
        let f = &self.flowlines[index];
        let (mut offset_m, mut measure_m, mut run) = (f64::INFINITY, 0.0, 0.0);
        for s in f.paths.iter().flat_map(|l| l.windows(2)) {
            let (d, t) = segment_distance(p, s[0], s[1]);
            let len = haversine_m(s[0], s[1]);
            if d < offset_m {
                offset_m = d;
                measure_m = run + t * len;
            }
            run += len;
        }
        StreamPosition {
            index,
            flowline_id: f.id.clone(),
            name: f.name.clone(),
            kind: f.kind,
            stream_order: f.stream_order,
            level_path: f.level_path,
            hydrosequence: f.hydrosequence,
            measure_m: measure_m * self.scales[index],
            offset_m,
            basis,
        }
    }

    /// Walks downstream from `from` and returns the reaches visited after it, stopping at an
    /// outlet, a reach not loaded or a loop.
    fn downstream(&self, from: usize) -> impl Iterator<Item = usize> + '_ {
        let mut cur = from;
        (0..self.flowlines.len()).map_while(move |_| {
            let dn = self.flowlines[cur].dn_hydrosequence;
            let next = *self.by_hydrosequence.get(&dn).filter(|_| dn != 0)?;
            cur = next;
            Some(next)
        })
    }

    /// Distance along the flowlines from `up` down to `down`, if `down` is reached.
    fn along_down(&self, up: &StreamPosition, down: &StreamPosition) -> Option<f64> {
        let mut d = self.lengths_m[up.index] - up.measure_m;
        for i in self.downstream(up.index) {
            if i == down.index {
                return Some(d + down.measure_m);
            }
            d += self.lengths_m[i];
        }
        None
    }

    fn joins(&self, from: &StreamPosition, level_path: u64) -> bool {
        self.downstream(from.index)
            .any(|i| self.flowlines[i].level_path == level_path)
    }
}

/// Relation hints for every station near a spot.
pub fn relations(
    spot: Point,
    stations: &[StationRef],
    flowlines: &[Flowline],
    waterbodies: &[Waterbody],
    options: HydroOptions,
) -> BTreeMap<StationKey, RelationHint> {
    Hydro::new(flowlines, waterbodies, options).relations(spot, stations)
}

/// Why a water type was suggested.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum WaterTypeReason {
    /// The NWS marine zone the spot lies in.
    MarineZone { kind: ZoneKind },
    /// The waterbody the spot is on; area of all parts, km², to 0.01.
    Waterbody {
        name: Option<String>,
        kind: WaterbodyKind,
        area_km2: f64,
    },
    /// A bay, harbour, sound or similar name.
    BayName { name: String },
    /// The stream the spot is on.
    Stream {
        name: Option<String>,
        stream_order: Option<u8>,
    },
    /// The type of the nearest USGS site.
    UsgsSite { site_type: UsgsSiteType },
}

/// A suggested water type with its reasons. The spot's stored choice always wins.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WaterTypeSuggestion {
    pub water_type: WaterType,
    pub reasons: Vec<WaterTypeReason>,
}

/// Suggests a water type from the spot's hydrography, the nearest USGS site's type and the NWS
/// marine zone it lies in. `None` when nothing points anywhere.
pub fn infer_water_type(
    spot: &HydroPosition,
    nearest_usgs_site: Option<UsgsSiteType>,
    zone: Option<ZoneKind>,
    options: HydroOptions,
) -> Option<WaterTypeSuggestion> {
    let mut reasons = Vec::new();
    let done = |water_type, reasons| {
        Some(WaterTypeSuggestion {
            water_type,
            reasons,
        })
    };
    if let Some(kind) = zone {
        reasons.push(WaterTypeReason::MarineZone { kind });
        match kind {
            ZoneKind::Offshore => return done(WaterType::Offshore, reasons),
            ZoneKind::GreatLakesNearshore | ZoneKind::GreatLakesOpen => {
                return done(WaterType::GreatLakes, reasons);
            }
            ZoneKind::Coastal => {}
        }
    }
    let coastal = zone == Some(ZoneKind::Coastal);
    let tidal_site = matches!(
        nearest_usgs_site,
        Some(UsgsSiteType::Estuary | UsgsSiteType::TidalStream)
    );
    if tidal_site {
        reasons.extend(nearest_usgs_site.map(|site_type| WaterTypeReason::UsgsSite { site_type }));
    }
    // Salt or tidal water, as far as the zone and the nearest USGS site tell.
    let tidal = coastal || tidal_site;
    if let Some(w) = &spot.waterbody {
        reasons.push(WaterTypeReason::Waterbody {
            name: w.name.clone(),
            kind: w.kind,
            area_km2: (w.area_km2 * 100.0).round() / 100.0,
        });
        let bay = w.name.as_deref().filter(|n| tidal && bay_like(n));
        if let Some(name) = bay {
            reasons.push(WaterTypeReason::BayName {
                name: name.to_owned(),
            });
            return done(WaterType::BayInshore, reasons);
        }
        let water_type = match w.kind {
            WaterbodyKind::Lake | WaterbodyKind::Reservoir
                if w.area_km2 < options.pond_max_area_km2 =>
            {
                WaterType::Pond
            }
            WaterbodyKind::Lake | WaterbodyKind::Reservoir => WaterType::LakeReservoir,
            WaterbodyKind::Estuary => WaterType::TidalRiverEstuary,
            WaterbodyKind::River | WaterbodyKind::Other if tidal => WaterType::TidalRiverEstuary,
            WaterbodyKind::River | WaterbodyKind::Other => WaterType::RiverStream,
        };
        return done(water_type, reasons);
    }
    if let Some(s) = &spot.stream {
        reasons.push(WaterTypeReason::Stream {
            name: s.name.clone(),
            stream_order: s.stream_order,
        });
        return done(
            if tidal {
                WaterType::TidalRiverEstuary
            } else {
                WaterType::RiverStream
            },
            reasons,
        );
    }
    if tidal {
        return done(
            if coastal {
                WaterType::Nearshore
            } else {
                WaterType::TidalRiverEstuary
            },
            reasons,
        );
    }
    let site_type = nearest_usgs_site?;
    let water_type = match site_type {
        UsgsSiteType::Stream => WaterType::RiverStream,
        UsgsSiteType::Lake => WaterType::LakeReservoir,
        UsgsSiteType::Estuary | UsgsSiteType::TidalStream | UsgsSiteType::Other => return None,
    };
    done(water_type, vec![WaterTypeReason::UsgsSite { site_type }])
}

/// The water type to use: the spot's stored choice, else the suggestion.
pub fn choose_water_type(
    stored: Option<WaterType>,
    suggestion: Option<&WaterTypeSuggestion>,
) -> Option<WaterType> {
    stored.or(suggestion.map(|s| s.water_type))
}

fn bay_like(name: &str) -> bool {
    const WORDS: [&str; 8] = [
        "bay", "harbor", "harbour", "sound", "inlet", "cove", "lagoon", "bight",
    ];
    name.split(|c: char| !c.is_alphanumeric())
        .any(|w| WORDS.iter().any(|k| w.eq_ignore_ascii_case(k)))
}

fn closer(pos: &StreamPosition, than: Option<&StreamPosition>) -> bool {
    than.is_none_or(|t| {
        pos.offset_m
            .total_cmp(&t.offset_m)
            .then_with(|| pos.flowline_id.cmp(&t.flowline_id))
            .is_lt()
    })
}

fn edges(ring: &[Point]) -> impl Iterator<Item = (Point, Point)> + '_ {
    ring.iter()
        .zip(ring.iter().cycle().skip(1))
        .map(|(a, b)| (*a, *b))
}

fn haversine_m(a: Point, b: Point) -> f64 {
    let (la1, la2) = (a.latitude.to_radians(), b.latitude.to_radians());
    let dlat = la2 - la1;
    let dlon = (b.longitude - a.longitude).to_radians();
    let h = (dlat / 2.0).sin().powi(2) + la1.cos() * la2.cos() * (dlon / 2.0).sin().powi(2);
    2.0 * EARTH_RADIUS_M * h.sqrt().min(1.0).asin()
}

/// Distance from `p` to segment `ab`, metres, and the position along it (0 at `a`, 1 at `b`), in
/// an equirectangular projection centred on `p`.
fn segment_distance(p: Point, a: Point, b: Point) -> (f64, f64) {
    let m_per_deg = EARTH_RADIUS_M.to_radians();
    let cos_lat = p.latitude.to_radians().cos();
    let xy = |q: Point| {
        (
            (q.longitude - p.longitude) * cos_lat * m_per_deg,
            (q.latitude - p.latitude) * m_per_deg,
        )
    };
    let ((ax, ay), (bx, by)) = (xy(a), xy(b));
    let (dx, dy) = (bx - ax, by - ay);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 {
        (-(ax * dx + ay * dy) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    ((ax + t * dx).hypot(ay + t * dy), t)
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;

    fn fixture(name: &str) -> Value {
        let path = format!(
            "{}/../../tests/fixtures/raw/hydro/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    fn points(v: &Value) -> Vec<Vec<Point>> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|l| {
                l.as_array()
                    .unwrap()
                    .iter()
                    .map(|p| Point::new(p[1].as_f64().unwrap(), p[0].as_f64().unwrap()))
                    .collect()
            })
            .collect()
    }

    fn text(a: &Value, key: &str) -> Option<String> {
        a[key].as_str().map(str::to_owned)
    }

    /// Layer 50 features; `level_path` fills a field the query did not ask for.
    fn flowlines(name: &str, level_path: Option<u64>) -> Vec<Flowline> {
        let int = |a: &Value, k: &str| a[k].as_f64().map(|x| x as u64);
        fixture(name)["features"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| {
                let a = &f["attributes"];
                Flowline {
                    id: text(a, "id3dhp").unwrap(),
                    name: text(a, "gnisidlabel"),
                    kind: FlowlineKind::from_label(a["featuretypelabel"].as_str().unwrap_or("")),
                    stream_order: int(a, "streamorder").map(|o| o as u8),
                    level_path: int(a, "levelpath").or(level_path).unwrap(),
                    hydrosequence: int(a, "hydrosequence").unwrap(),
                    dn_hydrosequence: int(a, "dnhydrosequence").unwrap(),
                    up_hydrosequence: int(a, "uphydrosequence").unwrap(),
                    length_km: a["lengthkm"].as_f64(),
                    paths: points(&f["geometry"]["paths"]),
                }
            })
            .collect()
    }

    /// Layer 60 features; `rings` supplies outlines the fixture lacks (synthetic).
    fn waterbodies(name: &str, rings: impl Fn(&str) -> Vec<Vec<Point>>) -> Vec<Waterbody> {
        fixture(name)["features"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| {
                let a = &f["attributes"];
                let id = text(a, "id3dhp").unwrap();
                let geometry = &f["geometry"]["rings"];
                Waterbody {
                    gnis_id: a["gnisid"].as_u64(),
                    name: text(a, "gnisidlabel"),
                    kind: WaterbodyKind::from_label(a["featuretypelabel"].as_str().unwrap()),
                    area_km2: a["areasqkm"].as_f64().unwrap(),
                    rings: if geometry.is_null() {
                        rings(&id)
                    } else {
                        points(geometry)
                    },
                    id,
                }
            })
            .collect()
    }

    /// A synthetic rectangular ring.
    fn rect(south: f64, west: f64, north: f64, east: f64) -> Vec<Point> {
        [
            (south, west),
            (north, west),
            (north, east),
            (south, east),
            (south, west),
        ]
        .map(|(la, lo)| Point::new(la, lo))
        .to_vec()
    }

    fn usgs(id: &str, site: UsgsSiteType, latitude: f64, longitude: f64) -> StationRef {
        StationRef {
            source: SourceId::USGS,
            id: id.into(),
            name: id.into(),
            kind: StationKind::UsgsSite(site),
            latitude,
            longitude,
            bin: None,
            reference_id: None,
        }
    }

    // Station coordinates from tests/fixtures/raw/usgs/usgs-monitoring-locations-bbox-croton-*.json.
    fn croton_gauge() -> StationRef {
        usgs(
            "USGS-01375000",
            UsgsSiteType::Stream,
            41.225_027_8,
            -73.858_888_9,
        )
    }
    fn purdy_brook_mouth() -> StationRef {
        usgs(
            "USGS-0137499455",
            UsgsSiteType::Stream,
            41.226_483_3,
            -73.846_802_4,
        )
    }
    fn furnace_brook() -> StationRef {
        usgs(
            "USGS-01374390",
            UsgsSiteType::Stream,
            41.239_538_6,
            -73.898_193_0,
        )
    }
    fn reservoir_site() -> StationRef {
        usgs(
            "USGS-01374995",
            UsgsSiteType::Lake,
            41.227_594_4,
            -73.857_080_5,
        )
    }

    /// On the Croton River, 1 km of channel below the gauge's snapped position.
    const BELOW_GAUGE: Point = Point::new(41.217_51, -73.864_60);
    /// On New Croton Reservoir, nearer Bailey Brook than the river's path through the lake.
    const ON_RESERVOIR: Point = Point::new(41.23, -73.84);

    fn key(s: &StationRef) -> StationKey {
        (s.source.clone(), s.id.clone())
    }

    #[test]
    fn croton_gauge_is_upstream_of_a_spot_below_it_and_a_brook_is_a_tributary() {
        let lines = flowlines("hydro-50-croton-envelope.json", None);
        let stations = [croton_gauge(), purdy_brook_mouth(), furnace_brook()];
        let map = relations(BELOW_GAUGE, &stations, &lines, &[], HydroOptions::default());

        let gauge = &map[&key(&stations[0])];
        assert_eq!(gauge.relation, Relation::Upstream);
        let along = gauge.along_river_m.unwrap();
        assert!((along - 1000.0).abs() < 15.0, "{along}");
        assert_eq!(
            gauge.reasons,
            [HydroReason::SameRiver {
                name: Some("Croton River".into())
            }]
        );

        let brook = &map[&key(&stations[1])];
        assert_eq!(brook.relation, Relation::Nearby);
        assert_eq!(
            brook.reasons,
            [HydroReason::Tributary {
                on_tributary: Subject::Station,
                tributary: Some("Purdy Brook".into()),
                river: Some("Croton River".into()),
            }]
        );

        let far = &map[&key(&stations[2])];
        assert_eq!(far.relation, Relation::Nearby);
        assert_eq!(
            far.reasons,
            [HydroReason::NotOnMappedStream {
                subject: Subject::Station
            }]
        );

        let snapshot: BTreeMap<String, &RelationHint> = map
            .iter()
            .map(|((s, id), h)| (format!("{s}/{id}"), h))
            .collect();
        insta::assert_yaml_snapshot!(snapshot);
    }

    #[test]
    fn swapping_spot_and_gauge_gives_downstream() {
        let lines = flowlines("hydro-50-croton-envelope.json", None);
        let below = usgs(
            "below",
            UsgsSiteType::Stream,
            BELOW_GAUGE.latitude,
            BELOW_GAUGE.longitude,
        );
        let gauge = croton_gauge();
        let at = Point::new(gauge.latitude, gauge.longitude);
        let map = relations(
            at,
            std::slice::from_ref(&below),
            &lines,
            &[],
            HydroOptions::default(),
        );
        let hint = &map[&key(&below)];
        assert_eq!(hint.relation, Relation::Downstream);
        assert!((hint.along_river_m.unwrap() - 1000.0).abs() < 15.0);
    }

    #[test]
    fn along_river_distance_matches_the_reach_lengths() {
        // The first 37 reaches of the Croton level path sum to 20.15 km (lengthkm).
        let name = "hydro-50-croton-levelpath-3767999-first25km-geom.json";
        let lines = flowlines(name, Some(3_767_999));
        let km: f64 = fixture(name)["features"].as_array().unwrap()[..37]
            .iter()
            .map(|f| f["attributes"]["lengthkm"].as_f64().unwrap())
            .sum();
        let mouth = Point::new(41.1526, -73.8966);
        let up = usgs("up", UsgsSiteType::Stream, 41.243, -73.7489);
        let map = relations(
            mouth,
            std::slice::from_ref(&up),
            &lines,
            &[],
            HydroOptions::default(),
        );
        let hint = &map[&key(&up)];
        assert_eq!(hint.relation, Relation::Upstream);
        let along = hint.along_river_m.unwrap();
        assert!(
            (along / (km * 1000.0) - 1.0).abs() < 0.001,
            "{along} vs {km} km"
        );
    }

    #[test]
    fn reservoir_spot_uses_the_river_path_and_the_gauge_below_the_dam_is_downstream() {
        let lines = flowlines("hydro-50-croton-envelope.json", None);
        let lakes = waterbodies("hydro-60-new-croton-reservoir-geom.json", |_| Vec::new());
        let hydro = Hydro::new(&lines, &lakes, HydroOptions::default());

        // The nearest flowline is Bailey Brook, beyond the snap tolerance.
        assert_eq!(hydro.snap(ON_RESERVOIR), None);
        let spot = hydro.locate_spot(ON_RESERVOIR);
        let lake = spot.waterbody.as_ref().unwrap();
        assert!(lake.inside);
        assert_eq!(lake.name.as_deref(), Some("New Croton Reservoir"));
        let path = spot.stream.as_ref().unwrap();
        assert_eq!(
            (path.name.as_deref(), path.basis),
            (Some("Croton River"), SnapBasis::WaterbodyPath)
        );

        let gauge = hydro.relation(&spot, &croton_gauge());
        assert_eq!(gauge.relation, Relation::Downstream);
        assert!(gauge.along_river_m.is_some());

        // The reservoir's own site sits on the dam, outside the outline.
        let site = hydro.relation(&spot, &reservoir_site());
        assert_eq!(site.relation, Relation::SameWaterBody);
        assert!(matches!(
            site.reasons[..],
            [HydroReason::WaterbodyShore { distance_m, .. }] if distance_m < 50.0
        ));

        // A brook-mouth stream gauge inside the outline still measures the brook.
        let brook = hydro.relation(&spot, &purdy_brook_mouth());
        assert_eq!(brook.relation, Relation::Nearby);
        assert!(matches!(brook.reasons[0], HydroReason::Tributary { .. }));
    }

    #[test]
    fn islands_are_not_water() {
        let lakes = waterbodies("hydro-60-new-croton-reservoir-geom.json", |_| Vec::new());
        assert_eq!(lakes[0].rings.len(), 5);
        assert!(lakes[0].contains(ON_RESERVOIR));
        assert!(!lakes[0].contains(Point::new(41.2515, -73.7306)));
        assert!(!lakes[0].contains(Point::new(41.27, -73.78)));

        let donut = Waterbody {
            id: "donut".into(),
            gnis_id: None,
            name: None,
            kind: WaterbodyKind::Lake,
            area_km2: 1.0,
            rings: vec![rect(0.0, 0.0, 1.0, 1.0), rect(0.4, 0.4, 0.6, 0.6)],
        };
        assert!(donut.contains(Point::new(0.2, 0.5)));
        assert!(!donut.contains(Point::new(0.5, 0.5)));
        let d = donut.distance_m(Point::new(0.5, 0.5));
        assert!((d - 0.1 * 111_195.0).abs() < 50.0, "{d}");
    }

    /// Great Sacandaga Lake arrives as several polygons sharing a GNIS id; the gauge is on the
    /// dam, outside them. Attributes from the fixture, outlines synthetic.
    fn sacandaga() -> Vec<Waterbody> {
        let mut parts = waterbodies("hydro-60-great-sacandaga-point.json", |id| match id {
            "LS2ZK" => vec![rect(43.310, -73.945, 43.322, -73.9285)],
            "JOCEU" => vec![rect(43.300, -73.970, 43.330, -73.945)],
            _ => vec![rect(43.3150, -73.9276, 43.3152, -73.9273)],
        });
        for (id, area, ring) in [
            ("main.synthetic", 83.57, rect(43.10, -74.20, 43.30, -73.97)),
            ("north.synthetic", 8.93, rect(43.30, -74.20, 43.35, -74.10)),
        ] {
            parts.push(Waterbody {
                id: id.into(),
                gnis_id: Some(973_352),
                name: Some("Great Sacandaga Lake".into()),
                kind: WaterbodyKind::Lake,
                area_km2: area,
                rings: vec![ring],
            });
        }
        parts
    }

    const ON_SACANDAGA: Point = Point::new(43.20, -74.10);

    #[test]
    fn a_lake_in_several_parts_is_one_water_body() {
        let parts = sacandaga();
        let gauge = usgs(
            "USGS-01323500",
            UsgsSiteType::Lake,
            43.316_027_8,
            -73.927_083_3,
        );
        let hydro = Hydro::new(&[], &parts, HydroOptions::default());
        let spot = hydro.locate_spot(ON_SACANDAGA);
        let lake = spot.waterbody.as_ref().unwrap();
        assert_eq!(lake.key, WaterbodyKey::Gnis(973_352));
        assert!((lake.area_km2 - 100.8).abs() < 0.01);
        let hint = hydro.relation(&spot, &gauge);
        assert_eq!(hint.relation, Relation::SameWaterBody);
        assert!(matches!(
            hint.reasons[0],
            HydroReason::WaterbodyShore { .. }
        ));

        // A stream gauge on the same spot of shore is not on the lake.
        let stream = usgs(
            "stream",
            UsgsSiteType::Stream,
            gauge.latitude,
            gauge.longitude,
        );
        assert_eq!(hydro.relation(&spot, &stream).relation, Relation::Nearby);
    }

    #[test]
    fn named_streams_win_within_the_tolerance() {
        let line = |id: &str, name: Option<&str>, hs: u64, lat: f64| Flowline {
            id: id.into(),
            name: name.map(str::to_owned),
            kind: FlowlineKind::ChannelLine,
            stream_order: Some(2),
            level_path: hs,
            hydrosequence: hs,
            dn_hydrosequence: 0,
            up_hydrosequence: 0,
            length_km: None,
            paths: vec![vec![Point::new(lat, -74.0), Point::new(lat, -73.99)]],
        };
        // 0.0005 degrees of latitude is about 56 m.
        let lines = [
            line("U", None, 1, 41.0005),
            line("N", Some("Named Brook"), 2, 40.9990),
        ];
        let at = Point::new(41.0, -73.995);
        let hydro = Hydro::new(&lines, &[], HydroOptions::default());
        let snapped = hydro.snap(at).unwrap();
        assert_eq!(
            (snapped.flowline_id.as_str(), snapped.basis),
            ("N", SnapBasis::PreferredNamed)
        );
        let tight = HydroOptions {
            snap_tolerance_m: 80.0,
            ..HydroOptions::default()
        };
        let hydro = Hydro::new(&lines, &[], tight);
        let snapped = hydro.snap(at).unwrap();
        assert_eq!(
            (snapped.flowline_id.as_str(), snapped.basis),
            ("U", SnapBasis::Nearest)
        );
        assert!(hydro.snap(Point::new(41.01, -73.995)).is_none());
    }

    fn infer(
        at: &HydroPosition,
        site: Option<UsgsSiteType>,
        zone: Option<ZoneKind>,
    ) -> Option<WaterType> {
        infer_water_type(at, site, zone, HydroOptions::default()).map(|s| s.water_type)
    }

    #[test]
    fn the_croton_below_the_dam_is_a_river() {
        let lines = flowlines("hydro-50-croton-envelope.json", None);
        let at = Hydro::new(&lines, &[], HydroOptions::default()).locate_spot(BELOW_GAUGE);
        let s = infer_water_type(
            &at,
            Some(UsgsSiteType::Stream),
            None,
            HydroOptions::default(),
        )
        .unwrap();
        assert_eq!(s.water_type, WaterType::RiverStream);
        assert_eq!(
            s.reasons,
            [WaterTypeReason::Stream {
                name: Some("Croton River".into()),
                stream_order: Some(7),
            }]
        );
    }

    #[test]
    fn reservoirs_and_lakes_in_parts_are_lakes() {
        let lines = flowlines("hydro-50-croton-envelope.json", None);
        let lakes = waterbodies("hydro-60-new-croton-reservoir-geom.json", |_| Vec::new());
        let at = Hydro::new(&lines, &lakes, HydroOptions::default()).locate_spot(ON_RESERVOIR);
        let s = infer_water_type(&at, Some(UsgsSiteType::Lake), None, HydroOptions::default());
        let s = s.unwrap();
        assert_eq!(s.water_type, WaterType::LakeReservoir);
        assert_eq!(
            s.reasons,
            [WaterTypeReason::Waterbody {
                name: Some("New Croton Reservoir".into()),
                kind: WaterbodyKind::Lake,
                area_km2: 8.74,
            }]
        );

        let parts = sacandaga();
        let at = Hydro::new(&[], &parts, HydroOptions::default()).locate_spot(ON_SACANDAGA);
        assert_eq!(infer(&at, None, None), Some(WaterType::LakeReservoir));
    }

    /// The Battery's waterbodies: attributes from the fixture, outlines synthetic. "Upper Bay"
    /// south of the Battery, the unnamed River polygon over the Hudson, two tiny lakes inland.
    fn harbor() -> Vec<Waterbody> {
        waterbodies("hydro-60-battery-envelope.json", |id| match id {
            "J09NV" => vec![rect(40.63, -74.07, 40.70, -74.00)],
            "JGLZF" => vec![rect(40.70, -74.03, 40.80, -74.00)],
            "JFXO1" => vec![rect(40.7100, -73.9960, 40.7103, -73.9955)],
            _ => vec![rect(40.7200, -73.9960, 40.7201, -73.9958)],
        })
    }

    #[test]
    fn the_battery_is_tidal_river_or_bay() {
        let water = harbor();
        let hydro = Hydro::new(&[], &water, HydroOptions::default());
        let coastal = Some(ZoneKind::Coastal);

        let battery = hydro.locate_spot(Point::new(40.7003, -74.0142));
        let s = infer_water_type(&battery, None, coastal, HydroOptions::default()).unwrap();
        assert_eq!(s.water_type, WaterType::TidalRiverEstuary);
        assert_eq!(
            s.reasons,
            [
                WaterTypeReason::MarineZone {
                    kind: ZoneKind::Coastal
                },
                WaterTypeReason::Waterbody {
                    name: None,
                    kind: WaterbodyKind::River,
                    area_km2: 175.6,
                },
            ]
        );
        // Without a marine zone, an estuary site nearby still makes it tidal.
        let estuary = Some(UsgsSiteType::Estuary);
        assert_eq!(
            infer(&battery, estuary, None),
            Some(WaterType::TidalRiverEstuary)
        );
        // An inland river polygon with no tidal hint is a river.
        assert_eq!(infer(&battery, None, None), Some(WaterType::RiverStream));

        let bay = hydro.locate_spot(Point::new(40.69, -74.03));
        let s = infer_water_type(&bay, None, coastal, HydroOptions::default()).unwrap();
        assert_eq!(s.water_type, WaterType::BayInshore);
        assert_eq!(
            s.reasons.last(),
            Some(&WaterTypeReason::BayName {
                name: "Upper Bay".into()
            })
        );
    }

    #[test]
    fn small_lakes_are_ponds() {
        let water = harbor();
        let hydro = Hydro::new(&[], &water, HydroOptions::default());
        let at = hydro.locate_spot(Point::new(40.71015, -73.99575));
        assert_eq!(
            at.waterbody.as_ref().unwrap().key,
            WaterbodyKey::Feature("JFXO1".into())
        );
        assert_eq!(infer(&at, None, None), Some(WaterType::Pond));
    }

    #[test]
    fn marine_zones_and_sites_decide_off_the_mapped_water() {
        let none = HydroPosition::default();
        let cases = [
            (None, Some(ZoneKind::Offshore), Some(WaterType::Offshore)),
            (
                None,
                Some(ZoneKind::GreatLakesOpen),
                Some(WaterType::GreatLakes),
            ),
            (
                None,
                Some(ZoneKind::GreatLakesNearshore),
                Some(WaterType::GreatLakes),
            ),
            (None, Some(ZoneKind::Coastal), Some(WaterType::Nearshore)),
            (
                Some(UsgsSiteType::TidalStream),
                None,
                Some(WaterType::TidalRiverEstuary),
            ),
            (
                Some(UsgsSiteType::Lake),
                None,
                Some(WaterType::LakeReservoir),
            ),
            (
                Some(UsgsSiteType::Stream),
                None,
                Some(WaterType::RiverStream),
            ),
            (Some(UsgsSiteType::Other), None, None),
            (None, None, None),
        ];
        for (site, zone, expected) in cases {
            assert_eq!(infer(&none, site, zone), expected, "{site:?} {zone:?}");
        }
    }

    #[test]
    fn the_stored_choice_wins() {
        let suggestion = WaterTypeSuggestion {
            water_type: WaterType::TidalRiverEstuary,
            reasons: Vec::new(),
        };
        assert_eq!(
            choose_water_type(Some(WaterType::Surf), Some(&suggestion)),
            Some(WaterType::Surf)
        );
        assert_eq!(
            choose_water_type(None, Some(&suggestion)),
            Some(WaterType::TidalRiverEstuary)
        );
        assert_eq!(choose_water_type(None, None), None);
    }

    #[test]
    fn bay_names_are_whole_words() {
        assert!(bay_like("Upper Bay"));
        assert!(bay_like("Long Island Sound"));
        assert!(!bay_like("Bayonne Creek"));
    }
}
