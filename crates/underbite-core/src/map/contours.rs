//! Depth contours from a grid by marching squares.
//!
//! [`contours`] traces the lines of equal value of a [`Grid`] at each of a list of levels,
//! [`levels`] lists the levels of an interval between two values, [`simplify`] thins a line with
//! the Douglas–Peucker algorithm at a tolerance in metres, and [`label_anchors`] places one label
//! on each long line. All of them are pure and deterministic.
//!
//! Values are canonical, so for the DEM a level is a height in metres, positive up: depth
//! contours are negative levels.
//!
//! # The tracing rule
//!
//! A grid point is *above* a level when its value is at least the level, and *below* otherwise.
//! Each cell between four neighbouring grid points that all hold a value is classified by which
//! of its corners are above; a cell with a missing corner is skipped, so lines stop at missing
//! data. A line crosses a cell edge whose two ends are on opposite sides, at the point found by
//! linear interpolation between the two ends' values and coordinates. The axes need not be evenly
//! spaced: a crossing on a north–south edge lies between that edge's own two latitudes, so a grid
//! of Web Mercator rows is traced row by row.
//!
//! A *saddle* cell has two diagonally opposite corners above and the other two below. It is
//! resolved by the average of its four corners, which stands for its centre: when the average
//! is above the level the two above corners are joined through the cell and the lines cut off the
//! two below corners; otherwise the above corners are cut off and the below ones joined.
//!
//! Lines run with the higher values on their left, so a ring around a peak runs counter-clockwise
//! and a ring around a hole clockwise (seen with east to the right and north up). A ring that
//! closes inside the grid repeats its first point at the end; a line that reaches the edge of the
//! grid or a missing value stays open.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::map::grid::Grid;
use crate::map::{GeoPoint, Polyline};

/// The most levels [`levels`] lists; a finer interval gives none.
pub const MAX_LEVELS: usize = 1_000;

/// The WGS 84 semi-major axis, metres.
const WGS84_A: f64 = 6_378_137.0;

/// The WGS 84 flattening.
const WGS84_F: f64 = 1.0 / 298.257_223_563;

/// The share of a line's length on each side of its label used to find the label's angle.
const LABEL_SPAN: f64 = 0.05;

/// Where to draw a line's value along it.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LabelAnchor {
    /// The index of the line in the slice given to [`label_anchors`].
    pub line: usize,
    /// The point halfway along the line.
    pub at: GeoPoint,
    /// The direction the label's baseline runs, degrees clockwise from true north, in `0..180`
    /// so that text along it reads from west to east on a north-up map (90 is level text).
    pub angle_deg: f64,
    /// The line's value, in canonical units.
    pub value: f64,
}

/// The levels between `min` and `max` (both included) that are whole multiples of `interval`,
/// ascending: `levels(-80.0, 0.0, 10.0)` is −80, −70, … 0 and `levels(-75.0, -5.0, 10.0)` is
/// −70 … −10.
///
/// Empty for a bound or an interval that is not a finite number, an interval that is not
/// positive, `min` above `max`, or more than [`MAX_LEVELS`] levels.
pub fn levels(min: f64, max: f64, interval: f64) -> Vec<f64> {
    if !(min.is_finite() && max.is_finite() && interval.is_finite()) || interval <= 0.0 || min > max
    {
        return Vec::new();
    }
    // A level that a division puts a hair outside the range (0.3 / 0.1 is 2.999…) still counts.
    let slack = 1e-9;
    let first = (min / interval - slack).ceil();
    let last = (max / interval + slack).floor();
    if !(first.is_finite() && last.is_finite()) || last < first || last - first >= MAX_LEVELS as f64
    {
        return Vec::new();
    }
    // In range: the count is below MAX_LEVELS and both ends are finite, whole numbers.
    let count = (last - first) as usize + 1;
    (0..count)
        // Adding 0.0 turns −0 into 0.
        .map(|k| (first + k as f64) * interval + 0.0)
        .collect()
}

/// The lines of equal value of `grid` at each level, by marching squares (see the module notes
/// for the rule).
///
/// The output holds the levels in ascending order (each once; a level that is not a finite
/// number is left out), and within a level the lines by their first point, south to north and
/// then west to east. A ring starts at its southernmost point (the westernmost of equals) and
/// ends with it again. Each line's `value` is its level.
pub fn contours(grid: &Grid, levels: &[f64]) -> Vec<Polyline> {
    let mut levels: Vec<f64> = levels.iter().copied().filter(|l| l.is_finite()).collect();
    levels.sort_by(f64::total_cmp);
    levels.dedup();
    levels
        .into_iter()
        .flat_map(|level| trace_level(grid, level))
        .collect()
}

/// `line` thinned by the Douglas–Peucker algorithm: every point removed lies within
/// `tolerance_m` metres of the simplified line.
///
/// Distances are measured on a local plane around the line's middle latitude, with longitudes
/// scaled by [`metres_per_degree`] there. The ends of an open line are kept. A closed ring (first
/// point repeated last) keeps at least three distinct points, so it stays a ring. A tolerance
/// that is not a positive number returns the line unchanged.
pub fn simplify(line: &Polyline, tolerance_m: f64) -> Polyline {
    let points = &line.points;
    let n = points.len();
    if !(tolerance_m > 0.0 && tolerance_m.is_finite()) || n <= 2 {
        return line.clone();
    }
    let plane = LocalPlane::around(points);
    let xy: Vec<(f64, f64)> = points.iter().map(|p| plane.project(*p)).collect();
    let mut keep = vec![false; n];
    keep[0] = true;
    keep[n - 1] = true;
    let mut spans = Vec::new();
    let closed = n >= 4 && points[0] == points[n - 1];
    if closed {
        // The point farthest from the start, then the point farthest from the two halves the
        // first one makes: with the start these are three corners the ring always keeps.
        // A ring has at least four points, so there is a point between its ends.
        let far = farthest(&xy, 0, n - 1, |p| dist(p, xy[0])).map_or(n - 1, |(k, _)| k);
        let third = farthest_from_spans(&xy, &[(0, far), (far, n - 1)]);
        let mut corners = vec![0, far, n - 1];
        corners.extend(third);
        corners.sort_unstable();
        corners.dedup();
        for pair in corners.windows(2) {
            keep[pair[1]] = true;
            spans.push((pair[0], pair[1]));
        }
    } else {
        spans.push((0, n - 1));
    }
    while let Some((a, b)) = spans.pop() {
        if let Some((k, d)) = farthest(&xy, a, b, |p| segment_dist(p, xy[a], xy[b]))
            && d > tolerance_m
        {
            keep[k] = true;
            spans.push((a, k));
            spans.push((k, b));
        }
    }
    Polyline {
        points: points
            .iter()
            .zip(&keep)
            .filter_map(|(p, &k)| k.then_some(*p))
            .collect(),
        value: line.value,
    }
}

/// One label for each line at least `min_length_m` metres long: halfway along it, turned to the
/// line's direction there (measured across a tenth of the line, so a jagged line gets its
/// general direction).
///
/// Lengths add up the segments, each measured at its own latitude. A negative minimum counts as
/// zero; one that is not a number gives no labels.
pub fn label_anchors(lines: &[Polyline], min_length_m: f64) -> Vec<LabelAnchor> {
    if min_length_m.is_nan() {
        return Vec::new();
    }
    let min_length_m = min_length_m.max(0.0);
    lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| {
            let along = cumulative_lengths(&line.points);
            let total = *along.last()?;
            if !(total > 0.0 && total.is_finite()) || total < min_length_m {
                return None;
            }
            let at = point_along(&line.points, &along, total / 2.0);
            let before = point_along(&line.points, &along, total * (0.5 - LABEL_SPAN));
            let after = point_along(&line.points, &along, total * (0.5 + LABEL_SPAN));
            let (lat_m, lon_m) = metres_per_degree(at.lat);
            let east = (after.lon - before.lon) * lon_m;
            let north = (after.lat - before.lat) * lat_m;
            let bearing = east.atan2(north).to_degrees().rem_euclid(360.0);
            let angle_deg = if bearing >= 180.0 {
                bearing - 180.0
            } else {
                bearing
            };
            Some(LabelAnchor {
                line: index,
                at,
                // Adding 0.0 turns −0 into 0.
                angle_deg: angle_deg + 0.0,
                value: line.value,
            })
        })
        .collect()
}

/// The length of a degree of latitude and of a degree of longitude at latitude `lat_deg`, in
/// metres, on the WGS 84 ellipsoid: the meridian radius of curvature and the parallel's radius
/// (the prime-vertical radius times the cosine of the latitude), times π / 180.
///
/// About 110 574 m and 111 320 m at the equator and 111 132 m and 78 847 m at 45°. A latitude
/// beyond ±90° counts as the pole.
pub fn metres_per_degree(lat_deg: f64) -> (f64, f64) {
    let phi = lat_deg.clamp(-90.0, 90.0).to_radians();
    let e2 = WGS84_F * (2.0 - WGS84_F);
    let w = (1.0 - e2 * phi.sin().powi(2)).sqrt();
    let meridian = WGS84_A * (1.0 - e2) / (w * w * w);
    let prime_vertical = WGS84_A / w;
    let per_radian = std::f64::consts::PI / 180.0;
    (
        meridian * per_radian,
        (prime_vertical * phi.cos()).max(0.0) * per_radian,
    )
}

/// Which side of the level a grid point is on.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Below,
    Above,
    Missing,
}

/// A cell edge, by the grid points at its ends: `South` runs east from point `(i, j)` to
/// `(i, j + 1)` and `West` runs north from `(i, j)` to `(i + 1, j)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Edge {
    South { i: usize, j: usize },
    West { i: usize, j: usize },
}

/// One of a cell's four edges.
#[derive(Clone, Copy)]
enum CellEdge {
    Bottom,
    Right,
    Top,
    Left,
}

impl CellEdge {
    /// This edge of the cell whose south-west corner is grid point `(i, j)`.
    fn of(self, i: usize, j: usize) -> Edge {
        match self {
            Self::Bottom => Edge::South { i, j },
            Self::Top => Edge::South { i: i + 1, j },
            Self::Left => Edge::West { i, j },
            Self::Right => Edge::West { i, j: j + 1 },
        }
    }
}

/// The directed segments through a cell, higher values on their left, by the cell's case: bit 0
/// for the south-west corner above the level, bit 1 the south-east, bit 2 the north-east and bit
/// 3 the north-west. `centre_above` resolves the two saddles (cases 5 and 10).
fn cell_segments(case: u8, centre_above: bool) -> &'static [(CellEdge, CellEdge)] {
    use CellEdge::{Bottom as B, Left as L, Right as R, Top as T};
    match (case, centre_above) {
        (1, _) => &[(B, L)],
        (2, _) => &[(R, B)],
        (3, _) => &[(R, L)],
        (4, _) => &[(T, R)],
        // South-west and north-east above: joined through the centre, or cut off.
        (5, true) => &[(B, R), (T, L)],
        (5, false) => &[(B, L), (T, R)],
        (6, _) => &[(T, B)],
        (7, _) => &[(T, L)],
        (8, _) => &[(L, T)],
        (9, _) => &[(B, T)],
        // South-east and north-west above: joined through the centre, or cut off.
        (10, true) => &[(L, B), (R, T)],
        (10, false) => &[(R, B), (L, T)],
        (11, _) => &[(R, T)],
        (12, _) => &[(L, R)],
        (13, _) => &[(B, R)],
        (14, _) => &[(L, B)],
        _ => &[],
    }
}

/// The lines of one level, in output order.
fn trace_level(grid: &Grid, level: f64) -> Vec<Polyline> {
    let (rows, cols) = (grid.lats.len(), grid.lons.len());
    if rows < 2 || cols < 2 || grid.values.len() != rows * cols {
        return Vec::new();
    }
    let value = |i: usize, j: usize| f64::from(grid.values[i * cols + j]);
    let sides: Vec<Side> = grid
        .values
        .iter()
        .map(|&v| {
            let v = f64::from(v);
            if !v.is_finite() {
                Side::Missing
            } else if v >= level {
                Side::Above
            } else {
                Side::Below
            }
        })
        .collect();
    let side = |i: usize, j: usize| sides[i * cols + j];

    let mut segments: Vec<(Edge, Edge)> = Vec::new();
    for i in 0..rows - 1 {
        for j in 0..cols - 1 {
            let corners = [
                side(i, j),
                side(i, j + 1),
                side(i + 1, j + 1),
                side(i + 1, j),
            ];
            if corners.contains(&Side::Missing) {
                continue;
            }
            let case = corners
                .iter()
                .enumerate()
                .fold(0u8, |c, (bit, &s)| c | (u8::from(s == Side::Above) << bit));
            let centre_above = matches!(case, 5 | 10) && {
                let sum = value(i, j) + value(i, j + 1) + value(i + 1, j + 1) + value(i + 1, j);
                sum / 4.0 >= level
            };
            for &(from, to) in cell_segments(case, centre_above) {
                segments.push((from.of(i, j), to.of(i, j)));
            }
        }
    }

    let crossing = |edge: Edge| -> GeoPoint {
        let ((ia, ja), (ib, jb)) = match edge {
            Edge::South { i, j } => ((i, j), (i, j + 1)),
            Edge::West { i, j } => ((i, j), (i + 1, j)),
        };
        let (va, vb) = (value(ia, ja), value(ib, jb));
        // The ends are on opposite sides, so their values differ; 0.5 only guards the division.
        let span = vb - va;
        let t = if span.abs() > 0.0 {
            ((level - va) / span).clamp(0.0, 1.0)
        } else {
            0.5
        };
        let lerp = |a: f64, b: f64| a + t * (b - a);
        match edge {
            Edge::South { i, .. } => GeoPoint {
                lat: grid.lats[i],
                lon: lerp(grid.lons[ja], grid.lons[jb]),
            },
            Edge::West { j, .. } => GeoPoint {
                lat: lerp(grid.lats[ia], grid.lats[ib]),
                lon: grid.lons[j],
            },
        }
    };

    let mut lines: Vec<Polyline> = join(&segments)
        .into_iter()
        .filter_map(|(edges, closed)| {
            let points = edges.into_iter().map(crossing).collect();
            finish(points, closed).map(|points| Polyline {
                points,
                value: level,
            })
        })
        .collect();
    lines.sort_by(|a, b| {
        let first = |l: &Polyline| l.points.first().copied();
        match (first(a), first(b)) {
            (Some(p), Some(q)) => p
                .lat
                .total_cmp(&q.lat)
                .then(p.lon.total_cmp(&q.lon))
                .then(a.points.len().cmp(&b.points.len())),
            _ => Ordering::Equal,
        }
    });
    lines
}

/// Joins directed segments end to start into chains of edges: open chains first, from the
/// segments that nothing leads into, then the rings that remain (whose last edge repeats the
/// first). Each edge starts at most one segment and ends at most one, because the segments keep
/// the higher values on their left in every cell.
fn join(segments: &[(Edge, Edge)]) -> Vec<(Vec<Edge>, bool)> {
    let by_start: BTreeMap<Edge, usize> = segments
        .iter()
        .enumerate()
        .map(|(k, &(from, _))| (from, k))
        .collect();
    let ends: BTreeSet<Edge> = segments.iter().map(|&(_, to)| to).collect();
    let mut used = vec![false; segments.len()];
    let mut chains = Vec::new();
    let follow = |start: usize, used: &mut [bool]| {
        let mut edges = vec![segments[start].0];
        let mut k = start;
        loop {
            used[k] = true;
            let to = segments[k].1;
            edges.push(to);
            match by_start.get(&to) {
                Some(&next) if !used[next] => k = next,
                _ => break,
            }
        }
        edges
    };
    for (k, &(from, _)) in segments.iter().enumerate() {
        if !used[k] && !ends.contains(&from) {
            chains.push((follow(k, &mut used), false));
        }
    }
    for k in 0..segments.len() {
        if !used[k] {
            let edges = follow(k, &mut used);
            let closed = edges.first() == edges.last();
            chains.push((edges, closed));
        }
    }
    chains
}

/// A chain's points without repeats (a line through a grid point that lies exactly on the level
/// crosses two edges there); a ring starting at its southernmost point and closed. `None` for a
/// line of fewer than two points or a ring of fewer than three.
fn finish(mut points: Vec<GeoPoint>, closed: bool) -> Option<Vec<GeoPoint>> {
    points.dedup();
    if !closed {
        return (points.len() >= 2).then_some(points);
    }
    while points.len() > 1 && points.first() == points.last() {
        points.pop();
    }
    if points.len() < 3 {
        return None;
    }
    let start = points
        .iter()
        .enumerate()
        .min_by(|(_, p), (_, q)| p.lat.total_cmp(&q.lat).then(p.lon.total_cmp(&q.lon)))
        .map_or(0, |(k, _)| k);
    points.rotate_left(start);
    points.push(points[0]);
    Some(points)
}

/// An equirectangular plane in metres around a set of points.
struct LocalPlane {
    lat0: f64,
    lon0: f64,
    lat_m: f64,
    lon_m: f64,
}

impl LocalPlane {
    /// The plane through the middle of `points`' latitudes, scaled there.
    fn around(points: &[GeoPoint]) -> Self {
        let (south, north) = points
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(s, n), p| {
                (s.min(p.lat), n.max(p.lat))
            });
        let lat0 = if south <= north {
            f64::midpoint(south, north)
        } else {
            0.0
        };
        let lon0 = points.first().map_or(0.0, |p| p.lon);
        let (lat_m, lon_m) = metres_per_degree(lat0);
        Self {
            lat0,
            lon0,
            lat_m,
            lon_m,
        }
    }

    /// A point's east and north offsets from the plane's origin, metres.
    fn project(&self, p: GeoPoint) -> (f64, f64) {
        (
            (p.lon - self.lon0) * self.lon_m,
            (p.lat - self.lat0) * self.lat_m,
        )
    }
}

fn dist(p: (f64, f64), q: (f64, f64)) -> f64 {
    (p.0 - q.0).hypot(p.1 - q.1)
}

/// The distance from `p` to the segment from `a` to `b`.
fn segment_dist(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    if len2 <= 0.0 {
        return dist(p, a);
    }
    let t = (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0);
    dist(p, (a.0 + t * dx, a.1 + t * dy))
}

/// The point strictly between indices `a` and `b` that is farthest by `measure`, with its
/// distance; the first of equals. `None` when there is no point between them.
fn farthest(
    xy: &[(f64, f64)],
    a: usize,
    b: usize,
    measure: impl Fn((f64, f64)) -> f64,
) -> Option<(usize, f64)> {
    (a + 1..b)
        .map(|k| (k, measure(xy[k])))
        .fold(None, |best, (k, d)| match best {
            Some((_, bd)) if bd >= d => best,
            _ => Some((k, d)),
        })
}

/// The point farthest from the segment of the span it lies in, over several spans.
fn farthest_from_spans(xy: &[(f64, f64)], spans: &[(usize, usize)]) -> Option<usize> {
    spans
        .iter()
        .filter_map(|&(a, b)| farthest(xy, a, b, |p| segment_dist(p, xy[a], xy[b])))
        .fold(None, |best: Option<(usize, f64)>, (k, d)| match best {
            Some((_, bd)) if bd >= d => best,
            _ => Some((k, d)),
        })
        .map(|(k, _)| k)
}

/// The distance along `points` to each point, metres, each segment measured at its middle
/// latitude.
fn cumulative_lengths(points: &[GeoPoint]) -> Vec<f64> {
    let mut total = 0.0;
    let mut along = Vec::with_capacity(points.len());
    if let Some(first) = points.first() {
        along.push(0.0);
        let mut prev = *first;
        for &p in &points[1..] {
            total += segment_length_m(prev, p);
            along.push(total);
            prev = p;
        }
    }
    along
}

fn segment_length_m(a: GeoPoint, b: GeoPoint) -> f64 {
    let (lat_m, lon_m) = metres_per_degree(f64::midpoint(a.lat, b.lat));
    ((b.lon - a.lon) * lon_m).hypot((b.lat - a.lat) * lat_m)
}

/// The point `distance` metres along `points`, given their [`cumulative_lengths`]; clamped to
/// the line's ends.
fn point_along(points: &[GeoPoint], along: &[f64], distance: f64) -> GeoPoint {
    let k = along.partition_point(|&d| d < distance);
    match (k.checked_sub(1), points.get(k)) {
        (Some(prev), Some(&b)) => {
            let a = points[prev];
            let span = along[k] - along[prev];
            let t = if span > 0.0 {
                ((distance - along[prev]) / span).clamp(0.0, 1.0)
            } else {
                0.0
            };
            GeoPoint {
                lat: a.lat + t * (b.lat - a.lat),
                lon: a.lon + t * (b.lon - a.lon),
            }
        }
        (None, Some(&p)) => p,
        (_, None) => points
            .last()
            .copied()
            .unwrap_or(GeoPoint { lat: 0.0, lon: 0.0 }),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)]

    use std::fmt::Write as _;

    use super::*;
    use crate::map::{GridHost, GridSource};
    use crate::time::UnixTime;

    fn source() -> GridSource {
        GridSource {
            node: GridHost::Pfeg,
            dataset: "DEM_global_mosaic".to_owned(),
            variables: vec!["elevation".to_owned()],
            altitude_axis: false,
            native_step_deg: 0.005,
        }
    }

    fn grid(lats: Vec<f64>, lons: Vec<f64>, values: Vec<f32>) -> Grid {
        Grid::new(source(), UnixTime(1_794_000_000), lats, lons, values, "m").unwrap()
    }

    /// A grid over evenly spaced axes with `f(lat, lon)` at each point.
    fn sampled(
        south: f64,
        west: f64,
        step: (f64, f64),
        size: (usize, usize),
        f: impl Fn(f64, f64) -> f64,
    ) -> Grid {
        let lats: Vec<f64> = (0..size.0).map(|i| south + i as f64 * step.0).collect();
        let lons: Vec<f64> = (0..size.1).map(|j| west + j as f64 * step.1).collect();
        let values = lats
            .iter()
            .flat_map(|&lat| lons.iter().map(move |&lon| (lat, lon)))
            .map(|(lat, lon)| f(lat, lon) as f32)
            .collect();
        grid(lats, lons, values)
    }

    fn pt(lat: f64, lon: f64) -> GeoPoint {
        GeoPoint { lat, lon }
    }

    fn assert_near(points: &[GeoPoint], expected: &[GeoPoint]) {
        assert_eq!(points.len(), expected.len(), "{points:?}");
        for (p, q) in points.iter().zip(expected) {
            assert!(
                (p.lat - q.lat).abs() < 1e-9 && (p.lon - q.lon).abs() < 1e-9,
                "{points:?} vs {expected:?}"
            );
        }
    }

    fn is_closed(line: &Polyline) -> bool {
        line.points.len() >= 4 && line.points.first() == line.points.last()
    }

    /// Twice the signed area in degrees², positive counter-clockwise.
    fn signed_area(points: &[GeoPoint]) -> f64 {
        points
            .windows(2)
            .map(|w| w[0].lon * w[1].lat - w[1].lon * w[0].lat)
            .sum()
    }

    /// A cone around the Battery: `peak` at the centre, falling `slope` metres per metre of
    /// distance, sampled every 0.002° (about 220 m by 170 m).
    const CENTRE: GeoPoint = GeoPoint {
        lat: 40.70,
        lon: -74.02,
    };

    fn distance_m(a: GeoPoint, b: GeoPoint) -> f64 {
        let (lat_m, lon_m) = metres_per_degree(CENTRE.lat);
        ((a.lon - b.lon) * lon_m).hypot((a.lat - b.lat) * lat_m)
    }

    fn cone(peak: f64, slope: f64) -> Grid {
        sampled(40.65, -74.07, (0.002, 0.002), (51, 51), |lat, lon| {
            peak - slope * distance_m(pt(lat, lon), CENTRE)
        })
    }

    #[test]
    fn levels_are_whole_multiples_of_the_interval() {
        assert_eq!(
            levels(-80.0, 0.0, 10.0),
            [-80.0, -70.0, -60.0, -50.0, -40.0, -30.0, -20.0, -10.0, 0.0]
        );
        assert_eq!(
            levels(-75.0, -5.0, 10.0),
            [-70.0, -60.0, -50.0, -40.0, -30.0, -20.0, -10.0]
        );
        assert_eq!(levels(-4.0, 4.0, 5.0), [0.0]);
        assert!(levels(-4.0, 4.0, 5.0)[0].is_sign_positive());
        assert_eq!(levels(1.0, 4.0, 5.0), Vec::<f64>::new());
        assert_eq!(levels(0.0, 0.3, 0.1).len(), 4);
        assert_eq!(levels(-6.096, -3.048, 1.524).len(), 3);
    }

    #[test]
    fn levels_refuse_bad_input() {
        assert_eq!(levels(0.0, -10.0, 1.0), Vec::<f64>::new());
        assert_eq!(levels(-10.0, 0.0, 0.0), Vec::<f64>::new());
        assert_eq!(levels(-10.0, 0.0, -1.0), Vec::<f64>::new());
        assert_eq!(levels(f64::NAN, 0.0, 1.0), Vec::<f64>::new());
        assert_eq!(levels(-10.0, f64::INFINITY, 1.0), Vec::<f64>::new());
        assert_eq!(levels(-10.0, 0.0, f64::NAN), Vec::<f64>::new());
        assert_eq!(levels(-1e300, 1e300, 1.0), Vec::<f64>::new());
        assert_eq!(levels(-1e300, 1e300, 1e-300), Vec::<f64>::new());
        assert_eq!(levels(0.0, 999.0, 1.0).len(), MAX_LEVELS);
        assert_eq!(levels(0.0, 1000.0, 1.0), Vec::<f64>::new());
    }

    #[test]
    fn metres_per_degree_match_the_wgs84_table() {
        // Lengths of a degree on WGS 84, kilometres, rounded to the metre.
        let table = [
            (0.0, 110.574, 111.320),
            (15.0, 110.649, 107.551),
            (30.0, 110.852, 96.486),
            (45.0, 111.132, 78.847),
            (60.0, 111.412, 55.800),
            (75.0, 111.618, 28.902),
            (90.0, 111.694, 0.0),
        ];
        for (lat, lat_km, lon_km) in table {
            let (lat_m, lon_m) = metres_per_degree(lat);
            assert!((lat_m - lat_km * 1000.0).abs() < 1.0, "{lat}: {lat_m}");
            assert!((lon_m - lon_km * 1000.0).abs() < 1.0, "{lat}: {lon_m}");
            assert_eq!(metres_per_degree(-lat), (lat_m, lon_m));
        }
        assert_eq!(metres_per_degree(120.0), metres_per_degree(90.0));
        // At the Battery, against the series 111 132.92 − 559.82 cos 2φ + 1.175 cos 4φ and
        // 111 412.84 cos φ − 93.5 cos 3φ: a degree of longitude is about three quarters of one of
        // latitude.
        let (lat_m, lon_m) = metres_per_degree(40.7);
        assert!((lat_m - 111_048.1).abs() < 1.0, "{lat_m}");
        assert!((lon_m - 84_516.2).abs() < 1.0, "{lon_m}");
    }

    #[test]
    fn a_cone_gives_one_ring_per_level_at_its_radius() {
        let g = cone(0.0, 0.01);
        let half_cell_m = 0.5 * distance_m(pt(40.70, -74.02), pt(40.702, -74.018));
        let lines = contours(&g, &levels(-40.0, -10.0, 10.0));
        assert_eq!(lines.len(), 4);
        for (line, level) in lines.iter().zip([-40.0, -30.0, -20.0, -10.0]) {
            assert_eq!(line.value, level);
            assert!(is_closed(line), "{level}");
            let radius = -level / 0.01;
            for p in &line.points {
                let r = distance_m(*p, CENTRE);
                assert!(
                    (r - radius).abs() <= half_cell_m,
                    "{level}: {r} vs {radius}"
                );
            }
            // Higher values (toward the peak) on the left: counter-clockwise.
            assert!(signed_area(&line.points) > 0.0);
            // A ring starts at its southernmost point.
            let south = line
                .points
                .iter()
                .map(|p| p.lat)
                .fold(f64::INFINITY, f64::min);
            assert_eq!(line.points[0].lat, south);
        }
    }

    #[test]
    fn a_hole_gives_clockwise_rings() {
        let g = cone(-50.0, -0.01);
        let lines = contours(&g, &[-30.0]);
        assert_eq!(lines.len(), 1);
        assert!(is_closed(&lines[0]));
        assert!(signed_area(&lines[0].points) < 0.0);
    }

    #[test]
    fn a_ring_cut_by_the_grid_edge_stays_open() {
        let g = cone(0.0, 0.01);
        // A radius of 4.5 km reaches past the grid's east and west edges, 4.2 km from the
        // centre, but not its north and south edges, 5.5 km away.
        let lines = contours(&g, &[-45.0]);
        assert_eq!(lines.len(), 2);
        for line in &lines {
            assert!(!is_closed(line));
            for end in [line.points[0], *line.points.last().unwrap()] {
                let on_edge = end.lat == g.lats[0]
                    || end.lat == *g.lats.last().unwrap()
                    || end.lon == g.lons[0]
                    || end.lon == *g.lons.last().unwrap();
                assert!(on_edge, "{end:?}");
            }
        }
    }

    #[test]
    fn a_plane_gives_straight_lines_across_the_grid() {
        // Height rises 1 m per 0.001° east and 0.5 m per 0.001° north.
        let g = sampled(40.6, -74.1, (0.004, 0.004), (20, 30), |lat, lon| {
            (lon + 74.1) * 1000.0 + (lat - 40.6) * 500.0 - 60.0
        });
        let lines = contours(&g, &levels(-40.0, 40.0, 20.0));
        assert_eq!(lines.len(), 5);
        for line in &lines {
            assert!(!is_closed(line));
            for p in &line.points {
                let h = (p.lon + 74.1) * 1000.0 + (p.lat - 40.6) * 500.0 - 60.0;
                // The grid holds f32 values, so the plane is exact to about 1e-5 m.
                assert!((h - line.value).abs() < 1e-4, "{h} vs {}", line.value);
            }
            let straight = simplify(line, 0.5);
            assert_eq!(straight.points.len(), 2);
            assert_eq!(straight.points[0], line.points[0]);
            assert_eq!(straight.points[1], *line.points.last().unwrap());
        }
        // Higher values on the left: with the ground rising to the east and north, the lines
        // run south-south-east.
        let first = &lines[0];
        assert!(first.points.last().unwrap().lat < first.points[0].lat);
    }

    #[test]
    fn crossings_use_each_rows_own_latitude() {
        // Web Mercator rows: the steps shrink toward the equator. The value is the row number,
        // so a level between two rows lies between those rows' own latitudes.
        let lats: Vec<f64> = (0..12)
            .map(|i| {
                let y = 4_960_000.0 + f64::from(i) * 9_000.0;
                (y / 6_378_137.0f64).sinh().atan().to_degrees()
            })
            .collect();
        let lons: Vec<f64> = (0..5).map(|j| -74.05 + f64::from(j) * 0.01).collect();
        let values: Vec<f32> = (0..lats.len())
            .flat_map(|i| std::iter::repeat_n(i as f32, lons.len()))
            .collect();
        let g = grid(lats.clone(), lons, values);
        let lines = contours(&g, &[4.25, 9.5]);
        assert_eq!(lines.len(), 2);
        let expected = [
            lats[4] + 0.25 * (lats[5] - lats[4]),
            lats[9] + 0.5 * (lats[10] - lats[9]),
        ];
        for (line, lat) in lines.iter().zip(expected) {
            assert_eq!(line.points.len(), 5);
            for p in &line.points {
                assert!((p.lat - lat).abs() < 1e-12, "{} vs {lat}", p.lat);
            }
        }
        // Evenly spaced rows would put the second line elsewhere.
        let even = lats[0] + 9.5 * (lats[11] - lats[0]) / 11.0;
        assert!((expected[1] - even).abs() > 1e-4);
    }

    #[test]
    fn rows_given_north_first_trace_the_same_lines() {
        let g = cone(0.0, 0.01);
        let cols = g.lons.len();
        let north_first: Vec<f32> = g.values.chunks(cols).rev().flatten().copied().collect();
        let mut lats = g.lats.clone();
        lats.reverse();
        let flipped = grid(lats, g.lons.clone(), north_first);
        let level = levels(-30.0, -10.0, 10.0);
        assert_eq!(contours(&g, &level), contours(&flipped, &level));
    }

    #[test]
    fn saddles_follow_the_centre_average() {
        let lats = vec![40.0, 40.1];
        let lons = vec![-74.1, -74.0];
        // Corners south-west, south-east, north-west, north-east (rows south first).
        let joined = grid(lats.clone(), lons.clone(), vec![1.0, 0.0, 0.0, 1.0]);
        let lines = contours(&joined, &[0.4]);
        // The centre average 0.5 is above 0.4: the south-west and north-east corners are joined,
        // and the lines cut off the south-east and north-west corners.
        assert_eq!(lines.len(), 2);
        // From the south edge to the east edge, and from the north edge to the west edge.
        assert_near(&lines[0].points, &[pt(40.0, -74.04), pt(40.04, -74.0)]);
        assert_near(&lines[1].points, &[pt(40.1, -74.06), pt(40.06, -74.1)]);

        // Lower the north-west corner: the average 0.375 is below 0.5, so the corners above
        // are each cut off.
        let apart = grid(lats, lons, vec![1.0, 0.0, -0.5, 1.0]);
        let lines = contours(&apart, &[0.5]);
        assert_eq!(lines.len(), 2);
        // Around the south-west corner from the south edge to the west edge, and around the
        // north-east corner from the north edge to the east edge.
        assert_near(
            &lines[0].points,
            &[pt(40.0, -74.05), pt(40.0 + 0.1 / 3.0, -74.1)],
        );
        assert_near(
            &lines[1].points,
            &[pt(40.1, -74.1 + 0.2 / 3.0), pt(40.05, -74.0)],
        );
    }

    #[test]
    fn every_case_keeps_the_higher_values_on_the_left() {
        // Corners in case-bit order: south-west, south-east, north-east, north-west.
        let corners = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)];
        let mid = |e: CellEdge| match e {
            CellEdge::Bottom => (0.5, 0.0),
            CellEdge::Right => (1.0, 0.5),
            CellEdge::Top => (0.5, 1.0),
            CellEdge::Left => (0.0, 0.5),
        };
        for case in 1u8..15 {
            for centre_above in [false, true] {
                let segments = cell_segments(case, centre_above);
                assert_eq!(segments.len(), if matches!(case, 5 | 10) { 2 } else { 1 });
                for &(from, to) in segments {
                    let (a, b) = (mid(from), mid(to));
                    let (mut left, mut right) = (Vec::new(), Vec::new());
                    for (bit, &(x, y)) in corners.iter().enumerate() {
                        let above = case & (1 << bit) != 0;
                        let cross = (b.0 - a.0) * (y - a.1) - (b.1 - a.1) * (x - a.0);
                        (if cross > 0.0 { &mut left } else { &mut right }).push(above);
                    }
                    // A segment that cuts one corner off: that corner is above exactly when it
                    // is on the left. One that halves the cell: above on the left, below on the
                    // right.
                    let ok = match (left.len(), right.len()) {
                        (1, _) => left[0],
                        (_, 1) => !right[0],
                        _ => left.iter().all(|&a| a) && right.iter().all(|&a| !a),
                    };
                    assert!(ok, "case {case}, centre above {centre_above}");
                }
            }
        }
    }

    #[test]
    fn missing_values_break_rings_into_open_lines() {
        let mut g = cone(0.0, 0.01);
        let cols = g.lons.len();
        // A missing point on the 1 km ring, due east of the centre.
        let j = 25 + (1000.0 / distance_m(pt(40.70, -74.02), pt(40.70, -74.018))).round() as usize;
        g.values[25 * cols + j] = f32::NAN;
        let lines = contours(&g, &[-10.0]);
        assert_eq!(lines.len(), 1);
        let line = &lines[0];
        assert!(!is_closed(line));
        // No vertex lies inside the four cells around the missing point.
        for p in &line.points {
            let inside = p.lat > g.lats[24]
                && p.lat < g.lats[26]
                && p.lon > g.lons[j - 1]
                && p.lon < g.lons[j + 1];
            assert!(!inside, "{p:?}");
        }
        // An all-missing grid has no lines.
        let empty = grid(vec![40.0, 40.1], vec![-74.0, -73.9], vec![f32::NAN; 4]);
        assert_eq!(contours(&empty, &[0.0]), Vec::<Polyline>::new());
    }

    #[test]
    fn a_bump_inside_the_grid_closes() {
        let mut values = vec![0.0f32; 25];
        values[12] = 10.0;
        let g = grid(
            (0..5).map(|i| 40.0 + f64::from(i) * 0.01).collect(),
            (0..5).map(|j| -74.0 + f64::from(j) * 0.01).collect(),
            values,
        );
        let lines = contours(&g, &[5.0]);
        assert_eq!(lines.len(), 1);
        let ring = &lines[0].points;
        assert_eq!(ring.len(), 5);
        assert_eq!(ring[0], ring[4]);
        // From the southernmost crossing, counter-clockwise round the bump.
        assert_near(
            ring,
            &[
                pt(40.015, -73.98),
                pt(40.02, -73.975),
                pt(40.025, -73.98),
                pt(40.02, -73.985),
                pt(40.015, -73.98),
            ],
        );
        // A point exactly on the level counts as above: a lone grid point at the level makes
        // no ring of its own.
        assert_eq!(contours(&g, &[10.0]), Vec::<Polyline>::new());
    }

    #[test]
    fn degenerate_grids_and_levels_give_no_lines() {
        let row = grid(vec![40.0], vec![-74.0, -73.9], vec![0.0, 1.0]);
        assert_eq!(contours(&row, &[0.5]), Vec::<Polyline>::new());
        let g = cone(0.0, 0.01);
        assert_eq!(contours(&g, &[]), Vec::<Polyline>::new());
        assert_eq!(
            contours(&g, &[f64::NAN, f64::INFINITY]),
            Vec::<Polyline>::new()
        );
        // Out of order and repeated levels come out once each, ascending.
        let lines = contours(&g, &[-10.0, -20.0, -10.0]);
        let values: Vec<f64> = lines.iter().map(|l| l.value).collect();
        assert_eq!(values, [-20.0, -10.0]);
    }

    #[test]
    fn simplification_keeps_every_removed_point_within_tolerance() {
        let g = cone(0.0, 0.01);
        let ring = contours(&g, &[-20.0]).remove(0);
        for tolerance in [2.0, 20.0, 100.0] {
            let thin = simplify(&ring, tolerance);
            assert!(is_closed(&thin));
            assert!(thin.points.len() <= ring.points.len());
            if tolerance >= 20.0 {
                assert!(thin.points.len() < ring.points.len());
            }
            assert!(thin.points.iter().all(|p| ring.points.contains(p)));
            let plane = LocalPlane::around(&ring.points);
            let kept: Vec<(f64, f64)> = thin.points.iter().map(|p| plane.project(*p)).collect();
            for p in &ring.points {
                let d = kept
                    .windows(2)
                    .map(|w| segment_dist(plane.project(*p), w[0], w[1]))
                    .fold(f64::INFINITY, f64::min);
                assert!(d <= tolerance + 1e-6, "{tolerance}: {d}");
            }
        }
        // A tolerance that is not a positive number changes nothing.
        assert_eq!(simplify(&ring, 0.0), ring);
        assert_eq!(simplify(&ring, -1.0), ring);
        assert_eq!(simplify(&ring, f64::NAN), ring);
    }

    #[test]
    fn simplification_never_collapses_a_ring() {
        let g = cone(0.0, 0.01);
        let ring = contours(&g, &[-20.0]).remove(0);
        let thin = simplify(&ring, 1e9);
        assert_eq!(thin.points.len(), 4);
        assert!(is_closed(&thin));
        let distinct = &thin.points[..3];
        assert!(
            distinct[0] != distinct[1] && distinct[1] != distinct[2] && distinct[0] != distinct[2]
        );
        // An open line keeps its two ends.
        let open = Polyline {
            points: ring.points[..10].to_vec(),
            value: ring.value,
        };
        let thin = simplify(&open, 1e9);
        assert_eq!(thin.points, [open.points[0], open.points[9]]);
    }

    #[test]
    fn simplification_measures_longitude_at_the_lines_latitude() {
        // A bump of 0.001° east in a line running north: about 85 m at 40.7°N, 111 m at the
        // equator and 43 m at 67°N.
        let line = |lat: f64| Polyline {
            points: vec![pt(lat - 0.01, 0.0), pt(lat, 0.001), pt(lat + 0.01, 0.0)],
            value: 0.0,
        };
        assert_eq!(simplify(&line(40.7), 80.0).points.len(), 3);
        assert_eq!(simplify(&line(40.7), 90.0).points.len(), 2);
        assert_eq!(simplify(&line(0.0), 110.0).points.len(), 3);
        assert_eq!(simplify(&line(67.0), 45.0).points.len(), 2);
        // A bump north is measured in degrees of latitude everywhere.
        let north = Polyline {
            points: vec![pt(40.7, -0.01), pt(40.701, 0.0), pt(40.7, 0.01)],
            value: 0.0,
        };
        assert_eq!(simplify(&north, 110.0).points.len(), 3);
        assert_eq!(simplify(&north, 112.0).points.len(), 2);
    }

    #[test]
    fn labels_sit_halfway_along_long_lines() {
        // A line 0.02° due east at 40.7°N, about 1.69 km, and a short one.
        let east = Polyline {
            points: vec![pt(40.7, -74.02), pt(40.7, -74.01), pt(40.7, -74.0)],
            value: -10.0,
        };
        let short = Polyline {
            points: vec![pt(40.6, -74.0), pt(40.601, -74.0)],
            value: -20.0,
        };
        let south = Polyline {
            points: vec![pt(40.72, -74.0), pt(40.70, -74.0)],
            value: -30.0,
        };
        let anchors = label_anchors(&[east.clone(), short, south], 1000.0);
        assert_eq!(anchors.len(), 2);
        assert_eq!(anchors[0].line, 0);
        assert_eq!(anchors[0].value, -10.0);
        assert!((anchors[0].at.lon + 74.01).abs() < 1e-12);
        assert!((anchors[0].angle_deg - 90.0).abs() < 1e-9);
        // A line heading south reads upward: its baseline runs north.
        assert_eq!(anchors[1].line, 2);
        assert_eq!(anchors[1].angle_deg, 0.0);
        assert!((anchors[1].at.lat - 40.71).abs() < 1e-12);
        // A line heading west gets the same angle as one heading east.
        let west = Polyline {
            points: east.points.iter().rev().copied().collect(),
            value: -10.0,
        };
        let a = label_anchors(&[west], 0.0);
        assert!((a[0].angle_deg - 90.0).abs() < 1e-9);
        // A north-east line at 45° on the ground, not in degrees.
        let (lat_m, lon_m) = metres_per_degree(40.7);
        let diagonal = Polyline {
            points: vec![
                pt(40.7, -74.0),
                pt(40.7 + 0.01, -74.0 + 0.01 * lat_m / lon_m),
            ],
            value: 0.0,
        };
        let a = label_anchors(&[diagonal], 0.0);
        assert!((a[0].angle_deg - 45.0).abs() < 0.01, "{}", a[0].angle_deg);
        assert_eq!(label_anchors(&[east], f64::NAN), Vec::<LabelAnchor>::new());
    }

    #[test]
    fn rings_get_a_label_on_their_far_side() {
        let g = cone(0.0, 0.01);
        let rings = contours(&g, &[-20.0, -10.0]);
        let anchors = label_anchors(&rings, 5000.0);
        // The 1 km ring is about 6.3 km round, the 2 km ring 12.6 km.
        assert_eq!(anchors.len(), 2);
        for a in &anchors {
            let radius = -a.value / 0.01;
            assert!((distance_m(a.at, CENTRE) - radius).abs() < 120.0);
            // Rings start in the south, so halfway round is the north, where the line runs
            // east to west.
            assert!(a.at.lat > CENTRE.lat);
            assert!((a.angle_deg - 90.0).abs() < 10.0, "{}", a.angle_deg);
        }
    }

    /// A small random number generator, so the property below is the same on every run.
    struct SplitMix(u64);

    impl SplitMix {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^ (z >> 31)
        }

        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }

        fn unit(&mut self) -> f64 {
            (self.next() >> 11) as f64 / (1u64 << 53) as f64
        }
    }

    /// An ascending axis of `n` unevenly spaced points.
    fn random_axis(rng: &mut SplitMix, start: f64, n: usize) -> Vec<f64> {
        let mut x = start;
        (0..n)
            .map(|_| {
                let v = x;
                x += 0.001 + rng.unit() * 0.01;
                v
            })
            .collect()
    }

    /// Whether `p` lies on a grid edge whose two ends hold values on either side of `level`.
    fn on_bracketing_edge(g: &Grid, p: GeoPoint, level: f64) -> bool {
        let v = |i: usize, j: usize| f64::from(g.value(i, j).unwrap_or(f32::NAN));
        let brackets = |a: f64, b: f64| {
            a.is_finite() && b.is_finite() && a.min(b) <= level && level <= a.max(b)
        };
        let between = |x: f64, lo: f64, hi: f64| lo - 1e-12 <= x && x <= hi + 1e-12;
        let (rows, cols) = (g.lats.len(), g.lons.len());
        let on_south_edge = (0..rows)
            .flat_map(|i| (0..cols - 1).map(move |j| (i, j)))
            .any(|(i, j)| {
                p.lat == g.lats[i]
                    && between(p.lon, g.lons[j], g.lons[j + 1])
                    && brackets(v(i, j), v(i, j + 1))
            });
        let on_west_edge = (0..rows - 1)
            .flat_map(|i| (0..cols).map(move |j| (i, j)))
            .any(|(i, j)| {
                p.lon == g.lons[j]
                    && between(p.lat, g.lats[i], g.lats[i + 1])
                    && brackets(v(i, j), v(i + 1, j))
            });
        on_south_edge || on_west_edge
    }

    #[test]
    fn every_vertex_lies_on_an_edge_that_brackets_its_level() {
        let mut rng = SplitMix(0x5eed_0fc0_7709);
        for case in 0..500 {
            let rows = 2 + rng.below(10) as usize;
            let cols = 2 + rng.below(10) as usize;
            let lats = random_axis(&mut rng, 40.5, rows);
            let lons = random_axis(&mut rng, -74.2, cols);
            // Whole-metre values make ties with the levels common; some are missing.
            let values: Vec<f32> = (0..rows * cols)
                .map(|_| match rng.below(12) {
                    0 => f32::NAN,
                    1..=5 => -(rng.below(8) as f32),
                    _ => -(rng.unit() * 8.0) as f32,
                })
                .collect();
            let g = grid(lats, lons, values);
            let lv = levels(-7.0, 0.0, 1.0 + rng.below(3) as f64);
            for line in contours(&g, &lv) {
                assert!(line.points.len() >= 2, "case {case}");
                for p in &line.points {
                    assert!(on_bracketing_edge(&g, *p, line.value), "case {case}: {p:?}");
                }
            }
        }
    }

    #[test]
    fn output_is_the_same_on_every_run() {
        let g = shelf();
        let lv = levels(-80.0, 0.0, 10.0);
        assert_eq!(contours(&g, &lv), contours(&g, &lv));
    }

    /// A synthetic shelf over the box of the recorded DEM export (−74.2…−73.0°E, 40.0…40.8°N,
    /// 240 by 160 cells of 0.005°, listed north first as the export is): the bottom falls from
    /// the north-west corner toward the south-east, a channel runs down it, a shoal rises off it,
    /// and a patch of cells has no data.
    fn shelf() -> Grid {
        let step = 0.005;
        let lats: Vec<f64> = (0..160)
            .map(|i| 40.8 - (f64::from(i) + 0.5) * step)
            .collect();
        let lons: Vec<f64> = (0..240)
            .map(|j| -74.2 + (f64::from(j) + 0.5) * step)
            .collect();
        let values = lats
            .iter()
            .flat_map(|&lat| lons.iter().map(move |&lon| (lat, lon)))
            .map(|(lat, lon)| {
                let x = (lon + 74.2) / 1.2;
                let y = (40.8 - lat) / 0.8;
                if (0.70..0.74).contains(&x) && (0.30..0.36).contains(&y) {
                    return f32::NAN;
                }
                let slope = 10.0 - 70.0 * x - 40.0 * y + 12.0 * x * y;
                let off = x - (0.15 + 0.35 * y);
                let across = off / 0.03;
                let channel = -25.0 / (1.0 + across * across);
                let (sx, sy) = (x - 0.6, y - 0.6);
                let shoal = 30.0 / (1.0 + (sx * sx + sy * sy) / 0.004);
                (slope + channel + shoal) as f32
            })
            .collect();
        grid(lats, lons, values)
    }

    #[test]
    fn shelf_contours_snapshot() {
        let g = shelf();
        assert!(g.lats[0] < g.lats[1]);
        let lines = contours(&g, &levels(-80.0, 0.0, 10.0));
        let mut by_level: BTreeMap<i64, Vec<&Polyline>> = BTreeMap::new();
        for line in &lines {
            by_level.entry(line.value as i64).or_default().push(line);
        }
        let mut table = String::from("level lines closed vertices simplified_25m labels_2km\n");
        for (level, lines) in &by_level {
            let owned: Vec<Polyline> = lines.iter().map(|l| (*l).clone()).collect();
            let closed = owned.iter().filter(|l| is_closed(l)).count();
            let vertices: usize = owned.iter().map(|l| l.points.len()).sum();
            let simplified: usize = owned.iter().map(|l| simplify(l, 25.0).points.len()).sum();
            let labels = label_anchors(&owned, 2000.0).len();
            writeln!(
                table,
                "{level} {} {closed} {vertices} {simplified} {labels}",
                owned.len()
            )
            .unwrap();
        }
        insta::assert_snapshot!(table);
    }
}
