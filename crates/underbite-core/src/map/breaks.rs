//! Sea surface temperature and chlorophyll breaks: where the gradient is strong, as lines.
//!
//! [`breaks`] finds the fronts of a [`Grid`]: it measures how fast the field changes per
//! kilometre, traces the field's lines of equal value every `spacing` with
//! [`contours`](crate::map::contours::contours), and keeps the stretches of those lines where the
//! change is at least a threshold. Each piece is a [`Polyline`] whose `value` is its strength:
//! the mean gradient along it, in °C per km for temperature and in log10 units per km for
//! chlorophyll. [`breaks_with`] takes the smoothing and the minimum length as well.
//!
//! # The field
//!
//! Temperature is used as it is. Chlorophyll a is used as the log10 of its concentration, so a
//! front is a ratio of concentrations and is found at the same place and with the same strength
//! whatever the absolute level; a concentration that is not a positive number counts as missing.
//!
//! The optional smoothing replaces each value by the mean of the 3 × 3 window around it. It only
//! applies where all nine values are present; elsewhere (beside missing data and on the outer
//! rows and columns) the smoothed field is missing, so smoothing never spreads a value into a gap
//! nor mixes a gap's border into the water beyond.
//!
//! # The gradient
//!
//! The gradient at a grid point is measured by central differences along each axis, in
//! kilometres between the grid points: north–south steps use the length of a degree of latitude,
//! east–west steps the length of a degree of longitude at the point's own latitude (both from
//! [`metres_per_degree`](crate::map::contours::metres_per_degree), so a degree of longitude
//! shrinks with the cosine of the latitude). Unevenly spaced axes (Web Mercator rows) use the
//! three-point difference for uneven steps, which is the usual central difference when the steps
//! are equal. Grids of any step therefore give comparable strengths.
//!
//! A grid point has a gradient only when it and its eight neighbours all hold a value: never
//! across a cloud, land or the edge of the grid. With smoothing that pushes a break two cells away
//! from missing data, without it one cell.
//!
//! # The pieces
//!
//! The strength at a point of a traced line is interpolated between the gradients of the grid
//! points around it; a point next to a grid point without a gradient has no strength. A piece is
//! a run of points whose strength is at least the threshold. Where the strength crosses the
//! threshold between two points the piece ends at the crossing, found by linear interpolation;
//! where a point has no strength the piece ends at the last point that has. A ring that is strong
//! all the way round stays a closed ring (its first point repeated last); a piece of a ring that
//! runs through the ring's first point is one piece. Pieces shorter than the minimum length
//! ([`DEFAULT_MIN_LENGTH_M`] unless given) are dropped.
//!
//! The output holds the pieces of each traced line in order along it, the lines in the order
//! [`contours`](crate::map::contours::contours) gives them (by level, then by first point), so it
//! is deterministic.

use crate::map::contours::{contours, levels, metres_per_degree};
use crate::map::grid::Grid;
use crate::map::{GeoPoint, Polyline};

/// The shortest break [`breaks`] keeps, metres.
pub const DEFAULT_MIN_LENGTH_M: f64 = 5_000.0;

/// What a grid holds, which decides how its values are compared.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BreakKind {
    /// Sea surface temperature, °C; the gradient is in °C per km.
    Sst,
    /// Chlorophyll a concentration, mg m⁻³; the gradient is in log10 units per km.
    Chlorophyll,
}

impl BreakKind {
    /// Whether [`breaks`] smooths this kind of grid: chlorophyll yes, as a satellite's pixels are
    /// noisy; temperature no, as MUR and GLSEA are already smooth analyses.
    pub const fn smooths_by_default(self) -> bool {
        matches!(self, Self::Chlorophyll)
    }

    /// A grid value in the field the gradient is taken of; NaN where it is missing.
    fn field(self, value: f32) -> f64 {
        let v = f64::from(value);
        match self {
            Self::Sst if v.is_finite() => v,
            Self::Chlorophyll if v.is_finite() && v > 0.0 => v.log10(),
            _ => f64::NAN,
        }
    }
}

/// How [`breaks_with`] finds breaks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BreakOptions {
    /// The least gradient drawn, per km in the kind's units (°C, or log10 of the
    /// concentration). A negative threshold counts as zero.
    pub threshold_per_km: f64,
    /// The step between the traced lines of equal value, in the kind's units.
    pub spacing: f64,
    /// Whether to smooth the field with a 3 × 3 mean first.
    pub smooth: bool,
    /// The shortest piece kept, metres. A negative length counts as zero.
    pub min_length_m: f64,
}

impl BreakOptions {
    /// The options [`breaks`] uses: the kind's default smoothing and [`DEFAULT_MIN_LENGTH_M`].
    pub const fn new(kind: BreakKind, threshold_per_km: f64, spacing: f64) -> Self {
        Self {
            threshold_per_km,
            spacing,
            smooth: kind.smooths_by_default(),
            min_length_m: DEFAULT_MIN_LENGTH_M,
        }
    }
}

/// The breaks of `grid`: stretches of its lines of equal value, every `spacing`, where the
/// gradient is at least `threshold_per_km` (see the module notes). Uses [`BreakOptions::new`].
pub fn breaks(grid: &Grid, kind: BreakKind, threshold_per_km: f64, spacing: f64) -> Vec<Polyline> {
    breaks_with(
        grid,
        kind,
        &BreakOptions::new(kind, threshold_per_km, spacing),
    )
}

/// The breaks of `grid` with every option given.
///
/// Empty for a threshold or minimum length that is not a number, a spacing that is not a positive
/// number (or so fine that [`levels`] lists none), a grid smaller than 3 × 3, or a grid whose
/// fields have been changed so that they no longer make a grid.
pub fn breaks_with(grid: &Grid, kind: BreakKind, options: &BreakOptions) -> Vec<Polyline> {
    if options.threshold_per_km.is_nan() || options.min_length_m.is_nan() {
        return Vec::new();
    }
    let threshold = options.threshold_per_km.max(0.0);
    let min_length_m = options.min_length_m.max(0.0);
    let Some(field) = Field::of(grid, kind, options.smooth) else {
        return Vec::new();
    };
    let Some((low, high)) = field.range() else {
        return Vec::new();
    };
    let levels = levels(low, high, options.spacing);
    if levels.is_empty() {
        return Vec::new();
    }
    let Ok(traced) = Grid::new(
        grid.source.clone(),
        grid.time,
        grid.lats.clone(),
        grid.lons.clone(),
        field.values.iter().map(|&v| v as f32).collect(),
        grid.unit.clone(),
    ) else {
        return Vec::new();
    };
    let gradient = field.gradient_per_km();
    contours(&traced, &levels)
        .iter()
        .flat_map(|line| {
            pieces(
                line,
                |p| field.strength_at(&gradient, p),
                threshold,
                min_length_m,
            )
        })
        .collect()
}

/// The field of a grid, smoothed or not, over the grid's axes.
struct Field<'a> {
    lats: &'a [f64],
    lons: &'a [f64],
    /// Row-major like the grid's values; NaN where missing.
    values: Vec<f64>,
}

impl<'a> Field<'a> {
    /// The field of `grid`; `None` when its fields do not make a grid of at least 3 × 3.
    fn of(grid: &'a Grid, kind: BreakKind, smooth: bool) -> Option<Self> {
        let (rows, cols) = (grid.lats.len(), grid.lons.len());
        if rows < 3
            || cols < 3
            || grid.values.len() != rows * cols
            || !ascending(&grid.lats)
            || !ascending(&grid.lons)
        {
            return None;
        }
        let raw: Vec<f64> = grid.values.iter().map(|&v| kind.field(v)).collect();
        let mut field = Self {
            lats: &grid.lats,
            lons: &grid.lons,
            values: raw,
        };
        if smooth {
            field.values = (0..rows * cols)
                .map(|k| {
                    let (i, j) = (k / cols, k % cols);
                    field
                        .window(i, j)
                        .map_or(f64::NAN, |w| w.iter().sum::<f64>() / 9.0)
                })
                .collect();
        }
        Some(field)
    }

    fn cols(&self) -> usize {
        self.lons.len()
    }

    fn at(&self, i: usize, j: usize) -> f64 {
        self.values[i * self.cols() + j]
    }

    /// The nine values around grid point `(i, j)`, row by row from the south-west; `None` on the
    /// outer rows and columns or when any of them is missing.
    fn window(&self, i: usize, j: usize) -> Option<[f64; 9]> {
        if i == 0 || j == 0 || i + 1 >= self.lats.len() || j + 1 >= self.cols() {
            return None;
        }
        let mut w = [0.0; 9];
        for (k, slot) in w.iter_mut().enumerate() {
            let v = self.at(i + k / 3 - 1, j + k % 3 - 1);
            if !v.is_finite() {
                return None;
            }
            *slot = v;
        }
        Some(w)
    }

    /// The least and greatest values present.
    fn range(&self) -> Option<(f64, f64)> {
        self.values
            .iter()
            .filter(|v| v.is_finite())
            .fold(None, |range, &v| match range {
                None => Some((v, v)),
                Some((lo, hi)) => Some((f64::min(lo, v), f64::max(hi, v))),
            })
    }

    /// The gradient's magnitude at each grid point, per km; NaN where there is none.
    fn gradient_per_km(&self) -> Vec<f64> {
        let (rows, cols) = (self.lats.len(), self.cols());
        let mut out = vec![f64::NAN; rows * cols];
        for i in 1..rows - 1 {
            let (lat_s, lat, lat_n) = (self.lats[i - 1], self.lats[i], self.lats[i + 1]);
            let south_km = (lat - lat_s) * metres_per_degree(f64::midpoint(lat_s, lat)).0 / 1e3;
            let north_km = (lat_n - lat) * metres_per_degree(f64::midpoint(lat, lat_n)).0 / 1e3;
            let lon_km_per_deg = metres_per_degree(lat).1 / 1e3;
            for j in 1..cols - 1 {
                if self.window(i, j).is_none() {
                    continue;
                }
                let west_km = (self.lons[j] - self.lons[j - 1]) * lon_km_per_deg;
                let east_km = (self.lons[j + 1] - self.lons[j]) * lon_km_per_deg;
                let centre = self.at(i, j);
                let d_north = derivative(
                    (self.at(i - 1, j), centre, self.at(i + 1, j)),
                    south_km,
                    north_km,
                );
                let d_east = derivative(
                    (self.at(i, j - 1), centre, self.at(i, j + 1)),
                    west_km,
                    east_km,
                );
                if let (Some(n), Some(e)) = (d_north, d_east) {
                    let g = n.hypot(e);
                    if g.is_finite() {
                        out[i * cols + j] = g;
                    }
                }
            }
        }
        out
    }

    /// The gradient at `p`, interpolated between the grid points around it; `None` outside the
    /// grid or when a grid point it depends on has no gradient.
    fn strength_at(&self, gradient: &[f64], at: GeoPoint) -> Option<f64> {
        let (row, north) = bracket(self.lats, at.lat)?;
        let (col, east) = bracket(self.lons, at.lon)?;
        let cols = self.cols();
        let corners = [
            (row, col, (1.0 - north) * (1.0 - east)),
            (row, col + 1, (1.0 - north) * east),
            (row + 1, col, north * (1.0 - east)),
            (row + 1, col + 1, north * east),
        ];
        let mut sum = 0.0;
        for (i, j, weight) in corners {
            if weight > 0.0 {
                let here = gradient[i * cols + j];
                if !here.is_finite() {
                    return None;
                }
                sum += weight * here;
            }
        }
        Some(sum)
    }
}

fn ascending(axis: &[f64]) -> bool {
    axis.iter().all(|v| v.is_finite()) && axis.windows(2).all(|w| w[0] < w[1])
}

/// The derivative at the middle of three values, `before_km` after the first and `after_km`
/// before the last: the three-point difference for uneven steps, exact for a parabola.
fn derivative(
    (before, centre, after): (f64, f64, f64),
    before_km: f64,
    after_km: f64,
) -> Option<f64> {
    if !(before_km > 0.0 && after_km > 0.0) {
        return None;
    }
    let (hb, ha) = (before_km, after_km);
    let d = (hb * hb * (after - centre) + ha * ha * (centre - before)) / (hb * ha * (hb + ha));
    d.is_finite().then_some(d)
}

/// The index of the axis interval holding `x` and the share of the way along it; `None` outside
/// the axis. An axis needs two values.
fn bracket(axis: &[f64], x: f64) -> Option<(usize, f64)> {
    let n = axis.len();
    if n < 2 || !(axis[0] <= x && x <= axis[n - 1]) {
        return None;
    }
    let k = axis
        .partition_point(|&a| a <= x)
        .saturating_sub(1)
        .min(n - 2);
    let t = ((x - axis[k]) / (axis[k + 1] - axis[k])).clamp(0.0, 1.0);
    Some((k, t))
}

/// The pieces of `line` where `strength` is at least `threshold`, at least `min_length_m` long,
/// each with its mean strength as its value.
fn pieces(
    line: &Polyline,
    strength: impl Fn(GeoPoint) -> Option<f64>,
    threshold: f64,
    min_length_m: f64,
) -> Vec<Polyline> {
    let points: Vec<(GeoPoint, Option<f64>)> =
        line.points.iter().map(|&p| (p, strength(p))).collect();
    let strong = |s: Option<f64>| s.is_some_and(|v| v >= threshold);
    let n = points.len();
    let closed = n >= 4 && line.points[0] == line.points[n - 1];
    let runs = if closed {
        let ring = &points[..n - 1];
        match ring.iter().position(|&(_, s)| !strong(s)) {
            // Strong all the way round: the ring as it is.
            None => vec![
                points
                    .iter()
                    .filter_map(|&(p, s)| s.map(|s| (p, s)))
                    .collect(),
            ],
            // Start at a weak point, so a piece through the ring's first point stays whole.
            Some(weak) => {
                let mut turned: Vec<_> =
                    ring[weak..].iter().chain(&ring[..weak]).copied().collect();
                turned.push(ring[weak]);
                strong_runs(&turned, threshold)
            }
        }
    } else {
        strong_runs(&points, threshold)
    };
    runs.into_iter()
        .filter_map(|run| finish(run, min_length_m))
        .collect()
}

/// The runs of points whose strength is at least `threshold`, cut where the strength crosses it.
fn strong_runs(points: &[(GeoPoint, Option<f64>)], threshold: f64) -> Vec<Vec<(GeoPoint, f64)>> {
    let mut runs = Vec::new();
    let mut run: Option<Vec<(GeoPoint, f64)>> = None;
    for (k, &(p, s)) in points.iter().enumerate() {
        let before = k.checked_sub(1).map(|b| points[b]);
        match s {
            Some(v) if v >= threshold => {
                let current = run.get_or_insert_with(|| {
                    // Coming from a weak point: start where the strength reaches the threshold.
                    match before {
                        Some((q, Some(w))) => vec![crossing((q, w), (p, v), threshold)],
                        _ => Vec::new(),
                    }
                });
                current.push((p, v));
            }
            _ => {
                if let Some(mut current) = run.take() {
                    if let (Some(v), Some((q, Some(w)))) = (s, before) {
                        current.push(crossing((q, w), (p, v), threshold));
                    }
                    runs.push(current);
                }
            }
        }
    }
    runs.extend(run);
    runs
}

/// The point between `a` and `b` where the strength, varying linearly, equals `threshold`.
/// One of the two is below it and the other at or above it.
fn crossing(a: (GeoPoint, f64), b: (GeoPoint, f64), threshold: f64) -> (GeoPoint, f64) {
    let ((pa, sa), (pb, sb)) = (a, b);
    let span = sb - sa;
    let t = if span.abs() > 0.0 {
        ((threshold - sa) / span).clamp(0.0, 1.0)
    } else {
        0.5
    };
    let at = GeoPoint {
        lat: pa.lat + t * (pb.lat - pa.lat),
        lon: pa.lon + t * (pb.lon - pa.lon),
    };
    (at, threshold)
}

/// A run as a [`Polyline`] valued by its length-weighted mean strength; `None` when it is shorter
/// than `min_length_m` or has no length.
fn finish(mut run: Vec<(GeoPoint, f64)>, min_length_m: f64) -> Option<Polyline> {
    run.dedup_by(|b, a| a.0 == b.0);
    let mut length = 0.0;
    let mut weighted = 0.0;
    for pair in run.windows(2) {
        let d = segment_length_m(pair[0].0, pair[1].0);
        length += d;
        weighted += d * f64::midpoint(pair[0].1, pair[1].1);
    }
    if !(length > 0.0 && length.is_finite()) || length < min_length_m {
        return None;
    }
    Some(Polyline {
        points: run.into_iter().map(|(p, _)| p).collect(),
        value: weighted / length,
    })
}

/// The length of a short segment in metres, measured at its middle latitude.
fn segment_length_m(a: GeoPoint, b: GeoPoint) -> f64 {
    let (lat_m, lon_m) = metres_per_degree(f64::midpoint(a.lat, b.lat));
    ((b.lon - a.lon) * lon_m).hypot((b.lat - a.lat) * lat_m)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)]

    use std::fmt::Write as _;

    use super::*;
    use crate::map::{ErddapNode, GridSource};
    use crate::time::UnixTime;

    /// The ADR defaults: 0.5 °C per km, and 0.15 log10 per 4 km.
    const SST_THRESHOLD: f64 = 0.5;
    const CHL_THRESHOLD: f64 = 0.0375;

    /// A tanh front of 2 °C over 5 km: `16 + tanh(x / L)` changes by 1.93 °C between ±2.5 km,
    /// with a peak gradient of 1 / L = 0.8 °C per km.
    const L_KM: f64 = 1.25;

    const STEP: f64 = 0.005;

    /// No breaks.
    const NONE: &[Polyline] = &[];

    fn source(step: f64) -> GridSource {
        GridSource {
            node: ErddapNode::Pfeg,
            dataset: "synthetic".to_owned(),
            variables: vec!["v".to_owned()],
            altitude_axis: false,
            native_step_deg: step,
        }
    }

    fn steps(from: f64, step: f64, count: usize) -> Vec<f64> {
        (0..count).map(|i| from + step * i as f64).collect()
    }

    /// A grid with `f(lat, lon)` at each point of the axes.
    fn build(lats: Vec<f64>, lons: Vec<f64>, f: impl Fn(f64, f64) -> f64) -> Grid {
        let values = lats
            .iter()
            .flat_map(|&lat| lons.iter().map(move |&lon| (lat, lon)))
            .map(|(lat, lon)| f(lat, lon) as f32)
            .collect();
        Grid::new(
            source(STEP),
            UnixTime(1_794_042_000),
            lats,
            lons,
            values,
            "degree_C",
        )
        .unwrap()
    }

    /// Kilometres east of `lon0` at latitude `lat`.
    fn east_km(lat: f64, lon: f64, lon0: f64) -> f64 {
        (lon - lon0) * metres_per_degree(lat).1 / 1e3
    }

    /// Kilometres north of `lat0`.
    fn north_km(lat: f64, lat0: f64) -> f64 {
        (lat - lat0) * metres_per_degree(f64::midpoint(lat, lat0)).0 / 1e3
    }

    /// Axes of `count` points around a centre, the centre a third of a step off a grid point so
    /// that no grid value sits exactly on a traced level.
    fn around(centre: f64, step: f64, count: usize) -> Vec<f64> {
        let half = (count / 2) as f64;
        steps(centre - (half + 1.0 / 3.0) * step, step, count)
    }

    /// An SST front running north–south through `(lat0, lon0)`, warmer to the east.
    fn east_front(lat0: f64, lon0: f64, lon_step: f64, l_km: f64) -> Grid {
        build(
            around(lat0, STEP, 81),
            around(lon0, lon_step, 81),
            |lat, lon| 16.0 + (east_km(lat, lon, lon0) / l_km).tanh(),
        )
    }

    /// An SST front running east–west through `lat0` on the given latitudes, warmer to the north.
    fn north_front(rows: Vec<f64>, front_lat: f64, centre_lon: f64) -> Grid {
        build(rows, around(centre_lon, STEP, 81), |lat, _| {
            16.0 + (north_km(lat, front_lat) / L_KM).tanh()
        })
    }

    fn length_m(line: &Polyline) -> f64 {
        line.points
            .windows(2)
            .map(|w| segment_length_m(w[0], w[1]))
            .sum()
    }

    fn is_closed(line: &Polyline) -> bool {
        line.points.len() >= 4 && line.points.first() == line.points.last()
    }

    /// Every point of every line has a nearest grid point whose `reach`-cell window holds values
    /// in `grid`: no break is drawn beside missing data.
    fn assert_clear_of_missing(grid: &Grid, lines: &[Polyline], reach: usize) {
        let (rows, cols) = (grid.lats.len(), grid.lons.len());
        for point in lines.iter().flat_map(|l| &l.points) {
            let (row, north) = bracket(&grid.lats, point.lat).unwrap();
            let (col, east) = bracket(&grid.lons, point.lon).unwrap();
            let row = row + usize::from(north > 0.5);
            let col = col + usize::from(east > 0.5);
            assert!(row >= reach && col >= reach && row + reach < rows && col + reach < cols);
            for i in row - reach..=row + reach {
                for j in col - reach..=col + reach {
                    assert!(
                        grid.value(i, j).unwrap().is_finite(),
                        "{point:?} is beside missing data at ({i}, {j})"
                    );
                }
            }
        }
    }

    fn sst(grid: &Grid, spacing: f64) -> Vec<Polyline> {
        breaks(grid, BreakKind::Sst, SST_THRESHOLD, spacing)
    }

    #[test]
    fn a_tanh_front_gives_one_line_along_the_front() {
        let (lat0, lon0) = (40.0, -72.0);
        let grid = east_front(lat0, lon0, STEP, L_KM);
        let lines = sst(&grid, 1.0);
        assert_eq!(lines.len(), 1, "{lines:?}");
        let line = &lines[0];
        assert!(!is_closed(line));
        for p in &line.points {
            assert!((p.lon - lon0).abs() < STEP / 2.0, "{p:?} is off the front");
        }
        // Central differences on a 0.43 km step read the 0.8 °C per km peak a little low.
        assert!((0.7..0.82).contains(&line.value), "{}", line.value);
        // From the second row to the last but one: the outer rows have no gradient.
        let south = line.points.iter().map(|p| p.lat).fold(f64::MAX, f64::min);
        let north = line.points.iter().map(|p| p.lat).fold(f64::MIN, f64::max);
        assert_eq!(south, grid.lats[1]);
        assert_eq!(north, grid.lats[79]);
        assert!(length_m(line) > 43_000.0, "{}", length_m(line));
    }

    #[test]
    fn a_finer_spacing_gives_a_line_per_level_across_the_front() {
        let grid = east_front(40.0, -72.0, STEP, L_KM);
        // 15.5, 16 and 16.5: tanh is ±0.5 at ±0.55 L, where the gradient is 0.6 °C per km.
        let lines = sst(&grid, 0.5);
        assert_eq!(lines.len(), 3, "{lines:?}");
        let middle = |l: &Polyline| l.points[l.points.len() / 2].lon;
        assert!(middle(&lines[0]) < middle(&lines[1]) && middle(&lines[1]) < middle(&lines[2]));
        assert!(lines[1].value > lines[0].value && lines[1].value > lines[2].value);
    }

    #[test]
    fn a_constant_field_gives_no_breaks() {
        for value in [16.0, 16.3] {
            let grid = build(around(40.0, STEP, 40), around(-72.0, STEP, 40), |_, _| {
                value
            });
            assert_eq!(sst(&grid, 1.0), NONE);
            assert_eq!(breaks(&grid, BreakKind::Sst, 0.0, 0.1), NONE);
        }
    }

    #[test]
    fn a_gentle_front_is_below_the_threshold() {
        // 2 °C over 50 km: a peak of 0.08 °C per km.
        let grid = east_front(40.0, -72.0, 0.02, 12.5);
        assert_eq!(sst(&grid, 1.0), NONE);
        let lines = breaks(&grid, BreakKind::Sst, 0.05, 1.0);
        assert_eq!(lines.len(), 1);
        assert!(
            (0.07..0.081).contains(&lines[0].value),
            "{}",
            lines[0].value
        );
    }

    #[test]
    fn east_west_steps_shrink_with_the_latitude() {
        // The same front on the same 0.5 km step at the equator and at 60° north: the
        // longitude step is twice as many degrees at 60°.
        let strength = |lat: f64| {
            let step = 0.5e3 / metres_per_degree(lat).1;
            let lines = sst(&east_front(lat, 10.0, step, L_KM), 1.0);
            assert_eq!(lines.len(), 1);
            lines[0].value
        };
        let (equator, sixty) = (strength(0.0), strength(60.0));
        assert!(
            (equator - sixty).abs() / equator < 0.01,
            "{equator} {sixty}"
        );
        assert!((0.7..0.82).contains(&sixty), "{sixty}");
    }

    #[test]
    fn a_north_south_gradient_has_the_same_strength() {
        let lines = sst(&north_front(around(40.0, STEP, 81), 40.0, -72.0), 1.0);
        assert_eq!(lines.len(), 1);
        for p in &lines[0].points {
            assert!((p.lat - 40.0).abs() < STEP / 2.0, "{p:?}");
        }
        assert!((0.7..0.82).contains(&lines[0].value), "{}", lines[0].value);
    }

    #[test]
    fn uneven_latitudes_give_the_same_front() {
        // Rows alternately 0.004° and 0.006° apart around the same front.
        let mut lats = vec![39.8 + 0.001];
        for k in 0..80 {
            let last = lats[lats.len() - 1];
            lats.push(last + if k % 2 == 0 { 0.004 } else { 0.006 });
        }
        let even = sst(&north_front(around(40.0, STEP, 81), 40.0, -72.0), 1.0);
        let uneven = sst(&north_front(lats, 40.0, -72.0), 1.0);
        assert_eq!(uneven.len(), 1);
        for p in &uneven[0].points {
            assert!((p.lat - 40.0).abs() < 0.003, "{p:?}");
        }
        let (a, b) = (even[0].value, uneven[0].value);
        assert!((a - b).abs() / a < 0.05, "{a} {b}");
    }

    /// Chlorophyll changing by a factor of two (0.3 in log10) over 5 km around `c0`.
    fn chlorophyll_front(c0: f64) -> Grid {
        let (lat0, lon0) = (40.0, -72.0);
        build(
            around(lat0, STEP, 81),
            around(lon0, STEP, 81),
            |lat, lon| c0 * 10f64.powf(0.15 * (east_km(lat, lon, lon0) / L_KM).tanh()),
        )
    }

    #[test]
    fn a_chlorophyll_front_is_found_in_log_space_at_any_level() {
        let found: Vec<Vec<Polyline>> = [0.1, 1.0, 10.0]
            .into_iter()
            .map(|c0| {
                breaks(
                    &chlorophyll_front(c0),
                    BreakKind::Chlorophyll,
                    CHL_THRESHOLD,
                    0.1,
                )
            })
            .collect();
        // log10 c0 − 0.1, log10 c0 and log10 c0 + 0.1.
        assert_eq!(found[0].len(), 3, "{:?}", found[0]);
        for other in &found[1..] {
            assert_eq!(other.len(), found[0].len());
            for (a, b) in found[0].iter().zip(other) {
                assert_eq!(a.points.len(), b.points.len());
                for (p, q) in a.points.iter().zip(&b.points) {
                    assert!((p.lat - q.lat).abs() < 1e-5 && (p.lon - q.lon).abs() < 1e-5);
                }
                assert!((a.value - b.value).abs() < 1e-3 * a.value);
            }
        }
        // The middle line lies on the front; its strength is near the 0.12 log10 per km peak,
        // a little lower for the smoothing.
        let middle = &found[1][1];
        for p in &middle.points {
            assert!((p.lon - -72.0).abs() < STEP / 2.0, "{p:?}");
        }
        assert!((0.09..0.122).contains(&middle.value), "{}", middle.value);
    }

    #[test]
    fn chlorophyll_that_is_not_positive_is_missing() {
        let mut zeros = chlorophyll_front(1.0);
        let mut holes = zeros.clone();
        let cols = zeros.lons.len();
        zeros.values[40 * cols + 40] = 0.0;
        zeros.values[20 * cols + 39] = -1.0;
        holes.values[40 * cols + 40] = f32::NAN;
        holes.values[20 * cols + 39] = f32::NAN;
        let lines = breaks(&zeros, BreakKind::Chlorophyll, CHL_THRESHOLD, 0.1);
        assert_ne!(lines, NONE);
        assert_eq!(
            lines,
            breaks(&holes, BreakKind::Chlorophyll, CHL_THRESHOLD, 0.1)
        );
        assert_clear_of_missing(&holes, &lines, 2);
    }

    /// A warm ring of radius `r_km` around 40° N 72° W whose edge is `l_km(bearing)` wide, the
    /// bearing as an angle counter-clockwise from east.
    fn ring(r_km: f64, l_km: impl Fn(f64) -> f64) -> Grid {
        let (lat0, lon0) = (40.0, -72.0);
        build(
            around(lat0, STEP, 101),
            around(lon0, STEP, 131),
            |lat, lon| {
                let (e, n) = (east_km(lat, lon, lon0), north_km(lat, lat0));
                16.0 + ((r_km - e.hypot(n)) / l_km(n.atan2(e))).tanh()
            },
        )
    }

    #[test]
    fn a_ring_eddy_gives_one_closed_ring() {
        let lines = sst(&ring(12.0, |_| L_KM), 1.0);
        assert_eq!(lines.len(), 1, "{lines:?}");
        let line = &lines[0];
        assert!(is_closed(line));
        for p in &line.points {
            let r = east_km(p.lat, p.lon, -72.0).hypot(north_km(p.lat, 40.0));
            assert!((r - 12.0).abs() < 0.3, "{p:?} at {r} km");
        }
        assert!((0.7..0.82).contains(&line.value), "{}", line.value);
        let circumference = 2.0 * std::f64::consts::PI * 12_000.0;
        assert!((length_m(line) - circumference).abs() < 0.01 * circumference);
    }

    #[test]
    fn a_ring_sharp_only_in_the_south_gives_one_piece_through_its_start() {
        // 1.25 km wide due south, 10 km due north: strong within about 34° of south.
        let lines = sst(
            &ring(12.0, |bearing| 1.25 + 8.75 * (1.0 + bearing.sin()) / 2.0),
            1.0,
        );
        assert_eq!(lines.len(), 1, "{lines:?}");
        let line = &lines[0];
        assert!(!is_closed(line));
        assert!(line.value >= SST_THRESHOLD);
        let southmost = line.points.iter().map(|p| p.lat).fold(f64::MAX, f64::min);
        assert!(north_km(southmost, 40.0) < -11.7, "{southmost}");
        for p in &line.points {
            assert!(north_km(p.lat, 40.0) < -9.0, "{p:?}");
        }
        let length = length_m(line);
        assert!((12_000.0..16_000.0).contains(&length), "{length}");
    }

    #[test]
    fn a_cloud_over_the_front_splits_it_and_nothing_is_drawn_beside_the_cloud() {
        let mut grid = east_front(40.0, -72.0, STEP, L_KM);
        let cols = grid.lons.len();
        for i in 35..45 {
            for j in 36..44 {
                grid.values[i * cols + j] = f32::NAN;
            }
        }
        // A single missing cell beside the front as well.
        grid.values[60 * cols + 41] = f32::NAN;
        let lines = sst(&grid, 1.0);
        assert!(lines.len() >= 2, "{lines:?}");
        assert_clear_of_missing(&grid, &lines, 1);
        for line in &lines {
            assert!(line.value >= SST_THRESHOLD);
        }
        let smoothed = breaks_with(
            &grid,
            BreakKind::Sst,
            &BreakOptions {
                smooth: true,
                ..BreakOptions::new(BreakKind::Sst, SST_THRESHOLD, 1.0)
            },
        );
        assert!(smoothed.len() >= 2);
        assert_clear_of_missing(&grid, &smoothed, 2);
    }

    #[test]
    fn a_jump_across_a_gap_is_not_a_break() {
        // 10 °C west of a one-cell gap and 20 °C east of it: no gradient is taken across it.
        let grid = build(around(40.0, STEP, 40), around(-72.0, STEP, 40), |_, lon| {
            if lon < -72.0 - STEP / 2.0 {
                10.0
            } else if lon < -72.0 + STEP / 2.0 {
                f64::NAN
            } else {
                20.0
            }
        });
        assert_eq!(grid.valid_count(), 40 * 39);
        assert_eq!(sst(&grid, 1.0), NONE);
        assert_eq!(breaks(&grid, BreakKind::Sst, 0.0, 0.5), NONE);
        let chl = build(grid.lats.clone(), grid.lons.clone(), |lat, lon| {
            let v = grid
                .sample_nearest(GeoPoint { lat, lon })
                .map_or(f64::NAN, f64::from);
            v / 10.0
        });
        assert_eq!(breaks(&chl, BreakKind::Chlorophyll, 0.0, 0.05), NONE);
    }

    #[test]
    fn short_pieces_are_dropped() {
        // Sharp only within about 1.5 km of the front's middle.
        let (lat0, lon0) = (40.0, -72.0);
        let grid = build(
            around(lat0, STEP, 81),
            around(lon0, STEP, 81),
            |lat, lon| {
                let n = north_km(lat, lat0);
                let l = 10.0 - 8.75 * (-(n / 5.0).powi(2)).exp();
                16.0 + (east_km(lat, lon, lon0) / l).tanh()
            },
        );
        assert_eq!(sst(&grid, 1.0), NONE);
        let all = BreakOptions {
            min_length_m: 0.0,
            ..BreakOptions::new(BreakKind::Sst, SST_THRESHOLD, 1.0)
        };
        let lines = breaks_with(&grid, BreakKind::Sst, &all);
        assert_eq!(lines.len(), 1);
        let length = length_m(&lines[0]);
        assert!(
            (2_000.0..DEFAULT_MIN_LENGTH_M).contains(&length),
            "{length}"
        );
        assert!(lines[0].value >= SST_THRESHOLD);
        // The piece ends where the strength crosses the threshold, between two traced points.
        let ends = [lines[0].points[0], *lines[0].points.last().unwrap()];
        let field = Field::of(&grid, BreakKind::Sst, false).unwrap();
        let gradient = field.gradient_per_km();
        for end in ends {
            let s = field.strength_at(&gradient, end);
            assert!(s.is_none_or(|s| (s - SST_THRESHOLD).abs() < 0.05), "{s:?}");
        }
    }

    #[test]
    fn a_threshold_above_the_peak_gives_nothing() {
        let grid = east_front(40.0, -72.0, STEP, L_KM);
        assert_eq!(breaks(&grid, BreakKind::Sst, 0.9, 1.0), NONE);
        assert_eq!(breaks(&grid, BreakKind::Sst, f64::INFINITY, 1.0), NONE);
        let low = breaks(&grid, BreakKind::Sst, -1.0, 1.0);
        assert_eq!(low, breaks(&grid, BreakKind::Sst, 0.0, 1.0));
        assert_ne!(low, NONE);
        // A low threshold keeps the front and nothing else.
        assert_eq!(breaks(&grid, BreakKind::Sst, 0.1, 1.0).len(), 1);
    }

    #[test]
    fn smoothing_keeps_a_plane_and_quiets_noise() {
        // 1 °C per 0.01° of longitude: a plane, which a 3 × 3 mean leaves as it is.
        let lats = around(40.0, STEP, 40);
        let lons = around(-72.0, STEP, 40);
        let plane = build(lats, lons, |_, lon| (lon + 72.0) * 100.0);
        let options = |smooth| BreakOptions {
            smooth,
            min_length_m: 0.0,
            ..BreakOptions::new(BreakKind::Sst, SST_THRESHOLD, 0.5)
        };
        let rough = breaks_with(&plane, BreakKind::Sst, &options(false));
        let smooth = breaks_with(&plane, BreakKind::Sst, &options(true));
        assert_ne!(rough, NONE);
        assert_ne!(smooth, NONE);
        // 100 °C per degree over 85.4 km per degree of longitude at 40°.
        let expected = 100.0 / (metres_per_degree(40.0).1 / 1e3);
        for line in rough.iter().chain(&smooth) {
            assert!(
                (line.value - expected).abs() < 5e-3 * expected,
                "{} {expected}",
                line.value
            );
        }

        // Noise of ±0.5 °C cell to cell on a 1 km grid.
        let mut seed: u64 = 0x2545_f491_4f6c_dd1d;
        let mut noise = || {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (seed >> 11) as f64 / (1u64 << 53) as f64 - 0.5
        };
        let lats = around(40.0, 0.01, 60);
        let lons = around(-72.0, 0.01, 60);
        let values: Vec<f32> = (0..lats.len() * lons.len())
            .map(|_| (16.0 + noise()) as f32)
            .collect();
        let speckled =
            Grid::new(source(0.01), UnixTime(0), lats, lons, values, "degree_C").unwrap();
        let options = |smooth| BreakOptions {
            smooth,
            min_length_m: 0.0,
            ..BreakOptions::new(BreakKind::Sst, SST_THRESHOLD, 0.25)
        };
        assert!(breaks_with(&speckled, BreakKind::Sst, &options(false)).len() > 10);
        assert_eq!(breaks_with(&speckled, BreakKind::Sst, &options(true)), NONE);
    }

    #[test]
    fn the_output_is_deterministic() {
        let grid = ring(12.0, |bearing| 1.25 + 8.75 * (1.0 + bearing.sin()) / 2.0);
        assert_eq!(sst(&grid, 0.5), sst(&grid, 0.5));
        let grid = chlorophyll_front(1.0);
        assert_eq!(
            breaks(&grid, BreakKind::Chlorophyll, CHL_THRESHOLD, 0.1),
            breaks(&grid, BreakKind::Chlorophyll, CHL_THRESHOLD, 0.1)
        );
    }

    #[test]
    fn bad_inputs_give_nothing_and_do_not_panic() {
        let grid = east_front(40.0, -72.0, STEP, L_KM);
        assert_eq!(breaks(&grid, BreakKind::Sst, f64::NAN, 1.0), NONE);
        for spacing in [0.0, -1.0, f64::NAN, f64::INFINITY, 1e-9] {
            assert_eq!(sst(&grid, spacing), NONE, "{spacing}");
        }
        let nan_length = BreakOptions {
            min_length_m: f64::NAN,
            ..BreakOptions::new(BreakKind::Sst, SST_THRESHOLD, 1.0)
        };
        assert_eq!(breaks_with(&grid, BreakKind::Sst, &nan_length), NONE);

        let small = build(around(40.0, STEP, 2), around(-72.0, STEP, 5), |_, lon| lon);
        assert_eq!(breaks(&small, BreakKind::Sst, 0.0, 0.001), NONE);
        let empty = build(around(40.0, STEP, 5), around(-72.0, STEP, 5), |_, _| {
            f64::NAN
        });
        assert_eq!(sst(&empty, 1.0), NONE);
        let infinite = build(around(40.0, STEP, 5), around(-72.0, STEP, 5), |_, _| {
            f64::INFINITY
        });
        assert_eq!(breaks(&infinite, BreakKind::Chlorophyll, 0.0, 1.0), NONE);

        let mut broken = grid.clone();
        broken.values.pop();
        assert_eq!(sst(&broken, 1.0), NONE);
        let mut broken = grid.clone();
        broken.lats.reverse();
        assert_eq!(sst(&broken, 1.0), NONE);
        let mut broken = grid.clone();
        broken.lons[3] = f64::NAN;
        assert_eq!(sst(&broken, 1.0), NONE);
        let mut broken = grid;
        broken.lats.truncate(2);
        assert_eq!(sst(&broken, 1.0), NONE);
    }

    fn fixture(name: &str) -> serde_json::Value {
        let path = format!(
            "{}/../../tests/fixtures/raw/erddap/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    /// The grid of an ERDDAP `.json` table whose last column holds the values.
    fn grid_from_table(table: &serde_json::Value, step: f64) -> Grid {
        let names = table["table"]["columnNames"].as_array().unwrap();
        let column = |name: &str| names.iter().position(|n| n == name).unwrap();
        let (lat_col, lon_col) = (column("latitude"), column("longitude"));
        let value_col = names.len() - 1;
        let rows = table["table"]["rows"].as_array().unwrap();
        let axis = |c: usize| {
            let mut a: Vec<f64> = rows.iter().map(|r| r[c].as_f64().unwrap()).collect();
            a.sort_by(f64::total_cmp);
            a.dedup();
            a
        };
        let (lats, lons) = (axis(lat_col), axis(lon_col));
        let mut values = vec![f32::NAN; lats.len() * lons.len()];
        for r in rows {
            let i = lats.binary_search_by(|a| a.total_cmp(&r[lat_col].as_f64().unwrap()));
            let j = lons.binary_search_by(|a| a.total_cmp(&r[lon_col].as_f64().unwrap()));
            if let Some(v) = r[value_col].as_f64() {
                values[i.unwrap() * lons.len() + j.unwrap()] = v as f32;
            }
        }
        let unit = table["table"]["columnUnits"][value_col].as_str().unwrap();
        Grid::new(
            source(step),
            UnixTime(1_794_042_000),
            lats,
            lons,
            values,
            unit,
        )
        .unwrap()
    }

    /// One row per piece: points, length, strength and ends, rounded.
    fn summary(lines: &[Polyline]) -> String {
        let mut out = format!("{} pieces\n", lines.len());
        for line in lines {
            let (a, b) = (line.points[0], *line.points.last().unwrap());
            writeln!(
                out,
                "  {:>3} points {:>5.1} km {:.3}/km {}  {:.4},{:.4} -> {:.4},{:.4}",
                line.points.len(),
                length_m(line) / 1e3,
                line.value,
                if is_closed(line) { "ring" } else { "line" },
                a.lat,
                a.lon,
                b.lat,
                b.lon,
            )
            .unwrap();
        }
        out
    }

    #[test]
    fn mur_fixture_breaks_snapshot() {
        // 0.05° MUR (strided by five), New York Bight, 2026-11-07; land is missing.
        let grid = grid_from_table(&fixture("jplmursst41-sst-subset-2026-11-07.json"), 0.01);
        let mut out = String::new();
        for (threshold, spacing) in [(SST_THRESHOLD, 0.5), (0.05, 0.25), (0.02, 0.25)] {
            let lines = breaks(&grid, BreakKind::Sst, threshold, spacing);
            assert_clear_of_missing(&grid, &lines, 1);
            assert!(lines.iter().all(|l| l.value >= threshold));
            write!(
                out,
                "threshold {threshold} °C/km, spacing {spacing} °C: {}",
                summary(&lines)
            )
            .unwrap();
        }
        insta::assert_snapshot!("mur_breaks", out);
    }

    #[test]
    fn viirs_chlorophyll_fixture_breaks_snapshot() {
        // 4 km VIIRS chlorophyll with cloud gaps, New York Bight, 2026-11-04.
        let grid = grid_from_table(
            &fixture("noaacwn20viirschladaily-chla-subset-2026-11-04.json"),
            0.0375,
        );
        let mut out = String::new();
        for (threshold, spacing) in [(CHL_THRESHOLD, 0.1), (0.01, 0.1)] {
            let lines = breaks(&grid, BreakKind::Chlorophyll, threshold, spacing);
            assert_clear_of_missing(&grid, &lines, 2);
            assert!(lines.iter().all(|l| l.value >= threshold));
            write!(
                out,
                "threshold {threshold} log10/km, spacing {spacing}: {}",
                summary(&lines)
            )
            .unwrap();
        }
        insta::assert_snapshot!("viirs_chlorophyll_breaks", out);
    }
}
