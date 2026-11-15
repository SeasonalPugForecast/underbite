//! NOAA National Data Buoy Center.
//!
//! [`Ndbc`] plans and parses the real-time station files ([`realtime`]); the station list is
//! read by [`stations`]. A station's coverage, meaning the measurements whose columns actually
//! hold values, comes from [`Ndbc::coverage_request`]: the flags in the station list are only
//! hints, since a buoy can be listed with meteorological sensors and report none.

pub mod realtime;
pub mod stations;

use underbite_core::model::{Coverage, Measurement, Requirements};
use underbite_core::provenance::{Attribution, SourceId};

use crate::request::{Place, Request, Resource, Response, Target};
use crate::source::{FetchContext, FragmentValue, Fragments, Source, SourceError, check_response};

/// Where the real-time files are.
const BASE: &str = "https://www.ndbc.noaa.gov/data/realtime2";

/// Seconds a station file may be reused: NDBC asks for no more than one fetch per 10 minutes.
const FILE_AGE_S: u32 = 600;

/// Carried by a request whose response is read for coverage only.
const COVERAGE_CARRY: &str = "coverage";

const MEASUREMENTS: [Measurement; 9] = [
    Measurement::Waves,
    Measurement::ObservedWind,
    Measurement::ObservedPressure,
    Measurement::WaterTemperature,
    Measurement::Salinity,
    Measurement::Turbidity,
    Measurement::Chlorophyll,
    Measurement::Ph,
    Measurement::OceanCurrent,
];

/// The NDBC real-time files as a source. It holds no settings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ndbc;

/// The extension of a station file resource.
fn extension(resource: Resource) -> Option<&'static str> {
    match resource {
        Resource::NdbcStandard => Some("txt"),
        Resource::NdbcSpectralSummary => Some("spec"),
        Resource::NdbcOcean => Some("ocean"),
        Resource::NdbcAdcp => Some("adcp"),
        _ => None,
    }
}

/// The URL of a station file. Land stations are listed in lower case but their files need
/// upper case (`BATN6.txt`); buoy ids are digits and unaffected.
fn url(id: &str, resource: Resource) -> Result<String, SourceError> {
    let extension = extension(resource)
        .ok_or_else(|| SourceError::Unsupported(format!("{resource:?} is not a station file")))?;
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Err(SourceError::Unsupported(format!(
            "{id:?} is not an NDBC station id"
        )));
    }
    Ok(format!("{BASE}/{}.{extension}", id.to_ascii_uppercase()))
}

fn station_id(target: &Target) -> Result<&str, SourceError> {
    match &target.place {
        Place::Station { station, .. } if station.source == SourceId::NDBC => Ok(&station.id),
        _ => Err(SourceError::Unsupported(
            "NDBC needs an NDBC station".into(),
        )),
    }
}

/// The file that serves a measurement, and whether the file is the main one for it.
fn files(measurement: Measurement) -> &'static [Resource] {
    match measurement {
        Measurement::Waves => &[Resource::NdbcStandard, Resource::NdbcSpectralSummary],
        Measurement::ObservedWind
        | Measurement::ObservedPressure
        | Measurement::WaterTemperature => &[Resource::NdbcStandard],
        Measurement::Salinity
        | Measurement::Turbidity
        | Measurement::Chlorophyll
        | Measurement::Ph => &[Resource::NdbcOcean],
        Measurement::OceanCurrent => &[Resource::NdbcAdcp],
        _ => &[],
    }
}

impl Ndbc {
    /// A request for `resource` of the station in `target` that is read for coverage only: the
    /// measurements whose columns hold values, with the time of the newest one, come back as
    /// [`FragmentValue::Coverage`]. It is optional, so a station without that file (a 404
    /// page) does not fail the link.
    pub fn coverage_request(
        &self,
        target: &Target,
        resource: Resource,
    ) -> Result<Request, SourceError> {
        let url = url(station_id(target)?, resource)?;
        Ok(Request::new(url, resource, target.clone())
            .optional()
            .max_age_hint(FILE_AGE_S)
            .with_carry(COVERAGE_CARRY))
    }
}

impl Source for Ndbc {
    fn id(&self) -> SourceId {
        SourceId::NDBC
    }

    fn attribution(&self) -> Attribution {
        Attribution {
            source: SourceId::NDBC,
            credit: "Buoy observations: NOAA National Data Buoy Center".into(),
            url: Some("https://www.ndbc.noaa.gov/".into()),
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
        let id = station_id(target)?;
        let wanted: Vec<Measurement> = target
            .measurements
            .iter()
            .copied()
            .filter(|m| {
                requirements.measurements.is_empty() || requirements.measurements.contains(m)
            })
            .filter(|m| !files(*m).is_empty())
            .collect();
        let mut requests: Vec<Request> = Vec::new();
        for resource in [
            Resource::NdbcStandard,
            Resource::NdbcSpectralSummary,
            Resource::NdbcOcean,
            Resource::NdbcAdcp,
        ] {
            let measurements: std::collections::BTreeSet<Measurement> = wanted
                .iter()
                .copied()
                .filter(|m| files(*m).contains(&resource))
                .collect();
            if measurements.is_empty() {
                continue;
            }
            let file_target = Target {
                measurements,
                ..target.clone()
            };
            let mut request =
                Request::new(url(id, resource)?, resource, file_target).max_age_hint(FILE_AGE_S);
            // Only the first file fails the link: stations rarely carry every file, and
            // `.spec` repeats what `.txt` has with more detail.
            if !requests.is_empty() || resource == Resource::NdbcSpectralSummary {
                request = request.optional();
            }
            requests.push(request);
        }
        if requests.is_empty() {
            return Err(SourceError::Unsupported(format!(
                "NDBC offers none of {:?}",
                target.measurements
            )));
        }
        Ok(requests)
    }

    fn parse(
        &self,
        request: &Request,
        response: &Response<'_>,
        _ctx: &FetchContext,
        out: &mut Fragments,
    ) -> Result<(), SourceError> {
        let resource = request.resource;
        let station_file = extension(resource).is_some();
        let body = match check_response(request, response) {
            // A station without the optional files answers 404 with an HTML page.
            Err(SourceError::NotFound(_)) if station_file && resource != Resource::NdbcStandard => {
                return Err(SourceError::NoData(Some(format!(
                    "the station has no .{} file",
                    extension(resource).unwrap_or_default()
                ))));
            }
            other => other?,
        };
        let text = std::str::from_utf8(body)
            .map_err(|_| SourceError::Parse("not valid text".to_string()))?;
        if !station_file {
            return Err(SourceError::Unsupported(format!(
                "{resource:?} is read with ndbc::stations::parse"
            )));
        }
        let coverage = request.carry() == Some(COVERAGE_CARRY);
        let window = (!coverage).then_some(request.target.window);
        let wants = |m: Measurement| coverage || request.target.measurements.contains(&m);
        let built = match resource {
            Resource::NdbcStandard => realtime::standard(text, request, window, wants)?,
            Resource::NdbcSpectralSummary => realtime::spectral(text, request, window, wants)?,
            Resource::NdbcOcean => realtime::ocean(text, request, window, wants)?,
            _ => realtime::adcp(text, request, window, wants)?,
        };
        if coverage {
            let id = station_id(&request.target)?;
            for b in built {
                out.push(
                    b.measurement,
                    FragmentValue::Coverage {
                        station_id: id.to_string(),
                        coverage: Coverage {
                            last_value_at: Some(b.last_value_at),
                            primary: b.primary,
                        },
                    },
                )?;
            }
            return Ok(());
        }
        if built.is_empty() {
            return Err(SourceError::NoData(Some(
                "no values for the measurements wanted".to_string(),
            )));
        }
        for b in built {
            out.push(b.measurement, b.value)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use underbite_core::provenance::{Relation, StationKind, StationRef};
    use underbite_core::time::UnixTime;

    use super::*;
    use crate::request::Window;
    use crate::source::Fragment;
    use crate::testutil::fixture_bytes;

    fn target(id: &str, kind: StationKind, ms: &[Measurement]) -> Target {
        Target {
            place: Place::Station {
                station: StationRef {
                    source: SourceId::NDBC,
                    id: id.into(),
                    name: id.into(),
                    kind,
                    latitude: 40.7,
                    longitude: -74.0,
                    bin: None,
                    reference_id: None,
                },
                distance_m: Some(2_500.0),
                relation: Relation::Nearby,
            },
            measurements: ms.iter().copied().collect(),
            window: Window {
                from: UnixTime(0),
                to: UnixTime(4_000_000_000),
            },
            datum: None,
        }
    }

    fn ctx() -> FetchContext {
        FetchContext::new(UnixTime(1_794_146_400), "underbite/test")
    }

    fn plan(id: &str, ms: &[Measurement]) -> Result<Vec<Request>, SourceError> {
        Ndbc.plan(
            &target(id, StationKind::Buoy, ms),
            &Requirements::default(),
            &ctx(),
        )
    }

    fn run(request: &Request, fixture: &str, status: u16) -> Result<Vec<Fragment>, SourceError> {
        let bytes = fixture_bytes("ndbc", fixture);
        let mut out = Fragments::new();
        Ndbc.parse(request, &Response::new(status, &bytes), &ctx(), &mut out)?;
        Ok(out.into_parts().0)
    }

    #[test]
    fn plans_one_request_per_file() {
        use Measurement as M;
        let requests = plan("44025", &[M::Waves, M::WaterTemperature, M::ObservedWind]).unwrap();
        let urls: Vec<_> = requests.iter().map(|r| r.url.as_str()).collect();
        assert_eq!(
            urls,
            [
                "https://www.ndbc.noaa.gov/data/realtime2/44025.txt",
                "https://www.ndbc.noaa.gov/data/realtime2/44025.spec"
            ]
        );
        assert!(requests[0].required && !requests[1].required);
        assert_eq!(requests[0].cache.max_age, Some(600));
        assert_eq!(
            requests[0].target.measurements,
            BTreeSet::from([M::Waves, M::WaterTemperature, M::ObservedWind])
        );
        assert_eq!(requests[1].target.measurements, BTreeSet::from([M::Waves]));
        assert!(requests.iter().all(|r| r.headers.is_empty()));

        let requests = plan("44069", &[M::Salinity, M::OceanCurrent]).unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests[0].url.ends_with("44069.ocean") && requests[0].required);
        assert!(requests[1].url.ends_with("44069.adcp") && !requests[1].required);
    }

    #[test]
    fn land_station_files_use_upper_case_ids() {
        let requests = plan("batn6", &[Measurement::ObservedPressure]).unwrap();
        assert_eq!(
            requests[0].url,
            "https://www.ndbc.noaa.gov/data/realtime2/BATN6.txt"
        );
        let requests = plan("4403585", &[Measurement::Waves]).unwrap();
        assert!(requests[0].url.ends_with("/4403585.txt"));
    }

    #[test]
    fn plan_refuses_what_it_cannot_serve() {
        use Measurement as M;
        assert!(matches!(
            plan("44025", &[M::Tide, M::Conductivity]),
            Err(SourceError::Unsupported(_))
        ));
        assert!(matches!(
            plan("../44025", &[M::Waves]),
            Err(SourceError::Unsupported(_))
        ));
        let requirements = Requirements {
            measurements: BTreeSet::from([M::ObservedWind]),
            ..Requirements::default()
        };
        let t = target("44025", StationKind::Buoy, &[M::Waves]);
        assert!(Ndbc.plan(&t, &requirements, &ctx()).is_err());
        let mut point = t;
        point.place = Place::Point(ctx().round(crate::request::Coordinates {
            latitude: 40.7,
            longitude: -74.0,
        }));
        assert!(Ndbc.plan(&point, &Requirements::default(), &ctx()).is_err());
    }

    #[test]
    fn parses_a_planned_request_into_fragments() {
        use Measurement as M;
        let requests = plan("44065", &[M::ObservedWind, M::ObservedPressure]).unwrap();
        let fragments = run(&requests[0], "ndbc-44065-head300.txt", 200).unwrap();
        let ms: Vec<_> = fragments.iter().map(|f| f.measurement).collect();
        assert_eq!(ms, [M::ObservedWind, M::ObservedPressure]);
    }

    #[test]
    fn a_station_without_the_wanted_columns_has_no_data() {
        let requests = plan("44025", &[Measurement::ObservedWind]).unwrap();
        let err = run(&requests[0], "ndbc-44025.txt", 200).unwrap_err();
        assert!(matches!(err, SourceError::NoData(_)), "{err:?}");
    }

    #[test]
    fn a_missing_ocean_file_is_no_data_not_an_error_to_show() {
        let requests = plan("44025", &[Measurement::Salinity]).unwrap();
        let err = run(&requests[0], "ndbc-44025.ocean-404.html", 404).unwrap_err();
        assert!(matches!(err, SourceError::NoData(_)), "{err:?}");
        let txt = plan("44025", &[Measurement::Waves]).unwrap();
        let err = run(&txt[0], "ndbc-44025.ocean-404.html", 404).unwrap_err();
        assert!(matches!(err, SourceError::NotFound(_)), "{err:?}");
    }

    #[test]
    fn coverage_comes_from_columns_that_hold_values() {
        use Measurement as M;
        let t = target("44025", StationKind::Buoy, &[]);
        let request = Ndbc.coverage_request(&t, Resource::NdbcStandard).unwrap();
        assert!(!request.required);
        let fragments = run(&request, "ndbc-44025.txt", 200).unwrap();
        let mut seen = std::collections::BTreeMap::new();
        for f in &fragments {
            let FragmentValue::Coverage {
                station_id,
                coverage,
            } = &f.value
            else {
                panic!("not coverage");
            };
            assert_eq!(station_id, "44025");
            seen.insert(f.measurement, *coverage);
        }
        assert_eq!(
            seen.keys().copied().collect::<Vec<_>>(),
            [M::WaterTemperature, M::Waves]
        );
        assert_eq!(
            seen[&M::Waves].last_value_at,
            Some(UnixTime(1_794_146_400 - 20 * 60))
        );
    }

    #[test]
    fn coverage_of_a_file_without_values_is_empty() {
        let t = target("batn6", StationKind::Cman, &[]);
        let request = Ndbc.coverage_request(&t, Resource::NdbcStandard).unwrap();
        let fragments = run(&request, "ndbc-BATN6-head300.txt", 200).unwrap();
        let ms: Vec<_> = fragments.iter().map(|f| f.measurement).collect();
        assert!(!ms.contains(&Measurement::ObservedWind));
        assert!(ms.contains(&Measurement::ObservedPressure));
    }

    #[test]
    fn the_station_list_is_not_parsed_through_the_source() {
        let t = target("44025", StationKind::Buoy, &[]);
        let request = stations::request(t);
        assert_eq!(request.url, stations::URL);
        let err = run(&request, "ndbc-activestations.xml", 200).unwrap_err();
        assert!(matches!(err, SourceError::Unsupported(_)));
    }

    #[test]
    fn credit_names_ndbc() {
        assert!(
            Ndbc.attribution()
                .credit
                .contains("NOAA National Data Buoy Center")
        );
        assert_eq!(Ndbc.id(), SourceId::NDBC);
        assert!(Ndbc.budget().is_none());
    }
}
