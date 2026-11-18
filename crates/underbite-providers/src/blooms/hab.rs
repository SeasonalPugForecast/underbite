//! NOAA NCCOS harmful algal bloom forecasts: the Gulf respiratory irritation forecast.
//!
//! Only the Gulf of Mexico forecast is machine readable: a zip with one CSV per forecast hour
//! (`RIFv2_modelrun_<run>_forecast_valid_<NN>hrs.csv`). [`parse_gulf_zip`] reads it into the
//! [`HabForecast`] of the nearest listed beach within [`MAX_BEACH_DISTANCE_M`], for the forecast
//! hour nearest `now`. Every other region publishes a PDF bulletin or a viewer, so those are
//! links: [`region_links`] lists them and [`links_near`] picks those whose area contains a
//! point.
//!
//! The CSV differs from its own data dictionary (a `Not Present` cell category, `fwri_sftp` as a
//! source, wind directions as floats), so a row is read by column name, unknown categories are
//! tolerated and a row that cannot be read is skipped rather than failing the file.
//!
//! [`HabGulf`] is the source, for a [`Place::Point`] in the Gulf forecast's area: the zip (which
//! names no place, so the point never leaves the device) parsed into the bloom block with the
//! region's link.

use std::collections::BTreeSet;
use std::io::{Cursor, Read};

use underbite_core::model::{ExternalLink, HabForecast, HabLevel, Measurement, Requirements};
use underbite_core::provenance::{Attribution, Provenance, Quality, Relation, SourceId, ValueKind};
use underbite_core::time::{UnixTime, parse_rfc3339};

use crate::request::{Coordinates, Place, Request, Resource, Response, Target};
use crate::source::{FetchContext, FragmentValue, Fragments, Source, SourceError, check_response};

/// The measurements the source supplies: the bloom block.
pub const MEASUREMENTS: [Measurement; 1] = [Measurement::Bloom];

/// How long a response may be reused, seconds: the model runs a few times a day.
pub const MAX_AGE_S: u32 = 3_600;

/// The current model run as a zip. It has no key and no history.
pub const GULF_ZIP_URL: &str = "https://nccospublicstor.blob.core.windows.net/hab-data/no_Explorer/rif_model/csv/RIFv2_currentmodelrun.zip";

/// A beach further than this from the spot says nothing about the spot.
pub const MAX_BEACH_DISTANCE_M: f32 = 10_000.0;

/// The forecast hours are 3 hours apart; a run whose last hour is this long past is stale.
const STALE_AFTER_S: i64 = 4 * 3_600;

/// Most bytes the zip may have.
const MAX_ZIP_BYTES: usize = 4 * 1024 * 1024;
/// Most entries the zip may list.
const MAX_ENTRIES: usize = 64;
/// Most bytes one forecast file may inflate to.
const MAX_ENTRY_BYTES: u64 = 2 * 1024 * 1024;
/// Most bytes all forecast files together may inflate to.
const MAX_TOTAL_BYTES: u64 = 8 * 1024 * 1024;

/// The credit every display of the forecast shows.
pub fn attribution() -> Attribution {
    Attribution {
        source: SourceId::NOAA_HAB,
        credit: "NOAA National Centers for Coastal Ocean Science (NCCOS) Gulf of Mexico \
                 respiratory irritation forecast. Cell counts: Florida Fish and Wildlife \
                 Research Institute (FWRI), HABscope, Pinellas County and the Sanibel-Captiva \
                 Conservation Foundation; wind: NOAA Meteorological Development Laboratory's \
                 National Digital Forecast Database."
            .into(),
        url: Some(
            "https://coastalscience.noaa.gov/science-areas/habs/hab-forecasts/gulf-coast".into(),
        ),
    }
}

/// A region NOAA publishes a bloom forecast for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HabRegion {
    /// Lake Erie (a bulletin in season).
    LakeErie,
    /// The Gulf of Maine (a dashboard while a bloom is present).
    GulfOfMaine,
    /// Washington and Oregon (NANOOS).
    PacificNorthwest,
    /// The Florida Gulf coast (the respiratory forecast).
    Florida,
    /// The Texas coast.
    Texas,
}

/// A region's link and the box that contains it: south, west, north, east in degrees.
struct Region {
    id: HabRegion,
    label: &'static str,
    url: &'static str,
    bounds: (f64, f64, f64, f64),
}

const REGIONS: [Region; 5] = [
    Region {
        id: HabRegion::LakeErie,
        label: "NOAA Lake Erie harmful algal bloom forecast",
        url: "https://coastalscience.noaa.gov/science-areas/habs/hab-forecasts/lake-erie/",
        bounds: (41.2, -83.7, 42.95, -78.8),
    },
    Region {
        id: HabRegion::GulfOfMaine,
        label: "NOAA Gulf of Maine harmful algal bloom forecast",
        url: "https://coastalscience.noaa.gov/science-areas/habs/hab-forecasts/gulf-of-maine/nowcast-forecast-simulation/",
        bounds: (41.5, -71.2, 45.3, -66.5),
    },
    Region {
        id: HabRegion::PacificNorthwest,
        label: "NANOOS Pacific Northwest harmful algal bloom forecast",
        url: "https://www.nanoos.org/products/habs/forecasts/home.php",
        bounds: (42.0, -125.5, 49.5, -122.0),
    },
    Region {
        id: HabRegion::Florida,
        label: "NOAA Florida respiratory forecast",
        url: "https://experience.arcgis.com/experience/1ef2c961393948f3861d5ec9831250ee",
        bounds: (24.3, -87.7, 31.1, -79.8),
    },
    Region {
        id: HabRegion::Texas,
        label: "NOAA Texas respiratory forecast",
        url: "https://experience.arcgis.com/experience/1ef2c961393948f3861d5ec9831250ee",
        bounds: (25.7, -97.9, 30.2, -93.5),
    },
];

fn link(region: &Region) -> ExternalLink {
    ExternalLink {
        label: region.label.into(),
        url: region.url.into(),
    }
}

/// The forecast page of every region, in a fixed order, for a place that lists them all.
pub fn region_links() -> Vec<(HabRegion, ExternalLink)> {
    REGIONS.iter().map(|r| (r.id, link(r))).collect()
}

/// The links of the regions whose area contains `point`; empty for most of the country.
pub fn links_near(point: Coordinates) -> Vec<ExternalLink> {
    REGIONS
        .iter()
        .filter(|r| contains(r, point))
        .map(link)
        .collect()
}

/// Whether `point` is in an area the Gulf forecast covers (the Florida and Texas regions).
pub fn in_gulf_area(point: Coordinates) -> bool {
    REGIONS
        .iter()
        .filter(|r| matches!(r.id, HabRegion::Florida | HabRegion::Texas))
        .any(|r| contains(r, point))
}

fn contains(region: &Region, point: Coordinates) -> bool {
    let (south, west, north, east) = region.bounds;
    (south..=north).contains(&point.latitude) && (west..=east).contains(&point.longitude)
}

/// One forecast row, kept only for the beach nearest the spot.
struct Row {
    location: String,
    level: HabLevel,
    confidence: Option<f32>,
    valid_at: UnixTime,
    run_at: Option<UnixTime>,
    distance_m: f32,
}

fn level(text: &str) -> HabLevel {
    match text.trim().to_ascii_lowercase().as_str() {
        "very low" => HabLevel::VeryLow,
        "low" => HabLevel::Low,
        "moderate" => HabLevel::Moderate,
        "high" => HabLevel::High,
        _ => HabLevel::Unknown,
    }
}

/// The column index of `name`, ignoring case and surrounding space.
fn column(headers: &csv::StringRecord, name: &str) -> Option<usize> {
    headers.iter().position(|h| {
        h.trim()
            .trim_start_matches('\u{feff}')
            .eq_ignore_ascii_case(name)
    })
}

/// The nearest beach of one forecast file within [`MAX_BEACH_DISTANCE_M`]. `Ok(None)` when the
/// file lists none; an error when it lacks the columns the forecast needs.
fn nearest_in_file(csv_bytes: &[u8], spot: Coordinates) -> Result<Option<Row>, SourceError> {
    let mut reader = csv::ReaderBuilder::new()
        .flexible(true)
        .trim(csv::Trim::All)
        .from_reader(csv_bytes);
    let headers = reader
        .headers()
        .map_err(|e| SourceError::Parse(format!("HAB forecast header: {e}")))?
        .clone();
    let need = |name: &str| {
        column(&headers, name)
            .ok_or_else(|| SourceError::Parse(format!("HAB forecast has no {name} column")))
    };
    let forecast = need("Forecast")?;
    let wind_time = need("Wind Time")?;
    let lat = need("Lat")?;
    let lon = need("Lon")?;
    let name = need("Location Name")?;
    let confidence = column(&headers, "Confidence");
    let run = column(&headers, "Model Run Time");

    let mut best: Option<Row> = None;
    for record in reader.records() {
        // A damaged row is skipped; the rest of the file is still good.
        let Ok(record) = record else { continue };
        let number = |i: usize| record.get(i).and_then(|s| s.parse::<f64>().ok());
        let (Some(latitude), Some(longitude)) = (number(lat), number(lon)) else {
            continue;
        };
        let Some(valid_at) = record.get(wind_time).and_then(parse_rfc3339) else {
            continue;
        };
        if !(-90.0..=90.0).contains(&latitude) || !(-180.0..=180.0).contains(&longitude) {
            continue;
        }
        let distance_m = distance_m(spot.latitude, spot.longitude, latitude, longitude);
        if distance_m > MAX_BEACH_DISTANCE_M
            || best.as_ref().is_some_and(|b| b.distance_m <= distance_m)
        {
            continue;
        }
        best = Some(Row {
            location: record.get(name).unwrap_or_default().to_string(),
            level: level(record.get(forecast).unwrap_or_default()),
            confidence: confidence
                .and_then(number)
                .filter(|c| c.is_finite())
                .map(|c| c.clamp(0.0, 1.0) as f32),
            valid_at,
            run_at: run.and_then(|i| record.get(i)).and_then(parse_rfc3339),
            distance_m,
        });
    }
    Ok(best)
}

/// The forecast files of a zip: `(name, bytes)`. Only entries named
/// `*_forecast_valid_*.csv` are inflated, within the size limits; the dictionary and README are
/// never read.
fn forecast_files(zip_bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>, SourceError> {
    if zip_bytes.len() > MAX_ZIP_BYTES {
        return Err(SourceError::Parse("HAB zip is too large".into()));
    }
    let mut archive = zip::ZipArchive::new(Cursor::new(zip_bytes))
        .map_err(|e| SourceError::Parse(format!("not a zip: {e}")))?;
    if archive.len() > MAX_ENTRIES {
        return Err(SourceError::Parse("HAB zip lists too many files".into()));
    }
    let mut files = Vec::new();
    let mut total = 0_u64;
    for index in 0..archive.len() {
        let name = match archive.name_for_index(index) {
            Some(Ok(name)) => name.into_owned(),
            _ => continue,
        };
        let lower = name.to_ascii_lowercase();
        let is_csv = std::path::Path::new(&lower)
            .extension()
            .is_some_and(|ext| ext == "csv");
        if !(is_csv && lower.contains("_forecast_valid_")) {
            continue;
        }
        let entry = archive
            .by_index(index)
            .map_err(|e| SourceError::Parse(format!("zip entry {name}: {e}")))?;
        // The declared size can lie, so the inflated bytes are counted as they come.
        let allowed = MAX_ENTRY_BYTES.min(MAX_TOTAL_BYTES - total);
        let mut bytes = Vec::new();
        entry
            .take(allowed + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| SourceError::Parse(format!("zip entry {name}: {e}")))?;
        if bytes.len() as u64 > allowed {
            return Err(SourceError::Parse(format!(
                "zip entry {name} inflates past the size limit"
            )));
        }
        total += bytes.len() as u64;
        files.push((name, bytes));
    }
    Ok(files)
}

/// Parses the Gulf zip into the forecast for the beach nearest `spot` within
/// [`MAX_BEACH_DISTANCE_M`], valid at the forecast hour nearest `now`.
///
/// # Errors
/// - [`SourceError::NoData`]: no listed beach is within the distance, or no forecast hour lists
///   one.
/// - [`SourceError::Stale`]: every forecast hour is more than four hours before `now`.
/// - [`SourceError::Parse`]: not a zip, over the size limits, no forecast files, or a forecast
///   file without the columns it needs.
pub fn parse_gulf_zip(
    zip_bytes: &[u8],
    spot: Coordinates,
    now: UnixTime,
) -> Result<HabForecast, SourceError> {
    let files = forecast_files(zip_bytes)?;
    if files.is_empty() {
        return Err(SourceError::Parse("HAB zip has no forecast files".into()));
    }
    let mut rows = Vec::new();
    for (_, bytes) in &files {
        if let Some(row) = nearest_in_file(bytes, spot)? {
            rows.push(row);
        }
    }
    let Some(latest) = rows.iter().map(|r| r.valid_at).max() else {
        return Err(SourceError::NoData(Some(
            "no listed beach within 10 km".into(),
        )));
    };
    if now.0 - latest.0 > STALE_AFTER_S {
        return Err(SourceError::Stale { latest });
    }
    // INVARIANT: `rows` is not empty, since `latest` was found in it.
    let row = rows
        .into_iter()
        .min_by_key(|r| ((r.valid_at.0 - now.0).abs(), r.valid_at))
        .ok_or(SourceError::NoData(None))?;
    Ok(HabForecast {
        provenance: Provenance {
            source: SourceId::NOAA_HAB,
            station: None,
            distance_m: Some(row.distance_m),
            relation: Relation::Nearby,
            observed_at: row.run_at.unwrap_or(row.valid_at),
            kind: ValueKind::Modelled,
            quality: Quality::Provisional,
        },
        location: row.location,
        level: row.level,
        confidence: row.confidence,
        valid_at: row.valid_at,
        distance_m: row.distance_m,
    })
}

/// Whether a box (south, west, north, east in degrees; `west <= east`) overlaps the Florida or
/// Texas forecast area.
pub fn overlaps_gulf_area(south: f64, west: f64, north: f64, east: f64) -> bool {
    REGIONS
        .iter()
        .filter(|r| matches!(r.id, HabRegion::Florida | HabRegion::Texas))
        .any(|r| {
            let (s, w, n, e) = r.bounds;
            south <= n && north >= s && west <= e && east >= w
        })
}

/// One beach of the forecast, at the forecast hour nearest a given time.
#[derive(Clone, Debug, PartialEq)]
pub struct HabBeach {
    /// The beach's name as the source writes it.
    pub location: String,
    pub latitude: f64,
    pub longitude: f64,
    pub level: HabLevel,
    /// The source's weighting, 0 to 1.
    pub confidence: Option<f32>,
    /// The time the forecast is valid for.
    pub valid_at: UnixTime,
    /// The model run's time, when the file gives it.
    pub run_at: Option<UnixTime>,
}

/// Every beach of the zip for which `inside(latitude, longitude)` holds, each at the forecast
/// hour nearest `now`, in name order. The file is region-wide, so the view is applied here and
/// nothing about it need leave the device.
///
/// # Errors
/// - [`SourceError::Stale`]: every forecast hour is more than four hours before `now`.
/// - [`SourceError::Parse`]: not a zip, over the size limits, no forecast files, or a forecast
///   file without the columns it needs.
pub fn parse_gulf_beaches(
    zip_bytes: &[u8],
    now: UnixTime,
    inside: impl Fn(f64, f64) -> bool,
) -> Result<Vec<HabBeach>, SourceError> {
    let files = forecast_files(zip_bytes)?;
    if files.is_empty() {
        return Err(SourceError::Parse("HAB zip has no forecast files".into()));
    }
    let mut best: std::collections::BTreeMap<(String, i64, i64), HabBeach> =
        std::collections::BTreeMap::new();
    let mut latest: Option<UnixTime> = None;
    for (_, bytes) in &files {
        let mut reader = csv::ReaderBuilder::new()
            .flexible(true)
            .trim(csv::Trim::All)
            .from_reader(bytes.as_slice());
        let headers = reader
            .headers()
            .map_err(|e| SourceError::Parse(format!("HAB forecast header: {e}")))?
            .clone();
        let need = |name: &str| {
            column(&headers, name)
                .ok_or_else(|| SourceError::Parse(format!("HAB forecast has no {name} column")))
        };
        let (forecast, wind_time) = (need("Forecast")?, need("Wind Time")?);
        let (lat, lon, name) = (need("Lat")?, need("Lon")?, need("Location Name")?);
        let confidence = column(&headers, "Confidence");
        let run = column(&headers, "Model Run Time");
        for record in reader.records() {
            let Ok(record) = record else { continue };
            let number = |i: usize| record.get(i).and_then(|s| s.parse::<f64>().ok());
            let (Some(latitude), Some(longitude)) = (number(lat), number(lon)) else {
                continue;
            };
            let Some(valid_at) = record.get(wind_time).and_then(parse_rfc3339) else {
                continue;
            };
            if !(-90.0..=90.0).contains(&latitude) || !(-180.0..=180.0).contains(&longitude) {
                continue;
            }
            latest = latest.max(Some(valid_at));
            if !inside(latitude, longitude) {
                continue;
            }
            let beach = HabBeach {
                location: record.get(name).unwrap_or_default().to_string(),
                latitude,
                longitude,
                level: level(record.get(forecast).unwrap_or_default()),
                confidence: confidence
                    .and_then(number)
                    .filter(|c| c.is_finite())
                    .map(|c| c.clamp(0.0, 1.0) as f32),
                valid_at,
                run_at: run.and_then(|i| record.get(i)).and_then(parse_rfc3339),
            };
            // Beaches are told apart by name and place (thousandths of a degree).
            let key = (
                beach.location.clone(),
                (latitude * 1_000.0).round() as i64,
                (longitude * 1_000.0).round() as i64,
            );
            let nearer = best
                .get(&key)
                .is_none_or(|b| (valid_at.0 - now.0).abs() < (b.valid_at.0 - now.0).abs());
            if nearer {
                best.insert(key, beach);
            }
        }
    }
    if let Some(latest) = latest.filter(|l| now.0 - l.0 > STALE_AFTER_S) {
        return Err(SourceError::Stale { latest });
    }
    Ok(best.into_values().collect())
}

/// The NOAA Gulf respiratory irritation forecast as a source of a beach's bloom forecast. It holds
/// no settings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HabGulf;

/// The point of a target in the Gulf forecast's area.
fn gulf_point(target: &Target) -> Result<Coordinates, SourceError> {
    let point = match target.place {
        Place::Point(p) => p.coordinates(),
        Place::Station { .. } | Place::Zone(_) => {
            return Err(SourceError::Unsupported(
                "the Gulf bloom forecast needs a point".into(),
            ));
        }
    };
    if !in_gulf_area(point) {
        return Err(SourceError::Unsupported(
            "the Gulf bloom forecast covers the Florida and Texas coasts only".into(),
        ));
    }
    Ok(point)
}

impl Source for HabGulf {
    fn id(&self) -> SourceId {
        SourceId::NOAA_HAB
    }

    fn attribution(&self) -> Attribution {
        attribution()
    }

    fn measurements(&self) -> &[Measurement] {
        &MEASUREMENTS
    }

    /// The current model run, for a point in the forecast's area.
    fn plan(
        &self,
        target: &Target,
        requirements: &Requirements,
        _ctx: &FetchContext,
    ) -> Result<Vec<Request>, SourceError> {
        let wanted = |m: &Measurement| {
            target.measurements.contains(m)
                && (requirements.measurements.is_empty() || requirements.measurements.contains(m))
        };
        if !MEASUREMENTS.iter().any(wanted) {
            return Err(SourceError::Unsupported(format!(
                "the Gulf bloom forecast offers none of {:?}",
                target.measurements
            )));
        }
        gulf_point(target)?;
        let target = Target {
            measurements: BTreeSet::from(MEASUREMENTS),
            ..target.clone()
        };
        Ok(vec![
            Request::new(GULF_ZIP_URL.to_string(), Resource::HabGulfZip, target)
                .max_age_hint(MAX_AGE_S),
        ])
    }

    /// The nearest beach's forecast and the region's link; no beach in range or an old run is
    /// an error, never "no bloom".
    fn parse(
        &self,
        request: &Request,
        response: &Response<'_>,
        ctx: &FetchContext,
        out: &mut Fragments,
    ) -> Result<(), SourceError> {
        let spot = gulf_point(&request.target)?;
        let forecast = parse_gulf_zip(check_response(request, response)?, spot, ctx.now)?;
        if let Some(info) = super::bloom_info(spot, None, Some(forecast), links_near(spot)) {
            out.push(Measurement::Bloom, FragmentValue::Bloom(info))?;
        }
        Ok(())
    }
}

/// Great-circle distance in metres between two points.
fn distance_m(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f32 {
    const EARTH_RADIUS_M: f64 = 6_371_008.8;
    let (p1, p2) = (lat1.to_radians(), lat2.to_radians());
    let dlat = p2 - p1;
    let dlon = (lon2 - lon1).to_radians();
    let a = (dlat / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dlon / 2.0).sin().powi(2);
    (2.0 * EARTH_RADIUS_M * a.sqrt().min(1.0).asin()) as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::fixture_bytes;

    /// Clearwater Beach, a beach the recorded file lists.
    const CLEARWATER: Coordinates = Coordinates {
        latitude: 27.978,
        longitude: -82.83,
    };
    /// Longboat Pass Boat Ramp, the first row of the recorded file.
    const LONGBOAT: Coordinates = Coordinates {
        latitude: 27.448,
        longitude: -82.688,
    };
    const ZIP: &str = "noaa-hab-gulf-rif-currentmodelrun-2026-11-17.zip";

    fn at(s: &str) -> UnixTime {
        parse_rfc3339(s).expect("a valid time")
    }

    fn zip() -> Vec<u8> {
        fixture_bytes("hab", ZIP)
    }

    /// A zip of stored (uncompressed) entries, built by hand since the zip crate here only reads.
    fn stored_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        fn crc32(data: &[u8]) -> u32 {
            let mut crc = !0_u32;
            for &b in data {
                crc ^= u32::from(b);
                for _ in 0..8 {
                    crc = if crc & 1 == 1 {
                        (crc >> 1) ^ 0xEDB8_8320
                    } else {
                        crc >> 1
                    };
                }
            }
            !crc
        }
        let mut out = Vec::new();
        let mut central = Vec::new();
        for (name, data) in entries {
            let offset = out.len() as u32;
            let crc = crc32(data);
            let size = data.len() as u32;
            let name = name.as_bytes();
            let name_len = name.len() as u16;
            for (target, signature) in [(&mut out, 0x0403_4b50_u32), (&mut central, 0x0201_4b50)] {
                target.extend(signature.to_le_bytes());
                if signature == 0x0201_4b50 {
                    target.extend(20_u16.to_le_bytes()); // made by
                }
                target.extend(20_u16.to_le_bytes()); // needed
                target.extend(0_u16.to_le_bytes()); // flags
                target.extend(0_u16.to_le_bytes()); // stored
                target.extend(0_u16.to_le_bytes()); // time
                target.extend(0x21_u16.to_le_bytes()); // date
                target.extend(crc.to_le_bytes());
                target.extend(size.to_le_bytes());
                target.extend(size.to_le_bytes());
                target.extend(name_len.to_le_bytes());
                target.extend(0_u16.to_le_bytes()); // extra
                if signature == 0x0201_4b50 {
                    target.extend([0_u8; 6]); // comment length, disk, internal attributes
                    target.extend(0_u32.to_le_bytes()); // external attrs
                    target.extend(offset.to_le_bytes());
                }
                target.extend(name);
            }
            out.extend(*data);
        }
        let central_offset = out.len() as u32;
        let central_size = central.len() as u32;
        out.extend(central);
        out.extend(0x0605_4b50_u32.to_le_bytes());
        out.extend([0_u8; 4]); // disks
        out.extend((entries.len() as u16).to_le_bytes());
        out.extend((entries.len() as u16).to_le_bytes());
        out.extend(central_size.to_le_bytes());
        out.extend(central_offset.to_le_bytes());
        out.extend(0_u16.to_le_bytes()); // comment
        out
    }

    const HEADER: &str = "Forecast,Sample Used,Cell Count Category,Sample Time,Wind Speed,Wind Dir,Wind Category,Wind Time,Shoreline Angle,Lat,Lon,Confidence,Location Name,Model Run Time,Cell Count Source\n";

    fn row(level: &str, wind_time: &str, lat: f64, lon: f64, name: &str) -> String {
        format!(
            "{level},0,Not Present,2026-11-16T16:25:00+00:00,18.0,67.5,Offshore,{wind_time},75.3,{lat},{lon},0.9,{name},2026-11-17T03:00:00-05:00,fwri_sftp\n"
        )
    }

    #[test]
    fn every_beach_in_a_box_is_listed_at_the_nearest_hour() {
        let now = at("2026-11-17T09:00:00Z");
        let beaches = parse_gulf_beaches(&zip(), now, |lat, lon| {
            (27.8..=28.2).contains(&lat) && (-83.0..=-82.6).contains(&lon)
        })
        .expect("beaches");
        let clearwater: Vec<_> = beaches
            .iter()
            .filter(|b| b.location == "Clearwater Beach")
            .collect();
        assert_eq!(clearwater.len(), 1);
        assert_eq!(clearwater[0].valid_at, at("2026-11-17T08:00:00Z"));
        let none = parse_gulf_beaches(&zip(), now, |_, _| false).expect("empty");
        assert_eq!(none.len(), 0);
        assert!(matches!(
            parse_gulf_beaches(&zip(), at("2026-11-20T00:00:00Z"), |_, _| true),
            Err(SourceError::Stale { .. })
        ));
        assert!(overlaps_gulf_area(27.0, -83.0, 28.0, -82.0));
        assert!(!overlaps_gulf_area(40.5, -74.3, 40.9, -73.8));
    }

    #[test]
    fn the_recorded_zip_gives_the_nearest_beach_at_the_nearest_hour() {
        let now = at("2026-11-17T09:00:00Z");
        let forecast = parse_gulf_zip(&zip(), LONGBOAT, now).expect("a forecast");
        assert_eq!(forecast.location, "Longboat Pass Boat Ramp (Sarasota Bay)");
        assert_eq!(forecast.level, HabLevel::VeryLow);
        assert_eq!(forecast.confidence, Some(0.9));
        // The 08:00Z hour (valid_01hrs) is nearest 09:00Z; the next is 11:00Z.
        assert_eq!(forecast.valid_at, at("2026-11-17T08:00:00Z"));
        assert!(forecast.distance_m < 10.0);
        let p = &forecast.provenance;
        assert_eq!(p.source, SourceId::NOAA_HAB);
        assert_eq!(p.kind, ValueKind::Modelled);
        assert_eq!(p.observed_at, at("2026-11-17T08:00:00Z"), "the run, -05:00");
    }

    #[test]
    fn a_later_time_picks_a_later_forecast_hour() {
        let now = at("2026-11-17T20:30:00Z");
        let forecast = parse_gulf_zip(&zip(), LONGBOAT, now).expect("a forecast");
        assert_eq!(forecast.valid_at, at("2026-11-17T20:00:00Z"));
    }

    #[test]
    fn a_spot_without_a_beach_in_range_is_no_data() {
        let hudson = Coordinates {
            latitude: 40.7033,
            longitude: -74.017,
        };
        assert!(matches!(
            parse_gulf_zip(&zip(), hudson, at("2026-11-17T09:00:00Z")),
            Err(SourceError::NoData(_))
        ));
        // Offshore, 24 km west of Clearwater Beach, is out of range as well.
        let far = Coordinates {
            latitude: CLEARWATER.latitude,
            longitude: -83.1,
        };
        let near = parse_gulf_zip(&zip(), CLEARWATER, at("2026-11-17T09:00:00Z"));
        assert!(near.is_ok(), "{near:?}");
        assert!(matches!(
            parse_gulf_zip(&zip(), far, at("2026-11-17T09:00:00Z")),
            Err(SourceError::NoData(_))
        ));
    }

    #[test]
    fn an_old_run_is_stale() {
        let err = parse_gulf_zip(&zip(), LONGBOAT, at("2026-11-20T09:00:00Z")).unwrap_err();
        assert_eq!(
            err,
            SourceError::Stale {
                latest: at("2026-11-18T14:00:00Z")
            }
        );
    }

    #[test]
    fn tolerates_values_the_dictionary_does_not_list() {
        let csv = format!(
            "{HEADER}{}{}",
            row(
                "Extreme",
                "2026-11-17T08:00:00+00:00",
                27.0,
                -82.0,
                "\"Quoted, Beach\""
            ),
            row(
                "Moderate",
                "2026-11-17T08:00:00+00:00",
                27.5,
                -82.0,
                "Far Beach"
            ),
        );
        let zip = stored_zip(&[("run_forecast_valid_01hrs.csv", csv.as_bytes())]);
        let spot = Coordinates {
            latitude: 27.0,
            longitude: -82.0,
        };
        let forecast = parse_gulf_zip(&zip, spot, at("2026-11-17T08:30:00Z")).unwrap();
        assert_eq!(forecast.level, HabLevel::Unknown);
        assert_eq!(forecast.location, "Quoted, Beach");
    }

    #[test]
    fn skips_damaged_rows_and_reads_the_rest() {
        let csv = format!(
            "{HEADER}Low,1,x,y\n{}Low,0,Not Present,t,1,2,Offshore,not a time,1,27.0,-82.0,0.9,Bad Time,t,s\n{}",
            row(
                "High",
                "2026-11-17T08:00:00+00:00",
                f64::NAN,
                -82.0,
                "NaN Beach"
            ),
            row(
                "High",
                "2026-11-17T08:00:00-05:00",
                27.0,
                -82.0,
                "Good Beach"
            ),
        );
        let zip = stored_zip(&[("run_forecast_valid_01hrs.csv", csv.as_bytes())]);
        let spot = Coordinates {
            latitude: 27.0,
            longitude: -82.0,
        };
        let forecast = parse_gulf_zip(&zip, spot, at("2026-11-17T13:00:00Z")).unwrap();
        assert_eq!(forecast.location, "Good Beach");
        assert_eq!(forecast.level, HabLevel::High);
        assert_eq!(
            forecast.valid_at,
            at("2026-11-17T13:00:00Z"),
            "-05:00 offset"
        );
    }

    #[test]
    fn a_file_without_the_needed_columns_is_a_parse_error() {
        let zip = stored_zip(&[("run_forecast_valid_01hrs.csv", b"A,B\n1,2\n")]);
        let err = parse_gulf_zip(&zip, LONGBOAT, at("2026-11-17T09:00:00Z")).unwrap_err();
        assert!(matches!(err, SourceError::Parse(_)), "{err:?}");
    }

    #[test]
    fn bad_archives_fail_softly() {
        let now = at("2026-11-17T09:00:00Z");
        assert!(matches!(
            parse_gulf_zip(b"not a zip", LONGBOAT, now),
            Err(SourceError::Parse(_))
        ));
        assert!(matches!(
            parse_gulf_zip(b"", LONGBOAT, now),
            Err(SourceError::Parse(_))
        ));
        let only_docs = stored_zip(&[("RIFv2_README.txt", b"hello")]);
        assert!(matches!(
            parse_gulf_zip(&only_docs, LONGBOAT, now),
            Err(SourceError::Parse(_))
        ));
    }

    #[test]
    fn inflating_past_the_limits_is_refused() {
        let now = at("2026-11-17T09:00:00Z");
        let big = vec![b'a'; MAX_ENTRY_BYTES as usize + 1];
        let zip = stored_zip(&[("run_forecast_valid_01hrs.csv", &big)]);
        assert!(matches!(
            parse_gulf_zip(&zip, LONGBOAT, now),
            Err(SourceError::Parse(m)) if m.contains("size limit")
        ));
        // Entries that are not forecasts are never inflated, whatever their size.
        let zip = stored_zip(&[("RIFv2_README.txt", &big)]);
        assert!(matches!(
            parse_gulf_zip(&zip, LONGBOAT, now),
            Err(SourceError::Parse(m)) if m.contains("no forecast files")
        ));
        let names: Vec<String> = (0..=MAX_ENTRIES).map(|i| format!("f{i}.txt")).collect();
        let entries: Vec<(&str, &[u8])> = names.iter().map(|n| (n.as_str(), &b"x"[..])).collect();
        assert!(matches!(
            parse_gulf_zip(&stored_zip(&entries), LONGBOAT, now),
            Err(SourceError::Parse(m)) if m.contains("too many")
        ));
    }

    #[test]
    fn hand_built_zips_read_like_recorded_ones() {
        let csv = format!(
            "{HEADER}{}",
            row(
                "Low",
                "2026-11-17T08:00:00+00:00",
                27.0,
                -82.0,
                "Test Beach"
            )
        );
        let zip = stored_zip(&[
            ("RIFv2_data_dictionary.csv", b"Field\n"),
            (
                "RIFv2_modelrun_x-EST_forecast_valid_01hrs.csv",
                csv.as_bytes(),
            ),
        ]);
        let spot = Coordinates {
            latitude: 27.0,
            longitude: -82.0,
        };
        let forecast = parse_gulf_zip(&zip, spot, at("2026-11-17T08:00:00Z")).unwrap();
        assert_eq!(forecast.level, HabLevel::Low);
    }

    #[test]
    fn links_cover_the_regions_and_nothing_for_new_york() {
        let regions: Vec<HabRegion> = region_links().into_iter().map(|(r, _)| r).collect();
        assert_eq!(
            regions,
            [
                HabRegion::LakeErie,
                HabRegion::GulfOfMaine,
                HabRegion::PacificNorthwest,
                HabRegion::Florida,
                HabRegion::Texas
            ]
        );
        let battery = Coordinates {
            latitude: 40.7033,
            longitude: -74.017,
        };
        assert_eq!(links_near(battery).len(), 0);
        let erie = Coordinates {
            latitude: 41.7,
            longitude: -82.8,
        };
        let links = links_near(erie);
        assert_eq!(links.len(), 1);
        assert!(links[0].label.contains("Lake Erie"));
        assert_eq!(
            links_near(LONGBOAT)[0].label,
            "NOAA Florida respiratory forecast"
        );
        assert!(
            region_links()
                .iter()
                .all(|(_, l)| l.url.starts_with("https://"))
        );
    }

    fn gulf_target(point: Coordinates, measurements: &[Measurement]) -> Target {
        let now = at("2026-11-17T09:00:00Z");
        Target {
            place: Place::Point(crate::request::RoundedPoint::new(point, 2)),
            measurements: measurements.iter().copied().collect(),
            window: crate::request::Window {
                from: now,
                to: UnixTime(now.0 + 86_400),
            },
            datum: None,
        }
    }

    #[test]
    fn source_plans_the_current_run_and_parses_the_nearest_beach() {
        let ctx = FetchContext::new(at("2026-11-17T09:00:00Z"), "underbite/test");
        let source: &dyn Source = &HabGulf;
        assert_eq!(source.id(), SourceId::NOAA_HAB);
        assert_eq!(source.measurements(), [Measurement::Bloom]);
        let target = gulf_target(LONGBOAT, &[Measurement::Bloom, Measurement::Waves]);
        let plan = source
            .plan(&target, &Requirements::default(), &ctx)
            .expect("a plan");
        let [request] = plan.as_slice() else {
            panic!("one request, got {plan:?}");
        };
        assert_eq!(request.url, GULF_ZIP_URL);
        assert!(!request.url.contains("27.4"), "the point stays here");
        assert_eq!(request.resource, Resource::HabGulfZip);
        assert!(request.required && !request.metered);
        assert_eq!(
            request.target.measurements,
            BTreeSet::from([Measurement::Bloom])
        );

        let zip = zip();
        let mut out = Fragments::new();
        source
            .parse(request, &Response::new(200, &zip), &ctx, &mut out)
            .expect("parses");
        let [fragment] = out.fragments() else {
            panic!("one fragment, got {:?}", out.fragments());
        };
        assert_eq!(fragment.measurement, Measurement::Bloom);
        let FragmentValue::Bloom(info) = &fragment.value else {
            panic!("a bloom block");
        };
        let hab = info.hab.as_ref().expect("a forecast");
        assert_eq!(hab.location, "Longboat Pass Boat Ramp (Sarasota Bay)");
        assert_eq!(hab.level, HabLevel::VeryLow);
        assert_eq!(hab.valid_at, at("2026-11-17T08:00:00Z"));
        assert!(hab.distance_m < MAX_BEACH_DISTANCE_M);
        assert!(info.cyan_image_date.is_none() && info.cyan_max_cells_ml.is_none());
        assert_eq!(info.links.len(), 1);
        assert_eq!(info.links[0].label, "NOAA Florida respiratory forecast");
    }

    #[test]
    fn source_says_stale_or_out_of_range_and_never_no_bloom() {
        let zip = zip();
        let late = FetchContext::new(at("2026-11-20T09:00:00Z"), "underbite/test");
        let request = HabGulf
            .plan(
                &gulf_target(LONGBOAT, &[Measurement::Bloom]),
                &Requirements::default(),
                &late,
            )
            .expect("a plan")
            .remove(0);
        let mut out = Fragments::new();
        assert!(matches!(
            HabGulf.parse(&request, &Response::new(200, &zip), &late, &mut out),
            Err(SourceError::Stale { .. })
        ));
        // In Florida, but 24 km offshore: no beach within range.
        let ctx = FetchContext::new(at("2026-11-17T09:00:00Z"), "underbite/test");
        let offshore = Coordinates {
            latitude: CLEARWATER.latitude,
            longitude: -83.1,
        };
        let request = HabGulf
            .plan(
                &gulf_target(offshore, &[Measurement::Bloom]),
                &Requirements::default(),
                &ctx,
            )
            .expect("a plan")
            .remove(0);
        assert!(matches!(
            HabGulf.parse(&request, &Response::new(200, &zip), &ctx, &mut out),
            Err(SourceError::NoData(_))
        ));
        assert!(matches!(
            HabGulf.parse(&request, &Response::new(200, b""), &ctx, &mut out),
            Err(SourceError::NoData(_))
        ));
        assert!(matches!(
            HabGulf.parse(&request, &Response::new(503, b"busy"), &ctx, &mut out),
            Err(SourceError::Status { status: 503, .. })
        ));
        assert!(out.is_empty());
    }

    #[test]
    fn source_plans_nothing_outside_the_gulf_or_without_a_bloom() {
        let ctx = FetchContext::new(at("2026-11-17T09:00:00Z"), "underbite/test");
        let none = Requirements::default();
        let battery = Coordinates {
            latitude: 40.7033,
            longitude: -74.017,
        };
        let galveston = Coordinates {
            latitude: 29.28,
            longitude: -94.8,
        };
        assert!(in_gulf_area(LONGBOAT) && in_gulf_area(galveston));
        assert!(!in_gulf_area(battery));
        for target in [
            gulf_target(battery, &[Measurement::Bloom]),
            gulf_target(LONGBOAT, &[Measurement::Cyanobacteria]),
        ] {
            assert!(matches!(
                HabGulf.plan(&target, &none, &ctx),
                Err(SourceError::Unsupported(_))
            ));
        }
        let lake_only = Requirements {
            measurements: BTreeSet::from([Measurement::LakeLevel]),
            ..Requirements::default()
        };
        assert!(matches!(
            HabGulf.plan(
                &gulf_target(LONGBOAT, &[Measurement::Bloom]),
                &lake_only,
                &ctx
            ),
            Err(SourceError::Unsupported(_))
        ));
        let mut zone = gulf_target(LONGBOAT, &[Measurement::Bloom]);
        zone.place = Place::Zone(crate::request::ZoneTarget {
            id: "GMZ830".into(),
            name: "Tampa Bay".into(),
            kind: underbite_core::model::ZoneKind::Coastal,
            office: None,
        });
        assert!(matches!(
            HabGulf.plan(&zone, &none, &ctx),
            Err(SourceError::Unsupported(_))
        ));
    }

    #[test]
    fn the_credit_names_noaa_and_the_cell_count_sources() {
        let a = attribution();
        assert_eq!(a.source, SourceId::NOAA_HAB);
        for part in [
            "National Centers for Coastal Ocean Science",
            "FWRI",
            "HABscope",
            "Pinellas",
        ] {
            assert!(a.credit.contains(part), "{part}");
        }
    }
}
