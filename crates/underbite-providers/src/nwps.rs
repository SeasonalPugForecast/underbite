//! NOAA National Water Prediction Service: river gauges, forecasts and flood categories.
//!
//! [`Nwps`] plans, for a gauge (a [`Place::Station`] of source NWPS, id = NWS location id),
//! the gauge's metadata (flood stages) and its official forecast, and, for a rounded point,
//! the gauges in a small box around it. The service answers plain JSON, needs no key and
//! states no limit.
//!
//! Observed stage and flow are USGS's data relayed, so this source never asks for the
//! 400 KB observed series. The forecast gives stage (feet) and flow (kcfs, thousands of cubic
//! feet per second); `-999` and `-9999` mean "no value". [`parse_gauge`] reads the rest of a
//! gauge's metadata (USGS id, notes, service state) that has no fragment shape.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use underbite_core::datum::{Height, Heights, VerticalDatum};
use underbite_core::model::{
    Coverage, FloodCategories, Measurement, Requirements, StationCandidate,
};
use underbite_core::provenance::{
    Attribution, Provenance, Quality, Relation, SourceId, StationKind, StationRef, ValueKind,
};
use underbite_core::series::{Sample, Series};
use underbite_core::time::{UnixTime, parse_rfc3339};
use underbite_core::units::{Canonical, to_canonical};

use crate::request::{
    Place, QueryBuilder, Request, Resource, Response, RoundedPoint, Target, encode_path_segment,
};
use crate::source::{FetchContext, FragmentValue, Fragments, Source, SourceError, check_response};

const BASE: &str = "https://api.water.noaa.gov/nwps/v1";

/// Half the side of the search box around a point, degrees (about 25 km).
const HALF_WIDTH_DEG: f64 = 0.25;

/// Seconds gauge metadata and gauge lists may be reused.
const METADATA_MAX_AGE_S: u32 = 86_400;
/// Seconds a forecast may be reused; forecasts are issued a few times a day.
const FORECAST_MAX_AGE_S: u32 = 3_600;

const MEASUREMENTS: [Measurement; 2] = [Measurement::FlowForecast, Measurement::FloodCategories];

/// The NWPS API as a source. It holds no settings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Nwps;

/// What a gauge's metadata says beyond its flood stages.
#[derive(Clone, Debug, PartialEq)]
pub struct Gauge {
    /// The NWS location id.
    pub lid: String,
    /// Display name.
    pub name: String,
    /// The USGS site number (without `USGS-`) when USGS operates the gauge.
    pub usgs_id: Option<String>,
    /// The National Water Model reach.
    pub reach_id: Option<String>,
    /// The datum of the gauge's stage values.
    pub datum: VerticalDatum,
    /// Flood stages above the gauge's datum; undefined ones are `None`.
    pub flood_categories: FloodCategories,
    /// Notes such as "Gauge reading affected by ocean tides.", as plain text.
    pub notes: Vec<String>,
    /// Whether the gauge is in service.
    pub in_service: bool,
    /// The service's message about the gauge's state, if any.
    pub service_message: Option<String>,
    /// Whether the gauge has a forecast product.
    pub has_forecast: bool,
    /// Credit lines the payload names.
    pub credits: Vec<Attribution>,
}

fn parse_error(what: impl std::fmt::Display) -> SourceError {
    SourceError::Parse(what.to_string())
}

/// Whether `v` is one of the service's "no value" markers. Stage can be negative, so only
/// these exact values count.
#[allow(clippy::float_cmp)] // exact markers, not measurements
fn is_sentinel(v: f64) -> bool {
    v == -999.0 || v == -9999.0
}

/// The datum a product code's stage is in: `HG` is height above the gauge zero, `HM` a tide
/// gauge in feet MLLW. Pool elevations (`HP`) and others have no datum we know.
fn datum_of(pedts: &str) -> Option<VerticalDatum> {
    match pedts.get(..2)? {
        "HG" => Some(VerticalDatum::Gauge),
        "HM" => Some(VerticalDatum::Mllw),
        _ => None,
    }
}

fn length_m(value: f64, unit: &str) -> Result<f32, SourceError> {
    match to_canonical(value as f32, unit).map_err(parse_error)? {
        Canonical::Length(m) => Ok(m),
        other => Err(parse_error(format!("{unit:?} is not a length: {other:?}"))),
    }
}

fn discharge_m3s(value: f64, unit: &str) -> Result<f32, SourceError> {
    match to_canonical(value as f32, unit).map_err(parse_error)? {
        Canonical::Discharge(q) => Ok(q),
        other => Err(parse_error(format!("{unit:?} is not a flow: {other:?}"))),
    }
}

/// URL of a gauge resource.
fn gauge_url(lid: &str, suffix: &str) -> String {
    format!("{BASE}/gauges/{}{suffix}", encode_path_segment(lid))
}

#[derive(Deserialize)]
struct RawGauge {
    #[serde(default)]
    lid: String,
    #[serde(default)]
    name: String,
    #[serde(rename = "usgsId", default)]
    usgs_id: Option<String>,
    #[serde(rename = "reachId", default)]
    reach_id: Option<String>,
    #[serde(default)]
    pedts: RawPedts,
    #[serde(default)]
    flood: RawFlood,
    #[serde(default)]
    hydronotes: Vec<RawNote>,
    #[serde(rename = "inService", default)]
    in_service: Option<RawService>,
    #[serde(rename = "dataAttribution", default)]
    data_attribution: Vec<RawAttribution>,
}

#[derive(Deserialize, Default)]
struct RawPedts {
    #[serde(default)]
    observed: String,
    #[serde(default)]
    forecast: String,
}

#[derive(Deserialize, Default)]
struct RawFlood {
    #[serde(rename = "stageUnits", default)]
    stage_units: Option<String>,
    #[serde(default)]
    categories: BTreeMap<String, RawThreshold>,
}

#[derive(Deserialize)]
struct RawThreshold {
    stage: Option<f64>,
}

#[derive(Deserialize)]
struct RawNote {
    #[serde(default)]
    statement: String,
}

#[derive(Deserialize)]
struct RawService {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    message: String,
}

#[derive(Deserialize)]
struct RawAttribution {
    #[serde(default)]
    title: String,
    #[serde(default)]
    url: String,
}

fn non_empty(s: Option<String>) -> Option<String> {
    s.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// A note's text without markup: the `<head>` block is dropped, tags removed, common
/// entities decoded, whitespace collapsed.
fn plain_text(html: &str) -> String {
    let mut text = html.to_string();
    if let (Some(start), Some(end)) = (text.find("<head>"), text.find("</head>"))
        && start < end
    {
        text.replace_range(start..end + "</head>".len(), "");
    }
    let mut out = String::with_capacity(text.len());
    let mut in_tag = false;
    for c in text.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    let out = out
        .replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&");
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Reads a `/gauges/{lid}` body.
///
/// # Errors
/// [`SourceError::Parse`] when the body is not a gauge or a threshold has an unknown unit.
pub fn parse_gauge(body: &[u8]) -> Result<Gauge, SourceError> {
    let raw: RawGauge =
        serde_json::from_slice(body).map_err(|e| parse_error(format!("not a gauge: {e}")))?;
    if raw.lid.is_empty() {
        return Err(parse_error("not a gauge: no lid"));
    }
    let datum = datum_of(&raw.pedts.observed)
        .or_else(|| datum_of(&raw.pedts.forecast))
        .unwrap_or(VerticalDatum::Gauge);
    let unit = raw.flood.stage_units.as_deref().unwrap_or("ft");
    let threshold = |key: &str| -> Result<Option<Height>, SourceError> {
        raw.flood
            .categories
            .get(key)
            .and_then(|t| t.stage)
            .filter(|v| !is_sentinel(*v))
            .map(|v| length_m(v, unit).map(|metres| Height { datum, metres }))
            .transpose()
    };
    let flood_categories = FloodCategories {
        action: threshold("action")?,
        minor: threshold("minor")?,
        moderate: threshold("moderate")?,
        major: threshold("major")?,
        provenance: None,
    };
    let notes = raw
        .hydronotes
        .iter()
        .map(|n| plain_text(&n.statement))
        .filter(|n| !n.is_empty())
        .collect();
    let credits = raw
        .data_attribution
        .iter()
        .filter(|a| !a.title.trim().is_empty())
        .map(|a| Attribution {
            source: SourceId::NWPS,
            credit: a.title.trim().to_string(),
            url: non_empty(Some(a.url.clone())),
        })
        .collect();
    let (in_service, service_message) = match raw.in_service {
        Some(s) => (s.enabled, non_empty(Some(s.message))),
        None => (true, None),
    };
    Ok(Gauge {
        lid: raw.lid,
        name: raw.name,
        usgs_id: non_empty(raw.usgs_id),
        reach_id: non_empty(raw.reach_id),
        datum,
        flood_categories,
        notes,
        in_service,
        service_message,
        has_forecast: !raw.pedts.forecast.is_empty(),
        credits,
    })
}

#[derive(Deserialize)]
struct RawList {
    #[serde(default)]
    gauges: Vec<RawListItem>,
}

#[derive(Deserialize)]
struct RawListItem {
    lid: String,
    #[serde(default)]
    name: String,
    latitude: f64,
    longitude: f64,
    #[serde(default)]
    pedts: RawPedts,
    #[serde(default)]
    status: RawStatuses,
}

#[derive(Deserialize, Default)]
struct RawStatuses {
    #[serde(default)]
    observed: RawStatus,
}

#[derive(Deserialize, Default)]
struct RawStatus {
    #[serde(rename = "floodCategory", default)]
    flood_category: String,
    #[serde(rename = "validTime", default)]
    valid_time: String,
}

/// A gauge's observed flood category, as a gauge list states it
/// (`status.observed.floodCategory`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FloodCategory {
    /// Below every flood stage.
    NoFlooding,
    /// At or above the action stage.
    Action,
    /// Minor flooding.
    Minor,
    /// Moderate flooding.
    Moderate,
    /// Major flooding.
    Major,
    /// Below the gauge's low-water threshold.
    LowThreshold,
    /// The gauge has no flood stages.
    NotDefined,
    /// The latest observation is too old to classify.
    ObsNotCurrent,
    /// The forecast is too old to classify.
    FcstNotCurrent,
    /// The gauge is out of service.
    OutOfService,
    /// Missing, or a value not seen when the service was checked.
    Unknown,
}

impl FloodCategory {
    /// The category the service writes as `code` (`no_flooding`, `action`, `minor`, ...).
    /// `minor`, `moderate` and `major` follow the names of the flood stages; they were not
    /// observed when the service was checked.
    pub fn from_code(code: &str) -> Self {
        match code.trim() {
            "no_flooding" => Self::NoFlooding,
            "action" => Self::Action,
            "minor" => Self::Minor,
            "moderate" => Self::Moderate,
            "major" => Self::Major,
            "low_threshold" => Self::LowThreshold,
            "not_defined" => Self::NotDefined,
            "obs_not_current" => Self::ObsNotCurrent,
            "fcst_not_current" => Self::FcstNotCurrent,
            "out_of_service" => Self::OutOfService,
            _ => Self::Unknown,
        }
    }
}

/// One gauge of a gauge list: the station and what it offers, and its observed state.
#[derive(Clone, Debug, PartialEq)]
pub struct ListedGauge {
    /// The gauge as a station candidate (see [`parse_gauges`]).
    pub candidate: StationCandidate,
    /// The observed flood category.
    pub flood_category: FloodCategory,
    /// When the category was observed; `None` for a gauge that never reported.
    pub observed_at: Option<UnixTime>,
}

/// The gauges in a `/gauges?bbox…` body, each with what it offers (see [`parse_gauges`]) and
/// its observed flood category.
///
/// # Errors
/// [`SourceError::Parse`] when the body is not a gauge list.
pub fn parse_gauge_list(body: &[u8]) -> Result<Vec<ListedGauge>, SourceError> {
    let list: RawList =
        serde_json::from_slice(body).map_err(|e| parse_error(format!("not a gauge list: {e}")))?;
    Ok(list
        .gauges
        .into_iter()
        .map(|g| {
            let observed = g.status.observed;
            // A gauge that never reported has the zero time `0001-01-01T00:00:00Z`.
            let observed_at = parse_rfc3339(&observed.valid_time).filter(|t| t.0 > 0);
            let flood_category = FloodCategory::from_code(&observed.flood_category);
            let last_value_at =
                observed_at.filter(|_| flood_category != FloodCategory::OutOfService);
            let coverage = Coverage {
                last_value_at,
                primary: true,
            };
            let mut offers = BTreeMap::new();
            if !g.pedts.forecast.is_empty() {
                offers.insert(Measurement::FlowForecast, coverage);
            }
            // The list does not carry thresholds, so any gauge may have flood stages; the
            // gauge's own metadata says.
            offers.insert(Measurement::FloodCategories, coverage);
            ListedGauge {
                candidate: StationCandidate {
                    station: StationRef {
                        source: SourceId::NWPS,
                        id: g.lid,
                        name: g.name,
                        kind: StationKind::NwpsGauge,
                        latitude: g.latitude,
                        longitude: g.longitude,
                        bin: None,
                        reference_id: None,
                    },
                    offers,
                },
                flood_category,
                observed_at,
            }
        })
        .collect())
}

/// The gauges in a `/gauges?bbox…` body and what each offers: a flow forecast when it has a
/// forecast product, and flood categories (the list does not say whether thresholds are
/// defined; the gauge's own metadata does). [`parse_gauge_list`] adds each gauge's observed
/// flood category.
///
/// # Errors
/// [`SourceError::Parse`] when the body is not a gauge list.
pub fn parse_gauges(body: &[u8]) -> Result<Vec<StationCandidate>, SourceError> {
    Ok(parse_gauge_list(body)?
        .into_iter()
        .map(|g| g.candidate)
        .collect())
}

#[derive(Deserialize)]
struct RawForecast {
    #[serde(default)]
    pedts: String,
    #[serde(rename = "issuedTime", default)]
    issued_time: String,
    #[serde(rename = "primaryUnits", default)]
    primary_units: String,
    #[serde(rename = "secondaryUnits", default)]
    secondary_units: String,
    #[serde(default)]
    data: Vec<RawPoint>,
}

#[derive(Deserialize)]
struct RawPoint {
    #[serde(rename = "validTime")]
    valid_time: String,
    primary: Option<f64>,
    secondary: Option<f64>,
}

/// A parsed forecast: stage (metres above `datum`, when the product's datum is known) and
/// flow (m³/s), each with its own samples, and the time it was issued.
#[derive(Clone, Debug, PartialEq)]
pub struct Forecast {
    /// When the forecast was issued.
    pub issued: UnixTime,
    /// The datum of `stage`, when known.
    pub datum: Option<VerticalDatum>,
    /// Stage samples, metres.
    pub stage: Vec<Sample<f32>>,
    /// Flow samples, m³/s.
    pub flow: Vec<Sample<f32>>,
}

/// Reads a `/gauges/{lid}/stageflow/forecast` body; sentinel values are dropped.
///
/// # Errors
/// [`SourceError::Parse`] when the body is not a forecast, a time is malformed or a unit is
/// unknown.
pub fn parse_forecast(body: &[u8]) -> Result<Forecast, SourceError> {
    let raw: RawForecast =
        serde_json::from_slice(body).map_err(|e| parse_error(format!("not a forecast: {e}")))?;
    let issued = parse_rfc3339(&raw.issued_time)
        .ok_or_else(|| parse_error(format!("issuedTime {:?}", raw.issued_time)))?;
    let (mut stage, mut flow) = (Vec::new(), Vec::new());
    for p in &raw.data {
        let time = parse_rfc3339(&p.valid_time)
            .ok_or_else(|| parse_error(format!("validTime {:?}", p.valid_time)))?;
        if let Some(v) = p.primary.filter(|v| !is_sentinel(*v)) {
            stage.push(Sample::new(time, length_m(v, &raw.primary_units)?));
        }
        // A negative flow is never real.
        if let Some(v) = p.secondary.filter(|v| !is_sentinel(*v) && *v >= 0.0) {
            flow.push(Sample::new(time, discharge_m3s(v, &raw.secondary_units)?));
        }
    }
    Ok(Forecast {
        issued,
        datum: datum_of(&raw.pedts),
        stage,
        flow,
    })
}

fn bbox_url(point: RoundedPoint) -> String {
    let c = point.coordinates();
    let coordinate = |v: f64| {
        let s = format!("{v:.4}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    };
    QueryBuilder::new(&format!("{BASE}/gauges"))
        .param("bbox.xmin", coordinate(c.longitude - HALF_WIDTH_DEG))
        .param("bbox.ymin", coordinate(c.latitude - HALF_WIDTH_DEG))
        .param("bbox.xmax", coordinate(c.longitude + HALF_WIDTH_DEG))
        .param(
            "bbox.ymax",
            coordinate((c.latitude + HALF_WIDTH_DEG).min(90.0)),
        )
        .param("srid", "EPSG_4326")
        .finish()
}

fn wants(target: &Target, requirements: &Requirements, m: Measurement) -> bool {
    target.measurements.contains(&m)
        && (requirements.measurements.is_empty() || requirements.measurements.contains(&m))
}

impl Source for Nwps {
    fn id(&self) -> SourceId {
        SourceId::NWPS
    }

    fn attribution(&self) -> Attribution {
        Attribution {
            source: SourceId::NWPS,
            credit: "NOAA National Water Prediction Service".into(),
            url: Some("https://water.noaa.gov/".into()),
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
        match &target.place {
            Place::Point(point) => Ok(vec![
                Request::new(bbox_url(*point), Resource::NwpsGauges, target.clone())
                    .max_age_hint(METADATA_MAX_AGE_S),
            ]),
            Place::Station { station, .. } if station.source == SourceId::NWPS => {
                let mut requests = Vec::new();
                if wants(target, requirements, Measurement::FloodCategories) {
                    requests.push(
                        Request::new(
                            gauge_url(&station.id, ""),
                            Resource::NwpsGauge,
                            target.clone(),
                        )
                        .max_age_hint(METADATA_MAX_AGE_S),
                    );
                }
                if wants(target, requirements, Measurement::FlowForecast) {
                    let request = Request::new(
                        gauge_url(&station.id, "/stageflow/forecast"),
                        Resource::NwpsStageFlowForecast,
                        target.clone(),
                    )
                    .max_age_hint(FORECAST_MAX_AGE_S);
                    requests.push(if requests.is_empty() {
                        request
                    } else {
                        request.optional()
                    });
                }
                if requests.is_empty() {
                    return Err(SourceError::Unsupported(format!(
                        "NWPS offers none of {:?}",
                        target.measurements
                    )));
                }
                Ok(requests)
            }
            _ => Err(SourceError::Unsupported(
                "NWPS needs an NWPS gauge or a point".into(),
            )),
        }
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
            Resource::NwpsGauge => {
                let mut gauge = parse_gauge(body)?;
                for credit in gauge.credits {
                    out.credit(credit);
                }
                if gauge.flood_categories == FloodCategories::default() {
                    return Err(SourceError::NoData(Some(
                        "the gauge defines no flood stages".into(),
                    )));
                }
                gauge.flood_categories.provenance =
                    Some(categories_provenance(&request.target, ctx.now));
                out.push(
                    Measurement::FloodCategories,
                    FragmentValue::FloodCategories(gauge.flood_categories),
                )
            }
            Resource::NwpsStageFlowForecast => {
                let forecast = parse_forecast(body)?;
                let provenance = forecast_provenance(&request.target, forecast.issued);
                let mut any = false;
                if let (Some(datum), false) = (forecast.datum, forecast.stage.is_empty()) {
                    out.push(
                        Measurement::FlowForecast,
                        FragmentValue::Heights(Heights {
                            datum,
                            series: Series::new(provenance.clone(), forecast.stage),
                        }),
                    )?;
                    any = true;
                }
                if !forecast.flow.is_empty() {
                    out.push(
                        Measurement::FlowForecast,
                        FragmentValue::Series(Series::new(provenance, forecast.flow)),
                    )?;
                    any = true;
                }
                if any {
                    Ok(())
                } else {
                    Err(SourceError::NoData(Some(
                        "the forecast has no values".into(),
                    )))
                }
            }
            Resource::NwpsGauges => {
                for candidate in parse_gauges(body)? {
                    for (m, coverage) in candidate.offers {
                        out.push(
                            m,
                            FragmentValue::Coverage {
                                station_id: candidate.station.id.clone(),
                                coverage,
                            },
                        )?;
                    }
                }
                Ok(())
            }
            other => Err(SourceError::Unsupported(format!(
                "NWPS does not parse {other:?}"
            ))),
        }
    }
}

/// Provenance of a gauge's flood stages: thresholds the forecast office set for that gauge,
/// read at `now`.
fn categories_provenance(target: &Target, now: UnixTime) -> Provenance {
    Provenance {
        observed_at: now,
        kind: ValueKind::Observed,
        quality: Quality::Approved,
        ..forecast_provenance(target, now)
    }
}

/// Provenance of a forecast: official guidance from the River Forecast Centers, issued at
/// `issued`.
fn forecast_provenance(target: &Target, issued: UnixTime) -> Provenance {
    let (station, distance_m, relation) = match &target.place {
        Place::Station {
            station,
            distance_m,
            relation,
        } => (Some(station.clone()), *distance_m, *relation),
        _ => (None, None, Relation::Nearby),
    };
    Provenance {
        source: SourceId::NWPS,
        station,
        distance_m,
        relation,
        observed_at: issued,
        kind: ValueKind::Modelled,
        quality: Quality::Provisional,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::request::{Coordinates, Window};
    use crate::testutil::fixture_bytes;

    const NOW: UnixTime = UnixTime(1_794_148_200);

    fn ctx() -> FetchContext {
        FetchContext::new(NOW, "underbite/test")
    }

    fn gauge(lid: &str) -> Gauge {
        parse_gauge(&fixture_bytes("nwps", &format!("nwps-gauge-{lid}.json"))).unwrap()
    }

    fn feet(ft: f32) -> f32 {
        ft * 0.3048
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    fn station_target(lid: &str, ms: &[Measurement]) -> Target {
        Target {
            place: Place::Station {
                station: StationRef {
                    source: SourceId::NWPS,
                    id: lid.into(),
                    name: lid.into(),
                    kind: StationKind::NwpsGauge,
                    latitude: 42.93,
                    longitude: -76.57,
                    bin: None,
                    reference_id: None,
                },
                distance_m: Some(1200.0),
                relation: Relation::SameWaterBody,
            },
            measurements: ms.iter().copied().collect::<BTreeSet<_>>(),
            window: Window { from: NOW, to: NOW },
            datum: None,
        }
    }

    fn levels(g: &Gauge) -> [Option<f32>; 4] {
        let c = &g.flood_categories;
        [c.action, c.minor, c.moderate, c.major].map(|h| h.map(|h| h.metres))
    }

    #[test]
    fn owasco_flood_stages_and_usgs_id() {
        let g = gauge("AUON6");
        assert_eq!(g.usgs_id.as_deref(), Some("04235440"));
        assert_eq!(g.reach_id.as_deref(), Some("21977249"));
        assert_eq!(g.datum, VerticalDatum::Gauge);
        for (got, ft) in levels(&g).into_iter().zip([7.0, 8.0, 9.0, 10.0]) {
            assert!(close(got.unwrap(), feet(ft)), "{got:?} vs {ft} ft");
        }
        assert!(g.has_forecast && g.in_service);
        assert!(g.notes[0].contains("affected by ice"));
        assert!(
            g.credits
                .iter()
                .any(|c| c.credit == "Observations courtesy of U.S. Geological Survey")
        );
    }

    #[test]
    fn cohoes_flood_stages() {
        let g = gauge("COHN6");
        for (got, ft) in levels(&g).into_iter().zip([17.0, 20.0, 21.0, 22.0]) {
            assert!(close(got.unwrap(), feet(ft)));
        }
        assert_eq!(g.usgs_id.as_deref(), Some("01357500"));
    }

    #[test]
    fn katonah_has_no_flood_stages_and_no_forecast() {
        let g = gauge("KTNN6");
        assert_eq!(g.flood_categories, FloodCategories::default());
        assert!(!g.has_forecast);
        let target = station_target("KTNN6", &[Measurement::FloodCategories]);
        let req = Nwps
            .plan(&target, &Requirements::default(), &ctx())
            .unwrap()
            .remove(0);
        let body = fixture_bytes("nwps", "nwps-gauge-KTNN6.json");
        let mut out = Fragments::new();
        let err = Nwps
            .parse(&req, &Response::new(200, &body), &ctx(), &mut out)
            .unwrap_err();
        assert!(matches!(err, SourceError::NoData(_)));
        assert_eq!(out.attributions().len(), 2);
    }

    #[test]
    fn albany_keeps_tidal_note() {
        let g = gauge("ALBN6");
        assert!(
            g.notes
                .iter()
                .any(|n| n == "Gauge reading affected by ocean tides.")
        );
        assert_eq!(g.usgs_id.as_deref(), Some("01359139"));
    }

    #[test]
    fn battery_is_mllw_without_usgs_id() {
        let g = gauge("BATN6");
        assert_eq!(g.usgs_id, None);
        assert_eq!(g.reach_id, None);
        assert_eq!(g.datum, VerticalDatum::Mllw);
        assert_eq!(g.flood_categories.major.unwrap().datum, VerticalDatum::Mllw);
        assert!(close(g.flood_categories.major.unwrap().metres, feet(9.4)));
        let note = &g.notes[0];
        assert!(!note.contains('<') && !note.contains("style1") && !note.contains("&nbsp;"));
        assert!(note.starts_with("Tide Forecasts are a total water level forecast"));
        assert!(
            g.credits
                .iter()
                .any(|c| c.credit == "NOAA National Ocean Service")
        );
    }

    #[test]
    fn cohoes_forecast_converts_stage_and_kcfs() {
        let f = parse_forecast(&fixture_bytes(
            "nwps",
            "nwps-gauge-COHN6-stageflow-forecast.json",
        ))
        .unwrap();
        assert_eq!(f.stage.len(), 12);
        assert_eq!(f.flow.len(), 12);
        assert_eq!(f.datum, Some(VerticalDatum::Gauge));
        assert!(close(f.stage[0].value, feet(8.6)));
        // 1.59 kcfs = 1590 ft³/s.
        assert!(close(f.flow[0].value, 1590.0 * 0.028_316_846));
        assert_eq!(f.issued, parse_rfc3339("2026-11-07T16:45:00Z").unwrap());
    }

    #[test]
    fn owasco_forecast_is_flat() {
        let f = parse_forecast(&fixture_bytes(
            "nwps",
            "nwps-gauge-AUON6-stageflow-forecast.json",
        ))
        .unwrap();
        assert_eq!(f.stage.len(), 12);
        assert!(f.stage.iter().all(|s| close(s.value, feet(1.8))));
    }

    #[test]
    fn sentinels_are_dropped_and_negative_stage_kept() {
        let body = br#"{"pedts":"HGIFF","issuedTime":"2026-11-07T16:45:00Z","primaryUnits":"ft",
            "secondaryUnits":"kcfs","data":[
            {"validTime":"2026-11-07T18:00:00Z","primary":-0.47,"secondary":-9999},
            {"validTime":"2026-11-07T19:00:00Z","primary":-999,"secondary":-999},
            {"validTime":"2026-11-07T20:00:00Z","primary":1.0,"secondary":-0.5}]}"#;
        let f = parse_forecast(body).unwrap();
        assert_eq!(f.stage.len(), 2);
        assert!(close(f.stage[0].value, feet(-0.47)));
        assert_eq!(f.flow.len(), 0);
    }

    #[test]
    fn forecast_fragments_carry_provenance() {
        let target = station_target("COHN6", &[Measurement::FlowForecast]);
        let reqs = Nwps
            .plan(&target, &Requirements::default(), &ctx())
            .unwrap();
        assert_eq!(reqs.len(), 1);
        assert_eq!(
            reqs[0].url,
            "https://api.water.noaa.gov/nwps/v1/gauges/COHN6/stageflow/forecast"
        );
        let body = fixture_bytes("nwps", "nwps-gauge-COHN6-stageflow-forecast.json");
        let mut out = Fragments::new();
        Nwps.parse(&reqs[0], &Response::new(200, &body), &ctx(), &mut out)
            .unwrap();
        let [heights, flow] = out.fragments() else {
            panic!("expected two fragments");
        };
        let FragmentValue::Heights(h) = &heights.value else {
            panic!("heights first");
        };
        assert_eq!(h.datum, VerticalDatum::Gauge);
        let p = h.series.provenance();
        assert_eq!(p.source, SourceId::NWPS);
        assert_eq!(p.station.as_ref().unwrap().id, "COHN6");
        assert_eq!(p.distance_m, Some(1200.0));
        assert!(matches!(flow.value, FragmentValue::Series(_)));
    }

    #[test]
    fn plan_both_measurements_makes_forecast_optional() {
        let target = station_target(
            "AUON6",
            &[Measurement::FlowForecast, Measurement::FloodCategories],
        );
        let reqs = Nwps
            .plan(&target, &Requirements::default(), &ctx())
            .unwrap();
        assert_eq!(reqs.len(), 2);
        assert_eq!(reqs[0].resource, Resource::NwpsGauge);
        assert!(reqs[0].required && !reqs[1].required);
    }

    #[test]
    fn plan_point_builds_a_small_box() {
        let target = Target {
            place: Place::Point(RoundedPoint::new(
                Coordinates {
                    latitude: 41.25,
                    longitude: -73.85,
                },
                2,
            )),
            measurements: BTreeSet::from([Measurement::FlowForecast]),
            window: Window { from: NOW, to: NOW },
            datum: None,
        };
        let reqs = Nwps
            .plan(&target, &Requirements::default(), &ctx())
            .unwrap();
        assert_eq!(
            reqs[0].url,
            "https://api.water.noaa.gov/nwps/v1/gauges?bbox.xmin=-74.1&bbox.ymin=41&\
             bbox.xmax=-73.6&bbox.ymax=41.5&srid=EPSG_4326"
        );
    }

    #[test]
    fn lower_hudson_list_gives_candidates() {
        let c = parse_gauges(&fixture_bytes("nwps", "nwps-gauges-bbox-lower-hudson.json")).unwrap();
        assert_eq!(c.len(), 32);
        assert!(c.iter().all(|c| c.station.source == SourceId::NWPS));
        let with_forecast = c
            .iter()
            .filter(|c| c.offers.contains_key(&Measurement::FlowForecast))
            .count();
        assert!(with_forecast > 0 && with_forecast < 32);
    }

    #[test]
    fn lower_hudson_list_gives_flood_categories() {
        let body = fixture_bytes("nwps", "nwps-gauges-bbox-lower-hudson.json");
        let gauges = parse_gauge_list(&body).unwrap();
        let count = |c: FloodCategory| gauges.iter().filter(|g| g.flood_category == c).count();
        assert_eq!(count(FloodCategory::NoFlooding), 23);
        assert_eq!(count(FloodCategory::NotDefined), 5);
        assert_eq!(count(FloodCategory::OutOfService), 3);
        assert_eq!(count(FloodCategory::ObsNotCurrent), 1);
        let by_lid = |lid: &str| {
            gauges
                .iter()
                .find(|g| g.candidate.station.id == lid)
                .unwrap()
        };
        assert_eq!(
            by_lid("HOHN4").observed_at,
            parse_rfc3339("2026-11-08T13:30:00Z")
        );
        let pelham = by_lid("HAMN6");
        assert_eq!(pelham.flood_category, FloodCategory::OutOfService);
        assert_eq!(pelham.observed_at, None);
        // The candidates are those of `parse_gauges`, in the same order.
        let candidates: Vec<StationCandidate> = gauges.into_iter().map(|g| g.candidate).collect();
        assert_eq!(candidates, parse_gauges(&body).unwrap());
    }

    #[test]
    fn flood_categories_read_every_code() {
        for (code, category) in [
            ("no_flooding", FloodCategory::NoFlooding),
            ("action", FloodCategory::Action),
            ("minor", FloodCategory::Minor),
            ("moderate", FloodCategory::Moderate),
            ("major", FloodCategory::Major),
            ("low_threshold", FloodCategory::LowThreshold),
            ("not_defined", FloodCategory::NotDefined),
            ("obs_not_current", FloodCategory::ObsNotCurrent),
            ("fcst_not_current", FloodCategory::FcstNotCurrent),
            ("out_of_service", FloodCategory::OutOfService),
            ("", FloodCategory::Unknown),
            ("record", FloodCategory::Unknown),
        ] {
            assert_eq!(FloodCategory::from_code(code), category, "{code}");
        }
        assert_eq!(
            serde_json::to_string(&FloodCategory::NoFlooding).unwrap(),
            "\"no_flooding\""
        );
    }

    #[test]
    fn not_found_is_classified() {
        let target = station_target("ZZZZ9", &[Measurement::FloodCategories]);
        let req = Nwps
            .plan(&target, &Requirements::default(), &ctx())
            .unwrap()
            .remove(0);
        let body = br#"{"code":5,"message":"[ZZZZ9] could not find gauge ID","details":[]}"#;
        let err = Nwps
            .parse(
                &req,
                &Response::new(404, body),
                &ctx(),
                &mut Fragments::new(),
            )
            .unwrap_err();
        assert!(matches!(err, SourceError::NotFound(Some(m)) if m.contains("ZZZZ9")));
    }
}
