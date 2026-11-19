//! The sources behind every value, with their credits.

use schemars::JsonSchema;
use serde::Serialize;
use whether_core::model::ProviderId;

use crate::map::catalogue;
use crate::model::{Measurement, SourceError, SourceErrorKind, SpotSnapshot};
use crate::provenance::{Provenance, Quality, Relation, SourceId, ValueKind};
use crate::series::Series;
use crate::time::UnixTime;
use crate::view::cards::flow::card_state;
use crate::view::format::Formatter;
use crate::view::strings::{Key, SafetyKey, Strings};
use crate::view::{
    CardChrome, CardId, CardState, Compact, CreditVm, Notice, NoticeKind, Quantity, ViewInput,
};

const PROVISIONAL_STATEMENT: &str = "https://waterdata.usgs.gov/provisional-data-statement/";
const OPEN_METEO: &str = "https://open-meteo.com/";
const OPEN_METEO_LICENCE: &str = "https://open-meteo.com/en/licence";

// ---------------------------------------------------------------------------------------------
// What each source asks to be shown with its data
// ---------------------------------------------------------------------------------------------

/// What a source or a bundled dataset asks for: the credit line, the licence it states, the
/// conditions that go with it, and the pages to read them on. The words are safety texts: the
/// same in every voice.
///
/// A credit that depends on the day's data is not kept here: the GEBCO release year, the
/// Copernicus DOI of each product and the credit lines NWPS names in its responses are read
/// where the data are (the catalogue's products, the service's capabilities, the snapshot's
/// attribution). A `credit` of none means the credit is built from those; see [`credit_text`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceTerms {
    /// The id the settings and the snapshot use.
    pub id: &'static str,
    /// The credit line; none when it is built at run time.
    pub credit: Option<SafetyKey>,
    /// The licence as the source states it.
    pub licence: SafetyKey,
    /// What the licence or the source asks beyond the credit.
    pub conditions: Option<SafetyKey>,
    /// The page that states the terms or the licence; empty when there is none.
    pub terms_url: &'static str,
    /// The source's home page; empty when there is none.
    pub site_url: &'static str,
}

const fn terms(
    id: &'static str,
    credit: Option<SafetyKey>,
    licence: SafetyKey,
    conditions: Option<SafetyKey>,
    terms_url: &'static str,
    site_url: &'static str,
) -> SourceTerms {
    SourceTerms {
        id,
        credit,
        licence,
        conditions,
        terms_url,
        site_url,
    }
}

const WEATHER_GOV_TERMS: &str = "https://www.weather.gov/disclaimer";
const COOPS_TERMS: &str = "https://tidesandcurrents.noaa.gov/disclaimers.html";
const ERDDAP_TERMS: &str = "https://coastwatch.pfeg.noaa.gov/erddap/legal.html";
const OPEN_NY_TERMS: &str = "https://data.ny.gov/dataset/OPEN-NY-Terms-Of-Use/77gx-ii52";
const PADUS_PAGE: &str =
    "https://www.usgs.gov/programs/gap-analysis-project/science/pad-us-data-overview";
const USGS_TERMS: &str =
    "https://www.usgs.gov/information-policies-and-instructions/copyrights-and-credits";
const COPERNICUS_TERMS: &str =
    "https://marine.copernicus.eu/user-corner/service-commitments-and-licence";
const GLERL_DATASET: &str = "https://apps.glerl.noaa.gov/erddap/griddap/GLSEA_ACSPO_GCS.html";
const HAB_PAGE: &str =
    "https://coastalscience.noaa.gov/science-areas/habs/hab-forecasts/gulf-coast/";

/// Every source the settings or a snapshot can name, with its credit, licence and terms.
pub const SOURCE_TERMS: [SourceTerms; 31] = [
    terms(
        "usgs",
        Some(SafetyKey::AttrUsgsCredit),
        SafetyKey::LicencePublicDomain,
        Some(SafetyKey::AttrUsgsConditions),
        USGS_TERMS,
        "https://waterdata.usgs.gov/",
    ),
    terms(
        "coops",
        Some(SafetyKey::AttrCoopsCredit),
        SafetyKey::LicencePublicDomain,
        Some(SafetyKey::AttrCoopsConditions),
        COOPS_TERMS,
        "https://tidesandcurrents.noaa.gov/",
    ),
    terms(
        "coops-harmonics",
        Some(SafetyKey::AttrCoopsHarmonicsCredit),
        SafetyKey::LicencePublicDomain,
        Some(SafetyKey::AttrCoopsHarmonicsConditions),
        COOPS_TERMS,
        "https://tidesandcurrents.noaa.gov/",
    ),
    terms(
        "ndbc",
        Some(SafetyKey::AttrNdbcCredit),
        SafetyKey::LicencePublicDomain,
        Some(SafetyKey::AttrNdbcConditions),
        WEATHER_GOV_TERMS,
        "https://www.ndbc.noaa.gov/",
    ),
    terms(
        "nwps",
        Some(SafetyKey::AttrNwpsCredit),
        SafetyKey::LicencePublicDomain,
        Some(SafetyKey::AttrNwpsConditions),
        WEATHER_GOV_TERMS,
        "https://water.noaa.gov/",
    ),
    terms(
        "nws",
        Some(SafetyKey::AttrNwsCredit),
        SafetyKey::LicencePublicDomain,
        Some(SafetyKey::AttrNwsConditions),
        WEATHER_GOV_TERMS,
        "https://www.weather.gov/",
    ),
    terms(
        "open-meteo-marine",
        Some(SafetyKey::AttrOpenMeteoMarineCredit),
        SafetyKey::LicenceCcBy4,
        Some(SafetyKey::AttrOpenMeteoMarineConditions),
        OPEN_METEO_LICENCE,
        OPEN_METEO,
    ),
    terms(
        "usgs-3dhp",
        Some(SafetyKey::AttrUsgs3dhpCredit),
        SafetyKey::LicencePublicDomain,
        None,
        "https://hydro.nationalmap.gov/arcgis/rest/services/3DHP_all/MapServer",
        "https://hydro.nationalmap.gov/",
    ),
    terms(
        "glerl",
        Some(SafetyKey::AttrGlerlCredit),
        SafetyKey::LicenceFreeNotLegal,
        Some(SafetyKey::AttrGlerlConditions),
        GLERL_DATASET,
        "https://www.glerl.noaa.gov/",
    ),
    terms(
        "cwms",
        Some(SafetyKey::AttrCwmsCredit),
        SafetyKey::LicencePublicDomain,
        Some(SafetyKey::AttrCwmsConditions),
        "https://cwms-data-api.readthedocs.io/latest/",
        "https://water.usace.army.mil/",
    ),
    terms(
        "rise",
        Some(SafetyKey::AttrRiseCredit),
        SafetyKey::LicenceUsGovernment,
        None,
        "https://www.usbr.gov/main/disclaimer.html",
        "https://data.usbr.gov/",
    ),
    terms(
        "cyan",
        Some(SafetyKey::AttrCyanCredit),
        SafetyKey::LicenceNoneStated,
        Some(SafetyKey::AttrCyanConditions),
        "https://www.epa.gov/water-research/cyanobacteria-assessment-network-application-cyan-app",
        "https://www.epa.gov/water-research/cyanobacteria-assessment-network-cyan",
    ),
    terms(
        "noaa-hab",
        Some(SafetyKey::AttrNoaaHabCredit),
        SafetyKey::LicenceUsGovernment,
        Some(SafetyKey::AttrNoaaHabConditions),
        HAB_PAGE,
        "https://coastalscience.noaa.gov/",
    ),
    terms(
        "ny-dec-stocking",
        Some(SafetyKey::AttrNyDecStockingCredit),
        SafetyKey::LicenceOpenNy,
        Some(SafetyKey::AttrNyDecStockingConditions),
        OPEN_NY_TERMS,
        "https://dec.ny.gov/",
    ),
    terms(
        "overpass",
        Some(SafetyKey::AttrOverpassCredit),
        SafetyKey::LicenceOdbl,
        Some(SafetyKey::AttrOverpassConditions),
        "https://www.openstreetmap.org/copyright",
        "https://www.openstreetmap.org/",
    ),
    terms(
        "padus",
        Some(SafetyKey::AttrPadusCredit),
        SafetyKey::LicencePublicDomain,
        Some(SafetyKey::AttrPadusConditions),
        PADUS_PAGE,
        PADUS_PAGE,
    ),
    terms(
        "gbif",
        Some(SafetyKey::AttrGbifCredit),
        SafetyKey::LicenceGbifRecords,
        Some(SafetyKey::AttrGbifConditions),
        "https://www.gbif.org/terms/data-user",
        "https://www.gbif.org/",
    ),
    terms(
        "inaturalist",
        Some(SafetyKey::AttrInaturalistCredit),
        SafetyKey::LicenceInaturalistRecords,
        Some(SafetyKey::AttrInaturalistConditions),
        "https://www.inaturalist.org/terms",
        "https://www.inaturalist.org/",
    ),
    terms(
        "usgs-nas",
        Some(SafetyKey::AttrUsgsNasCredit),
        SafetyKey::LicencePublicDomain,
        Some(SafetyKey::AttrUsgsNasConditions),
        "https://nas.er.usgs.gov/api/documentation.aspx",
        "https://nas.er.usgs.gov/",
    ),
    terms(
        "dfo-iwls",
        Some(SafetyKey::AttrDfoIwlsCredit),
        SafetyKey::LicenceChs,
        Some(SafetyKey::AttrDfoIwlsConditions),
        "https://www.tides.gc.ca/en/licence-agreement",
        "https://www.tides.gc.ca/",
    ),
    terms(
        "eccc-hydrometric",
        Some(SafetyKey::AttrEcccHydrometricCredit),
        SafetyKey::LicenceEccc,
        Some(SafetyKey::AttrEcccHydrometricConditions),
        "https://eccc-msc.github.io/open-data/licence/readme_en/",
        "https://wateroffice.ec.gc.ca/",
    ),
    terms(
        "mur-sst",
        Some(SafetyKey::AttrMurSstCredit),
        SafetyKey::LicenceJplPodaac,
        Some(SafetyKey::AttrMurSstConditions),
        ERDDAP_TERMS,
        "https://coastwatch.pfeg.noaa.gov/erddap/",
    ),
    terms(
        "hf-radar",
        Some(SafetyKey::AttrHfRadarCredit),
        SafetyKey::LicenceFreeNotLegal,
        Some(SafetyKey::AttrHfRadarConditions),
        ERDDAP_TERMS,
        "https://coastwatch.pfeg.noaa.gov/erddap/",
    ),
    terms(
        "copernicus",
        None,
        SafetyKey::LicenceCopernicus,
        Some(SafetyKey::AttrCopernicusConditions),
        COPERNICUS_TERMS,
        "https://marine.copernicus.eu/",
    ),
    terms(
        "open-meteo-storm",
        Some(SafetyKey::AttrOpenMeteoStormCredit),
        SafetyKey::LicenceCcBy4,
        Some(SafetyKey::AttrOpenMeteoStormConditions),
        OPEN_METEO_LICENCE,
        OPEN_METEO,
    ),
    terms(
        "nhc",
        Some(SafetyKey::AttrNhcCredit),
        SafetyKey::LicencePublicDomain,
        Some(SafetyKey::AttrNhcConditions),
        "https://www.nhc.noaa.gov/disclaimer.shtml",
        "https://www.nhc.noaa.gov/",
    ),
    terms(
        "openweather",
        None,
        SafetyKey::LicenceOwnOpenWeather,
        Some(SafetyKey::AttrOpenweatherConditions),
        "https://openweathermap.org/",
        "https://openweathermap.org/",
    ),
    terms(
        "open-meteo",
        None,
        SafetyKey::LicenceCcBy4,
        Some(SafetyKey::AttrOpenMeteoConditions),
        OPEN_METEO_LICENCE,
        OPEN_METEO,
    ),
    terms(
        "met-norway",
        None,
        SafetyKey::LicenceCcBy4,
        None,
        "https://api.met.no/doc/TermsOfService",
        "https://api.met.no/",
    ),
    terms(
        "acis",
        None,
        SafetyKey::LicenceUsGovernment,
        None,
        "https://www.rcc-acis.org/",
        "https://www.rcc-acis.org/",
    ),
    terms(
        "pirate-weather",
        None,
        SafetyKey::LicenceOwnPirateWeather,
        Some(SafetyKey::AttrPirateWeatherConditions),
        "https://pirateweather.net/",
        "https://pirateweather.net/",
    ),
];

/// The terms of a source, by id.
pub fn source_terms(id: &str) -> Option<&'static SourceTerms> {
    SOURCE_TERMS.iter().find(|t| t.id == id)
}

/// The weather provider a configuration id names.
fn weather_provider(id: &str) -> Option<ProviderId> {
    Some(match id {
        "openweather" => ProviderId::OpenWeather,
        "open-meteo" => ProviderId::OpenMeteo,
        "met-norway" => ProviderId::MetNorway,
        "acis" => ProviderId::Acis,
        "pirate-weather" => ProviderId::PirateWeather,
        _ => return None,
    })
}

/// The Copernicus credit of every product the map layers draw: "Generated using E.U. Copernicus
/// Marine Service Information" and the DOIs, read from the catalogue's products.
pub fn copernicus_credit(strings: Strings) -> String {
    let mut dois: Vec<&str> = Vec::new();
    for layer in &catalogue::LAYERS {
        if let Some(spec) = layer.copernicus()
            && !dois.contains(&spec.doi)
        {
            dois.push(spec.doi);
        }
    }
    let links: Vec<String> = dois
        .iter()
        .map(|d| format!("https://doi.org/{d}"))
        .collect();
    format!(
        "{}{}",
        strings.safety(SafetyKey::AttrCopernicusCreditPrefix),
        links.join(", ")
    )
}

/// The credit line of an entry: its own, or for those that depend on the day's data or on the
/// weather provider, the one read from there.
pub fn credit_text(t: &SourceTerms, strings: Strings) -> String {
    if let Some(key) = t.credit {
        strings.safety(key).to_owned()
    } else if let Some(p) = weather_provider(t.id) {
        p.attribution().to_owned()
    } else if t.id == "copernicus" {
        copernicus_credit(strings)
    } else {
        t.id.to_owned()
    }
}

/// What an entry asks beyond its credit; for Open-Meteo's marine data it includes the model
/// providers.
pub fn conditions_text(t: &SourceTerms, strings: Strings) -> Option<String> {
    let own = t.conditions.map(|k| strings.safety(k).to_owned());
    if t.id == "open-meteo-marine" {
        let models = strings.safety(SafetyKey::AttrOpenMeteoMarineModels);
        return Some(match own {
            Some(c) => format!("{c} {models}."),
            None => format!("{models}."),
        });
    }
    own
}

/// The licence of an entry as its source states it.
pub fn licence_text(t: &SourceTerms, strings: Strings) -> String {
    strings.safety(t.licence).to_owned()
}

/// The GEBCO grid's credit on the attribution page. The release year and the DOI are the
/// service's to state and change with each release, so the page says where they are read from;
/// the map layer shows them as the service's capabilities give them.
pub fn gebco_page_credit(strings: Strings) -> String {
    strings.safety(SafetyKey::AttrGebcoPage).to_owned()
}

/// A dataset the app carries or links that is not a source of values: its name and terms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dataset {
    pub name: SafetyKey,
    pub terms: SourceTerms,
}

/// The bundled datasets and the regulation links.
pub const DATASETS: [Dataset; 2] = [
    Dataset {
        name: SafetyKey::AttrFishbaseName,
        terms: terms(
            "fishbase",
            Some(SafetyKey::AttrFishbaseCredit),
            SafetyKey::LicenceCcByNc4,
            Some(SafetyKey::AttrFishbaseConditions),
            "https://www.fishbase.se/summary/citation.php",
            "https://www.fishbase.org/",
        ),
    },
    Dataset {
        name: SafetyKey::AttrRegulationsName,
        terms: terms(
            "regulations",
            Some(SafetyKey::AttrRegulationsCredit),
            SafetyKey::LicenceLinksOnly,
            None,
            "",
            "",
        ),
    },
];

/// The page that states the terms of a map layer's data, for the layers that have a source to
/// state them; none for a layer drawn from the user's own data and for a base map.
pub fn layer_terms_url(layer_id: &str) -> Option<&'static str> {
    Some(match layer_id {
        "noaa-chart"
        | "enc-depths"
        | "enc-seabed"
        | "enc-special-areas"
        | "enc-structure-points" => {
            "https://nauticalcharts.noaa.gov/data/gis-data-and-services.html"
        }
        "gebco-relief" => "https://www.gebco.net/data-products/gridded-bathymetry-data",
        "depth-shading" | "depth-contours" => {
            "https://www.ncei.noaa.gov/products/coastal-relief-model"
        }
        "sst" | "chlorophyll" | "clarity" | "hf-radar-currents" | "sst-breaks"
        | "chlorophyll-breaks" => ERDDAP_TERMS,
        "sst-imagery"
        | "chlorophyll-imagery"
        | "true-color-viirs-n20"
        | "true-color-viirs-n21"
        | "true-color-modis-terra"
        | "true-color-modis-aqua" => "https://nasa-gibs.github.io/gibs-api-docs/",
        "great-lakes-sst" => GLERL_DATASET,
        "radar" => "https://mesonet.agron.iastate.edu/disclaimer.php",
        "tide-stations" | "current-stations" => COOPS_TERMS,
        "usgs-gauges" => USGS_TERMS,
        "buoys" | "nwps-gauges" => WEATHER_GOV_TERMS,
        "lake-contours-mn" => "https://www.dnr.state.mn.us/wmas/kml/license.html",
        "artificial-reefs-ny" | "stocking-points" => OPEN_NY_TERMS,
        "public-land" => PADUS_PAGE,
        "access-points" => "https://www.openstreetmap.org/copyright",
        "species-records" => "https://www.gbif.org/terms/data-user",
        "hab-beaches" => HAB_PAGE,
        "storms" => OPEN_METEO_LICENCE,
        "copernicus-currents" | "copernicus-sst" | "copernicus-sea-level" => COPERNICUS_TERMS,
        _ => return None,
    })
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct SourcesCard {
    pub chrome: CardChrome,
    pub compact: Compact,
    pub full: SourcesFull,
}

/// A station that supplied a value.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct StationUseVm {
    /// What it supplies: "Water temperature".
    pub measurement: String,
    pub station: String,
    pub station_id: String,
    pub source: String,
    /// "0.4 mi"
    pub distance: Option<String>,
    /// "6 min ago"
    pub age: String,
    /// "Same water body", "Upstream"
    pub relation: String,
    /// "Observed", "Predicted", "Modelled"
    pub kind: String,
    pub state: CardState,
}

/// A source that did not deliver.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct SourceErrorVm {
    pub source: String,
    pub measurement: Option<String>,
    /// "Rate limited", "Timed out"
    pub kind: String,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct SourcesFull {
    pub credits: Vec<CreditVm>,
    pub stations: Vec<StationUseVm>,
    pub errors: Vec<SourceErrorVm>,
}

/// A value the snapshot holds, with where it came from and when its newest sample is.
struct Used<'a> {
    measurement: Measurement,
    provenance: &'a Provenance,
    /// Whether the value is a series, whose age is that of a sample.
    series: bool,
    /// The newest sample at or before now; none for a series with only samples ahead.
    latest: Option<UnixTime>,
}

/// What the snapshot holds, collected as of `now`.
struct Collected<'a> {
    items: Vec<Used<'a>>,
    now: UnixTime,
}

impl<'a> Collected<'a> {
    fn push(&mut self, used: Used<'a>) {
        self.items.push(used);
    }

    fn series<T>(&mut self, measurement: Measurement, series: Option<&'a Series<T>>) {
        if let Some(s) = series {
            self.items.push(Used {
                measurement,
                provenance: s.provenance(),
                series: true,
                latest: s
                    .samples()
                    .iter()
                    .rev()
                    .find(|x| x.time <= self.now)
                    .map(|x| x.time),
            });
        }
    }
}

/// Every provenance in the snapshot, in a fixed order.
#[allow(clippy::too_many_lines)]
fn used(snapshot: &SpotSnapshot, now: UnixTime) -> Vec<Used<'_>> {
    let w = &snapshot.water;
    let m = &snapshot.marine;
    let mut out = Collected {
        items: Vec::new(),
        now,
    };
    out.series(Measurement::WaterTemperature, w.temperature_c.as_ref());
    out.series(Measurement::Discharge, w.discharge_m3s.as_ref());
    out.series(
        Measurement::GageHeight,
        w.gage_height.as_ref().map(|h| &h.series),
    );
    out.series(Measurement::LakeLevel, w.level.as_ref().map(|h| &h.series));
    out.series(Measurement::Turbidity, w.turbidity.as_ref());
    out.series(
        Measurement::DissolvedOxygen,
        w.dissolved_oxygen_mg_l.as_ref(),
    );
    out.series(Measurement::Conductivity, w.conductivity_us_cm.as_ref());
    out.series(Measurement::Salinity, w.salinity_psu.as_ref());
    out.series(Measurement::Ph, w.ph.as_ref());
    out.series(Measurement::Chlorophyll, w.chlorophyll_ug_l.as_ref());
    out.series(Measurement::Clarity, w.clarity_kd490_per_m.as_ref());
    out.series(
        Measurement::Cyanobacteria,
        w.cyanobacteria_cells_ml.as_ref(),
    );
    out.series(Measurement::FlowForecast, w.discharge_forecast_m3s.as_ref());
    out.series(
        Measurement::FlowForecast,
        w.stage_forecast.as_ref().map(|h| &h.series),
    );
    for (measurement, percentile) in [
        (Measurement::Discharge, &w.discharge_percentile_for_date),
        (Measurement::LakeLevel, &w.level_percentile_for_date),
    ] {
        if let Some(p) = percentile.as_ref().and_then(|p| p.provenance.as_ref()) {
            out.push(Used {
                measurement,
                provenance: p,
                series: false,
                latest: None,
            });
        }
    }
    if let Some(t) = &m.tide {
        out.push(Used {
            measurement: Measurement::Tide,
            provenance: &t.provenance,
            series: false,
            latest: None,
        });
    }
    if let Some(c) = &m.tidal_current {
        out.push(Used {
            measurement: Measurement::TidalCurrent,
            provenance: &c.provenance,
            series: false,
            latest: None,
        });
    }
    out.series(Measurement::OceanCurrent, m.ocean_current.as_ref());
    out.series(Measurement::Waves, m.waves.as_ref());
    out.series(Measurement::SeaSurfaceTemp, m.sea_surface_temp_c.as_ref());
    out.series(Measurement::WavesForecast, m.waves_forecast.as_ref());
    out.series(
        Measurement::SeaSurfaceTempForecast,
        m.sea_surface_temp_forecast_c.as_ref(),
    );
    out.series(Measurement::ObservedWind, m.observed_wind.as_ref());
    out.series(
        Measurement::ObservedPressure,
        m.observed_pressure_hpa.as_ref(),
    );
    out.items
}

fn measurement_label(f: &Formatter, m: Measurement) -> &'static str {
    f.strings.measurement(m)
}

fn relation_label(f: &Formatter, r: Relation) -> &'static str {
    f.strings.relation(r)
}

fn kind_label(f: &Formatter, k: ValueKind) -> &'static str {
    f.strings.get(match k {
        ValueKind::Observed => Key::Observed,
        ValueKind::Predicted => Key::Predicted,
        ValueKind::Modelled => Key::ModelledCap,
        ValueKind::Satellite => Key::RelSatellite,
        ValueKind::Computed => Key::Computed,
    })
}

fn error_kind_label(f: &Formatter, k: SourceErrorKind) -> &'static str {
    f.strings.get(match k {
        SourceErrorKind::RateLimited => Key::ErrRateLimited,
        SourceErrorKind::NotFound => Key::ErrNotFound,
        SourceErrorKind::NoData => Key::ErrNoData,
        SourceErrorKind::Stale => Key::ErrStale,
        SourceErrorKind::Parse => Key::ErrParse,
        SourceErrorKind::Transport => Key::ErrTransport,
        SourceErrorKind::Other => Key::ErrOther,
    })
}

/// The credit a source is owed: the snapshot's own wording and link when it has them, else the
/// credit line the source asks for, with its licence as the source states it and its terms page.
fn credit_for(snapshot: &SpotSnapshot, id: &SourceId, strings: Strings) -> CreditVm {
    let entry = source_terms(id.as_str());
    let found = snapshot.attribution.iter().find(|a| a.source == *id);
    let name = strings.source_name(id);
    CreditVm {
        text: match (found, entry) {
            (Some(a), _) => a.credit.clone(),
            (None, Some(t)) => t
                .credit
                .map_or_else(|| name.clone(), |k| strings.safety(k).to_owned()),
            (None, None) => name.clone(),
        },
        link: found
            .and_then(|a| a.url.clone())
            .or_else(|| entry.map(|t| t.terms_url.to_owned()))
            .filter(|l| !l.is_empty()),
        licence: entry.map(|t| licence_text(t, strings)),
        source: name,
    }
}

fn add(out: &mut Vec<CreditVm>, credit: CreditVm) {
    if !out.contains(&credit) {
        out.push(credit);
    }
}

/// Every credit owed: the snapshot's attributions and each value's source, with Open-Meteo's
/// model providers and the provisional-data statement where they apply.
fn credits(snapshot: &SpotSnapshot, used: &[Used<'_>], strings: Strings) -> Vec<CreditVm> {
    let mut ids: Vec<SourceId> = snapshot
        .attribution
        .iter()
        .map(|a| a.source.clone())
        .collect();
    ids.extend(used.iter().map(|u| u.provenance.source.clone()));
    if snapshot.marine.marine_forecast.is_some() || !snapshot.marine.marine_alerts.is_empty() {
        ids.push(SourceId::NWS);
    }
    let mut out = Vec::new();
    let mut seen = Vec::new();
    for id in ids {
        if seen.contains(&id) {
            continue;
        }
        add(&mut out, credit_for(snapshot, &id, strings));
        seen.push(id);
    }

    let mut models = Vec::new();
    if let Some(w) = &snapshot.weather {
        add(
            &mut out,
            CreditVm {
                source: w.provider.display_name().to_owned(),
                text: w.provider.attribution().to_owned(),
                link: (w.provider == ProviderId::OpenMeteo).then(|| OPEN_METEO.to_owned()),
                licence: (w.provider == ProviderId::OpenMeteo)
                    .then(|| strings.safety(SafetyKey::LicenceCcBy4).to_owned()),
            },
        );
        for p in &w.supplemented_by {
            add(
                &mut out,
                CreditVm {
                    source: p.display_name().to_owned(),
                    text: p.short_credit().to_owned(),
                    link: None,
                    licence: None,
                },
            );
        }
        if w.provider == ProviderId::OpenMeteo && !w.models.is_empty() {
            let names: Vec<&str> = w
                .models
                .iter()
                .map(|m| whether_core::models::short_name(m))
                .collect();
            models.push(strings.fmt(Key::WeatherModels, &[("models", &names.join(", "))]));
        }
    }
    if seen.contains(&SourceId::OPEN_METEO_MARINE) {
        models.push(strings.safety(SafetyKey::CreditMarineModels).to_owned());
    }
    if !models.is_empty() {
        add(
            &mut out,
            CreditVm {
                source: strings.safety(SafetyKey::SourceModelProviders).to_owned(),
                text: models.join(". "),
                link: Some(OPEN_METEO_LICENCE.to_owned()),
                licence: None,
            },
        );
    }

    if used.iter().any(|u| {
        u.provenance.source == SourceId::USGS && u.provenance.quality == Quality::Provisional
    }) {
        add(
            &mut out,
            CreditVm {
                source: strings.source_name(&SourceId::USGS),
                text: strings.get(Key::ProvisionalStatement).to_owned(),
                link: Some(PROVISIONAL_STATEMENT.to_owned()),
                licence: None,
            },
        );
    }
    out
}

fn station_uses(input: &ViewInput, f: &Formatter, used: &[Used<'_>]) -> Vec<StationUseVm> {
    let mut out: Vec<StationUseVm> = Vec::new();
    for u in used {
        let p = u.provenance;
        let Some(station) = &p.station else { continue };
        let measurement = measurement_label(f, u.measurement);
        // Predictions and computed curves have no age to speak of.
        let aged = matches!(p.kind, ValueKind::Observed | ValueKind::Satellite);
        // A series is as old as its newest sample so far; one with only samples ahead has no age.
        let latest = if u.series {
            u.latest
        } else {
            Some(p.observed_at)
        };
        let aged = aged && latest.is_some();
        let state = card_state(input, f, u.measurement, p, latest.filter(|_| aged));
        // One row per station and kind of value, naming everything it supplies.
        let kind = kind_label(f, p.kind);
        if let Some(row) = out.iter_mut().find(|s| {
            s.station_id == station.id
                && s.source == f.strings.source_name(&p.source)
                && s.kind == kind
        }) {
            if !row.measurement.split(", ").any(|m| m == measurement) {
                row.measurement.push_str(", ");
                row.measurement.push_str(measurement);
            }
            if row.state == CardState::Fresh {
                row.state = state;
            }
            continue;
        }
        out.push(StationUseVm {
            measurement: measurement.to_owned(),
            station: station.name.clone(),
            station_id: station.id.clone(),
            source: f.strings.source_name(&p.source),
            distance: p.distance_m.map(|m| f.distance(Some(m)).full()),
            age: if let Some(t) = latest.filter(|_| aged) {
                f.age(input.now.0 - t.0)
            } else {
                f.strings.get(Key::Missing).to_owned()
            },
            relation: relation_label(f, p.relation).to_owned(),
            kind: kind.to_owned(),
            state,
        });
    }
    out
}

fn error_vm(f: &Formatter, e: &SourceError, count: usize) -> SourceErrorVm {
    // The source's own plain message: "USGS: rate limited, used NWPS stage".
    let mut message = e.message.clone();
    if count > 1 {
        let more = f
            .strings
            .fmt(Key::MoreCount, &[("n", &(count - 1).to_string())]);
        message = format!("{message} ({more})");
    }
    SourceErrorVm {
        source: f.strings.source_name(&e.source),
        measurement: e.measurement.map(|m| measurement_label(f, m).to_owned()),
        kind: error_kind_label(f, e.kind).to_owned(),
        message,
    }
}

/// Each source's problem with a measurement once, in the order they first occurred, with the
/// first message and how many more there were (one per station tried, say).
fn error_vms(f: &Formatter, errors: &[SourceError]) -> Vec<SourceErrorVm> {
    let mut groups: Vec<(&SourceError, usize)> = Vec::new();
    for e in errors {
        let same = |g: &&mut (&SourceError, usize)| {
            g.0.source == e.source && g.0.measurement == e.measurement && g.0.kind == e.kind
        };
        match groups.iter_mut().find(same) {
            Some(g) => g.1 += 1,
            None => groups.push((e, 1)),
        }
    }
    groups
        .into_iter()
        .map(|(e, count)| error_vm(f, e, count))
        .collect()
}

fn plural(f: &Formatter, n: usize, one: Key, many: Key) -> String {
    f.strings
        .fmt(if n == 1 { one } else { many }, &[("n", &n.to_string())])
}

/// Every credit, station and problem behind the page.
pub fn build(input: &ViewInput) -> Option<SourcesCard> {
    let snapshot = &input.snapshot;
    let f = Formatter::for_input(input);
    let used = used(snapshot, input.now);
    let credits = credits(snapshot, &used, f.strings);
    let stations = station_uses(input, &f, &used);
    let errors = error_vms(&f, &snapshot.errors);
    if credits.is_empty() && stations.is_empty() && errors.is_empty() {
        return None;
    }

    let m = &snapshot.marine;
    let mut notices: Vec<Notice> = Vec::new();
    if m.tide.is_some() || m.tidal_current.is_some() || m.ocean_current.is_some() {
        notices.push(f.notice(NoticeKind::NotForNavigation));
    }
    if used.iter().any(|u| {
        u.provenance.source == SourceId::USGS && u.provenance.quality == Quality::Provisional
    }) {
        notices.push(f.notice(NoticeKind::Provisional));
    }

    let title = f.strings.card_title(CardId::Sources).to_owned();
    let names: Vec<&str> = credits.iter().map(|c| c.source.as_str()).collect();
    let mut accessibility = format!("{title}: {}.", names.join(", "));
    for e in &errors {
        accessibility.push(' ');
        accessibility.push_str(&e.message);
    }
    let lines = [
        (!stations.is_empty())
            .then(|| plural(&f, stations.len(), Key::StationOne, Key::StationMany)),
        (!errors.is_empty()).then(|| plural(&f, errors.len(), Key::ProblemOne, Key::ProblemMany)),
    ]
    .into_iter()
    .flatten()
    .collect();
    Some(SourcesCard {
        compact: Compact {
            headline: Quantity {
                value: Some(credits.len() as f32),
                text: credits.len().to_string(),
                unit: f.strings.get(Key::UnitSources).to_owned(),
            },
            lines,
            spark: Vec::new(),
        },
        chrome: CardChrome {
            id: CardId::Sources,
            title,
            footnote: None,
            // The credits are the card's content, listed once in the full form.
            credits: Vec::new(),
            state: CardState::Fresh,
            notices,
            accessibility,
        },
        full: SourcesFull {
            credits,
            stations,
            errors,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::SourceErrorKind;
    use crate::view::Voice;
    use crate::view::tests::sample_input;

    fn plain() -> Strings {
        Strings::new(Voice::Plain)
    }

    fn names(card: &SourcesCard) -> Vec<&str> {
        card.full
            .credits
            .iter()
            .map(|c| c.source.as_str())
            .collect()
    }

    #[test]
    fn every_source_used_is_credited() {
        let input = sample_input();
        let card = build(&input).unwrap();
        let n = names(&card);
        for expected in [
            "NOAA CO-OPS",
            "NOAA NDBC",
            "USGS",
            "NOAA NWPS",
            "Open-Meteo Marine",
            "National Weather Service",
            "Open-Meteo",
        ] {
            assert!(n.contains(&expected), "{expected} missing from {n:?}");
        }
        // The snapshot's own wording wins over the default.
        let coops = card
            .full
            .credits
            .iter()
            .find(|c| c.source == "NOAA CO-OPS")
            .unwrap();
        assert_eq!(coops.text, "NOAA CO-OPS");
        assert_eq!(
            coops.link.as_deref(),
            Some("https://tidesandcurrents.noaa.gov")
        );
        // Open-Meteo's model providers are named, with the licence.
        let models = card
            .full
            .credits
            .iter()
            .find(|c| c.source == "Open-Meteo model providers")
            .unwrap();
        assert!(models.text.contains("ECMWF (WAM)") && models.text.contains("NOAA NCEP"));
        let marine = card
            .full
            .credits
            .iter()
            .find(|c| c.source == "Open-Meteo Marine")
            .unwrap();
        assert_eq!(marine.licence.as_deref(), Some("CC BY 4.0"));
        // Provisional USGS values link the statement.
        assert!(
            card.full
                .credits
                .iter()
                .any(|c| c.source == "USGS" && c.link.as_deref() == Some(PROVISIONAL_STATEMENT))
        );
        let kinds: Vec<NoticeKind> = card.chrome.notices.iter().map(|n| n.kind).collect();
        assert_eq!(
            kinds,
            [NoticeKind::NotForNavigation, NoticeKind::Provisional]
        );
    }

    /// The ids of every `SourceId` constant, read from where they are declared so that a new
    /// one without a credit fails here.
    fn declared_source_ids() -> Vec<String> {
        include_str!("../../provenance.rs")
            .lines()
            .filter_map(|l| l.split("SourceId::from_static(\"").nth(1))
            .filter_map(|rest| rest.split('"').next())
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn every_source_id_has_an_attribution_entry() {
        let ids = declared_source_ids();
        assert!(ids.len() >= 21, "the declarations were not read: {ids:?}");
        for id in &ids {
            let t = source_terms(id).unwrap_or_else(|| panic!("{id} has no attribution entry"));
            assert!(!credit_text(t, plain()).is_empty(), "{id}");
        }
        // The constants and the list agree.
        for id in [
            SourceId::USGS,
            SourceId::COOPS,
            SourceId::COOPS_HARMONICS,
            SourceId::NDBC,
            SourceId::NWPS,
            SourceId::OPEN_METEO_MARINE,
            SourceId::NWS,
            SourceId::USGS_3DHP,
            SourceId::GLERL,
            SourceId::CWMS,
            SourceId::RISE,
            SourceId::CYAN,
            SourceId::NOAA_HAB,
            SourceId::NY_DEC_STOCKING,
            SourceId::OVERPASS,
            SourceId::PADUS,
            SourceId::GBIF,
            SourceId::INATURALIST,
            SourceId::USGS_NAS,
            SourceId::DFO_IWLS,
            SourceId::ECCC_HYDROMETRIC,
        ] {
            assert!(ids.iter().any(|i| i == id.as_str()), "{id}");
        }
    }

    #[test]
    fn every_entry_states_a_licence_and_links_its_terms() {
        let mut seen = Vec::new();
        let all = SOURCE_TERMS.iter().chain(DATASETS.iter().map(|d| &d.terms));
        for t in all {
            assert!(!seen.contains(&t.id), "{} is listed twice", t.id);
            seen.push(t.id);
            assert!(!licence_text(t, plain()).is_empty(), "{}", t.id);
            if t.id != "regulations" {
                assert!(t.terms_url.starts_with("https://"), "{}", t.id);
                assert!(t.site_url.starts_with("https://"), "{}", t.id);
            }
        }
    }

    #[test]
    fn a_credit_that_depends_on_the_day_is_read_not_kept() {
        // Copernicus: the DOI of each product the layers draw.
        let t = source_terms("copernicus").unwrap();
        assert!(t.credit.is_none());
        let credit = credit_text(t, plain());
        for doi in ["moi-00016", "moi-00165"] {
            assert!(
                credit.starts_with("Generated using E.U. Copernicus Marine Service Information;")
                    && credit.contains(&format!("https://doi.org/10.48670/{doi}")),
                "{credit}"
            );
        }
        // The weather providers: whether's own wording.
        let open_meteo = credit_text(source_terms("open-meteo").unwrap(), plain());
        assert_eq!(open_meteo, ProviderId::OpenMeteo.attribution());
        // NWPS: no credit lines of the data are kept; the snapshot's win.
        let nwps = conditions_text(source_terms("nwps").unwrap(), plain()).unwrap();
        assert!(!nwps.contains("20"), "{nwps}");
        // The GEBCO grid year appears in no entry.
        for t in &SOURCE_TERMS {
            let text = format!(
                "{} {}",
                credit_text(t, plain()),
                conditions_text(t, plain()).unwrap_or_default()
            );
            assert!(!text.contains("GEBCO"), "{}", t.id);
        }
        assert!(!gebco_page_credit(plain()).contains("2026"));
    }

    #[test]
    fn open_meteo_marine_names_its_model_providers_with_their_dois() {
        let text = conditions_text(source_terms("open-meteo-marine").unwrap(), plain()).unwrap();
        for expected in [
            "10.48670/moi-00017",
            "10.48670/moi-00016",
            "10.24381/cds.adbb2d47",
            "ECMWF (WAM)",
            "NOAA NCEP",
            "DWD",
        ] {
            assert!(text.contains(expected), "{expected} missing from {text}");
        }
    }

    #[test]
    fn every_layer_with_a_source_links_its_terms() {
        // Base maps and the user's own layers have no source's terms to link; the wind,
        // pressure and wave layers have no source yet; Indiana states no terms for its
        // contours.
        let without = [
            "apple-standard",
            "apple-satellite",
            "apple-hybrid",
            "wind",
            "pressure",
            "waves",
            "spots",
            "catches",
            "tracks",
            "catch-heat",
            "lake-contours-in",
        ];
        for id in catalogue::ids() {
            let linked = layer_terms_url(id).is_some();
            assert_eq!(linked, !without.contains(&id), "{id}");
        }
    }

    #[test]
    fn a_credit_without_a_snapshot_attribution_has_its_licence_and_terms() {
        let mut snapshot = sample_input().snapshot;
        snapshot.attribution.clear();
        for id in declared_source_ids() {
            let c = credit_for(
                &snapshot,
                &SourceId::new(id.clone()),
                Strings::new(Voice::Plain),
            );
            assert!(c.licence.is_some(), "{id}");
            assert!(c.link.is_some(), "{id}");
            assert_ne!(c.text, id, "{id}");
        }
    }

    #[test]
    fn stations_carry_distance_relation_and_age() {
        let input = sample_input();
        let card = build(&input).unwrap();
        let temp = card
            .full
            .stations
            .iter()
            .find(|s| s.measurement.starts_with("Water temperature"))
            .unwrap();
        assert_eq!(temp.station, "The Battery");
        assert_eq!(temp.station_id, "8518750");
        assert_eq!(temp.distance.as_deref(), Some("0.4 mi"));
        assert_eq!(temp.relation, "Same water body");
        assert_eq!(temp.age, "just now");
        assert_eq!(temp.kind, "Observed");
        let tide = card
            .full
            .stations
            .iter()
            .find(|s| s.measurement.contains("Tide"))
            .unwrap();
        assert_eq!((tide.kind.as_str(), tide.age.as_str()), ("Predicted", "—"));
        // One row per station, source and kind of value.
        let mut keys: Vec<_> = card
            .full
            .stations
            .iter()
            .map(|s| (&s.source, &s.station_id, &s.kind))
            .collect();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), card.full.stations.len());
    }

    #[test]
    fn errors_are_plain_messages() {
        let mut input = sample_input();
        input.snapshot.errors.push(SourceError {
            source: SourceId::COOPS,
            measurement: Some(Measurement::Tide),
            kind: SourceErrorKind::Transport,
            message: "coops: HTTP 503 Service Unavailable, used coops-harmonics".into(),
        });
        let card = build(&input).unwrap();
        assert_eq!(card.full.errors.len(), 2);
        assert_eq!(
            card.full.errors[0].message,
            "USGS: rate limited, used NWPS stage"
        );
        assert_eq!(card.full.errors[0].kind, "Rate limited");
        assert_eq!(card.full.errors[1].measurement.as_deref(), Some("Tide"));
        assert!(card.chrome.accessibility.contains("used coops-harmonics"));
        assert_eq!(
            card.compact.lines,
            [
                format!("{} stations", card.full.stations.len()),
                "2 problems".to_owned()
            ]
        );
    }

    #[test]
    fn a_problem_repeated_at_several_stations_is_listed_once() {
        let mut input = sample_input();
        input.snapshot.errors.clear();
        for station in ["8518750", "8516945", "8531680"] {
            input.snapshot.errors.push(SourceError {
                source: SourceId::COOPS,
                measurement: Some(Measurement::ObservedWind),
                kind: SourceErrorKind::NoData,
                message: format!("coops {station}: no data"),
            });
        }
        input.snapshot.errors.push(SourceError {
            source: SourceId::COOPS,
            measurement: Some(Measurement::WaterTemperature),
            kind: SourceErrorKind::NoData,
            message: "coops 8518750: no data".into(),
        });
        let card = build(&input).unwrap();
        let shown: Vec<(&str, &str)> = card
            .full
            .errors
            .iter()
            .map(|e| (e.measurement.as_deref().unwrap(), e.message.as_str()))
            .collect();
        assert_eq!(
            shown,
            [
                ("Wind", "coops 8518750: no data (2 more)"),
                ("Water temperature", "coops 8518750: no data"),
            ]
        );
        assert!(card.compact.lines.contains(&"2 problems".to_owned()));
        // The snapshot keeps every error.
        assert_eq!(input.snapshot.errors.len(), 4);
    }

    #[test]
    fn nothing_known_is_no_card() {
        let mut input = sample_input();
        input.snapshot.water = crate::model::WaterConditions::default();
        input.snapshot.marine = crate::model::MarineConditions::default();
        input.snapshot.weather = None;
        input.snapshot.attribution.clear();
        input.snapshot.errors.clear();
        assert!(build(&input).is_none());
    }

    #[test]
    fn snapshots_in_both_voices() {
        let mut input = sample_input();
        input.snapshot.errors.push(SourceError {
            source: SourceId::COOPS,
            measurement: Some(Measurement::Tide),
            kind: SourceErrorKind::Transport,
            message: "coops: HTTP 503 Service Unavailable, used coops-harmonics".into(),
        });
        insta::assert_json_snapshot!("sources_luna", build(&input).unwrap());
        input.voice = Voice::Plain;
        insta::assert_json_snapshot!("sources_plain", build(&input).unwrap());
    }

    #[test]
    fn a_series_is_aged_from_its_newest_sample_so_far() {
        let mut input = sample_input();
        let now = input.now;
        let temp = input.snapshot.water.temperature_c.take().unwrap();
        let prov = temp.provenance().clone();
        let at = |dt: i64| crate::series::Sample::new(UnixTime(now.0 + dt), 12.0);
        input.snapshot.water.temperature_c =
            Some(Series::new(prov.clone(), vec![at(-1_800), at(7_200)]));
        let card = build(&input).unwrap();
        let age = |card: &SourcesCard| {
            card.full
                .stations
                .iter()
                .find(|s| s.measurement.starts_with("Water temperature"))
                .unwrap()
                .age
                .clone()
        };
        assert_eq!(age(&card), "30 min ago");
        input.snapshot.water.temperature_c = Some(Series::new(prov, vec![at(3_600), at(7_200)]));
        assert_eq!(age(&build(&input).unwrap()), "—");
    }
}
