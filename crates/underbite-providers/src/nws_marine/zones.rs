//! NWS marine zones for a point and coastal zone forecasts.
//!
//! A point resolves to one marine zone (`/points`, then `/zones?point=`). Only `coastal` zones
//! outside the Great Lakes have a forecast here; the forecast is text, never read for numbers.

use serde::Deserialize;
use underbite_core::model::{ForecastPeriod, ZoneForecast, ZoneKind, ZoneProduct};
use underbite_core::time::parse_rfc3339;

use crate::request::{RoundedPoint, ZoneTarget};
use crate::source::SourceError;

/// The API's base URL.
pub const BASE: &str = "https://api.weather.gov";

/// `/points/{lat},{lon}` for a rounded point.
pub fn point_url(point: RoundedPoint) -> String {
    let (lat, lon) = point.fmt_param();
    format!("{BASE}/points/{lat},{lon}")
}

/// `/zones?point=` for a rounded point, optionally only `coastal` zones.
pub fn zones_url(point: RoundedPoint, coastal_only: bool) -> String {
    let (lat, lon) = point.fmt_param();
    let kind = if coastal_only { "type=coastal&" } else { "" };
    format!("{BASE}/zones?{kind}point={lat},{lon}")
}

/// `/zones/coastal/{id}/forecast`.
pub fn forecast_url(zone_id: &str) -> String {
    format!("{BASE}/zones/coastal/{zone_id}/forecast")
}

#[derive(Deserialize)]
struct PointBody {
    properties: PointProperties,
}

#[derive(Deserialize)]
struct PointProperties {
    #[serde(rename = "type")]
    kind: Option<String>,
}

/// Whether `/points` says the point is over water.
pub fn point_is_marine(body: &[u8]) -> Result<bool, SourceError> {
    let point: PointBody = serde_json::from_slice(body)
        .map_err(|e| SourceError::Parse(format!("point lookup: {e}")))?;
    Ok(point.properties.kind.as_deref() == Some("marine"))
}

#[derive(Deserialize)]
struct ZonesBody {
    #[serde(default)]
    features: Vec<ZoneFeature>,
}

#[derive(Deserialize)]
struct ZoneFeature {
    properties: ZoneProperties,
}

#[derive(Deserialize)]
struct ZoneProperties {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    name: String,
    #[serde(rename = "gridIdentifier")]
    office: Option<String>,
}

/// Great Lakes marine zone prefixes: Erie, Ontario, Huron, Michigan, Superior, St. Clair, the
/// St. Lawrence and Lake Champlain. Only Erie and Ontario are verified against the API.
const LAKE_PREFIXES: [&str; 7] = ["LEZ", "LOZ", "LHZ", "LMZ", "LSZ", "LCZ", "SLZ"];

/// The zone's kind from the API's `type`, its id and its name.
///
/// Open-lake zones say "beyond 5 NM" (or "from 5NM off shoreline") in their name, and for Lake
/// Erie sit at 160 to 169.
pub fn classify(id: &str, api_type: &str, name: &str) -> ZoneKind {
    if api_type == "offshore" {
        return ZoneKind::Offshore;
    }
    if !LAKE_PREFIXES.iter().any(|p| id.starts_with(p)) {
        return ZoneKind::Coastal;
    }
    let squeezed: String = name
        .to_ascii_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    let number: Option<u32> = id.get(3..).and_then(|n| n.parse().ok());
    if squeezed.contains("beyond5nm")
        || squeezed.contains("from5nmoff")
        || number.is_some_and(|n| (160..=169).contains(&n))
    {
        ZoneKind::GreatLakesOpen
    } else {
        ZoneKind::GreatLakesNearshore
    }
}

/// The first marine (`coastal` or `offshore`) zone of a zone list, if any.
pub fn marine_zone(body: &[u8]) -> Result<Option<ZoneTarget>, SourceError> {
    let zones: ZonesBody = serde_json::from_slice(body)
        .map_err(|e| SourceError::Parse(format!("zone lookup: {e}")))?;
    Ok(zones
        .features
        .into_iter()
        .map(|f| f.properties)
        .find(|p| p.kind == "coastal" || p.kind == "offshore")
        .map(|p| ZoneTarget {
            kind: classify(&p.id, &p.kind, &p.name),
            office: p.office,
            id: p.id,
            name: p.name,
        }))
}

#[derive(Deserialize)]
struct ForecastBody {
    updated: String,
    #[serde(default)]
    periods: Vec<PeriodBody>,
}

#[derive(Deserialize)]
struct PeriodBody {
    name: String,
    #[serde(rename = "detailedForecast")]
    text: String,
}

/// The coastal zone forecast of `zone`, periods verbatim.
pub fn parse_forecast(body: &[u8], zone: &ZoneTarget) -> Result<ZoneForecast, SourceError> {
    let forecast: ForecastBody = serde_json::from_slice(body)
        .map_err(|e| SourceError::Parse(format!("zone forecast: {e}")))?;
    let issued = parse_rfc3339(&forecast.updated)
        .ok_or_else(|| SourceError::Parse(format!("zone forecast time {:?}", forecast.updated)))?;
    if forecast.periods.is_empty() {
        return Err(SourceError::NoData(Some("forecast has no periods".into())));
    }
    Ok(ZoneForecast {
        id: zone.id.clone(),
        name: zone.name.clone(),
        kind: zone.kind,
        issued,
        product: ZoneProduct::ZoneApi,
        periods: forecast
            .periods
            .into_iter()
            .map(|p| ForecastPeriod {
                name: p.name,
                text: p.text,
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::Coordinates;
    use crate::source::FetchContext;
    use crate::testutil::fixture_bytes;
    use underbite_core::time::UnixTime;

    fn zone(file: &str) -> Option<ZoneTarget> {
        marine_zone(&fixture_bytes("nws-marine", file)).unwrap()
    }

    #[test]
    fn nws_zones_urls_use_two_decimals() {
        let ctx = FetchContext::new(UnixTime(0), "underbite-test");
        let p = ctx.round(Coordinates {
            latitude: 40.7033,
            longitude: -74.0170,
        });
        assert_eq!(point_url(p), "https://api.weather.gov/points/40.7,-74.02");
        assert_eq!(
            zones_url(p, true),
            "https://api.weather.gov/zones?type=coastal&point=40.7,-74.02"
        );
        assert_eq!(
            zones_url(p, false),
            "https://api.weather.gov/zones?point=40.7,-74.02"
        );
    }

    #[test]
    fn nws_zones_find_the_recorded_marine_zones() {
        let z = zone("zones-coastal-point-40.7033_-74.0170.geo.json").unwrap();
        assert_eq!((z.id.as_str(), z.kind), ("ANZ338", ZoneKind::Coastal));
        assert_eq!(z.office.as_deref(), Some("OKX"));
        assert_eq!(z.name, "New York Harbor");
        let z = zone("zones-point-41.55_-81.72.geo.json").unwrap();
        assert_eq!(
            (z.id.as_str(), z.kind),
            ("LEZ146", ZoneKind::GreatLakesNearshore)
        );
        let z = zone("zones-point-41.9_-81.6-beyond-5nm.geo.json").unwrap();
        assert_eq!(
            (z.id.as_str(), z.kind),
            ("LEZ166", ZoneKind::GreatLakesOpen)
        );
        let z = zone("zones-point-39.5_-72.0-offshore.geo.json").unwrap();
        assert_eq!((z.id.as_str(), z.kind), ("ANZ815", ZoneKind::Offshore));
        assert_eq!(
            zone("zones-offshore-point-40.25_-73.16-empty.geo.json"),
            None
        );
    }

    #[test]
    fn nws_zones_classify_every_lake_erie_zone() {
        let body = fixture_bytes("nws-marine", "zones-coastal-area-LE.geo.json");
        let list: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let mut open = Vec::new();
        let mut near = Vec::new();
        for f in list["features"].as_array().unwrap() {
            let p = &f["properties"];
            let id = p["id"].as_str().unwrap();
            match classify(id, p["type"].as_str().unwrap(), p["name"].as_str().unwrap()) {
                ZoneKind::GreatLakesOpen => open.push(id.to_string()),
                ZoneKind::GreatLakesNearshore => near.push(id.to_string()),
                other => panic!("{id}: {other:?}"),
            }
        }
        assert_eq!(open.len(), 9);
        assert!(open.contains(&"LEZ061".to_string()) && open.contains(&"LEZ169".to_string()));
        assert_eq!(near.len(), 12);
        assert!(near.contains(&"LEZ444".to_string()));
        assert_eq!(classify("ANZ820", "offshore", "x"), ZoneKind::Offshore);
        assert_eq!(classify("PZZ535", "coastal", "x"), ZoneKind::Coastal);
    }

    #[test]
    fn nws_zones_point_lookup_knows_marine_from_land() {
        for file in [
            "points-40.7033_-74.0170.geo.json",
            "points-39.5_-72.0-offshore.geo.json",
            "points-41.55_-81.72.geo.json",
        ] {
            assert!(point_is_marine(&fixture_bytes("nws-marine", file)).unwrap());
        }
        assert!(!point_is_marine(br#"{"properties":{"type":"land"}}"#).unwrap());
    }

    #[test]
    fn nws_zones_forecasts_keep_ten_periods_verbatim() {
        let harbor = zone("zones-coastal-point-40.7033_-74.0170.geo.json").unwrap();
        let body = fixture_bytes("nws-marine", "zone-forecast-ANZ338.ld.json");
        let f = parse_forecast(&body, &harbor).unwrap();
        assert_eq!(f.periods.len(), 10);
        assert_eq!(f.periods[0].name, "Today");
        assert!(f.periods[0].text.starts_with("W winds 15 to 20 kt"));
        assert_eq!(f.product, ZoneProduct::ZoneApi);
        assert_eq!(
            f.issued,
            parse_rfc3339("2026-11-08T01:57:00-05:00").unwrap()
        );
        insta::assert_json_snapshot!("zone_forecast_anz338", f);
        let far = ZoneTarget {
            id: "ANZ383".into(),
            name: "Waters from Fire Island Inlet NY to Moriches Inlet NY from 20 to 60 NM".into(),
            kind: ZoneKind::Coastal,
            office: Some("OKX".into()),
        };
        let body = fixture_bytes("nws-marine", "zone-forecast-ANZ383.ld.json");
        let f = parse_forecast(&body, &far).unwrap();
        assert_eq!(f.periods.len(), 10);
        assert_eq!(f.id, "ANZ383");
    }
}
