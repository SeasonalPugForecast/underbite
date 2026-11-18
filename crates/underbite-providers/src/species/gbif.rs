//! GBIF occurrence search: records of a species inside a box, and observations by month.
//!
//! Sans-IO, for the map: a host sends [`occurrence_request`] for a page of records and hands the
//! body to [`parse_page`], asking again at [`GbifPage::next_offset`] while it is set; for the
//! typical-season strip it sends [`month_facet_request`] (no records, only counts) and reads it
//! with [`parse_month_facet`]. Every request carries [`user_agent`] and may be reused for
//! [`MAX_AGE_S`] seconds, as the service's own `cache-control` says. HTTP 429 is
//! [`GbifError::RateLimited`]: the host waits [`GbifError::retry_after_s`] before asking again.
//!
//! The box is rounded outward to 2 decimals (about 1 km) before it is sent. GBIF clamps `limit`
//! to [`MAX_LIMIT`] and refuses `offset + limit` above [`MAX_WINDOW`] (both measured).
//!
//! Each record keeps its own licence (`license`, a Creative Commons URL); a record with no licence
//! or one that is not CC0, CC BY or CC BY-NC is dropped and counted in
//! [`GbifPage::unlicensed`]. A photo is kept only with an open licence of its own. Only the
//! fields a [`SpeciesRecord`] carries are read: observer, identifier, rights-holder and photo
//! creator names (`recordedBy`, `identifiedBy`, `rightsHolder`, `media[].creator`, `extensions`)
//! are never read, so they cannot be stored. Credit: [`attribution`] with each record's dataset.

use serde::Deserialize;
use serde_json::Value;
use underbite_core::map::Bbox;
use underbite_core::model::nearby::{SpeciesPhoto, SpeciesRecord};
use underbite_core::provenance::{Attribution, SourceId};
use underbite_core::species::seasons::MonthObservations;
use underbite_core::time::{LocalDate, parse_date};

use super::{Licence, inaturalist_observation_id, request_box};
use crate::request::{QueryBuilder, Response};

const SEARCH_URL: &str = "https://api.gbif.org/v1/occurrence/search";

/// GBIF's page for one occurrence; the key follows.
const OCCURRENCE_PAGE: &str = "https://www.gbif.org/occurrence/";

/// The dataset GBIF republishes iNaturalist's research-grade observations in.
pub const INATURALIST_DATASET_KEY: &str = "50c9509d-22c7-4a22-a47d-8c48425ef4a7";

/// Records asked for per page. A record is about 7 KB of JSON, so pages stay well under the
/// service's maximum.
pub const PAGE_SIZE: u32 = 100;

/// The largest `limit` the service honours; larger values are clamped to it.
pub const MAX_LIMIT: u32 = 300;

/// The service refuses a request whose `offset + limit` exceeds this.
pub const MAX_WINDOW: u32 = 100_000;

/// How long a response may be reused, seconds (the service's `cache-control: max-age`).
pub const MAX_AGE_S: u32 = 600;

/// How long to wait after HTTP 429 when the response gives no `Retry-After`, seconds.
pub const DEFAULT_BACKOFF_S: u32 = 60;

/// The credit every display of GBIF records shows, with each record's dataset and licence.
pub const CREDIT: &str = "GBIF.org";

/// GBIF's home page, linked from the credit.
pub const GBIF_URL: &str = "https://www.gbif.org/";

/// The `User-Agent` GBIF sees: `underbite/<version> (+<project URL>)`, from the package's
/// `repository`; without the `+` part while none is set. It never names a person.
pub fn user_agent() -> String {
    user_agent_with(Some(env!("CARGO_PKG_REPOSITORY")))
}

fn user_agent_with(project_url: Option<&str>) -> String {
    let version = env!("CARGO_PKG_VERSION");
    match project_url
        .map(str::trim)
        .filter(|u| !u.is_empty() && !u.contains(".invalid"))
    {
        Some(url) => format!("underbite/{version} (+{url})"),
        None => format!("underbite/{version}"),
    }
}

/// The credit as an [`Attribution`].
pub fn attribution() -> Attribution {
    Attribution {
        source: SourceId::GBIF,
        credit: CREDIT.into(),
        url: Some(GBIF_URL.into()),
    }
}

/// One GET for the host to send, with its `User-Agent`. The URL carries no key or personal data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GbifRequest {
    pub url: String,
    pub user_agent: String,
    /// How long the response may be reused, seconds.
    pub max_age_s: u32,
}

/// A GBIF request could not be built, or its response read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GbifError {
    /// The request asks for something that cannot be sent.
    #[error("invalid GBIF request: {0}")]
    InvalidRequest(&'static str),
    /// HTTP 429: too many requests.
    #[error("GBIF is limiting requests; try again in {retry_after_s} s")]
    RateLimited { retry_after_s: u32 },
    /// Any other HTTP failure.
    #[error("GBIF answered HTTP {status}")]
    Status { status: u16 },
    /// The body is not the expected JSON.
    #[error("the GBIF response is not the expected JSON: {0}")]
    Parse(String),
}

impl GbifError {
    /// Seconds to wait before the next request, for [`GbifError::RateLimited`].
    pub fn retry_after_s(&self) -> Option<u32> {
        match self {
            Self::RateLimited { retry_after_s } => Some(*retry_after_s),
            _ => None,
        }
    }
}

/// The search for a species inside a box, without paging parameters.
fn search(scientific_name: &str, bbox: Bbox) -> Result<QueryBuilder, GbifError> {
    let name = scientific_name.trim();
    if name.is_empty() {
        return Err(GbifError::InvalidRequest("scientific name"));
    }
    let b = request_box(bbox).ok_or(GbifError::InvalidRequest("box"))?;
    Ok(QueryBuilder::new(SEARCH_URL)
        .param("scientificName", name)
        .param("decimalLatitude", format!("{:.2},{:.2}", b.south, b.north))
        .param("decimalLongitude", format!("{:.2},{:.2}", b.west, b.east))
        .param("hasCoordinate", "true"))
}

fn request(url: String) -> GbifRequest {
    GbifRequest {
        url,
        user_agent: user_agent(),
        max_age_s: MAX_AGE_S,
    }
}

/// The records of a species (scientific name, e.g. `Morone saxatilis`) inside a box, one page:
/// `offset` records skipped (0, then [`GbifPage::next_offset`]) and `limit` asked for, between 1
/// and [`MAX_LIMIT`] and never past [`MAX_WINDOW`] ([`PAGE_SIZE`] is a good value).
///
/// # Errors
/// [`GbifError::InvalidRequest`] for an empty name, a box that is not finite, outside the world
/// or inverted, or an offset at or past [`MAX_WINDOW`].
pub fn occurrence_request(
    scientific_name: &str,
    bbox: Bbox,
    offset: u32,
    limit: u32,
) -> Result<GbifRequest, GbifError> {
    if offset >= MAX_WINDOW {
        return Err(GbifError::InvalidRequest("offset"));
    }
    let limit = limit.clamp(1, MAX_LIMIT).min(MAX_WINDOW - offset);
    let url = search(scientific_name, bbox)?
        .param("limit", limit.to_string())
        .param("offset", offset.to_string())
        .finish();
    Ok(request(url))
}

/// How many records of a species inside a box fall in each calendar month, with no records
/// (`limit=0` and a month facet).
///
/// # Errors
/// [`GbifError::InvalidRequest`] for an empty name or a box that cannot be sent.
pub fn month_facet_request(scientific_name: &str, bbox: Bbox) -> Result<GbifRequest, GbifError> {
    let url = search(scientific_name, bbox)?
        .param("limit", "0")
        .param("facet", "month")
        .param("facetLimit", "12")
        .finish();
    Ok(request(url))
}

/// A record with what GBIF adds beyond the [`SpeciesRecord`].
#[derive(Clone, Debug, PartialEq)]
pub struct GbifRecord {
    pub record: SpeciesRecord,
    /// The iNaturalist observation this record republishes, to leave out a record shown twice
    /// (see [`super::without_inaturalist_duplicates`]).
    pub inaturalist_id: Option<u64>,
    /// The month observed (1 to 12), also when the day is not known.
    pub month: Option<u8>,
    /// The publishing dataset, for the credit.
    pub dataset_key: Option<String>,
    /// The dataset's title, for the credit.
    pub dataset_name: Option<String>,
}

/// The records one page returned.
#[derive(Clone, Debug, PartialEq)]
pub struct GbifPage {
    pub records: Vec<GbifRecord>,
    /// Rows the page held.
    pub rows: usize,
    /// Rows left out for having no licence or an unknown one.
    pub unlicensed: usize,
    /// Rows left out for a missing or invalid position, a missing id or taxon, or an absence
    /// record.
    pub skipped: usize,
    /// Records matching the search in all pages, as the service counts them.
    pub total: Option<u64>,
    /// The offset of the next page, while the service has more and it can be asked for.
    pub next_offset: Option<u32>,
}

/// Monthly counts for the typical-season strip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MonthFacet {
    /// January first; months with no record are left out.
    pub months: Vec<MonthObservations>,
    /// Records matching the search, including those with no month.
    pub total: Option<u64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Row {
    key: Option<Value>,
    #[serde(rename = "gbifID")]
    gbif_id: Option<Value>,
    species: Option<String>,
    scientific_name: Option<String>,
    decimal_latitude: Option<f64>,
    decimal_longitude: Option<f64>,
    coordinate_uncertainty_in_meters: Option<f64>,
    information_withheld: Option<String>,
    data_generalizations: Option<String>,
    year: Option<i32>,
    month: Option<u8>,
    day: Option<u8>,
    event_date: Option<String>,
    occurrence_status: Option<String>,
    license: Option<String>,
    dataset_key: Option<String>,
    dataset_name: Option<String>,
    #[serde(rename = "occurrenceID")]
    occurrence_id: Option<String>,
    references: Option<String>,
    catalog_number: Option<String>,
    media: Option<Vec<Media>>,
}

#[derive(Deserialize)]
struct Media {
    #[serde(rename = "type")]
    kind: Option<String>,
    format: Option<String>,
    identifier: Option<String>,
    license: Option<String>,
}

fn nonempty(s: Option<&str>) -> Option<String> {
    s.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn id_text(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::Number(n) => Some(n.to_string()),
        Value::String(s) => nonempty(Some(s)),
        _ => None,
    }
}

fn is_web_url(s: &str) -> bool {
    s.starts_with("https://") || s.starts_with("http://")
}

/// The day observed: from `year`, `month` and `day`, else from the date at the start of
/// `eventDate` (local time without an offset, or a range `a/b`). None when the day is not known.
fn observed_on(row: &Row) -> Option<LocalDate> {
    if let (Some(y), Some(m), Some(d)) = (row.year, row.month, row.day)
        && let Ok(date) = parse_date(&format!("{y:04}-{m:02}-{d:02}"))
    {
        return Some(date);
    }
    let text = row.event_date.as_deref()?.trim();
    parse_date(text.get(..10)?).ok()
}

/// Whether the publisher says it generalised the position (iNaturalist obscures some taxa).
fn obscured(row: &Row) -> bool {
    [&row.information_withheld, &row.data_generalizations]
        .into_iter()
        .flatten()
        .map(|t| t.to_ascii_lowercase())
        .any(|t| {
            ["coordinate", "location", "position", "obscur", "generaliz"]
                .iter()
                .any(|w| t.contains(w))
        })
}

/// The first still image with an open licence of its own.
fn photo(media: &[Media]) -> Option<SpeciesPhoto> {
    media.iter().find_map(|m| {
        let image = m.kind.as_deref() == Some("StillImage")
            || m.format.as_deref().is_some_and(|f| f.starts_with("image/"));
        let url = nonempty(m.identifier.as_deref()).filter(|u| is_web_url(u))?;
        let licence = nonempty(m.license.as_deref())?;
        (image && Licence::classify(&licence).is_some()).then_some(SpeciesPhoto {
            url,
            licence: Some(licence),
        })
    })
}

fn inaturalist_id(row: &Row) -> Option<u64> {
    let from_url = [&row.occurrence_id, &row.references]
        .into_iter()
        .flatten()
        .find_map(|u| inaturalist_observation_id(u));
    from_url.or_else(|| {
        (row.dataset_key.as_deref() == Some(INATURALIST_DATASET_KEY))
            .then(|| row.catalog_number.as_deref()?.trim().parse().ok())
            .flatten()
    })
}

enum Read {
    Kept(Box<GbifRecord>),
    Unlicensed,
    Skipped,
}

fn read_row(value: Value) -> Read {
    let Ok(row) = serde_json::from_value::<Row>(value) else {
        return Read::Skipped;
    };
    let Some(licence) = nonempty(row.license.as_deref()) else {
        return Read::Unlicensed;
    };
    if Licence::classify(&licence).is_none() {
        return Read::Unlicensed;
    }
    if row
        .occurrence_status
        .as_deref()
        .is_some_and(|s| s.trim().eq_ignore_ascii_case("absent"))
    {
        return Read::Skipped;
    }
    let (Some(lat), Some(lon)) = (row.decimal_latitude, row.decimal_longitude) else {
        return Read::Skipped;
    };
    if !((-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon)) {
        return Read::Skipped;
    }
    let Some(id) = id_text(row.key.as_ref()).or_else(|| id_text(row.gbif_id.as_ref())) else {
        return Read::Skipped;
    };
    let Some(taxon) =
        nonempty(row.species.as_deref()).or_else(|| nonempty(row.scientific_name.as_deref()))
    else {
        return Read::Skipped;
    };
    let uncertainty_m = row
        .coordinate_uncertainty_in_meters
        .filter(|u| u.is_finite() && *u >= 0.0)
        .map(|u| u as f32);
    let record = SpeciesRecord {
        source: SourceId::GBIF,
        url: Some(format!("{OCCURRENCE_PAGE}{id}")),
        id,
        taxon,
        latitude: lat,
        longitude: lon,
        uncertainty_m,
        obscured: obscured(&row),
        observed_on: observed_on(&row),
        licence: Some(licence),
        photo: row.media.as_deref().and_then(photo),
    };
    Read::Kept(Box::new(GbifRecord {
        inaturalist_id: inaturalist_id(&row),
        month: row.month.filter(|m| (1..=12).contains(m)),
        dataset_key: nonempty(row.dataset_key.as_deref()),
        dataset_name: nonempty(row.dataset_name.as_deref()),
        record,
    }))
}

/// The JSON object of a successful response.
fn body(response: &Response<'_>) -> Result<Value, GbifError> {
    match response.status {
        200..=299 => {}
        429 => {
            return Err(GbifError::RateLimited {
                retry_after_s: response.retry_after.unwrap_or(DEFAULT_BACKOFF_S),
            });
        }
        status => return Err(GbifError::Status { status }),
    }
    let v: Value =
        serde_json::from_slice(response.body).map_err(|e| GbifError::Parse(e.to_string()))?;
    if !v.is_object() {
        return Err(GbifError::Parse("not an object".to_owned()));
    }
    Ok(v)
}

/// Reads one page of [`occurrence_request`].
///
/// # Errors
/// [`GbifError::RateLimited`] for HTTP 429, [`GbifError::Status`] for another failure and
/// [`GbifError::Parse`] for a body that holds no result list.
pub fn parse_page(response: &Response<'_>) -> Result<GbifPage, GbifError> {
    let mut v = body(response)?;
    let Some(Value::Array(rows)) = v.get_mut("results").map(Value::take) else {
        return Err(GbifError::Parse("no results".to_owned()));
    };
    let n = rows.len();
    let mut records = Vec::new();
    let (mut unlicensed, mut skipped) = (0, 0);
    for row in rows {
        match read_row(row) {
            Read::Kept(r) => records.push(*r),
            Read::Unlicensed => unlicensed += 1,
            Read::Skipped => skipped += 1,
        }
    }
    let end = v
        .get("endOfRecords")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let offset = v.get("offset").and_then(Value::as_u64).unwrap_or(0);
    let limit = v.get("limit").and_then(Value::as_u64).unwrap_or(n as u64);
    let next = offset.saturating_add(limit.max(n as u64));
    let next_offset = (!end && n > 0 && next < u64::from(MAX_WINDOW))
        .then(|| u32::try_from(next).ok())
        .flatten();
    Ok(GbifPage {
        records,
        rows: n,
        unlicensed,
        skipped,
        total: v.get("count").and_then(Value::as_u64),
        next_offset,
    })
}

/// Reads [`month_facet_request`]: the counts sorted by month (the service sorts them by count).
/// A month outside 1 to 12 is ignored; no facet at all means no counts.
///
/// # Errors
/// As [`parse_page`].
pub fn parse_month_facet(response: &Response<'_>) -> Result<MonthFacet, GbifError> {
    let v = body(response)?;
    let facet = v
        .get("facets")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|f| {
            f.get("field")
                .and_then(Value::as_str)
                .is_some_and(|s| s.eq_ignore_ascii_case("month"))
        });
    let mut months: Vec<MonthObservations> = facet
        .and_then(|f| f.get("counts"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|c| {
            let month: u8 = c.get("name")?.as_str()?.trim().parse().ok()?;
            let count = u32::try_from(c.get("count")?.as_u64()?).unwrap_or(u32::MAX);
            ((1..=12).contains(&month) && count > 0).then_some(MonthObservations { month, count })
        })
        .collect();
    months.sort_by_key(|m| m.month);
    months.dedup_by_key(|m| m.month);
    Ok(MonthFacet {
        months,
        total: v.get("count").and_then(Value::as_u64),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::species::without_inaturalist_duplicates;
    use crate::testutil::fixture_bytes;

    const HARBOR: Bbox = Bbox {
        west: -74.4,
        south: 40.4,
        east: -73.6,
        north: 41.0,
    };

    fn query(url: &str) -> Vec<&str> {
        url.split_once('?').unwrap().1.split('&').collect()
    }

    fn fixture_page() -> GbifPage {
        let body = fixture_bytes(
            "tier2",
            "gbif-occurrence-striped-bass-nyharbor.synthetic.json",
        );
        parse_page(&Response::new(200, &body)).unwrap()
    }

    #[test]
    fn occurrence_request_is_a_rounded_box_with_paging() {
        let box_ = Bbox {
            west: -74.0170,
            south: 40.7033,
            east: -73.9911,
            north: 40.7201,
        };
        let r = occurrence_request("  Morone saxatilis ", box_, 100, PAGE_SIZE).unwrap();
        assert!(
            r.url
                .starts_with("https://api.gbif.org/v1/occurrence/search?")
        );
        assert_eq!(
            query(&r.url),
            [
                "scientificName=Morone%20saxatilis",
                "decimalLatitude=40.70,40.73",
                "decimalLongitude=-74.02,-73.99",
                "hasCoordinate=true",
                "limit=100",
                "offset=100",
            ]
        );
        assert_eq!(r.max_age_s, 600);
        assert!(r.user_agent.starts_with("underbite/"));
        insta::assert_snapshot!(
            occurrence_request("Morone saxatilis", HARBOR, 0, 2)
                .unwrap()
                .url
        );
    }

    #[test]
    fn limits_are_clamped_to_what_the_service_takes() {
        let big = occurrence_request("Morone saxatilis", HARBOR, 0, 5_000).unwrap();
        assert!(query(&big.url).contains(&"limit=300"));
        let zero = occurrence_request("Morone saxatilis", HARBOR, 0, 0).unwrap();
        assert!(query(&zero.url).contains(&"limit=1"));
        let tail = occurrence_request("Morone saxatilis", HARBOR, 99_950, 300).unwrap();
        assert!(query(&tail.url).contains(&"limit=50"));
        assert_eq!(
            occurrence_request("Morone saxatilis", HARBOR, MAX_WINDOW, 1),
            Err(GbifError::InvalidRequest("offset"))
        );
        assert_eq!(
            occurrence_request(" ", HARBOR, 0, 1),
            Err(GbifError::InvalidRequest("scientific name"))
        );
        let inverted = Bbox {
            west: 1.0,
            south: 0.0,
            east: 0.0,
            north: 1.0,
        };
        assert_eq!(
            month_facet_request("Morone saxatilis", inverted),
            Err(GbifError::InvalidRequest("box"))
        );
    }

    #[test]
    fn month_facet_request_asks_for_counts_only() {
        let r = month_facet_request("Morone saxatilis", HARBOR).unwrap();
        assert_eq!(
            query(&r.url),
            [
                "scientificName=Morone%20saxatilis",
                "decimalLatitude=40.40,41.00",
                "decimalLongitude=-74.40,-73.60",
                "hasCoordinate=true",
                "limit=0",
                "facet=month",
                "facetLimit=12",
            ]
        );
    }

    #[test]
    fn the_user_agent_names_the_project_never_a_person() {
        assert_eq!(
            user_agent_with(Some("https://example.org/underbite")),
            format!(
                "underbite/{} (+https://example.org/underbite)",
                env!("CARGO_PKG_VERSION")
            )
        );
        let bare = format!("underbite/{}", env!("CARGO_PKG_VERSION"));
        assert_eq!(user_agent_with(None), bare);
        assert_eq!(user_agent_with(Some(" ")), bare);
        assert_eq!(user_agent_with(Some("https://x.invalid/")), bare);
        assert!(!user_agent().contains('@'));
    }

    #[test]
    fn parses_the_harbor_fixture() {
        let page = fixture_page();
        assert_eq!((page.rows, page.unlicensed, page.skipped), (3, 0, 0));
        assert_eq!(page.total, Some(348));
        assert_eq!(page.next_offset, Some(3));
        let first = &page.records[0];
        assert_eq!(first.record.source, SourceId::GBIF);
        assert_eq!(first.record.id, "8000000001");
        assert_eq!(first.record.taxon, "Morone saxatilis");
        assert_eq!(
            (first.record.latitude, first.record.longitude),
            (40.5, -74.4)
        );
        assert_eq!(first.record.uncertainty_m, Some(100.0));
        assert!(!first.record.obscured);
        assert_eq!(
            first.record.observed_on,
            Some(LocalDate {
                year: 2026,
                month: 10,
                day: 28
            })
        );
        assert_eq!(Licence::of(&first.record), Some(Licence::Cc0));
        assert_eq!(
            first.record.url.as_deref(),
            Some("https://www.gbif.org/occurrence/8000000001")
        );
        let photo = first.record.photo.as_ref().unwrap();
        assert_eq!(
            photo.url,
            "https://example.invalid/photos/9300001/original.jpg"
        );
        assert_eq!(first.month, Some(10));
        assert_eq!(first.inaturalist_id, None, "not an iNaturalist URL");
        assert_eq!(
            first.dataset_name.as_deref(),
            Some("Synthetic research-grade observations")
        );
        let records: Vec<_> = page.records.iter().map(|r| &r.record).collect();
        insta::assert_yaml_snapshot!(records);
    }

    #[test]
    fn no_name_field_survives() {
        let body = fixture_bytes(
            "tier2",
            "gbif-occurrence-striped-bass-nyharbor.synthetic.json",
        );
        let raw = String::from_utf8(body).unwrap();
        assert!(raw.contains("Synthetic Observer"), "the fixture has names");
        let page = fixture_page();
        assert_eq!(page.records.len(), 3);
        let json =
            serde_json::to_string(&page.records.iter().map(|r| &r.record).collect::<Vec<_>>())
                .unwrap();
        let debug = format!("{page:?}");
        for text in [json.as_str(), debug.as_str()] {
            for word in [
                "Synthetic Observer",
                "Synthetic Publisher",
                "recordedBy",
                "identifiedBy",
                "rightsHolder",
                "creator",
                "Example Shore Road",
            ] {
                assert!(!text.contains(word), "{word}");
            }
        }
    }

    #[test]
    fn records_without_an_open_licence_are_dropped() {
        let body = br#"{"offset":0,"limit":20,"endOfRecords":true,"count":7,"results":[
          {"key":1,"species":"Morone saxatilis","decimalLatitude":40.7,"decimalLongitude":-74.0,
           "license":"http://creativecommons.org/licenses/by-nc/4.0/legalcode"},
          {"key":2,"species":"Morone saxatilis","decimalLatitude":40.7,"decimalLongitude":-74.0},
          {"key":3,"species":"Morone saxatilis","decimalLatitude":40.7,"decimalLongitude":-74.0,
           "license":"http://creativecommons.org/licenses/by-sa/4.0/legalcode"},
          {"key":4,"species":"Morone saxatilis","decimalLatitude":40.7,"decimalLongitude":-74.0,
           "license":"  "},
          {"key":5,"species":"Morone saxatilis","decimalLatitude":40.7,"decimalLongitude":-74.0,
           "license":"http://creativecommons.org/licenses/by/4.0/legalcode","occurrenceStatus":"ABSENT"},
          {"key":6,"species":"Morone saxatilis","decimalLatitude":95.0,"decimalLongitude":-74.0,
           "license":"http://creativecommons.org/licenses/by/4.0/legalcode"},
          {"key":7,"decimalLatitude":40.7,"decimalLongitude":-74.0,
           "license":"http://creativecommons.org/licenses/by/4.0/legalcode"}]}"#;
        let page = parse_page(&Response::new(200, body)).unwrap();
        assert_eq!((page.rows, page.unlicensed, page.skipped), (7, 3, 3));
        assert_eq!(page.records.len(), 1);
        assert_eq!(Licence::of(&page.records[0].record), Some(Licence::CcByNc));
        assert_eq!(page.next_offset, None, "end of records");
    }

    #[test]
    fn inaturalist_records_and_partial_dates_and_photos() {
        let body = br#"{"offset":20,"limit":20,"endOfRecords":false,"count":265,"results":[
          {"key":6235289760,"gbifID":"6235289760","datasetKey":"50c9509d-22c7-4a22-a47d-8c48425ef4a7",
           "scientificName":"Morone saxatilis (Walbaum, 1792)","decimalLatitude":40.71,"decimalLongitude":-74.02,
           "coordinateUncertaintyInMeters":27908.0,
           "informationWithheld":"Coordinate uncertainty increased to 27908m to protect threatened taxon",
           "year":2026,"month":4,"eventDate":"2026-04-11T08:25",
           "occurrenceID":"https://www.inaturalist.org/observations/348935944",
           "license":"http://creativecommons.org/licenses/by/4.0/legalcode",
           "media":[{"type":"StillImage","identifier":"https://example.invalid/a.jpg"},
                    {"type":"StillImage","identifier":"https://example.invalid/b.jpg",
                     "license":"http://creativecommons.org/licenses/by-nc/4.0/"}]},
          {"gbifID":"77","datasetKey":"50c9509d-22c7-4a22-a47d-8c48425ef4a7","catalogNumber":"55",
           "species":"Morone saxatilis","decimalLatitude":40.7,"decimalLongitude":-74.0,
           "eventDate":"2019-06-02/2019-06-04","year":2019,"month":6,"day":31,
           "license":"CC0_1_0",
           "media":[{"type":"Sound","identifier":"https://example.invalid/s.mp3",
                     "license":"http://creativecommons.org/publicdomain/zero/1.0/"}]}]}"#;
        let page = parse_page(&Response::new(200, body)).unwrap();
        assert_eq!(page.records.len(), 2);
        assert_eq!(page.next_offset, Some(40));
        let a = &page.records[0];
        assert_eq!(a.record.id, "6235289760");
        assert_eq!(a.record.taxon, "Morone saxatilis (Walbaum, 1792)");
        assert_eq!(a.inaturalist_id, Some(348_935_944));
        assert!(a.record.obscured);
        assert_eq!(a.record.uncertainty_m, Some(27_908.0));
        assert_eq!(
            a.record.observed_on,
            Some(LocalDate {
                year: 2026,
                month: 4,
                day: 11
            })
        );
        assert_eq!(a.month, Some(4));
        assert_eq!(
            a.record.photo.as_ref().map(|p| p.url.as_str()),
            Some("https://example.invalid/b.jpg"),
            "the photo without a licence is passed over"
        );
        let b = &page.records[1];
        assert_eq!(b.record.id, "77");
        assert_eq!(
            b.inaturalist_id,
            Some(55),
            "the catalogue number in the dataset"
        );
        assert_eq!(
            b.record.observed_on,
            Some(LocalDate {
                year: 2019,
                month: 6,
                day: 2
            }),
            "an invalid day falls back to the start of the range"
        );
        assert_eq!(b.record.photo, None, "a sound is not a photo");

        let inat = SpeciesRecord {
            source: SourceId::INATURALIST,
            id: "348935944".into(),
            ..a.record.clone()
        };
        let kept = without_inaturalist_duplicates(page.records.clone(), &[inat]);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].record.id, "77");
        let other = SpeciesRecord {
            source: SourceId::GBIF,
            id: "348935944".into(),
            ..a.record.clone()
        };
        assert_eq!(
            without_inaturalist_duplicates(page.records.clone(), &[other]).len(),
            2,
            "only iNaturalist records count"
        );
    }

    #[test]
    fn the_month_facet_is_sorted_by_month() {
        let body = fixture_bytes(
            "gbif",
            "gbif-facet-month-striped-bass-nyharbor-2026-11-17.json",
        );
        let facet = parse_month_facet(&Response::new(200, &body)).unwrap();
        assert_eq!(facet.total, Some(350));
        let months: Vec<(u8, u32)> = facet.months.iter().map(|m| (m.month, m.count)).collect();
        assert_eq!(
            months,
            [
                (1, 6),
                (2, 4),
                (3, 25),
                (4, 46),
                (5, 49),
                (6, 39),
                (7, 29),
                (8, 34),
                (9, 32),
                (10, 30),
                (11, 27),
                (12, 17)
            ]
        );
        assert_eq!(months.iter().map(|m| m.1).sum::<u32>(), 338);
        let odd = br#"{"count":3,"facets":[{"field":"MONTH","counts":[
            {"name":"13","count":1},{"name":"x","count":1},{"name":"2","count":2},{"name":"2","count":9}]}]}"#;
        let facet = parse_month_facet(&Response::new(200, odd)).unwrap();
        assert_eq!(facet.months, [MonthObservations { month: 2, count: 2 }]);
        let none = parse_month_facet(&Response::new(200, br#"{"count":0,"facets":[]}"#)).unwrap();
        assert_eq!(none.months.len(), 0);
    }

    #[test]
    fn the_two_record_fixture_parses() {
        let body = fixture_bytes(
            "gbif",
            "gbif-occurrence-striped-bass-nyharbor-2-records.synthetic.json",
        );
        let page = parse_page(&Response::new(200, &body)).unwrap();
        assert_eq!((page.rows, page.records.len()), (2, 2));
        assert_eq!(page.total, Some(350));
        assert_eq!(page.next_offset, Some(2));
    }

    #[test]
    fn failures_and_back_off() {
        let mut limited = Response::new(429, b"");
        assert_eq!(
            parse_page(&limited).unwrap_err().retry_after_s(),
            Some(DEFAULT_BACKOFF_S)
        );
        limited.retry_after = Some(120);
        assert_eq!(
            parse_month_facet(&limited),
            Err(GbifError::RateLimited { retry_after_s: 120 })
        );
        let too_far = Response::new(400, b"Max offset of 100001 exceeded: 100001 + 1");
        assert_eq!(parse_page(&too_far), Err(GbifError::Status { status: 400 }));
        assert_eq!(GbifError::Status { status: 400 }.retry_after_s(), None);
        for bad in [&b"<html>"[..], b"[]", b"{}", br#"{"results":{}}"#] {
            assert!(
                matches!(
                    parse_page(&Response::new(200, bad)),
                    Err(GbifError::Parse(_))
                ),
                "{}",
                String::from_utf8_lossy(bad)
            );
        }
        let a = attribution();
        assert_eq!((a.source, a.credit.as_str()), (SourceId::GBIF, "GBIF.org"));
    }
}
