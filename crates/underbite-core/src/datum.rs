//! Vertical datums and heights measured against them.
//!
//! Heights are metres. A height in one datum is never compared with or merged into a height in
//! another; only a [`DatumTable`] of a single station converts between datums, and only for that
//! station.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::series::Series;

/// The reference surface a height is measured from.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum VerticalDatum {
    /// Mean lower-low water, the usual US chart datum.
    Mllw,
    /// Mean low water.
    Mlw,
    /// Mean sea level.
    Msl,
    /// Mean tide level.
    Mtl,
    /// North American Vertical Datum of 1988.
    Navd88,
    /// National Geodetic Vertical Datum of 1929.
    Ngvd29,
    /// International Great Lakes Datum 1985.
    Igld85,
    /// A station's own datum (CO-OPS `STND`, a USGS gage datum).
    Gauge,
    /// A model's mean sea level.
    ModelMean,
}

impl VerticalDatum {
    /// The name CO-OPS uses for this datum in its datums endpoint; `None` for
    /// [`VerticalDatum::ModelMean`], which no station table lists.
    pub fn coops_name(self) -> Option<&'static str> {
        Some(match self {
            Self::Mllw => "MLLW",
            Self::Mlw => "MLW",
            Self::Msl => "MSL",
            Self::Mtl => "MTL",
            Self::Navd88 => "NAVD88",
            Self::Ngvd29 => "NGVD29",
            Self::Igld85 => "IGLD85",
            Self::Gauge => "STND",
            Self::ModelMean => return None,
        })
    }
}

/// A height in metres above a datum.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Height {
    /// The datum the height is measured from.
    pub datum: VerticalDatum,
    /// Height above the datum, metres.
    pub metres: f32,
}

impl Height {
    /// `self - other` in metres, or `None` when the datums differ.
    pub fn checked_sub(&self, other: &Height) -> Option<f32> {
        (self.datum == other.datum).then_some(self.metres - other.metres)
    }

    /// Ordering of two heights, or `None` when the datums differ or a value is NaN.
    pub fn checked_cmp(&self, other: &Height) -> Option<std::cmp::Ordering> {
        if self.datum != other.datum {
            return None;
        }
        self.metres.partial_cmp(&other.metres)
    }

    /// This height raised by `delta_m` metres, in the same datum.
    #[must_use]
    pub fn offset(&self, delta_m: f32) -> Height {
        Height {
            datum: self.datum,
            metres: self.metres + delta_m,
        }
    }
}

/// A series of heights, metres above one datum.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Heights {
    /// The datum of every sample.
    pub datum: VerticalDatum,
    /// Heights above the datum, metres.
    pub series: Series<f32>,
}

/// The datum values of one station, as the CO-OPS datums endpoint lists them.
///
/// Values are metres above the station datum `STND`, keyed by the datum's name (`MSL`, `MLLW`,
/// `MTL`, `STND`, `NAVD88`, ...). Conversion works for that station only.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DatumTable {
    /// The station id as its source writes it.
    pub station_id: String,
    /// Datum name to metres above `STND`.
    pub values: BTreeMap<String, f32>,
}

impl DatumTable {
    /// A table for `station_id` from named values in metres above `STND`.
    pub fn new(station_id: impl Into<String>, values: BTreeMap<String, f32>) -> Self {
        Self {
            station_id: station_id.into(),
            values,
        }
    }

    /// A value by CO-OPS name.
    pub fn get(&self, name: &str) -> Option<f32> {
        self.values.get(name).copied()
    }

    /// Metres of `datum` above `STND`, if this station lists it.
    pub fn offset(&self, datum: VerticalDatum) -> Option<f32> {
        self.get(datum.coops_name()?)
    }

    /// `Z0(D) = MSL - D`: how far mean sea level stands above datum `D`, metres.
    pub fn z0(&self, datum: VerticalDatum) -> Option<f32> {
        Some(self.offset(VerticalDatum::Msl)? - self.offset(datum)?)
    }

    /// Re-expresses `height` in datum `to`, or `None` when this station lacks either datum.
    pub fn convert(&self, height: &Height, to: VerticalDatum) -> Option<Height> {
        let from = self.offset(height.datum)?;
        let target = self.offset(to)?;
        Some(Height {
            datum: to,
            metres: height.metres + (from - target),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Deserialize)]
    struct MdapiDatums {
        datums: Vec<MdapiDatum>,
    }

    #[derive(Deserialize)]
    struct MdapiDatum {
        name: String,
        value: Option<f32>,
    }

    fn battery() -> DatumTable {
        let raw = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/raw/coops/coops-mdapi-8518750-datums-metric.json"
        ));
        let parsed: MdapiDatums = serde_json::from_str(raw).unwrap();
        let values = parsed
            .datums
            .into_iter()
            .filter_map(|d| Some((d.name, d.value?)))
            .collect();
        DatumTable::new("8518750", values)
    }

    #[test]
    fn z0_for_mllw_at_the_battery() {
        let z0 = battery().z0(VerticalDatum::Mllw).unwrap();
        assert!((z0 - 0.783).abs() < 0.0005, "z0 = {z0}");
    }

    #[test]
    fn convert_between_datums_of_one_station() {
        let t = battery();
        let h = Height {
            datum: VerticalDatum::Mllw,
            metres: 0.783,
        };
        let back = t.convert(&h, VerticalDatum::Msl).unwrap();
        assert_eq!(back.datum, VerticalDatum::Msl);
        assert!(back.metres.abs() < 0.0005);
        let gauge = t.convert(&h, VerticalDatum::Gauge).unwrap();
        assert!((gauge.metres - 1.785).abs() < 0.0005);
    }

    #[test]
    fn missing_datum_does_not_convert() {
        let t = battery();
        let h = Height {
            datum: VerticalDatum::Mllw,
            metres: 1.0,
        };
        assert!(t.convert(&h, VerticalDatum::Igld85).is_none());
        assert!(t.convert(&h, VerticalDatum::ModelMean).is_none());
        assert!(t.z0(VerticalDatum::ModelMean).is_none());
    }

    #[test]
    fn heights_in_different_datums_do_not_combine() {
        let a = Height {
            datum: VerticalDatum::Mllw,
            metres: 2.0,
        };
        let b = Height {
            datum: VerticalDatum::Navd88,
            metres: 1.0,
        };
        let c = Height {
            datum: VerticalDatum::Mllw,
            metres: 0.5,
        };
        assert_eq!(a.checked_sub(&b), None);
        assert_eq!(a.checked_cmp(&b), None);
        assert_eq!(a.checked_sub(&c), Some(1.5));
        assert_eq!(a.checked_cmp(&c), Some(std::cmp::Ordering::Greater));
        assert_eq!(a.offset(1.0).metres, 3.0);
    }

    #[test]
    fn schema_snapshot() {
        let schemas = serde_json::json!({
            "VerticalDatum": schemars::schema_for!(VerticalDatum),
            "Height": schemars::schema_for!(Height),
            "Heights": schemars::schema_for!(Heights),
            "DatumTable": schemars::schema_for!(DatumTable),
        });
        insta::assert_json_snapshot!(schemas);
    }
}
