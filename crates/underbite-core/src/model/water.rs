//! Freshwater and water-quality conditions: flow, level, temperature, clarity, flow vs normal.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::datum::{Height, Heights, VerticalDatum};
use crate::provenance::Provenance;
use crate::series::Series;
use crate::time::{LocalDate, UnixTime};

/// A link to the source's own page or data for a value, shown beside it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ExternalLink {
    /// Plain label ("USACE Philadelphia District: Beltzville Lake").
    pub label: String,
    pub url: String,
}

/// A reservoir project's releases and inflow. The pool level and its percentile are the
/// water's `level` and `level_percentile_for_date`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ReservoirInfo {
    /// The project's name ("Beltzville Lake").
    pub project: String,
    /// Release below the dam, m³/s; absent where the operator publishes none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outflow_m3s: Option<Series<f32>>,
    /// Inflow to the pool, m³/s.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inflow_m3s: Option<Series<f32>>,
    /// The operator's pages, for what the data does not carry (release schedules).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<ExternalLink>,
}

/// NOAA's respiratory-irritation level at a beach.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum HabLevel {
    VeryLow,
    Low,
    Moderate,
    High,
    /// A category the source added that this version does not know.
    Unknown,
}

/// A NOAA harmful-algal-bloom forecast for the nearest listed beach.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct HabForecast {
    /// The beach's name as the source writes it.
    pub location: String,
    pub level: HabLevel,
    /// The source's confidence weighting, 0 to 1; absent when the source gives none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
    /// The time the forecast is valid for.
    pub valid_at: UnixTime,
    /// From the spot to the beach, metres.
    pub distance_m: f32,
    /// Where the forecast came from (a model run, so `Modelled`), for its credit and age.
    pub provenance: Provenance,
}

/// Bloom data for the spot's lake or coast. The `CyAN` mean stays in `cyanobacteria_cells_ml`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BloomInfo {
    /// Maximum cell concentration over the lake's valid cells, cells/mL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cyan_max_cells_ml: Option<Series<f32>>,
    /// Number of 300 m cells with a valid retrieval, to judge how much of the lake was seen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cyan_valid_cells: Option<u32>,
    /// Date of the satellite composite the `CyAN` values come from, so a display can say how old.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cyan_image_date: Option<LocalDate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hab: Option<HabForecast>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<ExternalLink>,
}

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
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FloodCategories {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<Height>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minor: Option<Height>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub moderate: Option<Height>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub major: Option<Height>,
    /// The gauge that defines them, so a display can name and credit it and check it is the
    /// spot's own; absent in files written before it was recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<Provenance>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reservoir: Option<ReservoirInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bloom: Option<BloomInfo>,
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
