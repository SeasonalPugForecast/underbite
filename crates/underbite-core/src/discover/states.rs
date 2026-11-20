//! Which state a point is in, from simplified boundaries bundled with the app.
//!
//! The lookup is entirely local: the point never leaves the device. The boundaries are the
//! U.S. Census Bureau's 1:5,000,000 cartographic boundary file of the states, simplified to
//! about 150 m and stored as delta-encoded varints (see `tools/state_boundaries.py` for the
//! format and how the file is made). They follow the shore, so open water belongs to no state;
//! rivers and lakes that are borders run along the boundary line, so a point on the water
//! beside it is within a short distance of both states.
//!
//! [`states_at`] answers with every state a fishing point plausibly belongs to, nearest first:
//! the state that holds the point, then any state whose boundary is within
//! [`BORDER_TOLERANCE_M`]. A point at sea within [`COAST_REACH_M`] of a coast belongs to the
//! state of that coast, so near-shore salt water gets its marine regulations; farther out it
//! belongs to none.

use std::sync::OnceLock;

use super::hydro::Point;

/// A point this near a state's boundary is also in that state, metres. The simplified
/// boundaries are good to a few hundred metres; rivers that are borders are as wide.
pub const BORDER_TOLERANCE_M: f64 = 3_000.0;
/// A point at sea this near a state's coast belongs to that state, metres.
pub const COAST_REACH_M: f64 = 25_000.0;

/// The embedded boundaries.
const DATA: &[u8] = include_bytes!("../../data/states/us-states.bin");
const MAGIC: &[u8; 4] = b"USB1";

const M_PER_DEG_LAT: f64 = 110_574.0;
const M_PER_DEG_LON_EQ: f64 = 111_320.0;

/// A state's two-letter postal code (`NY`), or that of a district or territory.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StateCode([u8; 2]);

impl StateCode {
    /// The postal code, `NY`.
    pub fn as_str(&self) -> &str {
        // The codes are checked to be ASCII letters when the data is decoded.
        std::str::from_utf8(&self.0).unwrap_or("")
    }

    /// The ISO 3166-2 region of the regulations catalogue, `US-NY`.
    pub fn region(&self) -> String {
        format!("US-{}", self.as_str())
    }
}

#[derive(Debug)]
struct Ring {
    /// Bounding box in units: min x (lon), min y (lat), max x, max y.
    bbox: [i32; 4],
    /// The vertices in units; the ring closes from the last to the first.
    points: Vec<(i32, i32)>,
}

#[derive(Debug)]
struct Shape {
    code: StateCode,
    rings: Vec<Ring>,
}

#[derive(Debug)]
struct Boundaries {
    /// Units per degree.
    unit: f64,
    shapes: Vec<Shape>,
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn byte(&mut self) -> Option<u8> {
        let b = *self.data.get(self.pos)?;
        self.pos += 1;
        Some(b)
    }

    fn varint(&mut self) -> Option<u64> {
        let (mut value, mut shift) = (0u64, 0u32);
        loop {
            let b = self.byte()?;
            value |= u64::from(b & 0x7F).checked_shl(shift)?;
            if b & 0x80 == 0 {
                return Some(value);
            }
            shift += 7;
            if shift > 63 {
                return None;
            }
        }
    }

    fn zigzag(&mut self) -> Option<i64> {
        let n = self.varint()?;
        Some((n >> 1).cast_signed() ^ -((n & 1).cast_signed()))
    }
}

fn decode(data: &[u8]) -> Option<Boundaries> {
    if data.get(..4)? != MAGIC {
        return None;
    }
    let mut r = Reader { data, pos: 4 };
    let unit = r.varint()?;
    let states = r.varint()?;
    let mut shapes = Vec::new();
    for _ in 0..states {
        let code = [r.byte()?, r.byte()?];
        if !code.iter().all(u8::is_ascii_uppercase) {
            return None;
        }
        let mut rings = Vec::new();
        for _ in 0..r.varint()? {
            let n = r.varint()?;
            let mut points = Vec::with_capacity(usize::try_from(n).ok()?.min(1 << 20));
            let (mut x, mut y) = (0i64, 0i64);
            for _ in 0..n {
                x += r.zigzag()?;
                y += r.zigzag()?;
                points.push((i32::try_from(x).ok()?, i32::try_from(y).ok()?));
            }
            let first = *points.first()?;
            let mut bbox = [first.0, first.1, first.0, first.1];
            for &(x, y) in &points {
                bbox = [
                    bbox[0].min(x),
                    bbox[1].min(y),
                    bbox[2].max(x),
                    bbox[3].max(y),
                ];
            }
            rings.push(Ring { bbox, points });
        }
        shapes.push(Shape {
            code: StateCode(code),
            rings,
        });
    }
    Some(Boundaries {
        unit: unit as f64,
        shapes,
    })
}

fn boundaries() -> &'static Boundaries {
    static CELL: OnceLock<Boundaries> = OnceLock::new();
    CELL.get_or_init(|| {
        decode(DATA).unwrap_or(Boundaries {
            unit: 1.0,
            shapes: Vec::new(),
        })
    })
}

/// How a point stands against one state.
struct Standing {
    inside: bool,
    /// Distance to the nearest boundary, metres.
    distance_m: f64,
}

/// The standing of `point` against `shape`, or `None` when no ring of it is within `reach_m`
/// (by bounding box). Distances are metres on a plane exact at the point.
fn standing(b: &Boundaries, shape: &Shape, point: Point, reach_m: f64) -> Option<Standing> {
    let m_per_deg_lon = M_PER_DEG_LON_EQ * point.latitude.to_radians().cos().max(0.01);
    // The search box and the point, in units.
    let dlat = reach_m / M_PER_DEG_LAT * b.unit;
    let dlon = reach_m / m_per_deg_lon * b.unit;
    let (px, py) = (point.longitude * b.unit, point.latitude * b.unit);
    let to_m = |(x, y): (i32, i32)| {
        (
            (f64::from(x) - px) / b.unit * m_per_deg_lon,
            (f64::from(y) - py) / b.unit * M_PER_DEG_LAT,
        )
    };
    let mut crossings = 0u32;
    let mut best = f64::INFINITY;
    let mut near = false;
    for ring in &shape.rings {
        let [x0, y0, x1, y1] = ring.bbox.map(f64::from);
        if px < x0 - dlon || px > x1 + dlon || py < y0 - dlat || py > y1 + dlat {
            continue;
        }
        near = true;
        let Some(&last) = ring.points.last() else {
            continue;
        };
        let mut prev = to_m(last);
        for &p in &ring.points {
            let cur = to_m(p);
            let ((ax, ay), (bx, by)) = (prev, cur);
            // Even-odd rule: a ray from the point (the origin) towards +x.
            if (ay > 0.0) != (by > 0.0) && ax + (bx - ax) * (0.0 - ay) / (by - ay) > 0.0 {
                crossings += 1;
            }
            let (dx, dy) = (bx - ax, by - ay);
            let len2 = dx * dx + dy * dy;
            let t = if len2 == 0.0 {
                0.0
            } else {
                ((-ax * dx - ay * dy) / len2).clamp(0.0, 1.0)
            };
            let (cx, cy) = (ax + t * dx, ay + t * dy);
            best = best.min(cx.hypot(cy));
            prev = cur;
        }
    }
    near.then_some(Standing {
        inside: crossings % 2 == 1,
        distance_m: best,
    })
}

/// The states `point` is in or beside, nearest first; none when it is not in the United States
/// or its territories nor within [`COAST_REACH_M`] of their shores.
///
/// - The state that holds the point comes first, then every state whose boundary is within
///   [`BORDER_TOLERANCE_M`], nearest first.
/// - A point held by no state (water off the shore) gives the state of the nearest coast within
///   [`COAST_REACH_M`], then any other state whose coast is within [`BORDER_TOLERANCE_M`] farther.
/// - Equal distances order by postal code, so the answer is deterministic.
pub fn states_at(point: Point) -> Vec<StateCode> {
    if !(point.latitude.is_finite() && point.longitude.is_finite()) {
        return Vec::new();
    }
    let b = boundaries();
    let mut found: Vec<(bool, f64, StateCode)> = b
        .shapes
        .iter()
        .filter_map(|s| {
            standing(b, s, point, COAST_REACH_M).map(|st| (st.inside, st.distance_m, s.code))
        })
        .collect();
    found.sort_by(|a, c| c.0.cmp(&a.0).then(a.1.total_cmp(&c.1)).then(a.2.cmp(&c.2)));
    let held = found.first().is_some_and(|f| f.0);
    let nearest = found.first().map_or(0.0, |f| f.1);
    found
        .into_iter()
        .filter(|&(inside, d, _)| {
            if held {
                inside || d <= BORDER_TOLERANCE_M
            } else {
                d <= COAST_REACH_M && d <= nearest + BORDER_TOLERANCE_M
            }
        })
        .map(|(_, _, code)| code)
        .collect()
}

/// The size of the embedded boundaries, bytes.
pub const fn data_len() -> usize {
    DATA.len()
}

/// The postal codes the embedded boundaries hold a ring for, in order.
pub fn covered() -> Vec<StateCode> {
    boundaries()
        .shapes
        .iter()
        .filter(|s| !s.rings.is_empty())
        .map(|s| s.code)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(lat: f64, lon: f64) -> Vec<String> {
        states_at(Point::new(lat, lon))
            .iter()
            .map(|c| c.as_str().to_owned())
            .collect()
    }

    fn has(got: &[String], codes: &[&str]) -> bool {
        codes.iter().all(|c| got.iter().any(|g| g == c))
    }

    #[test]
    fn the_battery_is_in_new_york_and_beside_new_jersey() {
        assert_eq!(at(40.7033, -74.0170), ["NY", "NJ"]);
    }

    #[test]
    fn the_croton_river_is_in_new_york_only() {
        assert_eq!(at(41.2107, -73.8590), ["NY"]);
    }

    #[test]
    fn the_delaware_at_easton_is_pennsylvania_and_new_jersey() {
        let got = at(40.6890, -75.2050);
        assert_eq!(got.len(), 2, "{got:?}");
        assert!(has(&got, &["PA", "NJ"]), "{got:?}");
    }

    #[test]
    fn open_ocean_far_off_new_jersey_is_in_no_state() {
        // About 100 km east of Sandy Hook.
        assert_eq!(at(40.40, -72.90), Vec::<String>::new());
    }

    #[test]
    fn near_shore_salt_water_has_the_coast_state() {
        // About 10 km off Montauk Point's south shore.
        assert_eq!(at(40.95, -71.90).first().map(String::as_str), Some("NY"));
    }

    #[test]
    fn lake_tahoe_is_california_and_nevada() {
        let got = at(39.0968, -120.0324);
        assert!(has(&got, &["CA", "NV"]), "{got:?}");
    }

    #[test]
    fn lake_champlain_is_new_york_and_vermont() {
        let got = at(44.5, -73.3);
        assert!(has(&got, &["NY", "VT"]), "{got:?}");
    }

    #[test]
    fn the_columbia_at_portland_is_oregon_and_washington() {
        let got = at(45.62, -122.68);
        assert!(has(&got, &["OR", "WA"]), "{got:?}");
    }

    #[test]
    fn the_answer_is_deterministic_and_foreign_points_are_in_no_state() {
        assert_eq!(at(40.7033, -74.0170), at(40.7033, -74.0170));
        assert_eq!(
            states_at(Point::new(f64::NAN, 0.0)),
            Vec::<StateCode>::new()
        );
        // Paris.
        assert_eq!(at(48.85, 2.35), Vec::<String>::new());
    }

    #[test]
    fn every_state_has_a_ring_and_the_data_is_within_budget() {
        let have: Vec<String> = covered().iter().map(|c| c.as_str().to_owned()).collect();
        let want = "AK AL AR AS AZ CA CO CT DC DE FL GA GU HI IA ID IL IN KS KY LA MA MD ME MI MN \
                    MO MP MS MT NC ND NE NH NJ NM NV NY OH OK OR PA PR RI SC SD TN TX UT VA VI VT \
                    WA WI WV WY";
        for code in want.split(' ') {
            assert!(have.iter().any(|h| h == code), "{code} has no ring");
        }
        assert_eq!(have.len(), 56);
        assert!(data_len() <= 300_000, "{} bytes", data_len());
    }

    #[test]
    fn every_covered_state_has_regulation_links() {
        for c in covered() {
            assert!(
                !crate::regulations::all_links(&c.region()).is_empty(),
                "{}",
                c.region()
            );
        }
    }
}
