//! The sources behind every value, with their credits.

use schemars::JsonSchema;
use serde::Serialize;
use whether_core::model::ProviderId;

use crate::model::{Measurement, SourceError, SourceErrorKind, SpotSnapshot};
use crate::provenance::{Provenance, Quality, Relation, SourceId, ValueKind};
use crate::series::Series;
use crate::time::UnixTime;
use crate::view::cards::flow::card_state;
use crate::view::format::Formatter;
use crate::view::strings::{Key, Strings, source_name};
use crate::view::{
    CardChrome, CardId, CardState, Compact, CreditVm, Notice, NoticeKind, Quantity, ViewInput,
};

const PROVISIONAL_STATEMENT: &str = "https://waterdata.usgs.gov/provisional-data-statement/";
const OPEN_METEO: &str = "https://open-meteo.com/";
const OPEN_METEO_LICENCE: &str = "https://open-meteo.com/en/licence";
/// Licences as the sources state them.
const PUBLIC_DOMAIN: &str = "Public domain";
const CC_BY_4: &str = "CC BY 4.0";
/// What Open-Meteo's model providers are called as a credit.
const MODEL_PROVIDERS: &str = "Open-Meteo model providers";
/// The credit lines of the sources, as they ask to be credited.
const CREDIT_USGS: &str = "U.S. Geological Survey";
const CREDIT_COOPS: &str = "Tide, water-level and current data: NOAA/NOS CO-OPS";
const CREDIT_HARMONICS: &str = "Computed on this device from NOAA CO-OPS harmonic constants";
const CREDIT_NDBC: &str = "Observations: NOAA National Data Buoy Center";
const CREDIT_NWPS: &str = "River forecasts: NOAA National Water Prediction Service";
const CREDIT_NWS: &str = "Marine forecasts and alerts: NOAA National Weather Service";
const CREDIT_OPEN_METEO: &str = "Weather data by Open-Meteo.com";
/// The model providers Open-Meteo's marine data comes from, as its terms ask them credited.
const MARINE_MODELS: &str = "Marine models: Copernicus Marine Service (MeteoFrance MFWAM, SMOC), \
    ECMWF (WAM), NOAA NCEP (GFS Wave), DWD (EWAM, GWAM), Copernicus Climate Change Service \
    (ERA5-Ocean)";

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
    f.strings.get(match m {
        Measurement::WaterTemperature => Key::MeasWaterTemperature,
        Measurement::Discharge => Key::MeasDischarge,
        Measurement::GageHeight => Key::MeasGageHeight,
        Measurement::LakeLevel => Key::MeasLakeLevel,
        Measurement::Turbidity => Key::MeasTurbidity,
        Measurement::DissolvedOxygen => Key::MeasDissolvedOxygen,
        Measurement::Conductivity => Key::MeasConductivity,
        Measurement::Salinity => Key::MeasSalinity,
        Measurement::Ph => Key::MeasPh,
        Measurement::Chlorophyll => Key::MeasChlorophyll,
        Measurement::Clarity => Key::MeasClarity,
        Measurement::Cyanobacteria => Key::MeasCyanobacteria,
        Measurement::Tide => Key::MeasTide,
        Measurement::TidalCurrent => Key::MeasTidalCurrent,
        Measurement::OceanCurrent => Key::MeasOceanCurrent,
        Measurement::Waves => Key::MeasWaves,
        Measurement::WavesForecast => Key::MeasWavesForecast,
        Measurement::SeaSurfaceTemp => Key::MeasSeaSurfaceTemp,
        Measurement::SeaSurfaceTempForecast => Key::MeasSeaSurfaceTempForecast,
        Measurement::MarineForecast => Key::MeasMarineForecast,
        Measurement::MarineAlerts => Key::MeasMarineAlerts,
        Measurement::FloodCategories => Key::MeasFloodCategories,
        Measurement::FlowForecast => Key::MeasFlowForecast,
        Measurement::ObservedWind => Key::MeasWind,
        Measurement::ObservedPressure => Key::MeasPressure,
    })
}

fn relation_label(f: &Formatter, r: Relation) -> &'static str {
    f.strings.get(match r {
        Relation::SameWaterBody => Key::RelSameWaterBody,
        Relation::Upstream => Key::RelUpstream,
        Relation::Downstream => Key::RelDownstream,
        Relation::Nearby => Key::RelNearby,
        Relation::Gridded => Key::RelModelGrid,
        Relation::Satellite => Key::RelSatellite,
    })
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

/// The credit a source is owed: the snapshot's own wording and link when it has them.
fn credit_for(snapshot: &SpotSnapshot, id: &SourceId) -> CreditVm {
    let (text, link, licence) = match id.as_str() {
        "usgs" => (
            CREDIT_USGS,
            "https://www.usgs.gov/information-policies-and-instructions/copyrights-and-credits",
            Some(PUBLIC_DOMAIN),
        ),
        "coops" => (
            CREDIT_COOPS,
            "https://tidesandcurrents.noaa.gov/",
            Some(PUBLIC_DOMAIN),
        ),
        "coops-harmonics" => (
            CREDIT_HARMONICS,
            "https://tidesandcurrents.noaa.gov/",
            Some(PUBLIC_DOMAIN),
        ),
        "ndbc" => (
            CREDIT_NDBC,
            "https://www.ndbc.noaa.gov/",
            Some(PUBLIC_DOMAIN),
        ),
        "nwps" => (CREDIT_NWPS, "https://water.noaa.gov/", Some(PUBLIC_DOMAIN)),
        "nws" => (CREDIT_NWS, "https://www.weather.gov/", Some(PUBLIC_DOMAIN)),
        "open-meteo-marine" => (CREDIT_OPEN_METEO, OPEN_METEO, Some(CC_BY_4)),
        _ => ("", "", None),
    };
    let found = snapshot.attribution.iter().find(|a| a.source == *id);
    let name = source_name(id);
    CreditVm {
        text: match found {
            Some(a) => a.credit.clone(),
            None if text.is_empty() => name.clone(),
            None => text.to_owned(),
        },
        link: found
            .and_then(|a| a.url.clone())
            .or_else(|| (!link.is_empty()).then(|| link.to_owned())),
        licence: licence.map(str::to_owned),
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
        add(&mut out, credit_for(snapshot, &id));
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
                licence: (w.provider == ProviderId::OpenMeteo).then(|| CC_BY_4.to_owned()),
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
        models.push(MARINE_MODELS.to_owned());
    }
    if !models.is_empty() {
        add(
            &mut out,
            CreditVm {
                source: MODEL_PROVIDERS.to_owned(),
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
                source: source_name(&SourceId::USGS),
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
            s.station_id == station.id && s.source == source_name(&p.source) && s.kind == kind
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
            source: source_name(&p.source),
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
        source: source_name(&e.source),
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
