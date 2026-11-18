//! The species card: what the water temperature means now for the chosen species.
//!
//! One line per species: its stage (too cold, pre-spawn, spawning, ...) from the spot's water
//! temperature against the profile's sourced ranges, with the numbers behind it ("water 58 °F,
//! spawn usually 54–72 °F"). The card credits the water temperature's station and every profile
//! value it used, with its licence; a value the user edited is marked. With no water temperature
//! every stage is unknown, never "too cold".

use schemars::JsonSchema;
use serde::Serialize;

use crate::model::Measurement;
use crate::provenance::Provenance;
use crate::series::{Sample, Series};
use crate::species::profile::{Licence, Range, SourceRef, SpeciesProfile};
use crate::species::seasons::{DEFAULT_TROUT_STRESS_C, HeatBasis, Stage, StageReport, stage};
use crate::time::UnixTime;
use crate::view::cards::flow::{card_state, credits as station_credits, row, trend};
use crate::view::format::Formatter;
use crate::view::strings::{Key, SafetyKey, Strings};
use crate::view::{
    CardChrome, CardId, CardState, Compact, CreditVm, Notice, NoticeKind, Quantity, Section,
    TrendDirection, ViewInput,
};

/// Water temperature counts as moving beyond this change over the trend window, °C (as on the
/// water temperature card).
const TEMP_TREND_C: f32 = 0.2;

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct SpeciesCard {
    pub chrome: CardChrome,
    pub compact: Compact,
    pub full: SpeciesFull,
}

/// One species against the water now.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct SpeciesNowVm {
    /// The species id (`micropterus-dolomieu`).
    pub id: String,
    /// The name shown: "Smallmouth bass".
    pub name: String,
    pub stage: Stage,
    /// "Pre-spawn".
    pub stage_label: String,
    /// "Smallmouth bass · Pre-spawn · water 58 °F, spawn usually 54–72 °F".
    pub text: String,
    /// Safety text when the water is above the trout-stress threshold.
    pub caution: Option<String>,
    /// A range behind the stage is the user's own edit.
    pub edited: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct SpeciesFull {
    /// The water temperature the stages are read against; `None` when the spot has none.
    pub water: Option<Quantity>,
    /// In the order the species were chosen.
    pub species: Vec<SpeciesNowVm>,
    /// One per species: stage, water, the profile's ranges and the heat limit.
    pub sections: Vec<Section>,
}

/// The spot's water temperature at `now` and its trend.
pub(in crate::view) struct WaterNow<'a> {
    pub celsius: f32,
    pub trend: TrendDirection,
    pub provenance: &'a Provenance,
    pub time: UnixTime,
}

/// The sample that applies at `now`: the newest at or before it, else the first one ahead.
fn sample_at(series: &Series<f32>, now: UnixTime) -> Option<&Sample<f32>> {
    let samples = series.samples();
    let i = samples.partition_point(|s| s.time <= now);
    i.checked_sub(1)
        .and_then(|i| samples.get(i))
        .or_else(|| samples.first())
}

/// The water temperature the water temperature card shows: a thermometer first, then observed
/// sea surface temperature, then its forecast (which has no trend).
pub(in crate::view) fn water_now<'a>(
    snapshot: &'a crate::model::SpotSnapshot,
    now: UnixTime,
    f: &Formatter,
) -> Option<WaterNow<'a>> {
    let measured = snapshot
        .water
        .temperature_c
        .as_ref()
        .filter(|s| !s.is_empty());
    let sst = snapshot
        .marine
        .sea_surface_temp_c
        .as_ref()
        .filter(|s| !s.is_empty());
    let forecast = snapshot
        .marine
        .sea_surface_temp_forecast_c
        .as_ref()
        .filter(|s| !s.is_empty());
    let (series, observed) = match (measured.or(sst), forecast) {
        (Some(s), _) => (s, true),
        (None, Some(s)) => (s, false),
        (None, None) => return None,
    };
    let latest = sample_at(series, now)?;
    let direction = if observed {
        trend(f, series.samples(), TEMP_TREND_C, &|_| String::new())
            .map_or(TrendDirection::Unknown, |t| t.direction)
    } else {
        TrendDirection::Unknown
    };
    Some(WaterNow {
        celsius: latest.value,
        trend: direction,
        provenance: series.provenance(),
        time: latest.time,
    })
}

/// "Pre-spawn".
pub(in crate::view) fn stage_label(s: Strings, stage: Stage) -> &'static str {
    s.get(match stage {
        Stage::Unknown => Key::SpeciesStageUnknown,
        Stage::TooCold => Key::SpeciesStageTooCold,
        Stage::PreSpawn => Key::SpeciesStagePreSpawn,
        Stage::Spawning => Key::SpeciesStageSpawning,
        Stage::Active => Key::SpeciesStageActive,
        Stage::PostSpawnOrSummer => Key::SpeciesStagePostSpawn,
        Stage::HeatStress => Key::SpeciesStageHeatStress,
    })
}

/// "54–72 °F".
pub(in crate::view) fn temp_range(f: &Formatter, r: Range) -> String {
    let (lo, hi) = (f.temperature(Some(r.low)), f.temperature(Some(r.high)));
    f.strings.fmt(
        Key::SpeciesRange,
        &[("from", &lo.text), ("to", &hi.text), ("unit", &lo.unit)],
    )
}

/// The details of a stage: the water now and the range that matters for it.
pub(in crate::view) fn stage_details(f: &Formatter, r: &StageReport) -> String {
    let s = f.strings;
    let Some(celsius) = r.water_temp_c.filter(|c| c.is_finite()) else {
        return s.get(Key::SpeciesNoWater).to_owned();
    };
    let water = s.fmt(
        Key::SpeciesWaterNow,
        &[("value", &f.temperature(Some(celsius)).full())],
    );
    let range = match (r.stage, &r.heat_limit, &r.spawn_temp_c, &r.preferred_temp_c) {
        (Stage::HeatStress, Some(limit), _, _) => Some(s.fmt(
            Key::SpeciesStressedAbove,
            &[("limit", &f.temperature(Some(limit.celsius)).full())],
        )),
        (_, _, Some(spawn), _) => Some(s.fmt(
            Key::SpeciesSpawnUsually,
            &[("range", &temp_range(f, spawn.value))],
        )),
        (_, _, None, Some(preferred)) => Some(s.fmt(
            Key::SpeciesPrefers,
            &[("range", &temp_range(f, preferred.value))],
        )),
        _ if r.stage == Stage::Unknown => Some(s.get(Key::SpeciesNoRanges).to_owned()),
        _ => None,
    };
    match range {
        Some(range) => s.fmt(Key::SpeciesJoin, &[("first", &water), ("second", &range)]),
        None => water,
    }
}

/// The trout-stress caution, when the stage is heat stress from the user's threshold.
pub(in crate::view) fn caution(f: &Formatter, r: &StageReport) -> Option<String> {
    let l = r.heat_limit.as_ref()?;
    (r.stage == Stage::HeatStress && l.basis == HeatBasis::TroutStress).then(|| {
        f.strings.safety_fmt(
            SafetyKey::TroutStress,
            &[("threshold", &f.temperature(Some(l.celsius)).full())],
        )
    })
}

/// A licence in words.
pub(in crate::view) fn licence_label(s: Strings, licence: Licence) -> String {
    let key = match licence {
        Licence::PublicDomain => SafetyKey::LicencePublicDomain,
        Licence::CcBy4 => SafetyKey::LicenceCcBy4,
        Licence::CcByNc4 => SafetyKey::LicenceCcByNc4,
        Licence::Published => SafetyKey::LicencePublished,
        Licence::UserEdited => SafetyKey::LicenceUserEdited,
    };
    s.safety(key).to_owned()
}

/// Who typed an edited value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditedBy {
    /// The user, on this device.
    You,
    /// The person who shared the profile.
    Sender,
}

/// The credit a profile value's source is owed: its citation (or record reference) as text, its
/// link and its licence. An edited value is credited to whoever typed it, without a link.
pub(in crate::view) fn source_credit(s: Strings, src: &SourceRef, by: EditedBy) -> CreditVm {
    if src.licence == Licence::UserEdited {
        let name = s
            .get(match by {
                EditedBy::You => Key::SpeciesYourEdit,
                EditedBy::Sender => Key::SpeciesSenderEdit,
            })
            .to_owned();
        return CreditVm {
            source: name.clone(),
            text: name,
            link: None,
            licence: Some(licence_label(s, src.licence)),
        };
    }
    CreditVm {
        source: src.name.clone(),
        text: src
            .citation
            .clone()
            .or_else(|| src.reference.clone())
            .unwrap_or_else(|| src.name.clone()),
        link: src.url.clone(),
        licence: Some(licence_label(s, src.licence)),
    }
}

fn push_unique(out: &mut Vec<CreditVm>, credit: CreditVm) {
    if !out.contains(&credit) {
        out.push(credit);
    }
}

/// The profile values a stage rests on.
fn used_sources(p: &SpeciesProfile) -> Vec<&SourceRef> {
    [&p.spawn_temp_c, &p.preferred_temp_c, &p.tolerated_temp_c]
        .into_iter()
        .flatten()
        .map(|s| &s.source)
        .collect()
}

/// The card for `species` (profiles with the user's edits laid over them, in the user's order).
/// No species, no card.
#[allow(clippy::too_many_lines)]
pub fn build(input: &ViewInput, species: &[SpeciesProfile]) -> Option<SpeciesCard> {
    if species.is_empty() {
        return None;
    }
    let f = Formatter::for_input(input);
    let s = f.strings;
    let water = water_now(&input.snapshot, input.now, &f);
    let trout = input.trout_stress_c.unwrap_or(DEFAULT_TROUT_STRESS_C);

    let mut rows_vm = Vec::new();
    let mut sections = Vec::new();
    let mut credits = Vec::new();
    let mut fishbase = false;
    for p in species {
        let report = stage(
            p,
            water.as_ref().map(|w| w.celsius),
            water.as_ref().map_or(TrendDirection::Unknown, |w| w.trend),
            trout,
        );
        let label = stage_label(s, report.stage).to_owned();
        let details = stage_details(&f, &report);
        let text = s.fmt(
            Key::SpeciesLine,
            &[
                ("species", p.display_name()),
                ("stage", &label),
                ("details", &details),
            ],
        );
        let used = used_sources(p);
        let edited = used.iter().any(|src| src.licence == Licence::UserEdited);
        fishbase |= used.iter().any(|src| src.licence == Licence::CcByNc4);
        for src in &used {
            push_unique(&mut credits, source_credit(s, src, EditedBy::You));
        }

        let mut rows = vec![row(s.get(Key::SpeciesRowStage), &label)];
        if let Some(w) = &water {
            rows.push(row(
                s.get(Key::SpeciesRowWater),
                &f.temperature(Some(w.celsius)).full(),
            ));
        }
        for (key, value) in [
            (Key::SpeciesFieldSpawn, &report.spawn_temp_c),
            (Key::SpeciesFieldPreferred, &report.preferred_temp_c),
        ] {
            if let Some(v) = value {
                let mut r = row(s.get(key), &temp_range(&f, v.value));
                r.detail = Some(source_credit(s, &v.source, EditedBy::You).source);
                rows.push(r);
            }
        }
        if let Some(l) = &report.heat_limit {
            let basis = match &l.basis {
                HeatBasis::TroutStress => s.get(Key::SpeciesHeatTroutSetting).to_owned(),
                HeatBasis::Tolerated(src) => source_credit(s, src, EditedBy::You).source,
            };
            rows.push(row(
                s.get(Key::SpeciesRowHeatLimit),
                &s.fmt(
                    Key::SpeciesHeatLimitValue,
                    &[
                        ("limit", &f.temperature(Some(l.celsius)).full()),
                        ("basis", &basis),
                    ],
                ),
            ));
        }
        sections.push(Section {
            title: p.display_name().to_owned(),
            rows,
        });
        rows_vm.push(SpeciesNowVm {
            id: p.id.as_str().to_owned(),
            name: p.display_name().to_owned(),
            stage: report.stage,
            stage_label: label,
            text,
            caution: caution(&f, &report),
            edited,
        });
    }
    if let Some(w) = &water {
        for c in station_credits(input, &[w.provenance]) {
            push_unique(&mut credits, c);
        }
    }

    let headline = f.temperature(water.as_ref().map(|w| w.celsius));
    let mut lines: Vec<String> = rows_vm.iter().filter_map(|v| v.caution.clone()).collect();
    lines.dedup();
    lines.extend(rows_vm.iter().map(|v| v.text.clone()));
    lines.truncate(2);

    let mut notices: Vec<Notice> = Vec::new();
    if fishbase {
        notices.push(Notice {
            kind: NoticeKind::TextOnly,
            text: s.safety(SafetyKey::SpeciesFishBaseNotice).to_owned(),
        });
    }
    let title = s.card_title(CardId::Species).to_owned();
    let mut accessibility = format!("{title}: ");
    accessibility.push_str(
        &rows_vm
            .iter()
            .map(|v| {
                v.caution
                    .as_ref()
                    .map_or(v.text.clone(), |c| format!("{}. {c}", v.text))
            })
            .collect::<Vec<_>>()
            .join(". "),
    );
    let (footnote, state) = match &water {
        Some(w) => (
            Some(f.footnote_aged(
                &format!(
                    "{} {}",
                    s.get(Key::WaterWord),
                    f.temperature(Some(w.celsius)).full()
                ),
                w.provenance,
                input.now,
                Some(w.time).filter(|t| *t <= input.now),
            )),
            card_state(
                input,
                &f,
                Measurement::WaterTemperature,
                w.provenance,
                Some(w.time),
            ),
        ),
        None => (
            None,
            CardState::Unavailable {
                reason: crate::view::cards::flow::capitalize(s.get(Key::SpeciesNoWater)),
            },
        ),
    };
    Some(SpeciesCard {
        chrome: CardChrome {
            id: CardId::Species,
            title,
            footnote,
            credits,
            state,
            notices,
            accessibility,
        },
        compact: Compact {
            headline,
            lines,
            spark: Vec::new(),
        },
        full: SpeciesFull {
            water: water.as_ref().map(|w| f.temperature(Some(w.celsius))),
            species: rows_vm,
            sections,
        },
    })
}

#[cfg(test)]
pub(in crate::view) mod tests {
    use super::*;
    use crate::species::catalogue::Catalogue;
    use crate::species::profile::ProfileEdits;
    use crate::trip::SpeciesId;
    use crate::view::Voice;
    use crate::view::cards::flow::fixtures::{metric, us};
    use crate::view::cards::water_temp::tests::river;

    /// Bundled profiles with the edits laid over them.
    pub(in crate::view) fn profiles(ids: &[&str], edits: &ProfileEdits) -> Vec<SpeciesProfile> {
        let catalogue = Catalogue::bundled().unwrap();
        ids.iter()
            .map(|id| {
                catalogue
                    .profile(&SpeciesId::new(id).unwrap(), edits)
                    .unwrap()
            })
            .collect()
    }

    /// The species the view tests choose at the Croton: smallmouth bass and brown trout.
    pub(in crate::view) fn chosen() -> Vec<SpeciesProfile> {
        profiles(
            &["micropterus-dolomieu", "salmo-trutta"],
            &ProfileEdits::default(),
        )
    }

    fn card(input: &ViewInput, species: &[SpeciesProfile]) -> SpeciesCard {
        build(input, species).expect("a species card")
    }

    #[test]
    fn no_species_no_card() {
        assert!(build(&river(us(), Voice::Plain, 14.0, 15.0), &[]).is_none());
    }

    #[test]
    fn rising_water_below_spawning_is_pre_spawn_with_the_numbers() {
        // Smallmouth bass spawn at 12.8–22 °C (FishBase); the Croton warms from 10 to 11 °C.
        let c = card(&river(us(), Voice::Plain, 10.0, 11.0), &chosen());
        let bass = &c.full.species[0];
        assert_eq!(bass.id, "micropterus-dolomieu");
        assert_eq!(bass.stage, Stage::PreSpawn);
        assert_eq!(
            bass.text,
            "Smallmouth bass · Pre-spawn · water 52 °F, spawn usually 55–72 °F"
        );
        assert!(!bass.edited);
        assert_eq!(c.compact.headline.full(), "52 °F");
        assert_eq!(c.compact.lines[0], bass.text);
        assert_eq!(c.chrome.id, CardId::Species);
        assert_eq!(c.chrome.state, CardState::Fresh);
    }

    #[test]
    fn warm_water_stresses_trout_with_the_safety_text_first() {
        let c = card(&river(us(), Voice::Luna, 21.0, 22.0), &chosen());
        let trout = &c.full.species[1];
        assert_eq!(trout.stage, Stage::HeatStress);
        assert_eq!(
            trout.caution.as_deref(),
            Some("Consider not fishing for trout above 68 °F.")
        );
        assert!(
            trout.text.contains("stressed above 68 °F"),
            "{}",
            trout.text
        );
        assert_eq!(
            c.compact.lines[0],
            "Consider not fishing for trout above 68 °F."
        );
        assert_eq!(c.full.species[0].stage, Stage::Spawning);
    }

    #[test]
    fn every_value_used_is_credited_with_its_licence() {
        let c = card(&river(metric(), Voice::Plain, 14.0, 15.0), &chosen());
        let fishbase: Vec<&CreditVm> = c
            .chrome
            .credits
            .iter()
            .filter(|cr| cr.source == "FishBase")
            .collect();
        assert_ne!(fishbase.len(), 0);
        for cr in &fishbase {
            assert_eq!(cr.licence.as_deref(), Some("CC BY-NC 4.0"));
            assert!(cr.text.contains("Froese"), "{}", cr.text);
            assert!(cr.link.as_deref().unwrap().starts_with("https://"));
        }
        assert!(
            c.chrome
                .notices
                .iter()
                .any(|n| n.text.contains("non-commercially"))
        );
        // The water temperature's own source is credited too.
        assert!(c.chrome.credits.iter().any(|cr| cr.source != "FishBase"));
        assert!(
            c.full.sections[0]
                .rows
                .iter()
                .any(|r| r.detail.as_deref() == Some("FishBase"))
        );
    }

    #[test]
    fn an_edited_range_is_marked_and_credited_to_the_user() {
        let mut edits = ProfileEdits::default();
        edits
            .edit(&SpeciesId::new("micropterus-dolomieu").unwrap())
            .spawn_temp_c = Some(Range {
            low: 15.0,
            high: 18.0,
        });
        let species = profiles(&["micropterus-dolomieu"], &edits);
        let c = card(&river(metric(), Voice::Plain, 16.0, 16.5), &species);
        let bass = &c.full.species[0];
        assert!(bass.edited);
        assert_eq!(bass.stage, Stage::Spawning);
        assert!(bass.text.contains("15.0–18.0 °C"), "{}", bass.text);
        assert!(c.chrome.credits.iter().any(|cr| cr.source == "Your edit"
            && cr.link.is_none()
            && cr.licence.as_deref() == Some("Typed in, not from a source")));
    }

    #[test]
    fn no_water_temperature_is_unknown_never_too_cold() {
        let mut input = river(us(), Voice::Plain, 10.0, 11.0);
        input.snapshot.water.temperature_c = None;
        let c = card(&input, &chosen());
        assert!(c.full.species.iter().all(|v| v.stage == Stage::Unknown));
        assert_eq!(
            c.full.species[0].text,
            "Smallmouth bass · Stage unknown · no water temperature reading here"
        );
        assert!(matches!(c.chrome.state, CardState::Unavailable { .. }));
        assert_eq!(c.compact.headline.value, None);
        assert!(c.chrome.footnote.is_none());
    }

    #[test]
    fn card_snapshot() {
        let c = card(&river(us(), Voice::Luna, 12.0, 13.0), &chosen());
        insta::assert_json_snapshot!(c);
    }
}
