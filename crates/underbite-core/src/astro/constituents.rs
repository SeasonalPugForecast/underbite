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
//! - `ν′` (formula 224), `2ν″` (232), `R` and `1/Ra` for L2 (213, 214) with `I` for the
//!   instant; `Q`, `Qu` and `1/Qa` for M1 from the forms with `I` at its mean value (197, 203,
//!   204), which SP98's own tables use;
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

use std::collections::BTreeMap;

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

    /// The linear rate in degrees per mean solar hour.
    fn hourly_rate(&self) -> f64 {
        self.rates[0] / 3600.0 / HOURS_PER_CENTURY
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

/// A node factor and nodal phase.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NodeCorrection {
    /// Node factor `f`, multiplying the amplitude.
    pub f: f64,
    /// Nodal phase `u` in degrees, `-180..=180`, added to `V0`.
    pub u: f64,
}

/// Declares [`Constituent`] with its CO-OPS names.
macro_rules! constituents {
    ($($(#[doc = $doc:literal])* $variant:ident = $name:literal,)*) => {
        /// A tidal constituent, by the name CO-OPS gives it.
        ///
        /// Each variant states its origin in SP98: `V` (with `T`, `s`, `h`, `p`, `p1`), `u` and
        /// the node-factor formula for elementary constituents, or the combination for compound
        /// ones.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum Constituent {
            $($(#[doc = $doc])* $variant,)*
        }

        impl Constituent {
            /// Every constituent, in the order CO-OPS numbers them for The Battery (8518750).
            pub const ALL: [Self; 37] = [$(Self::$variant,)*];

            /// The name CO-OPS uses in `harcon` responses (`name` for tide stations,
            /// `constituentName` for current stations).
            pub fn coops_name(self) -> &'static str {
                match self {
                    $(Self::$variant => $name,)*
                }
            }
        }
    };
}

constituents! {
    /// Principal lunar semidiurnal (A39): `V = 2T − 2s + 2h`, `u = 2ξ − 2ν`, `f` formula 78.
    M2 = "M2",
    /// Principal solar semidiurnal (B39): `V = 2T`, `u = 0`, `f = 1`.
    S2 = "S2",
    /// Larger lunar elliptic semidiurnal (A40): `V = 2T − 3s + 2h + p`, `u = 2ξ − 2ν`, `f` 78.
    N2 = "N2",
    /// Lunisolar diurnal (note 2): `V = T + h − 90°`, `u = −ν′`, `f` formula 227.
    K1 = "K1",
    /// Shallow water, 2·M2.
    M4 = "M4",
    /// Lunar diurnal (A14): `V = T − 2s + h + 90°`, `u = 2ξ − ν`, `f` formula 75.
    O1 = "O1",
    /// Shallow water, 3·M2.
    M6 = "M6",
    /// Shallow water, M2 + K1.
    Mk3 = "MK3",
    /// Shallow water, 2·S2.
    S4 = "S4",
    /// Shallow water, M2 + N2.
    Mn4 = "MN4",
    /// Larger lunar evectional (A43): `V = 2T − 3s + 4h − p`, `u = 2ξ − 2ν`, `f` 78.
    Nu2 = "NU2",
    /// Shallow water, 3·S2.
    S6 = "S6",
    /// Variational (A45): `V = 2T − 4s + 4h`, `u = 2ξ − 2ν`, `f` 78.
    Mu2 = "MU2",
    /// Lunar elliptic semidiurnal second order (A42): `V = 2T − 4s + 2h + 2p`, `u = 2ξ − 2ν`,
    /// `f` 78.
    TwoN2 = "2N2",
    /// Lunar diurnal (A31): `V = T + 2s + h − 90°`, `u = −2ξ − ν`, `f` formula 77.
    Oo1 = "OO1",
    /// Smaller lunar evectional (A44): `V = 2T − s + p + 180°`, `u = 2ξ − 2ν`, `f` 78.
    Lam2 = "LAM2",
    /// Solar diurnal (B71): `V = T`, `u = 0`, `f = 1`.
    S1 = "S1",
    /// Smaller lunar elliptic diurnal (note 1, formula 194): `V = T − s + h + p − 90°`,
    /// `u = −ν − Qu`, `f` formula 207. This form carries the speed CO-OPS publishes; SP98
    /// Table 15 uses the equivalent form 201 (`V = T − s + h − 90°`, `u = ξ − ν + Q`).
    M1 = "M1",
    /// Smaller lunar elliptic diurnal (A24): `V = T + s + h − p − 90°`, `u = −ν`, `f` formula 76.
    J1 = "J1",
    /// Lunar monthly (A2): `V = s − p`, `u = 0`, `f` formula 73.
    Mm = "MM",
    /// Solar semiannual (B6): `V = 2h`, `u = 0`, `f = 1`.
    Ssa = "SSA",
    /// Solar annual (B64): `V = h`, `u = 0`, `f = 1`.
    Sa = "SA",
    /// Lunisolar synodic fortnightly, as SP98 Tables 14 and 15 treat it: S2 − M2.
    Msf = "MSF",
    /// Lunisolar fortnightly (A6): `V = 2s`, `u = −2ξ`, `f` formula 74.
    Mf = "MF",
    /// Larger lunar evectional diurnal (A18): `V = T − 3s + 3h − p + 90°`, `u = 2ξ − ν`, `f` 75.
    Rho1 = "RHO",
    /// Larger lunar elliptic diurnal (A15): `V = T − 3s + h + p + 90°`, `u = 2ξ − ν`, `f` 75.
    Q1 = "Q1",
    /// Larger solar elliptic (B40): `V = 2T − h + p1`, `u = 0`, `f = 1`.
    T2 = "T2",
    /// Smaller solar elliptic (B41): `V = 2T + h − p1 + 180°`, `u = 0`, `f = 1`.
    R2 = "R2",
    /// Lunar elliptic diurnal second order (A17): `V = T − 4s + h + 2p + 90°`, `u = 2ξ − ν`,
    /// `f` 75.
    TwoQ1 = "2Q1",
    /// Solar diurnal (B14): `V = T − h + 90°`, `u = 0`, `f = 1`.
    P1 = "P1",
    /// Shallow water, 2·S2 − M2.
    TwoSm2 = "2SM2",
    /// Lunar terdiurnal (A82): `V = 3T − 3s + 3h`, `u = 3ξ − 3ν`, `f` formula 149.
    M3 = "M3",
    /// Smaller lunar elliptic semidiurnal (note 3, formula 212): `V = 2T − s + 2h − p + 180°`,
    /// `u = 2ξ − 2ν − R`, `f` formula 215.
    L2 = "L2",
    /// Shallow water, 2·M2 − K1.
    TwoMk3 = "2MK3",
    /// Lunisolar semidiurnal (note 4): `V = 2T + 2h`, `u = −2ν″`, `f` formula 235.
    K2 = "K2",
    /// Shallow water, 4·M2.
    M8 = "M8",
    /// Shallow water, M2 + S2.
    Ms4 = "MS4",
}

/// The node-factor formula of an elementary constituent, named after the constituent SP98 states
/// it for (formula numbers in [`elementary_node_factor`]).
#[derive(Clone, Copy, Debug)]
enum NodeFormula {
    Unity,
    Mm,
    Mf,
    O1,
    J1,
    Oo1,
    M2,
    M3,
    M1,
    L2,
    K1,
    K2,
}

/// How a constituent's argument and node factor are built.
#[derive(Clone, Copy, Debug)]
enum Definition {
    /// Coefficients of `T, s, h, p, p1` in `V`, a constant added to `V` in degrees, the
    /// coefficients of `ξ, ν, ν′, 2ν″, R, Qu` in `u`, and the node-factor formula.
    Elementary([i8; 5], f64, [i8; 6], NodeFormula),
    /// Components with their multiples.
    Compound(&'static [(Constituent, i8)]),
}

const U_NONE: [i8; 6] = [0; 6];
const U_DIURNAL: [i8; 6] = [2, -1, 0, 0, 0, 0];
const U_SEMIDIURNAL: [i8; 6] = [2, -2, 0, 0, 0, 0];

impl Constituent {
    /// The constituent CO-OPS names `name`, or `None` for a name outside the table.
    pub fn from_coops_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.coops_name() == name)
    }

    fn definition(self) -> Definition {
        use Definition::{Compound, Elementary};
        use NodeFormula as F;
        match self {
            Self::Sa => Elementary([0, 0, 1, 0, 0], 0.0, U_NONE, F::Unity),
            Self::Ssa => Elementary([0, 0, 2, 0, 0], 0.0, U_NONE, F::Unity),
            Self::Mm => Elementary([0, 1, 0, -1, 0], 0.0, U_NONE, F::Mm),
            Self::Mf => Elementary([0, 2, 0, 0, 0], 0.0, [-2, 0, 0, 0, 0, 0], F::Mf),
            Self::TwoQ1 => Elementary([1, -4, 1, 2, 0], 90.0, U_DIURNAL, F::O1),
            Self::Q1 => Elementary([1, -3, 1, 1, 0], 90.0, U_DIURNAL, F::O1),
            Self::Rho1 => Elementary([1, -3, 3, -1, 0], 90.0, U_DIURNAL, F::O1),
            Self::O1 => Elementary([1, -2, 1, 0, 0], 90.0, U_DIURNAL, F::O1),
            Self::M1 => Elementary([1, -1, 1, 1, 0], -90.0, [0, -1, 0, 0, 0, -1], F::M1),
            Self::P1 => Elementary([1, 0, -1, 0, 0], 90.0, U_NONE, F::Unity),
            Self::S1 => Elementary([1, 0, 0, 0, 0], 0.0, U_NONE, F::Unity),
            Self::K1 => Elementary([1, 0, 1, 0, 0], -90.0, [0, 0, -1, 0, 0, 0], F::K1),
            Self::J1 => Elementary([1, 1, 1, -1, 0], -90.0, [0, -1, 0, 0, 0, 0], F::J1),
            Self::Oo1 => Elementary([1, 2, 1, 0, 0], -90.0, [-2, -1, 0, 0, 0, 0], F::Oo1),
            Self::TwoN2 => Elementary([2, -4, 2, 2, 0], 0.0, U_SEMIDIURNAL, F::M2),
            Self::Mu2 => Elementary([2, -4, 4, 0, 0], 0.0, U_SEMIDIURNAL, F::M2),
            Self::N2 => Elementary([2, -3, 2, 1, 0], 0.0, U_SEMIDIURNAL, F::M2),
            Self::Nu2 => Elementary([2, -3, 4, -1, 0], 0.0, U_SEMIDIURNAL, F::M2),
            Self::M2 => Elementary([2, -2, 2, 0, 0], 0.0, U_SEMIDIURNAL, F::M2),
            Self::Lam2 => Elementary([2, -1, 0, 1, 0], 180.0, U_SEMIDIURNAL, F::M2),
            Self::L2 => Elementary([2, -1, 2, -1, 0], 180.0, [2, -2, 0, 0, -1, 0], F::L2),
            Self::T2 => Elementary([2, 0, -1, 0, 1], 0.0, U_NONE, F::Unity),
            Self::S2 => Elementary([2, 0, 0, 0, 0], 0.0, U_NONE, F::Unity),
            Self::R2 => Elementary([2, 0, 1, 0, -1], 180.0, U_NONE, F::Unity),
            Self::K2 => Elementary([2, 0, 2, 0, 0], 0.0, [0, 0, 0, -1, 0, 0], F::K2),
            Self::M3 => Elementary([3, -3, 3, 0, 0], 0.0, [3, -3, 0, 0, 0, 0], F::M3),
            Self::Msf => Compound(&[(Self::S2, 1), (Self::M2, -1)]),
            Self::TwoSm2 => Compound(&[(Self::S2, 2), (Self::M2, -1)]),
            Self::Mk3 => Compound(&[(Self::M2, 1), (Self::K1, 1)]),
            Self::TwoMk3 => Compound(&[(Self::M2, 2), (Self::K1, -1)]),
            Self::M4 => Compound(&[(Self::M2, 2)]),
            Self::Mn4 => Compound(&[(Self::M2, 1), (Self::N2, 1)]),
            Self::Ms4 => Compound(&[(Self::M2, 1), (Self::S2, 1)]),
            Self::S4 => Compound(&[(Self::S2, 2)]),
            Self::M6 => Compound(&[(Self::M2, 3)]),
            Self::S6 => Compound(&[(Self::S2, 3)]),
            Self::M8 => Compound(&[(Self::M2, 4)]),
        }
    }

    /// Angular speed in degrees per mean solar hour: the hourly rates of the elements in `V`
    /// (`T` at 15°/h, the others from the linear terms of SP98 Table 1).
    pub fn speed(self) -> f64 {
        match self.definition() {
            Definition::Elementary(v, _, _, _) => {
                let rates = [
                    15.0,
                    MOON.hourly_rate(),
                    SUN.hourly_rate(),
                    LUNAR_PERIGEE.hourly_rate(),
                    SOLAR_PERIGEE.hourly_rate(),
                ];
                v.iter()
                    .zip(rates)
                    .map(|(&k, rate)| f64::from(k) * rate)
                    .sum()
            }
            Definition::Compound(parts) => {
                parts.iter().map(|&(c, k)| f64::from(k) * c.speed()).sum()
            }
        }
    }

    /// Equilibrium argument `V0` at Greenwich for the instant of `args`, degrees `0..360`.
    pub fn equilibrium_argument(self, args: &AstronomicalArguments) -> f64 {
        self.raw_argument(args).rem_euclid(360.0)
    }

    fn raw_argument(self, args: &AstronomicalArguments) -> f64 {
        match self.definition() {
            Definition::Elementary(v, constant, _, _) => {
                let elements = [args.t, args.s, args.h, args.p, args.p1];
                let sum: f64 = v.iter().zip(elements).map(|(&k, e)| f64::from(k) * e).sum();
                sum + constant
            }
            Definition::Compound(parts) => parts
                .iter()
                .map(|&(c, k)| f64::from(k) * c.raw_argument(args))
                .sum(),
        }
    }

    /// Nodal phase `u` for the node and perigee of `args`, degrees `-180..=180`.
    pub fn nodal_phase(self, args: &AstronomicalArguments) -> f64 {
        signed(self.raw_phase(args))
    }

    fn raw_phase(self, args: &AstronomicalArguments) -> f64 {
        match self.definition() {
            Definition::Elementary(_, _, u, _) => {
                let terms = [
                    args.xi,
                    args.nu,
                    args.nu_prime,
                    args.two_nu_double_prime,
                    args.r,
                    args.q_u,
                ];
                u.iter().zip(terms).map(|(&k, x)| f64::from(k) * x).sum()
            }
            Definition::Compound(parts) => parts
                .iter()
                .map(|&(c, k)| f64::from(k) * c.raw_phase(args))
                .sum(),
        }
    }

    /// Node factor `f` for the node and perigee of `args`. A compound constituent's factor is the
    /// product of its components', each raised to the absolute value of its multiple (SP98
    /// Table 2a).
    pub fn node_factor(self, args: &AstronomicalArguments) -> f64 {
        match self.definition() {
            Definition::Elementary(_, _, _, formula) => elementary_node_factor(formula, args),
            Definition::Compound(parts) => parts
                .iter()
                .map(|&(c, k)| c.node_factor(args).powi(i32::from(k.unsigned_abs())))
                .product(),
        }
    }

    /// `f` and `u` together.
    pub fn node_correction(self, args: &AstronomicalArguments) -> NodeCorrection {
        NodeCorrection {
            f: self.node_factor(args),
            u: self.nodal_phase(args),
        }
    }
}

/// The node factor of an elementary constituent, from the SP98 formula named in each arm.
fn elementary_node_factor(formula: NodeFormula, args: &AstronomicalArguments) -> f64 {
    let i = args.i.to_radians();
    let sin_i = i.sin();
    let sin_double_i = (2.0 * i).sin();
    let (sin_half, cos_half) = (i / 2.0).sin_cos();
    // Formula 75, also the base of M1's formula 207.
    let o1 = sin_i * cos_half.powi(2) / 0.3800;
    match formula {
        NodeFormula::Unity => 1.0,
        // (73) f(Mm) = (2/3 − sin²I) / 0.5021
        NodeFormula::Mm => (2.0 / 3.0 - sin_i.powi(2)) / 0.5021,
        // (74) f(Mf) = sin²I / 0.1578
        NodeFormula::Mf => sin_i.powi(2) / 0.1578,
        // (75) f(O1) = sin I cos²½I / 0.3800
        NodeFormula::O1 => o1,
        // (76) f(J1) = sin 2I / 0.7214
        NodeFormula::J1 => sin_double_i / 0.7214,
        // (77) f(OO1) = sin I sin²½I / 0.0164
        NodeFormula::Oo1 => sin_i * sin_half.powi(2) / 0.0164,
        // (78) f(M2) = cos⁴½I / 0.9154
        NodeFormula::M2 => cos_half.powi(4) / 0.9154,
        // (149) f(M3) = cos⁶½I / 0.8758
        NodeFormula::M3 => cos_half.powi(6) / 0.8758,
        // (207) f(M1) = f(O1) · 1/Qa
        NodeFormula::M1 => o1 * args.q_a_inverse,
        // (215) f(L2) = f(M2) · 1/Ra
        NodeFormula::L2 => cos_half.powi(4) / 0.9154 * args.r_a_inverse,
        // (227) f(K1) = (0.8965 sin²2I + 0.6001 sin 2I cos ν + 0.1006)^½
        NodeFormula::K1 => (0.8965 * sin_double_i.powi(2)
            + 0.6001 * sin_double_i * args.nu.to_radians().cos()
            + 0.1006)
            .sqrt(),
        // (235) f(K2) = (19.0444 sin⁴I + 2.7702 sin²I cos 2ν + 0.0981)^½
        NodeFormula::K2 => (19.0444 * sin_i.powi(4)
            + 2.7702 * sin_i.powi(2) * (2.0 * args.nu).to_radians().cos()
            + 0.0981)
            .sqrt(),
    }
}

/// `f` and `u` of every constituent for a UTC calendar year, evaluated at the middle of the year
/// as in SP98 Tables 14 and 15 and NOAA practice (Parker 2007, §2.2.4).
pub fn yearly_node_corrections(year: i32) -> BTreeMap<Constituent, NodeCorrection> {
    let args = AstronomicalArguments::mid_year(year);
    Constituent::ALL
        .into_iter()
        .map(|c| (c, c.node_correction(&args)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    macro_rules! fixture {
        ($file:literal) => {
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../tests/fixtures/raw/coops/",
                $file
            ))
        };
    }

    /// The recorded `harcon` responses: two tide stations and a current station, with the key
    /// that holds the constituent name.
    const FIXTURES: [(&str, &str); 3] = [
        (fixture!("coops-mdapi-8518750-harcon.json"), "name"),
        (fixture!("coops-mdapi-8729840-harcon-metric.json"), "name"),
        (
            fixture!("coops-mdapi-n03020-harcon-bin7.json"),
            "constituentName",
        ),
    ];

    /// `(name, speed)` of each entry; current stations have no `speed`.
    fn harcon(json: &str, name_key: &str) -> Vec<(String, Option<f64>)> {
        let value: serde_json::Value = serde_json::from_str(json).unwrap();
        let entries = value["HarmonicConstituents"].as_array().unwrap();
        entries
            .iter()
            .map(|c| {
                (
                    c[name_key].as_str().unwrap().to_owned(),
                    c["speed"].as_f64(),
                )
            })
            .collect()
    }

    fn utc(year: i32, month: u8, day: u8, hour: i64) -> UnixTime {
        UnixTime(days_from_civil(year, month, day) * SECONDS_PER_DAY + hour * 3600)
    }

    fn assert_angle(label: &str, actual: f64, expected: f64, tolerance: f64) {
        let diff = signed(actual - expected).abs();
        assert!(diff <= tolerance, "{label}: {actual:.4} vs {expected:.4}");
    }

    #[test]
    fn every_recorded_constituent_is_in_the_table() {
        let mut counts = Vec::new();
        for (json, key) in FIXTURES {
            let entries = harcon(json, key);
            for (name, _) in &entries {
                let c = Constituent::from_coops_name(name);
                assert_eq!(
                    c.map(Constituent::coops_name),
                    Some(name.as_str()),
                    "{name}"
                );
            }
            counts.push(entries.len());
        }
        assert_eq!(counts, [37, 37, 29]);
        assert_eq!(Constituent::from_coops_name("Z0"), None);
    }

    /// CO-OPS prints speeds to six or seven decimals, or to single precision for the fast
    /// constituents (`86.95232` for M6, whose speed is 86.952 312 6): allow 1e-6 °/h or one
    /// single-precision step, whichever is larger.
    #[test]
    fn speeds_match_coops_harcon() {
        for (json, key) in &FIXTURES[..2] {
            for (index, (name, speed)) in harcon(json, key).into_iter().enumerate() {
                let c = Constituent::from_coops_name(&name).unwrap();
                assert_eq!(c, Constituent::ALL[index], "CO-OPS order");
                let expected = speed.unwrap();
                let single = expected as f32;
                let step = f64::from(f32::from_bits(single.to_bits() + 1) - single);
                let diff = (c.speed() - expected).abs();
                assert!(
                    diff <= step.max(1e-6),
                    "{name}: {} vs {expected}",
                    c.speed()
                );
            }
        }
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

    /// f(M2), u(M2), f(O1) and f(K1) at mid-2026 and mid-2034 worked by hand from SP98 with the
    /// rounded coefficients it prints: N from Table 1, `cos I = 0.91370 − 0.03569 cos N`,
    /// `tan ½(N − ξ + ν) = 1.01883 tan ½N`, `tan ½(N − ξ − ν) = 0.64412 tan ½N` (p. 156), then
    /// `f(M2) = cos⁴½I / 0.9154` (78), `f(O1) = sin I cos²½I / 0.3800` (75),
    /// `f(K1) = (0.8965 sin²2I + 0.6001 sin 2I cos ν + 0.1006)^½` (227), `u(M2) = 2ξ − 2ν`.
    #[test]
    fn node_factors_match_sp98_formulas_by_hand() {
        for year in [2026, 2034] {
            // Both are common years: mid-year is 2 July 12:00, a whole number of days after
            // the epoch 1899-12-31 12:00.
            assert!(!is_leap_year(year));
            let days = days_from_civil(year, 7, 2) - days_from_civil(1899, 12, 31);
            let t = days as f64 / 36_525.0;
            let n_deg =
                259.0 + 10.0 / 60.0 + 57.12 / 3600.0 - 1_934.142_397_2 * t + 7.58 / 3600.0 * t * t;
            let n = n_deg.rem_euclid(360.0).to_radians();
            let i = (0.913_70 - 0.035_69 * n.cos()).acos();
            let plus = (1.018_83 * (n / 2.0).sin()).atan2((n / 2.0).cos());
            let minus = (0.644_12 * (n / 2.0).sin()).atan2((n / 2.0).cos());
            let (nu, xi) = (plus - minus, n - plus - minus);
            let s2i = (2.0 * i).sin();
            let by_hand = [
                (Constituent::M2, (i / 2.0).cos().powi(4) / 0.9154),
                (Constituent::O1, i.sin() * (i / 2.0).cos().powi(2) / 0.3800),
                (
                    Constituent::K1,
                    (0.8965 * s2i * s2i + 0.6001 * s2i * nu.cos() + 0.1006).sqrt(),
                ),
            ];

            let args = AstronomicalArguments::mid_year(year);
            for (c, expected) in by_hand {
                let f = c.node_factor(&args);
                assert!(
                    (f - expected).abs() < 2e-4,
                    "{year} {c:?}: {f} vs {expected}"
                );
            }
            let u_m2 = (2.0 * xi - 2.0 * nu).to_degrees();
            assert_angle("u(M2)", Constituent::M2.nodal_phase(&args), u_m2, 0.01);
        }
    }

    /// SP98 Table 14, node factors for the middle of 1990 and 1996 (three decimals, worked
    /// from SP98's tables at tenths of a degree of `I` and whole degrees of `P`): K2 changes by
    /// 0.05 and OO1 by 0.16 per degree of `I`, L2 and M1 by up to 0.007 per degree of `P`.
    #[test]
    fn node_factors_match_sp98_table_14() {
        use Constituent::{
            J1, K1, K2, L2, M1, M2, M3, M4, M6, M8, Mf, Mk3, Mm, Mn4, Ms4, Msf, O1, Oo1, Q1, S2,
            TwoMk3, TwoSm2,
        };
        let order = [
            J1, K1, K2, L2, M1, M2, M3, M4, M6, M8, O1, Oo1, Mk3, TwoMk3, Mf, Mm,
        ];
        #[rustfmt::skip]
        let rows = [
            (1990, [1.120, 1.079, 1.203, 1.216, 1.334, 0.977, 0.966, 0.955, 0.932, 0.911, 1.128,
                1.505, 1.054, 1.030, 1.303, 0.918]),
            (1996, [0.833, 0.886, 0.754, 1.107, 1.083, 1.037, 1.056, 1.075, 1.115, 1.156, 0.812,
                0.498, 0.918, 0.952, 0.636, 1.128]),
        ];
        for (year, values) in rows {
            let args = AstronomicalArguments::mid_year(year);
            for (c, expected) in order.into_iter().zip(values) {
                let f = c.node_factor(&args);
                let tolerance = if matches!(c, L2 | M1 | Oo1) {
                    0.005
                } else {
                    0.003
                };
                assert!(
                    (f - expected).abs() <= tolerance,
                    "{year} {c:?}: {f:.4} vs {expected}"
                );
            }
            // Rows the table shares: MN with M4, Q1 with O1, MS, 2SM and MSf with M2.
            let shared = [(Mn4, M4), (Q1, O1), (Ms4, M2), (TwoSm2, M2), (Msf, M2)];
            for (c, like) in shared {
                let diff = (c.node_factor(&args) - like.node_factor(&args)).abs();
                assert!(diff < 1e-12, "{year} {c:?}");
            }
            assert!((S2.node_factor(&args) - 1.0).abs() < 1e-12);
        }
    }

    /// SP98 Table 15, `V0 + u` at Greenwich for 1 January 00:00 of 1990 and 1996, with `u` for
    /// the middle of the year (one decimal). For M1 the table uses the second form of note 1
    /// (formula 201: `V = T − s + h − 90°`, `u = ξ − ν + Q`), which differs from formula 194 by
    /// `p` at the instant against `p − Qu = ξ + Q` at mid-year (formula 205); its `Q` comes
    /// from Table 10 at whole degrees of `P`, where `Q` changes by up to 2° per degree.
    #[test]
    fn arguments_match_sp98_table_15() {
        use Constituent::{
            J1, K1, K2, L2, Lam2, M1, M2, M3, M4, M6, M8, Mf, Mk3, Mm, Mn4, Ms4, Msf, Mu2, N2, Nu2,
            O1, Oo1, P1, Q1, R2, Rho1, S1, S2, S4, S6, Sa, Ssa, T2, TwoMk3, TwoN2, TwoQ1, TwoSm2,
        };
        let order = [
            J1, K1, K2, L2, M1, M2, M3, M4, M6, M8, N2, TwoN2, O1, Oo1, P1, Q1, TwoQ1, R2, S1, S2,
            S4, S6, T2, Lam2, Mu2, Nu2, Rho1, Mk3, TwoMk3, Mn4, Ms4, TwoSm2, Mf, Msf, Mm, Sa, Ssa,
        ];
        #[rustfmt::skip]
        let rows = [
            (1990, [314.4, 16.7, 213.9, 2.2, 85.9, 259.4, 29.1, 158.7, 58.1, 317.5, 324.3, 29.2,
                240.1, 338.4, 349.6, 305.0, 9.9, 177.6, 180.0, 0.0, 0.0, 0.0, 2.4, 246.6, 157.1,
                92.2, 72.9, 276.0, 142.1, 223.6, 259.4, 100.6, 319.2, 100.6, 295.1, 280.4, 200.8]),
            (1996, [134.0, 12.2, 204.0, 66.7, 293.5, 118.3, 177.4, 236.6, 354.9, 113.2, 357.8,
                237.3, 104.8, 102.3, 350.1, 344.3, 223.8, 177.1, 180.0, 0.0, 0.0, 0.0, 2.9, 60.0,
                236.1, 356.6, 343.1, 130.5, 224.4, 116.1, 118.3, 241.7, 88.8, 241.7, 120.5, 279.9,
                199.9]),
        ];
        for (year, values) in rows {
            let start = AstronomicalArguments::at(year_start(year));
            let middle = AstronomicalArguments::mid_year(year);
            for (c, expected) in order.into_iter().zip(values) {
                let mut actual = c.equilibrium_argument(&start) + c.nodal_phase(&middle);
                let mut tolerance = 0.15;
                if c == M1 {
                    actual += middle.p - start.p;
                    tolerance = 1.0;
                }
                assert_angle(&format!("{year} {c:?}"), actual, expected, tolerance);
            }
        }
    }

    #[test]
    fn equilibrium_argument_advances_by_the_speed() {
        for start in [
            utc(2026, 11, 8, 0),
            utc(2034, 6, 1, 7),
            utc(1999, 12, 31, 23),
        ] {
            let now = AstronomicalArguments::at(start);
            let later = AstronomicalArguments::at(UnixTime(start.0 + 3600));
            for c in Constituent::ALL {
                let step = c.equilibrium_argument(&later) - c.equilibrium_argument(&now);
                assert_angle(&format!("{c:?}"), step, c.speed(), 1e-6);
            }
        }
    }

    #[test]
    fn yearly_corrections_are_taken_at_mid_year() {
        assert_eq!(mid_year(2026), utc(2026, 7, 2, 12));
        assert_eq!(mid_year(2028), utc(2028, 7, 2, 0));
        assert_eq!(utc_year(UnixTime(year_start(2034).0 - 1)), 2033);
        assert_eq!(utc_year(year_start(2034)), 2034);
        let table = yearly_node_corrections(2026);
        assert_eq!(table.len(), 37);
        let args = AstronomicalArguments::at(mid_year(2026));
        assert_eq!(
            table[&Constituent::K1],
            Constituent::K1.node_correction(&args)
        );
        assert!(
            table
                .values()
                .all(|c| c.f > 0.0 && (-180.0..=180.0).contains(&c.u))
        );
    }
}
