//! The spot's water type, suggested from its NWS marine zone, its place on the mapped streams
//! and waterbodies, and the nearest USGS site.

use std::collections::BTreeSet;

use underbite_config::sources::SourcesConfig;
use underbite_core::discover::hydro::{
    self as hydrography, HydroOptions, Point, WaterTypeSuggestion,
};
use underbite_core::discover::rank::distance_m;
use underbite_core::model::{Measurement, Requirements, Spot, ZoneKind};
use underbite_core::provenance::{SourceId, StationKind, UsgsSiteType};
use underbite_providers::hydro;
use underbite_providers::nws_marine::NwsMarine;
use underbite_providers::nws_marine::zones as nws_zones;
use underbite_providers::request::{Coordinates, Request, Resource, RoundedPoint, Target};
use underbite_providers::source::{FetchContext, Source, SourceError, check_response};

use crate::host::{Call, Driver, FetchHost, Fetched};
use crate::hydrography::{HYDRO_RADIUS_M, hydro_pages};
use crate::lists::{StoredList, area_key, point_target, usgs_list};
use crate::store::{self, STATION_LIST_TTL_S};

/// How far from the spot the nearest USGS site may be to tell its water type, metres.
const NEAREST_SITE_M: f64 = HYDRO_RADIUS_M;

/// The kind of the NWS marine zone the spot lies in, through the same point and zone lookups
/// the marine forecast makes. `None` on land, outside the NWS's area or when a lookup fails.
async fn marine_zone<H: FetchHost>(
    host: &Driver<'_, H>,
    ctx: &FetchContext,
    spot: &Spot,
) -> Option<ZoneKind> {
    marine_zone_lookup(host, ctx, spot)
        .await
        .unwrap_or_else(|e| {
            tracing::debug!(error = %e, "marine zone of the spot");
            None
        })
}

/// [`marine_zone`], with why a lookup failed.
async fn marine_zone_lookup<H: FetchHost>(
    host: &Driver<'_, H>,
    ctx: &FetchContext,
    spot: &Spot,
) -> Result<Option<ZoneKind>, SourceError> {
    let target = Target {
        measurements: BTreeSet::from([Measurement::MarineForecast]),
        ..point_target(spot_point(ctx, spot), host.now)
    };
    let Some(point) = NwsMarine
        .plan(&target, &Requirements::default(), ctx)?
        .into_iter()
        .find(|r| r.resource == Resource::NwsPoint)
    else {
        return Ok(None);
    };
    let answer = send(host, &point).await?;
    let Some(zones) = NwsMarine
        .follow_up(&point, &answer.as_response(), ctx)?
        .into_iter()
        .find(|r| r.resource == Resource::NwsZones)
    else {
        return Ok(None);
    };
    let answer = send(host, &zones).await?;
    let body = check_response(&zones, &answer.as_response())?;
    Ok(nws_zones::marine_zone(body)?.map(|z| z.kind))
}

/// Sends `request` through the driver, outside the ledger.
async fn send<H: FetchHost>(
    host: &Driver<'_, H>,
    request: &Request,
) -> Result<Fetched, SourceError> {
    host.send(&Call::from(request))
        .await
        .map_err(|e| SourceError::Transport(e.to_string()))
}

/// The spot rounded by the context.
pub(crate) fn spot_point(ctx: &FetchContext, spot: &Spot) -> RoundedPoint {
    ctx.round(Coordinates {
        latitude: spot.latitude,
        longitude: spot.longitude,
    })
}

/// Where the spot lies on the mapped streams and waterbodies, from the same 3DHP queries
/// discovery makes around it.
async fn spot_position<H: FetchHost>(
    host: &Driver<'_, H>,
    ctx: &FetchContext,
    spot: &Spot,
) -> Result<hydrography::HydroPosition, SourceError> {
    let at = Coordinates {
        latitude: spot.latitude,
        longitude: spot.longitude,
    };
    let flowlines = hydro_pages(
        host,
        hydro::flowlines_query(at, HYDRO_RADIUS_M, ctx)?,
        hydro::parse_flowlines,
    )
    .await?;
    let waterbodies = hydro_pages(
        host,
        hydro::waterbodies_query(at, HYDRO_RADIUS_M, ctx)?,
        hydro::parse_waterbodies,
    )
    .await?;
    Ok(
        hydrography::Hydro::new(&flowlines, &waterbodies, HydroOptions::default())
            .locate_spot(Point::new(spot.latitude, spot.longitude)),
    )
}

/// The type of the nearest USGS site within [`NEAREST_SITE_M`], from the site list discovery
/// reads (the stored one while it is valid).
async fn nearest_usgs_site<H: FetchHost>(
    host: &Driver<'_, H>,
    ctx: &FetchContext,
    spot: &Spot,
) -> Option<UsgsSiteType> {
    let point = spot_point(ctx, spot);
    let stored = host
        .stored_within::<StoredList>(
            store::STATIONS,
            &area_key("usgs", point),
            STATION_LIST_TTL_S,
        )
        .await;
    let list = match stored {
        Some(list) => list,
        // Not stored here: discovery stores it, asking again through the kept responses.
        None => usgs_list(host, &point_target(point, host.now), ctx)
            .await
            .inspect_err(|e| tracing::debug!(error = %e, "USGS sites near the spot"))
            .ok()?,
    };
    list.candidates
        .iter()
        .filter_map(|c| match c.station.kind {
            StationKind::UsgsSite(site_type) => Some((
                distance_m(
                    spot.latitude,
                    spot.longitude,
                    c.station.latitude,
                    c.station.longitude,
                ),
                site_type,
            )),
            _ => None,
        })
        .filter(|(d, _)| *d <= NEAREST_SITE_M)
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, site_type)| site_type)
}

/// Suggests the spot's water type from its NWS marine zone, its place on the mapped streams and
/// waterbodies and the nearest USGS site, each looked up only when the configuration runs its
/// source. The responses are kept, so discovery and the sessions do not ask again.
pub(crate) async fn infer_water_type<H: FetchHost>(
    host: &Driver<'_, H>,
    ctx: &FetchContext,
    spot: &Spot,
    sources: &SourcesConfig,
) -> Option<WaterTypeSuggestion> {
    host.keeping(async {
        let zone = if sources.marine_forecast.chain.contains(&SourceId::NWS) {
            marine_zone(host, ctx, spot).await
        } else {
            None
        };
        // Offshore and Great Lakes zones settle it.
        let settled = matches!(
            zone,
            Some(ZoneKind::Offshore | ZoneKind::GreatLakesNearshore | ZoneKind::GreatLakesOpen)
        );
        let position = if settled {
            hydrography::HydroPosition::default()
        } else {
            spot_position(host, ctx, spot).await.unwrap_or_else(|e| {
                tracing::debug!(error = %e, "hydrography of the spot");
                hydrography::HydroPosition::default()
            })
        };
        let usgs = sources
            .groups()
            .iter()
            .any(|(_, g)| g.chain.contains(&SourceId::USGS));
        let site = if usgs && !settled {
            nearest_usgs_site(host, ctx, spot).await
        } else {
            None
        };
        hydrography::infer_water_type(&position, site, zone, HydroOptions::default())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{MemoryHost, battery, croton, ctx, now};
    use underbite_config::schema::Config;
    use underbite_core::discover::hydro::{WaterTypeReason, WaterbodyKind};

    fn inferred(
        host: &MemoryHost,
        spot: &Spot,
        sources: &SourcesConfig,
    ) -> Option<WaterTypeSuggestion> {
        pollster::block_on(async {
            let driver = Driver::new(host, now(), true).await;
            infer_water_type(&driver, &ctx(), spot, sources).await
        })
    }

    #[test]
    fn the_croton_below_the_dam_is_inferred_a_river() {
        // The recordings hold no NWS lookup for the Croton: the failed one leaves no zone.
        let host = MemoryHost::replay("croton").forgetful();
        let config = Config::default();
        let inferred = inferred(&host, &croton(), &config.sources).expect("inferred");
        assert_eq!(
            inferred.water_type,
            underbite_core::model::WaterType::RiverStream
        );
        assert_eq!(
            inferred.reasons,
            [WaterTypeReason::Waterbody {
                name: None,
                kind: WaterbodyKind::River,
                area_km2: 0.11,
            }]
        );
        assert!(
            host.missed().iter().all(|u| u.contains("api.weather.gov")),
            "{:#?}",
            host.missed()
        );
    }

    #[test]
    fn inference_asks_only_the_sources_the_configuration_runs() {
        let host = MemoryHost::replay("battery").forgetful();
        let mut config = Config::default();
        config.sources.marine_forecast.chain.clear();
        for group in [
            &mut config.sources.water_temperature,
            &mut config.sources.tide,
            &mut config.sources.currents,
            &mut config.sources.flow,
            &mut config.sources.lake_level,
            &mut config.sources.waves,
            &mut config.sources.sst,
            &mut config.sources.clarity,
            &mut config.sources.water_quality,
        ] {
            group.chain.retain(|id| *id != SourceId::USGS);
        }
        let inferred = inferred(&host, &battery(), &config.sources);
        let seen = host.seen();
        assert!(
            seen.iter().all(|u| !u.contains("api.weather.gov")
                && !u.contains("monitoring-locations")
                && !u.contains("time-series-metadata")),
            "{seen:?}"
        );
        // The hydrography alone: a river polygon with no tidal hint.
        assert!(inferred.is_none_or(|s| {
            s.reasons
                .iter()
                .all(|r| !matches!(r, WaterTypeReason::MarineZone { .. }))
        }),);
    }
}
