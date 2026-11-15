//! CO-OPS metadata API: stations, datums, harmonic constituents and offsets.
//!
//! Free functions: request builders and parsers, no [`Source`](crate::source::Source). The
//! single entry point for a host or a source is [`parse`], which dispatches on the request's
//! resource and returns a [`Metadata`] value.
//!
//! The datums and the tide constituents arrive in two responses and one result needs both:
//! plan [`datums_request`], then build the constituents request with [`harcon_follow_up`],
//! which carries the datum table to [`parse`] as request carry text.

use std::collections::BTreeMap;

use serde::Deserialize;
use underbite_core::astro::current::{
    CurrentConstituent, CurrentHarmonics, SubordinateCurrentOffsets,
};
use underbite_core::astro::tide::{
    HarmonicConstituent, HeightAdjustment, SubordinateTideOffsets, TideError, TideHarmonics,
};
use underbite_core::datum::{DatumTable, VerticalDatum};
use underbite_core::model::{Coverage, Measurement, StationCandidate};
use underbite_core::provenance::{SourceId, StationKind, StationRef};
use underbite_core::units::{Canonical, SourceUnit};

use crate::request::{Place, Request, Resource, Response, Target};
use crate::source::{SourceError, check_response};

/// The metadata API base.
pub const BASE: &str = "https://api.tidesandcurrents.noaa.gov/mdapi/prod/webapi";

/// Seconds a station list may be reused: the lists are large and change rarely.
pub const LIST_AGE_S: u32 = 7 * 86_400;
/// Seconds datums, constituents, offsets and bins may be reused.
pub const STATION_AGE_S: u32 = 30 * 86_400;

/// Unit assumed for additive (`F`) height offsets: feet. The response states no unit and the
/// CO-OPS documentation does not define `F`; subordinate-station offsets are published in
/// feet. Check with `live_tide_offsets_additive_unit` (ignored; needs the network).
pub const ADDITIVE_OFFSET_UNIT: SourceUnit = SourceUnit::Ft;

/// Which station list to fetch (`stations.json?type=…`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StationList {
    /// Tide prediction stations (`R` harmonic, `S` subordinate).
    TidePredictions,
    /// Current prediction stations, listed once per bin.
    CurrentPredictions,
    /// Water-level gauges.
    WaterLevels,
    /// Real-time current meters.
    Currents,
}

impl StationList {
    /// The `type` query value.
    pub const fn type_name(self) -> &'static str {
        match self {
            Self::TidePredictions => "tidepredictions",
            Self::CurrentPredictions => "currentpredictions",
            Self::WaterLevels => "waterlevels",
            Self::Currents => "currents",
        }
    }

    fn from_type_name(name: &str) -> Option<Self> {
        [
            Self::TidePredictions,
            Self::CurrentPredictions,
            Self::WaterLevels,
            Self::Currents,
        ]
        .into_iter()
        .find(|l| l.type_name() == name)
    }
}

/// One bin of a current meter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bin {
    /// The bin number.
    pub number: u16,
    /// Depth, metres.
    pub depth_m: f32,
}

/// What [`parse`] found.
#[derive(Clone, Debug, PartialEq)]
pub enum Metadata {
    /// A station list as candidates.
    Stations(Vec<StationCandidate>),
    /// A tide station's datums, metres above `STND`.
    Datums(DatumTable),
    /// A tide station's harmonics with `Z0` from the datums.
    TideHarmonics(TideHarmonics),
    /// One bin of a current station.
    CurrentHarmonics(CurrentHarmonics),
    /// A subordinate tide station's offsets; `None` for a reference station.
    TideOffsets(Option<SubordinateTideOffsets>),
    /// A subordinate current station's offsets.
    CurrentOffsets(SubordinateCurrentOffsets),
    /// The bins of a current meter.
    Bins(Vec<Bin>),
}

// ----------------------------------------------------------------------------------------
// Requests

fn station(target: &Target) -> Result<&StationRef, SourceError> {
    match &target.place {
        Place::Station { station, .. } => Ok(station),
        _ => Err(SourceError::Unsupported(
            "CO-OPS metadata needs a station target".into(),
        )),
    }
}

fn is_current(kind: StationKind) -> bool {
    matches!(
        kind,
        StationKind::CurrentHarmonic
            | StationKind::CurrentSubordinate
            | StationKind::CurrentWeakVariable
            | StationKind::PortsCurrentMeter
    )
}

/// The station list request. The list name travels as carry text so [`parse`] knows the shape.
pub fn stations_request(list: StationList, target: Target) -> Request {
    Request::new(
        format!("{BASE}/stations.json?type={}", list.type_name()),
        Resource::CoopsStations,
        target,
    )
    .max_age_hint(LIST_AGE_S)
    .with_carry(list.type_name())
}

/// `datums.json?units=metric` of the target station.
pub fn datums_request(target: Target) -> Result<Request, SourceError> {
    let id = station(&target)?.id.clone();
    Ok(Request::new(
        format!("{BASE}/stations/{id}/datums.json?units=metric"),
        Resource::CoopsDatums,
        target,
    )
    .max_age_hint(STATION_AGE_S))
}

/// The tide `harcon.json?units=metric` request, with the datum table to combine it with.
pub fn tide_harcon_request(target: Target, datums: &DatumTable) -> Result<Request, SourceError> {
    let id = station(&target)?.id.clone();
    let carry = serde_json::to_string(datums).map_err(parse_error)?;
    Ok(Request::new(
        format!("{BASE}/stations/{id}/harcon.json?units=metric"),
        Resource::CoopsHarmonics,
        target,
    )
    .max_age_hint(STATION_AGE_S)
    .with_carry(carry))
}

/// The follow-up of a [`datums_request`]: parses its response and plans the constituents
/// request, which carries the datum table.
pub fn harcon_follow_up(
    request: &Request,
    response: &Response<'_>,
) -> Result<Request, SourceError> {
    let id = &station(&request.target)?.id;
    let table = parse_datums(id, check_response(request, response)?)?;
    tide_harcon_request(request.target.clone(), &table)
}

/// The current `harcon.json?bin=N` request; the target's station must name its bin.
pub fn current_harcon_request(target: Target) -> Result<Request, SourceError> {
    let st = station(&target)?;
    let bin = st
        .bin
        .ok_or_else(|| SourceError::Unsupported("current constituents need a bin".into()))?;
    let id = st.id.clone();
    Ok(Request::new(
        format!("{BASE}/stations/{id}/harcon.json?bin={bin}"),
        Resource::CoopsHarmonics,
        target,
    )
    .max_age_hint(STATION_AGE_S))
}

/// `tidepredoffsets.json` of a subordinate tide station.
pub fn tide_offsets_request(target: Target) -> Result<Request, SourceError> {
    let id = station(&target)?.id.clone();
    Ok(Request::new(
        format!("{BASE}/stations/{id}/tidepredoffsets.json"),
        Resource::CoopsTideOffsets,
        target,
    )
    .max_age_hint(STATION_AGE_S))
}

/// `{id}_{bin}/currentpredictionoffsets.json` (the bare id path answers 404).
pub fn current_offsets_request(target: Target) -> Result<Request, SourceError> {
    let st = station(&target)?;
    let bin = st.bin.unwrap_or(1);
    let id = st.id.clone();
    Ok(Request::new(
        format!("{BASE}/stations/{id}_{bin}/currentpredictionoffsets.json"),
        Resource::CoopsCurrentOffsets,
        target,
    )
    .max_age_hint(STATION_AGE_S))
}

/// `bins.json?units=metric` of a current meter.
pub fn bins_request(target: Target) -> Result<Request, SourceError> {
    let id = station(&target)?.id.clone();
    Ok(Request::new(
        format!("{BASE}/stations/{id}/bins.json?units=metric"),
        Resource::CoopsBins,
        target,
    )
    .max_age_hint(STATION_AGE_S))
}

// ----------------------------------------------------------------------------------------
// Entry point

/// Parses a response to one of the metadata requests built here.
pub fn parse(request: &Request, response: &Response<'_>) -> Result<Metadata, SourceError> {
    let body = check_response(request, response)?;
    match request.resource {
        Resource::CoopsStations => {
            let list = request
                .carry()
                .and_then(StationList::from_type_name)
                .ok_or_else(|| SourceError::Parse("station list request lost its type".into()))?;
            parse_station_list(list, body).map(Metadata::Stations)
        }
        Resource::CoopsDatums => {
            parse_datums(&station(&request.target)?.id, body).map(Metadata::Datums)
        }
        Resource::CoopsHarmonics => {
            let st = station(&request.target)?;
            if is_current(st.kind) {
                parse_current_harmonics(&st.id, st.bin, body).map(Metadata::CurrentHarmonics)
            } else {
                let carried = request.carry().ok_or_else(|| {
                    SourceError::Parse("constituents without a datum table".into())
                })?;
                let table: DatumTable = serde_json::from_str(carried).map_err(parse_error)?;
                let datum = request.target.datum.unwrap_or(VerticalDatum::Mllw);
                parse_tide_harmonics(&st.id, body, &table, datum).map(Metadata::TideHarmonics)
            }
        }
        Resource::CoopsTideOffsets => parse_tide_offsets(body).map(Metadata::TideOffsets),
        Resource::CoopsCurrentOffsets => {
            parse_current_offsets(station(&request.target)?.bin, body).map(Metadata::CurrentOffsets)
        }
        Resource::CoopsBins => parse_bins(body).map(Metadata::Bins),
        other => Err(SourceError::Unsupported(format!(
            "{other:?} is not a metadata resource"
        ))),
    }
}

// ----------------------------------------------------------------------------------------
// Parsers

fn parse_error(e: impl std::fmt::Display) -> SourceError {
    SourceError::Parse(e.to_string())
}

fn json<'a, T: Deserialize<'a>>(body: &'a [u8]) -> Result<T, SourceError> {
    serde_json::from_slice(body).map_err(parse_error)
}

/// A length unit as the `units` field spells it.
fn length_unit(units: &str) -> Result<SourceUnit, SourceError> {
    match units.trim() {
        "meters" => Ok(SourceUnit::M),
        "feet" => Ok(SourceUnit::Ft),
        other => Err(parse_error(format!("unknown length units {other:?}"))),
    }
}

fn metres(value: f32, unit: SourceUnit) -> f32 {
    match unit.to_canonical(value) {
        Canonical::Length(m) => m,
        _ => value,
    }
}

#[derive(Deserialize)]
struct ListBody {
    stations: Vec<ListedStation>,
}

#[derive(Deserialize)]
struct ListedStation {
    id: String,
    name: String,
    lat: f64,
    lng: f64,
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    reference_id: Option<String>,
    #[serde(default)]
    currbin: Option<u16>,
    #[serde(default)]
    tidal: Option<bool>,
}

/// Station candidates from a station list body. Current prediction stations come once per bin.
pub fn parse_station_list(
    list: StationList,
    body: &[u8],
) -> Result<Vec<StationCandidate>, SourceError> {
    let listed: ListBody = json(body)?;
    let mut out = Vec::with_capacity(listed.stations.len());
    for s in listed.stations {
        let type_code = s.kind.as_deref().unwrap_or("");
        let reference_id = s.reference_id.filter(|r| !r.is_empty());
        let (kind, measurement, bin) = match list {
            StationList::TidePredictions => match type_code {
                "R" => (StationKind::TideHarmonic, Measurement::Tide, None),
                "S" => (StationKind::TideSubordinate, Measurement::Tide, None),
                _ => continue,
            },
            StationList::CurrentPredictions => {
                let kind = match type_code {
                    "H" => StationKind::CurrentHarmonic,
                    "S" => StationKind::CurrentSubordinate,
                    "W" => StationKind::CurrentWeakVariable,
                    _ => continue,
                };
                (kind, Measurement::TidalCurrent, s.currbin)
            }
            StationList::WaterLevels => {
                let measurement = if s.tidal == Some(false) {
                    Measurement::LakeLevel
                } else {
                    Measurement::Tide
                };
                (StationKind::WaterLevel, measurement, None)
            }
            StationList::Currents => (
                StationKind::PortsCurrentMeter,
                Measurement::TidalCurrent,
                None,
            ),
        };
        out.push(StationCandidate {
            station: StationRef {
                source: SourceId::COOPS,
                id: s.id,
                name: s.name,
                kind,
                latitude: s.lat,
                longitude: s.lng,
                bin,
                reference_id,
            },
            offers: BTreeMap::from([(
                measurement,
                Coverage {
                    last_value_at: None,
                    primary: true,
                },
            )]),
        });
    }
    Ok(out)
}

#[derive(Deserialize)]
struct DatumsBody {
    units: Option<String>,
    datums: Option<Vec<DatumRow>>,
}

#[derive(Deserialize)]
struct DatumRow {
    name: String,
    value: Option<f32>,
}

/// The datum table of `station_id` from a `datums.json` body (metres above `STND`).
pub fn parse_datums(station_id: &str, body: &[u8]) -> Result<DatumTable, SourceError> {
    let parsed: DatumsBody = json(body)?;
    let unit = length_unit(parsed.units.as_deref().unwrap_or("meters"))?;
    let rows = parsed
        .datums
        .filter(|r| !r.is_empty())
        .ok_or_else(|| SourceError::NoData(Some("the station lists no datums".into())))?;
    let values = rows
        .into_iter()
        .filter_map(|r| r.value.map(|v| (r.name, metres(v, unit))))
        .collect();
    Ok(DatumTable::new(station_id, values))
}

#[derive(Deserialize)]
struct TideHarconBody {
    units: Option<String>,
    #[serde(default, rename = "HarmonicConstituents")]
    constituents: Vec<TideHarconRow>,
}

#[derive(Deserialize)]
struct TideHarconRow {
    name: String,
    amplitude: f32,
    #[serde(rename = "phase_GMT")]
    phase_gmt: f32,
}

/// Tide harmonics from a `harcon.json` body and the station's datum table. Amplitudes are
/// converted to metres whatever the response's units; `Z0` is `MSL - datum`.
pub fn parse_tide_harmonics(
    station_id: &str,
    body: &[u8],
    table: &DatumTable,
    datum: VerticalDatum,
) -> Result<TideHarmonics, SourceError> {
    let parsed: TideHarconBody = json(body)?;
    let unit = length_unit(parsed.units.as_deref().unwrap_or("feet"))?;
    let constituents = parsed
        .constituents
        .into_iter()
        .map(|c| HarmonicConstituent {
            name: c.name,
            amplitude_m: metres(c.amplitude, unit),
            phase_gmt_deg: c.phase_gmt,
        })
        .collect();
    TideHarmonics::from_datum_table(station_id, constituents, table, datum).map_err(|e| match e {
        TideError::NoConstituents(_) => SourceError::Unsupported(e.to_string()),
        _ => SourceError::NoData(Some(e.to_string())),
    })
}

#[derive(Deserialize)]
struct CurrentHarconBody {
    units: Option<String>,
    #[serde(default, rename = "HarmonicConstituents")]
    constituents: Vec<CurrentHarconRow>,
}

#[derive(Deserialize)]
struct CurrentHarconRow {
    #[serde(rename = "binNbr")]
    bin: u16,
    #[serde(rename = "binDepth")]
    depth: Option<f32>,
    #[serde(rename = "constituentName")]
    name: String,
    #[serde(rename = "majorAmplitude")]
    amplitude: f64,
    #[serde(rename = "majorPhaseGMT")]
    phase_gmt: f64,
    #[serde(rename = "majorMeanSpeed")]
    mean: f64,
}

/// Current harmonics of one bin. With `bin` the rows of that bin are taken (the other bins of
/// an all-bins response are ignored); without it the response must hold exactly one bin.
/// Only the default units (metres, cm/s) are accepted. Flood and ebb directions are not in
/// the constituents and are left `None`.
pub fn parse_current_harmonics(
    station_id: &str,
    bin: Option<u16>,
    body: &[u8],
) -> Result<CurrentHarmonics, SourceError> {
    let parsed: CurrentHarconBody = json(body)?;
    let units = parsed.units.as_deref().unwrap_or("");
    if units.trim() != "meters, centimeters/second" {
        return Err(parse_error(format!(
            "current constituents in unexpected units {units:?}; request the default"
        )));
    }
    let mut rows = parsed.constituents;
    if let Some(bin) = bin {
        rows.retain(|r| r.bin == bin);
        if rows.is_empty() {
            return Err(SourceError::NotFound(Some(format!(
                "no constituents for bin {bin}"
            ))));
        }
    }
    let Some(first) = rows.first() else {
        return Err(SourceError::NoData(Some("no constituents".into())));
    };
    let (bin, depth, mean) = (first.bin, first.depth, first.mean);
    if rows.iter().any(|r| r.bin != bin) {
        return Err(parse_error("constituents of several bins; request one bin"));
    }
    Ok(CurrentHarmonics {
        station: station_id.to_owned(),
        bin,
        depth_m: depth,
        mean_major_cms: mean,
        constituents: rows
            .into_iter()
            .map(|r| CurrentConstituent {
                name: r.name,
                major_amplitude_cms: r.amplitude,
                major_phase_gmt_deg: r.phase_gmt,
            })
            .collect(),
        flood_dir_deg: None,
        ebb_dir_deg: None,
    })
}

#[derive(Deserialize)]
struct TideOffsetsBody {
    #[serde(default, rename = "refStationId")]
    reference: Option<String>,
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(rename = "heightOffsetHighTide")]
    height_high: Option<f32>,
    #[serde(rename = "heightOffsetLowTide")]
    height_low: Option<f32>,
    #[serde(rename = "timeOffsetHighTide")]
    time_high: Option<i32>,
    #[serde(rename = "timeOffsetLowTide")]
    time_low: Option<i32>,
    #[serde(default, rename = "heightAdjustedType")]
    adjusted: Option<String>,
}

/// Offsets of a subordinate tide station; `None` for a reference station (empty reference).
pub fn parse_tide_offsets(body: &[u8]) -> Result<Option<SubordinateTideOffsets>, SourceError> {
    let p: TideOffsetsBody = json(body)?;
    let reference = p.reference.clone().unwrap_or_default();
    if reference.is_empty() || p.kind.as_deref() == Some("R") {
        return Ok(None);
    }
    let missing = || parse_error("subordinate offsets with missing values");
    let adjust = |value: Option<f32>| -> Result<HeightAdjustment, SourceError> {
        let v = value.ok_or_else(missing)?;
        match p.adjusted.as_deref() {
            Some("R") => Ok(HeightAdjustment::Ratio(v)),
            Some("F") => Ok(HeightAdjustment::Add(metres(v, ADDITIVE_OFFSET_UNIT))),
            other => Err(parse_error(format!("unknown heightAdjustedType {other:?}"))),
        }
    };
    Ok(Some(SubordinateTideOffsets {
        reference_id: reference,
        high_time_min: p.time_high.ok_or_else(missing)?,
        low_time_min: p.time_low.ok_or_else(missing)?,
        high_height: adjust(p.height_high)?,
        low_height: adjust(p.height_low)?,
    }))
}

#[derive(Deserialize)]
struct CurrentOffsetsBody {
    id: String,
    #[serde(rename = "refStationId")]
    reference: String,
    #[serde(rename = "refStationBin")]
    reference_bin: u16,
    #[serde(rename = "meanFloodDir")]
    flood: Option<f32>,
    #[serde(rename = "meanEbbDir")]
    ebb: Option<f32>,
    #[serde(rename = "mfcTimeAdjMin")]
    mfc_time: i32,
    #[serde(rename = "mecTimeAdjMin")]
    mec_time: i32,
    #[serde(rename = "sbfTimeAdjMin")]
    sbf_time: i32,
    #[serde(rename = "sbeTimeAdjMin")]
    sbe_time: i32,
    #[serde(rename = "mfcAmpAdj")]
    mfc_amp: f64,
    #[serde(rename = "mecAmpAdj")]
    mec_amp: f64,
}

/// Offsets of a subordinate current station; `bin` is the station's own bin from the URL.
pub fn parse_current_offsets(
    bin: Option<u16>,
    body: &[u8],
) -> Result<SubordinateCurrentOffsets, SourceError> {
    let p: CurrentOffsetsBody = json(body)?;
    Ok(SubordinateCurrentOffsets {
        station: p.id,
        bin,
        reference_id: p.reference,
        reference_bin: p.reference_bin,
        mfc_time_adj_min: p.mfc_time,
        mec_time_adj_min: p.mec_time,
        sbf_time_adj_min: p.sbf_time,
        sbe_time_adj_min: p.sbe_time,
        mfc_amp_adj: p.mfc_amp,
        mec_amp_adj: p.mec_amp,
        flood_dir_deg: p.flood,
        ebb_dir_deg: p.ebb,
    })
}

#[derive(Deserialize)]
struct BinsBody {
    units: Option<String>,
    #[serde(default)]
    bins: Vec<BinRow>,
}

#[derive(Deserialize)]
struct BinRow {
    num: u16,
    depth: f32,
}

/// The bins of a current meter, depths in metres.
pub fn parse_bins(body: &[u8]) -> Result<Vec<Bin>, SourceError> {
    let p: BinsBody = json(body)?;
    let unit = length_unit(p.units.as_deref().unwrap_or("meters"))?;
    Ok(p.bins
        .into_iter()
        .map(|b| Bin {
            number: b.num,
            depth_m: metres(b.depth, unit),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use underbite_core::model::Measurement;

    use super::*;
    use crate::coops::datagetter::testing::{request, station_ref, target};
    use crate::testutil::fixture_bytes;

    fn fx(name: &str) -> Vec<u8> {
        fixture_bytes("coops", name)
    }

    fn tgt(id: &str, kind: StationKind, bin: Option<u16>) -> Target {
        target(station_ref(id, kind, bin), Measurement::Tide, 0, 86_400)
    }

    fn close(a: f32, b: f32, tol: f32) {
        assert!((a - b).abs() <= tol, "{a} vs {b}");
    }

    fn battery_datums() -> DatumTable {
        parse_datums("8518750", &fx("coops-mdapi-8518750-datums-metric.json")).unwrap()
    }

    #[test]
    fn datums_in_metres_and_feet() {
        let m = battery_datums();
        close(m.z0(VerticalDatum::Mllw).unwrap(), 0.783, 1e-4);
        let ft = parse_datums("8518750", &fx("coops-mdapi-8518750-datums.json")).unwrap();
        close(ft.z0(VerticalDatum::Mllw).unwrap(), 0.783, 0.002);
        assert_eq!(m.get("STND"), Some(0.0));
    }

    #[test]
    fn subordinate_datums_are_no_data() {
        let err = parse_datums("8518919", &fx("coops-mdapi-8518919-datums.json")).unwrap_err();
        assert!(matches!(err, SourceError::NoData(_)), "{err}");
    }

    #[test]
    fn battery_harmonics() {
        let table = battery_datums();
        let h = parse_tide_harmonics(
            "8518750",
            &fx("coops-mdapi-8518750-harcon-metric.json"),
            &table,
            VerticalDatum::Mllw,
        )
        .unwrap();
        assert_eq!(h.constituents.len(), 37);
        close(h.z0_m, 0.783, 1e-4);
        let m2 = &h.constituents[0];
        assert_eq!(m2.name, "M2");
        close(m2.amplitude_m, 0.671, 1e-5);
        close(m2.phase_gmt_deg, 18.2, 1e-4);
        let feet = parse_tide_harmonics(
            "8518750",
            &fx("coops-mdapi-8518750-harcon.json"),
            &table,
            VerticalDatum::Mllw,
        )
        .unwrap();
        for (a, b) in h.constituents.iter().zip(&feet.constituents) {
            close(a.amplitude_m, b.amplitude_m, 0.0005);
        }
    }

    #[test]
    fn pensacola_harmonics() {
        let table = parse_datums("8729840", &fx("coops-mdapi-8729840-datums-metric.json")).unwrap();
        let h = parse_tide_harmonics(
            "8729840",
            &fx("coops-mdapi-8729840-harcon-metric.json"),
            &table,
            VerticalDatum::Mllw,
        )
        .unwrap();
        assert_eq!(h.constituents.len(), 37);
        assert!(h.z0_m > 0.0);
    }

    #[test]
    fn subordinate_station_has_no_harmonics() {
        let table = battery_datums();
        let err = parse_tide_harmonics(
            "8518919",
            &fx("coops-mdapi-8518919-harcon.json"),
            &table,
            VerticalDatum::Mllw,
        )
        .unwrap_err();
        assert!(matches!(err, SourceError::Unsupported(_)), "{err}");
    }

    #[test]
    fn harmonics_chain_through_carry() {
        let t = tgt("8518750", StationKind::TideHarmonic, None);
        let first = datums_request(t).unwrap();
        assert_eq!(
            first.url,
            format!("{BASE}/stations/8518750/datums.json?units=metric")
        );
        let body = fx("coops-mdapi-8518750-datums-metric.json");
        let second = harcon_follow_up(&first, &Response::new(200, &body)).unwrap();
        assert!(
            second
                .url
                .ends_with("/stations/8518750/harcon.json?units=metric")
        );
        assert!(second.carry().is_some());
        let body = fx("coops-mdapi-8518750-harcon-metric.json");
        let Metadata::TideHarmonics(h) = parse(&second, &Response::new(200, &body)).unwrap() else {
            panic!("expected harmonics");
        };
        close(h.z0_m, 0.783, 1e-4);
        assert_eq!(h.datum, VerticalDatum::Mllw);
    }

    #[test]
    fn tarrytown_offsets() {
        let o = parse_tide_offsets(&fx("coops-mdapi-8518919-tidepredoffsets.json"))
            .unwrap()
            .unwrap();
        assert_eq!(o.reference_id, "8518750");
        assert_eq!((o.high_time_min, o.low_time_min), (109, 117));
        assert_eq!(o.high_height, HeightAdjustment::Ratio(0.7));
        assert_eq!(o.low_height, HeightAdjustment::Ratio(0.7));
    }

    #[test]
    fn reference_station_has_no_offsets() {
        assert_eq!(
            parse_tide_offsets(&fx("coops-mdapi-8729840-tidepredoffsets.json")).unwrap(),
            None
        );
    }

    #[test]
    fn additive_offsets_are_taken_as_feet() {
        let body = br#"{"refStationId":"8518750","type":"S","heightOffsetHighTide":1.0,
            "heightOffsetLowTide":-0.5,"timeOffsetHighTide":10,"timeOffsetLowTide":-5,
            "heightAdjustedType":"F"}"#;
        let o = parse_tide_offsets(body).unwrap().unwrap();
        let HeightAdjustment::Add(high) = o.high_height else {
            panic!("expected an addend");
        };
        close(high, 0.3048, 1e-5);
        assert_eq!(o.low_time_min, -5);
    }

    #[test]
    fn current_harmonics_of_nyh1927_bin_13() {
        let body = fx("coops-mdapi-NYH1927-harcon.json");
        let h = parse_current_harmonics("NYH1927", Some(13), &body).unwrap();
        assert_eq!(h.bin, 13);
        assert_eq!(h.constituents.len(), 25);
        assert!((h.mean_major_cms - -20.37).abs() < 1e-9);
        assert_eq!(h.depth_m, Some(1.98));
        assert_eq!(h.flood_dir_deg, None);
    }

    #[test]
    fn mixed_bins_are_rejected() {
        let body = fx("coops-mdapi-NYH1927-harcon.json");
        let err = parse_current_harmonics("NYH1927", None, &body).unwrap_err();
        assert!(matches!(err, SourceError::Parse(_)), "{err}");
        let err = parse_current_harmonics("NYH1927", Some(99), &body).unwrap_err();
        assert!(matches!(err, SourceError::NotFound(_)), "{err}");
    }

    #[test]
    fn english_current_constituents_are_rejected() {
        let body = fx("coops-mdapi-NYH1927-harcon-bin13-english.json");
        assert!(parse_current_harmonics("NYH1927", Some(13), &body).is_err());
    }

    #[test]
    fn narrows_bin_7() {
        let body = fx("coops-mdapi-n03020-harcon-bin7.json");
        let h = parse_current_harmonics("n03020", None, &body).unwrap();
        assert_eq!((h.bin, h.constituents.len()), (7, 29));
        assert!((h.mean_major_cms - -15.38).abs() < 1e-9);
    }

    #[test]
    fn current_harmonics_through_parse() {
        let t = tgt("n03020", StationKind::CurrentHarmonic, Some(7));
        let req = current_harcon_request(t).unwrap();
        assert!(req.url.ends_with("/stations/n03020/harcon.json?bin=7"));
        let body = fx("coops-mdapi-n03020-harcon-bin7.json");
        assert!(matches!(
            parse(&req, &Response::new(200, &body)).unwrap(),
            Metadata::CurrentHarmonics(_)
        ));
        assert!(current_harcon_request(tgt("n03020", StationKind::CurrentHarmonic, None)).is_err());
    }

    #[test]
    fn act3646_offsets() {
        let t = tgt("ACT3646", StationKind::CurrentSubordinate, Some(1));
        let req = current_offsets_request(t).unwrap();
        assert!(
            req.url
                .ends_with("/stations/ACT3646_1/currentpredictionoffsets.json")
        );
        let body = fx("coops-mdapi-ACT3646_1-currentpredictionoffsets.json");
        let Metadata::CurrentOffsets(o) = parse(&req, &Response::new(200, &body)).unwrap() else {
            panic!("expected offsets");
        };
        assert_eq!((o.station.as_str(), o.bin), ("ACT3646", Some(1)));
        assert_eq!((o.reference_id.as_str(), o.reference_bin), ("n03020", 7));
        assert_eq!(
            (
                o.mfc_time_adj_min,
                o.mec_time_adj_min,
                o.sbf_time_adj_min,
                o.sbe_time_adj_min
            ),
            (58, 60, 80, 53)
        );
        assert!((o.mfc_amp_adj - 1.05).abs() < 1e-9 && (o.mec_amp_adj - 1.19).abs() < 1e-9);
        assert_eq!((o.flood_dir_deg, o.ebb_dir_deg), (Some(31.0), Some(205.0)));
    }

    #[test]
    fn bins_in_metres() {
        let bins = parse_bins(&fx("coops-mdapi-NYH1927-bins-metric.json")).unwrap();
        assert_eq!(bins.len(), 14);
        assert_eq!(
            bins[0],
            Bin {
                number: 1,
                depth_m: 13.99
            }
        );
        assert_eq!(
            parse_bins(&fx("coops-mdapi-n03020-bins.json"))
                .unwrap()
                .len(),
            9
        );
    }

    fn list(list: StationList, name: &str) -> Vec<StationCandidate> {
        parse_station_list(list, &fx(name)).unwrap()
    }

    #[test]
    fn tide_prediction_stations() {
        let c = list(
            StationList::TidePredictions,
            "coops-mdapi-stations-tidepredictions-newyork-trimmed.json",
        );
        assert_eq!(c.len(), 90);
        let harmonic = c
            .iter()
            .filter(|c| c.station.kind == StationKind::TideHarmonic);
        assert_eq!(harmonic.count(), 33);
        let sub = c.iter().find(|c| c.station.id == "8516601").unwrap();
        assert_eq!(sub.station.kind, StationKind::TideSubordinate);
        assert_eq!(sub.station.reference_id.as_deref(), Some("8531680"));
        assert!(sub.offers.contains_key(&Measurement::Tide));
    }

    #[test]
    fn current_prediction_stations_come_once_per_bin() {
        let c = list(
            StationList::CurrentPredictions,
            "coops-mdapi-stations-currentpredictions-newyork-trimmed.json",
        );
        assert_eq!(c.len(), 195);
        let bins: Vec<_> = c
            .iter()
            .filter(|c| c.station.id == "NYH1927")
            .map(|c| c.station.bin)
            .collect();
        assert_eq!(bins, [Some(13), Some(8), Some(2)]);
        assert!(c.iter().all(|c| c.station.bin.is_some()));
        assert!(
            c.iter()
                .any(|c| c.station.kind == StationKind::CurrentWeakVariable)
        );
    }

    #[test]
    fn water_level_and_meter_lists() {
        let c = list(
            StationList::WaterLevels,
            "coops-mdapi-stations-waterlevels-newyork-trimmed.json",
        );
        assert_eq!(c.len(), 3);
        assert!(c.iter().all(|c| c.station.kind == StationKind::WaterLevel));
        let m = list(StationList::Currents, "coops-mdapi-stations-currents.json");
        assert_eq!(m.len(), 85);
        assert!(m.iter().any(|c| c.station.id == "n03020"));
        assert!(
            m.iter()
                .all(|c| c.station.kind == StationKind::PortsCurrentMeter)
        );
    }

    #[test]
    fn station_list_requests_are_cached_long() {
        let req = stations_request(
            StationList::CurrentPredictions,
            tgt("x", StationKind::TideHarmonic, None),
        );
        assert_eq!(
            req.url,
            format!("{BASE}/stations.json?type=currentpredictions")
        );
        assert_eq!(req.cache.max_age, Some(LIST_AGE_S));
        let body = fx("coops-mdapi-stations-currents.json");
        let req = stations_request(
            StationList::Currents,
            tgt("x", StationKind::TideHarmonic, None),
        );
        let Metadata::Stations(s) = parse(&req, &Response::new(200, &body)).unwrap() else {
            panic!("expected stations");
        };
        assert_eq!(s.len(), 85);
    }

    #[test]
    fn errors_pass_through_check_response() {
        let req = request(
            Resource::CoopsBins,
            station_ref("x", StationKind::PortsCurrentMeter, None),
            Measurement::TidalCurrent,
        );
        let err = parse(&req, &Response::new(404, b"Not Found")).unwrap_err();
        assert!(matches!(err, SourceError::NotFound(_)), "{err}");
    }
}
