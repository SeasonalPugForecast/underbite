//! The EPA Cyanobacteria Assessment Network: cyanobacteria cell concentrations of a lake from satellite composites.
//!
//! `CyAN` is an experimental research product and EPA's feed has gaps (no composite after March
//! 2026 at the time of writing), so everything here fails softly: a response that is not a lake,
//! has no composites, or whose newest composite is more than [`STALE_AFTER_S`] old is an error
//! the caller records as "no bloom data". It is never turned into a "no bloom" reading.
//!
//! [`request_url`] builds the one request; [`parse`] reads the body into a [`CyanReading`] that
//! carries the date of the composite, and [`CyanReading::bloom_info`] places it in the
//! snapshot's [`BloomInfo`].

use serde::Deserialize;
use underbite_core::model::{BloomInfo, ExternalLink};
use underbite_core::provenance::{Attribution, Provenance, Quality, Relation, SourceId, ValueKind};
use underbite_core::series::{Sample, Series};
use underbite_core::time::{LocalDate, UnixTime, UtcOffset};

use crate::request::{Coordinates, RoundedPoint};
use crate::source::SourceError;

/// Records older than this (about two weeks) mean the composite feed is not current.
pub const STALE_AFTER_S: i64 = 14 * 86_400;

/// The credit every display of `CyAN` values shows.
pub const CREDIT: &str = "U.S. EPA Cyanobacteria Assessment Network (CyAN)";

/// The web application's own caveat, shown with every `CyAN` value.
pub const EXPERIMENTAL_NOTE: &str = "Provisional satellite derived measures of cyanobacteria, which may contain errors and should be considered a research tool.";

/// EPA's page for the application, linked from the bloom card.
pub const INFO_URL: &str =
    "https://www.epa.gov/water-research/cyanobacteria-assessment-network-application-cyan-app";

const BASE: &str = "https://cyan.epa.gov/cyan/cyano/location/data";

/// How the composites are averaged.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Frequency {
    /// Seven-day composites, stamped on their last day. The card uses these.
    Weekly,
    /// Single-day composites.
    Daily,
}

impl Frequency {
    fn param(self) -> &'static str {
        match self {
            Self::Weekly => "weekly",
            Self::Daily => "daily",
        }
    }
}

/// The credit as an [`Attribution`].
pub fn attribution() -> Attribution {
    Attribution {
        source: SourceId::CYAN,
        credit: format!("{CREDIT}. {EXPERIMENTAL_NOTE}"),
        url: Some(INFO_URL.into()),
    }
}

/// The link to EPA's page for the application.
pub fn info_link() -> ExternalLink {
    ExternalLink {
        label: "EPA CyAN".into(),
        url: INFO_URL.into(),
    }
}

/// The request URL for a lake point. The point is already rounded for sending.
pub fn request_url(point: RoundedPoint, frequency: Frequency) -> String {
    let (lat, lon) = point.fmt_param();
    format!(
        "{BASE}/{lat}/{lon}/all?type=olci&frequency={}",
        frequency.param()
    )
}

/// One composite's values near the point.
#[derive(Clone, Debug, PartialEq)]
pub struct CyanRecord {
    /// The composite's date as an instant: US Eastern midnight, which is 04:00 or 05:00 UTC.
    pub image_time: UnixTime,
    /// The composite's calendar date (weekly composites: the last day of the window).
    pub image_date: LocalDate,
    /// Mean over the valid cells, cells/mL.
    pub mean_cells_ml: f32,
    /// Maximum over the same cells, cells/mL.
    pub max_cells_ml: Option<f32>,
    /// Cells with a valid retrieval.
    pub valid_cells: Option<u32>,
    /// The cell used, as the source snaps it.
    pub cell: Option<Coordinates>,
}

/// A lake's recent composites, newest first.
#[derive(Clone, Debug, PartialEq)]
pub struct CyanReading {
    /// The mapped waterbody nearest the point as the source names it.
    pub location: String,
    /// The composites, newest first; never empty and the newest is not stale.
    pub records: Vec<CyanRecord>,
}

impl CyanReading {
    /// The newest composite.
    pub fn latest(&self) -> &CyanRecord {
        // INVARIANT: `parse` returns a reading only with at least one record.
        &self.records[0]
    }

    /// The provenance of the values: a satellite composite, provisional, dated by the newest one.
    pub fn provenance(&self, spot: Coordinates) -> Provenance {
        let latest = self.latest();
        Provenance {
            source: SourceId::CYAN,
            station: None,
            distance_m: latest
                .cell
                .map(|c| distance_m(spot.latitude, spot.longitude, c.latitude, c.longitude)),
            relation: Relation::Satellite,
            observed_at: latest.image_time,
            kind: ValueKind::Satellite,
            quality: Quality::Provisional,
        }
    }

    fn series(&self, spot: Coordinates, pick: impl Fn(&CyanRecord) -> Option<f32>) -> Series<f32> {
        let samples = self
            .records
            .iter()
            .filter_map(|r| {
                pick(r).map(|value| Sample {
                    time: r.image_time,
                    value,
                    quality: None,
                })
            })
            .collect();
        Series::new(self.provenance(spot), samples)
    }

    /// The mean cell concentration of each composite, for `cyanobacteria_cells_ml`.
    pub fn mean_series(&self, spot: Coordinates) -> Series<f32> {
        self.series(spot, |r| Some(r.mean_cells_ml))
    }

    /// The maximum cell concentration of each composite.
    pub fn max_series(&self, spot: Coordinates) -> Series<f32> {
        self.series(spot, |r| r.max_cells_ml)
    }

    /// The bloom block for the snapshot, with the composite's date so a display can say how old
    /// it is. The mean goes to `cyanobacteria_cells_ml` through [`CyanReading::mean_series`].
    pub fn bloom_info(&self, spot: Coordinates) -> BloomInfo {
        let latest = self.latest();
        BloomInfo {
            cyan_max_cells_ml: Some(self.max_series(spot)).filter(|s| !s.is_empty()),
            cyan_valid_cells: latest.valid_cells,
            cyan_image_date: Some(latest.image_date),
            hab: None,
            links: vec![info_link()],
        }
    }
}

#[derive(Deserialize)]
struct Envelope {
    #[serde(rename = "metaInfo")]
    meta: Option<Meta>,
    outputs: Option<Vec<Output>>,
}

#[derive(Deserialize)]
struct Meta {
    #[serde(rename = "locationName")]
    location_name: Option<String>,
}

#[derive(Deserialize)]
struct Output {
    #[serde(rename = "imageDateLong")]
    image_date_ms: Option<i64>,
    #[serde(rename = "cellConcentration")]
    mean: Option<f64>,
    #[serde(rename = "maxCellConcentration")]
    max: Option<f64>,
    #[serde(rename = "validCellsCount")]
    valid: Option<u32>,
    latitude: Option<f64>,
    longitude: Option<f64>,
}

fn record(output: &Output) -> Option<CyanRecord> {
    let ms = output.image_date_ms?;
    let mean = output.mean.filter(|v| v.is_finite() && *v >= 0.0)?;
    let image_time = UnixTime(ms.div_euclid(1000));
    Some(CyanRecord {
        image_time,
        // The stamp is Eastern midnight (04:00 or 05:00 UTC), so its UTC date is the local date.
        image_date: image_time.local_date(UtcOffset(0)),
        mean_cells_ml: mean as f32,
        max_cells_ml: output
            .max
            .filter(|v| v.is_finite() && *v >= 0.0)
            .map(|v| v as f32),
        valid_cells: output.valid,
        cell: output
            .latitude
            .zip(output.longitude)
            .map(|(latitude, longitude)| Coordinates {
                latitude,
                longitude,
            }),
    })
}

/// Parses a `CyAN` location response at time `now`.
///
/// # Errors
/// - [`SourceError::NoData`]: the point is not a mapped waterbody (`Unknown Location`) or the
///   waterbody has no composites.
/// - [`SourceError::Stale`]: the newest composite is more than [`STALE_AFTER_S`] before `now`;
///   `latest` is its time. A stale feed says nothing about whether there is a bloom.
/// - [`SourceError::Parse`]: the body is not the expected shape.
pub fn parse(body: &[u8], now: UnixTime) -> Result<CyanReading, SourceError> {
    let envelope: Envelope = serde_json::from_slice(body)
        .map_err(|e| SourceError::Parse(format!("not a CyAN response: {e}")))?;
    let (Some(meta), Some(outputs)) = (envelope.meta, envelope.outputs) else {
        return Err(SourceError::Parse(
            "CyAN response has no metaInfo or outputs".into(),
        ));
    };
    let location = meta.location_name.unwrap_or_default();
    if location.is_empty() || location == "Unknown Location" {
        return Err(SourceError::NoData(Some(
            "not a mapped CyAN waterbody".into(),
        )));
    }
    if outputs.is_empty() {
        return Err(SourceError::NoData(Some(
            "no satellite composites for this waterbody".into(),
        )));
    }
    let mut records: Vec<CyanRecord> = outputs.iter().filter_map(record).collect();
    if records.is_empty() {
        return Err(SourceError::Parse(
            "CyAN outputs carry no usable composite".into(),
        ));
    }
    records.sort_by_key(|r| std::cmp::Reverse(r.image_time));
    let latest = records[0].image_time;
    if now.0 - latest.0 > STALE_AFTER_S {
        return Err(SourceError::Stale { latest });
    }
    Ok(CyanReading { location, records })
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
    use underbite_core::time::parse_rfc3339;

    const SPOT: Coordinates = Coordinates {
        latitude: 43.2,
        longitude: -76.0,
    };

    fn at(s: &str) -> UnixTime {
        parse_rfc3339(s).expect("a valid time")
    }

    fn oneida() -> Vec<u8> {
        fixture_bytes("cyan", "cyan-oneida-lake-weekly-2026-11-17.json")
    }

    #[test]
    fn request_url_uses_rounded_coordinates() {
        let point = RoundedPoint::new(
            Coordinates {
                latitude: 43.204_9,
                longitude: -75.996_1,
            },
            2,
        );
        assert_eq!(
            request_url(point, Frequency::Weekly),
            "https://cyan.epa.gov/cyan/cyano/location/data/43.2/-76/all?type=olci&frequency=weekly"
        );
        assert!(request_url(point, Frequency::Daily).ends_with("frequency=daily"));
    }

    #[test]
    fn parses_the_weekly_fixture_with_its_image_date() {
        let reading = parse(&oneida(), at("2026-03-30T12:00:00Z")).expect("fresh in March");
        assert_eq!(reading.location, "Maple Bay");
        assert_eq!(reading.records.len(), 16);
        let latest = reading.latest();
        assert_eq!(
            latest.image_date,
            LocalDate {
                year: 2026,
                month: 3,
                day: 28
            }
        );
        assert_eq!(latest.valid_cells, Some(4));
        assert!((latest.mean_cells_ml - 19_054.607).abs() < 1.0);
        assert!((latest.max_cells_ml.expect("a maximum") - 28_054.338).abs() < 1.0);
        let dates: Vec<_> = reading.records.iter().map(|r| r.image_time).collect();
        assert!(dates.windows(2).all(|w| w[0] > w[1]), "newest first");
    }

    #[test]
    fn a_stale_feed_is_never_no_bloom() {
        // The recorded response is from November; its newest composite is from March.
        let err = parse(&oneida(), at("2026-11-17T12:00:00Z")).unwrap_err();
        assert_eq!(
            err,
            SourceError::Stale {
                latest: UnixTime(1_774_670_400)
            }
        );
        // The edge: exactly 14 days is current, one second more is stale.
        let edge = UnixTime(1_774_670_400 + STALE_AFTER_S);
        assert!(parse(&oneida(), edge).is_ok());
        assert!(matches!(
            parse(&oneida(), UnixTime(edge.0 + 1)),
            Err(SourceError::Stale { .. })
        ));
    }

    #[test]
    fn unknown_location_is_no_data() {
        let body = fixture_bytes("cyan", "cyan-new-croton-unknown-location-2026-11-17.json");
        let err = parse(&body, at("2026-11-17T12:00:00Z")).unwrap_err();
        assert!(matches!(err, SourceError::NoData(_)), "{err:?}");
    }

    #[test]
    fn a_named_waterbody_without_composites_is_no_data() {
        let body = br#"{"metaInfo":{"locationName":"Maple Bay"},"outputs":[]}"#;
        assert!(matches!(
            parse(body, at("2026-11-17T12:00:00Z")),
            Err(SourceError::NoData(_))
        ));
    }

    #[test]
    fn sparse_estuary_records_are_stale_not_clear() {
        let body = fixture_bytes("cyan", "cyan-the-battery-weekly-2026-11-17.json");
        let err = parse(&body, at("2026-11-17T12:00:00Z")).unwrap_err();
        assert!(matches!(err, SourceError::Stale { .. }), "{err:?}");
    }

    #[test]
    fn shape_changes_are_parse_errors() {
        let now = at("2026-11-17T12:00:00Z");
        for body in [
            &b"not json"[..],
            br#"{"outputs":[]}"#,
            br#"{"metaInfo":{"locationName":"X"}}"#,
            br#"{"metaInfo":{"locationName":"X"},"outputs":[{"foo":1}]}"#,
            br#"{"metaInfo":{"locationName":"X"},"outputs":[{"imageDateLong":"soon","cellConcentration":1}]}"#,
            br"[]",
        ] {
            assert!(
                matches!(parse(body, now), Err(SourceError::Parse(_))),
                "{}",
                String::from_utf8_lossy(body)
            );
        }
    }

    #[test]
    fn records_without_a_value_are_skipped() {
        let body = br#"{"metaInfo":{"locationName":"X"},"outputs":[
            {"imageDateLong":1774670400000,"cellConcentration":-1.0},
            {"imageDateLong":1774065600000,"cellConcentration":10.0,"validCellsCount":2}]}"#;
        let reading = parse(body, at("2026-03-30T00:00:00Z")).expect("one usable record");
        assert_eq!(reading.records.len(), 1);
        assert_eq!(reading.latest().image_date.day, 21);
    }

    #[test]
    fn bloom_info_carries_the_image_date_and_series() {
        let reading = parse(&oneida(), at("2026-03-30T12:00:00Z")).unwrap();
        let bloom = reading.bloom_info(SPOT);
        assert_eq!(bloom.cyan_image_date, Some(reading.latest().image_date));
        assert_eq!(bloom.cyan_valid_cells, Some(4));
        assert_eq!(bloom.links, vec![info_link()]);
        let max = bloom.cyan_max_cells_ml.expect("max series");
        assert_eq!(max.samples().len(), 16);
        let mean = reading.mean_series(SPOT);
        let p = mean.provenance();
        assert_eq!(p.source, SourceId::CYAN);
        assert_eq!(p.kind, ValueKind::Satellite);
        assert_eq!(p.relation, Relation::Satellite);
        assert_eq!(p.observed_at, reading.latest().image_time);
        assert!(p.distance_m.expect("a distance") < 500.0);
    }

    #[test]
    fn the_credit_names_the_source_and_the_caveat() {
        let a = attribution();
        assert_eq!(a.source, SourceId::CYAN);
        assert!(a.credit.contains("U.S. EPA"));
        assert!(a.credit.contains("research tool"));
    }

    /// Run by hand: save a live response with curl (the URL is `request_url`'s) and point
    /// `UNDERBITE_LIVE_CYAN_BODY` at the file. The shape must still parse or fail softly.
    #[test]
    #[ignore = "needs a response body saved from the live EPA service"]
    fn live_cyan_shape() {
        let path = std::env::var("UNDERBITE_LIVE_CYAN_BODY").expect("UNDERBITE_LIVE_CYAN_BODY");
        let body = std::fs::read(path).expect("a readable body");
        let now = UnixTime(
            std::time::UNIX_EPOCH
                .elapsed()
                .expect("a clock")
                .as_secs()
                .cast_signed(),
        );
        match parse(&body, now) {
            Ok(_) | Err(SourceError::NoData(_) | SourceError::Stale { .. }) => {}
            Err(e) => panic!("CyAN shape changed: {e}"),
        }
    }
}
