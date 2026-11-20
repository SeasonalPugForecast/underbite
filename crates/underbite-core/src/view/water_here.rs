//! The water-here section of the map's point card: the water's name and kind, where the marker
//! is against it, the stocking lines for it, the official regulation and licence links for its
//! region, and the credits of the sources used.
//!
//! The words and the formatting are the view's. The regulation links are links to the agency's
//! pages with the date each was checked, never rules; the stocking lines are the stocking
//! card's, as the agency states them.

use schemars::JsonSchema;
use serde::Serialize;

use crate::discover::water_here::{Reach, WaterHere, WaterKind};
use crate::model::WaterType;
use crate::provenance::SourceId;
use crate::regulations::{self, RegKind, RegLink};
use crate::view::CreditVm;
use crate::view::cards::sources::{credit_text, licence_text, source_terms};
use crate::view::cards::stocking::StockingVm;
use crate::view::format::Formatter;
use crate::view::strings::{Key, SafetyKey, Strings};
use crate::view::trip::water_of;

/// The most stocking lines the section shows.
pub const MAX_STOCKING_LINES: usize = 6;

/// One regulation or licence link.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, JsonSchema)]
pub struct WaterLinkVm {
    /// "Freshwater regulations", "Fishing licence".
    pub title: String,
    /// "NYS DEC, link checked 2026-03-01".
    pub detail: String,
    /// Always `https://`.
    pub url: String,
}

/// What the section shows.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct WaterHereVm {
    /// The water's name, or "Unnamed stream".
    pub title: String,
    /// The source gave the water a name.
    pub named: bool,
    /// `stream`, `river`, `lake`, `pond`, `reservoir`, `tidal_river`, `estuary`, `bay` or
    /// `waterbody`.
    pub kind: String,
    /// "Stream", "Tidal river".
    pub kind_text: String,
    /// "The marker is on this water" or "120 ft from the marker".
    pub position: String,
    /// The water type "Add as spot" carries.
    pub water_type: WaterType,
    /// "River or stream", in the user's words.
    pub water_type_text: String,
    /// The name a new spot starts with; none for unnamed water.
    pub spot_name: Option<String>,
    /// "Stocking": the title of the lines below; none without lines.
    pub stocking_title: Option<String>,
    /// The agency's stocking lines for this water, newest first.
    pub stocking: Vec<String>,
    /// "Regulations and licence": the title of the links below; none without links.
    pub links_title: Option<String>,
    pub links: Vec<WaterLinkVm>,
    /// The regulations disclaimer, with links.
    pub disclaimer: Option<String>,
    /// Every source the section draws on.
    pub credits: Vec<CreditVm>,
}

/// What the section is built from.
#[derive(Clone, Copy, Debug)]
pub struct WaterHereInput<'a> {
    pub here: &'a WaterHere,
    /// The region the regulation links are for (ISO 3166-2, `US-NY`); none when it is not known.
    pub region: Option<&'a str>,
    /// The stocking lines of this water, newest first, from the lists that answered.
    pub stocking: &'a [StockingVm],
}

/// "Stream", "Tidal river", ...
fn kind_text(s: Strings, kind: WaterKind) -> &'static str {
    s.get(match kind {
        WaterKind::Stream => Key::WaterKindStream,
        WaterKind::River => Key::WaterKindRiver,
        WaterKind::Lake => Key::WaterKindLake,
        WaterKind::Pond => Key::WaterKindPond,
        WaterKind::Reservoir => Key::WaterKindReservoir,
        WaterKind::TidalRiver => Key::WaterKindTidalRiver,
        WaterKind::Estuary => Key::WaterKindEstuary,
        WaterKind::Bay => Key::WaterKindBay,
        WaterKind::Waterbody => Key::WaterKindWaterbody,
    })
}

/// A short distance: whole feet or metres under a kilometre, else the station distance's text.
fn distance_text(f: &Formatter, metres: f64) -> String {
    if metres < 1_000.0 {
        let unit = f.units.height;
        let v = unit.from_canonical(metres as f32).round().max(1.0);
        format!(
            "{} {}",
            f.strings.grouped(v, 0),
            f.strings.height_unit(unit)
        )
    } else {
        f.distance(Some(metres as f32)).full()
    }
}

fn credit(s: Strings, id: &SourceId) -> Option<CreditVm> {
    let terms = source_terms(id.as_str())?;
    Some(CreditVm {
        source: s.source_name(id),
        text: credit_text(terms, s),
        link: Some(terms.site_url.to_owned()).filter(|u| !u.is_empty()),
        licence: Some(licence_text(terms, s)),
    })
}

fn link_vm(s: Strings, link: RegLink) -> WaterLinkVm {
    let title = match link.kind {
        RegKind::Licence => SafetyKey::RegLicence,
        RegKind::Freshwater => SafetyKey::RegFreshwater,
        RegKind::Saltwater => SafetyKey::RegSaltwater,
        RegKind::Federal => SafetyKey::RegFederal,
    };
    let detail = s.safety_fmt(
        if link.verified {
            SafetyKey::RegChecked
        } else {
            SafetyKey::RegUnconfirmed
        },
        &[("agency", &link.label), ("date", &link.checked)],
    );
    WaterLinkVm {
        title: s.safety(title).to_owned(),
        detail,
        url: link.url,
    }
}

/// The section for the water at the marker.
pub fn build(f: &Formatter, input: &WaterHereInput<'_>) -> WaterHereVm {
    let s = f.strings;
    let here = input.here;
    let kind = kind_text(s, here.kind);
    let title = match &here.name {
        Some(name) => name.clone(),
        None => s.fmt(Key::WaterHereUnnamed, &[("kind", &kind.to_lowercase())]),
    };
    let position = match here.reach {
        Reach::On => s.get(Key::WaterHereOn).to_owned(),
        Reach::Away { distance_m } => s.fmt(
            Key::WaterHereAway,
            &[("distance", &distance_text(f, distance_m))],
        ),
    };
    let region = input.region.map(str::trim).filter(|r| !r.is_empty());
    let links: Vec<WaterLinkVm> = region
        .map(|r| regulations::links_in(r, water_of(here.water_type), "en"))
        .unwrap_or_default()
        .into_iter()
        .map(|l| link_vm(s, l))
        .collect();
    let stocking: Vec<String> = input
        .stocking
        .iter()
        .take(MAX_STOCKING_LINES)
        .map(|v| v.summary.clone())
        .collect();
    let mut credits: Vec<CreditVm> = credit(s, &SourceId::USGS_3DHP).into_iter().collect();
    if !stocking.is_empty() {
        credits.extend(credit(s, &SourceId::NY_DEC_STOCKING));
    }
    WaterHereVm {
        named: here.name.is_some(),
        spot_name: here.name.clone(),
        title,
        kind: here.kind.code().to_owned(),
        kind_text: kind.to_owned(),
        position,
        water_type: here.water_type,
        water_type_text: f.water_type_label(here.water_type),
        stocking_title: (!stocking.is_empty()).then(|| s.get(Key::CardStocking).to_owned()),
        stocking,
        links_title: (!links.is_empty()).then(|| s.get(Key::WaterHereLinksTitle).to_owned()),
        disclaimer: (!links.is_empty())
            .then(|| s.safety(SafetyKey::RegulationsDisclaimer).to_owned()),
        links,
        credits,
    }
}

/// The words around the section that do not depend on the water: its title, the loading line,
/// the button, and what a failed lookup says.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, JsonSchema)]
pub struct WaterHereWords {
    pub title: String,
    pub loading: String,
    /// "USGS 3DHP could not be reached".
    pub failed: String,
    pub add_as_spot: String,
}

/// The words of [`WaterHereWords`].
pub fn words(f: &Formatter) -> WaterHereWords {
    let s = f.strings;
    WaterHereWords {
        title: s.get(Key::WaterHereTitle).to_owned(),
        loading: s.get(Key::WaterHereLooking).to_owned(),
        failed: s.fmt(
            Key::NearbyFailed,
            &[("source", &s.source_name(&SourceId::USGS_3DHP))],
        ),
        add_as_spot: s.get(Key::WaterHereAdd).to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::units::{
        DischargeUnit, DistanceUnit, HeightUnit, PressureUnit, SpeedUnit, TempUnit,
    };
    use crate::view::strings::Strings;
    use crate::view::{DisplayUnits, Voice};
    use whether_core::presentation::TimeFormat;

    fn units(height: HeightUnit, distance: DistanceUnit) -> DisplayUnits {
        DisplayUnits {
            temperature: TempUnit::C,
            air_temperature: TempUnit::C,
            height,
            stage: height,
            lake_level: height,
            waves: height,
            discharge: DischargeUnit::M3s,
            current_speed: SpeedUnit::Ms,
            wind_speed: SpeedUnit::Kmh,
            pressure: PressureUnit::Hpa,
            distance,
        }
    }

    fn formatter() -> Formatter {
        Formatter::new(
            units(HeightUnit::Ft, DistanceUnit::Mi),
            Voice::Plain,
            TimeFormat::H24,
            "UTC",
        )
    }

    fn here(name: Option<&str>, kind: WaterKind, water_type: WaterType, reach: Reach) -> WaterHere {
        WaterHere {
            name: name.map(str::to_owned),
            kind,
            reach,
            water_type,
            stream_order: None,
            area_km2: None,
        }
    }

    #[test]
    fn unnamed_water_is_unnamed_in_the_core_words_and_never_given_a_name() {
        let f = formatter();
        let h = here(None, WaterKind::Stream, WaterType::RiverStream, Reach::On);
        let vm = build(
            &f,
            &WaterHereInput {
                here: &h,
                region: None,
                stocking: &[],
            },
        );
        assert_eq!(vm.title, "Unnamed stream");
        assert!(!vm.named);
        assert_eq!(vm.spot_name, None);
        assert_eq!(vm.position, "The marker is on this water");
        let h = here(
            None,
            WaterKind::TidalRiver,
            WaterType::TidalRiverEstuary,
            Reach::On,
        );
        let vm = build(
            &f,
            &WaterHereInput {
                here: &h,
                region: None,
                stocking: &[],
            },
        );
        assert_eq!(vm.title, "Unnamed tidal river");
        assert_eq!(vm.kind_text, "Tidal river");
    }

    #[test]
    fn regulation_links_follow_the_region_and_the_kind_of_water() {
        let f = formatter();
        let fresh = here(
            Some("Croton River"),
            WaterKind::Stream,
            WaterType::RiverStream,
            Reach::On,
        );
        let at = |region: Option<&str>, h: &WaterHere| {
            build(
                &f,
                &WaterHereInput {
                    here: h,
                    region,
                    stocking: &[],
                },
            )
        };
        let vm = at(Some("US-NY"), &fresh);
        let titles: Vec<&str> = vm.links.iter().map(|l| l.title.as_str()).collect();
        assert!(titles.contains(&"Freshwater regulations"), "{titles:?}");
        assert!(titles.contains(&"Fishing licence"), "{titles:?}");
        assert!(vm.links.iter().all(|l| l.url.starts_with("https://")));
        assert!(
            vm.links
                .iter()
                .all(|l| l.detail.contains("link checked") || l.detail.contains("not confirmed"))
        );
        assert!(vm.disclaimer.is_some());
        let salt = here(
            None,
            WaterKind::TidalRiver,
            WaterType::TidalRiverEstuary,
            Reach::On,
        );
        let titles: Vec<String> = at(Some("US-NY"), &salt)
            .links
            .into_iter()
            .map(|l| l.title)
            .collect();
        assert!(
            titles.iter().any(|t| t == "Saltwater regulations"),
            "{titles:?}"
        );
        assert!(
            !titles.iter().any(|t| t == "Freshwater regulations"),
            "{titles:?}"
        );
        let none = at(None, &fresh);
        assert!(none.links.is_empty() && none.links_title.is_none() && none.disclaimer.is_none());
    }

    #[test]
    fn distances_and_credits_come_from_the_view() {
        let f = formatter();
        let h = here(
            Some("Round Lake"),
            WaterKind::Lake,
            WaterType::LakeReservoir,
            Reach::Away { distance_m: 120.0 },
        );
        let vm = build(
            &f,
            &WaterHereInput {
                here: &h,
                region: None,
                stocking: &[],
            },
        );
        assert!(vm.position.ends_with("from the marker"), "{}", vm.position);
        assert_eq!(vm.position, "394 ft from the marker");
        let metric = Formatter::new(
            units(HeightUnit::M, DistanceUnit::Km),
            Voice::Plain,
            TimeFormat::H24,
            "UTC",
        );
        let vm = build(
            &metric,
            &WaterHereInput {
                here: &h,
                region: None,
                stocking: &[],
            },
        );
        assert_eq!(vm.position, "120 m from the marker");
        assert_eq!(vm.credits.len(), 1);
        assert_eq!(vm.credits[0].source, "USGS 3DHP");
        assert_eq!(
            vm.credits[0].text,
            Strings::new(Voice::Plain).safety(SafetyKey::AttrUsgs3dhpCredit)
        );
        assert_eq!(vm.stocking_title, None);
    }

    #[test]
    fn the_words_name_the_source_that_failed() {
        let w = words(&formatter());
        assert_eq!(w.title, "Water here");
        assert_eq!(w.failed, "USGS 3DHP could not be reached");
        assert_eq!(w.add_as_spot, "Add as spot");
    }
}
