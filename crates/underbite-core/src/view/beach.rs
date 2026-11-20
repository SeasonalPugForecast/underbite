//! The beach forecast of a shore zone, as the map shows it: the rip current risk category with its
//! colour role, the forecast's own words for surf, water temperature, UV, weather and winds, who
//! issued it and how old it is, and the safety line.
//!
//! The words are safety text: plain in every voice, and none says that water or a beach is safe.
//! A zone the office has issued nothing for says so ("No forecast issued"), never "low". The
//! categories are the forecasters' own and stand for themselves: the colour roles name a category,
//! not a verdict.
//!
//! Every field but the water temperature is shown as issued. A water temperature the office wrote
//! in whole degrees Fahrenheit ("in the lower 50s") is shown in the user's unit when that is
//! Celsius, as an approximation ("about 11 °C"); a phrase this module cannot read is shown as
//! issued.

use crate::time::UnixTime;
use crate::units::TempUnit;
use crate::view::format::Formatter;
use crate::view::strings::{Key, SafetyKey};

/// A rip current risk category of the National Weather Service.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RipCategory {
    Low,
    Moderate,
    High,
}

impl RipCategory {
    /// Every category, in order of risk.
    pub const ALL: [Self; 3] = [Self::Low, Self::Moderate, Self::High];

    /// The colour role the map draws the category in.
    pub const fn role(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Moderate => "moderate",
            Self::High => "high",
        }
    }

    const fn key(self) -> SafetyKey {
        match self {
            Self::Low => SafetyKey::BeachRiskLow,
            Self::Moderate => SafetyKey::BeachRiskModerate,
            Self::High => SafetyKey::BeachRiskHigh,
        }
    }
}

/// The colour role of a zone with no forecast: drawn neutral and outlined.
pub const ROLE_NONE: &str = "none";

/// What the forecast office issued for one zone. Every text is as the office wrote it; none is a
/// value in a unit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BeachFacts<'a> {
    pub name: &'a str,
    /// The issuing office's id, "OKX".
    pub office: Option<&'a str>,
    /// The category; none when nothing was issued or the word is no category.
    pub rip: Option<RipCategory>,
    /// The risk as issued, whatever the word.
    pub rip_text: Option<&'a str>,
    pub surf: Option<&'a str>,
    pub water_temperature: Option<&'a str>,
    pub uv: Option<&'a str>,
    pub weather: Option<&'a str>,
    pub winds: Option<&'a str>,
    /// "Rest of today".
    pub period: Option<&'a str>,
    /// "11/20/2026".
    pub product_date: Option<&'a str>,
    /// "409 PM EST".
    pub product_time: Option<&'a str>,
    /// When the forecast was issued.
    pub issued: Option<UnixTime>,
}

/// One line of the card.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BeachRow {
    /// "Surf".
    pub label: String,
    /// "Around 2 feet.", "about 11 °C".
    pub value: String,
}

/// The card of a zone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BeachCard {
    /// The beach zone's name as the office writes it.
    pub title: String,
    /// The risk's colour role: `low`, `moderate`, `high` or [`ROLE_NONE`].
    pub role: String,
    /// The risk in words: "Moderate", "No forecast issued".
    pub risk: String,
    /// The rows of what the office issued: the risk first, then the fields it gave.
    pub rows: Vec<BeachRow>,
    /// The period the forecast is for, as issued: "Rest of today".
    pub period: Option<String>,
    /// Who issued it and when: "Issued by NWS BOX, 409 PM EST 11/20/2026".
    pub issued: String,
    /// How old it is: "3 h ago".
    pub age: Option<String>,
    /// The line every card carries.
    pub safety: String,
    /// The zone read aloud.
    pub accessibility: String,
}

/// One entry of the map's legend.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BeachLegendEntry {
    /// The colour role.
    pub role: String,
    /// "Low", "No forecast issued".
    pub label: String,
}

/// The legend: the three categories, then no forecast.
pub fn legend(f: &Formatter) -> Vec<BeachLegendEntry> {
    let s = f.strings;
    RipCategory::ALL
        .iter()
        .map(|c| BeachLegendEntry {
            role: c.role().to_owned(),
            label: s.safety(c.key()).to_owned(),
        })
        .chain(std::iter::once(BeachLegendEntry {
            role: ROLE_NONE.to_owned(),
            label: s.safety(SafetyKey::BeachNoForecast).to_owned(),
        }))
        .collect()
}

/// The risk in words: the category, the forecaster's own word when it is no category, or "No
/// forecast issued".
pub fn risk_words(f: &Formatter, rip: Option<RipCategory>, rip_text: Option<&str>) -> String {
    let s = f.strings;
    match (rip, rip_text) {
        (Some(c), _) => s.safety(c.key()).to_owned(),
        (None, Some(text)) => text.to_owned(),
        (None, None) => s.safety(SafetyKey::BeachNoForecast).to_owned(),
    }
}

/// The colour role of a zone: its category, or [`ROLE_NONE`].
pub fn role(rip: Option<RipCategory>) -> &'static str {
    rip.map_or(ROLE_NONE, RipCategory::role)
}

/// A water temperature phrase in whole degrees Fahrenheit as degrees Celsius: "in the lower 50s"
/// is 52 °F, "in the mid 60s" 65, "in the upper 70s" 78, "around 68" 68 and "in the 50s" 55.
/// None for a phrase that is not one of these (a range, a word this does not know).
pub fn phrase_to_celsius(text: &str) -> Option<f32> {
    let lower = text.trim().trim_end_matches('.').to_ascii_lowercase();
    let mut rest = lower.as_str();
    for prefix in ["in the ", "around ", "near ", "about ", "the "] {
        if let Some(r) = rest.strip_prefix(prefix) {
            rest = r.trim_start();
            break;
        }
    }
    let (shift, rest) = ["lower ", "low ", "mid ", "upper ", "high "]
        .iter()
        .find_map(|w| {
            rest.strip_prefix(w).map(|r| {
                let shift = match *w {
                    "lower " | "low " => 2,
                    "mid " => 5,
                    _ => 8,
                };
                (Some(shift), r.trim_start())
            })
        })
        .unwrap_or((None, rest));
    let (digits, decade) = match rest.strip_suffix('s') {
        Some(d) => (d, true),
        None => (rest, false),
    };
    if digits.len() != 2 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n: i32 = digits.parse().ok()?;
    let fahrenheit = match (shift, decade) {
        (Some(s), true) => n + s,
        (None, true) => n + 5,
        (None, false) => n,
        // "lower 50", "mid 60": no such phrase.
        (Some(_), false) => return None,
    };
    if !(30..=100).contains(&fahrenheit) {
        return None;
    }
    #[allow(clippy::cast_precision_loss)] // INVARIANT: a two-digit temperature
    Some(TempUnit::F.to_canonical(fahrenheit as f32))
}

/// The water temperature in the user's unit when that unit is Celsius and the phrase can be read;
/// otherwise as issued.
pub fn water_temperature(f: &Formatter, issued: &str) -> String {
    if f.units.temperature == TempUnit::C
        && let Some(c) = phrase_to_celsius(issued)
    {
        // Whole degrees: the phrase is no more exact than that.
        let symbol = f.strings.temp_unit(TempUnit::C);
        let q = f.quantity(TempUnit::C.from_canonical(c).round(), 0, symbol);
        let value = format!("{} {}", q.text, q.unit);
        return f
            .strings
            .safety_fmt(SafetyKey::BeachWaterAbout, &[("value", &value)]);
    }
    issued.to_owned()
}

/// The card of a zone at `now`.
pub fn card(f: &Formatter, facts: &BeachFacts<'_>, now: UnixTime) -> BeachCard {
    let s = f.strings;
    let risk = risk_words(f, facts.rip, facts.rip_text);
    let mut rows = vec![BeachRow {
        label: s.get(Key::BeachRowRip).to_owned(),
        value: risk.clone(),
    }];
    let water = facts.water_temperature.map(|t| water_temperature(f, t));
    for (key, value) in [
        (Key::BeachRowSurf, facts.surf.map(str::to_owned)),
        (Key::BeachRowWaterTemperature, water),
        (Key::BeachRowUv, facts.uv.map(str::to_owned)),
        (Key::BeachRowWeather, facts.weather.map(str::to_owned)),
        (Key::BeachRowWinds, facts.winds.map(str::to_owned)),
    ] {
        if let Some(value) = value {
            rows.push(BeachRow {
                label: s.get(key).to_owned(),
                value,
            });
        }
    }
    let when = match (facts.product_time, facts.product_date) {
        (Some(t), Some(d)) => Some(format!("{t} {d}")),
        (Some(t), None) => Some(t.to_owned()),
        _ => None,
    };
    let issued = match (facts.office, when) {
        (Some(office), Some(when)) => s.safety_fmt(
            SafetyKey::BeachIssuedByAt,
            &[("office", office), ("when", &when)],
        ),
        (Some(office), None) if facts.rip_text.is_some() || rows.len() > 1 => {
            s.safety_fmt(SafetyKey::BeachIssuedBy, &[("office", office)])
        }
        (Some(office), None) => s.safety_fmt(SafetyKey::BeachNothingFrom, &[("office", office)]),
        (None, _) => s.safety(SafetyKey::BeachNothing).to_owned(),
    };
    BeachCard {
        accessibility: s.safety_fmt(
            SafetyKey::BeachZoneRead,
            &[("name", facts.name), ("risk", &risk)],
        ),
        title: facts.name.to_owned(),
        role: role(facts.rip).to_owned(),
        risk,
        rows,
        period: facts.period.map(str::to_owned),
        issued,
        age: facts.issued.map(|t| f.age(now.0 - t.0)),
        safety: s.safety(SafetyKey::BeachSafety).to_owned(),
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

    fn formatter(temperature: TempUnit, voice: Voice) -> Formatter {
        Formatter::new(
            DisplayUnits {
                temperature,
                air_temperature: temperature,
                height: HeightUnit::Ft,
                stage: HeightUnit::Ft,
                lake_level: HeightUnit::Ft,
                waves: HeightUnit::Ft,
                discharge: DischargeUnit::Cfs,
                current_speed: SpeedUnit::Kn,
                wind_speed: SpeedUnit::Mph,
                pressure: PressureUnit::InHg,
                distance: DistanceUnit::Mi,
            },
            voice,
            TimeFormat::H12,
            "UTC",
        )
    }

    const NOW: UnixTime = UnixTime(1_795_208_940 + 3 * 3_600 + 120);

    fn boston() -> BeachFacts<'static> {
        BeachFacts {
            name: "Eastern Norfolk Area Beaches",
            office: Some("BOX"),
            rip: Some(RipCategory::Moderate),
            rip_text: Some("Moderate"),
            surf: Some("Around 2 feet."),
            water_temperature: Some("in the lower 50s"),
            uv: Some("Low"),
            weather: Some("Mostly sunny."),
            winds: Some("West 10 to 15 mph."),
            period: Some("Rest of today"),
            product_date: Some("11/20/2026"),
            product_time: Some("409 PM EST"),
            issued: Some(UnixTime(1_795_208_940)),
        }
    }

    fn all_words(f: &Formatter) -> Vec<String> {
        let mut words = Vec::new();
        for (rip, text, temp) in [
            (Some(RipCategory::Low), Some("Low"), "in the lower 50s"),
            (Some(RipCategory::High), Some("High"), "around 68"),
            (None, None, "in the upper 70s"),
        ] {
            let facts = BeachFacts {
                rip,
                rip_text: text,
                water_temperature: Some(temp),
                ..boston()
            };
            let c = card(f, &facts, NOW);
            words.extend([c.title, c.risk, c.issued, c.safety, c.accessibility]);
            words.extend(c.rows.into_iter().map(|r| r.label));
        }
        words.extend(legend(f).into_iter().map(|e| e.label));
        words
    }

    #[test]
    fn a_card_carries_what_was_issued_its_office_and_its_age() {
        let f = formatter(TempUnit::F, Voice::Plain);
        let c = card(&f, &boston(), NOW);
        assert_eq!(c.title, "Eastern Norfolk Area Beaches");
        assert_eq!(c.role, "moderate");
        assert_eq!(c.risk, "Moderate");
        let rows: Vec<_> = c.rows.iter().map(|r| (&*r.label, &*r.value)).collect();
        assert_eq!(
            rows,
            [
                ("Rip current risk", "Moderate"),
                ("Surf", "Around 2 feet."),
                ("Water temperature", "in the lower 50s"),
                ("UV", "Low"),
                ("Weather", "Mostly sunny."),
                ("Winds", "West 10 to 15 mph."),
            ]
        );
        assert_eq!(c.period.as_deref(), Some("Rest of today"));
        assert_eq!(c.issued, "Issued by NWS BOX, 409 PM EST 11/20/2026");
        assert_eq!(c.age.as_deref(), Some("3 h ago"));
        assert_eq!(
            c.safety,
            "Rip currents can occur at any risk level and are most likely near groins, jetties, reefs and piers. Swim near a lifeguard."
        );
        assert_eq!(
            c.accessibility,
            "Eastern Norfolk Area Beaches, rip current risk Moderate"
        );
    }

    #[test]
    fn water_temperature_follows_the_users_unit() {
        let metric = formatter(TempUnit::C, Voice::Plain);
        let c = card(&metric, &boston(), NOW);
        let water = c
            .rows
            .iter()
            .find(|r| r.label == "Water temperature")
            .unwrap();
        assert_eq!(water.value, "about 11 °C");
        // Fahrenheit is shown as issued, not converted and back.
        let us = formatter(TempUnit::F, Voice::Plain);
        let c = card(&us, &boston(), NOW);
        let water = c
            .rows
            .iter()
            .find(|r| r.label == "Water temperature")
            .unwrap();
        assert_eq!(water.value, "in the lower 50s");
        // A phrase this cannot read is shown as issued in either unit.
        let odd = BeachFacts {
            water_temperature: Some("in the mid 50s to lower 60s"),
            ..boston()
        };
        let c = card(&metric, &odd, NOW);
        let water = c
            .rows
            .iter()
            .find(|r| r.label == "Water temperature")
            .unwrap();
        assert_eq!(water.value, "in the mid 50s to lower 60s");
    }

    #[test]
    fn temperature_phrases_are_read_as_fahrenheit() {
        let f = |t: &str| phrase_to_celsius(t).map(|c| (c * 10.0).round() / 10.0);
        assert_eq!(f("in the lower 50s"), Some(11.1));
        assert_eq!(f("In the mid 60s."), Some(18.3));
        assert_eq!(f("in the upper 70s"), Some(25.6));
        assert_eq!(f("in the 50s"), Some(12.8));
        assert_eq!(f("around 68"), Some(20.0));
        assert_eq!(f("near 41"), Some(5.0));
        for no in [
            "",
            "warm",
            "in the lower 50s to mid 60s",
            "lower 50",
            "in the 5s",
            "in the lower 120s",
            "in the 20s",
            "68 to 70",
        ] {
            assert_eq!(f(no), None, "{no:?}");
        }
    }

    #[test]
    fn a_zone_with_nothing_issued_says_so_and_is_never_low() {
        let f = formatter(TempUnit::F, Voice::Plain);
        let facts = BeachFacts {
            name: "Southern Queens Beaches",
            office: Some("OKX"),
            issued: Some(UnixTime(1_790_596_678)),
            ..BeachFacts::default()
        };
        let c = card(&f, &facts, UnixTime(1_790_596_678 + 53 * 86_400));
        assert_eq!(c.role, ROLE_NONE);
        assert_eq!(c.risk, "No forecast issued");
        assert_eq!(c.rows.len(), 1);
        assert_eq!(c.rows[0].value, "No forecast issued");
        assert_eq!(c.issued, "NWS OKX has issued no forecast for this zone");
        assert_eq!(c.age.as_deref(), Some("53 d ago"));
        assert_eq!(c.period, None);
        // The safety line stays: an unissued zone has rip currents too.
        assert!(
            c.safety
                .starts_with("Rip currents can occur at any risk level")
        );
        let no_office = card(
            &f,
            &BeachFacts {
                office: None,
                ..facts
            },
            NOW,
        );
        assert_eq!(
            no_office.issued,
            "The National Weather Service has issued no forecast for this zone"
        );
    }

    #[test]
    fn a_word_that_is_no_category_is_shown_as_issued_and_drawn_neutral() {
        let f = formatter(TempUnit::F, Voice::Plain);
        let facts = BeachFacts {
            rip: None,
            rip_text: Some("Extreme"),
            product_time: None,
            product_date: None,
            ..boston()
        };
        let c = card(&f, &facts, NOW);
        assert_eq!(c.role, ROLE_NONE);
        assert_eq!(c.risk, "Extreme");
        assert_eq!(c.issued, "Issued by NWS BOX");
    }

    #[test]
    fn the_legend_lists_the_categories_then_no_forecast() {
        let f = formatter(TempUnit::F, Voice::Plain);
        let l: Vec<_> = legend(&f).into_iter().map(|e| (e.role, e.label)).collect();
        assert_eq!(
            l,
            [
                ("low".to_owned(), "Low".to_owned()),
                ("moderate".to_owned(), "Moderate".to_owned()),
                ("high".to_owned(), "High".to_owned()),
                ("none".to_owned(), "No forecast issued".to_owned()),
            ]
        );
    }

    #[test]
    fn the_words_never_call_a_beach_safe_and_are_the_same_in_luna_s_voice() {
        let plain = formatter(TempUnit::C, Voice::Plain);
        let luna = formatter(TempUnit::C, Voice::Luna);
        let words = all_words(&plain);
        assert_eq!(words, all_words(&luna));
        let others = [
            Strings::new(Voice::Plain)
                .layer_legend("beach-forecast")
                .unwrap(),
            Strings::new(Voice::Plain)
                .layer_credit("beach-forecast")
                .unwrap(),
        ];
        for w in words.iter().map(String::as_str).chain(others) {
            let lower = w.to_ascii_lowercase();
            let safe = lower
                .split(|c: char| !c.is_ascii_alphabetic())
                .any(|word| word == "safe" || word == "safely");
            assert!(!safe, "{w}");
            assert!(!lower.contains("luna"), "{w}");
            assert!(!lower.contains("go ahead"), "{w}");
        }
    }

    #[test]
    fn the_layer_is_worded_and_credited() {
        let s = Strings::new(Voice::Plain);
        assert_eq!(
            s.layer_title("beach-forecast"),
            Some("Beach forecast (rip currents)")
        );
        assert_eq!(
            s.layer_credit("beach-forecast"),
            Some("Beach forecasts: National Weather Service")
        );
        assert_eq!(s.layer_source("beach-forecast"), Some("NWS beach forecast"));
        assert_eq!(s.layer_licence("beach-forecast"), Some("Public domain"));
        let legend = s.layer_legend("beach-forecast").unwrap();
        assert!(legend.contains("most likely near groins, jetties, reefs and piers"));
        assert!(legend.contains("no forecast issued is outlined and not filled"));
        assert_eq!(s.preset_name("swimming"), Some("Swimming"));
        assert_eq!(
            s.source_name(&crate::provenance::SourceId::NWS_BEACH_FORECAST),
            "NWS beach forecast"
        );
        // The forecast is not a chart or a current layer: no "not for navigation".
        assert!(
            !s.layer_credit("beach-forecast")
                .unwrap()
                .contains("navigation")
        );
    }
}
