//! Tidal constituents, node factors and equilibrium arguments.
//!
//! A tide (or tidal current) is predicted as `Z0 + Σ f·A·cos(V0 + u − κ)`, where `A` and `κ` are a
//! station's harmonic constants (CO-OPS `amplitude` and `phase_GMT`), `V0` is the equilibrium
//! argument at Greenwich for the instant, and `f` and `u` are the node factor and nodal phase,
//! which vary slowly with the longitude of the moon's node. This module supplies `V0`, `f`, `u`
//! and the speed of every constituent CO-OPS publishes; angles are degrees throughout.
//!
//! # References
//!
//! Every formula comes from P. Schureman, *Manual of Harmonic Analysis and Prediction of Tides*,
//! US Coast and Geodetic Survey Special Publication No. 98, revised 1940 edition reprinted 1958,
//! <https://tidesandcurrents.noaa.gov/publications/SpecialPubNo98.pdf>, cited below as SP98 with
//! its page and formula numbers:
//!
//! - mean longitudes `s`, `h`, `p`, `N`, `p1` as polynomials of Julian centuries from Greenwich
//!   mean noon, 1899-12-31 (Table 1, p. 162); `T` is 180° at each Greenwich midnight (p. 157);
//! - `I`, `ν` and `ξ` from the obliquity `ω` = 23.452° and the inclination `i` = 5.145° of the
//!   moon's orbit (Table 1, and the relations on p. 156);
//! - `ν′` (formula 224), `2ν″` (232), `R` and `1/Ra` for L2 (213, 214), `Q` and `1/Qa` for M1
//!   (197, 203, 204), all with `I` at its value for the instant and mean coefficients;
//! - arguments `V` and `u` of the elementary constituents (Table 2, pp. 164–166) and of the
//!   shallow-water constituents, whose `V` and `u` are the sums of their components' with the
//!   signs of the combination and whose `f` is the product of the components' (Table 2a,
//!   p. 167);
//! - node factors: formulas 73–78 (p. 25), 149 (p. 36), 207 (p. 43), 215 (p. 44), 227 (p. 45)
//!   and 235 (p. 46).
//!
//! `MSf` follows SP98's Tables 14 and 15 (footnote to Table 14: "factor f of `MS`, `2SM` and
//! `MSf` are each equal to factor f of M2"), which treat it as the compound S2 − M2, rather than
//! the elementary term A5 of Table 2. B. B. Parker, *Tidal Analysis and Prediction*, NOAA Special
//! Publication NOS CO-OPS 3 (2007), §2.2.4, confirms the practice of one `f` and `u` per year,
//! taken at the middle of the year, with `V0` for the instant.

use whether_core::time::{days_from_civil, is_leap_year};

use crate::time::UnixTime;

const SECONDS_PER_DAY: i64 = 86_400;
/// Hours in a Julian century of 36 525 days.
const HOURS_PER_CENTURY: f64 = 876_600.0;
/// 1899-12-31 12:00 UTC (Greenwich mean noon), the epoch of SP98 Table 1, in unix seconds.
const EPOCH_1900: i64 = -2_209_032_000;
/// Arc seconds in one revolution.
const REVOLUTION: f64 = 1_296_000.0;

/// Obliquity of the ecliptic `ω`, epoch 1900 (SP98 Table 1), degrees.
const OBLIQUITY: f64 = 23.452;
/// Inclination `i` of the moon's orbit to the ecliptic (SP98 Table 1), degrees.
const LUNAR_INCLINATION: f64 = 5.145;

/// A mean longitude as SP98 Table 1 gives it: degrees, minutes and seconds at the epoch, then
/// the coefficients of `T`, `T²` and `T³` in arc seconds, with `T` in Julian centuries.
struct Longitude {
    degrees: f64,
    minutes: f64,
    seconds: f64,
    rates: [f64; 3],
}

impl Longitude {
    /// The longitude in degrees, `0..360`, after `centuries` Julian centuries.
    fn at(&self, centuries: f64) -> f64 {
        let [c1, c2, c3] = self.rates;
        let arcsec = ((c3 * centuries + c2) * centuries + c1) * centuries;
        (self.degrees + self.minutes / 60.0 + (self.seconds + arcsec) / 3600.0).rem_euclid(360.0)
    }
}

/// Mean longitude of the moon, `s`.
const MOON: Longitude = Longitude {
    degrees: 270.0,
    minutes: 26.0,
    seconds: 14.72,
    rates: [1336.0 * REVOLUTION + 1_108_411.20, 9.09, 0.0068],
};
/// Mean longitude of the sun, `h`.
const SUN: Longitude = Longitude {
    degrees: 279.0,
    minutes: 41.0,
    seconds: 48.04,
    rates: [129_602_768.13, 1.089, 0.0],
};
/// Longitude of the lunar perigee, `p`.
const LUNAR_PERIGEE: Longitude = Longitude {
    degrees: 334.0,
    minutes: 19.0,
    seconds: 40.87,
    rates: [11.0 * REVOLUTION + 392_515.94, -37.24, -0.045],
};
/// Longitude of the moon's ascending node, `N`.
const LUNAR_NODE: Longitude = Longitude {
    degrees: 259.0,
    minutes: 10.0,
    seconds: 57.12,
    rates: [-(5.0 * REVOLUTION + 482_912.63), 7.58, 0.008],
};
/// Longitude of the solar perigee, `p1`.
const SOLAR_PERIGEE: Longitude = Longitude {
    degrees: 281.0,
    minutes: 13.0,
    seconds: 15.0,
    rates: [6_189.03, 1.63, 0.012],
};

/// The astronomical arguments of SP98 for one instant, in degrees.
///
/// The first six are the uniformly varying elements; the rest are the slowly varying functions
/// of the moon's node and perigee from which `u` and `f` are computed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AstronomicalArguments {
    /// `T`: hour angle of the mean sun at Greenwich, 180° at 00:00 UTC.
    pub t: f64,
    /// `s`: mean longitude of the moon.
    pub s: f64,
    /// `h`: mean longitude of the sun.
    pub h: f64,
    /// `p`: longitude of the lunar perigee.
    pub p: f64,
    /// `N`: longitude of the moon's ascending node.
    pub n: f64,
    /// `p1`: longitude of the solar perigee.
    pub p1: f64,
    /// `I`: inclination of the moon's orbit to the equator.
    pub i: f64,
    /// `ν`: right ascension of the lunar intersection.
    pub nu: f64,
    /// `ξ`: longitude in the moon's orbit of the lunar intersection.
    pub xi: f64,
    /// `ν′`: the term of `u` for K1 (SP98 formula 224).
    pub nu_prime: f64,
    /// `2ν″`: the term of `u` for K2 (SP98 formula 232).
    pub two_nu_double_prime: f64,
    /// `P = p − ξ`: the lunar perigee reckoned from the lunar intersection (SP98 formula 191).
    pub perigee_from_intersection: f64,
    /// `R`: the term of `u` for L2 (SP98 formula 214).
    pub r: f64,
    /// `1/Ra`: the factor of `f` for L2 (SP98 formula 213).
    pub r_a_inverse: f64,
    /// `Q`: the M1 argument term of SP98 formula 203, `tan Q = 0.483 tan P`.
    pub q: f64,
    /// `Qu = P − Q`: the term of `u` for M1 (SP98 formulas 194 and 204).
    pub q_u: f64,
    /// `1/Qa`: the factor of `f` for M1 (SP98 formula 197).
    pub q_a_inverse: f64,
}

impl AstronomicalArguments {
    /// The arguments at a UTC instant. Universal time stands in for the ephemeris time of the
    /// formulas; the difference (about a minute) moves the moon by less than 0.01°.
    pub fn at(time: UnixTime) -> Self {
        let centuries = (time.0 - EPOCH_1900) as f64 / (HOURS_PER_CENTURY * 3600.0);
        let hours_of_day = time.0.rem_euclid(SECONDS_PER_DAY) as f64 / 3600.0;
        let node = NodeTerms::new(LUNAR_NODE.at(centuries));
        let perigee = LUNAR_PERIGEE.at(centuries);
        let lunar = PerigeeTerms::new(perigee, &node);
        Self {
            t: (180.0 + 15.0 * hours_of_day).rem_euclid(360.0),
            s: MOON.at(centuries),
            h: SUN.at(centuries),
            p: perigee,
            n: node.n,
            p1: SOLAR_PERIGEE.at(centuries),
            i: node.i,
            nu: node.nu,
            xi: node.xi,
            nu_prime: node.nu_prime,
            two_nu_double_prime: node.two_nu_double_prime,
            perigee_from_intersection: lunar.big_p,
            r: lunar.r,
            r_a_inverse: lunar.r_a_inverse,
            q: lunar.q,
            q_u: lunar.q_u,
            q_a_inverse: lunar.q_a_inverse,
        }
    }

    /// The arguments at the middle of a UTC calendar year, where the yearly `f` and `u` are
    /// evaluated (see [`mid_year`]).
    pub fn mid_year(year: i32) -> Self {
        Self::at(mid_year(year))
    }
}

/// The functions of the node `N`, in degrees.
struct NodeTerms {
    n: f64,
    i: f64,
    nu: f64,
    xi: f64,
    nu_prime: f64,
    two_nu_double_prime: f64,
}

impl NodeTerms {
    fn new(n: f64) -> Self {
        // I, ν and ξ from the spherical triangle of SP98 p. 156:
        // cos I = cos i cos ω − sin i sin ω cos N,
        // tan ½(N − ξ + ν) = cos ½(ω − i) / cos ½(ω + i) · tan ½N,
        // tan ½(N − ξ − ν) = sin ½(ω − i) / sin ½(ω + i) · tan ½N.
        let (omega, incl) = (OBLIQUITY.to_radians(), LUNAR_INCLINATION.to_radians());
        let cos_inclination =
            incl.cos() * omega.cos() - incl.sin() * omega.sin() * n.to_radians().cos();
        let inclination = cos_inclination.acos();
        let (half_sin, half_cos) = (n / 2.0).to_radians().sin_cos();
        let (half_diff, half_sum) = ((omega - incl) / 2.0, f64::midpoint(omega, incl));
        let sum_ratio = half_diff.cos() / half_sum.cos();
        let diff_ratio = half_diff.sin() / half_sum.sin();
        let plus = (sum_ratio * half_sin).atan2(half_cos).to_degrees();
        let minus = (diff_ratio * half_sin).atan2(half_cos).to_degrees();
        let nu = plus - minus;

        let nu_rad = nu.to_radians();
        let sin_double = (2.0 * inclination).sin();
        let sin_squared = inclination.sin().powi(2);
        // SP98 formula 224: tan ν′ = sin 2I sin ν / (sin 2I cos ν + 0.3347).
        let nu_prime = (sin_double * nu_rad.sin()).atan2(sin_double * nu_rad.cos() + 0.3347);
        // SP98 formula 232: tan 2ν″ = sin²I sin 2ν / (sin²I cos 2ν + 0.0727).
        let (sin_2nu, cos_2nu) = (2.0 * nu_rad).sin_cos();
        let two_nu_double_prime = (sin_squared * sin_2nu).atan2(sin_squared * cos_2nu + 0.0727);
        Self {
            n,
            i: inclination.to_degrees(),
            nu,
            xi: signed(n - plus - minus),
            nu_prime: nu_prime.to_degrees(),
            two_nu_double_prime: two_nu_double_prime.to_degrees(),
        }
    }
}

/// The functions of the lunar perigee used by L2 and M1, in degrees.
struct PerigeeTerms {
    big_p: f64,
    r: f64,
    r_a_inverse: f64,
    q: f64,
    q_u: f64,
    q_a_inverse: f64,
}

impl PerigeeTerms {
    fn new(perigee: f64, node: &NodeTerms) -> Self {
        let big_p = (perigee - node.xi).rem_euclid(360.0);
        let (sin_double, cos_double) = (2.0 * big_p).to_radians().sin_cos();
        let tan2_half_i = (node.i / 2.0).to_radians().tan().powi(2);
        // SP98 formula 213: 1/Ra = (1 − 12 tan²½I cos 2P + 36 tan⁴½I)^½.
        let r_a_squared = 1.0 - 12.0 * tan2_half_i * cos_double + 36.0 * tan2_half_i.powi(2);
        // SP98 formula 214: tan R = sin 2P / (⅙ cot²½I − cos 2P).
        let r = sin_double.atan2(1.0 / (6.0 * tan2_half_i) - cos_double);
        // SP98 formula 203: tan Q = 0.483 tan P (Q in the quadrant of P); 204: Qu = P − Q.
        let (sin_single, cos_single) = big_p.to_radians().sin_cos();
        let q = (0.483 * sin_single)
            .atan2(cos_single)
            .to_degrees()
            .rem_euclid(360.0);
        Self {
            big_p,
            r: r.to_degrees(),
            r_a_inverse: r_a_squared.sqrt(),
            q,
            q_u: signed(big_p - q),
            // SP98 formula 197: 1/Qa = (2.310 + 1.435 cos 2P)^½.
            q_a_inverse: (2.310 + 1.435 * cos_double).sqrt(),
        }
    }
}

/// The start of a UTC calendar year.
pub fn year_start(year: i32) -> UnixTime {
    UnixTime(days_from_civil(year, 1, 1) * SECONDS_PER_DAY)
}

/// The middle of a UTC calendar year: noon on 2 July in common years, the preceding midnight in
/// leap years (SP98 p. 157, explanation of Table 15).
pub fn mid_year(year: i32) -> UnixTime {
    let days = if is_leap_year(year) { 366 } else { 365 };
    UnixTime(year_start(year).0 + days * SECONDS_PER_DAY / 2)
}

/// The UTC calendar year containing an instant.
pub fn utc_year(time: UnixTime) -> i32 {
    whether_core::time::civil_from_days(time.0.div_euclid(SECONDS_PER_DAY)).0
}

/// An angle in degrees reduced to `-180..=180`.
fn signed(degrees: f64) -> f64 {
    let x = degrees.rem_euclid(360.0);
    if x > 180.0 { x - 360.0 } else { x }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc(year: i32, month: u8, day: u8, hour: i64) -> UnixTime {
        UnixTime(days_from_civil(year, month, day) * SECONDS_PER_DAY + hour * 3600)
    }

    fn assert_angle(label: &str, actual: f64, expected: f64, tolerance: f64) {
        let diff = signed(actual - expected).abs();
        assert!(diff <= tolerance, "{label}: {actual:.4} vs {expected:.4}");
    }

    /// SP98 Table 1, "Mean longitude of solar and lunar elements for century years".
    #[test]
    fn elements_match_sp98_table_1() {
        let rows = [
            (1900, [280.190, 281.221, 277.026, 334.384, 259.156]),
            (2000, [279.973, 282.940, 211.744, 83.294, 125.069]),
        ];
        for (year, [h, p1, s, p, n]) in rows {
            let a = AstronomicalArguments::at(year_start(year));
            assert_angle("T", a.t, 180.0, 1e-9);
            let pairs = [(a.h, h), (a.p1, p1), (a.s, s), (a.p, p), (a.n, n)];
            for (index, (actual, expected)) in pairs.into_iter().enumerate() {
                assert_angle(&format!("{year} #{index}"), actual, expected, 6e-4);
            }
        }
    }

    /// SP98 Table 6 (two decimals; `ν`, `ξ`, `ν′`, `2ν″` negative for N between 180° and 360°;
    /// its `ν′` and `2ν″` come from slightly different earlier formulas, p. 156), and the
    /// extremes `I = ω ± i` with every other term zero at N = 0° and 180°.
    #[test]
    fn node_terms_match_sp98_table_6() {
        let (max, min) = (OBLIQUITY + LUNAR_INCLINATION, OBLIQUITY - LUNAR_INCLINATION);
        for (n, expected, tolerance) in [
            (0.0, [max, 0.0, 0.0, 0.0, 0.0], 1e-9),
            (180.0, [min, 0.0, 0.0, 0.0, 0.0], 1e-9),
            (40.0, [27.58, 7.15, 6.46, 5.08, 10.69], 0.01),
            (45.0, [27.32, 7.94, 7.18, 5.63, 11.82], 0.01),
            (315.0, [27.32, -7.94, -7.18, -5.63, -11.82], 0.01),
        ] {
            let t = NodeTerms::new(n);
            let actual = [t.i, t.nu, t.xi, t.nu_prime, t.two_nu_double_prime];
            for (index, (a, e)) in actual.into_iter().zip(expected).enumerate() {
                assert_angle(&format!("N = {n} #{index}"), a, e, tolerance);
            }
        }
    }

    #[test]
    fn yearly_corrections_are_taken_at_mid_year() {
        assert_eq!(mid_year(2026), utc(2026, 7, 2, 12));
        assert_eq!(mid_year(2028), utc(2028, 7, 2, 0));
        assert_eq!(utc_year(UnixTime(year_start(2034).0 - 1)), 2033);
        assert_eq!(utc_year(year_start(2034)), 2034);
    }
}
