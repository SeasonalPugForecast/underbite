//! Species views: the profile page with each value's source, and the season view.
//!
//! - [`species_list`]: the catalogue as rows, names first.
//! - [`profile_page`]: every field with its value in the user's units, its source and licence,
//!   edited fields marked, and what the editor pre-fills. A profile with `FishBase` values carries
//!   the CC BY-NC notice.
//! - [`season_view`]: the three kinds of season kept apart. *Now* reads the water temperature
//!   against the profile; *typical* is a month strip that names what each month drew on; *legal*
//!   is the official pages for the region, the user's notes and the regulations disclaimer, never
//!   dates or limits.
//! - [`apply_edit`] and [`edit_choices`]: the editor's text and picks, parsed in the user's units
//!   and locale into a [`SpeciesEdit`].

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::Serialize;

use crate::model::SpotSnapshot;
use crate::species::catalogue::Catalogue;
use crate::species::profile::{
    Field, FlowPreference, Licence, LightPreference, ProfileEdits, Range, Season, SourceRef,
    Sourced, SpeciesEdit, SpeciesProfile, WaterKind,
};
use crate::species::seasons::{
    LegalKind, MonthInput, MonthNormal, MonthObservations, Stage, legal, stage, typical_strip,
    water_of,
};
use crate::time::UnixTime;
use crate::trip::history::HistoryKind;
use crate::view::cards::species::{
    caution, licence_label, source_credit, stage_details, stage_label, temp_range, water_now,
};
use crate::view::format::Formatter;
use crate::view::strings::{Key, SafetyKey, Strings};
use crate::view::{CreditVm, Row, TrendDirection};

pub use crate::view::cards::species::EditedBy;

// ---------------------------------------------------------------------------------------------
// The list
// ---------------------------------------------------------------------------------------------

/// One species of the catalogue.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct SpeciesRow {
    pub id: String,
    /// "Smallmouth bass".
    pub name: String,
    /// "Micropterus dolomieu".
    pub scientific_name: String,
    pub water: Option<WaterKind>,
    /// "Fresh water".
    pub water_label: Option<String>,
    /// The user has edited the profile.
    pub edited: bool,
}

/// The catalogue with the user's edits, by name.
pub fn species_list(catalogue: &Catalogue, edits: &ProfileEdits, s: Strings) -> Vec<SpeciesRow> {
    let mut rows: Vec<SpeciesRow> = catalogue
        .iter()
        .filter_map(|base| catalogue.profile(&base.id, edits))
        .map(|p| SpeciesRow {
            id: p.id.as_str().to_owned(),
            name: p.display_name().to_owned(),
            scientific_name: p.scientific_name.value.clone(),
            water: p.water.as_ref().map(|w| w.value),
            water_label: p.water.as_ref().map(|w| water_label(s, w.value).to_owned()),
            edited: edits.species.contains_key(&p.id),
        })
        .collect();
    rows.sort_by_key(|r| (r.name.to_lowercase(), r.id.clone()));
    rows
}

// ---------------------------------------------------------------------------------------------
// The profile page
// ---------------------------------------------------------------------------------------------

/// Where a value came from, ready to show.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct SourceVm {
    /// "`FishBase`", "NOAA Fisheries", or "Your edit".
    pub name: String,
    pub licence: Licence,
    /// "CC BY-NC 4.0".
    pub licence_label: String,
    pub link: Option<String>,
    /// The record's own reference.
    pub reference: Option<String>,
    /// The citation the source asks for.
    pub citation: Option<String>,
}

/// One field of the profile page.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct ProfileFieldVm {
    /// The field the editor changes; `None` for the scientific name and the other names, which
    /// cannot be edited.
    pub field: Option<Field>,
    pub label: String,
    /// The value in the user's units, or "No source gives this".
    pub value: String,
    /// `None` when no source gives the field.
    pub source: Option<SourceVm>,
    /// The value is an edit, not the catalogue's.
    pub edited: bool,
    /// What the editor pre-fills, in the form [`apply_edit`] reads; empty for fields that
    /// cannot be edited.
    pub edit: Vec<String>,
}

/// A species profile, every value with its source.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct ProfilePage {
    pub id: String,
    pub name: String,
    pub scientific_name: String,
    pub fields: Vec<ProfileFieldVm>,
    /// Any field is edited.
    pub edited: bool,
    /// Every source of the page once, with its licence.
    pub credits: Vec<CreditVm>,
    /// The CC BY-NC notice when any value is from `FishBase`.
    pub notices: Vec<String>,
}

fn source_vm(s: Strings, src: &SourceRef, by: EditedBy) -> SourceVm {
    let credit = source_credit(s, src, by);
    SourceVm {
        name: credit.source,
        licence: src.licence,
        licence_label: licence_label(s, src.licence),
        link: src.url.clone(),
        reference: src.reference.clone(),
        citation: src.citation.clone(),
    }
}

fn water_label(s: Strings, w: WaterKind) -> &'static str {
    s.get(match w {
        WaterKind::Fresh => Key::SpeciesWaterFresh,
        WaterKind::Salt => Key::SpeciesWaterSalt,
        WaterKind::Both => Key::SpeciesWaterBoth,
    })
}

const fn water_id(w: WaterKind) -> &'static str {
    match w {
        WaterKind::Fresh => "fresh",
        WaterKind::Salt => "salt",
        WaterKind::Both => "both",
    }
}

const FLOWS: [(FlowPreference, &str, Key); 6] = [
    (
        FlowPreference::Incoming,
        "incoming",
        Key::SpeciesFlowIncoming,
    ),
    (
        FlowPreference::Outgoing,
        "outgoing",
        Key::SpeciesFlowOutgoing,
    ),
    (
        FlowPreference::SlackHigh,
        "slack_high",
        Key::SpeciesFlowSlackHigh,
    ),
    (
        FlowPreference::SlackLow,
        "slack_low",
        Key::SpeciesFlowSlackLow,
    ),
    (FlowPreference::Moving, "moving", Key::SpeciesFlowMoving),
    (FlowPreference::Slack, "slack", Key::SpeciesFlowSlack),
];

const LIGHTS: [(LightPreference, &str, Key); 5] = [
    (LightPreference::Dawn, "dawn", Key::SpeciesLightDawn),
    (LightPreference::Day, "day", Key::SpeciesLightDay),
    (LightPreference::Dusk, "dusk", Key::SpeciesLightDusk),
    (LightPreference::Night, "night", Key::SpeciesLightNight),
    (
        LightPreference::Overcast,
        "overcast",
        Key::SpeciesLightOvercast,
    ),
];

const SEASONS: [(Season, Key); 4] = [
    (Season::Spring, Key::SpeciesSeasonSpring),
    (Season::Summer, Key::SpeciesSeasonSummer),
    (Season::Autumn, Key::SpeciesSeasonAutumn),
    (Season::Winter, Key::SpeciesSeasonWinter),
];

fn flow_entry(p: FlowPreference) -> (&'static str, Key) {
    FLOWS
        .iter()
        .find(|(f, _, _)| *f == p)
        .map_or(("moving", Key::SpeciesFlowMoving), |(_, id, k)| (*id, *k))
}

fn light_entry(p: LightPreference) -> (&'static str, Key) {
    LIGHTS
        .iter()
        .find(|(l, _, _)| *l == p)
        .map_or(("day", Key::SpeciesLightDay), |(_, id, k)| (*id, *k))
}

fn join(s: Strings, items: &[String]) -> String {
    let mut out = String::new();
    for item in items {
        out = if out.is_empty() {
            item.clone()
        } else {
            s.fmt(Key::SpeciesJoin, &[("first", &out), ("second", item)])
        };
    }
    out
}

/// "2.0–10.0 m".
fn depth_range(f: &Formatter, r: Range) -> String {
    let (lo, hi) = (f.length(Some(r.low)), f.length(Some(r.high)));
    f.strings.fmt(
        Key::SpeciesRange,
        &[("from", &lo.text), ("to", &hi.text), ("unit", &lo.unit)],
    )
}

fn depth_text(f: &Formatter, d: &BTreeMap<Season, Range>) -> String {
    let s = f.strings;
    let mut ranges = d.values();
    if d.len() == SEASONS.len()
        && let Some(first) = ranges.next()
        && ranges.all(|r| r == first)
    {
        return s.fmt(
            Key::SpeciesDepthAllYear,
            &[("range", &depth_range(f, *first))],
        );
    }
    let parts: Vec<String> = SEASONS
        .iter()
        .filter_map(|(season, key)| {
            d.get(season).map(|r| {
                s.fmt(
                    Key::SpeciesDepthSeason,
                    &[("season", s.get(*key)), ("range", &depth_range(f, *r))],
                )
            })
        })
        .collect();
    join(s, &parts)
}

/// A temperature as the editor shows it: the number only, in the user's unit.
fn temp_number(f: &Formatter, c: f32) -> String {
    f.temperature(Some(c)).text
}

fn range_edit(f: &Formatter, r: Option<&Sourced<Range>>) -> Vec<String> {
    r.map_or_else(
        || vec![String::new(), String::new()],
        |r| vec![temp_number(f, r.value.low), temp_number(f, r.value.high)],
    )
}

fn depth_edit(f: &Formatter, d: Option<&Sourced<BTreeMap<Season, Range>>>) -> Vec<String> {
    SEASONS
        .iter()
        .flat_map(|(season, _)| match d.and_then(|d| d.value.get(season)) {
            Some(r) => [f.length(Some(r.low)).text, f.length(Some(r.high)).text],
            None => [String::new(), String::new()],
        })
        .collect()
}

/// The profile page. `by` says who typed the edited values: the user, or the sender of a
/// shared profile.
#[allow(clippy::too_many_lines)]
pub fn profile_page(p: &SpeciesProfile, f: &Formatter, by: EditedBy) -> ProfilePage {
    let s = f.strings;
    let missing = s.get(Key::SpeciesNotKnown).to_owned();
    let mut fields = Vec::new();
    let mut add = |field: Option<Field>,
                   key: Key,
                   source: Option<&SourceRef>,
                   value: Option<String>,
                   edit: Vec<String>| {
        fields.push(ProfileFieldVm {
            field,
            label: s.get(key).to_owned(),
            value: value.unwrap_or_else(|| missing.clone()),
            source: source.map(|src| source_vm(s, src, by)),
            edited: source.is_some_and(|src| src.licence == Licence::UserEdited),
            edit,
        });
    };
    add(
        None,
        Key::SpeciesFieldScientific,
        Some(&p.scientific_name.source),
        Some(p.scientific_name.value.clone()),
        Vec::new(),
    );
    add(
        Some(Field::CommonNames),
        Key::SpeciesFieldCommonNames,
        p.common_names.as_ref().map(|c| &c.source),
        p.common_names.as_ref().map(|c| join(s, &c.value)),
        p.common_names
            .as_ref()
            .map(|c| c.value.clone())
            .unwrap_or_default(),
    );
    for a in &p.aliases {
        add(
            None,
            Key::SpeciesFieldAlias,
            Some(&a.source),
            Some(a.value.clone()),
            Vec::new(),
        );
    }
    add(
        Some(Field::Water),
        Key::SpeciesFieldWater,
        p.water.as_ref().map(|w| &w.source),
        p.water.as_ref().map(|w| water_label(s, w.value).to_owned()),
        p.water
            .as_ref()
            .map(|w| vec![water_id(w.value).to_owned()])
            .unwrap_or_default(),
    );
    for (field, key, r) in [
        (
            Field::PreferredTemp,
            Key::SpeciesFieldPreferred,
            &p.preferred_temp_c,
        ),
        (
            Field::ToleratedTemp,
            Key::SpeciesFieldTolerated,
            &p.tolerated_temp_c,
        ),
        (Field::SpawnTemp, Key::SpeciesFieldSpawn, &p.spawn_temp_c),
    ] {
        add(
            Some(field),
            key,
            r.as_ref().map(|r| &r.source),
            r.as_ref().map(|r| temp_range(f, r.value)),
            range_edit(f, r.as_ref()),
        );
    }
    add(
        Some(Field::Depth),
        Key::SpeciesFieldDepth,
        p.depth_m.as_ref().map(|d| &d.source),
        p.depth_m.as_ref().map(|d| depth_text(f, &d.value)),
        depth_edit(f, p.depth_m.as_ref()),
    );
    add(
        Some(Field::Habitat),
        Key::SpeciesFieldHabitat,
        p.habitat.as_ref().map(|v| &v.source),
        p.habitat.as_ref().map(|v| join(s, &v.value)),
        p.habitat
            .as_ref()
            .map(|v| v.value.clone())
            .unwrap_or_default(),
    );
    add(
        Some(Field::Flow),
        Key::SpeciesFieldFlow,
        p.flow.as_ref().map(|v| &v.source),
        p.flow.as_ref().map(|v| {
            let words: Vec<String> = v
                .value
                .iter()
                .map(|x| s.get(flow_entry(*x).1).to_owned())
                .collect();
            join(s, &words)
        }),
        p.flow
            .as_ref()
            .map(|v| {
                v.value
                    .iter()
                    .map(|x| flow_entry(*x).0.to_owned())
                    .collect()
            })
            .unwrap_or_default(),
    );
    add(
        Some(Field::Light),
        Key::SpeciesFieldLight,
        p.light.as_ref().map(|v| &v.source),
        p.light.as_ref().map(|v| {
            let words: Vec<String> = v
                .value
                .iter()
                .map(|x| s.get(light_entry(*x).1).to_owned())
                .collect();
            join(s, &words)
        }),
        p.light
            .as_ref()
            .map(|v| {
                v.value
                    .iter()
                    .map(|x| light_entry(*x).0.to_owned())
                    .collect()
            })
            .unwrap_or_default(),
    );
    for (field, key, v) in [
        (Field::Lures, Key::SpeciesFieldLures, &p.lures),
        (Field::Regions, Key::SpeciesFieldRegions, &p.regions),
    ] {
        add(
            Some(field),
            key,
            v.as_ref().map(|v| &v.source),
            v.as_ref().map(|v| join(s, &v.value)),
            v.as_ref().map(|v| v.value.clone()).unwrap_or_default(),
        );
    }

    let mut credits: Vec<CreditVm> = Vec::new();
    for src in p.sources() {
        let c = source_credit(s, src, by);
        if !credits.contains(&c) {
            credits.push(c);
        }
    }
    let fishbase = p
        .sources()
        .iter()
        .any(|src| src.licence == Licence::CcByNc4);
    ProfilePage {
        id: p.id.as_str().to_owned(),
        name: p.display_name().to_owned(),
        scientific_name: p.scientific_name.value.clone(),
        edited: fields.iter().any(|x| x.edited),
        fields,
        credits,
        notices: if fishbase {
            vec![s.safety(SafetyKey::SpeciesFishBaseNotice).to_owned()]
        } else {
            Vec::new()
        },
    }
}

// ---------------------------------------------------------------------------------------------
// The season view
// ---------------------------------------------------------------------------------------------

/// The biological season: where the water puts the species now.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct SeasonNow {
    pub title: String,
    pub stage: Stage,
    pub stage_label: String,
    /// "water 58 °F, spawn usually 60–65 °F".
    pub text: String,
    /// The trout-stress safety text, when it applies.
    pub caution: Option<String>,
    /// Water, the profile's ranges and the heat limit.
    pub rows: Vec<Row>,
    pub credits: Vec<CreditVm>,
}

/// One month of the typical strip.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct TypicalMonthVm {
    /// 1 to 12.
    pub month: u8,
    /// "Apr".
    pub label: String,
    pub stage: Stage,
    pub stage_label: String,
    /// The month's typical water temperature.
    pub water: Option<String>,
    /// "12 observed".
    pub observations: Option<String>,
    /// "From the profile, water temperature 1991–2020, station records".
    pub basis: String,
}

/// The typical season: a strip of twelve months, January first.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct SeasonTypical {
    pub title: String,
    pub months: Vec<TypicalMonthVm>,
    /// Says so when no month has a typical water temperature.
    pub note: Option<String>,
}

/// An official page.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct LegalLinkVm {
    pub kind: LegalKind,
    /// "Freshwater regulations".
    pub title: String,
    /// "NYSDEC, link checked 2026-11-10".
    pub detail: String,
    pub link: String,
}

/// The legal season: links and the user's notes only, with the disclaimer.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct SeasonLegal {
    pub title: String,
    /// The region the links are for (`US-NY`).
    pub region: Option<String>,
    pub links: Vec<LegalLinkVm>,
    /// Why there are no links.
    pub empty: Option<String>,
    pub notes_title: String,
    /// The user's notes, as typed.
    pub notes: Option<String>,
    /// The regulations disclaimer: always shown.
    pub disclaimer: String,
}

/// The three seasons of one species, kept apart.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct SeasonView {
    pub id: String,
    pub name: String,
    pub now: SeasonNow,
    pub typical: SeasonTypical,
    pub legal: SeasonLegal,
}

/// What the season view is built from besides the profile.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SeasonInput<'a> {
    /// The water temperature now, °C, and its trend.
    pub water_c: Option<f32>,
    pub trend: Option<TrendDirection>,
    /// Typical water temperatures by month.
    pub normals: &'a [MonthNormal],
    /// Observations of the species by month.
    pub observations: &'a [MonthObservations],
    /// ISO 3166-2 (`US-NY`).
    pub region: Option<&'a str>,
    /// The user's own notes on the rules.
    pub notes: &'a str,
    /// The trout-stress threshold, °C.
    pub trout_stress_c: Option<f32>,
}

/// The spot's water temperature at `now` and its trend, as the species card reads it.
pub fn water_reading(
    snapshot: &SpotSnapshot,
    now: UnixTime,
    f: &Formatter,
) -> Option<(f32, TrendDirection)> {
    water_now(snapshot, now, f).map(|w| (w.celsius, w.trend))
}

fn history_kind(s: Strings, kind: HistoryKind) -> &'static str {
    s.get(match kind {
        HistoryKind::Station => Key::TripHistoryStation,
        HistoryKind::Reanalysis => Key::TripHistoryReanalysis,
        HistoryKind::Model => Key::TripHistoryModel,
    })
}

/// "water temperature 1991–2020, station records".
fn water_input(s: Strings, basis: &crate::trip::history::HistoryBasis) -> String {
    let kind = history_kind(s, basis.kind);
    match basis.years {
        Some(years) => s.fmt(
            Key::SpeciesInputWaterYears,
            &[
                ("first", &years.first.to_string()),
                ("last", &years.last.to_string()),
                ("kind", kind),
            ],
        ),
        None => s.fmt(
            Key::SpeciesInputWaterCount,
            &[("n", &basis.years_sampled.to_string()), ("kind", kind)],
        ),
    }
}

/// The three seasons of `profile`: now, typical months and the legal links.
pub fn season_view(
    profile: &SpeciesProfile,
    input: &SeasonInput<'_>,
    f: &Formatter,
    by: EditedBy,
) -> SeasonView {
    let trout = input
        .trout_stress_c
        .unwrap_or(crate::species::seasons::DEFAULT_TROUT_STRESS_C);
    SeasonView {
        id: profile.id.as_str().to_owned(),
        name: profile.display_name().to_owned(),
        now: season_now(profile, input, trout, f, by),
        typical: season_typical(profile, input, trout, f),
        legal: season_legal(profile, input, f.strings),
    }
}

fn season_now(
    profile: &SpeciesProfile,
    input: &SeasonInput<'_>,
    trout: f32,
    f: &Formatter,
    by: EditedBy,
) -> SeasonNow {
    let s = f.strings;
    let report = stage(
        profile,
        input.water_c,
        input.trend.unwrap_or(TrendDirection::Unknown),
        trout,
    );
    let mut rows = Vec::new();
    let mut credits: Vec<CreditVm> = Vec::new();
    if let Some(celsius) = input.water_c.filter(|c| c.is_finite()) {
        rows.push(Row {
            label: s.get(Key::SpeciesRowWater).to_owned(),
            value: f.temperature(Some(celsius)).full(),
            detail: None,
        });
    }
    for (key, range) in [
        (Key::SpeciesFieldSpawn, &profile.spawn_temp_c),
        (Key::SpeciesFieldPreferred, &profile.preferred_temp_c),
        (Key::SpeciesFieldTolerated, &profile.tolerated_temp_c),
    ] {
        if let Some(range) = range {
            let credit = source_credit(s, &range.source, by);
            rows.push(Row {
                label: s.get(key).to_owned(),
                value: temp_range(f, range.value),
                detail: Some(credit.source.clone()),
            });
            if !credits.contains(&credit) {
                credits.push(credit);
            }
        }
    }
    SeasonNow {
        title: s.get(Key::SpeciesSeasonNowTitle).to_owned(),
        stage: report.stage,
        stage_label: stage_label(s, report.stage).to_owned(),
        text: stage_details(f, &report),
        caution: caution(f, &report),
        rows,
        credits,
    }
}

fn season_typical(
    profile: &SpeciesProfile,
    input: &SeasonInput<'_>,
    trout: f32,
    f: &Formatter,
) -> SeasonTypical {
    let s = f.strings;
    let strip = typical_strip(profile, input.normals, input.observations, trout);
    let months: Vec<TypicalMonthVm> = strip
        .months
        .iter()
        .map(|month| {
            let words: Vec<String> = month
                .inputs
                .iter()
                .map(|used| match used {
                    MonthInput::Profile => s.get(Key::SpeciesInputProfile).to_owned(),
                    MonthInput::Observations => s.get(Key::SpeciesInputObservations).to_owned(),
                    MonthInput::WaterTemp => month
                        .water_temp
                        .as_ref()
                        .map(|band| water_input(s, &band.basis))
                        .unwrap_or_default(),
                })
                .filter(|w| !w.is_empty())
                .collect();
            TypicalMonthVm {
                month: month.month,
                label: s.month(month.month).to_owned(),
                stage: month.stage,
                stage_label: stage_label(s, month.stage).to_owned(),
                water: month
                    .water_temp
                    .as_ref()
                    .map(|band| f.temperature(Some(band.median)).full()),
                observations: month
                    .observations
                    .map(|n| s.fmt(Key::SpeciesObservations, &[("n", &n.to_string())])),
                basis: if words.is_empty() {
                    s.get(Key::SpeciesTypicalNothing).to_owned()
                } else {
                    s.fmt(Key::SpeciesTypicalFrom, &[("inputs", &join(s, &words))])
                },
            }
        })
        .collect();
    SeasonTypical {
        title: s.get(Key::SpeciesSeasonTypicalTitle).to_owned(),
        note: months
            .iter()
            .all(|month| month.water.is_none())
            .then(|| s.get(Key::SpeciesTypicalNoNormals).to_owned()),
        months,
    }
}

/// Links and notes, never rules.
fn season_legal(profile: &SpeciesProfile, input: &SeasonInput<'_>, s: Strings) -> SeasonLegal {
    let region = input.region.map(str::trim).filter(|code| !code.is_empty());
    let season = legal(
        region.unwrap_or_default(),
        water_of(profile),
        "en",
        input.notes,
    );
    let links: Vec<LegalLinkVm> = if region.is_some() {
        season
            .links
            .iter()
            .map(|link| LegalLinkVm {
                kind: link.kind,
                title: s
                    .safety(match link.kind {
                        LegalKind::Freshwater => SafetyKey::RegFreshwater,
                        LegalKind::Saltwater => SafetyKey::RegSaltwater,
                        LegalKind::Federal => SafetyKey::RegFederal,
                        LegalKind::Licence => SafetyKey::RegLicence,
                    })
                    .to_owned(),
                detail: s.safety_fmt(
                    if link.verified {
                        SafetyKey::RegChecked
                    } else {
                        SafetyKey::RegUnconfirmed
                    },
                    &[("agency", &link.label), ("date", &link.checked)],
                ),
                link: link.url.clone(),
            })
            .collect()
    } else {
        Vec::new()
    };
    let empty = match region {
        None => Some(s.get(Key::SpeciesLegalNoRegion).to_owned()),
        Some(code) if links.is_empty() => {
            Some(s.fmt(Key::SpeciesLegalNoLinks, &[("region", code)]))
        }
        Some(_) => None,
    };
    SeasonLegal {
        title: s.get(Key::SpeciesSeasonLegalTitle).to_owned(),
        region: region.map(str::to_owned),
        links,
        empty,
        notes_title: s.get(Key::SpeciesLegalNotes).to_owned(),
        notes: season.notes,
        disclaimer: s.safety(season.disclaimer).to_owned(),
    }
}

// ---------------------------------------------------------------------------------------------
// Editing
// ---------------------------------------------------------------------------------------------

/// Why the editor's input is not acceptable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum EditError {
    #[error("not a number")]
    NotANumber,
    #[error("only one end of a range")]
    BothEnds,
    #[error("the low end is above the high end")]
    Backwards,
    #[error("out of range")]
    OutOfRange,
    #[error("nothing entered")]
    Empty,
    #[error("not one of the choices")]
    NotAChoice,
}

impl EditError {
    /// What to tell the user, in their voice and locale.
    pub fn text(self, s: Strings) -> String {
        match self {
            Self::NotANumber => s.fmt(Key::SpeciesEditNumber, &[("example", &s.number(12.5, 1))]),
            Self::BothEnds => s.get(Key::SpeciesEditBothEnds).to_owned(),
            Self::Backwards => s.get(Key::SpeciesEditBackwards).to_owned(),
            Self::OutOfRange => s.get(Key::SpeciesEditOutOfRange).to_owned(),
            Self::Empty => s.get(Key::SpeciesEditEmpty).to_owned(),
            Self::NotAChoice => s.get(Key::SpeciesEditChoice).to_owned(),
        }
    }
}

/// One choice of a picker field.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, JsonSchema)]
pub struct EditChoice {
    pub id: String,
    pub label: String,
}

/// The choices of `water`, `flow` and `light`; none for the other fields.
pub fn edit_choices(field: Field, s: Strings) -> Vec<EditChoice> {
    let choice = |id: &str, key: Key| EditChoice {
        id: id.to_owned(),
        label: s.get(key).to_owned(),
    };
    match field {
        Field::Water => [WaterKind::Fresh, WaterKind::Salt, WaterKind::Both]
            .into_iter()
            .map(|w| EditChoice {
                id: water_id(w).to_owned(),
                label: water_label(s, w).to_owned(),
            })
            .collect(),
        Field::Flow => FLOWS.iter().map(|(_, id, k)| choice(id, *k)).collect(),
        Field::Light => LIGHTS.iter().map(|(_, id, k)| choice(id, *k)).collect(),
        _ => Vec::new(),
    }
}

/// A number with an optional minus sign and the locale's decimal mark. Blank is `None`.
fn parse_number(text: &str, decimal_mark: char) -> Result<Option<f32>, EditError> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let (negative, body) = match text.strip_prefix(['-', '\u{2212}']) {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let mut normalised = String::with_capacity(body.len() + 1);
    if negative {
        normalised.push('-');
    }
    let (mut digits, mut marked) = (false, false);
    for c in body.chars() {
        match c {
            '0'..='9' => {
                digits = true;
                normalised.push(c);
            }
            c if c == decimal_mark && !marked => {
                marked = true;
                normalised.push('.');
            }
            _ => return Err(EditError::NotANumber),
        }
    }
    if !digits {
        return Err(EditError::NotANumber);
    }
    normalised
        .parse::<f32>()
        .ok()
        .filter(|v| v.is_finite())
        .map(Some)
        .ok_or(EditError::NotANumber)
}

/// A low and a high end typed in a display unit, converted to canonical and checked.
fn parse_range(
    low: &str,
    high: &str,
    decimal_mark: char,
    to_canonical: &dyn Fn(f32) -> f32,
    (min, max): (f32, f32),
) -> Result<Option<Range>, EditError> {
    match (
        parse_number(low, decimal_mark)?,
        parse_number(high, decimal_mark)?,
    ) {
        (None, None) => Ok(None),
        (Some(l), Some(h)) => {
            let r = Range {
                low: to_canonical(l),
                high: to_canonical(h),
            };
            if r.low > r.high {
                Err(EditError::Backwards)
            } else if r.low < min - 0.05 || r.high > max + 0.05 {
                Err(EditError::OutOfRange)
            } else {
                Ok(Some(r))
            }
        }
        _ => Err(EditError::BothEnds),
    }
}

fn texts(values: &[String]) -> Result<Vec<String>, EditError> {
    let out: Vec<String> = values
        .iter()
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
        .collect();
    if out.is_empty() {
        Err(EditError::Empty)
    } else {
        Ok(out)
    }
}

fn picks<T: Copy + PartialEq>(
    values: &[String],
    table: &[(T, &str, Key)],
) -> Result<Vec<T>, EditError> {
    let mut out = Vec::new();
    for v in values.iter().map(|v| v.trim()).filter(|v| !v.is_empty()) {
        let (x, _, _) = table
            .iter()
            .find(|(_, id, _)| *id == v)
            .ok_or(EditError::NotAChoice)?;
        if !out.contains(x) {
            out.push(*x);
        }
    }
    if out.is_empty() {
        Err(EditError::Empty)
    } else {
        Ok(out)
    }
}

/// Lays the editor's input for one field over `edit`. The values have the form of
/// [`ProfileFieldVm::edit`]:
///
/// - common names, habitat, lures, regions: one entry per value (blank entries are dropped);
/// - water: one id of [`edit_choices`] (`fresh`, `salt`, `both`); flow and light: their ids;
/// - preferred, tolerated and spawning temperature: `[low, high]` in the user's temperature unit;
/// - depth: `[low, high]` for spring, summer, autumn and winter in turn (eight entries) in the
///   user's height unit; a blank pair leaves that season out.
///
/// Numbers use the locale's decimal mark. Nothing changes on an error.
///
/// # Errors
/// [`EditError`] for a value that cannot be read or cannot be right.
pub fn apply_edit(
    edit: &mut SpeciesEdit,
    field: Field,
    values: &[String],
    f: &Formatter,
) -> Result<(), EditError> {
    let mark = f.strings.locale.decimal_mark();
    let temp = f.units.temperature;
    let height = f.units.height;
    let get = |i: usize| values.get(i).map_or("", String::as_str);
    let temps = |bounds: (f32, f32)| {
        parse_range(get(0), get(1), mark, &|v| temp.to_canonical(v), bounds)?
            .ok_or(EditError::BothEnds)
    };
    match field {
        Field::CommonNames => edit.common_names = Some(texts(values)?),
        Field::Habitat => edit.habitat = Some(texts(values)?),
        Field::Lures => edit.lures = Some(texts(values)?),
        Field::Regions => edit.regions = Some(texts(values)?),
        Field::Water => {
            let w = match texts(values)?.as_slice() {
                [one] if one == "fresh" => WaterKind::Fresh,
                [one] if one == "salt" => WaterKind::Salt,
                [one] if one == "both" => WaterKind::Both,
                _ => return Err(EditError::NotAChoice),
            };
            edit.water = Some(w);
        }
        Field::Flow => edit.flow = Some(picks(values, &FLOWS)?),
        Field::Light => edit.light = Some(picks(values, &LIGHTS)?),
        Field::PreferredTemp => edit.preferred_temp_c = Some(temps((-5.0, 45.0))?),
        Field::ToleratedTemp => edit.tolerated_temp_c = Some(temps((-5.0, 50.0))?),
        Field::SpawnTemp => edit.spawn_temp_c = Some(temps((-5.0, 45.0))?),
        Field::Depth => {
            let mut depth = BTreeMap::new();
            for (i, (season, _)) in SEASONS.iter().enumerate() {
                if let Some(r) = parse_range(
                    get(2 * i),
                    get(2 * i + 1),
                    mark,
                    &|v| height.to_canonical(v),
                    (0.0, 12_000.0),
                )? {
                    depth.insert(*season, r);
                }
            }
            if depth.is_empty() {
                return Err(EditError::Empty);
            }
            edit.depth_m = Some(depth);
        }
    }
    Ok(())
}

#[cfg(test)]
pub(in crate::view) mod tests {
    use super::*;
    use crate::provenance::SourceId;
    use crate::trip::SpeciesId;
    use crate::trip::history::{Band, HistoryBasis};
    use crate::units::{HeightUnit, TempUnit};
    use crate::view::Voice;
    use crate::view::cards::flow::fixtures::{metric, us};
    use crate::view::locale::Locale;
    use whether_core::history::YearRange;
    use whether_core::presentation::TimeFormat;

    pub(in crate::view) fn formatter(units: crate::view::DisplayUnits, voice: Voice) -> Formatter {
        Formatter::new(units, voice, TimeFormat::H12, "America/New_York")
    }

    pub(in crate::view) fn id(s: &str) -> SpeciesId {
        SpeciesId::new(s).unwrap()
    }

    pub(in crate::view) fn catalogue() -> Catalogue {
        Catalogue::bundled().unwrap()
    }

    /// Smallmouth bass with the user's lures and spawning range.
    pub(in crate::view) fn edited_bass() -> (SpeciesProfile, ProfileEdits) {
        let mut edits = ProfileEdits::default();
        let e = edits.edit(&id("micropterus-dolomieu"));
        e.lures = Some(vec!["Tube jig".into(), "Ned rig".into()]);
        e.spawn_temp_c = Some(Range {
            low: 15.0,
            high: 18.0,
        });
        let p = catalogue()
            .profile(&id("micropterus-dolomieu"), &edits)
            .unwrap();
        (p, edits)
    }

    /// Typical Croton water temperatures, °C, January first.
    pub(in crate::view) fn croton_normals() -> Vec<MonthNormal> {
        [
            2.0, 2.5, 5.0, 9.5, 15.0, 20.0, 23.0, 22.5, 18.5, 12.5, 7.5, 3.5,
        ]
        .iter()
        .enumerate()
        .map(|(i, median)| MonthNormal {
            month: u8::try_from(i + 1).unwrap(),
            band: Band {
                p10: median - 1.0,
                median: *median,
                p90: median + 1.0,
                basis: HistoryBasis {
                    source: SourceId::USGS,
                    kind: HistoryKind::Station,
                    years: Some(YearRange {
                        first: 1991,
                        last: 2020,
                    }),
                    years_sampled: 30,
                    percentiles: None,
                },
            },
        })
        .collect()
    }

    #[test]
    fn the_list_is_by_name_and_marks_edited_species() {
        let (_, edits) = edited_bass();
        let rows = species_list(&catalogue(), &edits, Strings::new(Voice::Plain));
        assert_eq!(rows.len(), catalogue().len());
        let names: Vec<String> = rows.iter().map(|r| r.name.to_lowercase()).collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted);
        let bass = rows
            .iter()
            .find(|r| r.id == "micropterus-dolomieu")
            .unwrap();
        assert!(bass.edited);
        assert_eq!(bass.water_label.as_deref(), Some("Fresh water"));
        assert_eq!(rows.iter().filter(|r| r.edited).count(), 1);
    }

    #[test]
    fn every_shown_value_has_its_source_and_licence() {
        let f = formatter(us(), Voice::Plain);
        for p in catalogue().iter() {
            let page = profile_page(p, &f, EditedBy::You);
            for field in &page.fields {
                match &field.source {
                    Some(src) => {
                        assert_ne!(src.name, "");
                        assert_ne!(src.licence_label, "");
                        if src.licence == Licence::CcByNc4 {
                            assert_eq!(src.licence_label, "CC BY-NC 4.0");
                            assert!(src.link.is_some() && src.citation.is_some());
                        }
                    }
                    None => assert_eq!(field.value, "No source gives this", "{}", page.id),
                }
            }
            let nc = p.sources().iter().any(|s| s.licence == Licence::CcByNc4);
            assert_eq!(!page.notices.is_empty(), nc, "{}", page.id);
            assert!(!page.edited);
        }
    }

    #[test]
    fn edited_fields_are_marked_and_credited_to_whoever_typed_them() {
        let (p, _) = edited_bass();
        let f = formatter(us(), Voice::Plain);
        let page = profile_page(&p, &f, EditedBy::You);
        assert!(page.edited);
        let lures = page
            .fields
            .iter()
            .find(|x| x.field == Some(Field::Lures))
            .unwrap();
        assert!(lures.edited);
        assert_eq!(lures.value, "Tube jig, Ned rig");
        assert_eq!(lures.edit, vec!["Tube jig", "Ned rig"]);
        let src = lures.source.as_ref().unwrap();
        assert_eq!((src.name.as_str(), src.link.as_ref()), ("Your edit", None));
        let spawn = page
            .fields
            .iter()
            .find(|x| x.field == Some(Field::SpawnTemp))
            .unwrap();
        assert_eq!(spawn.value, "59–64 °F");
        assert_eq!(spawn.edit, vec!["59", "64"]);
        let tolerated = page
            .fields
            .iter()
            .find(|x| x.field == Some(Field::ToleratedTemp))
            .unwrap();
        assert!(!tolerated.edited);
        assert_eq!(tolerated.source.as_ref().unwrap().name, "FishBase");
        let shared = profile_page(&p, &f, EditedBy::Sender);
        assert!(
            shared
                .credits
                .iter()
                .any(|c| c.source == "Edited by the sender")
        );
    }

    #[test]
    fn depth_reads_by_season_or_all_year_in_the_users_unit() {
        let f = formatter(metric(), Voice::Plain);
        let r = Range {
            low: 2.0,
            high: 10.0,
        };
        let all: BTreeMap<Season, Range> = SEASONS.iter().map(|(s, _)| (*s, r)).collect();
        assert_eq!(depth_text(&f, &all), "2.0–10.0 m, all year");
        let two = BTreeMap::from([
            (Season::Summer, r),
            (
                Season::Winter,
                Range {
                    low: 8.0,
                    high: 20.0,
                },
            ),
        ]);
        assert_eq!(
            depth_text(&f, &two),
            "Summer: 2.0–10.0 m, Winter: 8.0–20.0 m"
        );
        let f = formatter(us(), Voice::Plain);
        assert_eq!(depth_text(&f, &all), "6.6–32.8 ft, all year");
    }

    #[test]
    fn the_editor_reads_the_users_units_and_locale() {
        let f = formatter(us(), Voice::Plain);
        let mut e = SpeciesEdit::default();
        apply_edit(&mut e, Field::SpawnTemp, &["59".into(), "64".into()], &f).unwrap();
        let r = e.spawn_temp_c.unwrap();
        assert!((r.low - 15.0).abs() < 0.01 && (r.high - 17.78).abs() < 0.01);
        let mf = formatter(metric(), Voice::Plain).with_locale(Locale::PSEUDO);
        apply_edit(
            &mut e,
            Field::PreferredTemp,
            &["-1,5".into(), "8".into()],
            &mf,
        )
        .unwrap();
        assert_eq!(e.preferred_temp_c.unwrap().low, -1.5);
        let mut depth = vec![String::new(); 8];
        depth[2] = "2".into();
        depth[3] = "10".into();
        apply_edit(
            &mut e,
            Field::Depth,
            &depth,
            &formatter(metric(), Voice::Plain),
        )
        .unwrap();
        assert_eq!(
            e.depth_m.as_ref().unwrap().get(&Season::Summer),
            Some(&Range {
                low: 2.0,
                high: 10.0
            })
        );
        assert_eq!(e.depth_m.as_ref().unwrap().len(), 1);
        apply_edit(&mut e, Field::Water, &["both".into()], &f).unwrap();
        apply_edit(
            &mut e,
            Field::Flow,
            &["incoming".into(), "moving".into(), "moving".into()],
            &f,
        )
        .unwrap();
        assert_eq!(
            e.flow,
            Some(vec![FlowPreference::Incoming, FlowPreference::Moving])
        );
        apply_edit(
            &mut e,
            Field::Habitat,
            &[" rocky points ".into(), String::new()],
            &f,
        )
        .unwrap();
        assert_eq!(e.habitat, Some(vec!["rocky points".to_owned()]));
        assert_eq!(e.water, Some(WaterKind::Both));
        assert_eq!(f.units.temperature, TempUnit::F);
        assert_eq!(f.units.height, HeightUnit::Ft);
    }

    #[test]
    fn the_editor_refuses_what_cannot_be_right_and_changes_nothing() {
        let f = formatter(us(), Voice::Plain);
        let mut e = SpeciesEdit::default();
        let two = |a: &str, b: &str| vec![a.to_owned(), b.to_owned()];
        for (field, values, err) in [
            (Field::SpawnTemp, two("64", "59"), EditError::Backwards),
            (Field::SpawnTemp, two("59", ""), EditError::BothEnds),
            (Field::SpawnTemp, two("", ""), EditError::BothEnds),
            (Field::SpawnTemp, two("5x", "60"), EditError::NotANumber),
            (Field::SpawnTemp, two("12.5.1", "60"), EditError::NotANumber),
            (
                Field::ToleratedTemp,
                two("40", "140"),
                EditError::OutOfRange,
            ),
            (Field::Depth, vec![String::new(); 8], EditError::Empty),
            (Field::Lures, vec![" ".into()], EditError::Empty),
            (Field::Water, vec!["brackish".into()], EditError::NotAChoice),
            (Field::Light, vec!["noon".into()], EditError::NotAChoice),
        ] {
            assert_eq!(
                apply_edit(&mut e, field, &values, &f),
                Err(err),
                "{field:?} {values:?}"
            );
        }
        assert!(e.is_empty());
        assert_eq!(
            EditError::NotANumber.text(Strings::new(Voice::Plain)),
            "Enter a number, such as 12.5."
        );
        assert_eq!(
            edit_choices(Field::Water, Strings::new(Voice::Plain)).len(),
            3
        );
        assert_eq!(
            edit_choices(Field::Flow, Strings::new(Voice::Plain)).len(),
            6
        );
        assert_eq!(edit_choices(Field::Lures, Strings::new(Voice::Plain)), []);
    }

    #[test]
    fn the_editor_pre_fills_what_it_reads_back() {
        let f = formatter(metric(), Voice::Plain);
        let (p, _) = edited_bass();
        let page = profile_page(&p, &f, EditedBy::You);
        let mut e = SpeciesEdit::default();
        for field in page.fields.iter().filter(|x| x.source.is_some()) {
            if let Some(which) = field.field {
                apply_edit(&mut e, which, &field.edit, &f).unwrap();
            }
        }
        let again = p.with_edits(&e);
        assert_eq!(
            again.spawn_temp_c.unwrap().value,
            p.spawn_temp_c.unwrap().value
        );
        assert_eq!(
            again.common_names.unwrap().value,
            p.common_names.unwrap().value
        );
    }

    #[test]
    fn the_season_view_keeps_three_sections_and_the_disclaimer() {
        let f = formatter(us(), Voice::Luna);
        let p = catalogue()
            .get(&id("micropterus-dolomieu"))
            .unwrap()
            .clone();
        let normals = croton_normals();
        let obs = [MonthObservations {
            month: 5,
            count: 12,
        }];
        let v = season_view(
            &p,
            &SeasonInput {
                water_c: Some(11.0),
                trend: Some(TrendDirection::Rising),
                normals: &normals,
                observations: &obs,
                region: Some("US-NY"),
                notes: "  Check the Croton special rules. ",
                trout_stress_c: Some(20.0),
            },
            &f,
            EditedBy::You,
        );
        assert_eq!(v.now.stage, Stage::PreSpawn);
        assert_eq!(v.now.text, "water 52 °F, spawn usually 55–72 °F");
        assert!(v.now.credits.iter().all(|c| c.licence.is_some()));
        assert_eq!(v.typical.months.len(), 12);
        assert!(v.typical.note.is_none());
        let may = &v.typical.months[4];
        assert_eq!(may.label, "May");
        assert_eq!(may.stage, Stage::Spawning);
        assert_eq!(may.observations.as_deref(), Some("12 observed"));
        assert_eq!(
            may.basis,
            "From the profile, water temperature 1991–2020, station records, observations"
        );
        assert_ne!(v.legal.links.len(), 0);
        assert!(v.legal.links.iter().all(|l| l.link.starts_with("https://")));
        assert!(v.legal.links.iter().all(|l| l.kind != LegalKind::Saltwater));
        assert_eq!(
            v.legal.notes.as_deref(),
            Some("Check the Croton special rules.")
        );
        assert!(v.legal.disclaimer.starts_with("Fishing regulations change"));
        // Nothing in the legal section is a date range or a limit: only links and notes.
        let json = serde_json::to_value(&v.legal).unwrap();
        assert_eq!(
            json.as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect::<Vec<_>>(),
            [
                "disclaimer",
                "empty",
                "links",
                "notes",
                "notes_title",
                "region",
                "title"
            ]
        );
    }

    #[test]
    fn without_inputs_the_season_view_says_what_is_missing() {
        let f = formatter(metric(), Voice::Plain);
        let p = catalogue().get(&id("salmo-trutta")).unwrap().clone();
        let v = season_view(&p, &SeasonInput::default(), &f, EditedBy::You);
        assert_eq!(v.now.stage, Stage::Unknown);
        assert_eq!(v.now.text, "no water temperature reading here");
        assert!(v.typical.months.iter().all(|m| m.stage == Stage::Unknown));
        assert_eq!(v.typical.months[0].basis, "Nothing to go on");
        assert!(v.typical.note.is_some());
        assert_eq!(v.legal.links, []);
        assert_eq!(
            v.legal.empty.as_deref(),
            Some("Set the spot's region to see the official regulation pages.")
        );
        assert_ne!(v.legal.disclaimer, "");
        let v = season_view(
            &p,
            &SeasonInput {
                region: Some("XX-ZZ"),
                ..SeasonInput::default()
            },
            &f,
            EditedBy::You,
        );
        assert_eq!(
            v.legal.empty.as_deref(),
            Some("No official pages are listed for XX-ZZ.")
        );
    }

    #[test]
    fn views_snapshot() {
        let f = formatter(us(), Voice::Luna);
        let (p, _) = edited_bass();
        let normals = croton_normals();
        insta::assert_json_snapshot!("profile_page", profile_page(&p, &f, EditedBy::You));
        insta::assert_json_snapshot!(
            "season_view",
            season_view(
                &p,
                &SeasonInput {
                    water_c: Some(16.0),
                    trend: Some(TrendDirection::Rising),
                    normals: &normals,
                    region: Some("US-NY"),
                    trout_stress_c: Some(20.0),
                    ..SeasonInput::default()
                },
                &f,
                EditedBy::You,
            )
        );
        insta::assert_json_snapshot!(
            "schemas",
            serde_json::json!({
                "SpeciesRow": schemars::schema_for!(SpeciesRow),
                "ProfilePage": schemars::schema_for!(ProfilePage),
                "SeasonView": schemars::schema_for!(SeasonView),
                "EditChoice": schemars::schema_for!(EditChoice),
            })
        );
    }
}
