//! Canonical units and conversions to display units.
//!
//! Inside the core everything is metres, m/s, m³/s, °C and hPa. Display units are chosen per
//! measurement group in the configuration; conversions take and return `f32` but compute in `f64`.
//! Unit strings written by data sources parse into a [`SourceUnit`]; an unknown string is an
//! error, never a guess, and turbidity units are labels that are never converted.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub use whether_core::units::{DistanceUnit, PressureUnit, WindUnit};

/// Metres per international foot: exactly 0.3048 by definition.
const M_PER_FT: f64 = 0.3048;
/// Cubic metres per second per cubic foot per second: 0.3048³, exactly.
const M3S_PER_CFS: f64 = 0.028_316_846_592;
/// Metres per second per knot: one nautical mile (1852 m) per hour, exactly 1852/3600.
const MS_PER_KN: f64 = 1852.0 / 3600.0;
/// Metres per second per mile per hour: 1609.344 m per 3600 s, exactly.
const MS_PER_MPH: f64 = 1609.344 / 3600.0;
/// Metres per second per kilometre per hour: 1000/3600.
const MS_PER_KMH: f64 = 1000.0 / 3600.0;
/// Metres per nautical mile, exactly.
const M_PER_NMI: f64 = 1852.0;
/// Hectopascals per inch of mercury (conventional, 3386.389 Pa).
const HPA_PER_INHG: f64 = 33.863_89;
/// Hectopascals per millimetre of mercury (conventional, 133.322387415 Pa).
const HPA_PER_MMHG: f64 = 1.333_223_874_15;
/// Hectopascals per kilopascal.
const HPA_PER_KPA: f64 = 10.0;

fn wide(v: f32) -> f64 {
    f64::from(v)
}

// Converted values are within f32 range, so narrowing loses only precision.
#[allow(clippy::cast_possible_truncation)]
fn narrow(v: f64) -> f32 {
    v as f32
}

/// Discharge display unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum DischargeUnit {
    /// Cubic feet per second.
    Cfs,
    /// Cubic metres per second.
    M3s,
}

impl DischargeUnit {
    /// Every unit, in the order a picker lists them.
    pub const ALL: [Self; 2] = [Self::Cfs, Self::M3s];

    /// Its symbol: "cfs" or "m³/s".
    pub const fn symbol(self) -> &'static str {
        match self {
            Self::Cfs => "cfs",
            Self::M3s => "m³/s",
        }
    }

    /// Converts m³/s to this unit.
    pub fn from_canonical(self, m3s: f32) -> f32 {
        match self {
            Self::Cfs => narrow(wide(m3s) / M3S_PER_CFS),
            Self::M3s => m3s,
        }
    }

    /// Converts a value in this unit to m³/s.
    pub fn to_canonical(self, v: f32) -> f32 {
        match self {
            Self::Cfs => narrow(wide(v) * M3S_PER_CFS),
            Self::M3s => v,
        }
    }
}

/// Height and depth display unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum HeightUnit {
    /// Feet.
    Ft,
    /// Metres.
    M,
}

impl HeightUnit {
    /// Every unit, in the order a picker lists them.
    pub const ALL: [Self; 2] = [Self::Ft, Self::M];

    /// Its symbol: "ft" or "m".
    pub const fn symbol(self) -> &'static str {
        match self {
            Self::Ft => "ft",
            Self::M => "m",
        }
    }

    /// Converts metres to this unit.
    pub fn from_canonical(self, m: f32) -> f32 {
        match self {
            Self::Ft => narrow(wide(m) / M_PER_FT),
            Self::M => m,
        }
    }

    /// Converts a value in this unit to metres.
    pub fn to_canonical(self, v: f32) -> f32 {
        match self {
            Self::Ft => narrow(wide(v) * M_PER_FT),
            Self::M => v,
        }
    }
}

/// Speed display unit for currents and wind (each has its own setting).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SpeedUnit {
    /// Knots.
    Kn,
    /// Miles per hour.
    Mph,
    /// Kilometres per hour.
    Kmh,
    /// Metres per second.
    Ms,
}

impl SpeedUnit {
    /// Every unit, in the order a picker lists them.
    pub const ALL: [Self; 4] = [Self::Kn, Self::Mph, Self::Kmh, Self::Ms];

    /// Its symbol: "kn", "mph", "km/h" or "m/s".
    pub const fn symbol(self) -> &'static str {
        match self {
            Self::Kn => "kn",
            Self::Mph => "mph",
            Self::Kmh => "km/h",
            Self::Ms => "m/s",
        }
    }

    /// The matching whether wind unit, for weather values.
    pub const fn to_wind(self) -> WindUnit {
        match self {
            Self::Kn => WindUnit::Knots,
            Self::Mph => WindUnit::Mph,
            Self::Kmh => WindUnit::Kmh,
            Self::Ms => WindUnit::Mps,
        }
    }

    const fn ms_per_unit(self) -> f64 {
        match self {
            Self::Kn => MS_PER_KN,
            Self::Mph => MS_PER_MPH,
            Self::Kmh => MS_PER_KMH,
            Self::Ms => 1.0,
        }
    }

    /// Converts m/s to this unit.
    pub fn from_canonical(self, ms: f32) -> f32 {
        narrow(wide(ms) / self.ms_per_unit())
    }

    /// Converts a value in this unit to m/s.
    pub fn to_canonical(self, v: f32) -> f32 {
        narrow(wide(v) * self.ms_per_unit())
    }
}

/// Temperature display unit for water and air.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub enum TempUnit {
    /// Degrees Celsius.
    C,
    /// Degrees Fahrenheit.
    F,
}

impl TempUnit {
    /// Every unit, in the order a picker lists them.
    pub const ALL: [Self; 2] = [Self::F, Self::C];

    /// Its symbol: "°C" or "°F".
    pub const fn symbol(self) -> &'static str {
        match self {
            Self::C => "°C",
            Self::F => "°F",
        }
    }

    /// The matching whether temperature unit, for weather values.
    pub const fn to_whether(self) -> whether_core::units::TemperatureUnit {
        match self {
            Self::C => whether_core::units::TemperatureUnit::Celsius,
            Self::F => whether_core::units::TemperatureUnit::Fahrenheit,
        }
    }

    /// Converts °C to this unit.
    pub fn from_canonical(self, c: f32) -> f32 {
        match self {
            Self::C => c,
            Self::F => narrow(wide(c) * 9.0 / 5.0 + 32.0),
        }
    }

    /// Converts a value in this unit to °C.
    pub fn to_canonical(self, v: f32) -> f32 {
        match self {
            Self::C => v,
            Self::F => narrow((wide(v) - 32.0) * 5.0 / 9.0),
        }
    }
}

/// Converts hPa to a pressure display unit.
pub fn pressure_from_hpa(unit: PressureUnit, hpa: f32) -> f32 {
    narrow(wide(hpa) / hpa_per(unit))
}

/// Converts a value in a pressure display unit to hPa.
pub fn pressure_to_hpa(unit: PressureUnit, v: f32) -> f32 {
    narrow(wide(v) * hpa_per(unit))
}

fn hpa_per(unit: PressureUnit) -> f64 {
    match unit {
        PressureUnit::Hpa => 1.0,
        PressureUnit::InHg => HPA_PER_INHG,
        PressureUnit::MmHg => HPA_PER_MMHG,
        PressureUnit::Kpa => HPA_PER_KPA,
    }
}

/// A turbidity unit label.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub enum TurbidityUnit {
    /// Formazin nephelometric units.
    Fnu,
    /// Formazin turbidity units.
    Ftu,
    /// Nephelometric turbidity units.
    Ntu,
}

impl TurbidityUnit {
    /// Its label: "FNU", "FTU" or "NTU".
    pub const fn label(self) -> &'static str {
        match self {
            Self::Fnu => "FNU",
            Self::Ftu => "FTU",
            Self::Ntu => "NTU",
        }
    }
}

/// A unit string as sources write it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceUnit {
    /// Cubic feet per second.
    Cfs,
    /// Thousands of cubic feet per second.
    Kcfs,
    /// Cubic metres per second.
    M3s,
    /// Feet.
    Ft,
    /// Metres.
    M,
    /// Nautical miles.
    Nmi,
    /// Centimetres per second.
    Cms,
    /// Metres per second.
    Ms,
    /// Kilometres per hour.
    Kmh,
    /// Miles per hour.
    Mph,
    /// Knots.
    Kn,
    /// Degrees Celsius.
    DegC,
    /// Degrees Fahrenheit.
    DegF,
    /// Hectopascals.
    Hpa,
    /// Turbidity: a label, never converted.
    Turbidity(TurbidityUnit),
}

/// A value in canonical units, tagged by its measurement group.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Canonical {
    /// m³/s.
    Discharge(f32),
    /// Metres.
    Length(f32),
    /// m/s.
    Speed(f32),
    /// °C.
    Temperature(f32),
    /// hPa.
    Pressure(f32),
    /// Turbidity as written, with its unit label.
    Turbidity(f32, TurbidityUnit),
}

/// An unrecognised unit string.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown unit {0:?}")]
pub struct UnknownUnit(pub String);

impl SourceUnit {
    /// Parses a unit string from a source (`ft^3/s`, `ft3/s`, `ft³/s`, `degC`, `deg C`, `_FNU`,
    /// `kcfs`, `km/h`, `mp/h`, `kn`, ...). Surrounding whitespace is ignored.
    ///
    /// # Errors
    /// [`UnknownUnit`] for any string not listed, including `undefined`.
    pub fn parse(s: &str) -> Result<Self, UnknownUnit> {
        Ok(match s.trim() {
            "ft^3/s" | "ft3/s" | "ft³/s" | "cfs" => Self::Cfs,
            "kcfs" => Self::Kcfs,
            "m^3/s" | "m3/s" | "m³/s" | "cms" => Self::M3s,
            "ft" => Self::Ft,
            "m" => Self::M,
            "nmi" => Self::Nmi,
            "cm/s" => Self::Cms,
            "m/s" => Self::Ms,
            "km/h" | "kmh" | "kph" => Self::Kmh,
            "mp/h" | "mph" | "mi/h" => Self::Mph,
            "kn" | "kt" | "kts" => Self::Kn,
            "degC" | "deg C" | "°C" | "C" => Self::DegC,
            "degF" | "deg F" | "°F" | "F" => Self::DegF,
            "hPa" | "mb" | "mbar" => Self::Hpa,
            "_FNU" | "FNU" => Self::Turbidity(TurbidityUnit::Fnu),
            "_FTU" | "FTU" => Self::Turbidity(TurbidityUnit::Ftu),
            "_NTU" | "NTU" => Self::Turbidity(TurbidityUnit::Ntu),
            other => return Err(UnknownUnit(other.to_owned())),
        })
    }

    /// Converts a value in this unit to its canonical unit.
    pub fn to_canonical(self, v: f32) -> Canonical {
        match self {
            Self::Cfs => Canonical::Discharge(DischargeUnit::Cfs.to_canonical(v)),
            Self::Kcfs => Canonical::Discharge(narrow(wide(v) * 1000.0 * M3S_PER_CFS)),
            Self::M3s => Canonical::Discharge(v),
            Self::Ft => Canonical::Length(HeightUnit::Ft.to_canonical(v)),
            Self::M => Canonical::Length(v),
            Self::Nmi => Canonical::Length(narrow(wide(v) * M_PER_NMI)),
            Self::Cms => Canonical::Speed(narrow(wide(v) / 100.0)),
            Self::Ms => Canonical::Speed(v),
            Self::Kmh => Canonical::Speed(SpeedUnit::Kmh.to_canonical(v)),
            Self::Mph => Canonical::Speed(SpeedUnit::Mph.to_canonical(v)),
            Self::Kn => Canonical::Speed(SpeedUnit::Kn.to_canonical(v)),
            Self::DegC => Canonical::Temperature(v),
            Self::DegF => Canonical::Temperature(TempUnit::F.to_canonical(v)),
            Self::Hpa => Canonical::Pressure(v),
            Self::Turbidity(t) => Canonical::Turbidity(v, t),
        }
    }
}

/// Parses `unit` and converts `value` to canonical units.
///
/// # Errors
/// [`UnknownUnit`] when the unit string is not recognised.
pub fn to_canonical(value: f32, unit: &str) -> Result<Canonical, UnknownUnit> {
    SourceUnit::parse(unit).map(|u| u.to_canonical(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32, tol: f32) {
        assert!((a - b).abs() <= tol, "{a} vs {b}");
    }

    #[test]
    fn cfs_to_cubic_metres() {
        close(DischargeUnit::Cfs.to_canonical(12.4), 0.3511, 0.0001);
        close(DischargeUnit::Cfs.from_canonical(0.3511), 12.4, 0.01);
    }

    #[test]
    fn kcfs_to_cfs() {
        let Ok(Canonical::Discharge(m3s)) = to_canonical(0.204, "kcfs") else {
            panic!("not discharge");
        };
        close(DischargeUnit::Cfs.from_canonical(m3s), 204.0, 0.01);
    }

    #[test]
    fn centimetres_per_second_to_knots() {
        let Ok(Canonical::Speed(ms)) = to_canonical(117.5, "cm/s") else {
            panic!("not speed");
        };
        close(ms, 1.175, 1e-6);
        close(SpeedUnit::Kn.from_canonical(ms), 2.28, 0.005);
    }

    #[test]
    fn length_temperature_pressure() {
        close(HeightUnit::Ft.to_canonical(10.0), 3.048, 1e-6);
        close(HeightUnit::Ft.from_canonical(3.048), 10.0, 1e-5);
        close(TempUnit::F.from_canonical(100.0), 212.0, 1e-4);
        close(TempUnit::F.to_canonical(32.0), 0.0, 1e-6);
        close(SpeedUnit::Kmh.to_canonical(36.0), 10.0, 1e-5);
        close(SpeedUnit::Mph.to_canonical(1.0), 0.44704, 1e-6);
        close(pressure_from_hpa(PressureUnit::InHg, 1013.25), 29.92, 0.005);
        close(pressure_to_hpa(PressureUnit::Kpa, 101.325), 1013.25, 1e-3);
    }

    #[test]
    fn source_strings() {
        for s in ["ft^3/s", "ft3/s", "ft³/s", "cfs"] {
            assert_eq!(SourceUnit::parse(s), Ok(SourceUnit::Cfs), "{s}");
        }
        for (s, u) in [
            ("degC", SourceUnit::DegC),
            ("deg C", SourceUnit::DegC),
            ("°F", SourceUnit::DegF),
            ("km/h", SourceUnit::Kmh),
            ("mp/h", SourceUnit::Mph),
            ("kn", SourceUnit::Kn),
            ("nmi", SourceUnit::Nmi),
            ("hPa", SourceUnit::Hpa),
        ] {
            assert_eq!(SourceUnit::parse(s), Ok(u), "{s}");
        }
        assert_eq!(
            to_canonical(3.5, "_FNU"),
            Ok(Canonical::Turbidity(3.5, TurbidityUnit::Fnu))
        );
        assert_eq!(to_canonical(2.0, "nmi"), Ok(Canonical::Length(3704.0)));
    }

    #[test]
    fn unknown_strings_are_errors() {
        for s in ["undefined", "furlongs", "", "uS/cm"] {
            assert!(SourceUnit::parse(s).is_err(), "{s}");
        }
    }

    #[test]
    fn config_names() {
        let json = |v: &dyn erased::Ser| v.json();
        assert_eq!(json(&SpeedUnit::Kmh), "\"kmh\"");
        assert_eq!(json(&SpeedUnit::Ms), "\"ms\"");
        assert_eq!(json(&DischargeUnit::M3s), "\"m3s\"");
        assert_eq!(json(&TempUnit::F), "\"F\"");
        assert_eq!(SpeedUnit::Ms.to_wind(), WindUnit::Mps);
    }

    mod erased {
        pub(super) trait Ser {
            fn json(&self) -> String;
        }
        impl<T: serde::Serialize> Ser for T {
            fn json(&self) -> String {
                serde_json::to_string(self).unwrap_or_default()
            }
        }
    }
}
