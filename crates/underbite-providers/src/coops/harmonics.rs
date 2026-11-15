//! Tides and currents computed on the device from CO-OPS harmonic constituents.
//!
//! [`CoopsHarmonics`] answers for a CO-OPS station without the data API: it asks the metadata
//! API for the station's datums and constituents (or, for a subordinate station, its offsets and
//! the reference station's), and computes the predictions in the parse of the last response.
//! The chains are:
//!
//! - harmonic tide station: datums, then constituents (carrying the datums);
//! - subordinate tide station: offsets, then the reference station's datums, then its
//!   constituents (carrying offsets and datums);
//! - harmonic current station: constituents of its bin;
//! - subordinate current station: offsets, then the reference station's constituents of the
//!   reference bin (carrying the offsets).
//!
//! Every request keeps the target of the planned station, so the result names that station;
//! only the URLs point at the reference station.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use underbite_core::astro::current::{CurrentError, CurrentPredictor};
use underbite_core::astro::tide::{SubordinateTideOffsets, TideError, TideSeriesBuilder};
use underbite_core::datum::{DatumTable, VerticalDatum};
use underbite_core::model::{CurrentSeries, Measurement, Requirements};
use underbite_core::provenance::{
    Attribution, Provenance, Quality, SourceId, StationKind, StationRef, ValueKind,
};

use super::mdapi;
use crate::request::{Place, Request, Resource, Response, Target};
use crate::source::{FetchContext, FragmentValue, Fragments, Source, SourceError, check_response};

/// Spacing of the computed curves.
const CURVE_STEP: Duration = Duration::from_mins(6);

const MEASUREMENTS: [Measurement; 2] = [Measurement::Tide, Measurement::TidalCurrent];

/// Tide and current predictions computed from CO-OPS harmonic constants. It holds no settings
/// and sends no metered requests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CoopsHarmonics;

/// What a tide request hands to the next one.
#[derive(Serialize, Deserialize)]
struct Carry {
    offsets: Option<SubordinateTideOffsets>,
    datums: Option<DatumTable>,
}

fn parse_error(e: impl std::fmt::Display) -> SourceError {
    SourceError::Parse(e.to_string())
}

fn tide_error(e: &TideError) -> SourceError {
    match e {
        TideError::UnknownConstituent(_) => SourceError::Parse(e.to_string()),
        TideError::MissingDatum { .. } => SourceError::NoData(Some(e.to_string())),
        _ => SourceError::Unsupported(e.to_string()),
    }
}

fn current_error(e: &CurrentError) -> SourceError {
    match e {
        CurrentError::ReferenceMismatch { .. } => SourceError::Unsupported(e.to_string()),
        _ => SourceError::Parse(e.to_string()),
    }
}

fn station_of(target: &Target) -> Result<&StationRef, SourceError> {
    match &target.place {
        Place::Station { station, .. } if station.source == SourceId::COOPS => Ok(station),
        _ => Err(SourceError::Unsupported(
            "computed predictions need a CO-OPS station".into(),
        )),
    }
}

/// `target` with the station replaced by the reference station `id`.
fn reference_target(target: &Target, id: &str, kind: StationKind, bin: Option<u16>) -> Target {
    let mut out = target.clone();
    if let Place::Station { station, .. } = &mut out.place {
        *station = StationRef {
            id: id.to_owned(),
            name: id.to_owned(),
            kind,
            bin,
            reference_id: None,
            ..station.clone()
        };
    }
    out
}

fn body_text(body: &[u8]) -> Result<&str, SourceError> {
    std::str::from_utf8(body).map_err(parse_error)
}

impl CoopsHarmonics {
    fn base(target: &Target, ctx: &FetchContext) -> Result<Provenance, SourceError> {
        let Place::Station {
            station,
            distance_m,
            relation,
        } = &target.place
        else {
            return Err(SourceError::Unsupported("a station target".into()));
        };
        Ok(Provenance {
            source: SourceId::COOPS_HARMONICS,
            station: Some(station.clone()),
            distance_m: *distance_m,
            relation: *relation,
            observed_at: ctx.now,
            kind: ValueKind::Computed,
            quality: Quality::Approved,
        })
    }

    fn tide(
        request: &Request,
        body: &[u8],
        ctx: &FetchContext,
        out: &mut Fragments,
    ) -> Result<(), SourceError> {
        let carry: Carry = serde_json::from_str(
            request
                .carry()
                .ok_or_else(|| parse_error("constituents without their datums"))?,
        )
        .map_err(parse_error)?;
        let table = carry
            .datums
            .ok_or_else(|| parse_error("constituents without a datum table"))?;
        let station = station_of(&request.target)?;
        let (reference_id, datum) = match &carry.offsets {
            Some(o) => (o.reference_id.as_str(), VerticalDatum::Mllw),
            None => (
                station.id.as_str(),
                request.target.datum.unwrap_or(VerticalDatum::Mllw),
            ),
        };
        let harmonics = mdapi::parse_tide_harmonics(reference_id, body, &table, datum)?;
        let builder = TideSeriesBuilder {
            from: request.target.window.from,
            to: request.target.window.to,
            step: CURVE_STEP,
            base: Self::base(&request.target, ctx)?,
        };
        let series = match &carry.offsets {
            Some(offsets) => builder.subordinate(&harmonics, offsets),
            None => builder.harmonic(&harmonics),
        }
        .map_err(|e| tide_error(&e))?;
        out.push(Measurement::Tide, FragmentValue::TideSeries(series))
    }

    fn current(
        request: &Request,
        body: &[u8],
        ctx: &FetchContext,
        out: &mut Fragments,
    ) -> Result<(), SourceError> {
        let station = station_of(&request.target)?;
        let (from, to) = (request.target.window.from, request.target.window.to);
        let provenance = Self::base(&request.target, ctx)?;
        let series = match request.carry() {
            None => {
                let harmonics = mdapi::parse_current_harmonics(&station.id, station.bin, body)?;
                let predictor = CurrentPredictor::new(&harmonics).map_err(|e| current_error(&e))?;
                predictor.series(from, to, CURVE_STEP.as_secs().cast_signed(), provenance)
            }
            Some(offsets_body) => {
                let offsets = mdapi::parse_current_offsets(station.bin, offsets_body.as_bytes())?;
                let harmonics = mdapi::parse_current_harmonics(
                    &offsets.reference_id,
                    Some(offsets.reference_bin),
                    body,
                )?;
                let predictor = CurrentPredictor::new(&harmonics).map_err(|e| current_error(&e))?;
                CurrentSeries {
                    events: predictor
                        .subordinate_events(&offsets, from, to)
                        .map_err(|e| current_error(&e))?,
                    curve: None,
                    flood_dir_deg: offsets.flood_dir_deg,
                    ebb_dir_deg: offsets.ebb_dir_deg,
                    bin: offsets.bin,
                    depth_m: None,
                    observed: None,
                    provenance,
                }
            }
        };
        out.push(
            Measurement::TidalCurrent,
            FragmentValue::CurrentSeries(series),
        )
    }
}

impl Source for CoopsHarmonics {
    fn id(&self) -> SourceId {
        SourceId::COOPS_HARMONICS
    }

    fn attribution(&self) -> Attribution {
        Attribution {
            source: SourceId::COOPS_HARMONICS,
            credit: "Tide and current predictions computed from NOAA/NOS CO-OPS harmonic \
                     constants"
                .into(),
            url: Some("https://tidesandcurrents.noaa.gov/".into()),
        }
    }

    fn measurements(&self) -> &[Measurement] {
        &MEASUREMENTS
    }

    fn plan(
        &self,
        target: &Target,
        requirements: &Requirements,
        _ctx: &FetchContext,
    ) -> Result<Vec<Request>, SourceError> {
        let station = station_of(target)?;
        let wants = |m: Measurement| {
            target.measurements.contains(&m)
                && (requirements.measurements.is_empty() || requirements.measurements.contains(&m))
        };
        let tide = wants(Measurement::Tide);
        let current = wants(Measurement::TidalCurrent);
        let request = match station.kind {
            StationKind::TideHarmonic if tide => mdapi::datums_request(target.clone())?,
            StationKind::TideSubordinate if tide => mdapi::tide_offsets_request(target.clone())?,
            StationKind::CurrentHarmonic if current => {
                mdapi::current_harcon_request(target.clone())?
            }
            StationKind::CurrentSubordinate if current => {
                mdapi::current_offsets_request(target.clone())?
            }
            kind => {
                return Err(SourceError::Unsupported(format!(
                    "no computed predictions of {:?} for a {kind:?} station",
                    target.measurements
                )));
            }
        };
        Ok(vec![request])
    }

    fn follow_up(
        &self,
        request: &Request,
        response: &Response<'_>,
        _ctx: &FetchContext,
    ) -> Result<Vec<Request>, SourceError> {
        let target = &request.target;
        let station = station_of(target)?;
        let next = match request.resource {
            Resource::CoopsTideOffsets => {
                let body = check_response(request, response)?;
                let offsets = mdapi::parse_tide_offsets(body)?.ok_or_else(|| {
                    SourceError::Unsupported(format!(
                        "station {} is a reference station",
                        station.id
                    ))
                })?;
                let reference = reference_target(
                    target,
                    &offsets.reference_id,
                    StationKind::TideHarmonic,
                    None,
                );
                let carry = Carry {
                    offsets: Some(offsets),
                    datums: None,
                };
                let mut next = mdapi::datums_request(reference)?
                    .with_carry(serde_json::to_string(&carry).map_err(parse_error)?);
                next.target = target.clone();
                next
            }
            Resource::CoopsDatums => {
                let body = check_response(request, response)?;
                let mut carry = match request.carry() {
                    Some(text) => serde_json::from_str(text).map_err(parse_error)?,
                    None => Carry {
                        offsets: None,
                        datums: None,
                    },
                };
                let reference = match &carry.offsets {
                    Some(o) => {
                        reference_target(target, &o.reference_id, StationKind::TideHarmonic, None)
                    }
                    None => target.clone(),
                };
                let id = station_of(&reference)?.id.clone();
                let table = mdapi::parse_datums(&id, body)?;
                let next = mdapi::tide_harcon_request(reference, &table)?;
                carry.datums = Some(table);
                let mut next = next.with_carry(serde_json::to_string(&carry).map_err(parse_error)?);
                next.target = target.clone();
                next
            }
            Resource::CoopsCurrentOffsets => {
                let body = check_response(request, response)?;
                let offsets = mdapi::parse_current_offsets(station.bin, body)?;
                let reference = reference_target(
                    target,
                    &offsets.reference_id,
                    StationKind::CurrentHarmonic,
                    Some(offsets.reference_bin),
                );
                let mut next =
                    mdapi::current_harcon_request(reference)?.with_carry(body_text(body)?);
                next.target = target.clone();
                next
            }
            _ => return Ok(Vec::new()),
        };
        Ok(vec![next])
    }

    fn parse(
        &self,
        request: &Request,
        response: &Response<'_>,
        ctx: &FetchContext,
        out: &mut Fragments,
    ) -> Result<(), SourceError> {
        let body = check_response(request, response)?;
        match request.resource {
            Resource::CoopsDatums | Resource::CoopsTideOffsets | Resource::CoopsCurrentOffsets => {
                Ok(())
            }
            Resource::CoopsHarmonics => match station_of(&request.target)?.kind {
                StationKind::TideHarmonic | StationKind::TideSubordinate => {
                    Self::tide(request, body, ctx, out)
                }
                StationKind::CurrentHarmonic | StationKind::CurrentSubordinate => {
                    Self::current(request, body, ctx, out)
                }
                kind => Err(SourceError::Unsupported(format!("{kind:?} station"))),
            },
            other => Err(SourceError::Unsupported(format!(
                "{other:?} is not a metadata resource"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use underbite_core::model::{CurrentKind, TideEvent, TideSourceKind};
    use underbite_core::time::UnixTime;

    use super::*;
    use crate::coops::Coops;
    use crate::coops::datagetter::{self, testing::station_ref, testing::target};
    use crate::session::{Chain, Link, Session, SessionOutcome};
    use crate::testutil::fixture_bytes;

    const DAY0: i64 = 1_794_096_000; // 2026-11-08 00:00 UTC
    const DAY: i64 = 86_400;
    const NOW: UnixTime = UnixTime(DAY0 - DAY);

    fn fx(name: &str) -> Vec<u8> {
        fixture_bytes("coops", name)
    }

    fn week_target(station: StationRef, m: Measurement) -> Target {
        target(station, m, DAY0, DAY0 + 7 * DAY - 1)
    }

    /// Runs a session over `chains`, answering the metadata URLs from `answers` (a URL
    /// fragment and a fixture) and failing every other request with 503. Returns the outcome
    /// and the URLs sent.
    fn drive(
        chains: BTreeMap<Measurement, Chain>,
        answers: &[(&str, &str)],
    ) -> (SessionOutcome, Vec<String>) {
        let requirements = Requirements {
            measurements: chains.keys().copied().collect(),
            ..Requirements::default()
        };
        let mut session = Session::new(
            chains,
            requirements,
            FetchContext::new(NOW, "underbite/test"),
        );
        let mut urls = Vec::new();
        while let Some(request) = session.next_request() {
            let url = request.url.clone();
            let body = answers
                .iter()
                .find(|(part, _)| url.contains(part))
                .map(|(_, name)| fx(name));
            match body {
                Some(body) => session.on_response(&Response::new(200, &body)),
                None => session.on_response(&Response::new(503, b"Service Unavailable")),
            }
            urls.push(url);
        }
        (session.finish(NOW), urls)
    }

    fn links(
        station: &StationRef,
        m: Measurement,
        with_coops: bool,
    ) -> BTreeMap<Measurement, Chain> {
        let t = week_target(station.clone(), m);
        let mut chain = Chain::default();
        if with_coops {
            chain.links.push(Link {
                source: Arc::new(Coops),
                target: t.clone(),
            });
        }
        chain.links.push(Link {
            source: Arc::new(CoopsHarmonics),
            target: t,
        });
        BTreeMap::from([(m, chain)])
    }

    fn recorded_events(name: &str) -> Vec<TideEvent> {
        let request = Request::new(
            "https://example.test/".into(),
            Resource::CoopsTideHighLow,
            week_target(
                station_ref("8518750", StationKind::TideHarmonic, None),
                Measurement::Tide,
            ),
        );
        datagetter::tide_events(&request, &fx(name), NOW)
            .unwrap()
            .events
    }

    /// Worst time error (minutes) and height error (metres) against `theirs`, event by event.
    fn event_errors(ours: &[TideEvent], theirs: &[TideEvent]) -> (i64, f32) {
        assert_eq!(ours.len(), theirs.len(), "event count");
        let mut worst = (0, 0.0_f32);
        for (a, b) in ours.iter().zip(theirs) {
            assert_eq!(a.kind, b.kind, "kind at {:?}", b.time);
            let dt = (a.time.0 - b.time.0).abs() / 60;
            let dh = (a.height_m.unwrap() - b.height_m.unwrap()).abs();
            worst = (worst.0.max(dt), worst.1.max(dh));
        }
        worst
    }

    const BATTERY: [(&str, &str); 2] = [
        ("/datums.json", "coops-mdapi-8518750-datums-metric.json"),
        ("/harcon.json", "coops-mdapi-8518750-harcon-metric.json"),
    ];

    #[test]
    fn the_battery_is_computed_when_coops_is_down() {
        let station = station_ref("8518750", StationKind::TideHarmonic, None);
        let (out, urls) = drive(links(&station, Measurement::Tide, true), &BATTERY);
        assert_eq!(urls.len(), 3, "{urls:?}");
        assert!(urls[0].contains("datagetter"), "{urls:?}");
        assert!(urls[1].ends_with("datums.json?units=metric"), "{urls:?}");
        assert!(urls[2].ends_with("harcon.json?units=metric"), "{urls:?}");

        assert_eq!(out.sources[&Measurement::Tide], SourceId::COOPS_HARMONICS);
        assert_eq!(out.errors.len(), 1);
        assert_eq!(out.errors[0].source, SourceId::COOPS);
        assert_eq!(
            out.errors[0].message,
            "coops: HTTP 503: Service Unavailable, used coops-harmonics"
        );
        assert!(
            out.attribution[0]
                .credit
                .contains("computed from NOAA/NOS CO-OPS harmonic constants")
        );

        let tide = out.marine.tide.expect("a computed tide");
        assert_eq!(tide.source_kind, TideSourceKind::Computed);
        assert_eq!(tide.datum, VerticalDatum::Mllw);
        assert_eq!(tide.provenance.kind, ValueKind::Computed);
        assert_eq!(tide.provenance.quality, Quality::Approved);
        assert_eq!(tide.provenance.source, SourceId::COOPS_HARMONICS);
        assert_eq!(out.stations[0].id, "8518750");

        let recorded = recorded_events("coops-8518750-hilo-2026-11-08.json");
        assert_eq!(recorded.len(), 27);
        let (dt, dh) = event_errors(&tide.events, &recorded);
        assert!(dt <= 6 && dh <= 0.04, "{dt} min, {dh} m");
        let curve = tide.curve.expect("a curve");
        assert_eq!(curve.samples().len(), 7 * 240);
    }

    #[test]
    fn the_metadata_requests_may_be_cached_for_weeks() {
        let station = station_ref("8518750", StationKind::TideHarmonic, None);
        let t = week_target(station, Measurement::Tide);
        let ctx = FetchContext::new(NOW, "underbite/test");
        let plan = CoopsHarmonics
            .plan(&t, &Requirements::default(), &ctx)
            .unwrap();
        assert_eq!(plan.len(), 1);
        assert!(plan[0].required);
        assert_eq!(plan[0].cache.max_age, Some(mdapi::STATION_AGE_S));
        assert!(CoopsHarmonics.budget().is_none());
    }

    #[test]
    fn tarrytown_goes_through_the_reference_station() {
        let station = station_ref("8518919", StationKind::TideSubordinate, None);
        let answers = [
            (
                "8518919/tidepredoffsets",
                "coops-mdapi-8518919-tidepredoffsets.json",
            ),
            (
                "8518750/datums.json",
                "coops-mdapi-8518750-datums-metric.json",
            ),
            (
                "8518750/harcon.json",
                "coops-mdapi-8518750-harcon-metric.json",
            ),
        ];
        let (out, urls) = drive(links(&station, Measurement::Tide, false), &answers);
        assert_eq!(urls.len(), 3, "{urls:?}");
        assert!(out.errors.is_empty(), "{:?}", out.errors);
        let tide = out.marine.tide.expect("a computed tide");
        assert_eq!(tide.source_kind, TideSourceKind::SubordinateStation);
        assert_eq!(tide.datum, VerticalDatum::Mllw);
        assert_eq!(out.stations[0].id, "8518919");
        assert_eq!(tide.provenance.station.as_ref().unwrap().id, "8518919");
        assert_eq!(tide.provenance.kind, ValueKind::Computed);
        let curve = tide.curve.as_ref().expect("a curve");
        assert_eq!(curve.provenance().quality, Quality::Estimated);

        let recorded = recorded_events("coops-8518919-hilo-2026-11-08.json");
        let first = tide.events[0].time;
        let theirs: Vec<_> = recorded.into_iter().filter(|e| e.time >= first).collect();
        let ours: Vec<_> = tide
            .events
            .iter()
            .copied()
            .filter(|e| e.time <= theirs[theirs.len() - 1].time)
            .collect();
        let (dt, dh) = event_errors(&ours, &theirs);
        assert!(dt <= 6 && dh <= 0.04, "{dt} min, {dh} m");
    }

    #[test]
    fn a_reference_station_has_no_offsets() {
        let station = station_ref("8518750", StationKind::TideSubordinate, None);
        let answers = [(
            "tidepredoffsets",
            "coops-mdapi-8518919-tidepredoffsets.json",
        )];
        let t = week_target(station, Measurement::Tide);
        let ctx = FetchContext::new(NOW, "underbite/test");
        let plan = CoopsHarmonics
            .plan(&t, &Requirements::default(), &ctx)
            .unwrap();
        let body = fx(answers[0].1);
        // Offsets with an empty reference mean a reference station.
        let reference = String::from_utf8(body).unwrap().replace("8518750", "");
        let err = CoopsHarmonics
            .follow_up(&plan[0], &Response::new(200, reference.as_bytes()), &ctx)
            .unwrap_err();
        assert!(matches!(err, SourceError::Unsupported(_)), "{err:?}");
    }

    #[test]
    fn the_battery_currents_of_nyh1927_are_computed() {
        let station = station_ref("NYH1927", StationKind::CurrentHarmonic, Some(13));
        let answers = [("NYH1927/harcon.json", "coops-mdapi-NYH1927-harcon.json")];
        let (out, urls) = drive(links(&station, Measurement::TidalCurrent, true), &answers);
        assert_eq!(urls.len(), 2, "{urls:?}");
        assert!(urls[1].ends_with("harcon.json?bin=13"), "{urls:?}");
        assert_eq!(out.errors.len(), 1);
        let current = out.marine.tidal_current.expect("computed currents");
        assert_eq!(current.bin, Some(13));
        assert_eq!(current.provenance.kind, ValueKind::Computed);
        assert_eq!(current.provenance.source, SourceId::COOPS_HARMONICS);

        let request = Request::new(
            "https://example.test/".into(),
            Resource::CoopsCurrentEvents,
            week_target(station, Measurement::TidalCurrent),
        );
        let recorded = datagetter::current_events(
            &request,
            &fx("coops-NYH1927-bin13-max-slack-2026-11-08.json"),
            NOW,
        )
        .unwrap()
        .events;
        assert_eq!(current.events.len(), recorded.len());
        for (a, b) in current.events.iter().zip(&recorded) {
            assert_eq!(a.kind, b.kind);
            assert!((a.time.0 - b.time.0).abs() <= 10 * 60, "{:?}", b.time);
            if a.kind != CurrentKind::Slack {
                assert!((a.speed_ms - b.speed_ms).abs() <= 0.05, "{:?}", b.time);
            }
        }
        let curve = current.curve.expect("a curve");
        assert_eq!(curve.samples().len(), 7 * 240);
    }

    #[test]
    fn a_subordinate_current_station_uses_the_reference_bin() {
        let station = station_ref("ACT3646", StationKind::CurrentSubordinate, Some(1));
        let answers = [
            (
                "ACT3646_1/currentpredictionoffsets",
                "coops-mdapi-ACT3646_1-currentpredictionoffsets.json",
            ),
            (
                "n03020/harcon.json?bin=7",
                "coops-mdapi-n03020-harcon-bin7.json",
            ),
        ];
        let (out, urls) = drive(links(&station, Measurement::TidalCurrent, false), &answers);
        assert_eq!(urls.len(), 2, "{urls:?}");
        assert!(out.errors.is_empty(), "{:?}", out.errors);
        let current = out.marine.tidal_current.expect("computed currents");
        assert!(current.curve.is_none());
        assert_eq!(current.flood_dir_deg, Some(31.0));
        assert_eq!(current.ebb_dir_deg, Some(205.0));
        assert_eq!(current.bin, Some(1));
        assert_eq!(out.stations[0].id, "ACT3646");
        assert!(current.events.len() > 20, "{}", current.events.len());
    }

    #[test]
    fn other_stations_are_not_supported() {
        let t = week_target(
            station_ref("8531680", StationKind::WaterLevel, None),
            Measurement::Tide,
        );
        let ctx = FetchContext::new(NOW, "underbite/test");
        assert!(matches!(
            CoopsHarmonics.plan(&t, &Requirements::default(), &ctx),
            Err(SourceError::Unsupported(_))
        ));
    }
}
