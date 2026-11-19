//! Species records: where a species has been seen, from GBIF, USGS Nonindigenous Aquatic Species
//! and iNaturalist, as `SpeciesRecord`s.
//!
//! The records are for the map only and are not part of the spot snapshot, so these modules have
//! no `Source` implementation: a host builds a request, sends the GET and hands the body to the
//! module's parser, as for the access layers.
//!
//! Shared here: [`Licence`], which reads the licence a record carries (a record with no licence or
//! an unknown one is dropped by the parsers), [`month_counts`] for the typical-season strip, and
//! [`without_inaturalist_duplicates`], since GBIF republishes iNaturalist's research-grade
//! observations.

pub mod gbif;
pub mod inat;
pub mod nas;

use std::collections::{BTreeMap, BTreeSet};

use underbite_core::map::Bbox;
use underbite_core::model::nearby::SpeciesRecord;
use underbite_core::provenance::SourceId;
use underbite_core::species::seasons::MonthObservations;

use crate::request::MAX_COORDINATE_DECIMALS;

/// A licence a species record may be shown under.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Licence {
    /// Creative Commons Zero: no rights reserved.
    Cc0,
    /// Creative Commons Attribution.
    CcBy,
    /// Creative Commons Attribution-NonCommercial.
    CcByNc,
    /// A U.S. Government work (USGS data), in the public domain.
    PublicDomain,
}

impl Licence {
    /// The licence a source's text names: a Creative Commons URL (`http` or `https`, with or
    /// without `legalcode`), GBIF's codes (`CC0_1_0`, `CC_BY_4_0`, `CC_BY_NC_4_0`), iNaturalist's
    /// codes (`cc0`, `cc-by`, `cc-by-nc`) or [`Licence::code`]. Anything else, including the
    /// share-alike and no-derivatives licences and an empty text, is `None`.
    pub fn classify(text: &str) -> Option<Self> {
        let t = text.trim().to_ascii_lowercase();
        let path = t
            .strip_prefix("https://")
            .or_else(|| t.strip_prefix("http://"))
            .map(|rest| rest.strip_prefix("www.").unwrap_or(rest));
        if let Some(path) = path {
            let rest = path.strip_prefix("creativecommons.org/")?;
            let mut parts = rest.split('/').filter(|p| !p.is_empty());
            return match (parts.next(), parts.next()) {
                (Some("publicdomain"), Some("zero")) => Some(Self::Cc0),
                (Some("licenses"), Some("by")) => Some(Self::CcBy),
                (Some("licenses"), Some("by-nc")) => Some(Self::CcByNc),
                _ => None,
            };
        }
        match t.as_str() {
            "cc0" | "cc0_1_0" | "cc0-1.0" => Some(Self::Cc0),
            "cc-by" | "cc_by_4_0" | "cc-by-4.0" => Some(Self::CcBy),
            "cc-by-nc" | "cc_by_nc_4_0" | "cc-by-nc-4.0" => Some(Self::CcByNc),
            "public-domain" => Some(Self::PublicDomain),
            _ => None,
        }
    }

    /// The licence of a record, from its `licence` text.
    pub fn of(record: &SpeciesRecord) -> Option<Self> {
        record.licence.as_deref().and_then(Self::classify)
    }

    /// A stable code: `cc0`, `cc-by`, `cc-by-nc` or `public-domain`.
    pub fn code(self) -> &'static str {
        match self {
            Self::Cc0 => "cc0",
            Self::CcBy => "cc-by",
            Self::CcByNc => "cc-by-nc",
            Self::PublicDomain => "public-domain",
        }
    }

    /// Whether the licence asks for the source to be credited.
    pub fn needs_credit(self) -> bool {
        matches!(self, Self::CcBy | Self::CcByNc)
    }

    /// Whether the licence permits commercial use.
    pub fn allows_commercial(self) -> bool {
        !matches!(self, Self::CcByNc)
    }
}

/// Observation counts per calendar month from the months of records (1 to 12; other values are
/// ignored), January first, months with no record left out. This is the shape the typical-season
/// strip takes.
pub fn month_counts(months: impl IntoIterator<Item = u8>) -> Vec<MonthObservations> {
    let mut counts: BTreeMap<u8, u32> = BTreeMap::new();
    for m in months.into_iter().filter(|m| (1..=12).contains(m)) {
        *counts.entry(m).or_default() += 1;
    }
    counts
        .into_iter()
        .map(|(month, count)| MonthObservations { month, count })
        .collect()
}

/// The iNaturalist observation id in a URL of the form `…inaturalist.org/observations/<id>`.
pub fn inaturalist_observation_id(url: &str) -> Option<u64> {
    let t = url.trim();
    let rest = t
        .strip_prefix("https://")
        .or_else(|| t.strip_prefix("http://"))?;
    let rest = rest.strip_prefix("www.").unwrap_or(rest);
    let id = rest.strip_prefix("inaturalist.org/observations/")?;
    let id = id.split(['/', '?', '#']).next()?;
    id.parse().ok()
}

/// GBIF records that are not also among the iNaturalist records shown: a GBIF record republished
/// from iNaturalist is left out when an iNaturalist record has the same observation id (the
/// iNaturalist record's `id`). Order is kept.
pub fn without_inaturalist_duplicates(
    gbif: Vec<gbif::GbifRecord>,
    inaturalist: &[SpeciesRecord],
) -> Vec<gbif::GbifRecord> {
    let shown: BTreeSet<u64> = inaturalist
        .iter()
        .filter(|r| r.source == SourceId::INATURALIST)
        .filter_map(|r| r.id.trim().parse().ok())
        .collect();
    gbif.into_iter()
        .filter(|r| r.inaturalist_id.is_none_or(|id| !shown.contains(&id)))
        .collect()
}

/// A box fit to send: finite, inside the world and not inverted, rounded outward to
/// [`MAX_COORDINATE_DECIMALS`] so it still covers the area asked for. `None` for a box that
/// cannot be sent (one across the antimeridian is two boxes).
pub fn request_box(bbox: Bbox) -> Option<Bbox> {
    let ok = [bbox.west, bbox.south, bbox.east, bbox.north]
        .iter()
        .all(|v| v.is_finite())
        && bbox.west <= bbox.east
        && bbox.south <= bbox.north
        && bbox.west >= -180.0
        && bbox.east <= 180.0
        && bbox.south >= -90.0
        && bbox.north <= 90.0;
    if !ok {
        return None;
    }
    let scale = 10f64.powi(i32::from(MAX_COORDINATE_DECIMALS));
    // A value already on the grid stays put (`-74.4 * 100` is `-7440.000000000001`).
    let down = |v: f64| (v * scale + 1e-6).floor() / scale;
    let up = |v: f64| (v * scale - 1e-6).ceil() / scale;
    Some(Bbox {
        west: down(bbox.west).max(-180.0),
        south: down(bbox.south).max(-90.0),
        east: up(bbox.east).min(180.0),
        north: up(bbox.north).min(90.0),
    })
}

/// A degree value as sent in a query: [`MAX_COORDINATE_DECIMALS`] places, never `-0.00`.
pub(crate) fn fmt_degrees(value: f64) -> String {
    let text = format!(
        "{value:.prec$}",
        prec = usize::from(MAX_COORDINATE_DECIMALS)
    );
    if text.starts_with('-') && text.bytes().all(|b| matches!(b, b'-' | b'0' | b'.')) {
        text[1..].to_owned()
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn licences_are_read_from_urls_and_codes() {
        for (text, want) in [
            (
                "http://creativecommons.org/publicdomain/zero/1.0/legalcode",
                Licence::Cc0,
            ),
            (
                "https://creativecommons.org/publicdomain/zero/1.0/",
                Licence::Cc0,
            ),
            (
                "http://creativecommons.org/licenses/by/4.0/legalcode",
                Licence::CcBy,
            ),
            (
                "https://www.creativecommons.org/licenses/by-nc/4.0/",
                Licence::CcByNc,
            ),
            ("CC_BY_NC_4_0", Licence::CcByNc),
            ("CC0_1_0", Licence::Cc0),
            (" cc-by ", Licence::CcBy),
            ("cc-by-nc", Licence::CcByNc),
            ("public-domain", Licence::PublicDomain),
        ] {
            assert_eq!(Licence::classify(text), Some(want), "{text}");
        }
        for text in [
            "",
            "unspecified",
            "cc-by-sa",
            "cc-by-nc-nd",
            "http://creativecommons.org/licenses/by-sa/4.0/",
            "http://creativecommons.org/licenses/by-nc-sa/4.0/legalcode",
            "https://example.invalid/licenses/by/4.0/",
            "All rights reserved",
        ] {
            assert_eq!(Licence::classify(text), None, "{text}");
        }
        assert!(Licence::CcByNc.needs_credit() && !Licence::CcByNc.allows_commercial());
        assert!(!Licence::Cc0.needs_credit() && Licence::Cc0.allows_commercial());
        for l in [
            Licence::Cc0,
            Licence::CcBy,
            Licence::CcByNc,
            Licence::PublicDomain,
        ] {
            assert_eq!(Licence::classify(l.code()), Some(l));
        }
    }

    #[test]
    fn months_are_counted_in_calendar_order() {
        let got = month_counts([5, 4, 5, 0, 13, 12, 5]);
        assert_eq!(
            got,
            vec![
                MonthObservations { month: 4, count: 1 },
                MonthObservations { month: 5, count: 3 },
                MonthObservations {
                    month: 12,
                    count: 1
                },
            ]
        );
        assert_eq!(month_counts([]).len(), 0);
    }

    #[test]
    fn inaturalist_ids_come_from_observation_urls() {
        for (url, want) in [
            (
                "https://www.inaturalist.org/observations/351381638",
                Some(351_381_638),
            ),
            ("http://inaturalist.org/observations/12/", Some(12)),
            ("https://www.inaturalist.org/observations/7?x=1", Some(7)),
            ("https://www.inaturalist.org/photos/7", None),
            ("https://example.invalid/observations/9200001", None),
            ("9200001", None),
        ] {
            assert_eq!(inaturalist_observation_id(url), want, "{url}");
        }
    }

    #[test]
    fn boxes_round_outward_and_bad_ones_are_refused() {
        let b = request_box(Bbox {
            west: -74.0170,
            south: 40.7033,
            east: -73.9911,
            north: 40.7201,
        })
        .unwrap();
        assert_eq!(
            (b.west, b.south, b.east, b.north),
            (-74.02, 40.7, -73.99, 40.73)
        );
        let edge = request_box(Bbox {
            west: -180.0,
            south: -90.0,
            east: 180.0,
            north: 90.0,
        })
        .unwrap();
        assert_eq!((edge.west, edge.north), (-180.0, 90.0));
        let on_grid = request_box(Bbox {
            west: -74.4,
            south: 40.4,
            east: -73.6,
            north: 41.0,
        })
        .unwrap();
        assert_eq!(
            (on_grid.west, on_grid.south, on_grid.east, on_grid.north),
            (-74.4, 40.4, -73.6, 41.0)
        );
        for bad in [
            (1.0, 0.0, 0.0, 1.0),
            (0.0, 1.0, 1.0, 0.0),
            (f64::NAN, 0.0, 1.0, 1.0),
            (0.0, 0.0, 181.0, 1.0),
            (0.0, -91.0, 1.0, 1.0),
        ] {
            let (west, south, east, north) = bad;
            assert!(
                request_box(Bbox {
                    west,
                    south,
                    east,
                    north
                })
                .is_none(),
                "{bad:?}"
            );
        }
    }
}
