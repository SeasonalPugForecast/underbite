//! USGS Nonindigenous Aquatic Species: records by state and HUC, filtered by distance.
//!
//! Sans-IO, for the map. The service has no box or radius search, so a host asks by state (`NY`)
//! and, when it knows it, the 8-digit hydrologic unit of the spot ([`search_request`]), pages with
//! [`NasPage::next_offset`], and keeps the records near the spot with [`near`]. The spot's own
//! position never leaves the device: only the state and the unit are sent.
//!
//! The database holds records of introduced species (each has a [`NasRecord::status`] such as
//! `established` or `failed`); it is not an atlas of where a species lives. Quirks the parser
//! handles: `endOfRecords` is a string, `count` is the size of the page and not a total, numeric
//! fields can be `null`, and `date` is free text (`1996`, `2005-7`, `1936-7-10`).
//!
//! The data are U.S. Government work in the public domain, so every record carries the licence
//! [`LICENCE`]. Only the fields a [`SpeciesRecord`] carries, and the few in [`NasRecord`], are
//! read: references (whose authors are people), comments and localities are not kept. Credit:
//! [`attribution`], which includes the citation USGS suggests, and [`DISCLAIMER`].

use std::collections::BTreeSet;

use serde::Deserialize;
use serde_json::Value;
use underbite_core::discover::rank::distance_m;
use underbite_core::model::nearby::SpeciesRecord;
use underbite_core::provenance::{Attribution, SourceId};
use underbite_core::time::{LocalDate, parse_date};

use super::Licence;
use crate::request::{QueryBuilder, Response};

const SEARCH_URL: &str = "https://nas.er.usgs.gov/api/v2/occurrence/search";

/// The service's page for one record; the key follows.
const RECORD_PAGE: &str = "https://nas.er.usgs.gov/queries/SpecimenViewer.aspx?SpecimenID=";

/// The database's home page, linked from the credit.
pub const NAS_URL: &str = "https://nas.er.usgs.gov/";

/// Records asked for per page (about 1 KB of JSON each).
pub const PAGE_SIZE: u32 = 500;

/// The largest `limit` known to be accepted.
pub const MAX_LIMIT: u32 = 1_000;

/// The licence every record carries (see [`Licence::PublicDomain`]).
pub const LICENCE: &str = "public-domain";

/// The data disclaimer of the database, shown with its records.
pub const DISCLAIMER: &str = "These data vary in accuracy, scale, completeness, extent of coverage and origin. It is the user's responsibility to use these data consistent with their intended purpose and within stated limitations.";

/// The citation USGS suggests, for data accessed on `accessed`.
pub fn citation(accessed: LocalDate) -> String {
    format!(
        "U.S. Geological Survey. {}. Nonindigenous Aquatic Species Database. Gainesville, Florida. Accessed {:04}-{:02}-{:02}.",
        accessed.year, accessed.year, accessed.month, accessed.day
    )
}

/// The credit as an [`Attribution`], for data accessed on `accessed`.
pub fn attribution(accessed: LocalDate) -> Attribution {
    Attribution {
        source: SourceId::USGS_NAS,
        credit: citation(accessed),
        url: Some(NAS_URL.into()),
    }
}

/// Which taxa to ask for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NasTaxon {
    /// Every taxon.
    Any,
    /// A group as the database names it: `Fishes`, `Crustaceans-Crayfish`, `Mollusks-Bivalves`.
    Group(String),
    /// A genus, e.g. `Channa`.
    Genus(String),
    /// A species: its genus and epithet, e.g. `Channa` and `argus`.
    Species { genus: String, species: String },
    /// The database's own species id.
    SpeciesId(u32),
}

/// Where and what to search.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NasQuery {
    /// The two-letter state abbreviation, e.g. `NY`.
    pub state: Option<String>,
    /// The 8-digit hydrologic unit, e.g. `02030101`. It is independent of the state: a unit
    /// may cross state lines.
    pub huc8: Option<String>,
    pub taxon: NasTaxon,
    /// Only records from these years, both included.
    pub years: Option<(i32, i32)>,
}

/// One GET for the host to send. The URL carries no key or personal data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NasRequest {
    pub url: String,
}

/// A NAS request could not be built, or its response read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NasError {
    /// The request asks for something that cannot be sent.
    #[error("invalid NAS request: {0}")]
    InvalidRequest(&'static str),
    /// The service answered with an HTTP failure.
    #[error("NAS answered HTTP {status}")]
    Status { status: u16 },
    /// The body is not the expected JSON.
    #[error("the NAS response is not the expected JSON: {0}")]
    Parse(String),
}

fn word(s: &str, what: &'static str) -> Result<String, NasError> {
    let s = s.trim();
    if s.is_empty() {
        Err(NasError::InvalidRequest(what))
    } else {
        Ok(s.to_owned())
    }
}

/// One page of the records of a state and/or hydrologic unit: `offset` records skipped (0, then
/// [`NasPage::next_offset`]) and `limit` asked for, between 1 and [`MAX_LIMIT`] ([`PAGE_SIZE`] is
/// a good value).
///
/// # Errors
/// [`NasError::InvalidRequest`] when neither a state nor a unit is given (that would be the whole
/// country), for a state that is not two letters, a unit that is not eight digits, an empty taxon
/// name or a year range that runs backwards.
pub fn search_request(query: &NasQuery, offset: u32, limit: u32) -> Result<NasRequest, NasError> {
    let state = query
        .state
        .as_deref()
        .map(|s| {
            let s = s.trim();
            if s.len() == 2 && s.bytes().all(|b| b.is_ascii_alphabetic()) {
                Ok(s.to_ascii_uppercase())
            } else {
                Err(NasError::InvalidRequest("state"))
            }
        })
        .transpose()?;
    let huc8 = query
        .huc8
        .as_deref()
        .map(|h| {
            let h = h.trim();
            if h.len() == 8 && h.bytes().all(|b| b.is_ascii_digit()) {
                Ok(h.to_owned())
            } else {
                Err(NasError::InvalidRequest("huc8"))
            }
        })
        .transpose()?;
    if state.is_none() && huc8.is_none() {
        return Err(NasError::InvalidRequest("place"));
    }
    let mut q = QueryBuilder::new(SEARCH_URL);
    if let Some(state) = state {
        q = q.param("state", state);
    }
    if let Some(huc8) = huc8 {
        q = q.param("huc8", huc8);
    }
    q = match &query.taxon {
        NasTaxon::Any => q,
        NasTaxon::Group(g) => q.param("group", word(g, "group")?),
        NasTaxon::Genus(g) => q.param("genus", word(g, "genus")?),
        NasTaxon::Species { genus, species } => q
            .param("genus", word(genus, "genus")?)
            .param("species", word(species, "species")?),
        NasTaxon::SpeciesId(id) => q.param("species_ID", id.to_string()),
    };
    if let Some((from, to)) = query.years {
        if from > to {
            return Err(NasError::InvalidRequest("years"));
        }
        q = q.param("year", format!("{from},{to}"));
    }
    let url = q
        .param("limit", limit.clamp(1, MAX_LIMIT).to_string())
        .param("offset", offset.to_string())
        .finish();
    Ok(NasRequest { url })
}

/// A record with what NAS adds beyond the [`SpeciesRecord`].
#[derive(Clone, Debug, PartialEq)]
pub struct NasRecord {
    pub record: SpeciesRecord,
    /// The common name, e.g. `Northern Snakehead`.
    pub common_name: Option<String>,
    /// The introduction's status as the database gives it: `established`, `collected`,
    /// `failed`, `extirpated`, …
    pub status: Option<String>,
    /// The record's 8-digit hydrologic unit.
    pub huc8: Option<String>,
    /// The year collected.
    pub year: Option<i32>,
    /// The month collected (1 to 12), also when the day is not known.
    pub month: Option<u8>,
    /// How the database rates the position: `Accurate`, `Approximate`, …
    pub accuracy: Option<String>,
    /// Whether the position is the centre of an area (a county or a unit), not a site.
    pub centroid: bool,
}

/// The records one page returned.
#[derive(Clone, Debug, PartialEq)]
pub struct NasPage {
    pub records: Vec<NasRecord>,
    /// Rows the page held.
    pub rows: usize,
    /// Rows left out for a missing or invalid position, id or taxon.
    pub skipped: usize,
    /// The offset of the next page while the service may have more: it did not say the records
    /// ended and the page was full.
    pub next_offset: Option<u32>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Row {
    key: Option<Value>,
    scientific_name: Option<String>,
    genus: Option<String>,
    species: Option<String>,
    common_name: Option<String>,
    decimal_latitude: Option<f64>,
    decimal_longitude: Option<f64>,
    lat_long_accuracy: Option<String>,
    #[serde(rename = "Centroid Type")]
    centroid_type: Option<String>,
    huc8: Option<String>,
    year: Option<i32>,
    month: Option<u8>,
    day: Option<u8>,
    date: Option<String>,
    status: Option<String>,
}

fn nonempty(s: Option<&str>) -> Option<String> {
    s.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// The day collected, from `year`, `month` and `day`, else from a full `date` (`1936-7-10`).
fn observed_on(row: &Row) -> Option<LocalDate> {
    let from_text = || -> Option<(i32, u8, u8)> {
        let mut parts = row.date.as_deref()?.trim().split('-');
        let y = parts.next()?.parse().ok()?;
        let m = parts.next()?.parse().ok()?;
        let d = parts.next()?.parse().ok()?;
        parts.next().is_none().then_some((y, m, d))
    };
    let (y, m, d) = row
        .year
        .zip(row.month)
        .zip(row.day)
        .map(|((y, m), d)| (y, m, d))
        .or_else(from_text)?;
    parse_date(&format!("{y:04}-{m:02}-{d:02}")).ok()
}

fn read_row(value: Value) -> Option<NasRecord> {
    let row: Row = serde_json::from_value(value).ok()?;
    let id = match row.key.as_ref()? {
        Value::Number(n) => n.to_string(),
        Value::String(s) => nonempty(Some(s))?,
        _ => return None,
    };
    let (lat, lon) = (row.decimal_latitude?, row.decimal_longitude?);
    let null_island = lat.abs() < f64::EPSILON && lon.abs() < f64::EPSILON;
    if null_island || !((-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon)) {
        return None;
    }
    let taxon = nonempty(row.scientific_name.as_deref()).or_else(|| {
        let genus = nonempty(row.genus.as_deref())?;
        Some(match nonempty(row.species.as_deref()) {
            Some(species) => format!("{genus} {species}"),
            None => genus,
        })
    })?;
    let record = SpeciesRecord {
        source: SourceId::USGS_NAS,
        url: Some(format!("{RECORD_PAGE}{id}")),
        id,
        taxon,
        latitude: lat,
        longitude: lon,
        uncertainty_m: None,
        obscured: false,
        observed_on: observed_on(&row),
        licence: Some(LICENCE.to_owned()),
        photo: None,
    };
    Some(NasRecord {
        record,
        common_name: nonempty(row.common_name.as_deref()),
        status: nonempty(row.status.as_deref()),
        huc8: nonempty(row.huc8.as_deref()),
        year: row.year,
        month: row.month.filter(|m| (1..=12).contains(m)),
        accuracy: nonempty(row.lat_long_accuracy.as_deref()),
        centroid: nonempty(row.centroid_type.as_deref()).is_some(),
    })
}

/// `endOfRecords`, which the service sends as a string (`"true"`); a boolean is read too.
fn flag(v: Option<&Value>) -> Option<bool> {
    match v? {
        Value::Bool(b) => Some(*b),
        Value::String(s) => match s.trim() {
            t if t.eq_ignore_ascii_case("true") => Some(true),
            t if t.eq_ignore_ascii_case("false") => Some(false),
            _ => None,
        },
        _ => None,
    }
}

/// Reads one page of [`search_request`].
///
/// # Errors
/// [`NasError::Status`] for an HTTP failure and [`NasError::Parse`] for a body that holds no
/// result list.
pub fn parse_page(response: &Response<'_>) -> Result<NasPage, NasError> {
    if !(200..=299).contains(&response.status) {
        return Err(NasError::Status {
            status: response.status,
        });
    }
    let mut v: Value =
        serde_json::from_slice(response.body).map_err(|e| NasError::Parse(e.to_string()))?;
    let Some(Value::Array(rows)) = v.get_mut("results").map(Value::take) else {
        return Err(NasError::Parse("no results".to_owned()));
    };
    let n = rows.len();
    let records: Vec<NasRecord> = rows.into_iter().filter_map(read_row).collect();
    let end = flag(v.get("endOfRecords")).unwrap_or(true);
    let limit = v.get("limit").and_then(Value::as_u64);
    let offset = v.get("offset").and_then(Value::as_u64).unwrap_or(0);
    let full = n > 0 && limit.is_none_or(|l| n as u64 >= l);
    let next_offset = (!end && full)
        .then(|| u32::try_from(offset.saturating_add(n as u64)).ok())
        .flatten();
    Ok(NasPage {
        skipped: n - records.len(),
        records,
        rows: n,
        next_offset,
    })
}

/// The records within `radius_m` metres of a point, nearest first (ties by id), each with its
/// distance; a record that appears twice (pages of a state and a unit overlap) is kept once.
pub fn near(
    records: &[NasRecord],
    latitude: f64,
    longitude: f64,
    radius_m: f64,
) -> Vec<(&NasRecord, f64)> {
    let mut seen = BTreeSet::new();
    let mut out: Vec<(&NasRecord, f64)> = records
        .iter()
        .filter(|r| seen.insert(r.record.id.as_str()))
        .map(|r| {
            let d = distance_m(latitude, longitude, r.record.latitude, r.record.longitude);
            (r, d)
        })
        .filter(|(_, d)| d.is_finite() && *d <= radius_m)
        .collect();
    out.sort_by(|a, b| {
        a.1.total_cmp(&b.1)
            .then_with(|| a.0.record.id.cmp(&b.0.record.id))
    });
    out
}

/// Whether a record's licence is the one NAS records carry.
pub fn is_public_domain(record: &SpeciesRecord) -> bool {
    Licence::of(record) == Some(Licence::PublicDomain)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::species::month_counts;
    use crate::testutil::fixture_bytes;

    /// The Battery, New York Harbor.
    const BATTERY: (f64, f64) = (40.7033, -74.0170);

    fn query(state: Option<&str>, huc8: Option<&str>, taxon: NasTaxon) -> NasQuery {
        NasQuery {
            state: state.map(str::to_owned),
            huc8: huc8.map(str::to_owned),
            taxon,
            years: None,
        }
    }

    fn page(source: &str, name: &str) -> NasPage {
        let body = fixture_bytes(source, name);
        parse_page(&Response::new(200, &body)).unwrap()
    }

    fn params(url: &str) -> Vec<&str> {
        url.split_once('?').unwrap().1.split('&').collect()
    }

    #[test]
    fn requests_name_the_state_and_unit_never_a_position() {
        let mut q = query(
            Some(" ny "),
            Some("02030101"),
            NasTaxon::Group("Fishes".into()),
        );
        q.years = Some((2020, 2026));
        let r = search_request(&q, 500, PAGE_SIZE).unwrap();
        assert!(
            r.url
                .starts_with("https://nas.er.usgs.gov/api/v2/occurrence/search?")
        );
        assert_eq!(
            params(&r.url),
            [
                "state=NY",
                "huc8=02030101",
                "group=Fishes",
                "year=2020,2026",
                "limit=500",
                "offset=500",
            ]
        );
        let s = search_request(
            &query(
                Some("NY"),
                None,
                NasTaxon::Species {
                    genus: "Channa".into(),
                    species: "argus".into(),
                },
            ),
            0,
            5_000,
        )
        .unwrap();
        assert_eq!(
            params(&s.url),
            [
                "state=NY",
                "genus=Channa",
                "species=argus",
                "limit=1000",
                "offset=0"
            ]
        );
        let id = search_request(
            &query(None, Some("02030102"), NasTaxon::SpeciesId(2265)),
            0,
            0,
        )
        .unwrap();
        assert_eq!(
            params(&id.url),
            ["huc8=02030102", "species_ID=2265", "limit=1", "offset=0"]
        );
        let group = search_request(
            &query(
                Some("NY"),
                None,
                NasTaxon::Group("Crustaceans-Crayfish".into()),
            ),
            0,
            10,
        )
        .unwrap();
        assert!(params(&group.url).contains(&"group=Crustaceans-Crayfish"));
        insta::assert_snapshot!(
            search_request(
                &query(Some("NY"), None, NasTaxon::Genus("Channa".into())),
                0,
                3
            )
            .unwrap()
            .url
        );
    }

    #[test]
    fn bad_requests_are_refused() {
        for (q, why) in [
            (query(None, None, NasTaxon::Any), "place"),
            (query(Some("New York"), None, NasTaxon::Any), "state"),
            (query(Some("N1"), None, NasTaxon::Any), "state"),
            (query(None, Some("0203010"), NasTaxon::Any), "huc8"),
            (query(None, Some("0203010a"), NasTaxon::Any), "huc8"),
            (
                query(Some("NY"), None, NasTaxon::Genus(" ".into())),
                "genus",
            ),
            (
                query(
                    Some("NY"),
                    None,
                    NasTaxon::Species {
                        genus: "Channa".into(),
                        species: String::new(),
                    },
                ),
                "species",
            ),
            (
                NasQuery {
                    years: Some((2026, 2020)),
                    ..query(Some("NY"), None, NasTaxon::Any)
                },
                "years",
            ),
        ] {
            assert_eq!(
                search_request(&q, 0, 10),
                Err(NasError::InvalidRequest(why)),
                "{q:?}"
            );
        }
    }

    #[test]
    fn parses_the_snakehead_fixture() {
        let p = page("tier2", "usgs-nas-occurrence-snakehead-ny.json");
        assert_eq!((p.rows, p.skipped), (3, 0));
        assert_eq!(p.next_offset, Some(3), "a full page that did not end");
        let first = &p.records[0];
        assert_eq!(first.record.source, SourceId::USGS_NAS);
        assert_eq!(first.record.id, "164584");
        assert_eq!(first.record.taxon, "Channa argus");
        assert_eq!(
            (first.record.latitude, first.record.longitude),
            (40.734_269, -73.840_69)
        );
        assert_eq!(first.record.observed_on, None, "the day is null");
        assert_eq!((first.year, first.month), (Some(2005), Some(7)));
        assert_eq!(first.common_name.as_deref(), Some("Northern Snakehead"));
        assert_eq!(first.status.as_deref(), Some("established"));
        assert_eq!(first.huc8.as_deref(), Some("02030102"));
        assert_eq!(first.accuracy.as_deref(), Some("Accurate"));
        assert!(!first.centroid);
        assert!(is_public_domain(&first.record));
        assert_eq!(
            first.record.url.as_deref(),
            Some("https://nas.er.usgs.gov/queries/SpecimenViewer.aspx?SpecimenID=164584")
        );
        assert_eq!(
            p.records[1].record.observed_on,
            Some(LocalDate {
                year: 2007,
                month: 5,
                day: 23
            })
        );
        let records: Vec<_> = p.records.iter().map(|r| &r.record).collect();
        insta::assert_yaml_snapshot!(records);
    }

    #[test]
    fn no_person_or_free_text_survives() {
        let p = page("tier2", "usgs-nas-occurrence-snakehead-ny.json");
        let json = serde_json::to_string(&p.records.iter().map(|r| &r.record).collect::<Vec<_>>())
            .unwrap();
        let debug = format!("{p:?}");
        for text in [json.as_str(), debug.as_str()] {
            for word in [
                "Cohen",
                "Mimoni",
                "author",
                "Frankenfish",
                "Meadow Lake",
                "parks department",
                "Queens",
            ] {
                assert!(!text.contains(word), "{word}");
            }
        }
    }

    #[test]
    fn records_are_kept_near_the_spot_only() {
        let p = page("tier2", "usgs-nas-occurrence-snakehead-ny.json");
        let near_battery = near(&p.records, BATTERY.0, BATTERY.1, 20_000.0);
        assert_eq!(near_battery.len(), 3);
        let d = near_battery[0].1;
        assert!((14_000.0..16_000.0).contains(&d), "{d}");
        let ids: Vec<&str> = near_battery
            .iter()
            .map(|(r, _)| r.record.id.as_str())
            .collect();
        assert_eq!(ids, ["164584", "238474", "238508"], "equal distances by id");
        assert_eq!(near(&p.records, BATTERY.0, BATTERY.1, 10_000.0).len(), 0);
        let twice: Vec<NasRecord> = p.records.iter().chain(&p.records).cloned().collect();
        assert_eq!(near(&twice, BATTERY.0, BATTERY.1, 20_000.0).len(), 3);
    }

    #[test]
    fn a_unit_crosses_state_lines_and_years_alone_have_no_day() {
        let p = page("nas", "nas-occurrence-huc8-02030101-limit5-2026-11-17.json");
        assert_eq!((p.rows, p.records.len()), (5, 5));
        assert_eq!(p.next_offset, Some(5));
        let first = &p.records[0];
        assert_eq!(first.record.taxon, "Hybopsis amblops");
        assert_eq!(first.record.observed_on, None, "the date is a year");
        assert_eq!((first.year, first.month), (Some(1996), None));
        assert_eq!(first.status.as_deref(), Some("failed"));
        let close = near(&p.records, BATTERY.0, BATTERY.1, 15_000.0);
        assert_eq!(close.len(), 1, "only the Hudson record by Manhattan");
        assert_eq!(close[0].0.record.taxon, "Scardinius erythrophthalmus");
        let last = &p.records[4];
        assert_eq!(
            last.record.observed_on,
            Some(LocalDate {
                year: 1936,
                month: 7,
                day: 10
            })
        );
    }

    #[test]
    fn months_feed_the_typical_strip() {
        let p = page(
            "nas",
            "nas-occurrence-ny-westchester-limit5-2026-11-17.json",
        );
        assert_eq!(p.records.len(), 5);
        let months = month_counts(p.records.iter().filter_map(|r| r.month));
        let got: Vec<(u8, u32)> = months.iter().map(|m| (m.month, m.count)).collect();
        assert_eq!(got, [(6, 1), (7, 2), (9, 2)]);
    }

    #[test]
    fn odd_rows_and_paging() {
        let body = br#"{"endOfRecords":"true","count":6,"offset":10,"limit":10,"results":[
          {"key":1,"genus":"Channa","species":"argus","decimalLatitude":40.7,"decimalLongitude":-74.0,
           "year":2020,"month":13,"day":null,"date":"2020-2-30","Centroid Type":"County"},
          {"key":2,"scientificName":"Channa argus","decimalLatitude":null,"decimalLongitude":-74.0},
          {"key":3,"scientificName":"Channa argus","decimalLatitude":0.0,"decimalLongitude":0.0},
          {"scientificName":"Channa argus","decimalLatitude":40.7,"decimalLongitude":-74.0},
          {"key":5,"decimalLatitude":40.7,"decimalLongitude":-74.0},
          {"key":"6","scientificName":"Channa argus","decimalLatitude":"40.7","decimalLongitude":-74.0}]}"#;
        let p = parse_page(&Response::new(200, body)).unwrap();
        assert_eq!((p.rows, p.records.len(), p.skipped), (6, 1, 5));
        assert_eq!(p.next_offset, None, "the records ended");
        let r = &p.records[0];
        assert_eq!(r.record.taxon, "Channa argus");
        assert_eq!((r.month, r.record.observed_on), (None, None));
        assert!(r.centroid);

        let more = br#"{"endOfRecords":"false","count":1,"offset":4,"limit":1,"results":[
          {"key":9,"scientificName":"Channa argus","decimalLatitude":40.7,"decimalLongitude":-74.0}]}"#;
        assert_eq!(
            parse_page(&Response::new(200, more)).unwrap().next_offset,
            Some(5)
        );
        let short = br#"{"endOfRecords":"false","count":1,"offset":0,"limit":5,"results":[
          {"key":9,"scientificName":"Channa argus","decimalLatitude":40.7,"decimalLongitude":-74.0}]}"#;
        assert_eq!(
            parse_page(&Response::new(200, short)).unwrap().next_offset,
            None,
            "a short page ends the listing"
        );
        let boolean = br#"{"endOfRecords":false,"offset":0,"results":[
          {"key":9,"scientificName":"Channa argus","decimalLatitude":40.7,"decimalLongitude":-74.0}]}"#;
        assert_eq!(
            parse_page(&Response::new(200, boolean))
                .unwrap()
                .next_offset,
            Some(1)
        );
    }

    #[test]
    fn failures_and_credit() {
        assert_eq!(
            parse_page(&Response::new(503, b"")),
            Err(NasError::Status { status: 503 })
        );
        for bad in [&b"<html>"[..], b"{}", br#"{"results":null}"#] {
            assert!(
                matches!(
                    parse_page(&Response::new(200, bad)),
                    Err(NasError::Parse(_))
                ),
                "{}",
                String::from_utf8_lossy(bad)
            );
        }
        let a = attribution(LocalDate {
            year: 2026,
            month: 11,
            day: 17,
        });
        assert_eq!(a.source, SourceId::USGS_NAS);
        assert_eq!(
            a.credit,
            "U.S. Geological Survey. 2026. Nonindigenous Aquatic Species Database. Gainesville, Florida. Accessed 2026-11-17."
        );
        assert!(DISCLAIMER.contains("vary in accuracy"));
    }
}
