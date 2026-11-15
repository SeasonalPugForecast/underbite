//! The words the view shows, in both voices.
//!
//! One table holds every fixed user-visible text. Playful texts ([`Key`]) have a plain and a Luna
//! variant; safety texts ([`SafetyKey`]) have exactly one, and no function takes a voice for them:
//! the split is by type, so a safety text cannot be given a Luna variant by accident. A later
//! localisation replaces this table.

use crate::model::WaterType;
use crate::provenance::SourceId;
use crate::view::{CardId, NoticeKind, Voice};

/// Texts that may be playful. Each has a plain and a Luna variant (often the same).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Key {
    /// `{n}` minutes, hours or days.
    UpdatedJustNow,
    UpdatedMinutes,
    UpdatedHours,
    UpdatedDays,
    AgeJustNow,
    AgeMinutes,
    AgeHours,
    AgeDays,
    /// `{station}`, `{distance}`.
    StationFooter,
    /// `{reason}`.
    GoNoGoBad,
    GoNoGoGood,
    /// `{range}`.
    NextWindow,
    NoWindows,
    NoData,
    NoCards,
    Loading,
    Flood,
    Ebb,
    Slack,
    Rising,
    Falling,
    Steady,
    /// "Rising 0.4 ft in 3 h": `{direction}`, `{amount}`, `{hours}`.
    TrendOver,
    Upstream,
    Downstream,
    Modelled,
    Satellite,
    /// `{distance}`, `{limit}`: why a value is greyed.
    FarReason,
    /// `{age}`, `{limit}`.
    StaleReason,
    PercentileMuchBelow,
    PercentileBelow,
    PercentileNormal,
    PercentileAbove,
    PercentileMuchAbove,
    /// `{band}`, `{ordinal}`, `{date}`.
    PercentileText,
    WaterRiver,
    WaterLake,
    WaterTidal,
    WaterBay,
    WaterSurf,
    WaterNearshore,
    WaterOffshore,
    WaterGreatLakes,
    WaterPond,
    WaterIce,
    Water,
    /// The placeholder for an unknown value.
    Missing,
}

/// Texts that are never playful: safety, advisories and regulations. No voice parameter exists
/// for them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SafetyKey {
    NotForNavigation,
    Modelled,
    Provisional,
    Approximate,
    TraditionalCue,
    TextOnly,
    RegulationsLink,
}

impl SafetyKey {
    /// The one text.
    pub const fn text(self) -> &'static str {
        match self {
            Self::NotForNavigation => "Not for navigation.",
            Self::Modelled => {
                "Modelled sea level, not a tide prediction; times can be off by hours near the coast."
            }
            Self::Provisional => "Provisional data, subject to revision.",
            Self::Approximate => "Approximate: check conditions locally before you go.",
            Self::TraditionalCue => "A traditional cue, not a promise.",
            Self::TextOnly => "Text only: open the map for the picture.",
            Self::RegulationsLink => {
                "Check the official source for current regulations; this is not legal advice."
            }
        }
    }
}

impl From<NoticeKind> for SafetyKey {
    fn from(kind: NoticeKind) -> Self {
        match kind {
            NoticeKind::NotForNavigation => Self::NotForNavigation,
            NoticeKind::Modelled => Self::Modelled,
            NoticeKind::Provisional => Self::Provisional,
            NoticeKind::Approximate => Self::Approximate,
            NoticeKind::TraditionalCue => Self::TraditionalCue,
            NoticeKind::TextOnly => Self::TextOnly,
        }
    }
}

impl Key {
    /// Every key, for tests and for a later localisation.
    pub const ALL: [Self; 47] = [
        Self::UpdatedJustNow,
        Self::UpdatedMinutes,
        Self::UpdatedHours,
        Self::UpdatedDays,
        Self::AgeJustNow,
        Self::AgeMinutes,
        Self::AgeHours,
        Self::AgeDays,
        Self::StationFooter,
        Self::GoNoGoBad,
        Self::GoNoGoGood,
        Self::NextWindow,
        Self::NoWindows,
        Self::NoData,
        Self::NoCards,
        Self::Loading,
        Self::Flood,
        Self::Ebb,
        Self::Slack,
        Self::Rising,
        Self::Falling,
        Self::Steady,
        Self::TrendOver,
        Self::Upstream,
        Self::Downstream,
        Self::Modelled,
        Self::Satellite,
        Self::FarReason,
        Self::StaleReason,
        Self::PercentileMuchBelow,
        Self::PercentileBelow,
        Self::PercentileNormal,
        Self::PercentileAbove,
        Self::PercentileMuchAbove,
        Self::PercentileText,
        Self::WaterRiver,
        Self::WaterLake,
        Self::WaterTidal,
        Self::WaterBay,
        Self::WaterSurf,
        Self::WaterNearshore,
        Self::WaterOffshore,
        Self::WaterGreatLakes,
        Self::WaterPond,
        Self::WaterIce,
        Self::Water,
        Self::Missing,
    ];

    /// `(plain, luna)`.
    const fn variants(self) -> (&'static str, &'static str) {
        match self {
            Self::UpdatedJustNow => ("updated just now", "updated just now"),
            Self::UpdatedMinutes => ("updated {n} min ago", "updated {n} min ago"),
            Self::UpdatedHours => ("updated {n} h ago", "updated {n} h ago"),
            Self::UpdatedDays => ("updated {n} d ago", "updated {n} d ago"),
            Self::AgeJustNow => ("just now", "just now"),
            Self::AgeMinutes => ("{n} min ago", "{n} min ago"),
            Self::AgeHours => ("{n} h ago", "{n} h ago"),
            Self::AgeDays => ("{n} d ago", "{n} d ago"),
            Self::StationFooter => (
                "{station}, {distance}",
                "Luna's watching {station}, {distance}",
            ),
            Self::GoNoGoBad => (
                "Not recommended: {reason}",
                "Luna's not impressed: {reason}",
            ),
            Self::GoNoGoGood => ("Good conditions.", "Luna approves."),
            Self::NextWindow => ("Next fishing window {range}", "Next snack time {range}"),
            Self::NoWindows => (
                "No fishing windows in this period.",
                "No snack times in this period.",
            ),
            Self::NoData => ("No data for this spot yet.", "Luna found nothing here yet."),
            Self::NoCards => ("Nothing to show.", "Luna has nothing to show."),
            Self::Loading => ("Loading", "Luna is paddling over"),
            Self::Flood => ("Flood", "Flood"),
            Self::Ebb => ("Ebb", "Ebb"),
            Self::Slack => ("Slack", "Slack"),
            Self::Rising => ("Rising", "Rising"),
            Self::Falling => ("Falling", "Falling"),
            Self::Steady => ("Steady", "Steady"),
            Self::TrendOver => (
                "{direction} {amount} in {hours} h",
                "{direction} {amount} in {hours} h",
            ),
            Self::Upstream => ("upstream", "upstream"),
            Self::Downstream => ("downstream", "downstream"),
            Self::Modelled => ("modelled", "modelled"),
            Self::Satellite => ("satellite", "satellite"),
            Self::FarReason => (
                "{distance} from your spot, beyond your {limit} limit",
                "{distance} from your spot, beyond your {limit} limit",
            ),
            Self::StaleReason => (
                "{age} old, beyond your {limit} limit",
                "{age} old, beyond your {limit} limit",
            ),
            Self::PercentileMuchBelow => ("much below normal", "much below normal"),
            Self::PercentileBelow => ("below normal", "below normal"),
            Self::PercentileNormal => ("normal", "normal"),
            Self::PercentileAbove => ("above normal", "above normal"),
            Self::PercentileMuchAbove => ("much above normal", "much above normal"),
            Self::PercentileText => (
                "{band}, {ordinal} percentile for {date}",
                "{band}, {ordinal} percentile for {date}",
            ),
            Self::WaterRiver => ("River or stream", "River or stream"),
            Self::WaterLake => ("Lake or reservoir", "Lake or reservoir"),
            Self::WaterTidal => ("Tidal river or estuary", "Tidal river or estuary"),
            Self::WaterBay => ("Bay or inshore", "Bay or inshore"),
            Self::WaterSurf => ("Surf", "Surf"),
            Self::WaterNearshore => ("Nearshore", "Nearshore"),
            Self::WaterOffshore => ("Offshore", "Offshore"),
            Self::WaterGreatLakes => ("Great Lakes", "Great Lakes"),
            Self::WaterPond => ("Pond", "Pond"),
            Self::WaterIce => ("Ice fishing", "Ice fishing"),
            Self::Water => ("Water", "Water"),
            Self::Missing => ("—", "—"),
        }
    }
}

/// The table, bound to a voice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Strings {
    pub voice: Voice,
}

impl Strings {
    pub const fn new(voice: Voice) -> Self {
        Self { voice }
    }

    /// A playful text in the voice.
    pub const fn get(self, key: Key) -> &'static str {
        let (plain, luna) = key.variants();
        match self.voice {
            Voice::Plain => plain,
            Voice::Luna => luna,
        }
    }

    /// A playful text with its `{name}` placeholders filled.
    pub fn fmt(self, key: Key, args: &[(&str, &str)]) -> String {
        fill(self.get(key), args)
    }

    /// A safety text: the same in every voice.
    #[allow(clippy::unused_self)]
    pub const fn safety(self, key: SafetyKey) -> &'static str {
        key.text()
    }

    /// The card title.
    pub const fn card_title(self, id: CardId) -> &'static str {
        let luna = matches!(self.voice, Voice::Luna);
        match id {
            CardId::Now => "Now",
            CardId::Tide => "Tide",
            CardId::Current => "Current",
            CardId::Flow => "River flow",
            CardId::WaterTemp => "Water temperature",
            CardId::Clarity => "Water clarity",
            CardId::Waves => "Waves",
            CardId::Wind => "Wind",
            CardId::Pressure => "Pressure",
            CardId::SunMoon => "Sun and moon",
            CardId::Solunar => "Solunar",
            CardId::Windows if luna => "Snack times",
            CardId::Windows => "Fishing windows",
            CardId::GoNoGo if luna => "Luna's verdict",
            CardId::GoNoGo => "Go or no go",
            CardId::StormAhead => "Storm ahead",
            CardId::MarineForecast => "Marine forecast",
            CardId::Stocking => "Stocking",
            CardId::Species => "Species",
            CardId::Alerts => "Alerts",
            CardId::Sources => "Sources",
        }
    }

    /// "River or stream", "Tidal river or estuary", ...
    pub const fn water_type(self, water_type: WaterType) -> &'static str {
        self.get(match water_type {
            WaterType::RiverStream => Key::WaterRiver,
            WaterType::LakeReservoir => Key::WaterLake,
            WaterType::TidalRiverEstuary => Key::WaterTidal,
            WaterType::BayInshore => Key::WaterBay,
            WaterType::Surf => Key::WaterSurf,
            WaterType::Nearshore => Key::WaterNearshore,
            WaterType::Offshore => Key::WaterOffshore,
            WaterType::GreatLakes => Key::WaterGreatLakes,
            WaterType::Pond => Key::WaterPond,
            WaterType::Ice => Key::WaterIce,
        })
    }
}

/// Fills `{name}` placeholders; unknown placeholders stay as written.
fn fill(template: &str, args: &[(&str, &str)]) -> String {
    let mut out = template.to_owned();
    for (name, value) in args {
        out = out.replace(&format!("{{{name}}}"), value);
    }
    out
}

/// A source's name as displayed.
pub fn source_name(id: &SourceId) -> String {
    match id.as_str() {
        "usgs" => "USGS".to_owned(),
        "coops" => "NOAA CO-OPS".to_owned(),
        "coops-harmonics" => "NOAA CO-OPS harmonics".to_owned(),
        "ndbc" => "NOAA NDBC".to_owned(),
        "nwps" => "NOAA NWPS".to_owned(),
        "open-meteo-marine" => "Open-Meteo Marine".to_owned(),
        "nws" => "National Weather Service".to_owned(),
        "usgs-3dhp" => "USGS 3DHP".to_owned(),
        other => other.to_owned(),
    }
}

/// English month abbreviation, 1-based; no locale lookups.
pub const fn month_abbrev(month: u8) -> &'static str {
    match month {
        1 => "Jan",
        2 => "Feb",
        3 => "Mar",
        4 => "Apr",
        5 => "May",
        6 => "Jun",
        7 => "Jul",
        8 => "Aug",
        9 => "Sep",
        10 => "Oct",
        11 => "Nov",
        12 => "Dec",
        _ => "?",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn luna_variants_exist_only_for_playful_text() {
        let luna = Strings::new(Voice::Luna);
        let plain = Strings::new(Voice::Plain);
        let args = [("station", "Hudson R at Battery"), ("distance", "0.4 mi")];
        assert_eq!(
            luna.fmt(Key::StationFooter, &args),
            "Luna's watching Hudson R at Battery, 0.4 mi"
        );
        assert_eq!(
            plain.fmt(Key::StationFooter, &args),
            "Hudson R at Battery, 0.4 mi"
        );
        assert_eq!(luna.get(Key::GoNoGoGood), "Luna approves.");
        assert_eq!(luna.card_title(CardId::Windows), "Snack times");
        assert_eq!(plain.card_title(CardId::Windows), "Fishing windows");
        for key in [
            SafetyKey::NotForNavigation,
            SafetyKey::Modelled,
            SafetyKey::Provisional,
            SafetyKey::Approximate,
            SafetyKey::TraditionalCue,
            SafetyKey::TextOnly,
            SafetyKey::RegulationsLink,
        ] {
            assert_eq!(luna.safety(key), plain.safety(key));
            assert!(!key.text().contains("Luna"));
        }
    }

    #[test]
    fn every_key_is_listed_and_placeholders_pair_up() {
        let mut seen = std::collections::BTreeSet::new();
        for key in Key::ALL {
            assert!(seen.insert(key), "{key:?} listed twice");
            let (plain, luna) = key.variants();
            let names = |s: &str| {
                let mut v: Vec<String> = s
                    .split('{')
                    .skip(1)
                    .filter_map(|p| p.split('}').next().map(str::to_owned))
                    .collect();
                v.sort_unstable();
                v
            };
            assert_eq!(names(plain), names(luna), "{key:?}");
        }
    }
}
