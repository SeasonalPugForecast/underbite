//! iNaturalist research-grade observations inside a box, with open-licence photos only.
//!
//! Sans-IO, for the map: a host sends [`observations_request`] only on a user action (opening the
//! records layer, asking for the next page) and hands the body to [`parse_page`], asking again at
//! [`InatPage::next_page`] while it is set. The service asks apps to stay at or under
//! [`MAX_REQUESTS_PER_MINUTE`] and [`MAX_REQUESTS_PER_DAY`] requests, a budget shared by every
//! user of the app, so the host spaces requests by [`MIN_INTERVAL_MS`] and reuses a response for
//! [`MAX_AGE_S`] seconds (the service's own `cache-control`). HTTP 429 is
//! [`InatError::RateLimited`]: the host waits [`InatError::retry_after_s`] before asking again.
//!
//! The request goes to version 2 of the API, which honours `fields`: version 1 ignores it and
//! returns about 75 KB per observation. The fields asked for name no user, and the parser reads
//! only the fields a [`SpeciesRecord`] carries, so no login, name, icon or user id can survive
//! even if a response holds them.
//!
//! The box is rounded outward to 2 decimals (about 1 km) before it is sent. An observation
//! without an open licence of its own is dropped and counted in [`InatPage::unlicensed`]. A photo
//! is kept only when its own `license_code` is open (null means all rights reserved), resized to
//! [`PHOTO_SIZE`]. An obscured observation is an area: its `uncertainty_m` is the public
//! positional accuracy (about 27 km), never the private one. Credit: [`attribution`], the
//! observation's own page (`SpeciesRecord::url`) and its licence.

use serde::Deserialize;
use serde_json::Value;
use underbite_core::map::Bbox;
use underbite_core::model::nearby::{SpeciesPhoto, SpeciesRecord};
use underbite_core::provenance::{Attribution, SourceId};
use underbite_core::time::{LocalDate, parse_date};

use super::{Licence, request_box};
use crate::request::{QueryBuilder, Response};

const OBSERVATIONS_URL: &str = "https://api.inaturalist.org/v2/observations";

/// The observation's page at iNaturalist; the id follows.
const OBSERVATION_PAGE: &str = "https://www.inaturalist.org/observations/";

/// The fields asked for (the service's Rison-like syntax, `!t` is true). No user fields.
const FIELDS: &str = "(id:!t,observed_on:!t,time_observed_at:!t,geojson:(coordinates:!t),\
obscured:!t,positional_accuracy:!t,public_positional_accuracy:!t,license_code:!t,\
taxon:(id:!t,name:!t,preferred_common_name:!t),photos:(id:!t,url:!t,license_code:!t))";

/// Requests per minute the service asks apps to stay at or under.
pub const MAX_REQUESTS_PER_MINUTE: u32 = 60;

/// Requests per day the service asks apps to stay under, for all users of the app together.
pub const MAX_REQUESTS_PER_DAY: u32 = 10_000;

/// The least time between two requests, milliseconds, that keeps to [`MAX_REQUESTS_PER_MINUTE`].
pub const MIN_INTERVAL_MS: u32 = 60_000 / MAX_REQUESTS_PER_MINUTE;

/// How long a response may be reused, seconds (the service's `cache-control: max-age`).
pub const MAX_AGE_S: u32 = 300;

/// How long to wait after HTTP 429 when the response gives no `Retry-After`, seconds.
pub const DEFAULT_BACKOFF_S: u32 = 60;

/// Observations asked for per page. With the fields above one is about 700 bytes.
pub const PAGE_SIZE: u32 = 100;

/// The largest `per_page` the service honours.
pub const MAX_PER_PAGE: u32 = 200;

/// The service refuses a request whose `page * per_page` exceeds this (it answers HTTP 500).
pub const MAX_WINDOW: u32 = 10_000;

/// The radius given to an obscured observation that states no public accuracy, metres (the
/// service obscures to a cell about 27 km across).
pub const OBSCURED_FALLBACK_M: f32 = 30_000.0;

/// The size photos are shown at.
pub const PHOTO_SIZE: PhotoSize = PhotoSize::Medium;

/// The credit every display of iNaturalist records shows, with each record's page and licence.
pub const CREDIT: &str = "iNaturalist";

/// iNaturalist's home page, linked from the credit.
pub const INATURALIST_URL: &str = "https://www.inaturalist.org/";

/// The sizes a photo is served at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhotoSize {
    /// 75 px.
    Square,
    /// 240 px.
    Small,
    /// 500 px.
    Medium,
    /// 1024 px.
    Large,
    /// 2048 px.
    Original,
}

impl PhotoSize {
    fn name(self) -> &'static str {
        match self {
            Self::Square => "square",
            Self::Small => "small",
            Self::Medium => "medium",
            Self::Large => "large",
            Self::Original => "original",
        }
    }
}

/// A photo URL with its size name (`square`, `small`, `medium`, `large` or `original`) in the
/// file name swapped for `size`; a URL whose file name is none of these is returned as it is.
pub fn photo_url(url: &str, size: PhotoSize) -> String {
    let Some(slash) = url.rfind('/') else {
        return url.to_owned();
    };
    let (dir, file) = url.split_at(slash + 1);
    for old in ["square", "small", "medium", "large", "original"] {
        if let Some(rest) = file.strip_prefix(old)
            && rest.starts_with('.')
        {
            return format!("{dir}{}{rest}", size.name());
        }
    }
    url.to_owned()
}

/// The credit as an [`Attribution`].
pub fn attribution() -> Attribution {
    Attribution {
        source: SourceId::INATURALIST,
        credit: CREDIT.into(),
        url: Some(INATURALIST_URL.into()),
    }
}

/// One GET for the host to send, with its `User-Agent`. The URL carries no key or personal data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InatRequest {
    pub url: String,
    pub user_agent: String,
    /// How long the response may be reused, seconds.
    pub max_age_s: u32,
}

/// An iNaturalist request could not be built, or its response read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InatError {
    /// The request asks for something that cannot be sent.
    #[error("invalid iNaturalist request: {0}")]
    InvalidRequest(&'static str),
    /// HTTP 429: too many requests.
    #[error("iNaturalist is limiting requests; try again in {retry_after_s} s")]
    RateLimited { retry_after_s: u32 },
    /// Any other HTTP failure.
    #[error("iNaturalist answered HTTP {status}")]
    Status { status: u16 },
    /// The body is not the expected JSON.
    #[error("the iNaturalist response is not the expected JSON: {0}")]
    Parse(String),
}

impl InatError {
    /// Seconds to wait before the next request, for [`InatError::RateLimited`].
    pub fn retry_after_s(&self) -> Option<u32> {
        match self {
            Self::RateLimited { retry_after_s } => Some(*retry_after_s),
            _ => None,
        }
    }
}

/// The research-grade observations of a species (scientific name, e.g. `Morone saxatilis`)
/// inside a box, newest first, one page: `page` from 1 (then [`InatPage::next_page`]) and
/// `per_page` between 1 and [`MAX_PER_PAGE`] ([`PAGE_SIZE`] is a good value). Only observations
/// with an open licence, and only open-licence photos, are asked for.
///
/// # Errors
/// [`InatError::InvalidRequest`] for an empty name, a box that is not finite, outside the world
/// or inverted, or a page past [`MAX_WINDOW`].
pub fn observations_request(
    scientific_name: &str,
    bbox: Bbox,
    page: u32,
    per_page: u32,
) -> Result<InatRequest, InatError> {
    let name = scientific_name.trim();
    if name.is_empty() {
        return Err(InatError::InvalidRequest("scientific name"));
    }
    let b = request_box(bbox).ok_or(InatError::InvalidRequest("box"))?;
    let per_page = per_page.clamp(1, MAX_PER_PAGE);
    if page == 0 || page.saturating_mul(per_page) > MAX_WINDOW {
        return Err(InatError::InvalidRequest("page"));
    }
    let url = QueryBuilder::new(OBSERVATIONS_URL)
        .param("taxon_name", name)
        .param("quality_grade", "research")
        .param("license", "cc-by,cc-by-nc,cc0")
        .param("photo_license", "cc-by,cc-by-nc,cc0")
        .param("nelat", format!("{:.2}", b.north))
        .param("nelng", format!("{:.2}", b.east))
        .param("swlat", format!("{:.2}", b.south))
        .param("swlng", format!("{:.2}", b.west))
        .param("order_by", "observed_on")
        .param("per_page", per_page.to_string())
        .param("page", page.to_string())
        .param("fields", FIELDS)
        .finish();
    Ok(InatRequest {
        url,
        user_agent: super::gbif::user_agent(),
        max_age_s: MAX_AGE_S,
    })
}

/// A record with what iNaturalist adds beyond the [`SpeciesRecord`].
#[derive(Clone, Debug, PartialEq)]
pub struct InatRecord {
    pub record: SpeciesRecord,
    /// The month observed (1 to 12), for the typical-season strip.
    pub month: Option<u8>,
    /// The taxon's common name, when it has one.
    pub common_name: Option<String>,
}

/// The records one page returned.
#[derive(Clone, Debug, PartialEq)]
pub struct InatPage {
    pub records: Vec<InatRecord>,
    /// Rows the page held.
    pub rows: usize,
    /// Rows left out for having no licence or one that is not CC0, CC BY or CC BY-NC.
    pub unlicensed: usize,
    /// Rows left out for a missing or invalid position, a missing id or taxon.
    pub skipped: usize,
    /// Observations matching the search in all pages, as the service counts them.
    pub total: Option<u64>,
    /// The next page number, while the service has more and it can be asked for.
    pub next_page: Option<u32>,
}

// Only what a `SpeciesRecord` carries is named: serde skips the rest, so a user object the
// service might add is never copied out of the parsed `Value`, which is dropped at once.
#[derive(Deserialize)]
struct Row {
    id: Option<u64>,
    observed_on: Option<String>,
    time_observed_at: Option<String>,
    geojson: Option<Geo>,
    obscured: Option<bool>,
    positional_accuracy: Option<f64>,
    public_positional_accuracy: Option<f64>,
    license_code: Option<String>,
    taxon: Option<Taxon>,
    photos: Option<Vec<Photo>>,
}

#[derive(Deserialize)]
struct Geo {
    coordinates: Option<Vec<f64>>,
}

#[derive(Deserialize)]
struct Taxon {
    name: Option<String>,
    preferred_common_name: Option<String>,
}

#[derive(Deserialize)]
struct Photo {
    url: Option<String>,
    license_code: Option<String>,
}

fn nonempty(s: Option<&str>) -> Option<String> {
    s.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn is_web_url(s: &str) -> bool {
    s.starts_with("https://") || s.starts_with("http://")
}

fn accuracy(v: Option<f64>) -> Option<f32> {
    v.filter(|u| u.is_finite() && *u >= 0.0).map(|u| u as f32)
}

/// The day observed: `observed_on`, else the date at the start of `time_observed_at`.
fn observed_on(row: &Row) -> Option<LocalDate> {
    [&row.observed_on, &row.time_observed_at]
        .into_iter()
        .flatten()
        .find_map(|t| parse_date(t.trim().get(..10)?).ok())
}

/// The first photo whose own licence is open (null is all rights reserved).
fn photo(photos: &[Photo]) -> Option<SpeciesPhoto> {
    photos.iter().find_map(|p| {
        let url = nonempty(p.url.as_deref()).filter(|u| is_web_url(u))?;
        let code = nonempty(p.license_code.as_deref())?;
        Licence::classify(&code)?;
        Some(SpeciesPhoto {
            url: photo_url(&url, PHOTO_SIZE),
            licence: Some(code),
        })
    })
}

enum Read {
    Kept(Box<InatRecord>),
    Unlicensed,
    Skipped,
}

fn read_row(value: Value) -> Read {
    let Ok(row) = serde_json::from_value::<Row>(value) else {
        return Read::Skipped;
    };
    let Some(code) = nonempty(row.license_code.as_deref()) else {
        return Read::Unlicensed;
    };
    if Licence::classify(&code).is_none() {
        return Read::Unlicensed;
    }
    let Some([lon, lat, ..]) = row.geojson.as_ref().and_then(|g| g.coordinates.as_deref()) else {
        return Read::Skipped;
    };
    let (lon, lat) = (*lon, *lat);
    if !((-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon)) {
        return Read::Skipped;
    }
    let Some(id) = row.id else {
        return Read::Skipped;
    };
    let Some(taxon) = row.taxon.as_ref().and_then(|t| nonempty(t.name.as_deref())) else {
        return Read::Skipped;
    };
    let obscured = row.obscured.unwrap_or(false);
    let public = accuracy(row.public_positional_accuracy);
    let uncertainty_m = if obscured {
        Some(public.unwrap_or(OBSCURED_FALLBACK_M))
    } else {
        public.or_else(|| accuracy(row.positional_accuracy))
    };
    let observed = observed_on(&row);
    let record = SpeciesRecord {
        source: SourceId::INATURALIST,
        id: id.to_string(),
        taxon,
        latitude: lat,
        longitude: lon,
        uncertainty_m,
        obscured,
        observed_on: observed,
        licence: Some(code),
        url: Some(format!("{OBSERVATION_PAGE}{id}")),
        photo: row.photos.as_deref().and_then(photo),
    };
    Read::Kept(Box::new(InatRecord {
        month: observed.map(|d| d.month).filter(|m| (1..=12).contains(m)),
        common_name: row
            .taxon
            .as_ref()
            .and_then(|t| nonempty(t.preferred_common_name.as_deref())),
        record,
    }))
}

/// Reads one page of [`observations_request`].
///
/// # Errors
/// [`InatError::RateLimited`] for HTTP 429, [`InatError::Status`] for another failure and
/// [`InatError::Parse`] for a body that holds no result list.
pub fn parse_page(response: &Response<'_>) -> Result<InatPage, InatError> {
    match response.status {
        200..=299 => {}
        429 => {
            return Err(InatError::RateLimited {
                retry_after_s: response.retry_after.unwrap_or(DEFAULT_BACKOFF_S),
            });
        }
        status => return Err(InatError::Status { status }),
    }
    let mut v: Value =
        serde_json::from_slice(response.body).map_err(|e| InatError::Parse(e.to_string()))?;
    let Some(Value::Array(rows)) = v.get_mut("results").map(Value::take) else {
        return Err(InatError::Parse("no results".to_owned()));
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
    let total = v.get("total_results").and_then(Value::as_u64);
    let page = v.get("page").and_then(Value::as_u64).unwrap_or(1);
    let per_page = v
        .get("per_page")
        .and_then(Value::as_u64)
        .unwrap_or(n as u64);
    let seen = page.saturating_mul(per_page);
    let next_window = page.saturating_add(1).saturating_mul(per_page);
    let more = n > 0 && total.is_some_and(|t| seen < t) && next_window <= u64::from(MAX_WINDOW);
    let next_page = more
        .then(|| u32::try_from(page.saturating_add(1)).ok())
        .flatten();
    Ok(InatPage {
        records,
        rows: n,
        unlicensed,
        skipped,
        total,
        next_page,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::fixture_bytes;

    const HARBOR: Bbox = Bbox {
        west: -74.4,
        south: 40.4,
        east: -73.6,
        north: 41.0,
    };

    const FIXTURE: &str = "inaturalist-observations-striped-bass-nyharbor.synthetic.json";

    fn page_of(json: &str) -> InatPage {
        parse_page(&Response::new(200, json.as_bytes())).unwrap()
    }

    fn fixture_page() -> InatPage {
        parse_page(&Response::new(200, &fixture_bytes("tier2", FIXTURE))).unwrap()
    }

    #[test]
    fn request_is_a_rounded_box_with_open_licences_and_no_user_fields() {
        let r = observations_request(" Morone saxatilis ", HARBOR, 2, PAGE_SIZE).unwrap();
        assert!(
            r.url
                .starts_with("https://api.inaturalist.org/v2/observations?")
        );
        let q: Vec<&str> = r.url.split_once('?').unwrap().1.split('&').collect();
        for want in [
            "taxon_name=Morone%20saxatilis",
            "quality_grade=research",
            "license=cc-by,cc-by-nc,cc0",
            "photo_license=cc-by,cc-by-nc,cc0",
            "nelat=41.00",
            "nelng=-73.60",
            "swlat=40.40",
            "swlng=-74.40",
            "per_page=100",
            "page=2",
        ] {
            assert!(q.contains(&want), "{want} in {q:?}");
        }
        assert!(!r.url.contains("user") && !r.url.contains("identifications"));
        assert_eq!(r.max_age_s, 300);
        assert!(r.user_agent.starts_with("underbite/"));
        insta::assert_snapshot!(
            observations_request("Morone saxatilis", HARBOR, 1, 3)
                .unwrap()
                .url
        );
    }

    #[test]
    fn a_box_is_rounded_outward_and_bad_requests_are_refused() {
        let box_ = Bbox {
            west: -74.0170,
            south: 40.7033,
            east: -73.9911,
            north: 40.7201,
        };
        let r = observations_request("Morone saxatilis", box_, 1, 10).unwrap();
        for want in ["nelat=40.73", "nelng=-73.99", "swlat=40.70", "swlng=-74.02"] {
            assert!(r.url.contains(want), "{want}");
        }
        assert_eq!(
            observations_request(" ", HARBOR, 1, 1),
            Err(InatError::InvalidRequest("scientific name"))
        );
        assert_eq!(
            observations_request(
                "x",
                Bbox {
                    west: 1.0,
                    south: 0.0,
                    east: 0.0,
                    north: 1.0
                },
                1,
                1
            ),
            Err(InatError::InvalidRequest("box"))
        );
        assert_eq!(
            observations_request("x", HARBOR, 0, 10),
            Err(InatError::InvalidRequest("page"))
        );
        assert_eq!(
            observations_request("x", HARBOR, 51, 200),
            Err(InatError::InvalidRequest("page"))
        );
        let big = observations_request("x", HARBOR, 1, 5_000).unwrap();
        assert!(big.url.contains("per_page=200"));
    }

    #[test]
    fn pacing_numbers_keep_to_the_services_ask() {
        assert_eq!(MAX_REQUESTS_PER_MINUTE, 60);
        assert_eq!(MIN_INTERVAL_MS, 1_000);
        assert_eq!(MAX_REQUESTS_PER_DAY, 10_000);
    }

    #[test]
    fn photo_sizes_are_swapped_in_the_file_name() {
        for (url, size, want) in [
            (
                "https://h.invalid/photos/1/square.jpg",
                PhotoSize::Medium,
                "https://h.invalid/photos/1/medium.jpg",
            ),
            (
                "https://h.invalid/photos/1/original.jpeg?x=1",
                PhotoSize::Small,
                "https://h.invalid/photos/1/small.jpeg?x=1",
            ),
            (
                "https://h.invalid/square/1/other.jpg",
                PhotoSize::Large,
                "https://h.invalid/square/1/other.jpg",
            ),
            ("nothing", PhotoSize::Large, "nothing"),
        ] {
            assert_eq!(photo_url(url, size), want, "{url}");
        }
    }

    #[test]
    fn parses_the_harbor_fixture() {
        let page = fixture_page();
        assert_eq!((page.rows, page.unlicensed, page.skipped), (3, 0, 0));
        assert_eq!(page.total, Some(3));
        assert_eq!(page.next_page, None);
        let first = &page.records[0];
        assert_eq!(first.record.source, SourceId::INATURALIST);
        assert_eq!(first.record.id, "9100001");
        assert_eq!(first.record.taxon, "Morone saxatilis");
        assert_eq!(first.common_name.as_deref(), Some("Striped Bass"));
        assert_eq!(
            (first.record.latitude, first.record.longitude),
            (40.5, -74.0)
        );
        assert_eq!(first.record.uncertainty_m, Some(10.0));
        assert!(!first.record.obscured);
        assert_eq!(
            first.record.observed_on,
            Some(LocalDate {
                year: 2026,
                month: 11,
                day: 1
            })
        );
        assert_eq!(first.month, Some(11));
        assert_eq!(Licence::of(&first.record), Some(Licence::Cc0));
        assert_eq!(
            first.record.url.as_deref(),
            Some("https://www.inaturalist.org/observations/9100001")
        );
        assert_eq!(
            first.record.photo.as_ref().unwrap().url,
            "https://example.invalid/photos/9000001/medium.jpg"
        );
        let records: Vec<_> = page.records.iter().map(|r| &r.record).collect();
        insta::assert_yaml_snapshot!(records);
    }

    #[test]
    fn an_obscured_observation_is_an_area_with_its_public_accuracy() {
        let page = fixture_page();
        let r = &page.records[1].record;
        assert!(r.obscured);
        assert_eq!(
            r.uncertainty_m,
            Some(28_000.0),
            "public, not the private 20 km"
        );
        // Obscured with no public accuracy: the service's cell, never a point.
        let p = page_of(
            r#"{"total_results":1,"page":1,"per_page":1,"results":[{"id":5,"observed_on":"2026-05-02",
            "geojson":{"coordinates":[-73.7,40.8]},"obscured":true,"positional_accuracy":10,
            "public_positional_accuracy":null,"license_code":"cc-by","taxon":{"name":"Morone saxatilis"}}]}"#,
        );
        assert_eq!(p.records[0].record.uncertainty_m, Some(OBSCURED_FALLBACK_M));
        assert_eq!(p.records[0].month, Some(5));
    }

    #[test]
    fn only_open_licences_are_kept() {
        let row = |lic: &str, photo_lic: &str| {
            format!(
                r#"{{"id":7,"observed_on":"2026-06-01","geojson":{{"coordinates":[-74.0,40.7]}},
                "license_code":{lic},"taxon":{{"name":"Morone saxatilis"}},
                "photos":[{{"url":"https://p.invalid/1/square.jpg","license_code":{photo_lic}}},
                {{"url":"https://p.invalid/2/square.jpg","license_code":"cc-by-nc"}}]}}"#
            )
        };
        let body = format!(
            r#"{{"total_results":4,"page":1,"per_page":4,"results":[{},{},{},{}]}}"#,
            row("null", "\"cc0\""),
            row("\"cc-by-sa\"", "\"cc0\""),
            row("\"cc-by\"", "null"),
            row("\"cc-by\"", "\"cc-by-nd\""),
        );
        let page = page_of(&body);
        assert_eq!((page.rows, page.unlicensed), (4, 2));
        assert_eq!(page.records.len(), 2);
        // A photo with no open licence of its own is not shown; the next open one is.
        for r in &page.records {
            let p = r.record.photo.as_ref().unwrap();
            assert_eq!(p.url, "https://p.invalid/2/medium.jpg");
            assert_eq!(p.licence.as_deref(), Some("cc-by-nc"));
        }
        // No open photo at all: the record stays, with no photo.
        let none = page_of(
            r#"{"results":[{"id":8,"geojson":{"coordinates":[-74.0,40.7]},"license_code":"cc0",
            "taxon":{"name":"Morone saxatilis"},"photos":[{"url":"https://s.invalid/1/square.jpg","license_code":null}]}]}"#,
        );
        assert_eq!(none.records.len(), 1);
        assert!(none.records[0].record.photo.is_none());
    }

    #[test]
    fn no_user_field_survives() {
        let body = r#"{"total_results":1,"page":1,"per_page":1,"results":[{"id":9,
            "observed_on":"2026-06-01","geojson":{"coordinates":[-74.0,40.7]},"license_code":"cc0",
            "taxon":{"name":"Morone saxatilis"},
            "user":{"id":424242,"login":"synthetic_login","name":"Synthetic Observer",
                    "icon":"https://u.invalid/icons/synthetic_icon.jpg","icon_url":"https://u.invalid/i.jpg"},
            "identifications":[{"user":{"login":"synthetic_identifier","id":434343}}],
            "photos":[{"url":"https://p.invalid/1/square.jpg","license_code":"cc0",
                       "attribution":"(c) Synthetic Observer, some rights reserved (CC0)"}]}]}"#;
        let page = page_of(body);
        assert_eq!(page.records.len(), 1);
        let json =
            serde_json::to_string(&page.records.iter().map(|r| &r.record).collect::<Vec<_>>())
                .unwrap();
        let debug = format!("{page:?}");
        for text in [json.as_str(), debug.as_str()] {
            for word in [
                "synthetic_login",
                "Synthetic Observer",
                "synthetic_icon",
                "synthetic_identifier",
                "424242",
                "434343",
                "login",
                "icon",
                "user",
            ] {
                assert!(!text.contains(word), "{word} in {text}");
            }
        }
    }

    #[test]
    fn the_fixture_holds_no_user_field_either() {
        let raw = String::from_utf8(fixture_bytes("tier2", FIXTURE)).unwrap();
        assert!(!raw.contains("\"user\"") && !raw.contains("login"));
    }

    #[test]
    fn rows_that_cannot_be_drawn_are_counted() {
        let page = page_of(
            r#"{"results":[
              {"id":1,"license_code":"cc0","taxon":{"name":"x"}},
              {"id":2,"license_code":"cc0","geojson":{"coordinates":[-74.0,95.0]},"taxon":{"name":"x"}},
              {"license_code":"cc0","geojson":{"coordinates":[-74.0,40.0]},"taxon":{"name":"x"}},
              {"id":4,"license_code":"cc0","geojson":{"coordinates":[-74.0,40.0]}},
              "junk"]}"#,
        );
        assert_eq!((page.rows, page.skipped, page.records.len()), (5, 5, 0));
    }

    #[test]
    fn paging_follows_the_total_and_the_window() {
        let row = r#"{"id":1,"license_code":"cc0","geojson":{"coordinates":[-74.0,40.0]},"taxon":{"name":"x"}}"#;
        let next = |total: u32, page: u32| {
            page_of(&format!(
                r#"{{"total_results":{total},"page":{page},"per_page":100,"results":[{row}]}}"#
            ))
            .next_page
        };
        assert_eq!(next(250, 1), Some(2));
        assert_eq!(next(250, 3), None);
        assert_eq!(
            next(99_999, 99),
            Some(100),
            "page 100 x 100 is the last allowed"
        );
        assert_eq!(next(99_999, 100), None);
    }

    #[test]
    fn errors_are_typed() {
        let limited = Response {
            retry_after: Some(30),
            ..Response::new(429, b"")
        };
        let e = parse_page(&limited).unwrap_err();
        assert_eq!(e.retry_after_s(), Some(30));
        assert_eq!(
            parse_page(&Response::new(429, b"")).unwrap_err(),
            InatError::RateLimited {
                retry_after_s: DEFAULT_BACKOFF_S
            }
        );
        assert_eq!(
            parse_page(&Response::new(500, b"{}")).unwrap_err(),
            InatError::Status { status: 500 }
        );
        assert!(matches!(
            parse_page(&Response::new(200, b"nope")),
            Err(InatError::Parse(_))
        ));
        assert!(matches!(
            parse_page(&Response::new(200, b"{}")),
            Err(InatError::Parse(_))
        ));
        assert_eq!(attribution().source, SourceId::INATURALIST);
    }
}
