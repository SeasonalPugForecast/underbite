//! Seasons: biological, typical and legal (as links), for a species and a region.
//!
//! Three separate values, all data for the view and none of them words:
//!
//! - [`stage`]: where the water temperature puts a species now, with the numbers kept so the view
//!   can say "water 58 °F, spawn usually 60-65 °F". The stage comes from the profile's sourced
//!   ranges and the temperature, never from a month name, so the southern hemisphere and the
//!   tropics need no special case. A missing temperature is [`Stage::Unknown`], never "too cold".
//! - [`typical_strip`]: one entry per calendar month, from the profile and whatever monthly water
//!   temperature normals and observation counts the caller has; each month lists the inputs it used.
//! - [`legal`]: the official regulation links of a region with the date each was checked, the
//!   user's own notes and the disclaimer key. Seasons, limits and sizes are never held as data.

use schemars::JsonSchema;
use serde::Serialize;

use super::profile::{Range, SourceRef, Sourced, SpeciesProfile, WaterKind};
use crate::regulations::{self, RegKind, RegLink, Water};
use crate::trip::history::Band;
use crate::view::TrendDirection;
use crate::view::strings::SafetyKey;

/// The default water temperature above which cold-water fish are stressed, °C.
pub const DEFAULT_TROUT_STRESS_C: f32 = 20.0;

/// Genera the trout-stress threshold applies to (trout, salmon, char).
const COLD_WATER_GENERA: &[&str] = &["salmo", "oncorhynchus", "salvelinus"];

/// Where the water temperature puts a species in its year.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    /// No temperature, or the profile has no range to compare it with.
    Unknown,
    /// Below the preferred range (or, with no preferred range, below spawning and not warming).
    TooCold,
    /// Warming towards the spawning range.
    PreSpawn,
    /// Inside the spawning range.
    Spawning,
    /// In the preferred range, with no spawning signal.
    Active,
    /// Above the spawning (or preferred) range and below the heat limit.
    PostSpawnOrSummer,
    /// Above the heat limit.
    HeatStress,
}

/// Why a heat limit applies.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HeatBasis {
    /// The configured trout-stress threshold, for trout, salmon and char.
    TroutStress,
    /// The top of the temperatures the profile says the fish tolerates.
    Tolerated(SourceRef),
}

/// A water temperature above which the fish is stressed.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct HeatLimit {
    pub celsius: f32,
    pub basis: HeatBasis,
}

/// The stage and the numbers behind it.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct StageReport {
    pub stage: Stage,
    pub water_temp_c: Option<f32>,
    pub trend: TrendDirection,
    /// The spawning range with its source, when the profile has one.
    pub spawn_temp_c: Option<Sourced<Range>>,
    pub preferred_temp_c: Option<Sourced<Range>>,
    /// The limit that was applied, when there is one.
    pub heat_limit: Option<HeatLimit>,
}

fn is_cold_water(profile: &SpeciesProfile) -> bool {
    let id = profile.id.as_str();
    COLD_WATER_GENERA
        .iter()
        .any(|g| id.strip_prefix(g).is_some_and(|r| r.starts_with('-')))
}

fn heat_limit(profile: &SpeciesProfile, trout_stress_c: f32) -> Option<HeatLimit> {
    let tolerated = profile.tolerated_temp_c.as_ref().map(|t| HeatLimit {
        celsius: t.value.high,
        basis: HeatBasis::Tolerated(t.source.clone()),
    });
    let trout = (is_cold_water(profile) && trout_stress_c.is_finite()).then_some(HeatLimit {
        celsius: trout_stress_c,
        basis: HeatBasis::TroutStress,
    });
    match (trout, tolerated) {
        (Some(a), Some(b)) => Some(if a.celsius <= b.celsius { a } else { b }),
        (a, b) => a.or(b),
    }
}

/// Where `water_temp_c` puts the species. `trout_stress_c` is the configured trout-stress
/// threshold (see [`DEFAULT_TROUT_STRESS_C`]); it applies to trout, salmon and char, and any
/// species also has the top of its tolerated range as a limit when the profile gives one.
pub fn stage(
    profile: &SpeciesProfile,
    water_temp_c: Option<f32>,
    trend: TrendDirection,
    trout_stress_c: f32,
) -> StageReport {
    let limit = heat_limit(profile, trout_stress_c);
    let spawn = profile.spawn_temp_c.as_ref().map(|s| s.value);
    let pref = profile.preferred_temp_c.as_ref().map(|s| s.value);
    let stage = match water_temp_c.filter(|t| t.is_finite()) {
        None => Stage::Unknown,
        Some(t) if limit.as_ref().is_some_and(|l| t > l.celsius) => Stage::HeatStress,
        Some(t) => classify(t, spawn, pref, trend),
    };
    StageReport {
        stage,
        water_temp_c,
        trend,
        spawn_temp_c: profile.spawn_temp_c.clone(),
        preferred_temp_c: profile.preferred_temp_c.clone(),
        heat_limit: limit,
    }
}

fn classify(t: f32, spawn: Option<Range>, pref: Option<Range>, trend: TrendDirection) -> Stage {
    let rising = trend == TrendDirection::Rising;
    if let Some(s) = spawn {
        if t >= s.low && t <= s.high {
            return Stage::Spawning;
        }
        if t > s.high {
            return Stage::PostSpawnOrSummer;
        }
        // Below the spawning range.
        return match pref {
            Some(p) if t < p.low => Stage::TooCold,
            Some(_) if !rising => Stage::Active,
            None if !rising => Stage::TooCold,
            _ => Stage::PreSpawn,
        };
    }
    match pref {
        Some(p) if t < p.low => Stage::TooCold,
        Some(p) if t > p.high => Stage::PostSpawnOrSummer,
        Some(_) => Stage::Active,
        None => Stage::Unknown,
    }
}

/// A month's typical water temperature: a band over past years and where it came from.
#[derive(Clone, Debug, PartialEq)]
pub struct MonthNormal {
    /// 1 to 12.
    pub month: u8,
    pub band: Band,
}

/// How many observations of the species fall in a month (from the log or a public record).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MonthObservations {
    /// 1 to 12.
    pub month: u8,
    pub count: u32,
}

/// What a month of the strip drew on.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MonthInput {
    /// The profile's sourced ranges (the stage is derived from them).
    Profile,
    /// Past water temperatures, with the years behind them.
    WaterTemp,
    /// Observation counts.
    Observations,
}

/// One calendar month.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct TypicalMonth {
    /// 1 to 12.
    pub month: u8,
    /// The stage at the month's median water temperature; `Unknown` with no temperature normal.
    pub stage: Stage,
    /// The month's typical water temperature, with its basis (source and years).
    pub water_temp: Option<Band>,
    pub observations: Option<u32>,
    pub inputs: Vec<MonthInput>,
}

/// The twelve months.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct TypicalStrip {
    pub months: Vec<TypicalMonth>,
}

/// The month `d` months from `m` (1 to 12), the year wrapping.
fn wrap_month(m: u8, d: i8) -> u8 {
    let v = (i16::from(m) - 1 + i16::from(d)).rem_euclid(12);
    u8::try_from(v).map_or(1, |v| v + 1)
}

/// A strip of twelve months, January first, from the profile and the optional inputs. A month's
/// trend is taken from the months either side of it (the year wraps), so no hemisphere is assumed.
/// Entries with a month outside 1 to 12 are ignored; the first entry for a month wins.
pub fn typical_strip(
    profile: &SpeciesProfile,
    normals: &[MonthNormal],
    observations: &[MonthObservations],
    trout_stress_c: f32,
) -> TypicalStrip {
    let temp = |m: u8| normals.iter().find(|n| n.month == m).map(|n| &n.band);
    let months = (1u8..=12)
        .map(|month| {
            let band = temp(month).cloned();
            let trend = match (temp(wrap_month(month, -1)), temp(wrap_month(month, 1))) {
                (Some(p), Some(n)) if n.median - p.median > 0.25 => TrendDirection::Rising,
                (Some(p), Some(n)) if p.median - n.median > 0.25 => TrendDirection::Falling,
                (Some(_), Some(_)) => TrendDirection::Steady,
                _ => TrendDirection::Unknown,
            };
            let st = stage(
                profile,
                band.as_ref().map(|b| b.median),
                trend,
                trout_stress_c,
            )
            .stage;
            let obs = observations
                .iter()
                .find(|o| o.month == month)
                .map(|o| o.count);
            let mut inputs = Vec::new();
            if st != Stage::Unknown {
                inputs.push(MonthInput::Profile);
            }
            if band.is_some() {
                inputs.push(MonthInput::WaterTemp);
            }
            if obs.is_some() {
                inputs.push(MonthInput::Observations);
            }
            TypicalMonth {
                month,
                stage: st,
                water_temp: band,
                observations: obs,
                inputs,
            }
        })
        .collect();
    TypicalStrip { months }
}

/// What a regulation link leads to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LegalKind {
    Freshwater,
    Saltwater,
    Licence,
    Federal,
}

/// An official page, with the date it was last checked.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, JsonSchema)]
pub struct LegalLink {
    pub kind: LegalKind,
    pub label: String,
    pub url: String,
    /// `YYYY-MM-DD`.
    pub checked: String,
    pub verified: bool,
}

impl From<RegLink> for LegalLink {
    fn from(l: RegLink) -> Self {
        Self {
            kind: match l.kind {
                RegKind::Freshwater => LegalKind::Freshwater,
                RegKind::Saltwater => LegalKind::Saltwater,
                RegKind::Licence => LegalKind::Licence,
                RegKind::Federal => LegalKind::Federal,
            },
            label: l.label,
            url: l.url,
            checked: l.checked,
            verified: l.verified,
        }
    }
}

/// The legal side of a season: links, never dates or limits.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct LegalSeason {
    pub region: String,
    pub links: Vec<LegalLink>,
    /// The oldest check date among the links; `None` when there are no links.
    pub oldest_checked: Option<String>,
    /// The user's own notes, as typed; `None` when blank.
    pub notes: Option<String>,
    /// The text the view must show beside the links.
    #[serde(skip)]
    #[schemars(skip)]
    pub disclaimer: SafetyKey,
}

/// The water to look links up for; `None` for a species found in either.
pub fn water_of(profile: &SpeciesProfile) -> Option<Water> {
    match profile.water.as_ref().map(|w| w.value) {
        Some(WaterKind::Fresh) => Some(Water::Fresh),
        Some(WaterKind::Salt) => Some(Water::Salt),
        Some(WaterKind::Both) | None => None,
    }
}

/// The official pages for `region` (an ISO 3166-2 code such as `US-NY`), in `lang` where the
/// agency has them, with the user's `notes`. `water` of `None` gives every page of the region.
pub fn legal(region: &str, water: Option<Water>, lang: &str, notes: &str) -> LegalSeason {
    let links: Vec<LegalLink> = match water {
        Some(w) => regulations::links_in(region, w, lang),
        None => regulations::all_links_in(region, lang),
    }
    .into_iter()
    .map(LegalLink::from)
    .collect();
    let oldest_checked = links.iter().map(|l| l.checked.clone()).min();
    let notes = notes.trim();
    LegalSeason {
        region: region.trim().to_owned(),
        links,
        oldest_checked,
        notes: (!notes.is_empty()).then(|| notes.to_owned()),
        disclaimer: SafetyKey::RegulationsDisclaimer,
    }
}

#[cfg(test)]
mod tests {
    use super::super::catalogue::Catalogue;
    use super::super::profile::Licence;
    use super::*;
    use crate::provenance::SourceId;
    use crate::trip::SpeciesId;
    use crate::trip::history::{HistoryBasis, HistoryKind};
    use whether_core::history::YearRange;

    fn src() -> SourceRef {
        SourceRef {
            name: "Test".to_owned(),
            url: None,
            licence: Licence::Published,
            reference: None,
            citation: None,
        }
    }

    fn sourced(low: f32, high: f32) -> Sourced<Range> {
        Sourced {
            value: Range { low, high },
            source: src(),
        }
    }

    fn profile(id: &str, spawn: Option<(f32, f32)>, pref: Option<(f32, f32)>) -> SpeciesProfile {
        let mut p = SpeciesProfile::new(
            SpeciesId::new(id).unwrap(),
            Sourced {
                value: "Test fish".to_owned(),
                source: src(),
            },
        );
        p.spawn_temp_c = spawn.map(|(l, h)| sourced(l, h));
        p.preferred_temp_c = pref.map(|(l, h)| sourced(l, h));
        p
    }

    fn st(p: &SpeciesProfile, t: Option<f32>, tr: TrendDirection) -> Stage {
        stage(p, t, tr, DEFAULT_TROUT_STRESS_C).stage
    }

    #[test]
    fn stages_follow_the_ranges() {
        use Stage::*;
        use TrendDirection::*;
        let p = profile("morone-saxatilis", Some((15.0, 18.0)), Some((12.0, 21.0)));
        assert_eq!(st(&p, None, Rising), Stage::Unknown);
        assert_eq!(st(&p, Some(8.0), Rising), TooCold);
        assert_eq!(st(&p, Some(13.0), Rising), PreSpawn);
        assert_eq!(st(&p, Some(13.0), Falling), Active);
        assert_eq!(st(&p, Some(16.0), Steady), Spawning);
        assert_eq!(st(&p, Some(20.0), Steady), PostSpawnOrSummer);
    }

    #[test]
    fn a_missing_temperature_is_unknown_not_cold() {
        let p = profile("morone-saxatilis", Some((15.0, 18.0)), None);
        let r = stage(&p, None, TrendDirection::Unknown, 20.0);
        assert_eq!(r.stage, Stage::Unknown);
        let r = stage(&p, Some(f32::NAN), TrendDirection::Unknown, 20.0);
        assert_eq!(r.stage, Stage::Unknown);
    }

    #[test]
    fn no_ranges_is_unknown() {
        let p = profile("morone-saxatilis", None, None);
        assert_eq!(st(&p, Some(15.0), TrendDirection::Steady), Stage::Unknown);
    }

    #[test]
    fn trout_stress_uses_the_configured_threshold() {
        let p = profile("oncorhynchus-mykiss", Some((6.0, 12.0)), Some((10.0, 21.0)));
        assert_eq!(
            st(&p, Some(19.0), TrendDirection::Steady),
            Stage::PostSpawnOrSummer
        );
        let r = stage(&p, Some(21.0), TrendDirection::Steady, 20.0);
        assert_eq!(r.stage, Stage::HeatStress);
        assert_eq!(r.heat_limit.as_ref().map(|l| l.celsius), Some(20.0));
        assert_eq!(r.heat_limit.map(|l| l.basis), Some(HeatBasis::TroutStress));
        let r = stage(&p, Some(19.0), TrendDirection::Steady, 18.0);
        assert_eq!(r.stage, Stage::HeatStress);
    }

    #[test]
    fn other_species_have_no_trout_limit_but_a_tolerated_one() {
        let mut p = profile("micropterus-salmoides", None, Some((20.0, 27.0)));
        assert_eq!(
            st(&p, Some(30.0), TrendDirection::Steady),
            Stage::PostSpawnOrSummer
        );
        p.tolerated_temp_c = Some(sourced(4.0, 32.0));
        assert_eq!(
            st(&p, Some(33.0), TrendDirection::Steady),
            Stage::HeatStress
        );
    }

    #[test]
    fn the_lower_heat_limit_wins() {
        let mut p = profile("salmo-trutta", None, Some((8.0, 17.0)));
        p.tolerated_temp_c = Some(sourced(0.0, 25.0));
        let r = stage(&p, Some(22.0), TrendDirection::Steady, 20.0);
        assert_eq!(r.heat_limit.map(|l| l.basis), Some(HeatBasis::TroutStress));
    }

    #[test]
    fn the_report_keeps_the_numbers_and_sources() {
        let p = profile("morone-saxatilis", Some((15.0, 18.0)), Some((12.0, 21.0)));
        let r = stage(&p, Some(14.5), TrendDirection::Rising, 20.0);
        assert_eq!(r.water_temp_c, Some(14.5));
        assert_eq!(r.spawn_temp_c.as_ref().map(|s| s.value.low), Some(15.0));
        assert_eq!(
            r.spawn_temp_c.map(|s| s.source.name),
            Some("Test".to_owned())
        );
    }

    fn normal(month: u8, median: f32) -> MonthNormal {
        MonthNormal {
            month,
            band: Band {
                p10: median - 1.0,
                median,
                p90: median + 1.0,
                basis: HistoryBasis {
                    source: SourceId::USGS,
                    kind: HistoryKind::Station,
                    years: Some(YearRange {
                        first: 2010,
                        last: 2024,
                    }),
                    years_sampled: 15,
                    percentiles: None,
                },
            },
        }
    }

    const NORTH: [f32; 12] = [
        3.0, 3.0, 6.0, 11.0, 15.0, 20.0, 23.0, 23.0, 20.0, 15.0, 9.0, 5.0,
    ];

    fn months(temps: &[f32; 12]) -> Vec<MonthNormal> {
        (1u8..).zip(temps).map(|(m, t)| normal(m, *t)).collect()
    }

    #[test]
    fn the_strip_has_twelve_months_and_says_what_it_used() {
        let p = profile("morone-saxatilis", Some((15.0, 18.0)), Some((12.0, 21.0)));
        let empty = typical_strip(&p, &[], &[], 20.0);
        assert_eq!(empty.months.len(), 12);
        assert!(
            empty
                .months
                .iter()
                .all(|m| m.stage == Stage::Unknown && m.inputs.is_empty())
        );

        let obs = [MonthObservations { month: 5, count: 7 }];
        let s = typical_strip(&p, &months(&NORTH), &obs, 20.0);
        let may = &s.months[4];
        assert_eq!(may.stage, Stage::Spawning);
        assert_eq!(may.observations, Some(7));
        assert_eq!(
            may.inputs,
            vec![
                MonthInput::Profile,
                MonthInput::WaterTemp,
                MonthInput::Observations
            ]
        );
        assert_eq!(s.months[0].stage, Stage::TooCold);
        assert_eq!(s.months[6].stage, Stage::PostSpawnOrSummer);
        assert_eq!(
            may.water_temp.as_ref().map(|b| b.basis.years_sampled),
            Some(15)
        );
    }

    #[test]
    fn a_southern_year_needs_no_special_case() {
        let p = profile("morone-saxatilis", Some((15.0, 18.0)), Some((12.0, 21.0)));
        let mut south = NORTH;
        south.rotate_left(6);
        let s = typical_strip(&p, &months(&south), &[], 20.0);
        assert_eq!(s.months[0].stage, Stage::PostSpawnOrSummer);
        assert_eq!(s.months[6].stage, Stage::TooCold);
    }

    #[test]
    fn a_gap_makes_the_trend_unknown_but_not_the_stage() {
        let p = profile("morone-saxatilis", None, Some((12.0, 21.0)));
        let s = typical_strip(&p, &[normal(6, 20.0)], &[], 20.0);
        assert_eq!(s.months[5].stage, Stage::Active);
        assert_eq!(s.months[4].stage, Stage::Unknown);
    }

    #[test]
    fn legal_is_links_notes_and_the_disclaimer_only() {
        let l = legal("US-NY", Some(Water::Salt), "en", "  Check the slot limit  ");
        assert_ne!(l.links.len(), 0);
        assert!(l.links.iter().all(|k| k.url.starts_with("https://")));
        assert!(l.links.iter().all(|k| k.checked.len() == 10));
        assert_eq!(l.notes.as_deref(), Some("Check the slot limit"));
        assert_eq!(l.disclaimer, SafetyKey::RegulationsDisclaimer);
        assert_eq!(
            l.oldest_checked,
            l.links.iter().map(|k| k.checked.clone()).min()
        );
        let all = legal("US-NY", None, "en", "");
        assert!(all.links.len() >= l.links.len());
        assert_eq!(all.notes, None);
        let none = legal("XX-YY", None, "en", "");
        assert!(none.links.is_empty() && none.oldest_checked.is_none());
    }

    #[test]
    fn water_follows_the_profile() {
        let mut p = profile("morone-saxatilis", None, None);
        assert_eq!(water_of(&p), None);
        p.water = Some(Sourced {
            value: WaterKind::Salt,
            source: src(),
        });
        assert_eq!(water_of(&p), Some(Water::Salt));
    }

    #[test]
    fn a_bundled_species_gives_snapshots() {
        let c = Catalogue::bundled().unwrap();
        let id = SpeciesId::new("salmo-trutta").unwrap();
        let p = c.get(&id).unwrap();
        let temps = [
            2.0, 3.0, 6.0, 10.0, 14.0, 19.0, 22.0, 22.0, 18.0, 12.0, 7.0, 3.0,
        ];
        let bass = c
            .get(&SpeciesId::new("micropterus-salmoides").unwrap())
            .unwrap();
        let s = typical_strip(bass, &months(&temps), &[], DEFAULT_TROUT_STRESS_C);
        insta::assert_json_snapshot!("largemouth_bass_strip", s);
        let r = stage(
            p,
            Some(21.0),
            TrendDirection::Rising,
            DEFAULT_TROUT_STRESS_C,
        );
        insta::assert_json_snapshot!("brown_trout_stage_21c", r);
    }
}
