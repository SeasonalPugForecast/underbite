//! NOAA CO-OPS tides, currents and station metadata.
//!
//! [`Coops`] plans and parses the data API ([`datagetter`]) for one station per target.

pub mod datagetter;
pub mod harmonics;
pub mod mdapi;

pub use harmonics::CoopsHarmonics;

use underbite_core::datum::{Heights, VerticalDatum};
use underbite_core::model::{CurrentSeries, Measurement, Requirements, TideSeries, TideSourceKind};
use underbite_core::provenance::{Attribution, SourceId, StationKind, StationRef};
use underbite_core::series::{Series, Vector};
use underbite_core::time::UnixTime;

use crate::request::{Place, Request, Resource, Response, Target};
use crate::source::{FetchContext, FragmentValue, Fragments, Source, SourceError, check_response};

use datagetter::{MAX_HILO_DAYS, MAX_INTERVAL_DAYS, MAX_SLACK_DAYS, Query};

/// Seconds a prediction may be reused from a cache.
const PREDICTION_AGE_S: u32 = 6 * 3_600;
/// Seconds an observation may be reused from a cache.
const OBSERVATION_AGE_S: u32 = 300;

const MEASUREMENTS: [Measurement; 5] = [
    Measurement::Tide,
    Measurement::TidalCurrent,
    Measurement::WaterTemperature,
    Measurement::ObservedWind,
    Measurement::ObservedPressure,
];

/// The CO-OPS data API as a source. It holds no settings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Coops;

/// One kind of request for a station.
struct Spec {
    resource: Resource,
    product: &'static str,
    interval: Option<&'static str>,
    measurement: Measurement,
    max_days: i64,
    /// Heights: sends and echoes the datum.
    height: bool,
    /// Observations are cut at `now`.
    observed: bool,
    /// `Some` fixes it; `None` means required unless a prediction is.
    required: Option<bool>,
}

impl Spec {
    fn prediction(
        resource: Resource,
        product: &'static str,
        interval: &'static str,
        measurement: Measurement,
        max_days: i64,
        required: bool,
    ) -> Self {
        Self {
            resource,
            product,
            interval: Some(interval),
            measurement,
            max_days,
            height: product == "predictions",
            observed: false,
            required: Some(required),
        }
    }

    fn observation(
        resource: Resource,
        product: &'static str,
        measurement: Measurement,
        height: bool,
        required: Option<bool>,
    ) -> Self {
        Self {
            resource,
            product,
            interval: None,
            measurement,
            max_days: MAX_INTERVAL_DAYS,
            height,
            observed: true,
            required,
        }
    }
}

fn datum_for(station: &StationRef, target: &Target) -> Result<VerticalDatum, SourceError> {
    let datum = if station.kind == StationKind::TideSubordinate {
        VerticalDatum::Mllw
    } else {
        target.datum.unwrap_or(VerticalDatum::Mllw)
    };
    if datum.coops_name().is_none() {
        return Err(SourceError::Unsupported(format!(
            "CO-OPS has no datum {datum:?}"
        )));
    }
    Ok(datum)
}

/// The requests a station of this kind answers for the measurements wanted.
fn specs(kind: StationKind, wants: impl Fn(Measurement) -> bool) -> Vec<Spec> {
    let mut specs = Vec::new();
    if wants(Measurement::Tide) {
        let tide = Measurement::Tide;
        let hilo = || {
            Spec::prediction(
                Resource::CoopsTideHighLow,
                "predictions",
                "hilo",
                tide,
                MAX_HILO_DAYS,
                true,
            )
        };
        let level =
            || Spec::observation(Resource::CoopsWaterLevel, "water_level", tide, true, None);
        match kind {
            StationKind::TideHarmonic => {
                specs.push(hilo());
                specs.push(Spec::prediction(
                    Resource::CoopsTideCurve,
                    "predictions",
                    "6",
                    tide,
                    MAX_INTERVAL_DAYS,
                    false,
                ));
                specs.push(level());
            }
            StationKind::TideSubordinate => specs.push(hilo()),
            StationKind::WaterLevel => specs.push(level()),
            _ => {}
        }
    }
    if wants(Measurement::TidalCurrent) {
        let current = Measurement::TidalCurrent;
        let events = || {
            Spec::prediction(
                Resource::CoopsCurrentEvents,
                "currents_predictions",
                "max_slack",
                current,
                MAX_SLACK_DAYS,
                true,
            )
        };
        match kind {
            StationKind::CurrentHarmonic => {
                specs.push(events());
                specs.push(Spec::prediction(
                    Resource::CoopsCurrentCurve,
                    "currents_predictions",
                    "6",
                    current,
                    MAX_INTERVAL_DAYS,
                    false,
                ));
            }
            StationKind::CurrentSubordinate | StationKind::CurrentWeakVariable => {
                specs.push(events());
            }
            StationKind::PortsCurrentMeter => specs.push(Spec::observation(
                Resource::CoopsCurrents,
                "currents",
                current,
                false,
                Some(false),
            )),
            _ => {}
        }
    }
    if matches!(
        kind,
        StationKind::TideHarmonic | StationKind::WaterLevel | StationKind::PortsCurrentMeter
    ) {
        for (m, resource, product) in [
            (
                Measurement::WaterTemperature,
                Resource::CoopsWaterTemperature,
                "water_temperature",
            ),
            (
                Measurement::ObservedPressure,
                Resource::CoopsAirPressure,
                "air_pressure",
            ),
            (Measurement::ObservedWind, Resource::CoopsWind, "wind"),
        ] {
            if wants(m) {
                specs.push(Spec::observation(resource, product, m, false, None));
            }
        }
    }
    specs
}

fn station_kind(request: &Request) -> Option<StationKind> {
    match &request.target.place {
        Place::Station { station, .. } => Some(station.kind),
        Place::Point(_) | Place::Zone(_) => None,
    }
}

/// A water-level station's observations as its tide: such a station has no predictions for
/// them to attach to, so they stand alone (no events, no curve).
fn observed_tide(level: Heights) -> TideSeries {
    TideSeries {
        // The station's own record passed through; the model has no observation-only kind.
        source_kind: TideSourceKind::HarmonicStation,
        datum: level.datum,
        events: Vec::new(),
        curve: None,
        provenance: level.series.provenance().clone(),
        observed: Some(level.series),
    }
}

/// A PORTS meter's observations as its tidal current: the meter has no predictions for them to
/// attach to, so they stand alone (no events, no curve, no flood or ebb direction).
fn observed_current(request: &Request, currents: Series<Vector>) -> CurrentSeries {
    let bin = match &request.target.place {
        Place::Station { station, .. } => station.bin,
        Place::Point(_) | Place::Zone(_) => None,
    };
    CurrentSeries {
        events: Vec::new(),
        curve: None,
        flood_dir_deg: None,
        ebb_dir_deg: None,
        bin,
        depth_m: None,
        provenance: currents.provenance().clone(),
        observed: Some(currents),
    }
}

impl Source for Coops {
    fn id(&self) -> SourceId {
        SourceId::COOPS
    }

    fn attribution(&self) -> Attribution {
        Attribution {
            source: SourceId::COOPS,
            credit: "Tide, water-level and current data: NOAA/NOS CO-OPS".into(),
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
        ctx: &FetchContext,
    ) -> Result<Vec<Request>, SourceError> {
        let Place::Station { station, .. } = &target.place else {
            return Err(SourceError::Unsupported("CO-OPS needs a station".into()));
        };
        if station.source != SourceId::COOPS {
            return Err(SourceError::Unsupported(format!(
                "{} is not a CO-OPS station",
                station.id
            )));
        }
        let specs = specs(station.kind, |m| {
            target.measurements.contains(&m)
                && (requirements.measurements.is_empty() || requirements.measurements.contains(&m))
        });
        if specs.is_empty() {
            return Err(SourceError::Unsupported(format!(
                "CO-OPS {:?} station {} offers none of {:?}",
                station.kind, station.id, target.measurements
            )));
        }
        let predicted = specs.iter().any(|s| s.required == Some(true));
        let datum = datum_for(station, target)?;
        let mut requests = Vec::new();
        for spec in &specs {
            let mut window = target.window;
            if spec.observed {
                window.to = UnixTime(window.to.0.min(ctx.now.0));
            }
            for chunk in datagetter::day_chunks(window, spec.max_days) {
                let url = datagetter::url(&Query {
                    product: spec.product,
                    station: &station.id,
                    window: chunk,
                    datum: spec.height.then(|| datum.coops_name()).flatten(),
                    interval: spec.interval,
                    bin: station.bin,
                });
                let chunk_target = Target {
                    place: target.place.clone(),
                    measurements: [spec.measurement].into(),
                    window: chunk,
                    datum: spec.height.then_some(datum),
                };
                let age = if spec.observed {
                    OBSERVATION_AGE_S
                } else {
                    PREDICTION_AGE_S
                };
                let mut request = Request::new(url, spec.resource, chunk_target).max_age_hint(age);
                if !spec.required.unwrap_or(!predicted) {
                    request = request.optional();
                }
                requests.push(request);
            }
        }
        Ok(requests)
    }

    fn parse(
        &self,
        request: &Request,
        response: &Response<'_>,
        ctx: &FetchContext,
        out: &mut Fragments,
    ) -> Result<(), SourceError> {
        let body = check_response(request, response)?;
        let now = ctx.now;
        let (measurement, value) = match request.resource {
            Resource::CoopsTideHighLow => (
                Measurement::Tide,
                FragmentValue::TideSeries(datagetter::tide_events(request, body, now)?),
            ),
            Resource::CoopsTideCurve => (
                Measurement::Tide,
                FragmentValue::TideSeries(datagetter::tide_curve(request, body, now)?),
            ),
            Resource::CoopsWaterLevel => {
                let level = datagetter::water_level(request, body)?;
                let value = if station_kind(request) == Some(StationKind::WaterLevel) {
                    FragmentValue::TideSeries(observed_tide(level))
                } else {
                    FragmentValue::Heights(level)
                };
                (Measurement::Tide, value)
            }
            Resource::CoopsWaterTemperature => (
                Measurement::WaterTemperature,
                FragmentValue::Series(datagetter::scalar(request, body, "degC")?),
            ),
            Resource::CoopsAirPressure => (
                Measurement::ObservedPressure,
                FragmentValue::Series(datagetter::scalar(request, body, "hPa")?),
            ),
            Resource::CoopsWind => (
                Measurement::ObservedWind,
                FragmentValue::Wind(datagetter::wind(request, body)?),
            ),
            Resource::CoopsCurrentEvents => (
                Measurement::TidalCurrent,
                FragmentValue::CurrentSeries(datagetter::current_events(request, body, now)?),
            ),
            Resource::CoopsCurrentCurve => (
                Measurement::TidalCurrent,
                FragmentValue::CurrentSeries(datagetter::current_curve(request, body, now)?),
            ),
            Resource::CoopsCurrents => {
                let currents = datagetter::observed_currents(request, body)?;
                let value = if station_kind(request) == Some(StationKind::PortsCurrentMeter) {
                    FragmentValue::CurrentSeries(observed_current(request, currents))
                } else {
                    FragmentValue::Vectors(currents)
                };
                (Measurement::TidalCurrent, value)
            }
            other => {
                return Err(SourceError::Unsupported(format!(
                    "{other:?} is not a CO-OPS data API resource"
                )));
            }
        };
        out.push(measurement, value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::Fragment;
    use crate::testutil::fixture_bytes;
    use datagetter::testing::{NOW, request as test_request, station_ref, target};

    const DAY0: i64 = 1_794_096_000; // 2026-11-08 00:00 UTC
    const DAY: i64 = 86_400;

    fn ctx() -> FetchContext {
        FetchContext::new(UnixTime(DAY0 + 10 * DAY), "underbite/test")
    }

    fn plan(id: &str, kind: StationKind, bin: Option<u16>, ms: &[Measurement]) -> Vec<Request> {
        let mut t = target(station_ref(id, kind, bin), ms[0], DAY0, DAY0 + 40 * DAY - 1);
        t.measurements = ms.iter().copied().collect();
        t.datum = Some(VerticalDatum::Msl);
        Coops.plan(&t, &Requirements::default(), &ctx()).unwrap()
    }

    fn count(requests: &[Request], resource: Resource) -> usize {
        requests.iter().filter(|r| r.resource == resource).count()
    }

    #[test]
    fn the_battery_plans_tide_level_and_temperature() {
        let requests = plan(
            "8518750",
            StationKind::TideHarmonic,
            None,
            &[Measurement::Tide, Measurement::WaterTemperature],
        );
        assert_eq!(count(&requests, Resource::CoopsTideHighLow), 1);
        assert_eq!(count(&requests, Resource::CoopsTideCurve), 2);
        assert_eq!(count(&requests, Resource::CoopsWaterLevel), 1);
        assert_eq!(count(&requests, Resource::CoopsWaterTemperature), 1);
        assert_eq!(requests.len(), 5);
        let hilo = &requests[0];
        assert!(hilo.required && hilo.url.contains("interval=hilo&format=json"));
        assert!(hilo.url.contains("datum=MSL&station=8518750"));
        assert!(hilo.url.contains("begin_date=20261108&end_date=20261217"));
        assert_eq!(hilo.target.datum, Some(VerticalDatum::Msl));
        let curves: Vec<_> = requests
            .iter()
            .filter(|r| r.resource == Resource::CoopsTideCurve)
            .collect();
        assert!(curves.iter().all(|r| !r.required));
        assert!(curves[0].url.contains("end_date=20261208"));
        assert!(curves[1].url.contains("begin_date=20261209"));
        let level = &requests[3];
        assert!(!level.required, "a prediction is required");
        assert!(level.url.contains("end_date=20261118"), "{}", level.url);
        let temp = &requests[4];
        assert!(!temp.url.contains("datum") && temp.target.datum.is_none());
    }

    #[test]
    fn a_water_level_station_requires_its_observations() {
        let requests = plan(
            "8531680",
            StationKind::WaterLevel,
            None,
            &[Measurement::Tide],
        );
        assert_eq!(requests.len(), 1);
        assert!(requests[0].required);
    }

    #[test]
    fn tarrytown_plans_hilo_in_mllw_only() {
        let requests = plan(
            "8518919",
            StationKind::TideSubordinate,
            None,
            &[Measurement::Tide],
        );
        assert_eq!(requests.len(), 1);
        assert!(requests[0].url.contains("datum=MLLW"));
        assert!(requests[0].url.contains("interval=hilo"));
        assert_eq!(requests[0].target.datum, Some(VerticalDatum::Mllw));
    }

    #[test]
    fn nyh1927_plans_events_and_curve_with_its_bin() {
        let requests = plan(
            "NYH1927",
            StationKind::CurrentHarmonic,
            Some(13),
            &[Measurement::TidalCurrent],
        );
        assert_eq!(count(&requests, Resource::CoopsCurrentEvents), 1);
        assert_eq!(count(&requests, Resource::CoopsCurrentCurve), 2);
        assert!(requests.iter().all(|r| r.url.contains("&bin=13")));
        assert!(requests[0].url.contains("interval=max_slack"));
        assert!(!requests[0].url.contains("vel_type"));
        assert!(requests[1].url.ends_with("&vel_type=default"));
        assert!(requests[0].required && !requests[1].required);
        assert!(requests.iter().all(|r| !r.url.contains("datum")));
    }

    #[test]
    fn act3646_plans_max_slack_only() {
        let requests = plan(
            "ACT3646",
            StationKind::CurrentSubordinate,
            Some(1),
            &[Measurement::TidalCurrent],
        );
        assert_eq!(requests.len(), 1);
        assert!(requests[0].url.contains("interval=max_slack"));
        assert!(requests[0].url.ends_with("&bin=1"));
    }

    #[test]
    fn ports_currents_are_optional_and_unsupported_asks_fail() {
        let requests = plan(
            "n06010",
            StationKind::PortsCurrentMeter,
            Some(4),
            &[Measurement::TidalCurrent],
        );
        assert!(requests.len() == 1 && !requests[0].required);
        let t = target(
            station_ref("8518919", StationKind::TideSubordinate, None),
            Measurement::ObservedWind,
            DAY0,
            DAY0 + DAY,
        );
        assert!(matches!(
            Coops.plan(&t, &Requirements::default(), &ctx()),
            Err(SourceError::Unsupported(_))
        ));
    }

    fn parse(
        resource: Resource,
        id: &str,
        kind: StationKind,
        bin: Option<u16>,
        m: Measurement,
        fixture: &str,
        status: u16,
    ) -> Result<Vec<Fragment>, SourceError> {
        let request = test_request(resource, station_ref(id, kind, bin), m);
        let bytes = fixture_bytes("coops", fixture);
        let mut out = Fragments::new();
        Coops.parse(
            &request,
            &Response::new(status, &bytes),
            &FetchContext::new(NOW, "underbite/test"),
            &mut out,
        )?;
        Ok(out.into_parts().0)
    }

    #[test]
    fn parse_dispatches_recorded_fixtures() {
        use Measurement as M;
        use StationKind as K;
        let cases = [
            (
                Resource::CoopsTideHighLow,
                "8518750",
                K::TideHarmonic,
                None,
                M::Tide,
                "coops-8518750-hilo-2026-11-08.json",
            ),
            (
                Resource::CoopsTideCurve,
                "8518750",
                K::TideHarmonic,
                None,
                M::Tide,
                "coops-8518750-predictions-6min-2026-11-08.json",
            ),
            (
                Resource::CoopsWaterLevel,
                "8518750",
                K::TideHarmonic,
                None,
                M::Tide,
                "coops-8518750-water-level-2026-11-07.json",
            ),
            (
                Resource::CoopsWaterTemperature,
                "8518750",
                K::TideHarmonic,
                None,
                M::WaterTemperature,
                "coops-8518750-water-temperature-2026-11-07.json",
            ),
            (
                Resource::CoopsAirPressure,
                "8518750",
                K::TideHarmonic,
                None,
                M::ObservedPressure,
                "coops-8518750-air-pressure-2026-11-07.json",
            ),
            (
                Resource::CoopsWind,
                "8531680",
                K::WaterLevel,
                None,
                M::ObservedWind,
                "coops-8531680-wind-2026-11-07.json",
            ),
            (
                Resource::CoopsCurrentEvents,
                "NYH1927",
                K::CurrentHarmonic,
                Some(13),
                M::TidalCurrent,
                "coops-NYH1927-bin13-max-slack-2026-11-08.json",
            ),
            (
                Resource::CoopsCurrentCurve,
                "NYH1927",
                K::CurrentHarmonic,
                Some(13),
                M::TidalCurrent,
                "coops-NYH1927-bin13-currents-predictions-6min-2026-11-08.json",
            ),
            (
                Resource::CoopsCurrents,
                "n06010",
                K::PortsCurrentMeter,
                Some(4),
                M::TidalCurrent,
                "coops-n06010-currents-2026-11-07.json",
            ),
        ];
        for (resource, id, kind, bin, m, fixture) in cases {
            let fragments = parse(resource, id, kind, bin, m, fixture, 200)
                .unwrap_or_else(|e| panic!("{fixture}: {e}"));
            assert_eq!(fragments.len(), 1, "{fixture}");
            assert_eq!(fragments[0].measurement, m, "{fixture}");
        }
        let hilo = parse(
            Resource::CoopsTideHighLow,
            "8518919",
            K::TideSubordinate,
            None,
            M::Tide,
            "coops-8518919-hilo-2026-11-08.json",
            200,
        )
        .unwrap();
        let FragmentValue::TideSeries(series) = &hilo[0].value else {
            panic!("not a tide series");
        };
        assert_eq!(series.source_kind, TideSourceKind::SubordinateStation);
    }

    #[test]
    fn observations_stand_alone_only_at_stations_without_predictions() {
        use Measurement as M;
        use StationKind as K;
        let level = |kind| {
            let fragments = parse(
                Resource::CoopsWaterLevel,
                "8518750",
                kind,
                None,
                M::Tide,
                "coops-8518750-water-level-2026-11-07.json",
                200,
            );
            fragments.unwrap().remove(0).value
        };
        let FragmentValue::TideSeries(tide) = level(K::WaterLevel) else {
            panic!("a water-level station's tide is its observations");
        };
        assert!(tide.events.is_empty() && tide.curve.is_none());
        let observed = tide.observed.unwrap();
        assert!(!observed.is_empty());
        assert_eq!(tide.datum, VerticalDatum::Mllw);
        assert_eq!(tide.provenance, *observed.provenance());
        assert!(matches!(level(K::TideHarmonic), FragmentValue::Heights(_)));

        let currents = parse(
            Resource::CoopsCurrents,
            "n06010",
            K::PortsCurrentMeter,
            Some(4),
            M::TidalCurrent,
            "coops-n06010-currents-2026-11-07.json",
            200,
        )
        .unwrap();
        let FragmentValue::CurrentSeries(current) = &currents[0].value else {
            panic!("a PORTS meter's current is its observations");
        };
        assert!(current.events.is_empty() && current.curve.is_none());
        assert_eq!(current.bin, Some(4));
        assert!(current.observed.as_ref().is_some_and(|o| !o.is_empty()));
    }

    #[test]
    fn parse_reports_errors_and_unsupported_resources() {
        use Measurement as M;
        let err = parse(
            Resource::CoopsTideCurve,
            "8518919",
            StationKind::TideSubordinate,
            None,
            M::Tide,
            "coops-error-subordinate-interval-6.json",
            200,
        )
        .unwrap_err();
        assert!(matches!(err, SourceError::NoData(_)));
        let err = parse(
            Resource::CoopsTideHighLow,
            "8518750",
            StationKind::TideHarmonic,
            None,
            M::Tide,
            "coops-error-no-product.txt",
            400,
        )
        .unwrap_err();
        assert!(matches!(err, SourceError::Status { status: 400, .. }));
        for resource in [Resource::CoopsAirTemperature, Resource::CoopsHarmonics] {
            let err = parse(
                resource,
                "8518750",
                StationKind::TideHarmonic,
                None,
                M::WaterTemperature,
                "coops-8518750-air-temperature-2026-11-07.json",
                200,
            )
            .unwrap_err();
            assert!(matches!(err, SourceError::Unsupported(_)), "{resource:?}");
        }
    }

    #[test]
    fn credit_names_noaa() {
        assert!(Coops.attribution().credit.contains("NOAA/NOS CO-OPS"));
        assert_eq!(Coops.id(), SourceId::COOPS);
        assert!(Coops.budget().is_none());
    }
}
