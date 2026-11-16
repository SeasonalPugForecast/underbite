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
use crate::view::strings::{Key, source_name};
use crate::view::{
    CardChrome, CardId, CardState, Compact, CreditVm, Notice, NoticeKind, Quantity, ViewInput,
};

const PROVISIONAL_STATEMENT: &str = "https://waterdata.usgs.gov/provisional-data-statement/";
const OPEN_METEO: &str = "https://open-meteo.com/";
const OPEN_METEO_LICENCE: &str = "https://open-meteo.com/en/licence";
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
    latest: Option<UnixTime>,
}

fn push_series<'a, T>(
    out: &mut Vec<Used<'a>>,
    measurement: Measurement,
    series: Option<&'a Series<T>>,
) {
    if let Some(s) = series {
        out.push(Used {
            measurement,
            provenance: s.provenance(),
            latest: s.latest().map(|x| x.time),
        });
    }
}

/// Every provenance in the snapshot, in a fixed order.
#[allow(clippy::too_many_lines)]
fn used(snapshot: &SpotSnapshot) -> Vec<Used<'_>> {
    let w = &snapshot.water;
    let m = &snapshot.marine;
    let mut out = Vec::new();
    push_series(
        &mut out,
        Measurement::WaterTemperature,
        w.temperature_c.as_ref(),
    );
    push_series(&mut out, Measurement::Discharge, w.discharge_m3s.as_ref());
    push_series(
        &mut out,
        Measurement::GageHeight,
        w.gage_height.as_ref().map(|h| &h.series),
    );
    push_series(
        &mut out,
        Measurement::LakeLevel,
        w.level.as_ref().map(|h| &h.series),
    );
    push_series(&mut out, Measurement::Turbidity, w.turbidity.as_ref());
    push_series(
        &mut out,
        Measurement::DissolvedOxygen,
        w.dissolved_oxygen_mg_l.as_ref(),
    );
    push_series(
        &mut out,
        Measurement::Conductivity,
        w.conductivity_us_cm.as_ref(),
    );
    push_series(&mut out, Measurement::Salinity, w.salinity_psu.as_ref());
    push_series(&mut out, Measurement::Ph, w.ph.as_ref());
    push_series(
        &mut out,
        Measurement::Chlorophyll,
        w.chlorophyll_ug_l.as_ref(),
    );
    push_series(
        &mut out,
        Measurement::Clarity,
        w.clarity_kd490_per_m.as_ref(),
    );
    push_series(
        &mut out,
        Measurement::Cyanobacteria,
        w.cyanobacteria_cells_ml.as_ref(),
    );
    push_series(
        &mut out,
        Measurement::FlowForecast,
        w.discharge_forecast_m3s.as_ref(),
    );
    push_series(
        &mut out,
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
                latest: None,
            });
        }
    }
    if let Some(t) = &m.tide {
        out.push(Used {
            measurement: Measurement::Tide,
            provenance: &t.provenance,
            latest: None,
        });
    }
    if let Some(c) = &m.tidal_current {
        out.push(Used {
            measurement: Measurement::TidalCurrent,
            provenance: &c.provenance,
            latest: None,
        });
    }
    push_series(
        &mut out,
        Measurement::OceanCurrent,
        m.ocean_current.as_ref(),
    );
    push_series(&mut out, Measurement::Waves, m.waves.as_ref());
    push_series(
        &mut out,
        Measurement::SeaSurfaceTemp,
        m.sea_surface_temp_c.as_ref(),
    );
    push_series(
        &mut out,
        Measurement::WavesForecast,
        m.waves_forecast.as_ref(),
    );
    push_series(
        &mut out,
        Measurement::SeaSurfaceTempForecast,
        m.sea_surface_temp_forecast_c.as_ref(),
    );
    push_series(
        &mut out,
        Measurement::ObservedWind,
        m.observed_wind.as_ref(),
    );
    push_series(
        &mut out,
        Measurement::ObservedPressure,
        m.observed_pressure_hpa.as_ref(),
    );
    out
}

const fn measurement_label(m: Measurement) -> &'static str {
    match m {
        Measurement::WaterTemperature => "Water temperature",
        Measurement::Discharge => "River flow",
        Measurement::GageHeight => "Gage height",
        Measurement::LakeLevel => "Lake level",
        Measurement::Turbidity => "Turbidity",
        Measurement::DissolvedOxygen => "Dissolved oxygen",
        Measurement::Conductivity => "Conductivity",
        Measurement::Salinity => "Salinity",
        Measurement::Ph => "pH",
        Measurement::Chlorophyll => "Chlorophyll",
        Measurement::Clarity => "Water clarity",
        Measurement::Cyanobacteria => "Cyanobacteria",
        Measurement::Tide => "Tide",
        Measurement::TidalCurrent => "Tidal current",
        Measurement::OceanCurrent => "Ocean current",
        Measurement::Waves => "Waves",
        Measurement::WavesForecast => "Wave forecast",
        Measurement::SeaSurfaceTemp => "Sea surface temperature",
        Measurement::SeaSurfaceTempForecast => "Sea surface temperature forecast",
        Measurement::MarineForecast => "Marine forecast",
        Measurement::MarineAlerts => "Marine alerts",
        Measurement::FloodCategories => "Flood categories",
        Measurement::FlowForecast => "River forecast",
        Measurement::ObservedWind => "Wind",
        Measurement::ObservedPressure => "Pressure",
    }
}

const fn relation_label(r: Relation) -> &'static str {
    match r {
        Relation::SameWaterBody => "Same water body",
        Relation::Upstream => "Upstream",
        Relation::Downstream => "Downstream",
        Relation::Nearby => "Nearby",
        Relation::Gridded => "Model grid",
        Relation::Satellite => "Satellite",
    }
}

const fn kind_label(k: ValueKind) -> &'static str {
    match k {
        ValueKind::Observed => "Observed",
        ValueKind::Predicted => "Predicted",
        ValueKind::Modelled => "Modelled",
        ValueKind::Satellite => "Satellite",
        ValueKind::Computed => "Computed",
    }
}

const fn error_kind_label(k: SourceErrorKind) -> &'static str {
    match k {
        SourceErrorKind::RateLimited => "Rate limited",
        SourceErrorKind::NotFound => "Not found",
        SourceErrorKind::NoData => "No data",
        SourceErrorKind::Stale => "Out of date",
        SourceErrorKind::Parse => "Unreadable response",
        SourceErrorKind::Transport => "Connection problem",
        SourceErrorKind::Other => "Failed",
    }
}

/// The credit a source is owed: the snapshot's own wording and link when it has them.
fn credit_for(snapshot: &SpotSnapshot, id: &SourceId) -> CreditVm {
    let (text, link, licence) = match id.as_str() {
        "usgs" => (
            "U.S. Geological Survey",
            "https://www.usgs.gov/information-policies-and-instructions/copyrights-and-credits",
            Some("Public domain"),
        ),
        "coops" => (
            "Tide, water-level and current data: NOAA/NOS CO-OPS",
            "https://tidesandcurrents.noaa.gov/",
            Some("Public domain"),
        ),
        "coops-harmonics" => (
            "Computed on this device from NOAA CO-OPS harmonic constants",
            "https://tidesandcurrents.noaa.gov/",
            Some("Public domain"),
        ),
        "ndbc" => (
            "Observations: NOAA National Data Buoy Center",
            "https://www.ndbc.noaa.gov/",
            Some("Public domain"),
        ),
        "nwps" => (
            "River forecasts: NOAA National Water Prediction Service",
            "https://water.noaa.gov/",
            Some("Public domain"),
        ),
        "nws" => (
            "Marine forecasts and alerts: NOAA National Weather Service",
            "https://www.weather.gov/",
            Some("Public domain"),
        ),
        "open-meteo-marine" => (
            "Weather data by Open-Meteo.com",
            OPEN_METEO,
            Some("CC BY 4.0"),
        ),
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
fn credits(snapshot: &SpotSnapshot, used: &[Used<'_>]) -> Vec<CreditVm> {
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
                licence: (w.provider == ProviderId::OpenMeteo).then(|| "CC BY 4.0".to_owned()),
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
            models.push(format!("Weather models: {}", names.join(", ")));
        }
    }
    if seen.contains(&SourceId::OPEN_METEO_MARINE) {
        models.push(MARINE_MODELS.to_owned());
    }
    if !models.is_empty() {
        add(
            &mut out,
            CreditVm {
                source: "Open-Meteo model providers".to_owned(),
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
                text: "Provisional data statement".to_owned(),
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
        let measurement = measurement_label(u.measurement);
        // Predictions and computed curves have no age to speak of.
        let aged = matches!(p.kind, ValueKind::Observed | ValueKind::Satellite);
        let latest = u.latest.unwrap_or(p.observed_at);
        let state = card_state(input, f, u.measurement, p, aged.then_some(latest));
        // One row per station and kind of value, naming everything it supplies.
        let kind = kind_label(p.kind);
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
            age: if aged {
                f.age(input.now.0 - latest.0)
            } else {
                f.strings.get(Key::Missing).to_owned()
            },
            relation: relation_label(p.relation).to_owned(),
            kind: kind.to_owned(),
            state,
        });
    }
    out
}

fn error_vm(e: &SourceError) -> SourceErrorVm {
    SourceErrorVm {
        source: source_name(&e.source),
        measurement: e.measurement.map(|m| measurement_label(m).to_owned()),
        kind: error_kind_label(e.kind).to_owned(),
        // The source's own plain message: "USGS: rate limited, used NWPS stage".
        message: e.message.clone(),
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// Every credit, station and problem behind the page.
pub fn build(input: &ViewInput) -> Option<SourcesCard> {
    let snapshot = &input.snapshot;
    let f = Formatter::for_input(input);
    let used = used(snapshot);
    let credits = credits(snapshot, &used);
    let stations = station_uses(input, &f, &used);
    let errors: Vec<SourceErrorVm> = snapshot.errors.iter().map(error_vm).collect();
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
        (!stations.is_empty()).then(|| plural(stations.len(), "station", "stations")),
        (!errors.is_empty()).then(|| plural(errors.len(), "problem", "problems")),
    ]
    .into_iter()
    .flatten()
    .collect();
    Some(SourcesCard {
        compact: Compact {
            headline: Quantity {
                value: Some(credits.len() as f32),
                text: credits.len().to_string(),
                unit: "sources".to_owned(),
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
}
