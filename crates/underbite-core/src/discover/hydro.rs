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

use std::cell::OnceCell;
use std::collections::BTreeMap;
use std::collections::btree_map::Entry;

use serde::{Deserialize, Serialize};

use crate::model::{WaterType, ZoneKind};
use crate::provenance::{Relation, SourceId, StationKind, StationRef, UsgsSiteType};

/// Mean Earth radius (IUGG), metres.
const EARTH_RADIUS_M: f64 = 6_371_008.8;

/// Two places this close along one flowline measure the same water, metres.
pub const SAME_PLACE_M: f64 = 100.0;

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
            // Every edge once, the closing one first; the parity does not depend on the order.
            let Some(mut a) = ring.last() else {
                continue;
            };
            for b in ring {
                inside ^= crosses(*a, *b, p);
                a = b;
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

/// Extra distance, metres, before a bounding box rules a feature out: far more than the
/// rounding of the projected distances, so a skipped feature could never have been within reach.
const SLACK_M: f64 = 1.0;

/// The same for two boxes, degrees (about a centimetre).
const SLACK_DEG: f64 = 1e-7;

/// A latitude and longitude box, empty when it holds no point.
#[derive(Clone, Copy, Debug)]
struct Bounds {
    south: f64,
    west: f64,
    north: f64,
    east: f64,
}

impl Bounds {
    const EMPTY: Self = Self {
        south: f64::INFINITY,
        west: f64::INFINITY,
        north: f64::NEG_INFINITY,
        east: f64::NEG_INFINITY,
    };

    fn of<'p>(points: impl IntoIterator<Item = &'p Point>) -> Self {
        points.into_iter().fold(Self::EMPTY, |b, p| Self {
            south: b.south.min(p.latitude),
            west: b.west.min(p.longitude),
            north: b.north.max(p.latitude),
            east: b.east.max(p.longitude),
        })
    }

    fn union(self, o: Self) -> Self {
        Self {
            south: self.south.min(o.south),
            west: self.west.min(o.west),
            north: self.north.max(o.north),
            east: self.east.max(o.east),
        }
    }

    /// Whether the boxes overlap or come within `slack` degrees of each other.
    fn meets(&self, o: &Self, slack: f64) -> bool {
        self.south <= o.north + slack
            && o.south <= self.north + slack
            && self.west <= o.east + slack
            && o.west <= self.east + slack
    }

    /// A lower bound, metres, of the distance from `p` to any segment inside the box, in the
    /// projection [`Projection`] measures segments in (infinite for an empty box).
    fn distance_m(&self, p: Point) -> f64 {
        let gap = |lo: f64, hi: f64, x: f64| (lo - x).max(x - hi).max(0.0);
        let at = Projection::at(p);
        (gap(self.west, self.east, p.longitude) * at.cos_lat * at.m_per_deg)
            .hypot(gap(self.south, self.north, p.latitude) * at.m_per_deg)
    }
}

/// A waterbody's edges sorted into latitude bands, for point-in-polygon tests that only look at
/// the edges level with the point.
#[derive(Debug)]
struct Outline {
    bounds: Bounds,
    /// Band height, degrees of latitude.
    height: f64,
    bands: Vec<Vec<(Point, Point)>>,
}

impl Outline {
    fn new(w: &Waterbody) -> Self {
        let bounds = Bounds::of(w.rings.iter().flatten());
        let count = w.rings.iter().map(Vec::len).sum::<usize>();
        let span = bounds.north - bounds.south;
        let n = if span > 0.0 {
            (count / 4).clamp(1, 4096)
        } else {
            1
        };
        let mut outline = Self {
            bounds,
            height: span / n as f64,
            bands: vec![Vec::new(); n],
        };
        for (a, b) in w.rings.iter().flat_map(|r| edges(r)) {
            let (lo, hi) = (a.latitude.min(b.latitude), a.latitude.max(b.latitude));
            for band in outline.band(lo)..=outline.band(hi) {
                outline.bands[band].push((a, b));
            }
        }
        outline
    }

    /// The band of a latitude; monotone, so an edge spanning `lo..=hi` sits in every band a
    /// latitude between them falls in.
    fn band(&self, latitude: f64) -> usize {
        let i = if self.height > 0.0 {
            // Saturates below 0 and above the last band.
            ((latitude - self.bounds.south) / self.height) as usize
        } else {
            0
        };
        i.min(self.bands.len() - 1)
    }

    /// [`Waterbody::contains`] over the edges of the point's band only: an edge the test counts
    /// spans the point's latitude, so it is in that band, and the count's parity is unchanged.
    fn contains(&self, p: Point) -> bool {
        if !(self.bounds.south..=self.bounds.north).contains(&p.latitude) {
            return false;
        }
        self.bands[self.band(p.latitude)]
            .iter()
            .fold(false, |inside, &(a, b)| inside ^ crosses(a, b, p))
    }
}

/// The polygons of one [`WaterbodyKey`].
#[derive(Debug)]
struct Group {
    parts: Vec<usize>,
    bounds: Bounds,
    /// The flowlines through the group, found on first use.
    through: OnceCell<Vec<usize>>,
}

/// The hydrography around a spot, indexed for relation queries.
///
/// Each flowline and waterbody carries its bounding box, so a point only meets the features
/// within the snap or shore tolerance; waterbody edges are banded by latitude for
/// point-in-polygon tests, and the flowlines through a waterbody are found once.
#[derive(Debug)]
pub struct Hydro<'a> {
    flowlines: &'a [Flowline],
    waterbodies: &'a [Waterbody],
    options: HydroOptions,
    by_hydrosequence: BTreeMap<u64, usize>,
    groups: BTreeMap<WaterbodyKey, Group>,
    lengths_m: Vec<f64>,
    /// Source length over geometry length, per flowline.
    scales: Vec<f64>,
    /// Per flowline: its bounding box and the length of each segment in path order, metres.
    reaches: Vec<(Bounds, Vec<f64>)>,
    outlines: Vec<Outline>,
    /// Every feature scanned and every polygon test over all edges, to check the index against.
    #[cfg(test)]
    exhaustive: bool,
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
        let outlines: Vec<Outline> = waterbodies.iter().map(Outline::new).collect();
        let mut groups: BTreeMap<WaterbodyKey, Group> = BTreeMap::new();
        for (i, w) in waterbodies.iter().enumerate() {
            let key = w
                .gnis_id
                .map_or_else(|| WaterbodyKey::Feature(w.id.clone()), WaterbodyKey::Gnis);
            let group = groups.entry(key).or_insert_with(|| Group {
                parts: Vec::new(),
                bounds: Bounds::EMPTY,
                through: OnceCell::new(),
            });
            group.parts.push(i);
            group.bounds = group.bounds.union(outlines[i].bounds);
        }
        let reaches: Vec<(Bounds, Vec<f64>)> = flowlines
            .iter()
            .map(|f| {
                let segments = f
                    .paths
                    .iter()
                    .flat_map(|p| p.windows(2))
                    .map(|s| haversine_m(s[0], s[1]))
                    .collect();
                (Bounds::of(f.paths.iter().flatten()), segments)
            })
            .collect();
        let geometry_m: Vec<f64> = reaches.iter().map(|(_, s)| s.iter().sum()).collect();
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
            reaches,
            outlines,
            #[cfg(test)]
            exhaustive: false,
        }
    }

    /// Whether everything in the box is farther than `limit_m` from `p`.
    // `self` only switches the index off in tests.
    #[cfg_attr(not(test), expect(clippy::unused_self))]
    fn beyond(&self, bounds: &Bounds, p: Point, limit_m: f64) -> bool {
        #[cfg(test)]
        if self.exhaustive {
            return false;
        }
        bounds.distance_m(p) > limit_m + SLACK_M
    }

    /// Whether every point of one box is clearly outside the other: a point that far outside a
    /// polygon's box is outside the polygon, rounding of the edge crossings included.
    // `self` only switches the index off in tests.
    #[cfg_attr(not(test), expect(clippy::unused_self))]
    fn apart(&self, a: &Bounds, b: &Bounds) -> bool {
        #[cfg(test)]
        if self.exhaustive {
            return false;
        }
        !a.meets(b, SLACK_DEG)
    }

    /// Whether waterbody `i` contains `p`.
    fn part_contains(&self, i: usize, p: Point) -> bool {
        #[cfg(test)]
        if self.exhaustive {
            return self.waterbodies[i].contains(p);
        }
        self.outlines[i].contains(p)
    }

    /// [`Waterbody::distance_m`] with the banded point-in-polygon test.
    fn part_distance(&self, i: usize, p: Point) -> f64 {
        if self.part_contains(i, p) {
            return 0.0;
        }
        let at = Projection::at(p);
        self.waterbodies[i]
            .rings
            .iter()
            .flat_map(|r| edges(r))
            .map(|(a, b)| at.segment(a, b).0)
            .fold(f64::INFINITY, f64::min)
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
            if self.beyond(&self.reaches[i].0, p, self.options.snap_tolerance_m) {
                continue;
            }
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
        // No hit beyond the shore tolerance: only inside or within it matters here.
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
                let relation = if d.abs() <= SAME_PLACE_M {
                    Relation::SameWaterBody
                } else if d < 0.0 {
                    Relation::Upstream
                } else {
                    Relation::Downstream
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

    /// Relation hints for every station, keyed by source and station id. A key listed twice is
    /// related once, from its last entry.
    pub fn relations(
        &self,
        spot: Point,
        stations: &[StationRef],
    ) -> BTreeMap<StationKey, RelationHint> {
        let at = self.locate_spot(spot);
        let mut out = BTreeMap::new();
        for s in stations.iter().rev() {
            if let Entry::Vacant(e) = out.entry((s.source.clone(), s.id.clone())) {
                e.insert(self.relation(&at, s));
            }
        }
        out
    }

    fn locate(&self, p: Point, mode: Mode) -> HydroPosition {
        let snapped = self.snap(p);
        // A waterbody beyond the shore tolerance is never taken, nor does it outrank one within.
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
            // A gauge inside a wide river or a lake, away from every flowline, is on the main
            // flow path through it.
            (Some(w), Mode::StreamGauge) => snapped.or_else(|| self.waterbody_path(&w.key, p)),
            (None, _) => snapped,
        };
        HydroPosition { waterbody, stream }
    }

    /// Where `p` is against the group's polygons; `None` for a group whose box is beyond the
    /// shore tolerance, which neither holds the point nor measures it from the shore.
    fn group_hit(&self, key: &WaterbodyKey, p: Point) -> Option<WaterbodyHit> {
        let group = self.groups.get(key)?;
        if self.beyond(&group.bounds, p, self.options.shore_tolerance_m) {
            return None;
        }
        let parts = &group.parts;
        let (part, distance_m) = parts
            .iter()
            .map(|&i| (i, self.part_distance(i, p)))
            .min_by(|x, y| x.1.total_cmp(&y.1))?;
        let w = &self.waterbodies[part];
        Some(WaterbodyHit {
            key: key.clone(),
            name: parts.iter().find_map(|&i| self.waterbodies[i].name.clone()),
            kind: w.kind,
            area_km2: parts.iter().map(|&i| self.waterbodies[i].area_km2).sum(),
            inside: self.part_contains(part, p),
            distance_m,
        })
    }

    /// The flowlines with a vertex or a segment midpoint inside the group, in index order.
    fn through(&self, group: &Group) -> Vec<usize> {
        let inside = |q: Point| group.parts.iter().any(|&i| self.part_contains(i, q));
        (0..self.flowlines.len())
            .filter(|&i| {
                !self.apart(&self.reaches[i].0, &group.bounds)
                    && self.flowlines[i]
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
            .collect()
    }

    /// The highest-order flowline through the waterbody (named first), nearest the point.
    fn waterbody_path(&self, key: &WaterbodyKey, p: Point) -> Option<StreamPosition> {
        let group = self.groups.get(key)?;
        let through = group.through.get_or_init(|| self.through(group));
        let rank = |i: usize| {
            (
                self.flowlines[i].stream_order,
                self.flowlines[i].name.is_some(),
            )
        };
        let best = through.iter().map(|&i| rank(i)).max()?;
        through
            .iter()
            .copied()
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
        let at = Projection::at(p);
        let (mut offset_m, mut measure_m, mut run) = (f64::INFINITY, 0.0, 0.0);
        let segments = f.paths.iter().flat_map(|l| l.windows(2));
        for (s, &len) in segments.zip(&self.reaches[index].1) {
            let (d, t) = at.segment(s[0], s[1]);
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

/// Whether edge `ab` crosses the ray from `p` towards the east (the even-odd rule's test).
fn crosses(a: Point, b: Point, p: Point) -> bool {
    (a.latitude > p.latitude) != (b.latitude > p.latitude)
        && p.longitude
            < a.longitude
                + (p.latitude - a.latitude) * (b.longitude - a.longitude)
                    / (b.latitude - a.latitude)
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
    Projection::at(p).segment(a, b)
}

/// An equirectangular projection centred on a point, metres.
#[derive(Clone, Copy)]
struct Projection {
    p: Point,
    m_per_deg: f64,
    cos_lat: f64,
}

impl Projection {
    fn at(p: Point) -> Self {
        Self {
            p,
            m_per_deg: EARTH_RADIUS_M.to_radians(),
            cos_lat: p.latitude.to_radians().cos(),
        }
    }

    /// Distance from the centre to segment `ab` and the position along it.
    fn segment(&self, a: Point, b: Point) -> (f64, f64) {
        let xy = |q: Point| {
            (
                (q.longitude - self.p.longitude) * self.cos_lat * self.m_per_deg,
                (q.latitude - self.p.latitude) * self.m_per_deg,
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

    /// Layer 50 features on the network; `level_path` fills a field the query did not ask for.
    fn flowlines(name: &str, level_path: Option<u64>) -> Vec<Flowline> {
        let int = |a: &Value, k: &str| a[k].as_f64().map(|x| x as u64);
        fixture(name)["features"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|f| !f["attributes"]["hydrosequence"].is_null())
            .map(|f| {
                let a = &f["attributes"];
                Flowline {
                    id: text(a, "id3dhp").unwrap(),
                    name: text(a, "gnisidlabel"),
                    kind: FlowlineKind::from_label(a["featuretypelabel"].as_str().unwrap_or("")),
                    stream_order: int(a, "streamorder").map(|o| o as u8),
                    level_path: int(a, "levelpath").or(level_path).unwrap(),
                    hydrosequence: int(a, "hydrosequence").unwrap(),
                    dn_hydrosequence: int(a, "dnhydrosequence").unwrap_or(0),
                    up_hydrosequence: int(a, "uphydrosequence").unwrap_or(0),
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

    /// One straight reach of a river flowing south along -74.0, from 41.02 to 40.98.
    fn south_reach() -> Flowline {
        Flowline {
            id: "R".into(),
            name: Some("Main River".into()),
            kind: FlowlineKind::ChannelLine,
            stream_order: Some(5),
            level_path: 7,
            hydrosequence: 7,
            dn_hydrosequence: 0,
            up_hydrosequence: 0,
            length_km: None,
            paths: vec![vec![Point::new(41.02, -74.0), Point::new(40.98, -74.0)]],
        }
    }

    #[test]
    fn places_within_100_m_along_one_flowline_are_the_same_water() {
        let lines = [south_reach()];
        let hydro = Hydro::new(&lines, &[], HydroOptions::default());
        let spot = hydro.locate_spot(Point::new(41.0, -74.0));
        // 0.0008 degrees of latitude is about 89 m, 0.0012 about 133 m.
        let gauge = |id, lat| usgs(id, UsgsSiteType::Stream, lat, -74.0002);
        let near = hydro.relation(&spot, &gauge("near", 41.0008));
        assert_eq!(near.relation, Relation::SameWaterBody);
        let along = near.along_river_m.unwrap();
        assert!((along - 89.0).abs() < 1.0, "{along}");
        let above = hydro.relation(&spot, &gauge("above", 41.0012));
        assert_eq!(above.relation, Relation::Upstream);
        let below = hydro.relation(&spot, &gauge("below", 40.9988));
        assert_eq!(below.relation, Relation::Downstream);
        assert!(SAME_PLACE_M < above.along_river_m.unwrap());
    }

    #[test]
    fn a_gauge_inside_a_wide_river_takes_its_flow_path() {
        let lines = [south_reach()];
        // A river 0.02 degrees (about 1.7 km) wide around its flowline.
        let river = Waterbody {
            id: "W".into(),
            gnis_id: None,
            name: Some("Main River".into()),
            kind: WaterbodyKind::River,
            area_km2: 7.0,
            rings: vec![rect(40.98, -74.01, 41.02, -73.99)],
        };
        let hydro = Hydro::new(
            &lines,
            std::slice::from_ref(&river),
            HydroOptions::default(),
        );
        let spot = hydro.locate_spot(Point::new(41.01, -74.0));
        // About 670 m east of the flowline, inside the outline; another beyond it.
        let inside = usgs("in", UsgsSiteType::Stream, 41.0, -73.992);
        let outside = usgs("out", UsgsSiteType::Stream, 41.0, -73.985);
        let hint = hydro.relation(&spot, &inside);
        assert_eq!(hint.relation, Relation::Downstream);
        assert!(hint.reasons.contains(&HydroReason::Snapped {
            subject: Subject::Station,
            basis: SnapBasis::WaterbodyPath,
            name: Some("Main River".into()),
        }));
        let hint = hydro.relation(&spot, &outside);
        assert!(hint.reasons.contains(&HydroReason::NotOnMappedStream {
            subject: Subject::Station
        }));
        // Without the outline the gauge inside is off the network too.
        let bare = Hydro::new(&lines, &[], HydroOptions::default());
        let spot = bare.locate_spot(Point::new(41.01, -74.0));
        assert_eq!(bare.relation(&spot, &inside).relation, Relation::Nearby);
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

    fn raw(path: &str) -> String {
        let path = format!(
            "{}/../../tests/fixtures/raw/{path}",
            env!("CARGO_MANIFEST_DIR")
        );
        std::fs::read_to_string(path).unwrap()
    }

    fn station(source: SourceId, id: &str, kind: StationKind, lat: f64, lon: f64) -> StationRef {
        StationRef {
            source,
            id: id.into(),
            name: id.into(),
            kind,
            latitude: lat,
            longitude: lon,
            bin: None,
            reference_id: None,
        }
    }

    /// Every station in the recorded CO-OPS, NDBC, USGS and NWPS lists within `km` of `at`, each
    /// also as a twin of the other placement mode (stream gauge or not).
    fn listed_stations(at: Point, usgs_lists: &[&str], km: f64) -> Vec<StationRef> {
        let mut out = Vec::new();
        for (file, kind) in [
            ("tidepredictions", StationKind::TideHarmonic),
            ("waterlevels", StationKind::WaterLevel),
            ("currentpredictions", StationKind::CurrentHarmonic),
        ] {
            let v: Value = serde_json::from_str(&raw(&format!(
                "coops/coops-mdapi-stations-{file}-newyork-trimmed.json"
            )))
            .unwrap();
            for s in v["stations"].as_array().unwrap() {
                let (lat, lon) = (s["lat"].as_f64().unwrap(), s["lng"].as_f64().unwrap());
                let id = s["id"].as_str().unwrap();
                out.push(station(SourceId::COOPS, id, kind, lat, lon));
            }
        }
        for name in usgs_lists {
            let v: Value = serde_json::from_str(&raw(&format!("usgs/{name}"))).unwrap();
            for f in v["features"].as_array().unwrap() {
                let c = &f["geometry"]["coordinates"];
                let site = match f["properties"]["site_type_code"].as_str().unwrap() {
                    "ST" => UsgsSiteType::Stream,
                    "LK" => UsgsSiteType::Lake,
                    "ES" => UsgsSiteType::Estuary,
                    _ => UsgsSiteType::Other,
                };
                out.push(station(
                    SourceId::USGS,
                    f["id"].as_str().unwrap(),
                    StationKind::UsgsSite(site),
                    c[1].as_f64().unwrap(),
                    c[0].as_f64().unwrap(),
                ));
            }
        }
        let v: Value =
            serde_json::from_str(&raw("nwps/nwps-gauges-bbox-lower-hudson.json")).unwrap();
        for g in v["gauges"].as_array().unwrap() {
            out.push(station(
                SourceId::NWPS,
                g["lid"].as_str().unwrap(),
                StationKind::NwpsGauge,
                g["latitude"].as_f64().unwrap(),
                g["longitude"].as_f64().unwrap(),
            ));
        }
        let xml = raw("ndbc/ndbc-activestations.xml");
        for tag in xml.split("<station ").skip(1) {
            let attr = |k: &str| {
                let start = tag.find(&format!(" {k}=\"")).map(|i| i + k.len() + 3)?;
                let len = tag[start..].find('"')?;
                Some(tag[start..start + len].to_owned())
            };
            let id = tag.split('"').nth(1).unwrap().to_owned();
            let (Some(lat), Some(lon)) = (attr("lat"), attr("lon")) else {
                continue;
            };
            let kind = if attr("type").as_deref() == Some("buoy") {
                StationKind::Buoy
            } else {
                StationKind::Cman
            };
            out.push(station(
                SourceId::NDBC,
                &id,
                kind,
                lat.parse().unwrap(),
                lon.parse().unwrap(),
            ));
        }
        out.retain(|s| haversine_m(at, Point::new(s.latitude, s.longitude)) <= km * 1000.0);
        let twins: Vec<StationRef> = out
            .iter()
            .map(|s| {
                let kind = match s.kind {
                    StationKind::UsgsSite(UsgsSiteType::Stream) | StationKind::NwpsGauge => {
                        StationKind::UsgsSite(UsgsSiteType::Lake)
                    }
                    _ => StationKind::UsgsSite(UsgsSiteType::Stream),
                };
                let id = format!("{}+twin", s.id);
                station(s.source.clone(), &id, kind, s.latitude, s.longitude)
            })
            .collect();
        out.extend(twins);
        out
    }

    const CROTON_SPOT: Point = Point::new(41.2250, -73.8589);

    /// The features of several recorded answers, each id once, the first answer first.
    fn network(lines: &[&str], lakes: &[&str]) -> (Vec<Flowline>, Vec<Waterbody>) {
        let mut reaches: Vec<Flowline> = Vec::new();
        for name in lines {
            for f in flowlines(name, None) {
                if !reaches.iter().any(|g| g.id == f.id) {
                    reaches.push(f);
                }
            }
        }
        let mut bodies: Vec<Waterbody> = Vec::new();
        for name in lakes {
            for w in waterbodies(name, |_| Vec::new()) {
                if !bodies.iter().any(|v| v.id == w.id) {
                    bodies.push(w);
                }
            }
        }
        (reaches, bodies)
    }

    /// Scans every feature and tests every polygon edge.
    fn exhaustive<'a>(lines: &'a [Flowline], lakes: &'a [Waterbody]) -> Hydro<'a> {
        Hydro {
            exhaustive: true,
            ..Hydro::new(lines, lakes, HydroOptions::default())
        }
    }

    #[test]
    fn the_index_changes_no_relation_on_the_recorded_networks() {
        let croton_lists = [
            "usgs-monitoring-locations-bbox-croton-st.json",
            "usgs-monitoring-locations-bbox-croton-lk.json",
            "usgs-monitoring-locations-bbox-croton-es.json",
        ];
        let battery = Point::new(40.7033, -74.0170);
        let far = |ids: [&str; 5], layer| ids.map(|id| format!("hydro-{layer}-{id}-geom.json"));
        let far_ids = ["hunter-brook", "ktnn6", "wnyn6", "wptn6", "xrvn6"];
        let (lines_50, lakes_60) = (far(far_ids, 50), far(far_ids, 60));
        let mut second = (
            vec!["hydro-50-croton-geom.json"],
            vec!["hydro-60-croton-geom.json"],
        );
        second.0.extend(lines_50.iter().map(String::as_str));
        second.1.extend(lakes_60.iter().map(String::as_str));
        let cases = [
            (
                "croton, spot's network",
                CROTON_SPOT,
                network(
                    &["hydro-50-croton-geom.json"],
                    &["hydro-60-croton-geom.json"],
                ),
                listed_stations(CROTON_SPOT, &croton_lists, 50.0),
            ),
            (
                "croton, with the far gauges' networks",
                CROTON_SPOT,
                network(&second.0, &second.1),
                listed_stations(CROTON_SPOT, &croton_lists, 50.0),
            ),
            (
                "battery",
                battery,
                network(
                    &["hydro-50-battery-geom.json"],
                    &["hydro-60-battery-geom.json"],
                ),
                listed_stations(
                    battery,
                    &["usgs-monitoring-locations-bbox-manhattan-st.json"],
                    50.0,
                ),
            ),
        ];
        for (case, spot, (lines, lakes), stations) in &cases {
            let indexed = Hydro::new(lines, lakes, HydroOptions::default());
            let scan = exhaustive(lines, lakes);
            assert_eq!(
                indexed.locate_spot(*spot),
                scan.locate_spot(*spot),
                "{case}"
            );
            let fast = indexed.relations(*spot, stations);
            let slow = scan.relations(*spot, stations);
            assert_eq!(fast.len(), slow.len(), "{case}");
            for (key, hint) in &slow {
                assert_eq!(fast.get(key), Some(hint), "{case}: {key:?}");
            }
            // The networks place stations every way, so the comparison covers each path.
            let kinds: std::collections::BTreeSet<String> =
                fast.values().map(|h| format!("{:?}", h.relation)).collect();
            assert!(kinds.len() >= 3, "{case}: {kinds:?}");
        }
    }

    #[test]
    fn bay_names_are_whole_words() {
        assert!(bay_like("Upper Bay"));
        assert!(bay_like("Long Island Sound"));
        assert!(!bay_like("Bayonne Creek"));
    }
}
