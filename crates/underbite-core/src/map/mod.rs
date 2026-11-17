//! Map layers: tile addressing, the layer catalogue, gridded data and on-device raster rendering.
//!
//! Engine-neutral: nothing here knows a map engine; the displays draw what this module describes.
//! Coordinates are WGS 84 degrees, speeds are metres per second and bearings are degrees clockwise
//! from true north.

use std::borrow::Cow;
use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub mod breaks;
pub mod catalogue;
pub mod contours;
pub mod dem;
pub mod enc;
pub mod grid;
pub mod render;
pub mod tiles;

/// The stable id of a map layer, such as `sst` or `radar`.
///
/// A string newtype rather than an enum, like [`SourceId`](crate::provenance::SourceId): a layer
/// added later needs no contract change, and an id this version does not know survives decoding so
/// that migration can drop it with a note.
#[derive(
    Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct LayerId(Cow<'static, str>);

impl LayerId {
    /// An id known at compile time.
    pub const fn from_static(id: &'static str) -> Self {
        Self(Cow::Borrowed(id))
    }

    /// An id read at run time.
    pub fn new(id: impl Into<String>) -> Self {
        Self(Cow::Owned(id.into()))
    }

    /// The id as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for LayerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A point on the earth, in degrees.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct GeoPoint {
    /// Degrees north of the equator, -90 to 90.
    pub lat: f64,
    /// Degrees east of Greenwich, -180 to 180.
    pub lon: f64,
}

/// A box in degrees. `west` is at most `east`: a box across the antimeridian is two boxes.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Bbox {
    pub west: f64,
    pub south: f64,
    pub east: f64,
    pub north: f64,
}

/// A Web Mercator tile address (the XYZ scheme: `y` counts down from the north).
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
pub struct TileKey {
    pub z: u8,
    pub x: u32,
    pub y: u32,
}

/// A line of equal value, such as a depth contour or a temperature break.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Polyline {
    pub points: Vec<GeoPoint>,
    /// What the line stands for, in canonical units (metres for depth, °C for temperature).
    pub value: f64,
}

/// A current or wind arrow at a point.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Arrow {
    pub at: GeoPoint,
    /// Speed in metres per second.
    pub speed_ms: f32,
    /// The direction the flow goes toward, degrees clockwise from true north.
    pub toward_deg: f32,
}

/// The server a gridded layer's data comes from: three ERDDAP servers and the NCEI elevation
/// model.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum GridHost {
    /// The NOAA West Coast ERDDAP node (`coastwatch.pfeg.noaa.gov`): MUR SST and HF radar.
    Pfeg,
    /// The NOAA national ERDDAP node (`coastwatch.noaa.gov`): VIIRS ocean colour.
    Coastwatch,
    /// NOAA GLERL (`apps.glerl.noaa.gov`): Great Lakes SST.
    Glerl,
    /// The NCEI DEM global mosaic, an `ArcGIS` image service rather than an ERDDAP server.
    NceiDem,
}

/// The former name of [`GridHost`], from when every grid came from an ERDDAP server.
#[deprecated(note = "use GridHost")]
pub type ErddapNode = GridHost;

impl GridHost {
    /// The ERDDAP server's base URL, without a trailing slash (`/griddap/{dataset}.json`
    /// follows); `None` for a host that is not an ERDDAP server.
    pub const fn erddap_base_url(self) -> Option<&'static str> {
        match self {
            Self::Pfeg => Some("https://coastwatch.pfeg.noaa.gov/erddap"),
            Self::Coastwatch => Some("https://coastwatch.noaa.gov/erddap"),
            Self::Glerl => Some("https://apps.glerl.noaa.gov/erddap"),
            Self::NceiDem => None,
        }
    }
}

/// Where a gridded layer's data comes from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct GridSource {
    pub node: GridHost,
    /// The ERDDAP dataset id, such as `jplMURSST41`.
    pub dataset: String,
    /// The variables to request, such as `analysed_sst`, or `water_u` and `water_v` for currents.
    pub variables: Vec<String>,
    /// Whether the dataset has an altitude axis between time and latitude (VIIRS ocean colour
    /// does, at 0.0; MUR SST and HF radar do not).
    pub altitude_axis: bool,
    /// The dataset's grid spacing in degrees, used to choose a stride for a visible area.
    pub native_step_deg: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layer_ids_serialise_as_plain_strings() {
        let id = LayerId::new("sst");
        assert_eq!(serde_json::to_string(&id).unwrap(), "\"sst\"");
        assert_eq!(
            serde_json::from_str::<LayerId>("\"sst\"").unwrap(),
            LayerId::from_static("sst")
        );
        assert_eq!(id.to_string(), "sst");
    }

    #[test]
    fn node_urls_match_the_documented_servers() {
        let urls = [
            (GridHost::Pfeg, "https://coastwatch.pfeg.noaa.gov/erddap"),
            (GridHost::Coastwatch, "https://coastwatch.noaa.gov/erddap"),
            (GridHost::Glerl, "https://apps.glerl.noaa.gov/erddap"),
        ];
        for (host, url) in urls {
            assert_eq!(host.erddap_base_url(), Some(url));
        }
        assert_eq!(GridHost::NceiDem.erddap_base_url(), None);
    }

    #[test]
    fn host_names_are_stable() {
        let names = [
            (GridHost::Pfeg, "\"pfeg\""),
            (GridHost::Coastwatch, "\"coastwatch\""),
            (GridHost::Glerl, "\"glerl\""),
            (GridHost::NceiDem, "\"ncei_dem\""),
        ];
        for (host, name) in names {
            assert_eq!(serde_json::to_string(&host).unwrap(), name);
        }
    }
}
