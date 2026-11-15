//! Freshwater and water-quality conditions: flow, level, temperature, clarity, flow vs normal.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::datum::{Height, Heights, VerticalDatum};
use crate::provenance::Provenance;
use crate::series::Series;

/// The unit a turbidity was measured in. The units are different methods and never convert.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum TurbidityUnit {
    Fnu,
    Ftu,
    Ntu,
}

/// One turbidity reading with the unit it was measured in.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Turbidity {
    pub value: f32,
    pub unit: TurbidityUnit,
}

/// NWPS flood stage thresholds, each absent when the gauge does not define it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FloodCategories {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<Height>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minor: Option<Height>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub moderate: Option<Height>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub major: Option<Height>,
}

/// Where a [`Percentile`] came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PercentileBasis {
    /// The USGS Statistics API's day-of-year normals.
    StatisticsApi,
    /// Computed on the device from the daily record of `years` years.
    ComputedFromDaily { years: u32 },
}

/// WaterWatch-style class of a value among its day-of-year percentiles. The bounds live in
/// the derivation, not here.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum PercentileClass {
    MuchBelowNormal,
    BelowNormal,
    Normal,
    AboveNormal,
    MuchAboveNormal,
    // Below the lowest value on record for this day of the year.
    RecordLow,
    // Above the highest value on record for this day of the year.
    RecordHigh,
}

/// The 5, 10, 25, 50, 75, 90 and 95th percentile values for a day of year.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PercentileThresholds {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p05: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p10: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p25: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p50: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p75: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p90: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p95: Option<f32>,
}

/// A current value placed among the normals for its day of year. Values are in the
/// canonical unit of the quantity (m³/s for discharge, metres for level).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Percentile {
    /// Day of year as `MM-DD`.
    pub day_of_year: String,
    pub thresholds: PercentileThresholds,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mean: Option<f32>,
    pub value: f32,
    /// Percentile rank of `value` (0 to 100), interpolated between the thresholds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rank: Option<f32>,
    /// True when the true rank lies beyond `rank`: the value is outside the 5th to 95th
    /// percentile band and no `min` or `max` bounds it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub rank_clamped: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class: Option<PercentileClass>,
    pub basis: PercentileBasis,
    /// Number of years in the sample.
    pub sample_count: u32,
    pub approved_only: bool,
    /// Vertical datum of the values; set for level percentiles, absent for discharge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub datum: Option<VerticalDatum>,
    /// The station and source the normals came from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<Provenance>,
}

/// Water conditions at the spot's stations. Every series carries its own provenance.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WaterConditions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature_c: Option<Series<f32>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discharge_m3s: Option<Series<f32>>,
    /// Stage above the gauge's datum (`Gauge`, `Ngvd29` or `Navd88` as reported).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gage_height: Option<Heights>,
    /// Lake or reservoir level.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<Heights>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turbidity: Option<Series<Turbidity>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dissolved_oxygen_mg_l: Option<Series<f32>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conductivity_us_cm: Option<Series<f32>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub salinity_psu: Option<Series<f32>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ph: Option<Series<f32>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chlorophyll_ug_l: Option<Series<f32>>,
    /// Diffuse attenuation coefficient at 490 nm, per metre.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clarity_kd490_per_m: Option<Series<f32>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cyanobacteria_cells_ml: Option<Series<f32>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discharge_forecast_m3s: Option<Series<f32>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage_forecast: Option<Heights>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flood_categories: Option<FloodCategories>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discharge_percentile_for_date: Option<Percentile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level_percentile_for_date: Option<Percentile>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_water_serializes_to_an_empty_object() {
        let json = serde_json::to_string(&WaterConditions::default()).unwrap();
        assert_eq!(json, "{}");
        assert_eq!(
            serde_json::from_str::<WaterConditions>("{}").unwrap(),
            WaterConditions::default()
        );
    }

    #[test]
    fn names_are_snake_case() {
        assert_eq!(
            serde_json::to_string(&PercentileClass::MuchAboveNormal).unwrap(),
            "\"much_above_normal\""
        );
        assert_eq!(
            serde_json::to_string(&PercentileClass::RecordLow).unwrap(),
            "\"record_low\""
        );
        assert_eq!(
            serde_json::to_string(&PercentileClass::RecordHigh).unwrap(),
            "\"record_high\""
        );
        assert_eq!(
            serde_json::to_string(&PercentileBasis::ComputedFromDaily { years: 16 }).unwrap(),
            "{\"computed_from_daily\":{\"years\":16}}"
        );
    }
}
