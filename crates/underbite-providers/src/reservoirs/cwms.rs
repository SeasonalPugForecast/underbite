//! The USACE CWMS Data API: pool elevation, outflow and inflow of a reservoir project.
//!
//! One `timeseries` request per series the project publishes (see [`super::catalogue`]); the
//! requests carry no coordinates, only the project's own series names. Each answer is a JSON
//! document whose `values` are rows `[epoch_ms, value | null, quality]`; this module parses them
//! into the pool's [`Heights`] and the release and inflow [`Series`], and assembles the
//! [`ReservoirInfo`] a snapshot carries.
//!
//! Requests ask for `unit=SI`, so elevations come in metres and flows in m³/s; any other unit in
//! an answer is an error, never a conversion. A missing value (`null`, quality 5) is absent. The
//! quality code is the CWMS 32-bit integer: a value is dropped when its validity bits say missing
//! or rejected, flagged when they say questionable or a screening test failed, and otherwise
//! provisional (an unscreened series, quality 0, is not bad). The catalogue's `extents` are stale
//! and are never read.
//!
//! Corps data are provisional and subject to revision.

use serde::Deserialize;
use serde_json::Value;
use underbite_core::datum::{Heights, VerticalDatum};
use underbite_core::model::ReservoirInfo;
use underbite_core::provenance::{
    Attribution, Provenance, Quality, Relation, SourceId, StationKind, StationRef, ValueKind,
};
use underbite_core::series::{Sample, Series};
use underbite_core::time::{UnixTime, UtcOffset};

use super::catalogue::Project;
use crate::request::{QueryBuilder, Response, Window};
use crate::source::SourceError;

/// The service's base URL.
pub const BASE: &str = "https://cwms-data.usace.army.mil/cwms-data";

/// The `Accept` header every request carries: version 2 of the JSON format.
pub const ACCEPT: &str = "application/json;version=2";

/// Which part of a project a series is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Role {
    /// Pool elevation, metres.
    Pool,
    /// Release below the dam, m³/s.
    Outflow,
    /// Inflow to the pool, m³/s.
    Inflow,
}

impl Role {
    /// The unit `unit=SI` gives this role.
    fn unit(self) -> &'static str {
        match self {
            Self::Pool => "m",
            Self::Outflow | Self::Inflow => "cms",
        }
    }
}

/// One GET the host sends: the URL, its headers, and which part of the project it fetches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CwmsRequest {
    /// The full URL.
    pub url: String,
    /// The request headers (`Accept`).
    pub headers: Vec<(&'static str, String)>,
    /// The part of the project the answer is.
    pub role: Role,
}

/// The credit every display of CWMS data shows.
pub fn attribution() -> Attribution {
    Attribution {
        source: SourceId::CWMS,
        credit: "U.S. Army Corps of Engineers (provisional data)".into(),
        url: Some("https://cwms-data.usace.army.mil/".into()),
    }
}

/// `YYYY-MM-DDTHH:MM:SSZ`.
fn format_time(t: UnixTime) -> String {
    let l = t.to_local(UtcOffset(0));
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        l.date.year, l.date.month, l.date.day, l.hour, l.minute, l.second
    )
}

fn request(project: &Project, name: &str, role: Role, window: Window) -> CwmsRequest {
    let url = QueryBuilder::new(&format!("{BASE}/timeseries"))
        .param("office", project.office)
        .param("name", name)
        .param("begin", format_time(window.from))
        .param("end", format_time(window.to))
        .param("unit", "SI")
        .finish();
    CwmsRequest {
        url,
        headers: vec![("Accept", ACCEPT.to_string())],
        role,
    }
}

/// The requests for the series `project` publishes over `window`: pool, release, inflow, in that
/// order, each only where the catalogue names a series.
pub fn requests(project: &Project, window: Window) -> Vec<CwmsRequest> {
    [
        (Role::Pool, project.pool),
        (Role::Outflow, project.outflow),
        (Role::Inflow, project.inflow),
    ]
    .into_iter()
    .filter_map(|(role, name)| Some(request(project, name?, role, window)))
    .collect()
}

/// How the station relates to the spot, for provenance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Origin {
    /// From the spot to the project, metres.
    pub distance_m: Option<f32>,
    /// How the project relates to the spot.
    pub relation: Relation,
}

/// One parsed answer: the samples of one page of one series.
#[derive(Clone, Debug, PartialEq)]
pub struct Page {
    /// The series' part of the project.
    pub role: Role,
    /// The datum of an elevation series; `None` for flows.
    pub datum: Option<VerticalDatum>,
    /// The values the page carried, ascending, without missing or rejected ones.
    pub samples: Vec<Sample<f32>>,
    /// The URL of the next page, when the service says there is one.
    pub next: Option<String>,
}

#[derive(Deserialize)]
struct Body {
    units: Option<String>,
    #[serde(default)]
    values: Vec<Vec<Value>>,
    #[serde(rename = "next-page")]
    next_page: Option<String>,
    #[serde(rename = "vertical-datum-info")]
    datum: Option<DatumInfo>,
}

#[derive(Deserialize)]
struct DatumInfo {
    #[serde(rename = "native-datum")]
    native: Option<String>,
}

/// The datum an elevation is above: the series' native datum, else the project's own gauge datum
/// (CWMS writes `OTHER` for a project datum such as Raystown's `PCD`).
fn datum(info: Option<&DatumInfo>) -> VerticalDatum {
    match info.and_then(|i| i.native.as_deref()) {
        Some("NAVD-88") => VerticalDatum::Navd88,
        Some("NGVD-29") => VerticalDatum::Ngvd29,
        _ => VerticalDatum::Gauge,
    }
}

/// What a quality code means for a value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Verdict {
    /// Show it as provisional.
    Keep,
    /// Show it, labelled unreliable.
    Flag,
    /// Do not show it.
    Drop,
}

/// Bits 15 to 25: a screening test failed.
const TEST_FAILED_MASK: i64 = 0x03FF_8000;

/// The CWMS quality integer: validity is bits 1 to 4 (1 okay, 2 missing, 4 questionable,
/// 8 rejected, 0 unknown). Quality 0 is an unscreened value and is kept.
fn judge(quality: i64) -> Verdict {
    match (quality >> 1) & 15 {
        2 | 8 => Verdict::Drop,
        4 => Verdict::Flag,
        _ if quality & TEST_FAILED_MASK != 0 => Verdict::Flag,
        _ => Verdict::Keep,
    }
}

/// Parses `response` to a request for `role`; `url` is the request's URL, which the next page's
/// URL extends.
pub fn parse(role: Role, url: &str, response: &Response<'_>) -> Result<Page, SourceError> {
    let body = check(response)?;
    let parsed: Body = serde_json::from_slice(body)
        .map_err(|e| SourceError::Parse(format!("not a CWMS time series: {e}")))?;
    match parsed.units.as_deref() {
        Some(unit) if unit == role.unit() => {}
        other => {
            return Err(SourceError::Parse(format!(
                "unit {other:?}, expected {} (request unit=SI)",
                role.unit()
            )));
        }
    }
    let mut samples = Vec::with_capacity(parsed.values.len());
    for row in &parsed.values {
        let (Some(ms), Some(value)) = (
            row.first().and_then(Value::as_i64),
            row.get(1).and_then(Value::as_f64),
        ) else {
            continue; // `null` is a missing value, whatever its quality
        };
        if !value.is_finite() {
            continue;
        }
        let quality = row.get(2).and_then(Value::as_i64).unwrap_or(0);
        let verdict = judge(quality);
        if verdict == Verdict::Drop {
            continue;
        }
        let mut sample = Sample::new(UnixTime(ms.div_euclid(1000)), value as f32);
        if verdict == Verdict::Flag {
            sample.quality = Some(Quality::Flagged);
        }
        samples.push(sample);
    }
    samples.sort_by_key(|s| s.time);
    let next = parsed.next_page.filter(|c| !c.is_empty()).map(|c| {
        let base = url.split("&page=").next().unwrap_or(url);
        QueryBuilder::new(base).param("page", c).finish()
    });
    Ok(Page {
        role,
        datum: (role == Role::Pool).then(|| datum(parsed.datum.as_ref())),
        samples,
        next,
    })
}

/// The status and body checks every parser of this module starts with.
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

/// The `message` of a JSON error body, trimmed and cut; HTML and plain text give none.
fn message(body: &[u8]) -> Option<String> {
    let value: Value = serde_json::from_slice(body).ok()?;
    let text = value.get("message")?.as_str()?.trim();
    (!text.is_empty()).then(|| text.chars().take(200).collect())
}

fn provenance(project: &Project, origin: Origin, observed_at: UnixTime) -> Provenance {
    Provenance {
        source: SourceId::CWMS,
        station: Some(StationRef {
            source: SourceId::CWMS,
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

/// The series of `pages` of one role, or [`SourceError::NoData`] when they carry no value.
fn series(project: &Project, origin: Origin, pages: &[Page]) -> Result<Series<f32>, SourceError> {
    let samples: Vec<Sample<f32>> = pages.iter().flat_map(|p| p.samples.clone()).collect();
    let latest = samples
        .iter()
        .map(|s| s.time)
        .max()
        .ok_or_else(|| SourceError::NoData(Some(format!("no values for {}", project.name))))?;
    Ok(Series::new(provenance(project, origin, latest), samples))
}

/// The pool's heights from the pages of its elevation series.
pub fn pool(project: &Project, origin: Origin, pages: &[Page]) -> Result<Heights, SourceError> {
    let datum = pages
        .iter()
        .find_map(|p| p.datum)
        .unwrap_or(VerticalDatum::Gauge);
    Ok(Heights {
        datum,
        series: series(project, origin, pages)?,
    })
}

/// A project as a snapshot carries it.
#[derive(Clone, Debug, PartialEq)]
pub struct Reservoir {
    /// The pool level with its datum, for the water's `level`.
    pub level: Option<Heights>,
    /// Release, inflow and the operator's link, for the water's `reservoir`.
    pub info: ReservoirInfo,
}

/// Assembles a project from the pages of each series its requests fetched. A role with no pages
/// or no values is absent, not an error; `Err` only when the project has nothing at all.
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
    use super::super::catalogue::find;
    use super::*;
    use crate::testutil::fixture_bytes;

    const ORIGIN: Origin = Origin {
        distance_m: Some(12_000.0),
        relation: Relation::SameWaterBody,
    };

    fn ok(body: &[u8]) -> Response<'_> {
        Response {
            status: 200,
            body,
            content_type: Some("application/json;version=2"),
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

    fn page(dir: &str, name: &str, role: Role) -> Page {
        let bytes = fixture_bytes(dir, name);
        parse(role, "https://example.invalid/timeseries?a=b", &ok(&bytes)).expect("parses")
    }

    fn json(units: &str, values: &str) -> Vec<u8> {
        format!(r#"{{"name":"X.Elev.Inst.15Minutes.0.v","units":"{units}","values":{values}}}"#)
            .into_bytes()
    }

    const T: Window = Window {
        from: UnixTime(1_794_895_200),
        to: UnixTime(1_794_902_400),
    };

    #[test]
    fn plans_one_request_per_published_series() {
        let raystown = find("NAB", "Raystown").expect("listed");
        let planned = requests(raystown, T);
        let roles: Vec<Role> = planned.iter().map(|r| r.role).collect();
        assert_eq!(roles, [Role::Pool, Role::Outflow, Role::Inflow]);
        assert_eq!(
            planned[0].url,
            "https://cwms-data.usace.army.mil/cwms-data/timeseries?office=NAB\
             &name=Raystown.Elev.Inst.15Minutes.0.Best-NAB\
             &begin=2026-11-17T06%3A00%3A00Z&end=2026-11-17T08%3A00%3A00Z&unit=SI"
        );
        assert_eq!(
            planned[0].headers,
            vec![("Accept", "application/json;version=2".to_string())]
        );
        let beltzville = find("NAP", "Beltzville").expect("listed");
        let roles: Vec<Role> = requests(beltzville, T).iter().map(|r| r.role).collect();
        assert_eq!(
            roles,
            [Role::Pool, Role::Inflow],
            "Philadelphia has no outflow"
        );
        let hartwell = find("SAS", "Hartwell").expect("listed");
        let roles: Vec<Role> = requests(hartwell, T).iter().map(|r| r.role).collect();
        assert_eq!(roles, [Role::Outflow]);
    }

    #[test]
    fn series_names_with_spaces_are_percent_encoded() {
        let carlyle = find("MVS", "Carlyle Lk-Kaskaskia").expect("listed");
        let url = &requests(carlyle, T)[0].url;
        assert!(
            url.contains("name=Carlyle%20Lk-Kaskaskia.Elev.Inst.30Minutes.0.lrgsShef-rev"),
            "{url}"
        );
        assert!(!url.contains(' '));
    }

    #[test]
    fn raystown_pool_is_metres_above_the_project_datum() {
        let p = page("cwms", "cwms-nab-raystown-elev-2026-11-17.json", Role::Pool);
        assert_eq!(p.samples.len(), 9);
        assert_eq!(p.datum, Some(VerticalDatum::Gauge));
        assert_eq!(p.samples[0].time, UnixTime(1_794_895_200));
        assert!((p.samples[0].value - 237.491_02).abs() < 1e-3);
        assert!(p.samples.iter().all(|s| s.quality.is_none()));
        assert_eq!(p.next, None);
        let raystown = find("NAB", "Raystown").expect("listed");
        let heights = pool(raystown, ORIGIN, &[p]).expect("a pool");
        let prov = heights.series.provenance();
        assert_eq!(prov.source, SourceId::CWMS);
        assert_eq!(
            prov.station.as_ref().map(|s| s.id.as_str()),
            Some("NAB/Raystown")
        );
        assert_eq!(prov.observed_at, UnixTime(1_794_902_400));
        assert_eq!(prov.kind, ValueKind::Observed);
        assert_eq!(prov.quality, Quality::Provisional);
        assert_eq!(prov.distance_m, Some(12_000.0));
    }

    #[test]
    fn datum_follows_the_series() {
        let ngvd = page("cwms", "cwms-mvp-baldhill-elev-2026-11-17.json", Role::Pool);
        assert_eq!(ngvd.datum, Some(VerticalDatum::Ngvd29));
        let beltzville = page(
            "tier2",
            "cwms-nap-beltzville-elev-2026-11-08.json",
            Role::Pool,
        );
        assert_eq!(beltzville.datum, Some(VerticalDatum::Navd88));
        let allatoona = page(
            "cwms",
            "cwms-sam-allatoona-elev-2026-11-17.json",
            Role::Pool,
        );
        assert_eq!(allatoona.datum, Some(VerticalDatum::Ngvd29));
        assert_eq!(allatoona.samples.len(), 3);
        // Unscreened values (quality 0) are kept as they are.
        assert!(allatoona.samples.iter().all(|s| s.quality.is_none()));
    }

    #[test]
    fn null_values_with_quality_5_are_absent() {
        let p = page(
            "tier2",
            "cwms-nap-beltzville-elev-2026-11-08.json",
            Role::Pool,
        );
        assert_eq!(p.samples.len(), 7, "9 rows, the last two null");
        assert_eq!(
            p.samples.last().map(|s| s.time),
            Some(UnixTime(1_794_144_600))
        );
    }

    #[test]
    fn outflow_is_a_flow_without_a_datum() {
        let p = page(
            "cwms",
            "cwms-nab-raystown-flow-out-2026-11-17.json",
            Role::Outflow,
        );
        assert_eq!(p.datum, None);
        assert_eq!(p.samples.len(), 9);
        assert!((p.samples[0].value - 13.599_167).abs() < 1e-4);
        let b = page(
            "cwms",
            "cwms-mvp-baldhill-flow-out-2026-11-17.json",
            Role::Outflow,
        );
        assert!(b.samples.len() > 1);
    }

    #[test]
    fn a_reservoir_collects_level_outflow_and_links() {
        let raystown = find("NAB", "Raystown").expect("listed");
        let pages = [
            page("cwms", "cwms-nab-raystown-elev-2026-11-17.json", Role::Pool),
            page(
                "cwms",
                "cwms-nab-raystown-flow-out-2026-11-17.json",
                Role::Outflow,
            ),
        ];
        let r = assemble(raystown, ORIGIN, &pages).expect("a reservoir");
        assert_eq!(
            r.level.as_ref().map(|h| h.datum),
            Some(VerticalDatum::Gauge)
        );
        assert_eq!(r.info.project, "Raystown Lake");
        assert!(r.info.outflow_m3s.is_some());
        assert!(r.info.inflow_m3s.is_none());
        assert_eq!(r.info.links.len(), 1);
        assert_eq!(
            r.info
                .outflow_m3s
                .as_ref()
                .map(|s| s.provenance().source.clone()),
            Some(SourceId::CWMS)
        );
    }

    #[test]
    fn a_project_with_no_values_at_all_has_no_data() {
        let raystown = find("NAB", "Raystown").expect("listed");
        let empty = parse(Role::Pool, "u", &ok(&json("m", "[]"))).expect("parses");
        let err = assemble(raystown, ORIGIN, &[empty]).expect_err("nothing to show");
        assert!(matches!(err, SourceError::NoData(_)), "{err:?}");
        let nulls =
            parse(Role::Pool, "u", &ok(&json("m", "[[1794895200000,null,5]]"))).expect("parses");
        assert_eq!(nulls.samples.len(), 0);
    }

    #[test]
    fn quality_codes_follow_the_cwms_bit_layout() {
        assert_eq!(judge(0), Verdict::Keep, "unscreened");
        assert_eq!(judge(1), Verdict::Keep, "screened, validity unknown");
        assert_eq!(judge(3), Verdict::Keep, "screened okay");
        assert_eq!(judge(5), Verdict::Drop, "screened missing");
        assert_eq!(judge(9), Verdict::Flag, "screened questionable");
        assert_eq!(judge(17), Verdict::Drop, "screened rejected");
        // Okay, but the rate-of-change test failed (bit 17).
        assert_eq!(judge(3 | (1 << 17)), Verdict::Flag);
        // Bits beyond the test flags (replacement method, protected) do not matter.
        assert_eq!(judge(3 | (1 << 11) | (1 << 31)), Verdict::Keep);
    }

    #[test]
    fn questionable_values_are_flagged_and_rejected_ones_dropped() {
        let body = json(
            "m",
            "[[1794895200000,10.0,3],[1794896100000,11.0,9],[1794897000000,12.0,17],\
             [1794897900000,13.0,0]]",
        );
        let p = parse(Role::Pool, "u", &ok(&body)).expect("parses");
        let values: Vec<(f32, Option<Quality>)> =
            p.samples.iter().map(|s| (s.value, s.quality)).collect();
        assert_eq!(
            values,
            [(10.0, None), (11.0, Some(Quality::Flagged)), (13.0, None)]
        );
    }

    #[test]
    fn a_pool_in_feet_is_refused() {
        let err = parse(Role::Pool, "u", &ok(&json("ft", "[[1,2.0,3]]"))).expect_err("feet");
        assert!(matches!(err, SourceError::Parse(_)), "{err:?}");
        let err = parse(Role::Outflow, "u", &ok(&json("cfs", "[[1,2.0,3]]"))).expect_err("cfs");
        assert!(matches!(err, SourceError::Parse(_)), "{err:?}");
        let err = parse(Role::Outflow, "u", &ok(&json("m", "[[1,2.0,3]]"))).expect_err("wrong");
        assert!(matches!(err, SourceError::Parse(_)), "{err:?}");
    }

    #[test]
    fn rows_are_sorted_and_malformed_rows_skipped() {
        let body = json(
            "cms",
            r#"[[1794896100000,2.0,3],[1794895200000,1.0,3],["x",3.0,3],[1794897000000],[1794897900000,"y",3]]"#,
        );
        let p = parse(Role::Inflow, "u", &ok(&body)).expect("parses");
        let values: Vec<f32> = p.samples.iter().map(|s| s.value).collect();
        assert_eq!(values, [1.0, 2.0]);
    }

    #[test]
    fn the_next_page_cursor_extends_the_request() {
        let body = br#"{"units":"m","values":[[1794895200000,1.0,3]],"next-page":"abc=="}"#;
        let url = "https://x/timeseries?office=NAB&name=N&unit=SI";
        let p = parse(Role::Pool, url, &ok(body)).expect("parses");
        let next = p.next.expect("a next page");
        assert_eq!(next, format!("{url}&page=abc%3D%3D"));
        // A follow-up's own cursor replaces the old one.
        let again = parse(Role::Pool, &next, &ok(body)).expect("parses");
        assert_eq!(again.next, Some(next));
    }

    #[test]
    fn pages_join_into_one_series() {
        let raystown = find("NAB", "Raystown").expect("listed");
        let a = parse(
            Role::Outflow,
            "u",
            &ok(&json("cms", "[[1794896100000,2.0,3]]")),
        )
        .expect("parses");
        let b = parse(
            Role::Outflow,
            "u",
            &ok(&json("cms", "[[1794895200000,1.0,3]]")),
        )
        .expect("parses");
        let s = series(raystown, ORIGIN, &[a, b]).expect("a series");
        let values: Vec<f32> = s.samples().iter().map(|x| x.value).collect();
        assert_eq!(values, [1.0, 2.0]);
        assert_eq!(s.provenance().observed_at, UnixTime(1_794_896_100));
    }

    #[test]
    fn errors_map_to_source_errors() {
        let err = parse(
            Role::Pool,
            "u",
            &status(404, br#"{"message":"no such series"}"#),
        )
        .expect_err("404");
        assert_eq!(err, SourceError::NotFound(Some("no such series".into())));
        let err = parse(
            Role::Pool,
            "u",
            &status(400, b"<html><body>Bad</body></html>"),
        )
        .expect_err("400");
        assert_eq!(
            err,
            SourceError::Status {
                status: 400,
                message: None
            }
        );
        let mut limited = status(429, b"");
        limited.retry_after = Some(30);
        assert_eq!(
            parse(Role::Pool, "u", &limited).expect_err("429"),
            SourceError::RateLimited {
                retry_after_s: Some(30)
            }
        );
        assert!(matches!(
            parse(Role::Pool, "u", &ok(b"")).expect_err("empty"),
            SourceError::NoData(_)
        ));
        assert!(matches!(
            parse(Role::Pool, "u", &ok(b"<html>x</html>")).expect_err("html"),
            SourceError::Parse(_)
        ));
        assert!(matches!(
            parse(Role::Pool, "u", &ok(b"{\"units\":")).expect_err("cut"),
            SourceError::Parse(_)
        ));
    }

    #[test]
    fn the_credit_names_the_corps() {
        let a = attribution();
        assert_eq!(a.source, SourceId::CWMS);
        assert!(a.credit.contains("U.S. Army Corps of Engineers"));
    }
}
