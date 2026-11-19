//! NWS marine zones, forecasts and alerts.
//!
//! [`NwsMarine`] looks up the marine zone of a point (`/points`, then `/zones?point=`), then
//! reads the zone's coastal forecast and its active alerts. The zone found goes into the
//! follow-up requests as [`Place::Zone`]: the forecast response does not name its zone. Great
//! Lakes and offshore zones have no forecast endpoint; their text products are another unit's
//! follow-ups ([`product_requests`]).

pub mod alerts;
pub mod products;
pub mod zones;

use underbite_core::model::{Measurement, Requirements, ZoneKind};
use underbite_core::provenance::{Attribution, SourceId};

use crate::request::{Place, Request, Resource, Response, Target, ZoneTarget};
use crate::source::{FetchContext, FragmentValue, Fragments, Source, SourceError, check_response};

/// An answer of no data for the lookup of a point outside the API's coverage (404
/// "InvalidPoint"): nothing there, not a lost endpoint.
fn outside_coverage(request: &Request, response: &Response<'_>) -> Result<(), SourceError> {
    if request.resource == Resource::NwsPoint
        && response.status == 404
        && is_invalid_point(response.body)
    {
        return Err(SourceError::NoData(Some(
            "the point is outside NWS coverage".into(),
        )));
    }
    Ok(())
}

/// Whether `body` is the API's `InvalidPoint` problem document.
fn is_invalid_point(body: &[u8]) -> bool {
    serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("type")?.as_str().map(str::to_owned))
        .is_some_and(|t| t.ends_with("/problems/InvalidPoint"))
}

/// Seconds a point or zone lookup may be reused (the API sets one day for `/points`).
const LOOKUP_AGE_S: u32 = 86_400;
/// Seconds a zone forecast may be reused.
const FORECAST_AGE_S: u32 = 3_600;
/// Seconds alerts may be reused.
const ALERTS_AGE_S: u32 = 5;

const MEASUREMENTS: [Measurement; 2] = [Measurement::MarineForecast, Measurement::MarineAlerts];

/// NWS marine zone forecasts and alerts as a source. It holds no settings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NwsMarine;

/// A request with the headers the API wants: the caller's `User-Agent` and an `Accept`.
fn get(
    url: String,
    resource: Resource,
    target: Target,
    accept: &'static str,
    ctx: &FetchContext,
    age_s: u32,
) -> Request {
    Request::new(url, resource, target)
        .header("User-Agent", ctx.user_agent.clone())
        .header("Accept", accept)
        .max_age_hint(age_s)
}

const GEO_JSON: &str = "application/geo+json";
const LD_JSON: &str = "application/ld+json";

/// The follow-ups that read the text products of a zone with no forecast endpoint: Great Lakes
/// and offshore zones: the text product that holds the zone's forecast, or none when no product
/// is known for the zone.
pub fn product_requests(zone: &ZoneTarget, target: &Target, ctx: &FetchContext) -> Vec<Request> {
    products::requests(zone, target, ctx)
}

impl Source for NwsMarine {
    fn id(&self) -> SourceId {
        SourceId::NWS
    }

    fn attribution(&self) -> Attribution {
        Attribution {
            source: SourceId::NWS,
            credit: "NOAA National Weather Service".into(),
            url: Some("https://www.weather.gov/".into()),
        }
    }

    fn measurements(&self) -> &[Measurement] {
        &MEASUREMENTS
    }

    fn plan(
        &self,
        target: &Target,
        requirements: &Requirements,
        ctx: &FetchContext,
    ) -> Result<Vec<Request>, SourceError> {
        let Place::Point(point) = &target.place else {
            return Err(SourceError::Unsupported(
                "NWS marine zones need a point".into(),
            ));
        };
        let measurements = target
            .measurements
            .iter()
            .copied()
            .filter(|m| {
                MEASUREMENTS.contains(m)
                    && (requirements.measurements.is_empty()
                        || requirements.measurements.contains(m))
            })
            .collect();
        let target = Target {
            measurements,
            ..target.clone()
        };
        if target.measurements.is_empty() {
            return Err(SourceError::Unsupported(
                "no marine forecast or alerts wanted".into(),
            ));
        }
        Ok(vec![get(
            zones::point_url(*point),
            Resource::NwsPoint,
            target,
            GEO_JSON,
            ctx,
            LOOKUP_AGE_S,
        )])
    }

    fn follow_up(
        &self,
        request: &Request,
        response: &Response<'_>,
        ctx: &FetchContext,
    ) -> Result<Vec<Request>, SourceError> {
        outside_coverage(request, response)?;
        let body = check_response(request, response)?;
        let target = &request.target;
        match request.resource {
            Resource::NwsPoint => {
                let Place::Point(point) = &target.place else {
                    return Err(SourceError::Unsupported(
                        "point lookup without a point".into(),
                    ));
                };
                // A point the API calls land can still lie inside a coastal zone's edge.
                let marine = zones::point_is_marine(body)?;
                let mut next = get(
                    zones::zones_url(*point, !marine),
                    Resource::NwsZones,
                    target.clone(),
                    GEO_JSON,
                    ctx,
                    LOOKUP_AGE_S,
                );
                next.required = request.required;
                Ok(vec![next])
            }
            Resource::NwsZones => {
                let zone = zones::marine_zone(body)?.ok_or_else(|| {
                    SourceError::NoData(Some("no marine zone at this point".into()))
                })?;
                let mut requests = Vec::new();
                let zone_target = |measurement: Measurement| Target {
                    place: Place::Zone(zone.clone()),
                    measurements: [measurement].into(),
                    ..target.clone()
                };
                if target.measurements.contains(&Measurement::MarineForecast) {
                    if zone.kind == ZoneKind::Coastal {
                        requests.push(
                            get(
                                zones::forecast_url(&zone.id),
                                Resource::NwsZoneForecast,
                                zone_target(Measurement::MarineForecast),
                                LD_JSON,
                                ctx,
                                FORECAST_AGE_S,
                            )
                            .optional(),
                        );
                    } else {
                        requests.extend(product_requests(&zone, target, ctx));
                    }
                }
                if target.measurements.contains(&Measurement::MarineAlerts) {
                    requests.push(
                        get(
                            alerts::url(&zone.id),
                            Resource::NwsAlerts,
                            zone_target(Measurement::MarineAlerts),
                            GEO_JSON,
                            ctx,
                            ALERTS_AGE_S,
                        )
                        .optional(),
                    );
                }
                Ok(requests)
            }
            _ => Ok(Vec::new()),
        }
    }

    fn parse(
        &self,
        request: &Request,
        response: &Response<'_>,
        ctx: &FetchContext,
        out: &mut Fragments,
    ) -> Result<(), SourceError> {
        outside_coverage(request, response)?;
        // A zone without a forecast endpoint answers 404 "Marine Forecast Not Supported" (or a
        // plain 404): unsupported here, so the session turns to the text products.
        if request.resource == Resource::NwsZoneForecast && response.status == 404 {
            return Err(SourceError::Unsupported(
                "no zone forecast for this zone".into(),
            ));
        }
        let body = check_response(request, response)?;
        match request.resource {
            Resource::NwsPoint | Resource::NwsZones => Ok(()),
            Resource::NwsZoneForecast => {
                let Place::Zone(zone) = &request.target.place else {
                    return Err(SourceError::Unsupported("forecast without a zone".into()));
                };
                let forecast = zones::parse_forecast(body, zone)?;
                out.push(
                    Measurement::MarineForecast,
                    FragmentValue::ZoneForecast(forecast),
                )
            }
            Resource::NwsProduct => {
                let Place::Zone(zone) = &request.target.place else {
                    return Err(SourceError::Unsupported("product without a zone".into()));
                };
                let forecast = products::parse_forecast(body, zone, ctx.now)?;
                out.push(
                    Measurement::MarineForecast,
                    FragmentValue::ZoneForecast(forecast),
                )
            }
            Resource::NwsAlerts => out.push(
                Measurement::MarineAlerts,
                FragmentValue::Alerts(alerts::parse(body)?),
            ),
            other => Err(SourceError::Unsupported(format!(
                "NWS marine does not serve {other:?}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::{Coordinates, Window};
    use crate::testutil::fixture_bytes;
    use std::collections::BTreeSet;
    use underbite_core::model::ZoneProduct;
    use underbite_core::time::UnixTime;

    const NOW: UnixTime = UnixTime(1_794_000_000);

    fn ctx() -> FetchContext {
        FetchContext::new(NOW, "underbite-test/0")
    }

    fn target() -> Target {
        Target {
            place: Place::Point(ctx().round(Coordinates {
                latitude: 40.7033,
                longitude: -74.0170,
            })),
            measurements: BTreeSet::from(MEASUREMENTS),
            window: Window { from: NOW, to: NOW },
            datum: None,
        }
    }

    fn fixture(name: &str) -> Vec<u8> {
        fixture_bytes("nws-marine", name)
    }

    fn lookup(point_file: &str, zones_file: &str) -> Vec<Request> {
        let src = NwsMarine;
        let plan = src
            .plan(&target(), &Requirements::default(), &ctx())
            .unwrap();
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].url, "https://api.weather.gov/points/40.7,-74.02");
        let body = fixture(point_file);
        let next = src
            .follow_up(&plan[0], &Response::new(200, &body), &ctx())
            .unwrap();
        assert_eq!(next.len(), 1);
        let body = fixture(zones_file);
        src.follow_up(&next[0], &Response::new(200, &body), &ctx())
            .unwrap()
    }

    #[test]
    fn nws_marine_harbor_chain_reaches_forecast_and_alerts() {
        let next = lookup(
            "points-40.7033_-74.0170.geo.json",
            "zones-coastal-point-40.7033_-74.0170.geo.json",
        );
        let urls: Vec<_> = next.iter().map(|r| r.url.as_str()).collect();
        assert_eq!(
            urls,
            [
                "https://api.weather.gov/zones/coastal/ANZ338/forecast",
                "https://api.weather.gov/alerts/active?zone=ANZ338"
            ]
        );
        let Place::Zone(zone) = &next[0].target.place else {
            panic!("the forecast request names its zone");
        };
        assert_eq!(
            (zone.id.as_str(), zone.office.as_deref()),
            ("ANZ338", Some("OKX"))
        );
        assert!(next.iter().all(|r| !r.required));
        assert!(
            next[0]
                .headers
                .contains(&("User-Agent", "underbite-test/0".into()))
        );
        assert!(
            next[0]
                .headers
                .contains(&("Accept", "application/ld+json".into()))
        );
        assert_eq!(next[0].cache.max_age, Some(3_600));
        assert_eq!(next[1].cache.max_age, Some(5));

        let mut out = Fragments::new();
        let body = fixture("zone-forecast-ANZ338.ld.json");
        NwsMarine
            .parse(&next[0], &Response::new(200, &body), &ctx(), &mut out)
            .unwrap();
        let body = fixture("alerts-active-zone-ANZ338.geo.json");
        NwsMarine
            .parse(&next[1], &Response::new(200, &body), &ctx(), &mut out)
            .unwrap();
        assert_eq!(out.fragments().len(), 2);
        match &out.fragments()[0].value {
            FragmentValue::ZoneForecast(f) => {
                assert_eq!((f.id.as_str(), f.product), ("ANZ338", ZoneProduct::ZoneApi));
            }
            other => panic!("{other:?}"),
        }
        match &out.fragments()[1].value {
            FragmentValue::Alerts(a) => assert_eq!(a.len(), 1),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn nws_marine_lake_and_offshore_zones_ask_for_their_text_product_and_alerts() {
        for (point, zones, id, product) in [
            (
                "points-41.55_-81.72.geo.json",
                "zones-point-41.55_-81.72.geo.json",
                "LEZ146",
                "https://api.weather.gov/products/types/NSH/locations/CLE/latest",
            ),
            (
                "points-39.5_-72.0-offshore.geo.json",
                "zones-point-39.5_-72.0-offshore.geo.json",
                "ANZ815",
                "https://api.weather.gov/products/types/OFF/locations/NT1/latest",
            ),
        ] {
            let next = lookup(point, zones);
            assert_eq!(next.len(), 2, "{id}");
            assert_eq!(next[0].resource, Resource::NwsProduct);
            assert_eq!(next[0].url, product);
            assert_eq!(next[1].resource, Resource::NwsAlerts);
            assert!(next[1].url.ends_with(id));
        }
    }

    #[test]
    fn nws_marine_zones_without_a_forecast_endpoint_are_unsupported() {
        let request = lookup(
            "points-40.7033_-74.0170.geo.json",
            "zones-coastal-point-40.7033_-74.0170.geo.json",
        )
        .remove(0);
        for file in [
            "zone-forecast-LEZ146-not-found.problem.json",
            "zone-forecast-ANZ820-offshore-unsupported.problem.json",
        ] {
            let body = fixture(file);
            let mut out = Fragments::new();
            let err = NwsMarine
                .parse(&request, &Response::new(404, &body), &ctx(), &mut out)
                .unwrap_err();
            assert!(
                matches!(err, SourceError::Unsupported(_)),
                "{file}: {err:?}"
            );
            assert!(out.is_empty());
        }
    }

    #[test]
    fn nws_marine_an_empty_alert_list_parses_to_no_alerts() {
        let request = lookup(
            "points-40.7033_-74.0170.geo.json",
            "zones-coastal-point-40.7033_-74.0170.geo.json",
        )
        .remove(1);
        let body = fixture("alerts-active-zone-ANZ338-event-gale-warning-empty.geo.json");
        let mut out = Fragments::new();
        NwsMarine
            .parse(&request, &Response::new(200, &body), &ctx(), &mut out)
            .unwrap();
        assert!(matches!(&out.fragments()[0].value, FragmentValue::Alerts(a) if a.is_empty()));
    }

    #[test]
    fn nws_marine_a_point_outside_coverage_has_no_data() {
        let plan = NwsMarine
            .plan(&target(), &Requirements::default(), &ctx())
            .unwrap();
        let body = fixture("points-45.0_-30.0-invalid-point.problem.json");
        let err = NwsMarine
            .follow_up(&plan[0], &Response::new(404, &body), &ctx())
            .unwrap_err();
        assert!(matches!(err, SourceError::NoData(_)), "{err:?}");

        // Any other 404 on the lookup is still a missing endpoint.
        let other = br#"{"type":"https://api.weather.gov/problems/NotFound","title":"Not Found"}"#;
        for body in [&other[..], b"not json"] {
            let err = NwsMarine
                .follow_up(&plan[0], &Response::new(404, body), &ctx())
                .unwrap_err();
            assert!(matches!(err, SourceError::NotFound(_)), "{err:?}");
        }
    }

    /// Runs a session over the marine forecast and alerts at the test point, answering each URL
    /// through `reply`; the measurements listed as failed.
    fn failed_through_a_session(reply: impl Fn(&str) -> (u16, Vec<u8>)) -> Vec<Measurement> {
        use crate::session::{Chain, Link, Session};
        use std::collections::BTreeMap;
        use std::sync::Arc;

        let link = Link {
            source: Arc::new(NwsMarine),
            target: target(),
        };
        let chains = MEASUREMENTS
            .iter()
            .map(|m| {
                let chain = Chain {
                    links: vec![link.clone()],
                    max_age_s: None,
                };
                (*m, chain)
            })
            .collect::<BTreeMap<_, _>>();
        let requirements = Requirements {
            measurements: MEASUREMENTS.into_iter().collect(),
            ..Requirements::default()
        };
        let mut session = Session::new(chains, requirements, ctx());
        while let Some(request) = session.next_request() {
            let (status, body) = reply(&request.url);
            session.on_response(&Response::new(status, &body));
        }
        let mut failed: Vec<Measurement> = session
            .finish(NOW)
            .errors
            .iter()
            .filter_map(|e| e.measurement)
            .collect();
        failed.sort();
        failed
    }

    #[test]
    fn nws_marine_a_point_outside_coverage_lists_no_error() {
        let body = fixture("points-45.0_-30.0-invalid-point.problem.json");
        let failed = failed_through_a_session(|_| (404, body.clone()));
        assert_eq!(failed, []);
    }

    #[test]
    fn nws_marine_a_zone_product_404_stays_listed() {
        let point = fixture("points-41.55_-81.72.geo.json");
        let zones = fixture("zones-point-41.55_-81.72.geo.json");
        let failed = failed_through_a_session(|url| {
            if url.contains("/points/") {
                (200, point.clone())
            } else if url.contains("/zones?") {
                (200, zones.clone())
            } else {
                (404, fixture("zone-forecast-LEZ146-not-found.problem.json"))
            }
        });
        assert!(failed.contains(&Measurement::MarineForecast), "{failed:?}");
    }

    #[test]
    fn nws_marine_a_land_point_falls_back_to_coastal_zones() {
        let plan = NwsMarine
            .plan(&target(), &Requirements::default(), &ctx())
            .unwrap();
        let body = br#"{"properties":{"type":"land"}}"#;
        let next = NwsMarine
            .follow_up(&plan[0], &Response::new(200, body), &ctx())
            .unwrap();
        assert_eq!(
            next[0].url,
            "https://api.weather.gov/zones?type=coastal&point=40.7,-74.02"
        );
        assert!(next[0].required);
        // Land zones only: no marine zone to read.
        let land =
            br#"{"features":[{"properties":{"id":"NYC061","type":"county","name":"New York"}}]}"#;
        let err = NwsMarine
            .follow_up(&next[0], &Response::new(200, land), &ctx())
            .unwrap_err();
        assert!(matches!(err, SourceError::NoData(_)));
    }

    #[test]
    fn nws_marine_plan_needs_a_point_and_a_wanted_measurement() {
        let mut t = target();
        t.measurements = BTreeSet::from([Measurement::Tide]);
        assert!(matches!(
            NwsMarine.plan(&t, &Requirements::default(), &ctx()),
            Err(SourceError::Unsupported(_))
        ));
        let only_alerts = Requirements {
            measurements: BTreeSet::from([Measurement::MarineAlerts]),
            ..Requirements::default()
        };
        let plan = NwsMarine.plan(&target(), &only_alerts, &ctx()).unwrap();
        assert_eq!(
            plan[0].target.measurements,
            BTreeSet::from([Measurement::MarineAlerts])
        );
        assert_eq!(
            NwsMarine.attribution().credit,
            "NOAA National Weather Service"
        );
    }
}
