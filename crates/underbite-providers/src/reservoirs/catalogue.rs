//! The bundled table of reservoir projects: where each is and which series it publishes.
//!
//! CWMS has no search by position (its catalogue takes an office, not a box) and every project
//! carries many versions of each series, most of them forecasts or archives. So the app ships the
//! projects whose observed series were checked against the live service, with the exact series
//! names, and finds the nearest one on the device. Nothing about the spot leaves the device to
//! find a project.
//!
//! TVA publishes lake levels and release schedules only on a web page that no script can read, so
//! its reservoirs are link-only entries: the card shows the operator's page and no values.

use underbite_core::map::GeoPoint;
use underbite_core::map::tiles::haversine_m;
use underbite_core::model::ExternalLink;

/// A project whose pool, release or inflow comes from the CWMS Data API.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Project {
    /// The Corps district office that owns the data (`NAB`).
    pub office: &'static str,
    /// The CWMS location name, exactly as the service writes it (`Raystown`).
    pub location: &'static str,
    /// The name shown to the user.
    pub name: &'static str,
    /// Two-letter state.
    pub state: &'static str,
    /// Degrees north, from the CWMS location.
    pub latitude: f64,
    /// Degrees east, from the CWMS location.
    pub longitude: f64,
    /// Pool elevation series name, if a current observed one exists.
    pub pool: Option<&'static str>,
    /// Release series name, if the district publishes one.
    pub outflow: Option<&'static str>,
    /// Inflow series name, if checked.
    pub inflow: Option<&'static str>,
    /// The district's name for the link ("USACE Baltimore District").
    pub district: &'static str,
    /// The district's web site.
    pub district_url: &'static str,
}

impl Project {
    /// The CWMS station id of this project, `office/location`.
    pub fn station_id(&self) -> String {
        format!("{}/{}", self.office, self.location)
    }

    /// The operator's page, for what the data does not carry.
    pub fn links(&self) -> Vec<ExternalLink> {
        vec![ExternalLink {
            label: format!("{}: {}", self.district, self.name),
            url: self.district_url.to_string(),
        }]
    }
}

/// A reservoir whose operator publishes no data a script can read: a link and nothing else.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LinkProject {
    /// The name shown to the user.
    pub name: &'static str,
    /// Two-letter state of the dam.
    pub state: &'static str,
    /// Approximate dam position, degrees north; for choosing the nearest project only.
    pub latitude: f64,
    /// Approximate dam position, degrees east.
    pub longitude: f64,
    /// The operator ("Tennessee Valley Authority").
    pub operator: &'static str,
    /// The operator's page of lake levels and release schedules.
    pub url: &'static str,
}

impl LinkProject {
    /// The operator's page as a link.
    pub fn link(&self) -> ExternalLink {
        ExternalLink {
            label: format!("{}: {}", self.operator, self.name),
            url: self.url.to_string(),
        }
    }
}

/// TVA's page of lake levels and release schedules.
pub const TVA_LAKE_LEVELS: &str = "https://www.tva.com/environment/lake-levels";

const NAB: (&str, &str) = (
    "USACE Baltimore District",
    "https://www.nab.usace.army.mil/",
);
const NAE: (&str, &str) = (
    "USACE New England District",
    "https://www.nae.usace.army.mil/",
);
const NAP: (&str, &str) = (
    "USACE Philadelphia District",
    "https://www.nap.usace.army.mil/",
);
const MVP: (&str, &str) = ("USACE St. Paul District", "https://www.mvp.usace.army.mil/");
const MVS: (&str, &str) = (
    "USACE St. Louis District",
    "https://www.mvs.usace.army.mil/",
);
const SAM: (&str, &str) = ("USACE Mobile District", "https://www.sam.usace.army.mil/");
const SAS: (&str, &str) = ("USACE Savannah District", "https://www.sas.usace.army.mil/");
const LRL: (&str, &str) = (
    "USACE Louisville District",
    "https://www.lrl.usace.army.mil/",
);
const SPK: (&str, &str) = (
    "USACE Sacramento District",
    "https://www.spk.usace.army.mil/",
);

/// The projects whose series were checked against the live service (see
/// `docs/providers/cwms.md`). Outflow is absent where the district publishes none (Philadelphia),
/// only a forecast (Black Rock Dam) or where it was not checked; the pool is absent at Hartwell,
/// whose observed elevation series ended.
pub static PROJECTS: &[Project] = &[
    Project {
        office: "NAB",
        location: "Raystown",
        name: "Raystown Lake",
        state: "PA",
        latitude: 40.4271,
        longitude: -78.0014,
        pool: Some("Raystown.Elev.Inst.15Minutes.0.Best-NAB"),
        outflow: Some("Raystown.Flow-Out.Inst.15Minutes.0.Best-NAB-Computed-PY3"),
        inflow: Some("Raystown.Flow-In.Inst.1Hour.0.Best-NAB-Computed"),
        district: NAB.0,
        district_url: NAB.1,
    },
    Project {
        office: "NAB",
        location: "Cowanesque",
        name: "Cowanesque Lake",
        state: "PA",
        latitude: 41.9836,
        longitude: -77.1509,
        pool: Some("Cowanesque.Elev.Inst.15Minutes.0.Best-NAB"),
        outflow: Some("Cowanesque.Flow-Out.Inst.15Minutes.0.Best-NAB-Computed"),
        inflow: None,
        district: NAB.0,
        district_url: NAB.1,
    },
    Project {
        office: "NAE",
        location: "BRD",
        name: "Black Rock Dam",
        state: "CT",
        latitude: 41.6575,
        longitude: -73.1056,
        pool: Some("BRD.Elev.Inst.15Minutes.0.DCP-rev"),
        outflow: None,
        inflow: None,
        district: NAE.0,
        district_url: NAE.1,
    },
    Project {
        office: "NAP",
        location: "Beltzville",
        name: "Beltzville Lake",
        state: "PA",
        latitude: 40.8478,
        longitude: -75.6383,
        pool: Some("Beltzville.Elev.Inst.15Minutes.0.Rev-DCP"),
        outflow: None,
        inflow: Some("Beltzville.Flow-In.Inst.1Hour.0.Computed"),
        district: NAP.0,
        district_url: NAP.1,
    },
    Project {
        office: "MVP",
        location: "Baldhill_Dam",
        name: "Lake Ashtabula",
        state: "ND",
        latitude: 47.0362,
        longitude: -98.0815,
        pool: Some("Baldhill_Dam.Elev.Inst.15Minutes.0.rev-NGVD29"),
        outflow: Some("Baldhill_Dam.Flow-Out.Inst.15Minutes.0.rev"),
        inflow: None,
        district: MVP.0,
        district_url: MVP.1,
    },
    Project {
        office: "MVP",
        location: "MissHW_Leech",
        name: "Leech Lake",
        state: "MN",
        latitude: 47.2475,
        longitude: -94.2218,
        pool: Some("MissHW_Leech.Elev.Inst.15Minutes.0.rev-NGVD29"),
        outflow: Some("MissHW_Leech.Flow-Out.Inst.15Minutes.0.rev"),
        inflow: None,
        district: MVP.0,
        district_url: MVP.1,
    },
    Project {
        office: "MVS",
        location: "Carlyle Lk-Kaskaskia",
        name: "Carlyle Lake",
        state: "IL",
        latitude: 38.618,
        longitude: -89.3511,
        pool: Some("Carlyle Lk-Kaskaskia.Elev.Inst.30Minutes.0.lrgsShef-rev"),
        outflow: Some("Carlyle Lk-Kaskaskia.Flow-Out.Ave.~1Day.1Day.lakerep-rev"),
        inflow: None,
        district: MVS.0,
        district_url: MVS.1,
    },
    Project {
        office: "MVS",
        location: "Mark Twain Lk-Salt",
        name: "Mark Twain Lake",
        state: "MO",
        latitude: 39.5249,
        longitude: -91.6443,
        pool: Some("Mark Twain Lk-Salt.Elev.Inst.30Minutes.0.lrgsShef-rev"),
        outflow: None,
        inflow: None,
        district: MVS.0,
        district_url: MVS.1,
    },
    Project {
        office: "SAM",
        location: "Allatoona",
        name: "Lake Allatoona",
        state: "GA",
        latitude: 34.1628,
        longitude: -84.7278,
        pool: Some("Allatoona.Elev.Inst.1Hour.0.Raw-USGS"),
        outflow: Some("Allatoona.Flow-Out.Inst.15Minutes.0.Raw-USGS"),
        inflow: None,
        district: SAM.0,
        district_url: SAM.1,
    },
    Project {
        office: "SAM",
        location: "Buford",
        name: "Lake Lanier",
        state: "GA",
        latitude: 34.16,
        longitude: -84.0733,
        pool: Some("Buford.Elev.Inst.15Minutes.0.Raw-USGS"),
        outflow: Some("Buford.Flow-Out.Inst.15Minutes.0.Raw-UR"),
        inflow: None,
        district: SAM.0,
        district_url: SAM.1,
    },
    Project {
        office: "SAM",
        location: "Carters",
        name: "Carters Lake",
        state: "GA",
        latitude: 34.614,
        longitude: -84.671,
        pool: Some("Carters.Elev.Inst.15Minutes.0.Raw-USGS"),
        outflow: None,
        inflow: None,
        district: SAM.0,
        district_url: SAM.1,
    },
    Project {
        office: "SAS",
        location: "Hartwell",
        name: "Hartwell Lake",
        state: "GA",
        latitude: 34.3575,
        longitude: -82.8214,
        pool: None,
        outflow: Some("Hartwell.Flow-Out.Ave.1Hour.1Hour.Raw-SHEF_SAS"),
        inflow: None,
        district: SAS.0,
        district_url: SAS.1,
    },
    Project {
        office: "LRL",
        location: "Barren-Lake",
        name: "Barren River Lake",
        state: "KY",
        latitude: 36.8967,
        longitude: -86.125,
        pool: Some("Barren-Lake.Elev.Inst.15Minutes.0.USGS-rev"),
        outflow: None,
        inflow: None,
        district: LRL.0,
        district_url: LRL.1,
    },
    Project {
        office: "SPK",
        location: "Boca",
        name: "Boca Reservoir",
        state: "CA",
        latitude: 39.383,
        longitude: -120.1,
        pool: Some("Boca.Elev.Inst.1Hour.0.Calc-usgs"),
        outflow: None,
        inflow: None,
        district: SPK.0,
        district_url: SPK.1,
    },
];

macro_rules! tva {
    ($name:expr, $state:expr, $lat:expr, $lon:expr) => {
        LinkProject {
            name: $name,
            state: $state,
            latitude: $lat,
            longitude: $lon,
            operator: "Tennessee Valley Authority",
            url: TVA_LAKE_LEVELS,
        }
    };
}

/// TVA reservoirs, as links. Positions are approximate and not yet checked against TVA's own
/// data; they only pick the nearest link.
pub static LINK_PROJECTS: &[LinkProject] = &[
    tva!("Norris Lake", "TN", 36.2247, -84.0922),
    tva!("Douglas Lake", "TN", 36.0153, -83.5372),
    tva!("Fontana Lake", "NC", 35.4517, -83.8061),
    tva!("Watts Bar Lake", "TN", 35.6297, -84.7814),
    tva!("Chickamauga Lake", "TN", 35.1042, -85.2306),
    tva!("Guntersville Lake", "AL", 34.4147, -86.3922),
    tva!("Wheeler Lake", "AL", 34.8014, -87.3794),
    tva!("Pickwick Lake", "TN", 35.0636, -88.2514),
    tva!("Kentucky Lake", "KY", 37.0097, -88.2625),
];

/// The project at `office` / `location`.
pub fn find(office: &str, location: &str) -> Option<&'static Project> {
    PROJECTS
        .iter()
        .find(|p| p.office == office && p.location == location)
}

/// A project near a spot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Near {
    /// Values from CWMS.
    Cwms(&'static Project),
    /// A link only.
    Link(&'static LinkProject),
}

impl Near {
    /// The name shown to the user.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Cwms(p) => p.name,
            Self::Link(p) => p.name,
        }
    }
}

/// A project and its distance from the spot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Nearby {
    /// The project.
    pub project: Near,
    /// From the spot to the project, metres.
    pub distance_m: f32,
}

/// Projects within `max_m` metres of (`latitude`, `longitude`), nearest first; ties keep the
/// table's order, CWMS projects before link-only ones.
pub fn nearby(latitude: f64, longitude: f64, max_m: f32) -> Vec<Nearby> {
    let at = GeoPoint {
        lat: latitude,
        lon: longitude,
    };
    let distance = |lat, lon| haversine_m(at, GeoPoint { lat, lon }) as f32;
    let mut out: Vec<Nearby> = PROJECTS
        .iter()
        .map(|p| Nearby {
            project: Near::Cwms(p),
            distance_m: distance(p.latitude, p.longitude),
        })
        .chain(LINK_PROJECTS.iter().map(|p| Nearby {
            project: Near::Link(p),
            distance_m: distance(p.latitude, p.longitude),
        }))
        .filter(|n| n.distance_m <= max_m)
        .collect();
    out.sort_by(|a, b| a.distance_m.total_cmp(&b.distance_m));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Battery, the New York test place.
    const BATTERY: (f64, f64) = (40.7033, -74.0170);
    /// Croton Point, on the lower Hudson.
    const CROTON: (f64, f64) = (41.2064, -73.8856);

    #[test]
    fn black_rock_dam_is_the_nearest_pool_to_new_york() {
        let near = nearby(CROTON.0, CROTON.1, 150_000.0);
        let first = near.first().expect("a project within 150 km");
        assert_eq!(first.project.name(), "Black Rock Dam");
        assert!(
            (75_000.0..90_000.0).contains(&first.distance_m),
            "{first:?}"
        );
    }

    #[test]
    fn nothing_is_within_a_few_kilometres_of_the_battery() {
        assert_eq!(nearby(BATTERY.0, BATTERY.1, 30_000.0), Vec::new());
        let wide = nearby(BATTERY.0, BATTERY.1, 120_000.0);
        assert_eq!(wide, Vec::new());
    }

    #[test]
    fn nearby_sorts_by_distance_and_mixes_link_only_projects() {
        let near = nearby(36.0, -84.0, 100_000.0);
        assert!(near.iter().any(|n| matches!(n.project, Near::Link(_))));
        assert!(near.windows(2).all(|w| w[0].distance_m <= w[1].distance_m));
    }

    #[test]
    fn every_project_names_a_series_for_its_location() {
        for p in PROJECTS {
            for name in [p.pool, p.outflow, p.inflow].into_iter().flatten() {
                assert!(
                    name.starts_with(&format!("{}.", p.location)),
                    "{} / {name}",
                    p.location
                );
            }
            assert!(p.pool.is_some() || p.outflow.is_some(), "{}", p.location);
            assert!((-90.0..=90.0).contains(&p.latitude));
            assert!((-180.0..=180.0).contains(&p.longitude));
            assert_eq!(p.office.len(), 3);
        }
    }

    #[test]
    fn pool_series_are_instantaneous_elevations() {
        for p in PROJECTS {
            if let Some(pool) = p.pool {
                assert!(pool.contains(".Elev.Inst."), "{pool}");
            }
            if let Some(inflow) = p.inflow {
                assert!(inflow.contains(".Flow-In.Inst."), "{inflow}");
            }
            if let Some(out) = p.outflow {
                assert!(out.contains(".Flow-Out."), "{out}");
            }
            // Forecast and archive versions never stand for an observation.
            for name in [p.pool, p.outflow, p.inflow].into_iter().flatten() {
                for bad in ["Fcst", "Forecast", "ARCHIVE", "HISTORIAN", "Manual"] {
                    assert!(!name.contains(bad), "{name}");
                }
            }
        }
    }

    #[test]
    fn no_outflow_where_the_district_publishes_none() {
        let beltzville = find("NAP", "Beltzville").expect("Beltzville is listed");
        assert!(beltzville.outflow.is_none() && beltzville.inflow.is_some());
        assert!(
            PROJECTS
                .iter()
                .filter(|p| p.office == "NAP" || p.office == "NAN")
                .all(|p| p.outflow.is_none())
        );
        assert!(find("NAN", "Anything").is_none());
    }

    #[test]
    fn links_name_the_operator_and_the_project() {
        let raystown = find("NAB", "Raystown").expect("Raystown is listed");
        assert_eq!(raystown.station_id(), "NAB/Raystown");
        assert_eq!(
            raystown.links(),
            vec![ExternalLink {
                label: "USACE Baltimore District: Raystown Lake".into(),
                url: "https://www.nab.usace.army.mil/".into(),
            }]
        );
        let norris = LINK_PROJECTS[0].link();
        assert_eq!(norris.label, "Tennessee Valley Authority: Norris Lake");
        assert_eq!(norris.url, TVA_LAKE_LEVELS);
    }

    #[test]
    fn locations_are_unique() {
        let mut keys: Vec<_> = PROJECTS.iter().map(|p| (p.office, p.location)).collect();
        keys.sort_unstable();
        let n = keys.len();
        keys.dedup();
        assert_eq!(keys.len(), n);
    }
}
