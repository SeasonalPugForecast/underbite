//! A low-precision lunar ephemeris: position, phase, rise, set and transits.
//!
//! Everything follows J. Meeus, *Astronomical Algorithms*, 2nd ed. (1998):
//!
//! - ch. 47: the moon's geocentric longitude, latitude and distance from the periodic terms of
//!   Tables 47.A and 47.B, all 60 rows of each (Meeus's own truncation of ELP-2000/82, about 10″
//!   in longitude and 4″ in latitude), plus the additive terms of A1, A2 and A3;
//! - ch. 22: nutation by the four-term approximation (0.5″ in Δψ, 0.1″ in Δε) and the IAU 1980
//!   mean obliquity; ch. 12: Greenwich mean sidereal time (IAU 1982);
//! - ch. 25: the sun's apparent longitude and distance at low accuracy (0.01°);
//! - ch. 40 (the "alternative" method) and ch. 11: topocentric hour angle and declination;
//! - ch. 15: the moon rises or sets when its geocentric centre is at
//!   `h0 = 0.7275 π − 0°34′`, π its horizontal parallax and 34′ the standard refraction (the upper
//!   limb on the apparent horizon); it transits when its hour angle is 0° (upper) or 180° (lower);
//! - ch. 48: the illuminated fraction from the phase angle.
//!
//! Principal phases are the instants when the apparent longitudes of moon and sun differ by 0°,
//! 90°, 180° or 270°, found as roots rather than from the series of ch. 49.
//!
//! ΔT (TT − UT) comes from the Espenak and Meeus polynomial for 2005–2050 (NASA, "Polynomial
//! Expressions for Delta T"): `ΔT = 62.92 + 0.32217 t + 0.005589 t²` seconds, `t` years after
//! 2000. UT1 is taken as UTC (within a second). No function reads the clock.
//!
//! Events are found by sampling a window and refining each sign change by bisection to the second,
//! so a day can lack a rise or a set, as the moon rises about 50 minutes later each day.

use crate::model::{MoonDay, MoonPhase, PrincipalPhase, PrincipalPhaseTime};
use crate::time::UnixTime;

const UNIX_EPOCH_JD: f64 = 2_440_587.5;
const J2000: f64 = 2_451_545.0;
const DAY_S: i64 = 86_400;
/// Equatorial radius of the Earth (Meeus's IAU 1976 value), km.
const EARTH_RADIUS_KM: f64 = 6_378.14;
/// Flattening of the IAU 1976 ellipsoid.
const EARTH_FLATTENING: f64 = 1.0 / 298.257;
const AU_KM: f64 = 149_597_870.7;
/// Standard refraction at the horizon, degrees.
const REFRACTION_DEG: f64 = 34.0 / 60.0;
/// Sampling step for rise, set and transits: the moon's hour angle moves about 7° in it.
const EVENT_STEP_S: i64 = 20 * 60;
/// Sampling step for principal phases: the elongation moves about 3° in it.
const PHASE_STEP_S: i64 = 6 * 3_600;

/// The moon's apparent geocentric position, referred to the true equator and equinox of date.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MoonPosition {
    /// Apparent ecliptic longitude, degrees in 0..360.
    pub longitude_deg: f64,
    /// Ecliptic latitude, degrees.
    pub latitude_deg: f64,
    /// Apparent right ascension, degrees in 0..360.
    pub right_ascension_deg: f64,
    /// Apparent declination, degrees.
    pub declination_deg: f64,
    /// Distance between the centres of the Earth and the moon, metres.
    pub distance_m: f64,
    /// Equatorial horizontal parallax, degrees.
    pub horizontal_parallax_deg: f64,
}

/// What happens to the moon at a [`MoonEvent`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MoonEventKind {
    /// The upper limb rises above the horizon.
    Rise,
    /// The upper limb sets below the horizon.
    Set,
    /// The centre crosses the meridian above the pole (hour angle 0°).
    UpperTransit,
    /// The centre crosses the meridian below the pole (hour angle 180°).
    LowerTransit,
}

/// A rise, set or transit of the moon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MoonEvent {
    pub kind: MoonEventKind,
    /// The instant, to the second.
    pub time: UnixTime,
}

fn sin_d(x: f64) -> f64 {
    x.to_radians().sin()
}

fn cos_d(x: f64) -> f64 {
    x.to_radians().cos()
}

/// `x` in 0..360.
fn rev(x: f64) -> f64 {
    x.rem_euclid(360.0)
}

/// `x` in -180..180.
fn wrap180(x: f64) -> f64 {
    let r = rev(x);
    if r >= 180.0 { r - 360.0 } else { r }
}

/// `c[0] + c[1] t + c[2] t² + …`
fn poly(t: f64, c: &[f64]) -> f64 {
    c.iter().rev().fold(0.0, |acc, &k| acc * t + k)
}

/// The Julian day (UT) of an instant.
fn julian_day(t: UnixTime) -> f64 {
    t.0 as f64 / DAY_S as f64 + UNIX_EPOCH_JD
}

/// ΔT in seconds, Espenak and Meeus 2005–2050.
fn delta_t_s(jd_ut: f64) -> f64 {
    let years = (jd_ut - J2000) / 365.25;
    poly(years, &[62.92, 0.322_17, 0.005_589])
}

/// Julian centuries of Terrestrial Time since J2000.0.
fn centuries_tt(jd_ut: f64) -> f64 {
    let jde = jd_ut + delta_t_s(jd_ut) / DAY_S as f64;
    (jde - J2000) / 36_525.0
}

/// Geocentric ecliptic coordinates, mean equinox of date.
#[derive(Clone, Copy, Debug)]
struct Ecliptic {
    lon_deg: f64,
    lat_deg: f64,
    distance_km: f64,
}

/// Meeus ch. 47, `t` in Julian centuries TT since J2000.0.
fn moon_ecliptic(t: f64) -> Ecliptic {
    let mean_lon = poly(
        t,
        &[
            218.316_447_7,
            481_267.881_234_21,
            -0.001_578_6,
            1.0 / 538_841.0,
            -1.0 / 65_194_000.0,
        ],
    );
    let elong = poly(
        t,
        &[
            297.850_192_1,
            445_267.111_403_4,
            -0.001_881_9,
            1.0 / 545_868.0,
            -1.0 / 113_065_000.0,
        ],
    );
    let sun_anom = poly(
        t,
        &[
            357.529_109_2,
            35_999.050_290_9,
            -0.000_153_5,
            1.0 / 24_490_000.0,
        ],
    );
    let moon_anom = poly(
        t,
        &[
            134.963_396_4,
            477_198.867_505_5,
            0.008_741_4,
            1.0 / 69_699.0,
            -1.0 / 14_712_000.0,
        ],
    );
    let node_arg = poly(
        t,
        &[
            93.272_095,
            483_202.017_523_3,
            -0.003_653_9,
            -1.0 / 3_526_000.0,
            1.0 / 863_310_000.0,
        ],
    );
    let a1 = 119.75 + 131.849 * t;
    let a2 = 53.09 + 479_264.29 * t;
    let a3 = 313.45 + 481_266.484 * t;
    let e = poly(t, &[1.0, -0.002_516, -0.000_007_4]);
    let eccentricity = |m: i8| match m.unsigned_abs() {
        0 => 1.0,
        1 => e,
        _ => e * e,
    };
    let argument = |d: i8, m: i8, mp: i8, f: i8| {
        f64::from(d) * elong
            + f64::from(m) * sun_anom
            + f64::from(mp) * moon_anom
            + f64::from(f) * node_arg
    };

    let mut sum_l = 3_958.0 * sin_d(a1) + 1_962.0 * sin_d(mean_lon - node_arg) + 318.0 * sin_d(a2);
    let mut sum_r = 0.0;
    for &(d, m, mp, f, l, r) in &TABLE_47A {
        let arg = argument(d, m, mp, f);
        let k = eccentricity(m);
        sum_l += f64::from(l) * k * sin_d(arg);
        sum_r += f64::from(r) * k * cos_d(arg);
    }
    let mut sum_b = -2_235.0 * sin_d(mean_lon)
        + 382.0 * sin_d(a3)
        + 175.0 * sin_d(a1 - node_arg)
        + 175.0 * sin_d(a1 + node_arg)
        + 127.0 * sin_d(mean_lon - moon_anom)
        - 115.0 * sin_d(mean_lon + moon_anom);
    for &(d, m, mp, f, b) in &TABLE_47B {
        sum_b += f64::from(b) * eccentricity(m) * sin_d(argument(d, m, mp, f));
    }
    Ecliptic {
        lon_deg: rev(mean_lon + sum_l / 1e6),
        lat_deg: sum_b / 1e6,
        distance_km: 385_000.56 + sum_r / 1e3,
    }
}

/// Nutation in longitude and obliquity, degrees (Meeus ch. 22, four-term approximation).
fn nutation(t: f64) -> (f64, f64) {
    let node = 125.044_52 - 1_934.136_261 * t;
    let sun = 280.466_5 + 36_000.769_8 * t;
    let moon = 218.316_5 + 481_267.881_3 * t;
    let dpsi = -17.2 * sin_d(node) - 1.32 * sin_d(2.0 * sun) - 0.23 * sin_d(2.0 * moon)
        + 0.21 * sin_d(2.0 * node);
    let deps = 9.2 * cos_d(node) + 0.57 * cos_d(2.0 * sun) + 0.1 * cos_d(2.0 * moon)
        - 0.09 * cos_d(2.0 * node);
    (dpsi / 3_600.0, deps / 3_600.0)
}

/// Mean obliquity of the ecliptic, degrees (IAU 1980, Meeus 22.2).
fn mean_obliquity(t: f64) -> f64 {
    poly(t, &[84_381.448, -46.815, -0.000_59, 0.001_813]) / 3_600.0
}

/// Greenwich mean sidereal time, degrees (IAU 1982, Meeus ch. 12).
fn mean_sidereal_deg(jd_ut: f64) -> f64 {
    let jd0 = (jd_ut - 0.5).floor() + 0.5;
    let t0 = (jd0 - J2000) / 36_525.0;
    let seconds = poly(
        t0,
        &[24_110.548_41, 8_640_184.812_866, 0.093_104, 0.000_006_2],
    ) + (jd_ut - jd0) * DAY_S as f64 * 1.002_737_909_35;
    rev(seconds / 240.0)
}

/// The sun's apparent longitude (degrees) and distance (km), Meeus ch. 25 low accuracy.
fn sun_apparent(t: f64) -> (f64, f64) {
    let mean_lon = poly(t, &[280.466_46, 36_000.769_83, 0.000_303_2]);
    let anomaly = poly(t, &[357.529_11, 35_999.050_29, -0.000_153_7]);
    let center = poly(t, &[1.914_602, -0.004_817, -0.000_014]) * sin_d(anomaly)
        + (0.019_993 - 0.000_101 * t) * sin_d(2.0 * anomaly)
        + 0.000_289 * sin_d(3.0 * anomaly);
    let e = poly(t, &[0.016_708_634, -0.000_042_037, -0.000_000_126_7]);
    let radius_au = 1.000_001_018 * (1.0 - e * e) / (1.0 + e * cos_d(anomaly + center));
    let node = 125.04 - 1_934.136 * t;
    let lon = rev(mean_lon + center - 0.005_69 - 0.004_78 * sin_d(node));
    (lon, radius_au * AU_KM)
}

/// The moon's apparent place and the apparent sidereal time at Greenwich.
#[derive(Clone, Copy, Debug)]
struct Sky {
    moon: MoonPosition,
    sidereal_deg: f64,
}

fn sky(t: UnixTime) -> Sky {
    let jd_ut = julian_day(t);
    let tc = centuries_tt(jd_ut);
    let ecl = moon_ecliptic(tc);
    let (dpsi, deps) = nutation(tc);
    let eps = mean_obliquity(tc) + deps;
    let lon = rev(ecl.lon_deg + dpsi);
    // Rotate the ecliptic unit vector about the x axis by the obliquity.
    let (x, y, z) = (
        cos_d(ecl.lat_deg) * cos_d(lon),
        cos_d(ecl.lat_deg) * sin_d(lon),
        sin_d(ecl.lat_deg),
    );
    let (y_eq, z_eq) = (
        y * cos_d(eps) - z * sin_d(eps),
        y * sin_d(eps) + z * cos_d(eps),
    );
    Sky {
        moon: MoonPosition {
            longitude_deg: lon,
            latitude_deg: ecl.lat_deg,
            right_ascension_deg: rev(y_eq.atan2(x).to_degrees()),
            declination_deg: z_eq.clamp(-1.0, 1.0).asin().to_degrees(),
            distance_m: ecl.distance_km * 1e3,
            horizontal_parallax_deg: (EARTH_RADIUS_KM / ecl.distance_km).asin().to_degrees(),
        },
        sidereal_deg: rev(mean_sidereal_deg(jd_ut) + dpsi * cos_d(eps)),
    }
}

impl Sky {
    /// Local hour angle, degrees in -180..180, longitude east positive.
    fn hour_angle_deg(&self, lon: f64) -> f64 {
        wrap180(self.sidereal_deg + lon - self.moon.right_ascension_deg)
    }

    /// Geocentric altitude of the centre, degrees.
    fn altitude_deg(&self, lat: f64, lon: f64) -> f64 {
        let dec = self.moon.declination_deg;
        let sin_h =
            sin_d(lat) * sin_d(dec) + cos_d(lat) * cos_d(dec) * cos_d(self.hour_angle_deg(lon));
        sin_h.clamp(-1.0, 1.0).asin().to_degrees()
    }

    /// Geocentric altitude above the rise and set altitude `h0` (Meeus ch. 15).
    fn above_horizon_deg(&self, lat: f64, lon: f64) -> f64 {
        let h0 = 0.7275 * self.moon.horizontal_parallax_deg - REFRACTION_DEG;
        self.altitude_deg(lat, lon) - h0
    }
}

/// The moon's apparent geocentric position at `t`.
pub fn moon_position(t: UnixTime) -> MoonPosition {
    sky(t).moon
}

/// The moon centre's topocentric altitude at `t` for an observer at sea level, degrees, without
/// refraction (Meeus ch. 40 and ch. 11).
pub fn topocentric_altitude_deg(t: UnixTime, lat: f64, lon: f64) -> f64 {
    let now = sky(t);
    let dec = now.moon.declination_deg;
    let hour = now.hour_angle_deg(lon);
    let ratio = 1.0 - EARTH_FLATTENING;
    let reduced_lat = (ratio * lat.to_radians().tan()).atan();
    let (rho_sin, rho_cos) = (ratio * reduced_lat.sin(), reduced_lat.cos());
    let sin_par = sin_d(now.moon.horizontal_parallax_deg);
    // Meeus 40.6 and 40.7 (A, B, C).
    let term_a = cos_d(dec) * sin_d(hour);
    let term_b = cos_d(dec) * cos_d(hour) - rho_cos * sin_par;
    let term_c = sin_d(dec) - rho_sin * sin_par;
    let topo_hour = term_a.atan2(term_b);
    let norm = (term_a * term_a + term_b * term_b + term_c * term_c).sqrt();
    let topo_dec = (term_c / norm).asin();
    let sin_h = sin_d(lat) * topo_dec.sin() + cos_d(lat) * topo_dec.cos() * topo_hour.cos();
    sin_h.clamp(-1.0, 1.0).asin().to_degrees()
}

/// The instants in `[from, to)` where `f` changes sign, each with whether it goes up. Steps are
/// `step` seconds; jumps of more than 180 (an angle wrapping around) are not roots.
fn roots(
    from: UnixTime,
    to: UnixTime,
    step: i64,
    f: impl Fn(UnixTime) -> f64,
) -> Vec<(UnixTime, bool)> {
    let mut found = Vec::new();
    if to <= from {
        return found;
    }
    let mut t1 = from;
    let mut f1 = f(t1);
    while t1 < to {
        let t2 = UnixTime((t1.0 + step).min(to.0));
        let f2 = f(t2);
        if (f1 < 0.0) != (f2 < 0.0) && (f2 - f1).abs() < 180.0 {
            let rising = f1 < 0.0;
            let (mut lo, mut hi) = (t1.0, t2.0);
            while hi - lo > 1 {
                let mid = lo + (hi - lo) / 2;
                if (f(UnixTime(mid)) < 0.0) == rising {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            // Of the two seconds around the root, the one where `f` is nearer zero.
            let (flo, fhi) = (f(UnixTime(lo)), f(UnixTime(hi)));
            let root = if flo.abs() <= fhi.abs() { lo } else { hi };
            if root < to.0 {
                found.push((UnixTime(root), rising));
            }
        }
        t1 = t2;
        f1 = f2;
    }
    found
}

/// Rises, sets and both transits of the moon in `[from, to)` at `lat`, `lon` (degrees, east
/// positive), in time order. Kinds that do not happen in the window are absent.
pub fn moon_events(lat: f64, lon: f64, from: UnixTime, to: UnixTime) -> Vec<MoonEvent> {
    let mut events: Vec<MoonEvent> = roots(from, to, EVENT_STEP_S, |t| {
        sky(t).above_horizon_deg(lat, lon)
    })
    .into_iter()
    .map(|(time, rising)| MoonEvent {
        kind: if rising {
            MoonEventKind::Rise
        } else {
            MoonEventKind::Set
        },
        time,
    })
    .collect();
    for (kind, offset) in [
        (MoonEventKind::UpperTransit, 0.0),
        (MoonEventKind::LowerTransit, 180.0),
    ] {
        let crossings = roots(from, to, EVENT_STEP_S, |t| {
            wrap180(sky(t).hour_angle_deg(lon) - offset)
        });
        events.extend(
            crossings
                .into_iter()
                .filter(|&(_, rising)| rising)
                .map(|(time, _)| MoonEvent { kind, time }),
        );
    }
    events.sort_by_key(|e| (e.time, e.kind));
    events
}

/// Apparent longitudes of the moon and sun, and the moon's ecliptic latitude and distance and the
/// sun's distance, at Julian centuries TT `t`.
fn moon_and_sun(t: f64) -> (Ecliptic, f64, f64) {
    let moon = moon_ecliptic(t);
    let (dpsi, _) = nutation(t);
    let (sun_lon, sun_km) = sun_apparent(t);
    (
        Ecliptic {
            lon_deg: rev(moon.lon_deg + dpsi),
            ..moon
        },
        sun_lon,
        sun_km,
    )
}

/// The moon's apparent longitude minus the sun's at `t`, degrees in 0..360: 0 at new moon, 90 at
/// first quarter, 180 at full and 270 at last quarter.
pub fn elongation_deg(t: UnixTime) -> f64 {
    let (moon, sun_lon, _) = moon_and_sun(centuries_tt(julian_day(t)));
    rev(moon.lon_deg - sun_lon)
}

/// The astronomical phase angle (Sun–Moon–Earth), degrees, 0 at full moon, Meeus 48.2 and 48.3.
fn phase_angle_sme_deg(t: f64) -> f64 {
    let (moon, sun_lon, sun_km) = moon_and_sun(t);
    let cos_psi = cos_d(moon.lat_deg) * cos_d(moon.lon_deg - sun_lon);
    let sin_psi = (1.0 - cos_psi * cos_psi).max(0.0).sqrt();
    (sun_km * sin_psi)
        .atan2(moon.distance_km - sun_km * cos_psi)
        .to_degrees()
}

/// The illuminated fraction of the moon's disc at `t`, 0 to 1 (Meeus 48.1).
pub fn illuminated_fraction(t: UnixTime) -> f64 {
    let i = phase_angle_sme_deg(centuries_tt(julian_day(t)));
    f64::midpoint(1.0, cos_d(i))
}

/// New moons, quarters and full moons in `[from, to)`, in time order.
pub fn principal_phases(from: UnixTime, to: UnixTime) -> Vec<PrincipalPhaseTime> {
    let mut phases = Vec::new();
    for (phase, target) in [
        (PrincipalPhase::New, 0.0),
        (PrincipalPhase::FirstQuarter, 90.0),
        (PrincipalPhase::Full, 180.0),
        (PrincipalPhase::LastQuarter, 270.0),
    ] {
        let crossings = roots(from, to, PHASE_STEP_S, |t| {
            wrap180(elongation_deg(t) - target)
        });
        phases.extend(
            crossings
                .into_iter()
                .filter(|&(_, rising)| rising)
                .map(|(time, _)| PrincipalPhaseTime { phase, time }),
        );
    }
    phases.sort_by_key(|p| p.time);
    phases
}

/// The phase name between principal phases, from the elongation.
fn intermediate_phase(elongation: f64) -> MoonPhase {
    match rev(elongation) {
        e if e < 90.0 => MoonPhase::WaxingCrescent,
        e if e < 180.0 => MoonPhase::WaxingGibbous,
        e if e < 270.0 => MoonPhase::WaningGibbous,
        _ => MoonPhase::WaningCrescent,
    }
}

fn principal_name(phase: PrincipalPhase) -> MoonPhase {
    match phase {
        PrincipalPhase::New => MoonPhase::New,
        PrincipalPhase::FirstQuarter => MoonPhase::FirstQuarter,
        PrincipalPhase::Full => MoonPhase::Full,
        PrincipalPhase::LastQuarter => MoonPhase::LastQuarter,
    }
}

/// The moon over the window `[from, to)` (a day, in UTC or local time): the first rise, set and
/// transits in it; elongation and illuminated fraction at its middle; the name of a principal
/// phase that falls inside it, otherwise the intermediate name; and the principal phase nearest to
/// its middle.
pub fn moon_day(lat: f64, lon: f64, from: UnixTime, to: UnixTime) -> MoonDay {
    let events = moon_events(lat, lon, from, to);
    let first = |kind| events.iter().find(|e| e.kind == kind).map(|e| e.time);
    let middle = UnixTime(from.0 + (to.0 - from.0) / 2);
    let elongation = elongation_deg(middle);
    let phase = principal_phases(from, to).first().map_or_else(
        || intermediate_phase(elongation),
        |p| principal_name(p.phase),
    );
    // Principal phases are at most about 8 days apart.
    let nearest_principal = principal_phases(
        UnixTime(middle.0 - 10 * DAY_S),
        UnixTime(middle.0 + 10 * DAY_S),
    )
    .into_iter()
    .min_by_key(|p| (p.time.0 - middle.0).abs());
    MoonDay {
        rise: first(MoonEventKind::Rise),
        set: first(MoonEventKind::Set),
        upper_transit: first(MoonEventKind::UpperTransit),
        lower_transit: first(MoonEventKind::LowerTransit),
        elongation_deg: elongation as f32,
        illuminated_fraction: illuminated_fraction(middle) as f32,
        phase,
        nearest_principal,
    }
}

// Tables 47.A and 47.B of Meeus, transcribed from soniakeys/meeus (moonposition.go), which carries
// this notice:
//
// Copyright 2013 Sonia Keys. License: MIT. Permission is hereby granted, free of charge, to any
// person obtaining a copy of this software and associated documentation files (the "Software"),
// to deal in the Software without restriction, including without limitation the rights to use,
// copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to
// permit persons to whom the Software is furnished to do so, subject to the following conditions:
// The above copyright notice and this permission notice shall be included in all copies or
// substantial portions of the Software. THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY
// KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS
// FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS
// BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR
// OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS
// IN THE SOFTWARE.

/// Table 47.A: multiples of D, M, M′, F; Σl (1e-6 degree) and Σr (1e-3 km).
#[rustfmt::skip]
const TABLE_47A: [(i8, i8, i8, i8, i32, i32); 60] = [
    (0, 0, 1, 0, 6_288_774, -20_905_355),
    (2, 0, -1, 0, 1_274_027, -3_699_111),
    (2, 0, 0, 0, 658_314, -2_955_968),
    (0, 0, 2, 0, 213_618, -569_925),
    (0, 1, 0, 0, -185_116, 48_888),
    (0, 0, 0, 2, -114_332, -3_149),
    (2, 0, -2, 0, 58_793, 246_158),
    (2, -1, -1, 0, 57_066, -152_138),
    (2, 0, 1, 0, 53_322, -170_733),
    (2, -1, 0, 0, 45_758, -204_586),
    (0, 1, -1, 0, -40_923, -129_620),
    (1, 0, 0, 0, -34_720, 108_743),
    (0, 1, 1, 0, -30_383, 104_755),
    (2, 0, 0, -2, 15_327, 10_321),
    (0, 0, 1, 2, -12_528, 0),
    (0, 0, 1, -2, 10_980, 79_661),
    (4, 0, -1, 0, 10_675, -34_782),
    (0, 0, 3, 0, 10_034, -23_210),
    (4, 0, -2, 0, 8_548, -21_636),
    (2, 1, -1, 0, -7_888, 24_208),
    (2, 1, 0, 0, -6_766, 30_824),
    (1, 0, -1, 0, -5_163, -8_379),
    (1, 1, 0, 0, 4_987, -16_675),
    (2, -1, 1, 0, 4_036, -12_831),
    (2, 0, 2, 0, 3_994, -10_445),
    (4, 0, 0, 0, 3_861, -11_650),
    (2, 0, -3, 0, 3_665, 14_403),
    (0, 1, -2, 0, -2_689, -7_003),
    (2, 0, -1, 2, -2_602, 0),
    (2, -1, -2, 0, 2_390, 10_056),
    (1, 0, 1, 0, -2_348, 6_322),
    (2, -2, 0, 0, 2_236, -9_884),
    (0, 1, 2, 0, -2_120, 5_751),
    (0, 2, 0, 0, -2_069, 0),
    (2, -2, -1, 0, 2_048, -4_950),
    (2, 0, 1, -2, -1_773, 4_130),
    (2, 0, 0, 2, -1_595, 0),
    (4, -1, -1, 0, 1_215, -3_958),
    (0, 0, 2, 2, -1_110, 0),
    (3, 0, -1, 0, -892, 3_258),
    (2, 1, 1, 0, -810, 2_616),
    (4, -1, -2, 0, 759, -1_897),
    (0, 2, -1, 0, -713, -2_117),
    (2, 2, -1, 0, -700, 2_354),
    (2, 1, -2, 0, 691, 0),
    (2, -1, 0, -2, 596, 0),
    (4, 0, 1, 0, 549, -1_423),
    (0, 0, 4, 0, 537, -1_117),
    (4, -1, 0, 0, 520, -1_571),
    (1, 0, -2, 0, -487, -1_739),
    (2, 1, 0, -2, -399, 0),
    (0, 0, 2, -2, -381, -4_421),
    (1, 1, 1, 0, 351, 0),
    (3, 0, -2, 0, -340, 0),
    (4, 0, -3, 0, 330, 0),
    (2, -1, 2, 0, 327, 0),
    (0, 2, 1, 0, -323, 1_165),
    (1, 1, -1, 0, 299, 0),
    (2, 0, 3, 0, 294, 0),
    (2, 0, -1, -2, 0, 8_752),
];

/// Table 47.B: multiples of D, M, M′, F; Σb (1e-6 degree).
#[rustfmt::skip]
const TABLE_47B: [(i8, i8, i8, i8, i32); 60] = [
    (0, 0, 0, 1, 5_128_122),
    (0, 0, 1, 1, 280_602),
    (0, 0, 1, -1, 277_693),
    (2, 0, 0, -1, 173_237),
    (2, 0, -1, 1, 55_413),
    (2, 0, -1, -1, 46_271),
    (2, 0, 0, 1, 32_573),
    (0, 0, 2, 1, 17_198),
    (2, 0, 1, -1, 9_266),
    (0, 0, 2, -1, 8_822),
    (2, -1, 0, -1, 8_216),
    (2, 0, -2, -1, 4_324),
    (2, 0, 1, 1, 4_200),
    (2, 1, 0, -1, -3_359),
    (2, -1, -1, 1, 2_463),
    (2, -1, 0, 1, 2_211),
    (2, -1, -1, -1, 2_065),
    (0, 1, -1, -1, -1_870),
    (4, 0, -1, -1, 1_828),
    (0, 1, 0, 1, -1_794),
    (0, 0, 0, 3, -1_749),
    (0, 1, -1, 1, -1_565),
    (1, 0, 0, 1, -1_491),
    (0, 1, 1, 1, -1_475),
    (0, 1, 1, -1, -1_410),
    (0, 1, 0, -1, -1_344),
    (1, 0, 0, -1, -1_335),
    (0, 0, 3, 1, 1_107),
    (4, 0, 0, -1, 1_021),
    (4, 0, -1, 1, 833),
    (0, 0, 1, -3, 777),
    (4, 0, -2, 1, 671),
    (2, 0, 0, -3, 607),
    (2, 0, 2, -1, 596),
    (2, -1, 1, -1, 491),
    (2, 0, -2, 1, -451),
    (0, 0, 3, -1, 439),
    (2, 0, 2, 1, 422),
    (2, 0, -3, -1, 421),
    (2, 1, -1, 1, -366),
    (2, 1, 0, 1, -351),
    (4, 0, 0, 1, 331),
    (2, -1, 1, 1, 315),
    (2, -2, 0, -1, 302),
    (0, 0, 1, 3, -283),
    (2, 1, 1, -1, -229),
    (1, 1, 0, -1, 223),
    (1, 1, 0, 1, 223),
    (0, 1, -2, -1, -220),
    (2, 1, -1, -1, -220),
    (1, 0, 1, 1, -185),
    (2, -1, -2, -1, 181),
    (0, 1, 2, 1, -177),
    (4, 0, -2, -1, 176),
    (4, -1, -1, -1, 166),
    (1, 0, 1, -1, -164),
    (4, 0, 1, -1, 132),
    (1, 0, -1, -1, -119),
    (4, -1, 0, -1, 115),
    (2, -2, 0, 1, 107),
];

#[cfg(test)]
mod tests {
    use super::*;
    use whether_core::time::days_from_civil;

    const BATTERY: (f64, f64) = (40.7033, -74.0170);
    const ANTIPODE: (f64, f64) = (40.7033, 105.9830);

    fn fixture(name: &str) -> serde_json::Value {
        let path = format!(
            "{}/../../tests/fixtures/raw/usno/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
        serde_json::from_str(&text).unwrap()
    }

    fn midnight(year: i32, month: u8, day: u8) -> UnixTime {
        UnixTime(days_from_civil(year, month, day) * DAY_S)
    }

    /// Seconds of an `HH:MM` string.
    fn hm(s: &str) -> i64 {
        let (h, m) = s.split_once(':').unwrap();
        h.parse::<i64>().unwrap() * 3_600 + m.parse::<i64>().unwrap() * 60
    }

    /// Rounded to the minute, as USNO prints.
    fn to_minute(t: UnixTime) -> i64 {
        (t.0 + 30).div_euclid(60) * 60
    }

    /// One USNO day at `place`: its midnight and its data.
    fn usno_day(place: (f64, f64), day: u8) -> (UnixTime, serde_json::Value) {
        let name = format!(
            "usno-rstt-{:.4}_{:.4}-2026-11-{day:02}.json",
            place.0, place.1
        );
        let doc = fixture(&name);
        (midnight(2026, 11, day), doc["properties"]["data"].clone())
    }

    fn phen(data: &serde_json::Value, name: &str) -> Option<i64> {
        data["moondata"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["phen"] == name)
            .map(|p| hm(p["time"].as_str().unwrap()))
    }

    /// Worst error in seconds of our `kind` at `place` against `name` in USNO's files for
    /// `reference`, over all 15 days; asserts that events are absent where USNO has none.
    fn worst_error(
        place: (f64, f64),
        kind: MoonEventKind,
        reference: (f64, f64),
        name: &str,
    ) -> i64 {
        let mut worst = 0;
        for day in 8..=22 {
            let (start, data) = usno_day(reference, day);
            let ours: Vec<i64> = moon_events(place.0, place.1, start, UnixTime(start.0 + DAY_S))
                .iter()
                .filter(|e| e.kind == kind)
                .map(|e| to_minute(e.time) - start.0)
                .collect();
            match phen(&data, name) {
                Some(expected) => {
                    assert_eq!(ours.len(), 1, "{kind:?} on 11-{day}: {ours:?}");
                    worst = worst.max((ours[0] - expected).abs());
                }
                None => assert!(ours.is_empty(), "{kind:?} on 11-{day}: {ours:?}"),
            }
        }
        eprintln!("{place:?} {kind:?}: worst {worst} s");
        worst
    }

    #[test]
    fn meeus_example_47a() {
        // 1992 April 12, 0h TD.
        let t = (2_448_724.5 - J2000) / 36_525.0;
        let e = moon_ecliptic(t);
        assert!((e.lon_deg - 133.162_655).abs() < 1e-6, "{e:?}");
        assert!((e.lat_deg + 3.229_126).abs() < 1e-6, "{e:?}");
        assert!((e.distance_km - 368_409.7).abs() < 0.1, "{e:?}");
        let parallax = (EARTH_RADIUS_KM / e.distance_km).asin().to_degrees();
        assert!((parallax - 0.991_990).abs() < 1e-6, "{parallax}");
        // Example 48.a: phase angle 69.0756°, here with the moon's apparent longitude.
        let i = phase_angle_sme_deg(t);
        assert!((i - 69.075_6).abs() < 0.01, "{i}");
    }

    #[test]
    fn rise_set_and_upper_transit_match_usno_at_the_battery_and_its_antipode() {
        for place in [BATTERY, ANTIPODE] {
            for (kind, name) in [
                (MoonEventKind::Rise, "Rise"),
                (MoonEventKind::Set, "Set"),
                (MoonEventKind::UpperTransit, "Upper Transit"),
            ] {
                assert!(worst_error(place, kind, place, name) <= 120);
            }
        }
    }

    #[test]
    fn lower_transits_are_the_antipodes_upper_transits() {
        let worst = worst_error(
            BATTERY,
            MoonEventKind::LowerTransit,
            ANTIPODE,
            "Upper Transit",
        );
        assert!(worst <= 120);
    }

    #[test]
    fn principal_phases_match_usno_2026_and_2027() {
        for year in [2026, 2027] {
            let doc = fixture(&format!("usno-moon-phases-year-{year}.json"));
            let expected = doc["phasedata"].as_array().unwrap();
            let ours = principal_phases(midnight(year, 1, 1), midnight(year + 1, 1, 1));
            assert_eq!(ours.len(), expected.len(), "{year}");
            let mut worst = 0;
            for (p, e) in ours.iter().zip(expected) {
                let name = match p.phase {
                    PrincipalPhase::New => "New Moon",
                    PrincipalPhase::FirstQuarter => "First Quarter",
                    PrincipalPhase::Full => "Full Moon",
                    PrincipalPhase::LastQuarter => "Last Quarter",
                };
                assert_eq!(e["phase"], name);
                let at = midnight(
                    year,
                    u8::try_from(e["month"].as_u64().unwrap()).unwrap(),
                    u8::try_from(e["day"].as_u64().unwrap()).unwrap(),
                )
                .0 + hm(e["time"].as_str().unwrap());
                worst = worst.max((to_minute(p.time) - at).abs());
            }
            eprintln!("phases {year}: worst {worst} s");
            assert!(worst <= 300);
        }
    }

    #[test]
    fn moon_day_matches_usno_phase_fraction_and_closest_phase() {
        // USNO's `fracillum` is the fraction at the middle of the requested day (at 00:00 UTC
        // ours differs from it by up to 5 points, at 12:00 by under half a point).
        let mut worst = 0.0_f64;
        for place in [BATTERY, ANTIPODE] {
            for day in 8..=22 {
                let (start, data) = usno_day(place, day);
                let md = moon_day(place.0, place.1, start, UnixTime(start.0 + DAY_S));
                let percent: f64 = data["fracillum"]
                    .as_str()
                    .unwrap()
                    .trim_end_matches('%')
                    .parse()
                    .unwrap();
                let noon = illuminated_fraction(UnixTime(start.0 + DAY_S / 2));
                assert!((f64::from(md.illuminated_fraction) - noon).abs() < 1e-6);
                worst = worst.max((noon * 100.0 - percent).abs());
                let name = match md.phase {
                    MoonPhase::New => "New Moon",
                    MoonPhase::WaxingCrescent => "Waxing Crescent",
                    MoonPhase::FirstQuarter => "First Quarter",
                    MoonPhase::WaxingGibbous => "Waxing Gibbous",
                    MoonPhase::Full => "Full Moon",
                    MoonPhase::WaningGibbous => "Waning Gibbous",
                    MoonPhase::LastQuarter => "Last Quarter",
                    MoonPhase::WaningCrescent => "Waning Crescent",
                };
                assert_eq!(data["curphase"], name, "11-{day}");
                let closest = &data["closestphase"];
                let nearest = md.nearest_principal.unwrap();
                let at = midnight(
                    2026,
                    u8::try_from(closest["month"].as_u64().unwrap()).unwrap(),
                    u8::try_from(closest["day"].as_u64().unwrap()).unwrap(),
                )
                .0 + hm(closest["time"].as_str().unwrap());
                assert!((to_minute(nearest.time) - at).abs() <= 300, "11-{day}");
            }
        }
        eprintln!("illuminated fraction: worst {worst:.2} points");
        assert!(worst <= 1.0);
    }

    #[test]
    fn topocentric_centre_is_below_the_horizon_at_moonrise() {
        // At rise the upper limb touches the refracted horizon: the centre is a semidiameter
        // (about 15′) and 34′ of refraction below the true horizon.
        let start = midnight(2026, 11, 8);
        let rise = moon_events(BATTERY.0, BATTERY.1, start, UnixTime(start.0 + DAY_S))
            .into_iter()
            .find(|e| e.kind == MoonEventKind::Rise)
            .unwrap();
        let h = topocentric_altitude_deg(rise.time, BATTERY.0, BATTERY.1);
        assert!((h + 0.83).abs() < 0.05, "{h}");
    }

    #[test]
    fn position_is_in_range_and_events_are_ordered() {
        let p = moon_position(midnight(2026, 11, 15));
        assert!((0.0..360.0).contains(&p.right_ascension_deg));
        assert!(p.declination_deg.abs() < 29.0);
        assert!((356_000e3..407_000e3).contains(&p.distance_m), "{p:?}");
        let start = midnight(2026, 11, 1);
        let week = moon_events(BATTERY.0, BATTERY.1, start, UnixTime(start.0 + 7 * DAY_S));
        assert!(week.windows(2).all(|w| w[0].time <= w[1].time));
        // A lunar day lasts about 24 h 50 min: under seven of each event in a week.
        assert!((25..=28).contains(&week.len()), "{week:?}");
        assert_eq!(moon_events(0.0, 0.0, start, start), Vec::new());
    }
}
