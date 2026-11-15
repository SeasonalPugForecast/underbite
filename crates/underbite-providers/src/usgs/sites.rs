//! USGS monitoring locations near a spot.
//!
//! [`locations_requests`] lists the sites in a box around a rounded point, one request per site
//! type (a comma list matched nothing), and [`parse_locations`] reads them as stations.
//! [`metadata_request`] asks which series the sites in the box serve and [`parse_metadata`]
//! turns them into per-site coverage (`last_value_at` is the series' `end`, which may lag).
//! [`candidates`] joins the two. `drainage_area` and `altitude` have unverified units and are
//! not read.

use std::collections::BTreeMap;

use serde::Deserialize;
use underbite_core::model::{Coverage, Measurement, StationCandidate};
use underbite_core::provenance::{SourceId, StationKind, StationRef, UsgsSiteType};
use underbite_core::time::parse_rfc3339;

use super::ogc::{BASE, with_key};
use crate::request::{Place, QueryBuilder, Request, Resource, Response, Target};
use crate::source::{FetchContext, FragmentValue, Fragments, SourceError, check_response};

/// Site types searched, as USGS writes them.
const SITE_TYPES: &[&str] = &["ST", "ES", "LK", "ST-TS"];

/// Properties asked of `monitoring-locations`.
const LOCATION_PROPERTIES: &str = "id,monitoring_location_name,site_type_code";

/// Properties asked of `time-series-metadata`.
const METADATA_PROPERTIES: &str =
    "monitoring_location_id,parameter_code,statistic_id,primary,begin,end";

/// Most features per page.
const LIMIT: u32 = 1000;

/// Instantaneous values.
const INSTANTANEOUS: &str = "00011";
/// Daily means.
const DAILY_MEAN: &str = "00003";

/// The measurement a parameter code stands for. This repeats the parameters of `ogc`'s table
/// (which is private to that module) and must be kept in step with it.
fn measurement(code: &str) -> Option<Measurement> {
    Some(match code {
        "00060" => Measurement::Discharge,
        "00065" => Measurement::GageHeight,
        "00010" => Measurement::WaterTemperature,
        "63680" => Measurement::Turbidity,
        "00300" => Measurement::DissolvedOxygen,
        "00095" => Measurement::Conductivity,
        "00400" => Measurement::Ph,
        "90860" => Measurement::Salinity,
        "62614" | "62615" | "62616" | "00062" => Measurement::LakeLevel,
        "62619" | "62620" => Measurement::Tide,
        _ => return None,
    })
}

fn site_type(code: &str) -> UsgsSiteType {
    match code {
        "ST" => UsgsSiteType::Stream,
        "ES" => UsgsSiteType::Estuary,
        "LK" => UsgsSiteType::Lake,
        "ST-TS" => UsgsSiteType::TidalStream,
        _ => UsgsSiteType::Other,
    }
}

fn parse_error(what: impl std::fmt::Display) -> SourceError {
    SourceError::Parse(what.to_string())
}

/// A coordinate with at most 4 decimals and no trailing zeros.
fn coordinate(v: f64) -> String {
    let s = format!("{v:.4}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// `west,south,east,north` of the box of `half_width_deg` around the target's rounded point.
fn bbox(target: &Target, half_width_deg: f64) -> Result<String, SourceError> {
    let Place::Point(point) = &target.place else {
        return Err(SourceError::Unsupported(
            "a site search needs a rounded point".to_string(),
        ));
    };
    let c = point.coordinates();
    let (lat, lon) = (c.latitude, c.longitude);
    let h = half_width_deg.abs();
    Ok([lon - h, lat - h, lon + h, (lat + h).min(90.0)]
        .map(coordinate)
        .join(","))
}

/// `monitoring-locations` requests for the box around the target's point, one per site type,
/// with geometry (the coordinates are the point).
///
/// # Errors
/// [`SourceError::Unsupported`] when the target is not a rounded point.
pub fn locations_requests(
    target: &Target,
    half_width_deg: f64,
    ctx: &FetchContext,
) -> Result<Vec<Request>, SourceError> {
    let bbox = bbox(target, half_width_deg)?;
    Ok(SITE_TYPES
        .iter()
        .map(|site_type| {
            let url = QueryBuilder::new(&format!("{BASE}/collections/monitoring-locations/items"))
                .param("f", "json")
                .param("bbox", &bbox)
                .param("site_type_code", *site_type)
                .param("skipGeometry", "false")
                .param("limit", LIMIT.to_string())
                .param("properties", LOCATION_PROPERTIES)
                .finish();
            with_key(
                Request::new(url, Resource::UsgsMonitoringLocations, target.clone())
                    .metered()
                    .max_age_hint(86_400),
                ctx,
            )
        })
        .collect())
}

/// A `time-series-metadata` request for the series of the sites in the box around the
/// target's point. USGS refreshes it at most daily, so it may be cached for a day.
///
/// # Errors
/// [`SourceError::Unsupported`] when the target is not a rounded point.
pub fn metadata_request(
    target: &Target,
    half_width_deg: f64,
    ctx: &FetchContext,
) -> Result<Request, SourceError> {
    let url = QueryBuilder::new(&format!("{BASE}/collections/time-series-metadata/items"))
        .param("f", "json")
        .param("bbox", bbox(target, half_width_deg)?)
        .param("skipGeometry", "true")
        .param("limit", LIMIT.to_string())
        .param("properties", METADATA_PROPERTIES)
        .finish();
    Ok(with_key(
        Request::new(url, Resource::UsgsTimeSeriesMetadata, target.clone())
            .metered()
            .max_age_hint(86_400),
        ctx,
    ))
}

#[derive(Deserialize)]
struct Collection {
    features: Vec<Feature>,
}

#[derive(Deserialize)]
struct Feature {
    properties: Props,
    geometry: Option<Geometry>,
}

#[derive(Deserialize)]
struct Geometry {
    #[serde(default)]
    coordinates: Vec<f64>,
}

#[derive(Deserialize, Default)]
struct Props {
    id: Option<String>,
    monitoring_location_name: Option<String>,
    site_type_code: Option<String>,
    monitoring_location_id: Option<String>,
    parameter_code: Option<String>,
    statistic_id: Option<String>,
    primary: Option<String>,
    end: Option<String>,
}

fn collection(body: &[u8]) -> Result<Collection, SourceError> {
    serde_json::from_slice(body).map_err(parse_error)
}

/// The stations in a `monitoring-locations` body, in its order. Sites without coordinates are
/// left out.
///
/// # Errors
/// [`SourceError::Parse`] when the body is not a feature collection.
pub fn parse_locations(body: &[u8]) -> Result<Vec<StationRef>, SourceError> {
    let mut stations = Vec::new();
    for feature in collection(body)?.features {
        let p = feature.properties;
        let (Some(id), Some(&[lon, lat, ..])) = (
            p.id,
            feature.geometry.as_ref().map(|g| g.coordinates.as_slice()),
        ) else {
            continue;
        };
        stations.push(StationRef {
            source: SourceId::USGS,
            name: p.monitoring_location_name.unwrap_or_else(|| id.clone()),
            id,
            kind: StationKind::UsgsSite(site_type(p.site_type_code.as_deref().unwrap_or(""))),
            latitude: lat,
            longitude: lon,
            bin: None,
            reference_id: None,
        });
    }
    Ok(stations)
}

/// What each site in a `time-series-metadata` body offers, by site id. Per measurement the
/// instantaneous series count when there is one (else the daily mean); `last_value_at` is the
/// newest `end` of them and `primary` is set when one is the site's primary series. Series
/// without an `end` and parameters the app does not read are left out.
///
/// # Errors
/// [`SourceError::Parse`] when the body is not a feature collection or a time is malformed.
pub fn parse_metadata(
    body: &[u8],
) -> Result<BTreeMap<String, BTreeMap<Measurement, Coverage>>, SourceError> {
    // (instantaneous, coverage) per site and measurement; instantaneous series win.
    let mut found: BTreeMap<String, BTreeMap<Measurement, (bool, Coverage)>> = BTreeMap::new();
    for feature in collection(body)?.features {
        let p = feature.properties;
        let (Some(site), Some(m), Some(end)) = (
            p.monitoring_location_id,
            p.parameter_code.as_deref().and_then(measurement),
            p.end,
        ) else {
            continue;
        };
        let instant = match p.statistic_id.as_deref() {
            Some(INSTANTANEOUS) => true,
            Some(DAILY_MEAN) => false,
            _ => continue,
        };
        let end = parse_rfc3339(&end).ok_or_else(|| parse_error(format!("end {end:?}")))?;
        let primary = p.primary.as_deref() == Some("Primary");
        let slot = found.entry(site).or_default().entry(m);
        let new = Coverage {
            last_value_at: Some(end),
            primary,
        };
        slot.and_modify(|(was_instant, c)| {
            if instant && !*was_instant {
                *was_instant = true;
                *c = new;
            } else if instant == *was_instant {
                c.last_value_at = c.last_value_at.max(new.last_value_at);
                c.primary |= primary;
            }
        })
        .or_insert((instant, new));
    }
    Ok(found
        .into_iter()
        .map(|(site, ms)| (site, ms.into_iter().map(|(m, (_, c))| (m, c)).collect()))
        .collect())
}

/// The sites that offer at least one measurement the app reads, with what they offer.
pub fn candidates(
    stations: Vec<StationRef>,
    offers: &BTreeMap<String, BTreeMap<Measurement, Coverage>>,
) -> Vec<StationCandidate> {
    stations
        .into_iter()
        .filter_map(|station| {
            let offers = offers.get(&station.id)?.clone();
            (!offers.is_empty()).then_some(StationCandidate { station, offers })
        })
        .collect()
}

/// Parses a response to a site request. Time-series metadata becomes one `Coverage` fragment per
/// site and measurement; a location list has no fragment shape, so it is
/// [`SourceError::Unsupported`]: read it with [`parse_locations`].
pub(super) fn parse(
    request: &Request,
    response: &Response<'_>,
    _ctx: &FetchContext,
    out: &mut Fragments,
) -> Result<(), SourceError> {
    let body = check_response(request, response)?;
    match request.resource {
        Resource::UsgsTimeSeriesMetadata => {
            for (station_id, offers) in parse_metadata(body)? {
                for (m, coverage) in offers {
                    out.push(
                        m,
                        FragmentValue::Coverage {
                            station_id: station_id.clone(),
                            coverage,
                        },
                    )?;
                }
            }
            Ok(())
        }
        _ => Err(SourceError::Unsupported(
            "site lists are read with sites::parse_locations".to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use underbite_core::time::UnixTime;

    use super::*;
    use crate::request::{RoundedPoint, Window};
    use crate::source::Source;
    use crate::testutil::fixture_bytes;
    use crate::usgs::Usgs;
    use underbite_core::model::Requirements;

    use crate::request::Coordinates;

    const NOW: UnixTime = UnixTime(1_794_148_200);

    fn target() -> Target {
        Target {
            place: Place::Point(RoundedPoint::new(
                Coordinates {
                    latitude: 40.7033,
                    longitude: -74.017,
                },
                2,
            )),
            measurements: BTreeSet::from([Measurement::Discharge]),
            window: Window { from: NOW, to: NOW },
            datum: None,
        }
    }

    fn ctx() -> FetchContext {
        FetchContext::new(NOW, "underbite/test")
    }

    fn locations(name: &str) -> Vec<StationRef> {
        parse_locations(&fixture_bytes("usgs", name)).unwrap()
    }

    fn metadata(name: &str) -> BTreeMap<String, BTreeMap<Measurement, Coverage>> {
        parse_metadata(&fixture_bytes("usgs", name)).unwrap()
    }

    #[test]
    fn requests_use_one_site_type_each_and_a_west_first_box() {
        let requests = locations_requests(&target(), 0.05, &ctx()).unwrap();
        assert_eq!(requests.len(), 4);
        for (r, t) in requests.iter().zip(["ST", "ES", "LK", "ST-TS"]) {
            assert!(
                r.url.contains("bbox=-74.07,40.65,-73.97,40.75"),
                "{}",
                r.url
            );
            assert!(r.url.contains(&format!("site_type_code={t}&")), "{}", r.url);
            assert!(r.url.contains("skipGeometry=false"), "{}", r.url);
            assert_eq!(r.resource, Resource::UsgsMonitoringLocations);
        }
        let m = metadata_request(&target(), 0.05, &ctx()).unwrap();
        assert!(m.url.contains("/time-series-metadata/items"));
        assert!(m.url.contains("skipGeometry=true"));
        let station = Target {
            place: station_place(),
            ..target()
        };
        assert!(locations_requests(&station, 0.05, &ctx()).is_err());
    }

    pub(super) fn station_place() -> Place {
        Place::Zone(crate::request::ZoneTarget {
            id: "ANZ338".into(),
            name: "x".into(),
            kind: underbite_core::model::ZoneKind::Coastal,
            office: None,
        })
    }

    #[test]
    fn manhattan_stream_sites() {
        let sites = locations("usgs-monitoring-locations-bbox-manhattan-st.json");
        let ids: Vec<_> = sites.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["USGS-01376515", "USGS-01376520"]);
        assert_eq!(sites[1].kind, StationKind::UsgsSite(UsgsSiteType::Stream));
        assert!((sites[1].latitude - 40.721_528).abs() < 1e-6);
        assert!((sites[1].longitude + 74.015_611).abs() < 1e-6);
        assert_eq!(sites[1].name, "HUDSON RIVER AT PIER 25 AT NEW YORK NY");
    }

    #[test]
    fn estuary_and_lake_lists() {
        let es = locations("usgs-monitoring-locations-bbox-hudson-poughkeepsie-es.json");
        assert_eq!(es.len(), 42);
        assert!(
            es.iter()
                .all(|s| s.kind == StationKind::UsgsSite(UsgsSiteType::Estuary))
        );
        let lk = locations("usgs-monitoring-locations-bbox-great-sacandaga-lk.json");
        assert_eq!(lk.len(), 6);
        assert!(
            lk.iter()
                .all(|s| s.kind == StationKind::UsgsSite(UsgsSiteType::Lake))
        );
    }

    #[test]
    fn croton_discharge_and_gage_height_are_live() {
        let offers = metadata("usgs-time-series-metadata-croton-01375000.json");
        let croton = &offers["USGS-01375000"];
        assert_eq!(croton.len(), 2);
        let q = croton[&Measurement::Discharge];
        // The instantaneous series ends 2026-11-08T05:45Z; the daily mean ends earlier.
        assert_eq!(q.last_value_at, parse_rfc3339("2026-11-08T05:45:00+00:00"));
        assert!(q.primary);
        assert!(croton.contains_key(&Measurement::GageHeight));
    }

    #[test]
    fn pier_25_series_all_ended_in_april() {
        let offers = metadata("usgs-time-series-metadata-hudson-pier-25-01376520.json");
        let site = &offers["USGS-01376520"];
        assert!(site.contains_key(&Measurement::Turbidity));
        let april = parse_rfc3339("2026-04-01T00:00:00+00:00").unwrap();
        let march = parse_rfc3339("2026-03-31T00:00:00+00:00").unwrap();
        for (m, c) in site {
            let t = c.last_value_at.unwrap();
            assert!(t >= march && t <= UnixTime(april.0 + 86_400), "{m:?} {t:?}");
            assert!(t < NOW);
        }
    }

    #[test]
    fn lake_and_dead_series_metadata() {
        let lake = metadata("usgs-time-series-metadata-great-sacandaga-01323500.json");
        let level = lake["USGS-01323500"][&Measurement::LakeLevel];
        assert_eq!(
            level.last_value_at,
            parse_rfc3339("2026-11-08T08:30:00+00:00")
        );
        // A series with no begin or end is not an offer.
        let pough = metadata("usgs-time-series-metadata-hudson-poughkeepsie-01372058.json");
        assert!(!pough["USGS-01372058"].contains_key(&Measurement::Discharge));
        assert!(pough["USGS-01372058"].contains_key(&Measurement::Tide));
    }

    #[test]
    fn candidates_join_sites_with_their_offers() {
        let mut offers = metadata("usgs-time-series-metadata-hudson-pier-25-01376520.json");
        offers.extend(metadata("usgs-time-series-metadata-croton-01375000.json"));
        let found = candidates(
            locations("usgs-monitoring-locations-bbox-manhattan-st.json"),
            &offers,
        );
        let [one] = found.as_slice() else {
            panic!("{found:?}");
        };
        assert_eq!(one.station.id, "USGS-01376520");
        assert!(one.offers.contains_key(&Measurement::WaterTemperature));
    }

    #[test]
    fn metadata_parse_gives_coverage_fragments_and_locations_are_unsupported() {
        let usgs = Usgs::new();
        let body = fixture_bytes("usgs", "usgs-time-series-metadata-croton-01375000.json");
        let request = metadata_request(&target(), 0.05, &ctx()).unwrap();
        let mut out = Fragments::new();
        usgs.parse(&request, &Response::new(200, &body), &ctx(), &mut out)
            .unwrap();
        assert_eq!(out.fragments().len(), 2);
        assert!(out.fragments().iter().all(|f| matches!(
            &f.value,
            FragmentValue::Coverage { station_id, .. } if station_id == "USGS-01375000"
        )));
        let list = locations_requests(&target(), 0.05, &ctx())
            .unwrap()
            .remove(0);
        let body = fixture_bytes("usgs", "usgs-monitoring-locations-bbox-manhattan-st.json");
        let err = usgs
            .parse(
                &list,
                &Response::new(200, &body),
                &ctx(),
                &mut Fragments::new(),
            )
            .unwrap_err();
        assert!(matches!(err, SourceError::Unsupported(_)), "{err:?}");
        // Planning for a point does not search sites.
        let plan = usgs.plan(&target(), &Requirements::default(), &ctx());
        assert!(plan.is_err());
    }
}
