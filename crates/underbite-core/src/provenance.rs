//! Where a value came from: source, station, distance, kind and quality.
//!
//! Every value the app shows carries a [`Provenance`] so the UI can print
//! "Hudson R at Battery, NY, 0.4 mi, 6 min ago" and grey out values that are too old or too far.

use std::borrow::Cow;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::time::UnixTime;

/// Identifier of a data source, such as `usgs` or `open-meteo-marine`.
///
/// A string newtype rather than an enum so a new source does not change the contract; the
/// configuration validates ids against the provider registry. Values are kebab-case.
#[derive(
    Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct SourceId(Cow<'static, str>);

impl SourceId {
    /// USGS Water Data APIs.
    pub const USGS: SourceId = SourceId::from_static("usgs");
    /// NOAA CO-OPS data API.
    pub const COOPS: SourceId = SourceId::from_static("coops");
    /// Tides or currents computed on device from CO-OPS harmonic constituents.
    pub const COOPS_HARMONICS: SourceId = SourceId::from_static("coops-harmonics");
    /// NOAA National Data Buoy Center.
    pub const NDBC: SourceId = SourceId::from_static("ndbc");
    /// NOAA National Water Prediction Service.
    pub const NWPS: SourceId = SourceId::from_static("nwps");
    /// Open-Meteo marine forecast model.
    pub const OPEN_METEO_MARINE: SourceId = SourceId::from_static("open-meteo-marine");
    /// US National Weather Service.
    pub const NWS: SourceId = SourceId::from_static("nws");
    /// USGS 3D Hydrography Program network.
    pub const USGS_3DHP: SourceId = SourceId::from_static("usgs-3dhp");
    /// US Army Corps of Engineers water management (CWMS Data API).
    pub const CWMS: SourceId = SourceId::from_static("cwms");
    /// US Bureau of Reclamation RISE.
    pub const RISE: SourceId = SourceId::from_static("rise");
    /// EPA Cyanobacteria Assessment Network.
    pub const CYAN: SourceId = SourceId::from_static("cyan");
    /// NOAA NCCOS harmful algal bloom forecasts.
    pub const NOAA_HAB: SourceId = SourceId::from_static("noaa-hab");
    /// New York State DEC fish stocking lists.
    pub const NY_DEC_STOCKING: SourceId = SourceId::from_static("ny-dec-stocking");
    /// OpenStreetMap through the Overpass API.
    pub const OVERPASS: SourceId = SourceId::from_static("overpass");
    /// USGS Protected Areas Database of the United States.
    pub const PADUS: SourceId = SourceId::from_static("padus");
    /// Global Biodiversity Information Facility.
    pub const GBIF: SourceId = SourceId::from_static("gbif");
    /// iNaturalist.
    pub const INATURALIST: SourceId = SourceId::from_static("inaturalist");
    /// USGS Nonindigenous Aquatic Species.
    pub const USGS_NAS: SourceId = SourceId::from_static("usgs-nas");
    /// Fisheries and Oceans Canada, Canadian Hydrographic Service: Integrated Water Level System.
    pub const DFO_IWLS: SourceId = SourceId::from_static("dfo-iwls");
    /// Environment and Climate Change Canada hydrometric data (real-time and HYDAT).
    pub const ECCC_HYDROMETRIC: SourceId = SourceId::from_static("eccc-hydrometric");

    /// A compile-time id.
    pub const fn from_static(id: &'static str) -> Self {
        Self(Cow::Borrowed(id))
    }

    /// An id built at run time.
    pub fn new(id: impl Into<String>) -> Self {
        Self(Cow::Owned(id.into()))
    }

    /// The id as written in configuration and files.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SourceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Type of a USGS monitoring site (`site_type_code`).
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum UsgsSiteType {
    /// `ST`: stream.
    Stream,
    /// `ES`: estuary.
    Estuary,
    /// `LK`: lake or reservoir.
    Lake,
    /// `ST-TS`: tidal stream.
    TidalStream,
    /// Any other site type code.
    Other,
}

/// What kind of station a [`StationRef`] names.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum StationKind {
    /// CO-OPS harmonic tide prediction station.
    TideHarmonic,
    /// CO-OPS subordinate tide prediction station.
    TideSubordinate,
    /// CO-OPS harmonic current prediction station (`H`).
    CurrentHarmonic,
    /// CO-OPS subordinate current prediction station (`S`).
    CurrentSubordinate,
    /// CO-OPS weak-and-variable current station (`W`).
    CurrentWeakVariable,
    /// A water-level gauge.
    WaterLevel,
    /// A PORTS current meter.
    PortsCurrentMeter,
    /// A moored or drifting buoy.
    Buoy,
    /// A C-MAN coastal station.
    Cman,
    /// A USGS site with its site type.
    UsgsSite(UsgsSiteType),
    /// An NWPS river gauge.
    NwpsGauge,
}

/// A station as its source describes it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StationRef {
    /// The source that owns the station.
    pub source: SourceId,
    /// The id exactly as the source writes it (`8518750`, `NYH1927`, `USGS-01375000`, `44025`).
    pub id: String,
    /// Display name.
    pub name: String,
    /// Station type.
    pub kind: StationKind,
    /// Latitude, degrees north.
    pub latitude: f64,
    /// Longitude, degrees east.
    pub longitude: f64,
    /// CO-OPS current bin.
    pub bin: Option<u16>,
    /// Reference station of a subordinate station.
    pub reference_id: Option<String>,
}

/// How the station relates to the spot.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Relation {
    /// On the same water body as the spot.
    SameWaterBody,
    /// Upstream of the spot on the same river.
    Upstream,
    /// Downstream of the spot on the same river.
    Downstream,
    /// Close by, water body not established.
    Nearby,
    /// A model grid cell.
    Gridded,
    /// A satellite pixel.
    Satellite,
}

/// How the value was obtained.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ValueKind {
    /// Measured.
    Observed,
    /// Predicted from harmonics or tables.
    Predicted,
    /// Output of a forecast or analysis model.
    Modelled,
    /// Retrieved from satellite imagery.
    Satellite,
    /// Derived on device from other values.
    Computed,
}

/// Quality of a value, as far as the source says.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Quality {
    /// Subject to revision.
    Provisional,
    /// Reviewed and approved.
    Approved,
    /// Estimated by the source (USGS `ESTIMATED`).
    Estimated,
    /// Affected by ice (USGS `ICE`).
    IceAffected,
    /// Unreliable: equipment or maintenance (USGS `EQUIP`, `MAINT`).
    Flagged,
}

/// Where one value or series came from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Provenance {
    /// The data source.
    pub source: SourceId,
    /// The station, when the value comes from one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub station: Option<StationRef>,
    /// Distance from the spot to the station, metres.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distance_m: Option<f32>,
    /// How the station relates to the spot.
    pub relation: Relation,
    /// When the value was observed, predicted for or issued.
    pub observed_at: UnixTime,
    /// Observed, predicted, modelled, satellite or computed.
    pub kind: ValueKind,
    /// Source-reported quality.
    pub quality: Quality,
}

/// The credit a display owes a source.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Attribution {
    /// The source credited.
    pub source: SourceId,
    /// Credit text as the source requires it.
    pub credit: String,
    /// Link to the source or its terms.
    pub url: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Provenance {
        Provenance {
            source: SourceId::COOPS,
            station: Some(StationRef {
                source: SourceId::COOPS,
                id: "8518750".into(),
                name: "The Battery".into(),
                kind: StationKind::TideHarmonic,
                latitude: 40.7003,
                longitude: -74.0142,
                bin: None,
                reference_id: None,
            }),
            distance_m: Some(640.0),
            relation: Relation::SameWaterBody,
            observed_at: UnixTime(1_794_000_000),
            kind: ValueKind::Observed,
            quality: Quality::Provisional,
        }
    }

    #[test]
    fn provenance_round_trips() {
        let p = sample();
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(serde_json::from_str::<Provenance>(&json).unwrap(), p);
    }

    #[test]
    fn source_id_is_a_plain_kebab_string() {
        let json = serde_json::to_string(&SourceId::OPEN_METEO_MARINE).unwrap();
        assert_eq!(json, "\"open-meteo-marine\"");
        let back: SourceId = serde_json::from_str("\"ndbc\"").unwrap();
        assert_eq!(back, SourceId::NDBC);
        assert_eq!(SourceId::new("custom").as_str(), "custom");
    }

    #[test]
    fn enums_use_snake_case() {
        assert_eq!(
            serde_json::to_string(&Quality::IceAffected).unwrap(),
            "\"ice_affected\""
        );
        assert_eq!(
            serde_json::to_string(&StationKind::UsgsSite(UsgsSiteType::TidalStream)).unwrap(),
            "{\"usgs_site\":\"tidal_stream\"}"
        );
    }

    #[test]
    fn schema_snapshot() {
        let schemas = serde_json::json!({
            "Provenance": schemars::schema_for!(Provenance),
            "StationRef": schemars::schema_for!(StationRef),
            "Attribution": schemars::schema_for!(Attribution),
        });
        insta::assert_json_snapshot!(schemas);
    }
}
