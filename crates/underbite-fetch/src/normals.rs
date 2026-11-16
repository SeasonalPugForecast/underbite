//! Where a USGS discharge or lake level stands among the normals of its day.

use std::collections::BTreeSet;

use jiff::tz::TimeZone;
use underbite_core::model::{
    Measurement, Percentile, SourceError as ModelSourceError, WaterConditions,
};
use underbite_core::provenance::SourceId;
use underbite_core::time::{local_date, standard_offset};
use underbite_providers::request::{Place, Request, Target, Window};
use underbite_providers::source::{FetchContext, FragmentValue, Fragments, Source, SourceError};
use underbite_providers::usgs::Usgs;
use underbite_providers::usgs::statistics::Reading;

use crate::host::{Driver, FetchHost};

/// Where the latest discharge or lake level stands among the normals of its day, from the
/// USGS Statistics API (or the site's daily means when it has none). Only values from a USGS
/// site have normals; the day is the site's standard-time day in the spot's zone.
pub(crate) async fn fetch_normals<H: FetchHost>(
    host: &Driver<'_, H>,
    ctx: &FetchContext,
    water: &mut WaterConditions,
    tz: Option<&TimeZone>,
) -> Vec<ModelSourceError> {
    let mut errors = Vec::new();
    let level_datum = water.level.as_ref().map(|h| h.datum);
    let wanted = [
        (Measurement::Discharge, water.discharge_m3s.as_ref(), None),
        (
            Measurement::LakeLevel,
            water.level.as_ref().map(|h| &h.series),
            level_datum,
        ),
    ];
    let mut found: Vec<(Measurement, Percentile)> = Vec::new();
    for (measurement, series, datum) in wanted {
        let Some(series) = series else { continue };
        let (Some(latest), Some(station)) = (series.latest(), &series.provenance().station) else {
            continue;
        };
        if station.source != SourceId::USGS {
            continue;
        }
        let Some(offset) = tz.and_then(|tz| {
            let date = local_date(latest.time, tz).ok()?;
            standard_offset(tz, date).ok()
        }) else {
            continue;
        };
        let provenance = series.provenance();
        let target = Target {
            place: Place::Station {
                station: station.clone(),
                distance_m: provenance.distance_m,
                relation: provenance.relation,
            },
            measurements: BTreeSet::from([measurement]),
            window: Window {
                from: host.now,
                to: host.now,
            },
            datum,
        };
        let reading = Reading {
            measurement,
            datum,
            value: latest.value,
            at: latest.time,
        };
        let usgs = Usgs::new();
        let first = match usgs.statistics_request(&target, &reading, offset, ctx) {
            Ok(request) => request,
            Err(e) => {
                errors.push(e.to_model(SourceId::USGS, Some(measurement)));
                continue;
            }
        };
        let mut queue = vec![first];
        let mut asked = 0;
        while let Some(request) = queue.pop() {
            asked += 1;
            let mut out = Fragments::new();
            let next = statistics(host, &usgs, &request, ctx, &mut out).await;
            match next {
                Ok(next) if asked <= usgs.max_follow_ups() => queue.extend(next),
                Ok(_) => {}
                Err(e) => errors.push(e.to_model(SourceId::USGS, Some(measurement))),
            }
            for fragment in out.fragments() {
                if let FragmentValue::Percentile(p) = &fragment.value {
                    found.push((fragment.measurement, p.clone()));
                }
            }
        }
    }
    for (measurement, percentile) in found {
        match measurement {
            Measurement::Discharge => water.discharge_percentile_for_date = Some(percentile),
            Measurement::LakeLevel => water.level_percentile_for_date = Some(percentile),
            _ => {}
        }
    }
    errors
}

/// One statistics request: its fragments into `out`, and the requests that follow it.
async fn statistics<H: FetchHost>(
    host: &Driver<'_, H>,
    usgs: &Usgs,
    request: &Request,
    ctx: &FetchContext,
    out: &mut Fragments,
) -> Result<Vec<Request>, SourceError> {
    let fetched = host.get(usgs, request).await?;
    let response = fetched.as_response();
    usgs.parse(request, &response, ctx, out)?;
    usgs.follow_up(request, &response, ctx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{MemoryHost, ctx, now};
    use underbite_core::provenance::{
        Provenance, Quality, Relation, StationKind, StationRef, UsgsSiteType, ValueKind,
    };
    use underbite_core::series::{Sample, Series};
    use underbite_core::time::{parse_rfc3339, zone};

    #[test]
    fn croton_discharge_takes_its_place_among_the_normals_of_8_november() {
        const CFS: f32 = 0.028_316_846;
        let host = MemoryHost::replay("croton");
        let dam = StationRef {
            source: SourceId::USGS,
            id: "USGS-01375000".into(),
            name: "CROTON R AT NEW CROTON DAM NR CROTON-ON-HUDSON NY".into(),
            kind: StationKind::UsgsSite(UsgsSiteType::Stream),
            latitude: 41.225_027_8,
            longitude: -73.858_888_9,
            bin: None,
            reference_id: None,
        };
        let at = parse_rfc3339("2026-11-08T14:15:00Z").expect("time");
        let provenance = Provenance {
            source: SourceId::USGS,
            station: Some(dam),
            distance_m: Some(20.0),
            relation: Relation::SameWaterBody,
            observed_at: at,
            kind: ValueKind::Observed,
            quality: Quality::Provisional,
        };
        let mut water = WaterConditions {
            discharge_m3s: Some(Series::new(provenance, vec![Sample::new(at, 12.4 * CFS)])),
            ..WaterConditions::default()
        };
        let tz = zone("America/New_York").expect("zone");
        pollster::block_on(async {
            let driver = Driver::new(&host, now(), true).await;
            let errors = fetch_normals(&driver, &ctx(), &mut water, Some(&tz)).await;
            assert_eq!(errors, []);
            let percentile = water
                .discharge_percentile_for_date
                .clone()
                .expect("percentile");
            assert_eq!(percentile.day_of_year, "11-08");
            assert!(percentile.thresholds.p50.is_some());
            let station = percentile
                .provenance
                .and_then(|p| p.station)
                .expect("station");
            assert_eq!(station.id, "USGS-01375000");
            assert!(host.seen().iter().any(|u| u.contains("observationNormals")));
            // Without a zone there is no standard-time day to ask about.
            let mut water = WaterConditions {
                discharge_m3s: water.discharge_m3s.take(),
                ..WaterConditions::default()
            };
            assert_eq!(fetch_normals(&driver, &ctx(), &mut water, None).await, []);
            assert!(water.discharge_percentile_for_date.is_none());
        });
    }
}
