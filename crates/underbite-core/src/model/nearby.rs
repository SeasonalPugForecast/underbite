//! Map-only records: places to fish from and species sightings. They are fetched for the
//! visible map on request and are not part of the spot snapshot.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::map::GeoPoint;
use crate::provenance::SourceId;
use crate::time::LocalDate;

/// What an access feature is.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum AccessKind {
    Ramp,
    Pier,
    FishingSpot,
    Parking,
    PublicLand,
}

/// Whether the public may enter, as the source records it (never legal permission).
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum AccessLevel {
    Open,
    Restricted,
    Closed,
    Unknown,
}

/// Where a feature is.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AccessGeometry {
    Point(GeoPoint),
    Line(Vec<GeoPoint>),
    /// Rings, the first the outer boundary; each ring is closed implicitly.
    Polygon(Vec<Vec<GeoPoint>>),
}

/// A ramp, pier, fishing spot, parking area or stretch of public land.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AccessFeature {
    pub source: SourceId,
    pub kind: AccessKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub geometry: AccessGeometry,
    pub access: AccessLevel,
}

/// A photo of a record with its own licence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SpeciesPhoto {
    pub url: String,
    /// Licence code or URL as the source gives it; absent means no open licence is stated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub licence: Option<String>,
}

/// One sighting of a species. It names no observer and no rights holder.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SpeciesRecord {
    pub source: SourceId,
    /// The record's id at the source.
    pub id: String,
    /// The taxon as the source names it (scientific name).
    pub taxon: String,
    pub latitude: f64,
    pub longitude: f64,
    /// Radius the position is good to, metres; for an obscured record it is the obscuring cell.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uncertainty_m: Option<f32>,
    /// True when the source deliberately blurred the position.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub obscured: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_on: Option<LocalDate>,
    /// Licence code or URL as the source gives it; absent when none is stated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub licence: Option<String>,
    /// The record's page at the source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub photo: Option<SpeciesPhoto>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn access_feature_round_trips_each_geometry() {
        let p = GeoPoint {
            lat: 40.70,
            lon: -74.01,
        };
        for geometry in [
            AccessGeometry::Point(p),
            AccessGeometry::Line(vec![p, p]),
            AccessGeometry::Polygon(vec![vec![p, p, p]]),
        ] {
            let f = AccessFeature {
                source: SourceId::OVERPASS,
                kind: AccessKind::Ramp,
                name: None,
                geometry,
                access: AccessLevel::Unknown,
            };
            let json = serde_json::to_string(&f).unwrap();
            assert_eq!(serde_json::from_str::<AccessFeature>(&json).unwrap(), f);
        }
    }

    #[test]
    fn species_record_round_trips_and_has_no_person_fields() {
        let r = SpeciesRecord {
            source: SourceId::INATURALIST,
            id: "1".into(),
            taxon: "Morone saxatilis".into(),
            latitude: 40.7,
            longitude: -74.0,
            uncertainty_m: Some(27_000.0),
            obscured: true,
            observed_on: Some(LocalDate {
                year: 2026,
                month: 6,
                day: 1,
            }),
            licence: Some("cc-by-nc".into()),
            url: Some("https://www.inaturalist.org/observations/1".into()),
            photo: Some(SpeciesPhoto {
                url: "https://example.invalid/p.jpg".into(),
                licence: Some("cc-by-nc".into()),
            }),
        };
        let json = serde_json::to_string(&r).unwrap();
        assert_eq!(serde_json::from_str::<SpeciesRecord>(&json).unwrap(), r);
        for word in ["user", "observer", "creator", "rights"] {
            assert!(!json.contains(word), "{word}");
        }
    }
}
