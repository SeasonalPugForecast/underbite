//! New York State DEC stocking lists from the state's open data portal (Socrata).
//!
//! Two datasets, both published as JSON arrays of rows whose every value is a string:
//!
//! - `e52k-ymww`, the actual lists since 2011: `year`, `county`, `waterbody`, `town`, `month` (an
//!   English month name), `number`, `species`, `size_inches` ("6.7"). The numbers are estimates and
//!   a year's list appears about mid-May of the following year, so the current year is absent.
//! - `d9y2-n436`, the current season's planned spring trout stocking: `year`, `county`, `town`,
//!   `waterbody`, `date` (free text such as "March - third week"), `number`, `species_name`,
//!   `size_inches` (text with units such as "12 -14 inches"). Planned covers spring trout only.
//!
//! This module is host-neutral: it builds the GET requests and parses the answers into
//! [`StockingEvent`]s, and sends and caches nothing. No application token is used. Every event
//! carries what the data says (`planned`, the month and year, or the date text); nothing here
//! decides whether an event is recent, and the autumn stocking appears in neither dataset.
//!
//! [`plan`] picks the catalogue's waterbodies near a spot and builds the requests for them;
//! [`requests`] does the same for a known county and waterbody list.

use serde_json::Value;
use underbite_core::model::{LengthRange, StockingEvent, StockingWhen};
use underbite_core::provenance::{Attribution, SourceId};

use super::catalogue::{Catalogue, canonical_county};
use crate::request::QueryBuilder;
use crate::source::SourceError;

/// Base URL of the portal's resource endpoints.
pub const BASE: &str = "https://data.ny.gov/resource";

/// Most rows asked of one request.
pub const ROW_LIMIT: u32 = 1000;

/// Most waterbodies named in one request, which keeps the URL short.
pub const MAX_WATERBODIES: usize = 25;

const METRES_PER_INCH: f64 = 0.0254;

/// Which list a request or an answer belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Dataset {
    /// `e52k-ymww`: stockings that took place.
    Actual,
    /// `d9y2-n436`: the current season's planned spring trout stocking.
    Planned,
}

impl Dataset {
    /// The portal's dataset id.
    pub fn id(self) -> &'static str {
        match self {
            Self::Actual => "e52k-ymww",
            Self::Planned => "d9y2-n436",
        }
    }

    /// Parses an answer from this dataset.
    ///
    /// # Errors
    /// [`SourceError::Parse`] when the body is not the expected array of rows.
    pub fn parse(self, body: &[u8]) -> Result<Vec<StockingEvent>, SourceError> {
        match self {
            Self::Actual => parse_actual(body),
            Self::Planned => parse_planned(body),
        }
    }
}

/// One GET the host sends and the dataset whose parser reads the answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StockingRequest {
    /// The full URL.
    pub url: String,
    /// The request headers (`Accept`); no token header.
    pub headers: Vec<(&'static str, String)>,
    /// The list the answer is from.
    pub dataset: Dataset,
}

/// The credit every display of these lists shows.
pub fn attribution() -> Attribution {
    Attribution {
        source: SourceId::NY_DEC_STOCKING,
        credit: "NYS DEC (stocking numbers are estimates)".into(),
        url: Some("https://data.ny.gov/".into()),
    }
}

/// A `SoQL` string literal: single-quoted, apostrophes doubled.
fn literal(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

fn squash(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The requests for `waterbodies` of `county`: the actual list, newest year first, then the
/// planned list. Names and county match without regard to case; at most [`MAX_WATERBODIES`]
/// waterbodies are named, and nothing is requested for none.
pub fn requests(county: &str, waterbodies: &[&str]) -> Vec<StockingRequest> {
    let mut names: Vec<String> = Vec::new();
    for w in waterbodies {
        let name = squash(w);
        if !name.is_empty() && !names.iter().any(|n| n.eq_ignore_ascii_case(&name)) {
            names.push(name);
        }
    }
    names.truncate(MAX_WATERBODIES);
    let county = squash(county);
    if names.is_empty() || county.is_empty() {
        return Vec::new();
    }
    let list = names
        .iter()
        .map(|n| literal(&n.to_uppercase()))
        .collect::<Vec<_>>()
        .join(",");
    let filter = format!(
        "upper(county)={} AND upper(waterbody) in ({list})",
        literal(&county.to_uppercase())
    );
    [
        (Dataset::Actual, "year DESC,waterbody,month"),
        (Dataset::Planned, "waterbody,date"),
    ]
    .into_iter()
    .map(|(dataset, order)| StockingRequest {
        url: QueryBuilder::new(&format!("{BASE}/{}.json", dataset.id()))
            .param("$where", &filter)
            .param("$order", order)
            .param("$limit", ROW_LIMIT.to_string())
            .finish(),
        headers: vec![("Accept", "application/json".to_owned())],
        dataset,
    })
    .collect()
}

/// The requests for the catalogue's waterbodies within `radius_m` metres of a point: placed
/// waterbodies only, nearest first, grouped by county in the order of the nearest of each. Empty
/// when none is near.
pub fn plan(
    catalogue: &Catalogue,
    latitude: f64,
    longitude: f64,
    radius_m: f64,
) -> Vec<StockingRequest> {
    let mut counties: Vec<(String, Vec<&str>)> = Vec::new();
    for (placed, _) in catalogue
        .near(latitude, longitude, radius_m)
        .into_iter()
        .take(MAX_WATERBODIES)
    {
        let county = canonical_county(&placed.county);
        match counties.iter_mut().find(|(c, _)| *c == county) {
            Some((_, names)) => names.push(&placed.waterbody),
            None => counties.push((county, vec![&placed.waterbody])),
        }
    }
    counties
        .iter()
        .flat_map(|(county, names)| requests(county, names))
        .collect()
}

/// Parses an answer from the actual list into events, in the order the portal sent them. A month
/// that is not an English month name is kept as text with the year; a row without a year, a
/// waterbody or a species is skipped.
///
/// # Errors
/// [`SourceError::Parse`] when the body is not an array of rows (a portal error object included).
pub fn parse_actual(body: &[u8]) -> Result<Vec<StockingEvent>, SourceError> {
    Ok(rows(body)?
        .iter()
        .filter_map(|row| {
            let year = text(row, "year")?.parse::<u16>().ok()?;
            let month_text = text(row, "month")?;
            let when = match month(month_text) {
                Some(month) => StockingWhen::Month { year, month },
                None => StockingWhen::DateText(format!("{month_text} {year}")),
            };
            event(row, "species", when, false)
        })
        .collect())
}

/// Parses an answer from the planned list into events, in the order the portal sent them. The
/// date is the list's free text, kept as written.
///
/// # Errors
/// [`SourceError::Parse`] when the body is not an array of rows.
pub fn parse_planned(body: &[u8]) -> Result<Vec<StockingEvent>, SourceError> {
    Ok(rows(body)?
        .iter()
        .filter_map(|row| {
            let when = StockingWhen::DateText(text(row, "date")?.to_owned());
            event(row, "species_name", when, true)
        })
        .collect())
}

fn rows(body: &[u8]) -> Result<Vec<Value>, SourceError> {
    match serde_json::from_slice::<Value>(body.trim_ascii()) {
        Ok(Value::Array(rows)) => Ok(rows),
        Ok(Value::Object(o)) => Err(SourceError::Parse(format!(
            "an error answer, not rows: {}",
            o.get("message")
                .and_then(Value::as_str)
                .unwrap_or("no message")
        ))),
        Ok(_) => Err(SourceError::Parse("not an array of rows".into())),
        Err(e) => Err(SourceError::Parse(format!("not JSON: {e}"))),
    }
}

/// A non-empty, trimmed string field.
fn text<'a>(row: &'a Value, field: &str) -> Option<&'a str> {
    let s = row.get(field)?.as_str()?.trim();
    (!s.is_empty()).then_some(s)
}

fn event(
    row: &Value,
    species_field: &str,
    when: StockingWhen,
    planned: bool,
) -> Option<StockingEvent> {
    Some(StockingEvent {
        waterbody: squash(text(row, "waterbody")?),
        county: text(row, "county").map(canonical_county),
        town: text(row, "town").map(str::to_owned),
        when,
        species_text: text(row, species_field)?.to_owned(),
        number_estimate: text(row, "number").and_then(|n| n.replace(',', "").parse().ok()),
        size_m: text(row, "size_inches").and_then(size),
        planned,
        source: SourceId::NY_DEC_STOCKING,
    })
}

/// 1 to 12 for an English month name or its three-letter abbreviation.
fn month(name: &str) -> Option<u8> {
    const NAMES: [&str; 12] = [
        "january",
        "february",
        "march",
        "april",
        "may",
        "june",
        "july",
        "august",
        "september",
        "october",
        "november",
        "december",
    ];
    let lower = name.trim().trim_end_matches('.').to_lowercase();
    NAMES
        .iter()
        .position(|m| *m == lower || (lower.len() == 3 && m.starts_with(&lower)))
        .and_then(|i| u8::try_from(i + 1).ok())
}

/// A length in inches ("7", "6.7", "8 inches") or a range ("12 -14 inches", "9 - 10 inches"),
/// in metres. Anything else is absent.
fn size(text: &str) -> Option<LengthRange> {
    let mut numbers: Vec<f64> = Vec::new();
    let mut current = String::new();
    for c in text.chars().chain(std::iter::once(' ')) {
        if c.is_ascii_digit() || (c == '.' && !current.is_empty()) {
            current.push(c);
        } else if !current.is_empty() {
            numbers.push(current.trim_end_matches('.').parse().ok()?);
            current.clear();
        }
    }
    let (lo, hi) = match numbers.as_slice() {
        [a] => (*a, *a),
        [a, b] => (a.min(*b), a.max(*b)),
        _ => return None,
    };
    if lo <= 0.0 {
        return None;
    }
    #[allow(clippy::cast_possible_truncation)]
    Some(LengthRange {
        min_m: (lo * METRES_PER_INCH) as f32,
        max_m: (hi * METRES_PER_INCH) as f32,
    })
}

#[cfg(test)]
#[allow(clippy::assert_is_empty)]
mod tests {
    use super::*;

    const ACTUAL: &[u8] = include_bytes!(
        "../../../../tests/fixtures/raw/tier2/socrata-ny-e52k-ymww-westchester-2025.json"
    );
    const PLANNED: &[u8] = include_bytes!(
        "../../../../tests/fixtures/raw/tier2/socrata-ny-d9y2-n436-westchester.json"
    );

    const CATALOGUE: &str = r#"{"source":"test","recorded":"2026-11-17","placed":[
      {"county":"Westchester","waterbody":"Kensico Reservoir","name":"Kensico Reservoir",
       "latitude":41.1,"longitude":-73.75,"kind":"Lake","source":"manual"},
      {"county":"Westchester","waterbody":"Dogwood Pond","name":"Dogwood Pond",
       "latitude":41.25,"longitude":-73.9,"kind":"Pond","source":"manual"},
      {"county":"Putnam","waterbody":"Tilly Foster's Brook","name":"Tilly Foster Brook",
       "latitude":41.4,"longitude":-73.6,"kind":"River","source":"manual"},
      {"county":"Essex","waterbody":"Far Lake","name":"Far Lake",
       "latitude":44.0,"longitude":-74.0,"kind":"Lake","source":"manual"}],
      "unmatched":[]}"#;

    fn decoded(url: &str) -> String {
        url.replace("%20", " ")
            .replace("%27", "'")
            .replace("%28", "(")
            .replace("%29", ")")
            .replace("%3D", "=")
    }

    #[test]
    fn actual_fixture_parses() {
        let events = parse_actual(ACTUAL).unwrap();
        assert_eq!(events.len(), 5);
        let first = &events[0];
        assert_eq!(first.waterbody, "Kensico Reservoir");
        assert_eq!(first.county.as_deref(), Some("Westchester"));
        assert_eq!(first.town.as_deref(), Some("Harrison"));
        assert_eq!(
            first.when,
            StockingWhen::Month {
                year: 2025,
                month: 4
            }
        );
        assert_eq!(first.number_estimate, Some(8920));
        assert!(!first.planned);
        let size = first.size_m.unwrap();
        assert!((f64::from(size.min_m) - 6.7 * 0.0254).abs() < 1e-6);
        assert_eq!(size.min_m, size.max_m);
        insta::assert_yaml_snapshot!(events);
    }

    #[test]
    fn planned_fixture_parses() {
        let events = parse_planned(PLANNED).unwrap();
        assert_eq!(events.len(), 5);
        assert!(events.iter().all(|e| e.planned));
        assert_eq!(
            events[0].when,
            StockingWhen::DateText("March - third week".into())
        );
        assert_eq!(events[0].species_text, "Brown Trout");
        let range = events[1].size_m.unwrap();
        assert!((f64::from(range.min_m) - 12.0 * 0.0254).abs() < 1e-6);
        assert!((f64::from(range.max_m) - 14.0 * 0.0254).abs() < 1e-6);
        insta::assert_yaml_snapshot!(events);
    }

    #[test]
    fn dataset_parse_dispatches() {
        assert_eq!(Dataset::Actual.parse(ACTUAL).unwrap().len(), 5);
        assert!(Dataset::Planned.parse(PLANNED).unwrap()[0].planned);
    }

    #[test]
    fn sizes() {
        let m = |t: &str| size(t).map(|r| (r.min_m, r.max_m));
        assert_eq!(m("8 inches"), Some((0.2032, 0.2032)));
        assert_eq!(m("9 - 10 inches"), Some((0.2286, 0.254)));
        assert_eq!(m("14 -12 inches"), Some((0.3048, 0.3556)));
        assert_eq!(m(""), None);
        assert_eq!(m("various"), None);
        assert_eq!(m("0"), None);
        assert_eq!(m("6 to 8 or 10"), None);
    }

    #[test]
    fn months() {
        assert_eq!(month("April"), Some(4));
        assert_eq!(month(" december "), Some(12));
        assert_eq!(month("Sept"), None);
        assert_eq!(month("Oct"), Some(10));
        assert_eq!(month("Spring"), None);
    }

    #[test]
    fn unknown_month_is_kept_as_text_and_bad_rows_are_skipped() {
        let body = br#"[{"year":"2024","county":"SULLIVAN","waterbody":"  Mongaup  Pond ","month":"Spring","species":"Brown Trout"},
            {"year":"x","waterbody":"A","month":"May","species":"Trout"},
            {"year":"2024","month":"May","species":"Trout"},
            {"year":"2024","county":"Putnam","waterbody":"B","month":"May","species":"Trout","number":"1,200","size_inches":"n/a"}]"#;
        let events = parse_actual(body).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].waterbody, "Mongaup Pond");
        assert_eq!(events[0].county.as_deref(), Some("Sullivan"));
        assert_eq!(events[0].when, StockingWhen::DateText("Spring 2024".into()));
        assert_eq!(events[1].number_estimate, Some(1200));
        assert_eq!(events[1].size_m, None);
    }

    #[test]
    fn bad_bodies_are_errors_and_empty_is_empty() {
        assert!(parse_actual(b"[]").unwrap().is_empty());
        assert!(matches!(parse_actual(b"nope"), Err(SourceError::Parse(_))));
        assert!(matches!(
            parse_planned(b"\"x\""),
            Err(SourceError::Parse(_))
        ));
        let err =
            parse_actual(br#"{"error":true,"message":"Query coordinator error"}"#).unwrap_err();
        assert!(err.to_string().contains("Query coordinator error"), "{err}");
    }

    #[test]
    fn requests_quote_apostrophes_and_carry_no_token() {
        let reqs = requests(
            "St Lawrence",
            &[
                "Tilly Foster's Brook",
                "tilly  foster's brook",
                "Dogwood Pond",
            ],
        );
        assert_eq!(reqs.len(), 2);
        assert_eq!(reqs[0].dataset, Dataset::Actual);
        assert_eq!(reqs[1].dataset, Dataset::Planned);
        let actual = decoded(&reqs[0].url);
        assert!(
            actual.starts_with("https://data.ny.gov/resource/e52k-ymww.json?"),
            "{actual}"
        );
        assert!(actual.contains("upper(county)='ST LAWRENCE'"), "{actual}");
        assert!(
            actual.contains("in ('TILLY FOSTER''S BROOK','DOGWOOD POND')"),
            "{actual}"
        );
        assert!(
            reqs[1]
                .url
                .starts_with("https://data.ny.gov/resource/d9y2-n436.json?")
        );
        for r in &reqs {
            assert!(!r.url.to_lowercase().contains("token"));
            assert!(
                r.headers
                    .iter()
                    .all(|(k, _)| !k.to_lowercase().contains("token"))
            );
        }
        insta::assert_debug_snapshot!(reqs);
    }

    #[test]
    fn nothing_to_ask_is_no_request() {
        assert!(requests("Westchester", &[]).is_empty());
        assert!(requests("Westchester", &["  "]).is_empty());
        assert!(requests(" ", &["Pond"]).is_empty());
    }

    #[test]
    fn plan_groups_the_near_waterbodies_by_county() {
        let catalogue = Catalogue::from_json(CATALOGUE).unwrap();
        let reqs = plan(&catalogue, 41.1, -73.75, 25_000.0);
        assert_eq!(reqs.len(), 2);
        let url = decoded(&reqs[0].url);
        assert!(
            url.contains("'WESTCHESTER'") && url.contains("'KENSICO RESERVOIR','DOGWOOD POND'"),
            "{url}"
        );
        let wide = plan(&catalogue, 41.1, -73.75, 60_000.0);
        assert_eq!(wide.len(), 4);
        assert!(decoded(&wide[2].url).contains("'PUTNAM'"));
        assert!(decoded(&wide[2].url).contains("'TILLY FOSTER''S BROOK'"));
        assert!(plan(&catalogue, 10.0, 10.0, 25_000.0).is_empty());
    }

    #[test]
    fn credit_names_the_agency() {
        let a = attribution();
        assert_eq!(a.source, SourceId::NY_DEC_STOCKING);
        assert!(a.credit.starts_with("NYS DEC"));
    }
}
