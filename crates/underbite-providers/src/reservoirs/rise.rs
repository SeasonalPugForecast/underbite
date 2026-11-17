//! The USBR RISE API: pool elevation, release and inflow of a Reclamation reservoir.
//!
//! RISE is a JSON:API service. A project is a location whose catalogue items (one per parameter)
//! are known from a bundled table, so a request is one `result` query per item and carries no
//! coordinates. Navigating from a location through its catalogue records to its items is how the
//! table is built, never something done at run time. A `result` query must name an `itemId`
//! (`locationId` or `modelRunId` also pass the server, but answer with every parameter of the
//! location); filters the server does not know are silently ignored and return the whole series, so
//! every request names its item and a page whose `meta.totalItems` is implausibly large is refused.
//!
//! Values come in the item's own unit (`ft`, `cfs`) and are converted to metres and m³/s here.
//! Daily values are stamped at local midnight expressed in UTC (07:00Z in winter for Mountain time)
//! and are kept as the instants the service states. A missing value (`null`) is absent. The
//! elevation datum is a property of the location, not of the item; the table records it where it
//! is NAVD88 or NGVD29 and falls back to the pool's own gauge datum otherwise (EGM96, USGS 1912,
//! unstated).
//!
//! Reclamation data are provisional and subject to revision.

use serde::Deserialize;
use serde_json::Value;
use underbite_core::datum::{Heights, VerticalDatum};
use underbite_core::map::GeoPoint;
use underbite_core::map::tiles::haversine_m;
use underbite_core::model::{ExternalLink, ReservoirInfo};
use underbite_core::provenance::{
    Attribution, Provenance, Quality, SourceId, StationKind, StationRef, ValueKind,
};
use underbite_core::series::{Sample, Series};
use underbite_core::time::{UnixTime, UtcOffset, parse_rfc3339};

use super::cwms::{Origin, Reservoir, Role};
use crate::request::{QueryBuilder, Response, Window};
use crate::source::SourceError;

/// The service's base URL.
pub const BASE: &str = "https://data.usbr.gov/rise/api";

/// Metres per international foot.
const M_PER_FT: f64 = 0.3048;
/// Cubic metres per second per cubic foot per second.
const M3S_PER_CFS: f64 = 0.028_316_846_592;
/// The most rows the server returns for one `result` page.
const MAX_PAGE: i64 = 1000;
/// A `totalItems` above this means a filter was ignored and the whole series came back.
const IMPLAUSIBLE_TOTAL: i64 = 5000;
const DAY: i64 = 86_400;

/// A unit a catalogue item reports in (`parameterUnit`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    /// Feet, for an elevation.
    Feet,
    /// Cubic feet per second, for a release or an inflow.
    CubicFeetPerSecond,
}

impl Unit {
    /// The unit named by a catalogue item's `parameterUnit`; `None` for any other (acre-feet of
    /// storage, degrees, …).
    pub fn from_rise(name: &str) -> Option<Self> {
        match name.trim() {
            "ft" => Some(Self::Feet),
            "cfs" => Some(Self::CubicFeetPerSecond),
            _ => None,
        }
    }

    /// The canonical value (metres or m³/s) of `v` in this unit.
    fn to_canonical(self, v: f64) -> f64 {
        match self {
            Self::Feet => v * M_PER_FT,
            Self::CubicFeetPerSecond => v * M3S_PER_CFS,
        }
    }

    /// Whether this unit fits a role.
    fn fits(self, role: Role) -> bool {
        matches!(
            (self, role),
            (Self::Feet, Role::Pool) | (Self::CubicFeetPerSecond, Role::Outflow | Role::Inflow)
        )
    }
}

/// A catalogue item and the unit its results are in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Item {
    /// The RISE catalogue item id.
    pub id: u32,
    /// The item's `parameterUnit`.
    pub unit: Unit,
}

#[allow(clippy::unnecessary_wraps)] // reads as the table's `Option` column
const fn ft(id: u32) -> Option<Item> {
    Some(Item {
        id,
        unit: Unit::Feet,
    })
}

#[allow(clippy::unnecessary_wraps)]
const fn cfs(id: u32) -> Option<Item> {
    Some(Item {
        id,
        unit: Unit::CubicFeetPerSecond,
    })
}

/// A Reclamation reservoir: where it is and which items it publishes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Project {
    /// The RISE location id; the station id.
    pub location: u32,
    /// The name shown to the user.
    pub name: &'static str,
    /// Two-letter state.
    pub state: &'static str,
    /// Degrees north, from the RISE location.
    pub latitude: f64,
    /// Degrees east, from the RISE location.
    pub longitude: f64,
    /// The datum of the pool elevation.
    pub datum: VerticalDatum,
    /// Daily pool elevation (ft).
    pub pool: Option<Item>,
    /// Daily total release (cfs).
    pub outflow: Option<Item>,
    /// Daily inflow (cfs).
    pub inflow: Option<Item>,
}

/// Reclamation's web site, the operator's link.
pub const USBR_URL: &str = "https://www.usbr.gov/";

impl Project {
    /// The station id of this project: its RISE location id.
    pub fn station_id(&self) -> String {
        self.location.to_string()
    }

    /// The operator's page, for what the data does not carry.
    pub fn links(&self) -> Vec<ExternalLink> {
        vec![ExternalLink {
            label: format!("U.S. Bureau of Reclamation: {}", self.name),
            url: USBR_URL.to_string(),
        }]
    }
}

macro_rules! project {
    ($loc:expr, $name:expr, $state:expr, $lat:expr, $lon:expr, $datum:ident, $pool:expr, $out:expr, $inflow:expr) => {
        Project {
            location: $loc,
            name: $name,
            state: $state,
            latitude: $lat,
            longitude: $lon,
            datum: VerticalDatum::$datum,
            pool: $pool,
            outflow: $out,
            inflow: $inflow,
        }
    };
}

/// The reservoirs whose items were checked against the live service (see `docs/providers/rise.md`).
/// Datum: `Gauge` where RISE states EGM96 or USGS 1912 or nothing. Anderson Ranch's release series
/// ended and is not listed; storage is not carried.
#[rustfmt::skip]
pub static PROJECTS: &[Project] = &[
    project!(1535, "Flaming Gorge Reservoir", "UT", 40.98, -109.57, Gauge, ft(341), cfs(4314), cfs(339)),
    project!(393, "Lake Powell", "AZ", 37.06, -111.30, Gauge, ft(508), cfs(4315), cfs(511)),
    project!(3514, "Lake Mead", "NV", 36.02, -114.74, Gauge, ft(6123), cfs(6125), None),
    project!(1536, "Fontenelle Reservoir", "WY", 42.06, -110.10, Gauge, ft(349), cfs(4313), cfs(4281)),
    project!(423, "Navajo Reservoir", "NM", 36.86, -107.53, Gauge, ft(612), cfs(4316), cfs(616)),
    project!(281, "Boysen Reservoir", "WY", 43.40, -108.18, Ngvd29, ft(196), cfs(199), cfs(197)),
    project!(433, "Pathfinder Reservoir", "WY", 42.47, -106.85, Gauge, ft(645), cfs(647), None),
    project!(295, "Canyon Ferry Lake", "MT", 46.65, -111.73, Gauge, ft(14), None, None),
    project!(337, "Fresno Reservoir", "MT", 48.61, -109.95, Ngvd29, ft(355), cfs(357), None),
    project!(471, "Shasta Lake", "CA", 40.72, -122.42, Navd88, ft(10863), cfs(10869), cfs(10867)),
    project!(3203, "Trinity Lake", "CA", 40.80, -122.76, Navd88, ft(10853), cfs(10859), cfs(10857)),
    project!(334, "Folsom Lake", "CA", 38.68, -121.18, Navd88, ft(10775), cfs(10772), cfs(10773)),
    project!(387, "Keswick Reservoir", "CA", 40.60, -122.44, Navd88, ft(10795), cfs(10792), cfs(10793)),
    project!(3641, "Anderson Ranch Reservoir", "ID", 43.36, -115.45, Ngvd29, ft(129_153), None, None),
    project!(3642, "Arrowrock Reservoir", "ID", 43.60, -115.92, Ngvd29, ft(129_218), None, cfs(129_219)),
    project!(7192, "Lake Owyhee", "OR", 43.64, -117.24, Navd88, ft(132_262), cfs(132_270), None),
    project!(7185, "Jackson Lake", "WY", 43.86, -110.59, Navd88, ft(131_200), None, None),
    project!(323, "Elephant Butte Reservoir", "NM", 33.22, -107.18, Gauge, ft(332), cfs(4377), cfs(330)),
    project!(7302, "Lake Meredith", "TX", 35.71, -101.55, Navd88, ft(134_006), None, None),
];

/// The project at RISE location `location`.
pub fn find(location: u32) -> Option<&'static Project> {
    PROJECTS.iter().find(|p| p.location == location)
}

/// Projects within `max_m` metres of (`latitude`, `longitude`), nearest first, with the distance.
pub fn nearby(latitude: f64, longitude: f64, max_m: f32) -> Vec<(&'static Project, f32)> {
    let at = GeoPoint {
        lat: latitude,
        lon: longitude,
    };
    let mut out: Vec<(&'static Project, f32)> = PROJECTS
        .iter()
        .map(|p| {
            let d = haversine_m(
                at,
                GeoPoint {
                    lat: p.latitude,
                    lon: p.longitude,
                },
            ) as f32;
            (p, d)
        })
        .filter(|(_, d)| *d <= max_m)
        .collect();
    out.sort_by(|a, b| a.1.total_cmp(&b.1));
    out
}

/// The credit every display of RISE data shows.
pub fn attribution() -> Attribution {
    Attribution {
        source: SourceId::RISE,
        credit: "U.S. Bureau of Reclamation (RISE; provisional data)".into(),
        url: Some("https://data.usbr.gov/".into()),
    }
}

/// The disclaimer RISE attaches to its data, shown with them.
pub const DISCLAIMER: &str = "Data are provisional and subject to revision unless otherwise \
    noted. Data users are cautioned to consider carefully the provisional nature of the \
    information before using it.";

/// One GET the host sends: the URL and which part of the project it fetches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseRequest {
    /// The full URL.
    pub url: String,
    /// The `Accept` header.
    pub headers: Vec<(&'static str, String)>,
    /// The part of the project the answer is.
    pub role: Role,
    /// The item the request names; [`parse`] checks the rows against it.
    pub item: Item,
}

/// `YYYY-MM-DD` of the UTC day containing `t`.
fn format_date(t: UnixTime) -> String {
    let l = t.to_local(UtcOffset(0));
    format!("{:04}-{:02}-{:02}", l.date.year, l.date.month, l.date.day)
}

fn request(item: Item, role: Role, window: Window) -> RiseRequest {
    // Daily values are stamped at local midnight in UTC: start one day early so the value for the
    // window's first day is inside it.
    let after = UnixTime(window.from.0.saturating_sub(DAY));
    let days = (window.to.0.saturating_sub(after.0) / DAY + 2).clamp(2, MAX_PAGE);
    let url = QueryBuilder::new(&format!("{BASE}/result"))
        .param("itemId", item.id.to_string())
        .param("dateTime%5Bafter%5D", format_date(after))
        .param("itemsPerPage", days.to_string())
        .param("order%5BdateTime%5D", "DESC")
        .finish();
    RiseRequest {
        url,
        headers: vec![("Accept", "application/vnd.api+json".to_string())],
        role,
        item,
    }
}

/// The requests for the items `project` publishes over `window`: pool, release, inflow, in that
/// order, each only where the table names an item. Every request names its `itemId`.
pub fn requests(project: &Project, window: Window) -> Vec<RiseRequest> {
    [
        (Role::Pool, project.pool),
        (Role::Outflow, project.outflow),
        (Role::Inflow, project.inflow),
    ]
    .into_iter()
    .filter_map(|(role, item)| Some(request(item?, role, window)))
    .collect()
}

/// One parsed answer: the samples of one page of one item.
#[derive(Clone, Debug, PartialEq)]
pub struct Page {
    /// The item's part of the project.
    pub role: Role,
    /// The values the page carried, ascending, in metres or m³/s, without missing ones.
    pub samples: Vec<Sample<f32>>,
    /// The URL of the next page, when the total says there is one.
    pub next: Option<String>,
}

#[derive(Deserialize)]
struct Body {
    #[serde(default)]
    meta: Meta,
    #[serde(default)]
    data: Vec<Row>,
}

#[derive(Default, Deserialize)]
struct Meta {
    #[serde(rename = "totalItems")]
    total_items: Option<i64>,
    #[serde(rename = "itemsPerPage")]
    items_per_page: Option<i64>,
    #[serde(rename = "currentPage")]
    current_page: Option<i64>,
}

#[derive(Deserialize)]
struct Row {
    attributes: Attributes,
}

#[derive(Deserialize)]
struct Attributes {
    #[serde(rename = "itemId")]
    item_id: Option<i64>,
    #[serde(rename = "dateTime")]
    date_time: Option<String>,
    result: Option<Value>,
}

/// Parses `response` to the request `req`; the next page's URL extends the request's.
pub fn parse(req: &RiseRequest, response: &Response<'_>) -> Result<Page, SourceError> {
    let body = check(response)?;
    if !req.item.unit.fits(req.role) {
        return Err(SourceError::Parse(format!(
            "unit {:?} does not fit {:?}",
            req.item.unit, req.role
        )));
    }
    let parsed: Body = serde_json::from_slice(body)
        .map_err(|e| SourceError::Parse(format!("not a RISE result list: {e}")))?;
    if parsed
        .meta
        .total_items
        .is_some_and(|n| n > IMPLAUSIBLE_TOTAL)
    {
        return Err(SourceError::Parse(
            "the service ignored a filter and returned the whole series".into(),
        ));
    }
    let mut samples = Vec::with_capacity(parsed.data.len());
    for row in &parsed.data {
        let a = &row.attributes;
        if a.item_id.is_some_and(|id| id != i64::from(req.item.id)) {
            return Err(SourceError::Parse(format!(
                "a row of item {:?} in the answer for item {}",
                a.item_id, req.item.id
            )));
        }
        let (Some(time), Some(value)) = (
            a.date_time.as_deref().and_then(parse_rfc3339),
            a.result.as_ref().and_then(Value::as_f64),
        ) else {
            continue; // a missing value is absent
        };
        if !value.is_finite() {
            continue;
        }
        samples.push(Sample::new(time, req.item.unit.to_canonical(value) as f32));
    }
    samples.sort_by_key(|s| s.time);
    let next = match (
        parsed.meta.total_items,
        parsed.meta.items_per_page,
        parsed.meta.current_page,
    ) {
        (Some(total), Some(per), Some(page)) if per > 0 && total > page.saturating_mul(per) => {
            let base = req.url.split("&page=").next().unwrap_or(&req.url);
            Some(
                QueryBuilder::new(base)
                    .param("page", (page + 1).to_string())
                    .finish(),
            )
        }
        _ => None,
    };
    Ok(Page {
        role: req.role,
        samples,
        next,
    })
}

/// The status and body checks, tolerant of the JSON:API and problem+json envelopes.
fn check<'a>(response: &Response<'a>) -> Result<&'a [u8], SourceError> {
    let body = response.body.trim_ascii();
    match response.status {
        200..=299 => {}
        404 | 410 => return Err(SourceError::NotFound(message(body))),
        429 => {
            return Err(SourceError::RateLimited {
                retry_after_s: response.retry_after,
            });
        }
        status => {
            return Err(SourceError::Status {
                status,
                message: message(body),
            });
        }
    }
    if body.is_empty() {
        return Err(SourceError::NoData(Some("empty response".into())));
    }
    if body[0] != b'{' {
        return Err(SourceError::Parse("not a JSON object".into()));
    }
    Ok(body)
}

/// The `detail` (else `title`) of a problem+json body, trimmed and cut; other bodies give none.
fn message(body: &[u8]) -> Option<String> {
    let value: Value = serde_json::from_slice(body).ok()?;
    let text = ["detail", "title", "message"]
        .iter()
        .find_map(|k| value.get(k)?.as_str())?
        .trim();
    (!text.is_empty()).then(|| text.chars().take(200).collect())
}

fn provenance(project: &Project, origin: Origin, observed_at: UnixTime) -> Provenance {
    Provenance {
        source: SourceId::RISE,
        station: Some(StationRef {
            source: SourceId::RISE,
            id: project.station_id(),
            name: project.name.to_string(),
            kind: StationKind::WaterLevel,
            latitude: project.latitude,
            longitude: project.longitude,
            bin: None,
            reference_id: None,
        }),
        distance_m: origin.distance_m,
        relation: origin.relation,
        observed_at,
        kind: ValueKind::Observed,
        quality: Quality::Provisional,
    }
}

fn series(project: &Project, origin: Origin, pages: &[Page]) -> Result<Series<f32>, SourceError> {
    let samples: Vec<Sample<f32>> = pages.iter().flat_map(|p| p.samples.clone()).collect();
    let latest = samples
        .iter()
        .map(|s| s.time)
        .max()
        .ok_or_else(|| SourceError::NoData(Some(format!("no values for {}", project.name))))?;
    Ok(Series::new(provenance(project, origin, latest), samples))
}

/// The pool's heights from the pages of its elevation item, in the project's datum.
pub fn pool(project: &Project, origin: Origin, pages: &[Page]) -> Result<Heights, SourceError> {
    Ok(Heights {
        datum: project.datum,
        series: series(project, origin, pages)?,
    })
}

/// Assembles a project from the pages of each item its requests fetched. A role with no pages or
/// no values is absent, not an error; `Err` only when the project has nothing at all.
pub fn assemble(
    project: &Project,
    origin: Origin,
    pages: &[Page],
) -> Result<Reservoir, SourceError> {
    let of =
        |role: Role| -> Vec<Page> { pages.iter().filter(|p| p.role == role).cloned().collect() };
    let level = pool(project, origin, &of(Role::Pool)).ok();
    let outflow = series(project, origin, &of(Role::Outflow)).ok();
    let inflow = series(project, origin, &of(Role::Inflow)).ok();
    if level.is_none() && outflow.is_none() && inflow.is_none() {
        return Err(SourceError::NoData(Some(format!(
            "no values for {}",
            project.name
        ))));
    }
    Ok(Reservoir {
        level,
        info: ReservoirInfo {
            project: project.name.to_string(),
            outflow_m3s: outflow,
            inflow_m3s: inflow,
            links: project.links(),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::fixture_bytes;
    use underbite_core::provenance::Relation;

    const ORIGIN: Origin = Origin {
        distance_m: Some(9_000.0),
        relation: Relation::SameWaterBody,
    };

    fn ok(body: &[u8]) -> Response<'_> {
        Response {
            status: 200,
            body,
            content_type: Some("application/vnd.api+json"),
            date: None,
            expires: None,
            cache_max_age: None,
            last_modified: None,
            etag: None,
            retry_after: None,
            rate_limit_remaining: None,
        }
    }

    fn status(code: u16, body: &[u8]) -> Response<'_> {
        Response {
            status: code,
            ..ok(body)
        }
    }

    /// 2026-11-15T00:00Z to 2026-11-17T00:00Z.
    const W: Window = Window {
        from: UnixTime(1_794_700_800),
        to: UnixTime(1_794_873_600),
    };

    fn req(item: Option<Item>, role: Role) -> RiseRequest {
        request(item.expect("an item"), role, W)
    }

    fn fresno_pool() -> Page {
        let bytes = fixture_bytes("tier2", "rise-fresno-reservoir-elevation-2026-11.json");
        let fresno = find(337).expect("listed");
        parse(&req(fresno.pool, Role::Pool), &ok(&bytes)).expect("parses")
    }

    #[test]
    fn plans_one_request_per_published_item() {
        let fresno = find(337).expect("listed");
        let planned = requests(fresno, W);
        let roles: Vec<Role> = planned.iter().map(|r| r.role).collect();
        assert_eq!(roles, [Role::Pool, Role::Outflow]);
        assert_eq!(
            planned[0].url,
            "https://data.usbr.gov/rise/api/result?itemId=355\
             &dateTime%5Bafter%5D=2026-11-14&itemsPerPage=5&order%5BdateTime%5D=DESC"
        );
        assert_eq!(
            planned[0].headers,
            vec![("Accept", "application/vnd.api+json".to_string())]
        );
        let canyon = find(295).expect("listed");
        assert_eq!(requests(canyon, W).len(), 1);
        for p in PROJECTS {
            for r in requests(p, W) {
                assert!(r.url.contains("/result?itemId="), "{}", r.url);
                assert!(!r.url.contains('[') && !r.url.contains(']'));
            }
        }
    }

    #[test]
    fn fresno_elevation_is_metres_in_ascending_order() {
        let p = fresno_pool();
        assert_eq!(p.samples.len(), 7);
        assert!(p.samples.windows(2).all(|w| w[0].time < w[1].time));
        // 2567.24 ft on 2026-11-07, stamped 07:00Z (local midnight, MST).
        let last = p.samples.last().expect("a value");
        assert_eq!(
            last.time,
            parse_rfc3339("2026-11-07T07:00:00+00:00").expect("time")
        );
        assert!((f64::from(last.value) - 2567.24 * 0.3048).abs() < 1e-2);
        assert_eq!(p.next, None);
    }

    #[test]
    fn pool_carries_the_locations_datum_and_provenance() {
        let fresno = find(337).expect("listed");
        let heights = pool(fresno, ORIGIN, &[fresno_pool()]).expect("a pool");
        assert_eq!(heights.datum, VerticalDatum::Ngvd29);
        let prov = heights.series.provenance();
        assert_eq!(prov.source, SourceId::RISE);
        assert_eq!(prov.station.as_ref().map(|s| s.id.as_str()), Some("337"));
        assert_eq!(prov.kind, ValueKind::Observed);
        assert_eq!(prov.quality, Quality::Provisional);
        assert_eq!(prov.distance_m, Some(9_000.0));
        let flaming = find(1535).expect("listed");
        let h = pool(flaming, ORIGIN, &[fresno_pool()]).expect("a pool");
        assert_eq!(h.datum, VerticalDatum::Gauge, "EGM96 is not modelled");
    }

    #[test]
    fn release_is_converted_from_cfs() {
        let bytes = fixture_bytes(
            "rise",
            "rise-result-4314-flaming-gorge-release-total-cfs-2026-11-17.json",
        );
        let flaming = find(1535).expect("listed");
        let page = parse(&req(flaming.outflow, Role::Outflow), &ok(&bytes)).expect("parses");
        assert!(page.samples.len() > 1);
        let raw: Value = serde_json::from_slice(&bytes).expect("json");
        let newest = raw["data"][0]["attributes"]["result"]
            .as_f64()
            .expect("number");
        let got = page.samples.last().expect("a value").value;
        assert!((f64::from(got) - newest * 0.028_316_846_592).abs() < 1e-3);
    }

    #[test]
    fn a_reservoir_collects_level_outflow_and_links() {
        let flaming = find(1535).expect("listed");
        let elev = fixture_bytes(
            "rise",
            "rise-result-341-flaming-gorge-elevation-2026-11-17.json",
        );
        let rel = fixture_bytes(
            "rise",
            "rise-result-4314-flaming-gorge-release-total-cfs-2026-11-17.json",
        );
        let pages = [
            parse(&req(flaming.pool, Role::Pool), &ok(&elev)).expect("elevation"),
            parse(&req(flaming.outflow, Role::Outflow), &ok(&rel)).expect("release"),
        ];
        let r = assemble(flaming, ORIGIN, &pages).expect("a reservoir");
        assert!(r.info.outflow_m3s.is_some());
        assert!(r.info.inflow_m3s.is_none());
        assert_eq!(r.info.project, "Flaming Gorge Reservoir");
        assert_eq!(r.info.links, flaming.links());
        let level = r.level.expect("level");
        let v = level.series.samples().last().expect("a value").value;
        assert!((f64::from(v) - 6003.54 * 0.3048).abs() < 1e-2, "{v}");
    }

    #[test]
    fn nothing_at_all_is_no_data() {
        let fresno = find(337).expect("listed");
        assert!(matches!(
            assemble(fresno, ORIGIN, &[]),
            Err(SourceError::NoData(_))
        ));
    }

    #[test]
    fn null_and_mismatched_rows() {
        let item = ft(355);
        let body = br#"{"meta":{"totalItems":2,"itemsPerPage":10,"currentPage":1},"data":[
            {"attributes":{"itemId":355,"dateTime":"2026-11-07T07:00:00+00:00","result":null}},
            {"attributes":{"itemId":355,"dateTime":"2026-11-06T07:00:00+00:00","result":10.0}}]}"#;
        let p = parse(&req(item, Role::Pool), &ok(body)).expect("parses");
        assert_eq!(p.samples.len(), 1);
        assert!((p.samples[0].value - 3.048).abs() < 1e-4);
        let other = br#"{"data":[{"attributes":{"itemId":999,
            "dateTime":"2026-11-06T07:00:00+00:00","result":1.0}}]}"#;
        assert!(matches!(
            parse(&req(item, Role::Pool), &ok(other)),
            Err(SourceError::Parse(_))
        ));
    }

    #[test]
    fn a_next_page_is_offered_while_the_total_exceeds_the_rows_seen() {
        let r = req(cfs(4314), Role::Outflow);
        let body = br#"{"meta":{"totalItems":25,"itemsPerPage":10,"currentPage":1},"data":[]}"#;
        let p = parse(&r, &ok(body)).expect("parses");
        let next = p.next.expect("a next page");
        assert!(
            next.starts_with(&r.url) && next.ends_with("&page=2"),
            "{next}"
        );
        let last = br#"{"meta":{"totalItems":25,"itemsPerPage":10,"currentPage":3},"data":[]}"#;
        assert_eq!(parse(&r, &ok(last)).expect("parses").next, None);
    }

    #[test]
    fn an_ignored_filter_is_refused() {
        let body = br#"{"meta":{"totalItems":29029,"itemsPerPage":10,"currentPage":1},"data":[]}"#;
        assert!(matches!(
            parse(&req(ft(355), Role::Pool), &ok(body)),
            Err(SourceError::Parse(_))
        ));
    }

    #[test]
    fn errors_map_to_source_errors() {
        let bytes = fixture_bytes("rise", "rise-result-no-filter-error-2026-11-17.json");
        let r = req(ft(355), Role::Pool);
        match parse(&r, &status(400, &bytes)) {
            Err(SourceError::Status {
                status: 400,
                message,
            }) => assert!(message.is_some_and(|m| m.contains("itemId"))),
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            parse(&r, &status(404, b"{}")),
            Err(SourceError::NotFound(_))
        ));
        assert!(matches!(
            parse(&r, &ok(b"<html>")),
            Err(SourceError::Parse(_))
        ));
        assert!(matches!(parse(&r, &ok(b"")), Err(SourceError::NoData(_))));
        let empty = parse(&r, &ok(br#"{"data":[]}"#)).expect("parses");
        assert_eq!(empty.samples.len(), 0);
    }

    #[test]
    fn units_come_from_the_item() {
        assert_eq!(Unit::from_rise("ft"), Some(Unit::Feet));
        assert_eq!(Unit::from_rise("cfs"), Some(Unit::CubicFeetPerSecond));
        assert_eq!(Unit::from_rise("af"), None);
        assert!(parse(&req(ft(1), Role::Outflow), &ok(br#"{"data":[]}"#)).is_err());
    }

    #[test]
    fn nearby_finds_the_reservoir_and_new_york_has_none() {
        let near = nearby(48.6, -109.9, 20_000.0);
        assert_eq!(near.first().map(|(p, _)| p.name), Some("Fresno Reservoir"));
        assert_eq!(nearby(40.7033, -74.0170, 1_000_000.0).len(), 0);
    }

    #[test]
    fn attribution_names_the_bureau_and_the_data_are_provisional() {
        let a = attribution();
        assert_eq!(a.source, SourceId::RISE);
        assert!(a.credit.contains("Bureau of Reclamation"));
        assert!(DISCLAIMER.starts_with("Data are provisional"));
    }
}
