//! Tides, currents, waves, sea temperature, station observations and NWS marine text.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::datum::VerticalDatum;
use crate::provenance::Provenance;
use crate::series::{Series, Vector};
use crate::time::UnixTime;

/// How a tide series was produced.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TideSourceKind {
    HarmonicStation,
    SubordinateStation,
    Computed,
    /// A model's sea level relative to its own mean: shown without heights.
    ModelMean,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TideKind {
    High,
    Low,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TideEvent {
    pub time: UnixTime,
    pub kind: TideKind,
    /// Absent for `ModelMean`, whose curve has no datum heights.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height_m: Option<f32>,
}

/// Tide predictions (or a modelled curve) with the datum their heights are against.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TideSeries {
    pub source_kind: TideSourceKind,
    pub datum: VerticalDatum,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<TideEvent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub curve: Option<Series<f32>>,
    /// Observed water level in the same datum.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed: Option<Series<f32>>,
    pub provenance: Provenance,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CurrentKind {
    MaxFlood,
    MaxEbb,
    Slack,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CurrentEvent {
    pub time: UnixTime,
    pub kind: CurrentKind,
    /// Signed along the major axis, positive toward flood.
    pub speed_ms: f32,
}

/// Tidal current predictions and observations at a station.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CurrentSeries {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<CurrentEvent>,
    /// Signed major-axis speed in m/s, positive toward flood.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub curve: Option<Series<f32>>,
    /// Direction the flood sets toward, degrees true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flood_dir_deg: Option<f32>,
    /// Direction the ebb sets toward, degrees true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ebb_dir_deg: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bin: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth_m: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed: Option<Series<Vector>>,
    pub provenance: Provenance,
}

/// A wave direction: degrees, or a compass point where the source gives only text (NDBC).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WaveDirection {
    /// Degrees true the waves come from.
    Degrees(f32),
    /// A compass point such as `SSE` the waves come from.
    Compass(String),
}

/// One wave train: wind waves or a swell.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WaveTrain {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height_m: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub period_s: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction_from: Option<WaveDirection>,
}

/// The sea state at one time.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WaveSample {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub significant_height_m: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dominant_period_s: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mean_period_s: Option<f32>,
    /// Degrees true the waves come from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction_from_deg: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wind_waves: Option<WaveTrain>,
    /// Zero to three swell trains.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub swells: Vec<WaveTrain>,
    /// NDBC's steepness text (`VERY_STEEP`, `STEEP`, `AVERAGE`, `N/A`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steepness: Option<String>,
}

/// Waves over time from one source.
pub type WaveSeries = Series<WaveSample>;

/// Wind at a station.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Wind {
    pub speed_ms: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gust_ms: Option<f32>,
    /// Degrees true the wind blows from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_deg: Option<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ZoneKind {
    Coastal,
    GreatLakesNearshore,
    GreatLakesOpen,
    Offshore,
}

/// The NWS product a forecast came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ZoneProduct {
    ZoneApi,
    Cwf,
    Nsh,
    Glf,
    Off,
}

/// One named period of a text forecast.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ForecastPeriod {
    pub name: String,
    pub text: String,
}

/// An NWS marine zone forecast: text only, never parsed into numbers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ZoneForecast {
    pub id: String,
    pub name: String,
    pub kind: ZoneKind,
    pub issued: UnixTime,
    pub product: ZoneProduct,
    pub periods: Vec<ForecastPeriod>,
}

/// NWS alert severity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AlertSeverity {
    Extreme,
    Severe,
    Moderate,
    Minor,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MarineAlert {
    pub id: String,
    /// NWS event name, for example `Small Craft Advisory`.
    pub event: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headline: Option<String>,
    /// The NWS description as issued (paragraphs kept), for the full form.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The NWS instruction as issued.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instruction: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub onset: Option<UnixTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ends: Option<UnixTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires: Option<UnixTime>,
    pub severity: AlertSeverity,
    /// UGC zone ids.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub zones: Vec<String>,
    /// Office, phenomenon, significance and number, to de-duplicate with land alerts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vtec_key: Option<String>,
}

/// Everything about the water's surface and the sea at the spot.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MarineConditions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tide: Option<TideSeries>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tidal_current: Option<CurrentSeries>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ocean_current: Option<Series<Vector>>,
    /// Observed waves (a buoy).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waves: Option<WaveSeries>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sea_surface_temp_c: Option<Series<f32>>,
    /// Modelled waves ahead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waves_forecast: Option<WaveSeries>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sea_surface_temp_forecast_c: Option<Series<f32>>,
    /// Wind observed at a station, apart from whether's weather.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_wind: Option<Series<Wind>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_pressure_hpa: Option<Series<f32>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub marine_forecast: Option<ZoneForecast>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub marine_alerts: Vec<MarineAlert>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_marine_serializes_to_an_empty_object() {
        assert_eq!(
            serde_json::to_string(&MarineConditions::default()).unwrap(),
            "{}"
        );
    }

    #[test]
    fn wave_direction_keeps_degrees_or_compass() {
        let t = WaveTrain {
            height_m: Some(0.4),
            period_s: None,
            direction_from: Some(WaveDirection::Compass("SSE".into())),
        };
        let json = serde_json::to_string(&t).unwrap();
        assert_eq!(
            json,
            "{\"height_m\":0.4,\"direction_from\":{\"compass\":\"SSE\"}}"
        );
        assert_eq!(serde_json::from_str::<WaveTrain>(&json).unwrap(), t);
    }
}
