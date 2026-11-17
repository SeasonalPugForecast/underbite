//! Chart structure from the NOAA ENC services: the picture groups of the NOAA Chart Display
//! Service and the points of ENC Direct to GIS.
//!
//! The Chart Display Service is a picture: its 13 display groups cannot be queried for features
//! (REST `identify` and `query` fail, `GetFeatureInfo` answers one pixel and carries no
//! coordinates). The three groups that show depth and structure are described by [`EncGroup`] and
//! become WMS picture layers in the catalogue. Tappable points (wrecks, obstructions, rocks) come
//! from ENC Direct to GIS, read by `underbite_providers::encdirect` into [`StructurePoint`]s.
//!
//! **Depths are not displayed.** Neither service states the unit or the vertical datum of the
//! depth a wreck or obstruction carries (S-57 stores metres below the cell's sounding datum, an
//! IHO convention NOAA's responses never repeat), so a point has no depth to show; see
//! [`StructurePoint::raw_valsou_unverified`].

use crate::map::GeoPoint;

/// The credit of the ENC Direct points.
pub const ENC_DIRECT_CREDIT: &str =
    "NOAA Office of Coast Survey, ENC Direct to GIS. Not to be used for navigation.";

/// The credit of the Chart Display pictures.
pub const ENC_PICTURE_CREDIT: &str =
    "NOAA Office of Coast Survey, NOAA Chart Display Service (ENC). Not to be used for navigation.";

/// A display group of the NOAA Chart Display Service that the app draws as a picture layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EncGroup {
    /// The WMS `LAYERS` value (a comma list of group ids; a single id here).
    pub groups: &'static str,
    /// What the service calls the group.
    pub service_name: &'static str,
    /// The shallowest zoom the app asks it at.
    pub min_zoom: u8,
    /// What was seen in the group and what was not verified.
    pub note: &'static str,
}

/// Group 2: depth areas, depth contours and soundings.
pub const DEPTHS: EncGroup = EncGroup {
    groups: "2",
    service_name: "Depths, currents, etc",
    min_zoom: 12,
    note: "Depth areas, contours and soundings. Soundings showed at zoom 16 and not at zoom 14 \
           in the cached tiles; the zoom at which each class appears was not measured and \
           follows each chart's scale. Depth units are as charted and not stated by the service.",
};

/// Group 3: wrecks, obstructions, cables and pipelines.
pub const SEABED: EncGroup = EncGroup {
    groups: "3",
    service_name: "Seabed, obstructions, pipelines",
    min_zoom: 12,
    note: "Wrecks, obstructions and foul areas. The image was empty at 38 m per pixel \
           (about 1:137,000) and rich at 35 m per pixel and finer, checked in the Lower Bay \
           only; the limit follows each chart's scale. Fish havens are drawn here by \
           inference, not verified.",
};

/// Group 5: restricted, caution and other special areas.
pub const SPECIAL_AREAS: EncGroup = EncGroup {
    groups: "5",
    service_name: "Special areas",
    min_zoom: 12,
    note: "Restricted and caution areas. The zoom at which they appear was not measured. \
           Regulations are not data: these are charted areas, not the rules.",
};

/// What a structure point is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StructureKind {
    Wreck,
    /// An obstruction (S-57 `OBSTRN`): a snag, a foul area, a fish haven and the like. The
    /// category says which.
    Obstruction,
    /// An underwater or awash rock (`UWTROC`).
    Rock,
}

impl StructureKind {
    /// The longest category text the layer holds: ENC Direct cuts `CATWRK` at 30 characters and
    /// `CATOBS` at 25. A rock has no category.
    pub const fn category_limit(self) -> Option<usize> {
        match self {
            Self::Wreck => Some(30),
            Self::Obstruction => Some(25),
            Self::Rock => None,
        }
    }
}

/// The date of a chart object's survey (`SORDAT`): ENC Direct gives `YYYYMMDD`, `YYYYMM` or
/// `YYYY`, so the month and day may be missing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceDate {
    pub year: i32,
    pub month: Option<u8>,
    pub day: Option<u8>,
}

impl SourceDate {
    /// Reads `YYYYMMDD`, `YYYYMM` or `YYYY`; anything else (including a month or day out of
    /// range) is none.
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        if !s.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let year: i32 = s.get(0..4)?.parse().ok()?;
        let month: Option<u8> = match s.len() {
            4 => None,
            6 | 8 => Some(s.get(4..6)?.parse().ok()?),
            _ => return None,
        };
        let day: Option<u8> = if s.len() == 8 {
            Some(s.get(6..8)?.parse().ok()?)
        } else {
            None
        };
        if month.is_some_and(|m| !(1..=12).contains(&m))
            || day.is_some_and(|d| !(1..=31).contains(&d))
        {
            return None;
        }
        Some(Self { year, month, day })
    }
}

/// A charted wreck, obstruction or rock from ENC Direct to GIS.
#[derive(Clone, Debug, PartialEq)]
pub struct StructurePoint {
    /// The feature's id in its layer (`OBJECTID`): stable within a band's layer, not across bands.
    pub object_id: u64,
    pub kind: StructureKind,
    /// `CATWRK` or `CATOBS` as ENC Direct words it: for example "dangerous wreck" or "snag/stump".
    /// Cut at [`StructureKind::category_limit`] characters by the service; none for a rock.
    pub category: Option<String>,
    /// Whether the category has as many characters as the layer holds, so it may be cut short.
    pub category_may_be_cut: bool,
    /// The charted name (`OBJNAM`), when it has one.
    pub name: Option<String>,
    /// When the object was surveyed or reported (`SORDAT`).
    pub source_date: Option<SourceDate>,
    pub position: GeoPoint,
    /// The chart cell the object comes from (`DSNM`, for example `US5NY1BE.000`).
    pub chart_cell: Option<String>,
    /// The raw `VALSOU` number, **never to be shown**. Its unit and vertical datum are stated by
    /// neither ENC Direct nor the Chart Display Service (the S-57 convention is metres below the
    /// cell's sounding datum, which these services do not repeat), so it must not be formatted
    /// as a depth or mixed with DEM depths. Kept so a later verification can use it.
    pub raw_valsou_unverified: Option<f64>,
}

impl StructurePoint {
    /// Whether the category says fish haven (an `OBSTRN` object with `CATOBS` "fish haven").
    pub fn is_fish_haven(&self) -> bool {
        self.kind == StructureKind::Obstruction
            && self
                .category
                .as_deref()
                .is_some_and(|c| c.eq_ignore_ascii_case("fish haven"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_dates_may_be_partial() {
        assert_eq!(
            SourceDate::parse("20131220"),
            Some(SourceDate {
                year: 2013,
                month: Some(12),
                day: Some(20)
            })
        );
        assert_eq!(
            SourceDate::parse("200604"),
            Some(SourceDate {
                year: 2006,
                month: Some(4),
                day: None
            })
        );
        assert_eq!(
            SourceDate::parse("1998"),
            Some(SourceDate {
                year: 1998,
                month: None,
                day: None
            })
        );
        for bad in [
            "",
            "2013122",
            "20131320",
            "20130001",
            "2013-12-20",
            "abcd",
            "é2013",
        ] {
            assert_eq!(SourceDate::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn groups_are_the_three_the_catalogue_draws() {
        assert_eq!(
            [DEPTHS.groups, SEABED.groups, SPECIAL_AREAS.groups],
            ["2", "3", "5"]
        );
        for g in [DEPTHS, SEABED, SPECIAL_AREAS] {
            assert!(g.note.len() > 20);
        }
        assert!(ENC_DIRECT_CREDIT.contains("Not to be used for navigation."));
        assert!(ENC_PICTURE_CREDIT.contains("Not to be used for navigation."));
    }

    #[test]
    fn a_fish_haven_is_an_obstruction_with_that_category() {
        let mut p = StructurePoint {
            object_id: 1,
            kind: StructureKind::Obstruction,
            category: Some("Fish Haven".to_owned()),
            category_may_be_cut: false,
            name: None,
            source_date: None,
            position: GeoPoint {
                lat: 30.0,
                lon: -86.0,
            },
            chart_cell: None,
            raw_valsou_unverified: None,
        };
        assert!(p.is_fish_haven());
        p.kind = StructureKind::Wreck;
        assert!(!p.is_fish_haven());
    }
}
