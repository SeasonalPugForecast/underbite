//! The catch log's views: rows, a catch's detail with its conditions, and what worked.
//!
//! Every word and value is written here, in the user's units, voice and locale; the catch's own
//! time zone is used for its times. A catch's coordinates are private and appear in no text: a
//! place is shown by name only. The editor types its numbers as text, so they are parsed here,
//! with the locale's decimal mark ([`parse_length`], [`parse_weight`],
//! [`parse_pounds_ounces`]), and shown back the same way ([`edit_length`], [`edit_weight`],
//! [`edit_pounds_ounces`]).
//!
//! What worked counts the catches along each condition. The counts say when fish were caught,
//! never when fishing is best: the log has no record of the time spent fishing without a bite.
//! A condition known for fewer than [`MIN_CATCHES_FOR_PATTERN`] catches is not read at all.

use jiff::tz::TimeZone;
use schemars::JsonSchema;
use serde::Serialize;
use whether_core::presentation::TimeFormat;

use crate::log::LogBook;
use crate::log::entry::{
    CatchConditions, CatchEntry, CurrentSet, Fate, Light, PressureTrend, TideDirection, TurnKind,
};
use crate::log::insights::{
    Filter, FlowBin, Histogram, Insights, MIN_CATCHES_FOR_PATTERN, SolunarBin, TEMPERATURE_BIN_C,
    TemperatureBin,
};
use crate::model::SolunarKind;
use crate::provenance::SourceId;
use crate::species::catalogue::Catalogue;
use crate::time::{local_date, zone};
use crate::units::HeightUnit;
use crate::view::Voice;
use crate::view::format::Formatter;
use crate::view::locale::Locale;
use crate::view::strings::{Key, Strings};

// ---------------------------------------------------------------------------------------------
// Rows and detail
// ---------------------------------------------------------------------------------------------

/// What became of the fish. The plain voice words [`Self::KeptForSashimi`] as "Kept".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CatchFate {
    Released,
    Kept,
    KeptForSashimi,
}

impl From<Fate> for CatchFate {
    fn from(fate: Fate) -> Self {
        match fate {
            Fate::Released => Self::Released,
            Fate::Kept => Self::Kept,
            Fate::KeptForSashimi => Self::KeptForSashimi,
        }
    }
}

/// A saved spot's name, to put a name to the catches that carry its id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpotName {
    pub spot_id: String,
    pub name: String,
}

/// One catch in the list.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct CatchRow {
    pub id: String,
    /// The catalogue's name, else the angler's own words, else "Unknown species".
    pub species: String,
    /// "28.5 in · 9.5 lb", whichever of the two is known; none when neither is.
    pub size: Option<String>,
    /// "Nov 8 2026 · 6:10 AM" in the catch's own time zone.
    pub when: String,
    /// The saved spot's name, else the area the catch was filed under; never coordinates.
    pub place: Option<String>,
    pub fate: CatchFate,
    /// "Released", "Kept" or (in Luna's voice) "Kept for sashimi".
    pub fate_label: String,
    pub has_conditions: bool,
    /// The row in one plain line for `VoiceOver`.
    pub summary: String,
}

/// A text that says what to do when there is nothing to list.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, JsonSchema)]
pub struct EmptyState {
    pub title: String,
    pub body: String,
}

/// The log, newest catch first.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct CatchList {
    pub rows: Vec<CatchRow>,
    /// Set when there are no rows.
    pub empty: Option<EmptyState>,
}

/// The kinds of condition a catch's detail lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConditionKind {
    Tide,
    TideHeight,
    Current,
    Flow,
    GageHeight,
    WaterTemperature,
    Turbidity,
    Pressure,
    PressureTrend,
    Wind,
    AirTemperature,
    Moon,
    Solunar,
    Light,
}

/// One condition at the catch with where it came from.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct ConditionLine {
    pub kind: ConditionKind,
    pub label: String,
    pub value: String,
    /// The source, the station and how far the reading was from the catch, always plain:
    /// "NOAA CO-OPS · The Battery · 6 min before the catch".
    pub footnote: String,
}

/// A labelled text of the catch: lure, method, notes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, JsonSchema)]
pub struct DetailLine {
    pub label: String,
    pub value: String,
}

/// One catch in full.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct CatchDetail {
    pub row: CatchRow,
    /// Lure, method and notes, those that were filled in.
    pub details: Vec<DetailLine>,
    /// The names of the photos kept with the catch.
    pub photos: Vec<String>,
    pub conditions_title: String,
    pub conditions: Vec<ConditionLine>,
    /// Set when the catch has no conditions.
    pub no_conditions: Option<String>,
}

/// The formatter for one catch: its own time zone, the caller's units, voice, clock and locale.
fn formatter_for(f: &Formatter, time_zone: &str) -> Formatter {
    Formatter::new(f.units, f.strings.voice, f.clock, time_zone).with_locale(f.strings.locale)
}

fn zone_of(name: &str) -> TimeZone {
    zone(name).unwrap_or(TimeZone::UTC)
}

fn join(strings: Strings, parts: Vec<String>) -> String {
    let mut parts = parts.into_iter();
    let Some(first) = parts.next() else {
        return String::new();
    };
    parts.fold(first, |acc, next| {
        strings.fmt(Key::LogJoin, &[("first", &acc), ("second", &next)])
    })
}

fn lower_first(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |c| {
        c.to_lowercase().chain(chars).collect::<String>()
    })
}

fn species_name(strings: Strings, entry: &CatchEntry, catalogue: &Catalogue) -> String {
    let named = entry
        .species
        .as_ref()
        .and_then(|id| catalogue.get(id))
        .map(|p| {
            p.common_names
                .as_ref()
                .and_then(|n| n.value.first().cloned())
                .unwrap_or_else(|| p.scientific_name.value.clone())
        });
    named
        .or_else(|| entry.species_text.clone().filter(|t| !t.trim().is_empty()))
        .unwrap_or_else(|| strings.get(Key::LogSpeciesUnknown).to_owned())
}

fn place_name(entry: &CatchEntry, places: &[SpotName]) -> Option<String> {
    entry
        .location
        .spot_id
        .as_ref()
        .and_then(|id| places.iter().find(|p| &p.spot_id == id))
        .map(|p| p.name.clone())
        .or_else(|| entry.location.area.clone())
        .filter(|n| !n.trim().is_empty())
}

fn fate_label(strings: Strings, fate: Fate) -> &'static str {
    strings.get(match fate {
        Fate::Released => Key::LogReleased,
        Fate::Kept => Key::LogKept,
        Fate::KeptForSashimi => Key::LogKeptForSashimi,
    })
}

/// The units fish are measured in: inches and pounds where the user reads heights in feet,
/// centimetres and kilograms where they read them in metres.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
pub struct FishUnits {
    pub length: LengthUnit,
    pub weight: WeightUnit,
}

/// The units the editor's fields are in for this formatter.
#[must_use]
pub fn fish_units(f: &Formatter) -> FishUnits {
    match f.units.height {
        HeightUnit::Ft => FishUnits {
            length: LengthUnit::Inches,
            weight: WeightUnit::Pounds,
        },
        HeightUnit::M => FishUnits {
            length: LengthUnit::Centimetres,
            weight: WeightUnit::Kilograms,
        },
    }
}

fn length_text(f: &Formatter, metres: f32) -> String {
    let unit = fish_units(f).length;
    let value = length_from_metres(metres, unit);
    format!(
        "{} {}",
        f.strings.number(value, 1),
        f.strings.get(length_symbol(unit))
    )
}

fn weight_text(f: &Formatter, kg: f32) -> String {
    let (value, unit, decimals) = match fish_units(f).weight {
        // Under a pound, whole ounces read better.
        WeightUnit::Pounds if weight_from_kg(kg, WeightUnit::Ounces).round() < 16.0 => (
            weight_from_kg(kg, WeightUnit::Ounces),
            WeightUnit::Ounces,
            0,
        ),
        WeightUnit::Pounds => (
            weight_from_kg(kg, WeightUnit::Pounds),
            WeightUnit::Pounds,
            1,
        ),
        _ => (kg, WeightUnit::Kilograms, if kg < 1.0 { 2 } else { 1 }),
    };
    format!(
        "{} {}",
        f.strings.number(value, decimals),
        f.strings.get(weight_symbol(unit))
    )
}

/// One catch as a row.
#[must_use]
pub fn catch_row(
    entry: &CatchEntry,
    f: &Formatter,
    catalogue: &Catalogue,
    places: &[SpotName],
) -> CatchRow {
    let strings = f.strings;
    let ef = formatter_for(f, &entry.time_zone);
    let species = species_name(strings, entry, catalogue);
    let size = match (entry.length_m, entry.weight_kg) {
        (Some(l), Some(w)) => Some(strings.fmt(
            Key::LogSizeJoin,
            &[
                ("length", &length_text(f, l)),
                ("weight", &weight_text(f, w)),
            ],
        )),
        (Some(l), None) => Some(length_text(f, l)),
        (None, Some(w)) => Some(weight_text(f, w)),
        (None, None) => None,
    };
    let year = local_date(entry.caught_at, &zone_of(&entry.time_zone)).map_or(0, |d| d.year);
    let when = strings.fmt(
        Key::LogWhen,
        &[
            ("date", &ef.date_of(entry.caught_at)),
            ("year", &year.to_string()),
            ("time", &ef.clock(entry.caught_at)),
        ],
    );
    let place = place_name(entry, places);
    let fate_text = fate_label(strings, entry.fate).to_owned();
    let mut spoken: Vec<String> = size.iter().cloned().collect();
    spoken.push(when.clone());
    spoken.extend(place.iter().cloned());
    // The sashimi tag is Luna's: the spoken line says the plain meaning.
    spoken.push(
        Strings::localized(Voice::Plain, strings.locale)
            .get(match entry.fate {
                Fate::Released => Key::LogReleased,
                Fate::Kept | Fate::KeptForSashimi => Key::LogKept,
            })
            .to_owned(),
    );
    let summary = strings.fmt(
        Key::LogRowSummary,
        &[("species", &species), ("details", &join(strings, spoken))],
    );
    CatchRow {
        id: entry.id.clone(),
        species,
        size,
        when,
        place,
        fate: entry.fate.into(),
        fate_label: fate_text,
        has_conditions: entry.conditions.as_ref().is_some_and(|c| !c.is_empty()),
        summary,
    }
}

/// The live catches of the book, newest first.
#[must_use]
pub fn catch_list(
    book: &LogBook,
    f: &Formatter,
    catalogue: &Catalogue,
    places: &[SpotName],
) -> CatchList {
    let mut entries: Vec<&CatchEntry> = book.live().collect();
    entries.sort_by(|a, b| {
        b.caught_at
            .0
            .cmp(&a.caught_at.0)
            .then_with(|| a.id.cmp(&b.id))
    });
    let rows: Vec<CatchRow> = entries
        .into_iter()
        .map(|e| catch_row(e, f, catalogue, places))
        .collect();
    let empty = rows.is_empty().then(|| EmptyState {
        title: f.strings.get(Key::LogEmptyTitle).to_owned(),
        body: f.strings.get(Key::LogEmptyBody).to_owned(),
    });
    CatchList { rows, empty }
}

fn footnote(ef: &Formatter, source: &SourceId, station_name: Option<&str>, age_s: i64) -> String {
    let s = ef.strings;
    let mut parts = vec![s.source_name(source)];
    parts.extend(station_name.map(str::to_owned));
    parts.push(if age_s.abs() < 60 {
        s.get(Key::LogAgeNow).to_owned()
    } else if age_s > 0 {
        s.fmt(Key::LogAgeBefore, &[("age", &ef.duration(age_s))])
    } else {
        s.fmt(Key::LogAgeAfter, &[("age", &ef.duration(-age_s))])
    });
    parts.join(" · ")
}

fn hours_text(s: Strings, hours: f32) -> String {
    s.fmt(Key::DurationHours, &[("n", &s.number(hours, 1))])
}

fn turn_word(s: Strings, turn: TurnKind) -> &'static str {
    s.get(match turn {
        TurnKind::High => Key::LogHigh,
        TurnKind::Low => Key::LogLow,
    })
}

fn direction_word(s: Strings, d: TideDirection) -> &'static str {
    s.get(match d {
        TideDirection::Rising => Key::LogTideRising,
        TideDirection::Falling => Key::LogTideFalling,
    })
}

fn set_word(s: Strings, set: CurrentSet) -> &'static str {
    s.get(match set {
        CurrentSet::Flood => Key::Flood,
        CurrentSet::Ebb => Key::Ebb,
        CurrentSet::Slack => Key::Slack,
    })
}

fn trend_word(s: Strings, t: PressureTrend) -> &'static str {
    s.get(match t {
        PressureTrend::Rising => Key::Rising,
        PressureTrend::Falling => Key::Falling,
        PressureTrend::Steady => Key::Steady,
    })
}

fn light_word(s: Strings, l: Light) -> &'static str {
    s.get(match l {
        Light::Dawn => Key::LogLightDawn,
        Light::Day => Key::LogLightDay,
        Light::Dusk => Key::LogLightDusk,
        Light::Night => Key::LogLightNight,
    })
}

/// The conditions of a catch, in the order of the cards, in the catch's own zone and the user's
/// units.
#[allow(clippy::too_many_lines)]
fn condition_lines(c: &CatchConditions, entry: &CatchEntry, f: &Formatter) -> Vec<ConditionLine> {
    let ef = formatter_for(f, &entry.time_zone);
    let s = ef.strings;
    let date = local_date(c.at, &zone_of(&entry.time_zone)).ok();
    let mut out = Vec::new();
    let mut push_sourced = |kind: ConditionKind,
                            label: &str,
                            value: String,
                            src: &SourceId,
                            station: Option<&str>,
                            age_s: i64| {
        out.push(ConditionLine {
            kind,
            label: label.to_owned(),
            value,
            footnote: footnote(&ef, src, station, age_s),
        });
    };

    if let Some(t) = &c.tide {
        let tide = &t.value;
        let stage = s.fmt(
            Key::LogTideStage,
            &[
                ("direction", direction_word(s, tide.direction)),
                ("since", &hours_text(s, tide.hours_since_previous)),
                ("previous", turn_word(s, tide.previous)),
                ("to", &hours_text(s, tide.hours_to_next)),
                ("next", turn_word(s, tide.next)),
            ],
        );
        push_sourced(
            ConditionKind::Tide,
            s.get(Key::MeasTide),
            stage,
            &t.source,
            t.station_name.as_deref(),
            t.age_s,
        );
        if let Some(h) = tide.height {
            push_sourced(
                ConditionKind::TideHeight,
                s.get(Key::LogLabelTideHeight),
                ef.height(Some(h)).full(),
                &t.source,
                t.station_name.as_deref(),
                t.age_s,
            );
        }
    }
    if let Some(cur) = &c.current {
        let speed = ef.current_speed(Some(cur.value.speed_ms)).full();
        let mut value = match cur.value.set {
            Some(set) => s.fmt(
                Key::LogCurrentSet,
                &[("speed", &speed), ("set", &lower_first(set_word(s, set)))],
            ),
            None => speed,
        };
        if let Some(deg) = cur.value.toward_deg {
            value = s.fmt(
                Key::LogToward,
                &[("value", &value), ("direction", &ef.direction(Some(deg)))],
            );
        }
        push_sourced(
            ConditionKind::Current,
            s.get(Key::MeasTidalCurrent),
            value,
            &cur.source,
            cur.station_name.as_deref(),
            cur.age_s,
        );
    }
    if let Some(fl) = &c.flow {
        let flow = ef.discharge(Some(fl.value.discharge_m3s)).full();
        let percentile = date
            .and_then(|d| ef.percentile(fl.value.percentile_class, fl.value.percentile, false, d));
        let value = match percentile {
            Some(p) => s.fmt(Key::LogFlowValue, &[("flow", &flow), ("percentile", &p)]),
            None => flow,
        };
        push_sourced(
            ConditionKind::Flow,
            s.get(Key::MeasDischarge),
            value,
            &fl.source,
            fl.station_name.as_deref(),
            fl.age_s,
        );
    }
    if let Some(g) = &c.gage_height {
        push_sourced(
            ConditionKind::GageHeight,
            s.get(Key::MeasGageHeight),
            ef.stage(Some(g.value.height)).full(),
            &g.source,
            g.station_name.as_deref(),
            g.age_s,
        );
    }
    if let Some(w) = &c.water_temperature_c {
        push_sourced(
            ConditionKind::WaterTemperature,
            s.get(Key::MeasWaterTemperature),
            ef.temperature(Some(w.value)).full(),
            &w.source,
            w.station_name.as_deref(),
            w.age_s,
        );
    }
    if let Some(t) = &c.turbidity {
        push_sourced(
            ConditionKind::Turbidity,
            s.get(Key::MeasTurbidity),
            ef.turbidity(Some(t.value.value), t.value.unit).full(),
            &t.source,
            t.station_name.as_deref(),
            t.age_s,
        );
    }
    if let Some(p) = &c.pressure_hpa {
        push_sourced(
            ConditionKind::Pressure,
            s.get(Key::MeasPressure),
            ef.pressure(Some(p.value)).full(),
            &p.source,
            p.station_name.as_deref(),
            p.age_s,
        );
    }
    if let Some(t) = &c.pressure_trend {
        push_sourced(
            ConditionKind::PressureTrend,
            s.get(Key::LogLabelPressureTrend),
            trend_word(s, t.value).to_owned(),
            &t.source,
            t.station_name.as_deref(),
            t.age_s,
        );
    }
    if let Some(w) = &c.wind {
        let mut value = ef.wind_speed(Some(w.value.speed_ms)).full();
        if let Some(g) = w.value.gust_ms {
            value = s.fmt(
                Key::LogWindGust,
                &[("wind", &value), ("gust", &ef.wind_speed(Some(g)).full())],
            );
        }
        if let Some(deg) = w.value.from_deg {
            value = s.fmt(
                Key::LogWindFrom,
                &[("wind", &value), ("direction", &ef.direction(Some(deg)))],
            );
        }
        push_sourced(
            ConditionKind::Wind,
            s.get(Key::MeasWind),
            value,
            &w.source,
            w.station_name.as_deref(),
            w.age_s,
        );
    }
    if let Some(a) = &c.air_temperature_c {
        push_sourced(
            ConditionKind::AirTemperature,
            s.get(Key::LogLabelAirTemperature),
            ef.temperature(Some(a.value)).full(),
            &a.source,
            a.station_name.as_deref(),
            a.age_s,
        );
    }
    let computed = s.get(Key::LogComputed);
    if let Some(m) = &c.moon {
        out.push(ConditionLine {
            kind: ConditionKind::Moon,
            label: s.get(Key::LogLabelMoon).to_owned(),
            value: s.fmt(
                Key::LogMoonValue,
                &[
                    ("phase", s.moon_phase(m.phase)),
                    (
                        "lit",
                        &ef.percent(Some(m.illuminated_fraction * 100.0)).full(),
                    ),
                ],
            ),
            footnote: computed.to_owned(),
        });
    }
    if let Some(p) = &c.solunar {
        let label = s.get(match p.kind {
            SolunarKind::Major => Key::LogSolunarMajor,
            SolunarKind::Minor => Key::LogSolunarMinor,
        });
        out.push(ConditionLine {
            kind: ConditionKind::Solunar,
            label: s.get(Key::CardSolunar).to_owned(),
            value: s.fmt(
                Key::LogSolunarPeriod,
                &[("label", label), ("range", &ef.clock_range(p.start, p.end))],
            ),
            footnote: computed.to_owned(),
        });
    }
    if let Some(l) = c.light {
        out.push(ConditionLine {
            kind: ConditionKind::Light,
            label: s.get(Key::LogLabelLight).to_owned(),
            value: light_word(s, l).to_owned(),
            footnote: computed.to_owned(),
        });
    }
    out
}

/// One catch in full, its conditions each with its source.
#[must_use]
pub fn catch_detail(
    entry: &CatchEntry,
    f: &Formatter,
    catalogue: &Catalogue,
    places: &[SpotName],
) -> CatchDetail {
    let s = f.strings;
    let text = |t: &Option<String>| t.clone().filter(|t| !t.trim().is_empty());
    let mut details = Vec::new();
    for (key, value) in [
        (Key::LogLure, text(&entry.lure)),
        (Key::LogMethod, text(&entry.method)),
        (Key::LogNotes, text(&entry.notes)),
    ] {
        if let Some(value) = value {
            details.push(DetailLine {
                label: s.get(key).to_owned(),
                value,
            });
        }
    }
    let conditions = entry
        .conditions
        .as_ref()
        .map(|c| condition_lines(c, entry, f))
        .unwrap_or_default();
    let no_conditions = conditions
        .is_empty()
        .then(|| s.get(Key::LogNoConditions).to_owned());
    CatchDetail {
        row: catch_row(entry, f, catalogue, places),
        details,
        photos: entry.photos.iter().map(|p| p.as_str().to_owned()).collect(),
        conditions_title: s.get(Key::LogConditionsTitle).to_owned(),
        conditions,
        no_conditions,
    }
}

// ---------------------------------------------------------------------------------------------
// What worked
// ---------------------------------------------------------------------------------------------

/// The conditions the catches are counted along.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InsightKind {
    TideDirection,
    SinceTurn,
    Current,
    WaterTemperature,
    Flow,
    PressureTrend,
    MoonPhase,
    Solunar,
    Light,
    HourOfDay,
}

/// One value of a condition and how many catches had it.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct InsightBar {
    /// A stable id of the value: "rising", "high-2", "from-10", "hour-6".
    pub key: String,
    pub label: String,
    pub count: usize,
    /// The count against the largest bar of the section, 0 to 1, for drawing.
    pub fraction: f32,
    /// The one value with more catches than any other.
    pub peak: bool,
    /// "Rising tide: 12 of 20".
    pub spoken: String,
}

/// The catches counted along one condition.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct InsightSection {
    pub kind: InsightKind,
    pub title: String,
    /// Empty when too few catches have the condition recorded.
    pub bars: Vec<InsightBar>,
    /// "12 of 20 striped bass catches: rising tide". None when too few to tell.
    pub sentence: Option<String>,
    /// Says there are too few catches to tell.
    pub too_few: Option<String>,
    /// Says how many catches have no record of the condition.
    pub unknown: Option<String>,
}

/// What worked: sections of bars, and the notes that keep them honest.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct InsightsView {
    pub total: usize,
    /// "Catches counted: 20".
    pub headline: String,
    /// Set when there are no catches at all.
    pub empty: Option<String>,
    /// Set when there are some, but too few to tell anything.
    pub too_few: Option<String>,
    /// Counts say when fish were caught, not when fishing is best.
    pub notice: String,
    /// Empty unless there are at least [`MIN_CATCHES_FOR_PATTERN`] catches.
    pub sections: Vec<InsightSection>,
}

struct Ctx<'a> {
    f: &'a Formatter,
    subject: String,
}

fn snake<K: Serialize>(key: &K) -> String {
    serde_json::to_value(key)
        .ok()
        .and_then(|v| v.as_str().map(|s| s.replace('_', "-")))
        .unwrap_or_default()
}

fn section<K: Copy>(
    ctx: &Ctx<'_>,
    kind: InsightKind,
    title: &str,
    h: &Histogram<K>,
    id: impl Fn(K) -> String,
    label: impl Fn(K) -> String,
) -> InsightSection {
    let s = ctx.f.strings;
    let title = title.to_owned();
    if !h.enough() {
        return InsightSection {
            kind,
            title,
            bars: Vec::new(),
            sentence: None,
            too_few: Some(s.fmt(
                Key::LogSectionFew,
                &[
                    ("known", &h.known.to_string()),
                    ("min", &MIN_CATCHES_FOR_PATTERN.to_string()),
                ],
            )),
            unknown: None,
        };
    }
    let max = h.buckets.iter().map(|b| b.count).max().unwrap_or(0);
    let leaders = h.buckets.iter().filter(|b| b.count == max).count();
    let known = h.known.to_string();
    let mut sentence = None;
    let bars: Vec<InsightBar> = h
        .buckets
        .iter()
        .map(|b| {
            let text = label(b.key);
            let peak = max > 0 && leaders == 1 && b.count == max;
            if peak {
                sentence = Some(s.fmt(
                    Key::LogSentence,
                    &[
                        ("n", &b.count.to_string()),
                        ("known", &known),
                        ("subject", &ctx.subject),
                        ("value", &lower_first(&text)),
                    ],
                ));
            }
            InsightBar {
                key: id(b.key),
                spoken: s.fmt(
                    Key::LogBarSpoken,
                    &[
                        ("label", &text),
                        ("n", &b.count.to_string()),
                        ("known", &known),
                    ],
                ),
                label: text,
                count: b.count,
                fraction: if max == 0 {
                    0.0
                } else {
                    b.count as f32 / max as f32
                },
                peak,
            }
        })
        .collect();
    InsightSection {
        kind,
        title,
        bars,
        sentence: sentence.or_else(|| Some(s.get(Key::LogNoStandout).to_owned())),
        too_few: None,
        unknown: (h.unknown > 0).then(|| {
            s.fmt(
                Key::LogSectionUnknown,
                &[
                    ("unknown", &h.unknown.to_string()),
                    ("total", &h.total.to_string()),
                ],
            )
        }),
    }
}

fn hour_label(f: &Formatter, hour: u8) -> String {
    match f.clock {
        TimeFormat::H24 => format!("{hour:02}:00"),
        TimeFormat::H12 => {
            let h = match hour % 12 {
                0 => 12,
                h => h,
            };
            f.strings.twelve_hour(&h.to_string(), hour >= 12)
        }
    }
}

/// What worked, from [`crate::log::insights::summarise`] over the book and the same filter.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn insights_view(
    insights: &Insights,
    filter: &Filter,
    f: &Formatter,
    catalogue: &Catalogue,
) -> InsightsView {
    let s = f.strings;
    let subject = match filter.species.as_ref() {
        Some(id) => {
            let name = catalogue.get(id).map(|p| {
                p.common_names
                    .as_ref()
                    .and_then(|n| n.value.first().cloned())
                    .unwrap_or_else(|| p.scientific_name.value.clone())
            });
            match name {
                Some(name) => s.fmt(Key::LogSubjectSpecies, &[("species", &name)]),
                None => s.get(Key::LogSubjectCatches).to_owned(),
            }
        }
        None => s.get(Key::LogSubjectCatches).to_owned(),
    };
    let mut view = InsightsView {
        total: insights.total,
        headline: s.fmt(Key::LogInsightsTotal, &[("n", &insights.total.to_string())]),
        empty: (insights.total == 0).then(|| s.get(Key::LogInsightsEmpty).to_owned()),
        too_few: (insights.total > 0 && !insights.enough()).then(|| {
            s.fmt(
                Key::LogInsightsFew,
                &[
                    ("n", &insights.total.to_string()),
                    ("min", &MIN_CATCHES_FOR_PATTERN.to_string()),
                ],
            )
        }),
        notice: s.get(Key::LogInsightsNote).to_owned(),
        sections: Vec::new(),
    };
    if !insights.enough() {
        return view;
    }
    let ctx = Ctx { f, subject };
    let t = |k: Key| s.get(k);
    view.sections = vec![
        section(
            &ctx,
            InsightKind::TideDirection,
            t(Key::LogSectionTide),
            &insights.tide_direction,
            |d| snake(&d),
            |d| direction_word(s, d).to_owned(),
        ),
        section(
            &ctx,
            InsightKind::SinceTurn,
            t(Key::LogSectionSinceTurn),
            &insights.since_turn,
            |b| format!("{}-{}", snake(&b.turn), b.hours),
            |b| {
                let last = insights
                    .since_turn
                    .buckets
                    .iter()
                    .filter(|x| x.key.turn == b.turn)
                    .map(|x| x.key.hours)
                    .max()
                    == Some(b.hours);
                let turn = turn_word(s, b.turn);
                if last {
                    s.fmt(
                        Key::LogHoursAfterLast,
                        &[("from", &b.hours.to_string()), ("turn", turn)],
                    )
                } else {
                    s.fmt(
                        Key::LogHoursAfter,
                        &[
                            ("from", &b.hours.to_string()),
                            ("to", &(u16::from(b.hours) + 1).to_string()),
                            ("turn", turn),
                        ],
                    )
                }
            },
        ),
        section(
            &ctx,
            InsightKind::Current,
            t(Key::MeasTidalCurrent),
            &insights.current,
            |c| snake(&c),
            |c| set_word(s, c).to_owned(),
        ),
        section(
            &ctx,
            InsightKind::WaterTemperature,
            t(Key::MeasWaterTemperature),
            &insights.water_temperature,
            |b: TemperatureBin| format!("from-{}", b.from_c),
            |b: TemperatureBin| {
                let low = f.temperature(Some(b.from_c as f32));
                let high = f.temperature(Some((b.from_c + TEMPERATURE_BIN_C) as f32));
                s.fmt(
                    Key::LogRange,
                    &[("from", &low.text), ("to", &high.text), ("unit", &low.unit)],
                )
            },
        ),
        section(
            &ctx,
            InsightKind::Flow,
            t(Key::MeasDischarge),
            &insights.flow,
            |b| snake(&b),
            |b| {
                s.get(match b {
                    FlowBin::Under10 => Key::LogFlowUnder10,
                    FlowBin::From10To25 => Key::LogFlow10To25,
                    FlowBin::From25To75 => Key::LogFlow25To75,
                    FlowBin::From75To90 => Key::LogFlow75To90,
                    FlowBin::Over90 => Key::LogFlowOver90,
                })
                .to_owned()
            },
        ),
        section(
            &ctx,
            InsightKind::PressureTrend,
            t(Key::LogLabelPressureTrend),
            &insights.pressure_trend,
            |b| snake(&b),
            |b| trend_word(s, b).to_owned(),
        ),
        section(
            &ctx,
            InsightKind::MoonPhase,
            t(Key::LogLabelMoon),
            &insights.moon_phase,
            |b| snake(&b),
            |b| s.moon_phase(b).to_owned(),
        ),
        section(
            &ctx,
            InsightKind::Solunar,
            t(Key::CardSolunar),
            &insights.solunar,
            |b| snake(&b),
            |b| {
                s.get(match b {
                    SolunarBin::Major => Key::LogSolunarMajor,
                    SolunarBin::Minor => Key::LogSolunarMinor,
                    SolunarBin::Outside => Key::LogSolunarOutside,
                })
                .to_owned()
            },
        ),
        section(
            &ctx,
            InsightKind::Light,
            t(Key::LogLabelLight),
            &insights.light,
            |b| snake(&b),
            |b| light_word(s, b).to_owned(),
        ),
        section(
            &ctx,
            InsightKind::HourOfDay,
            t(Key::LogSectionHour),
            &insights.hour_of_day,
            |h| format!("hour-{h}"),
            |h| hour_label(f, h),
        ),
    ];
    view
}

// ---------------------------------------------------------------------------------------------
// The editor's numbers
// ---------------------------------------------------------------------------------------------

/// The unit a length is typed in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LengthUnit {
    Inches,
    Centimetres,
}

/// The unit a weight is typed in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WeightUnit {
    Pounds,
    Ounces,
    Kilograms,
    Grams,
}

/// Why typed text is not a size.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    #[error("not a number")]
    NotANumber,
    #[error("zero or less")]
    NotPositive,
    #[error("too large")]
    TooLarge,
    #[error("pounds must be whole")]
    PoundsNotWhole,
    #[error("ounces must be under 16")]
    OuncesTooMany,
}

impl ParseError {
    /// The message for the field, in the user's voice and locale.
    #[must_use]
    pub fn text(self, strings: Strings) -> String {
        match self {
            Self::NotANumber => strings.fmt(
                Key::LogParseNumber,
                &[("example", &strings.number(12.5, 1))],
            ),
            Self::NotPositive => strings.get(Key::LogParseNotPositive).to_owned(),
            Self::TooLarge => strings.get(Key::LogParseTooLarge).to_owned(),
            Self::PoundsNotWhole => strings.get(Key::LogParsePounds).to_owned(),
            Self::OuncesTooMany => strings.get(Key::LogParseOunces).to_owned(),
        }
    }
}

const M_PER_IN: f64 = 0.0254;
const KG_PER_LB: f64 = 0.453_592_37;
const KG_PER_OZ: f64 = KG_PER_LB / 16.0;
/// No fish is longer than this.
const MAX_LENGTH_M: f64 = 5.0;
/// No fish is heavier than this.
const MAX_WEIGHT_KG: f64 = 1_000.0;

fn length_from_metres(metres: f32, unit: LengthUnit) -> f32 {
    let m = f64::from(metres);
    (match unit {
        LengthUnit::Inches => m / M_PER_IN,
        LengthUnit::Centimetres => m * 100.0,
    }) as f32
}

fn weight_from_kg(kg: f32, unit: WeightUnit) -> f32 {
    let kg = f64::from(kg);
    (match unit {
        WeightUnit::Pounds => kg / KG_PER_LB,
        WeightUnit::Ounces => kg / KG_PER_OZ,
        WeightUnit::Kilograms => kg,
        WeightUnit::Grams => kg * 1_000.0,
    }) as f32
}

const fn length_symbol(unit: LengthUnit) -> Key {
    match unit {
        LengthUnit::Inches => Key::UnitInches,
        LengthUnit::Centimetres => Key::UnitCentimetres,
    }
}

const fn weight_symbol(unit: WeightUnit) -> Key {
    match unit {
        WeightUnit::Pounds => Key::UnitPounds,
        WeightUnit::Ounces => Key::UnitOunces,
        WeightUnit::Kilograms => Key::UnitKilograms,
        WeightUnit::Grams => Key::UnitGrams,
    }
}

/// A non-negative number written with the locale's decimal mark and digits only. Blank is `None`.
fn parse_number(text: &str, locale: Locale) -> Result<Option<f64>, ParseError> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let mark = locale.decimal_mark();
    let mut normalised = String::with_capacity(text.len());
    let (mut digits, mut marked) = (false, false);
    for c in text.chars() {
        match c {
            '0'..='9' => {
                digits = true;
                normalised.push(c);
            }
            c if c == mark && !marked => {
                marked = true;
                normalised.push('.');
            }
            _ => return Err(ParseError::NotANumber),
        }
    }
    if !digits {
        return Err(ParseError::NotANumber);
    }
    normalised
        .parse::<f64>()
        .map(Some)
        .map_err(|_| ParseError::NotANumber)
}

fn checked(value: f64, max: f64) -> Result<f32, ParseError> {
    if !value.is_finite() || value > max {
        Err(ParseError::TooLarge)
    } else if value <= 0.0 {
        Err(ParseError::NotPositive)
    } else {
        Ok(value as f32)
    }
}

/// A length typed in `unit`, in metres; blank text is no value.
///
/// # Errors
/// [`ParseError`] for anything but digits with at most one decimal mark of the `locale`, for zero
/// and for a length no fish has.
pub fn parse_length(
    text: &str,
    unit: LengthUnit,
    locale: Locale,
) -> Result<Option<f32>, ParseError> {
    parse_number(text, locale)?
        .map(|v| {
            checked(
                match unit {
                    LengthUnit::Inches => v * M_PER_IN,
                    LengthUnit::Centimetres => v / 100.0,
                },
                MAX_LENGTH_M,
            )
        })
        .transpose()
}

/// A weight typed in `unit`, in kilograms; blank text is no value.
///
/// # Errors
/// As [`parse_length`].
pub fn parse_weight(
    text: &str,
    unit: WeightUnit,
    locale: Locale,
) -> Result<Option<f32>, ParseError> {
    parse_number(text, locale)?
        .map(|v| {
            checked(
                match unit {
                    WeightUnit::Pounds => v * KG_PER_LB,
                    WeightUnit::Ounces => v * KG_PER_OZ,
                    WeightUnit::Kilograms => v,
                    WeightUnit::Grams => v / 1_000.0,
                },
                MAX_WEIGHT_KG,
            )
        })
        .transpose()
}

/// A weight typed as whole pounds and ounces in two fields, in kilograms. Either field may be
/// blank; both blank is no value.
///
/// # Errors
/// As [`parse_length`], and [`ParseError::PoundsNotWhole`] for a fraction of a pound,
/// [`ParseError::OuncesTooMany`] for 16 ounces or more.
pub fn parse_pounds_ounces(
    pounds: &str,
    ounces: &str,
    locale: Locale,
) -> Result<Option<f32>, ParseError> {
    let lb = parse_number(pounds, locale)?;
    let oz = parse_number(ounces, locale)?;
    if lb.is_none() && oz.is_none() {
        return Ok(None);
    }
    let lb = lb.unwrap_or(0.0);
    let oz = oz.unwrap_or(0.0);
    if lb.fract() != 0.0 {
        return Err(ParseError::PoundsNotWhole);
    }
    if oz >= 16.0 {
        return Err(ParseError::OuncesTooMany);
    }
    checked(lb * KG_PER_LB + oz * KG_PER_OZ, MAX_WEIGHT_KG).map(Some)
}

/// A number for a field: at most `decimals` places, no trailing zeros, the locale's mark.
fn field_number(value: f32, decimals: u8, locale: Locale) -> String {
    let text = locale.number(value, decimals);
    let mark = locale.decimal_mark();
    if text.contains(mark) {
        text.trim_end_matches('0').trim_end_matches(mark).to_owned()
    } else {
        text
    }
}

/// A length for the editor's field, in `unit`: "28.5".
#[must_use]
pub fn edit_length(metres: f32, unit: LengthUnit, locale: Locale) -> String {
    field_number(length_from_metres(metres, unit), 1, locale)
}

/// A weight for the editor's field, in `unit`: "9.5".
#[must_use]
pub fn edit_weight(kg: f32, unit: WeightUnit, locale: Locale) -> String {
    field_number(weight_from_kg(kg, unit), 2, locale)
}

/// The two fields of a weight in pounds and ounces.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, JsonSchema)]
pub struct PoundsOunces {
    pub pounds: String,
    pub ounces: String,
}

/// A weight for the editor's two fields: whole pounds and the ounces to a tenth.
#[must_use]
pub fn edit_pounds_ounces(kg: f32, locale: Locale) -> PoundsOunces {
    let tenths = (f64::from(kg) / KG_PER_OZ * 10.0).round();
    let total_oz = tenths / 10.0;
    let pounds = (total_oz / 16.0).floor();
    let ounces = total_oz - pounds * 16.0;
    PoundsOunces {
        pounds: if pounds > 0.0 {
            format!("{pounds:.0}")
        } else {
            String::new()
        },
        ounces: field_number(ounces as f32, 1, locale),
    }
}

#[cfg(test)]
pub(in crate::view) mod tests {
    #![allow(clippy::assert_is_empty)]

    use super::*;
    use crate::datum::{Height, VerticalDatum};
    use crate::log::entry::{
        CatchLocation, CurrentCondition, FlowCondition, GageCondition, MoonCondition,
        SolunarCondition, Sourced, TideCondition, WindCondition,
    };
    use crate::log::insights::summarise;
    use crate::model::{MoonPhase, PercentileClass, SolunarCause, Turbidity, TurbidityUnit};
    use crate::time::UnixTime;
    use crate::trip::SpeciesId;
    use crate::view::cards::flow::fixtures::{metric, us};

    const AGENCY: &str = include_str!("../../data/species/synthetic/agency.synthetic.json");

    /// 2026-11-08 06:10 in New York.
    pub(in crate::view) const CAUGHT: i64 = 1_794_136_200;
    const BAIT: &str = "Bucktail jig";

    pub(in crate::view) fn catalogue() -> Catalogue {
        Catalogue::from_files(&[("agency.synthetic.json", AGENCY)], &[]).unwrap()
    }

    pub(in crate::view) fn formatter(units: crate::view::DisplayUnits, voice: Voice) -> Formatter {
        Formatter::new(units, voice, TimeFormat::H12, "America/New_York")
    }

    pub(in crate::view) fn places() -> Vec<SpotName> {
        vec![SpotName {
            spot_id: "battery".into(),
            name: "The Battery".into(),
        }]
    }

    fn sourced<T>(value: T, station: Option<&str>, age_s: i64) -> Sourced<T> {
        Sourced {
            value,
            source: SourceId::COOPS,
            station: station.map(|_| "8518750".to_owned()),
            station_name: station.map(str::to_owned),
            age_s,
        }
    }

    fn base(id: &str, caught: i64) -> CatchEntry {
        let mut location = CatchLocation::new(40.7003, -74.0142);
        location.spot_id = Some("battery".into());
        let mut e = CatchEntry::new(
            id,
            UnixTime(caught),
            "America/New_York",
            location,
            UnixTime(caught),
        );
        e.species = Some(SpeciesId::new("morone-saxatilis").unwrap());
        e
    }

    /// A striped bass with every condition.
    pub(in crate::view) fn full_entry() -> CatchEntry {
        let mut e = base("c1", CAUGHT);
        e.length_m = Some(0.7239);
        e.weight_kg = Some(4.31);
        e.fate = Fate::KeptForSashimi;
        e.lure = Some(BAIT.into());
        e.method = Some("Casting from the seawall".into());
        e.notes = Some("Second fish of the morning".into());
        e.photos = vec![crate::log::entry::PhotoRef::new("c1.jpg").unwrap()];
        e.conditions = Some(CatchConditions {
            at: UnixTime(CAUGHT),
            tide: Some(sourced(
                TideCondition {
                    direction: TideDirection::Rising,
                    previous: TurnKind::Low,
                    hours_since_previous: 2.5,
                    next: TurnKind::High,
                    hours_to_next: 3.7,
                    height: Some(Height {
                        datum: VerticalDatum::Mllw,
                        metres: 0.85,
                    }),
                },
                Some("The Battery"),
                240,
            )),
            current: Some(sourced(
                CurrentCondition {
                    speed_ms: 0.72,
                    set: Some(CurrentSet::Flood),
                    toward_deg: Some(45.0),
                },
                Some("Hudson River entrance"),
                -1_800,
            )),
            flow: Some(Sourced {
                value: FlowCondition {
                    discharge_m3s: 34.0,
                    percentile: Some(45.0),
                    percentile_class: Some(PercentileClass::Normal),
                },
                source: SourceId::USGS,
                station: Some("01375000".into()),
                station_name: Some("Croton River".into()),
                age_s: 600,
            }),
            gage_height: Some(sourced(
                GageCondition {
                    height: Height {
                        datum: VerticalDatum::Navd88,
                        metres: 1.2,
                    },
                },
                Some("Croton River"),
                0,
            )),
            water_temperature_c: Some(sourced(12.3, Some("The Battery"), 360)),
            turbidity: Some(sourced(
                Turbidity {
                    value: 4.2,
                    unit: TurbidityUnit::Fnu,
                },
                Some("Croton River"),
                900,
            )),
            pressure_hpa: Some(sourced(1017.6, Some("The Battery"), 360)),
            pressure_trend: Some(sourced(PressureTrend::Falling, Some("The Battery"), 360)),
            wind: Some(sourced(
                WindCondition {
                    speed_ms: 5.1,
                    gust_ms: Some(8.2),
                    from_deg: Some(225.0),
                },
                None,
                7_200,
            )),
            air_temperature_c: Some(sourced(9.0, None, 7_200)),
            moon: Some(MoonCondition {
                phase: MoonPhase::WaxingCrescent,
                illuminated_fraction: 0.27,
                elongation_deg: 50.0,
            }),
            solunar: Some(SolunarCondition {
                kind: SolunarKind::Major,
                cause: SolunarCause::LowerTransit,
                start: UnixTime(CAUGHT - 1_800),
                end: UnixTime(CAUGHT + 5_400),
            }),
            light: Some(Light::Dawn),
        });
        e
    }

    /// A catch with no conditions and a sparse record: the angler's own words for the fish.
    pub(in crate::view) fn bare_entry() -> CatchEntry {
        let mut e = base("c0", CAUGHT - 86_400 * 400);
        e.species = None;
        e.species_text = Some("Hickory shad".into());
        e.weight_kg = Some(0.45);
        e.location.spot_id = None;
        e.location.area = Some("Lower Hudson".into());
        e
    }

    /// Twenty striped bass: 12 on a rising tide, 6 on a falling one; two without conditions; and
    /// one perch.
    pub(in crate::view) fn bass_book() -> LogBook {
        let mut book = LogBook::new();
        for i in 0..20 {
            let mut e = base(&format!("b{i:02}"), CAUGHT + i64::from(i) * 3_600);
            if i < 18 {
                e.conditions = Some(CatchConditions {
                    at: e.caught_at,
                    tide: Some(sourced(
                        TideCondition {
                            direction: if i < 12 {
                                TideDirection::Rising
                            } else {
                                TideDirection::Falling
                            },
                            previous: TurnKind::Low,
                            hours_since_previous: (i % 4) as f32 + 0.5,
                            next: TurnKind::High,
                            hours_to_next: 2.0,
                            height: None,
                        },
                        Some("The Battery"),
                        0,
                    )),
                    water_temperature_c: Some(sourced(10.0 + (i % 6) as f32, None, 0)),
                    light: Some(if i % 3 == 0 { Light::Dawn } else { Light::Day }),
                    ..CatchConditions::default()
                });
            }
            book.upsert(e);
        }
        let mut other = base("w1", CAUGHT);
        other.species = None;
        other.species_text = Some("Perch".into());
        book.upsert(other);
        book
    }

    fn all_text(value: &impl Serialize) -> String {
        serde_json::to_string_pretty(value).unwrap()
    }

    #[test]
    fn a_full_catch_in_both_voices_and_unit_systems() {
        let c = catalogue();
        let e = full_entry();
        insta::assert_snapshot!(
            "detail_us_luna",
            all_text(&catch_detail(
                &e,
                &formatter(us(), Voice::Luna),
                &c,
                &places()
            ))
        );
        insta::assert_snapshot!(
            "detail_metric_plain",
            all_text(&catch_detail(
                &e,
                &formatter(metric(), Voice::Plain),
                &c,
                &places()
            ))
        );
    }

    #[test]
    fn a_catch_without_conditions() {
        let c = catalogue();
        let e = bare_entry();
        let luna = catch_detail(&e, &formatter(us(), Voice::Luna), &c, &places());
        assert!(luna.conditions.is_empty());
        assert!(luna.no_conditions.is_some());
        assert!(!luna.row.has_conditions);
        insta::assert_snapshot!("bare_us_luna", all_text(&luna));
        insta::assert_snapshot!(
            "bare_metric_plain",
            all_text(&catch_detail(
                &e,
                &formatter(metric(), Voice::Plain),
                &c,
                &places()
            ))
        );
    }

    #[test]
    fn sashimi_is_lunas_and_the_spoken_line_is_plain() {
        let c = catalogue();
        let e = full_entry();
        let luna = catch_row(&e, &formatter(us(), Voice::Luna), &c, &places());
        let plain = catch_row(&e, &formatter(us(), Voice::Plain), &c, &places());
        assert_eq!(luna.fate_label, "Kept for sashimi");
        assert_eq!(plain.fate_label, "Kept");
        assert_eq!(luna.fate, CatchFate::KeptForSashimi);
        assert!(luna.summary.ends_with(", Kept"), "{}", luna.summary);
        assert!(!luna.summary.contains("sashimi"));
    }

    #[test]
    fn times_are_in_the_catchs_zone_and_places_are_names() {
        let c = catalogue();
        let mut e = full_entry();
        e.time_zone = "America/Los_Angeles".into();
        let row = catch_row(&e, &formatter(us(), Voice::Plain), &c, &places());
        assert_eq!(row.when, "Nov 8 2026 · 3:10 AM");
        assert_eq!(row.place.as_deref(), Some("The Battery"));
        assert_eq!(row.size.as_deref(), Some("28.5 in · 9.5 lb"));
        let metric = catch_row(&e, &formatter(metric(), Voice::Plain), &c, &places());
        assert_eq!(metric.size.as_deref(), Some("72.4 cm · 4.3 kg"));
        // No coordinates in anything shown.
        let json = all_text(&catch_detail(&e, &formatter(us(), Voice::Luna), &c, &[]));
        assert!(!json.contains("40.70") && !json.contains("74.01"), "{json}");
        assert_eq!(
            catch_row(&e, &formatter(us(), Voice::Plain), &c, &[]).place,
            None
        );
    }

    #[test]
    fn the_list_is_newest_first_and_says_what_to_do_when_empty() {
        let c = catalogue();
        let mut book = LogBook::new();
        let empty = catch_list(&book, &formatter(us(), Voice::Luna), &c, &places());
        assert!(empty.rows.is_empty());
        insta::assert_snapshot!("empty_luna", all_text(&empty));
        let plain = catch_list(&book, &formatter(us(), Voice::Plain), &c, &places());
        insta::assert_snapshot!("empty_plain", all_text(&plain));
        book.upsert(bare_entry());
        book.upsert(full_entry());
        book.delete("c0", UnixTime(CAUGHT));
        let list = catch_list(&book, &formatter(us(), Voice::Plain), &c, &places());
        assert_eq!(list.rows.len(), 1);
        assert!(list.empty.is_none());
        let mut book = LogBook::new();
        book.upsert(bare_entry());
        book.upsert(full_entry());
        let list = catch_list(&book, &formatter(us(), Voice::Plain), &c, &places());
        assert_eq!(
            list.rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            ["c1", "c0"]
        );
    }

    fn insights_for(
        book: &LogBook,
        filter: &Filter,
        units: crate::view::DisplayUnits,
        voice: Voice,
    ) -> InsightsView {
        insights_view(
            &summarise(book, filter),
            filter,
            &formatter(units, voice),
            &catalogue(),
        )
    }

    #[test]
    fn insights_in_both_voices_and_unit_systems() {
        let book = bass_book();
        let filter = Filter {
            species: Some(SpeciesId::new("morone-saxatilis").unwrap()),
            ..Filter::default()
        };
        let v = insights_for(&book, &filter, us(), Voice::Luna);
        assert_eq!(v.total, 20);
        let tide = &v.sections[0];
        assert_eq!(
            tide.sentence.as_deref(),
            Some("12 of 18 Striped bass catches: rising tide")
        );
        assert_eq!(
            tide.unknown.as_deref(),
            Some("2 of 20 catches have no record of this.")
        );
        insta::assert_snapshot!("insights_us_luna", all_text(&v));
        insta::assert_snapshot!(
            "insights_metric_plain",
            all_text(&insights_for(&book, &filter, metric(), Voice::Plain))
        );
    }

    #[test]
    fn too_few_catches_say_so_and_never_say_when_fishing_is_best() {
        let mut book = LogBook::new();
        let none = insights_for(&book, &Filter::default(), us(), Voice::Luna);
        assert!(none.empty.is_some() && none.sections.is_empty());
        insta::assert_snapshot!("insights_empty_luna", all_text(&none));
        insta::assert_snapshot!(
            "insights_empty_plain",
            all_text(&insights_for(&book, &Filter::default(), us(), Voice::Plain))
        );
        for i in 0..4 {
            book.upsert(base(&format!("f{i}"), CAUGHT + i64::from(i)));
        }
        let few = insights_for(&book, &Filter::default(), us(), Voice::Plain);
        assert_eq!(
            few.too_few.as_deref(),
            Some("Too few catches to tell: 4 logged, 10 needed.")
        );
        assert!(few.sections.is_empty() && few.empty.is_none());
        // Ten catches, but only one with conditions: the section says it is too few.
        for i in 4..10 {
            book.upsert(base(&format!("f{i}"), CAUGHT + i64::from(i)));
        }
        let mut e = base("g1", CAUGHT);
        e.conditions = full_entry().conditions;
        book.upsert(e);
        let v = insights_for(&book, &Filter::default(), us(), Voice::Plain);
        let tide = &v.sections[0];
        assert!(tide.bars.is_empty() && tide.sentence.is_none());
        assert_eq!(
            tide.too_few.as_deref(),
            Some("Too few catches with this recorded to tell: 1 of the 10 needed.")
        );
        for view in [&none, &few, &v] {
            assert!(view.notice.contains("not when fishing is best"));
            let text = all_text(view).to_lowercase();
            assert!(
                !text.contains("best time") && !text.contains("prime"),
                "{text}"
            );
        }
    }

    #[test]
    fn a_tie_does_not_pick_a_winner() {
        let mut book = LogBook::new();
        for i in 0..10 {
            let mut e = base(&format!("t{i}"), CAUGHT);
            e.conditions = Some(CatchConditions {
                at: e.caught_at,
                light: Some(if i < 5 { Light::Dawn } else { Light::Day }),
                ..CatchConditions::default()
            });
            book.upsert(e);
        }
        let v = insights_for(&book, &Filter::default(), us(), Voice::Plain);
        let light = v
            .sections
            .iter()
            .find(|s| s.kind == InsightKind::Light)
            .unwrap();
        assert!(light.bars.iter().all(|b| !b.peak));
        assert_eq!(
            light.sentence.as_deref(),
            Some("No single value stands out: the catches are spread evenly.")
        );
    }

    fn comma() -> Locale {
        Locale {
            numbers: crate::view::locale::NumberStyle::Comma,
            ..Locale::ENGLISH
        }
    }

    #[test]
    fn typed_numbers_use_the_locales_decimal_mark() {
        let en = Locale::ENGLISH;
        let comma = comma();
        let cm = |t: &str, l| parse_length(t, LengthUnit::Centimetres, l);
        assert_eq!(cm("12.5", en), Ok(Some(0.125)));
        assert_eq!(cm("12,5", comma), Ok(Some(0.125)));
        // The other locale's mark is not guessed at.
        assert_eq!(cm("12,5", en), Err(ParseError::NotANumber));
        assert_eq!(cm("12.5", comma), Err(ParseError::NotANumber));
        assert_eq!(cm(" 30 ", en), Ok(Some(0.3)));
        assert_eq!(cm("", en), Ok(None));
        assert_eq!(cm("  ", en), Ok(None));
        for bad in [
            "abc", "1e3", "-4", "+4", "1.2.3", ".", "1 000", "inf", "NaN", "٣",
        ] {
            assert_eq!(cm(bad, en), Err(ParseError::NotANumber), "{bad}");
        }
        assert_eq!(cm("0", en), Err(ParseError::NotPositive));
        assert_eq!(cm("0.0", en), Err(ParseError::NotPositive));
        assert_eq!(cm("100000", en), Err(ParseError::TooLarge));
        assert_eq!(cm(&"9".repeat(400), en), Err(ParseError::TooLarge));
        let inches = parse_length("28.5", LengthUnit::Inches, en)
            .unwrap()
            .unwrap();
        assert!((inches - 0.7239).abs() < 1e-6);
        let kg = parse_weight("9.5", WeightUnit::Pounds, en)
            .unwrap()
            .unwrap();
        assert!((kg - 4.3091).abs() < 1e-3);
        assert_eq!(parse_weight("450", WeightUnit::Grams, en), Ok(Some(0.45)));
        assert_eq!(
            parse_weight("2,5", WeightUnit::Kilograms, comma),
            Ok(Some(2.5))
        );
    }

    #[test]
    fn pounds_and_ounces_are_two_fields() {
        let en = Locale::ENGLISH;
        let kg = parse_pounds_ounces("3", "4", en).unwrap().unwrap();
        assert!((kg - 1.4742).abs() < 1e-3, "{kg}");
        assert_eq!(parse_pounds_ounces("", "", en), Ok(None));
        let oz_only = parse_pounds_ounces("", "8", en).unwrap().unwrap();
        assert!((oz_only - 0.2268).abs() < 1e-3);
        assert_eq!(
            parse_pounds_ounces("2.5", "", en),
            Err(ParseError::PoundsNotWhole)
        );
        assert_eq!(
            parse_pounds_ounces("2", "16", en),
            Err(ParseError::OuncesTooMany)
        );
        assert_eq!(
            parse_pounds_ounces("0", "0", en),
            Err(ParseError::NotPositive)
        );
        // "3 lb 4 oz" is not text this parses.
        assert_eq!(
            parse_pounds_ounces("3 lb 4 oz", "", en),
            Err(ParseError::NotANumber)
        );
    }

    #[test]
    fn the_editor_shows_back_what_it_parsed() {
        let en = Locale::ENGLISH;
        let comma = comma();
        assert_eq!(edit_length(0.7239, LengthUnit::Inches, en), "28.5");
        assert_eq!(edit_length(0.5, LengthUnit::Centimetres, comma), "50");
        assert_eq!(edit_length(0.455, LengthUnit::Centimetres, comma), "45,5");
        assert_eq!(edit_weight(4.31, WeightUnit::Kilograms, comma), "4,31");
        assert_eq!(edit_weight(0.453_592_37, WeightUnit::Pounds, en), "1");
        let po = edit_pounds_ounces(1.4742, en);
        assert_eq!((po.pounds.as_str(), po.ounces.as_str()), ("3", "4"));
        let po = edit_pounds_ounces(0.2268, comma);
        assert_eq!((po.pounds.as_str(), po.ounces.as_str()), ("", "8"));
        // A rounding that reaches 16 ounces carries into the pounds.
        let po = edit_pounds_ounces(0.453_592_37 - 0.000_2, en);
        assert_eq!((po.pounds.as_str(), po.ounces.as_str()), ("1", "0"));
        for kg in [0.07_f32, 0.45, 1.0, 4.31, 12.7] {
            let po = edit_pounds_ounces(kg, en);
            let back = parse_pounds_ounces(&po.pounds, &po.ounces, en)
                .unwrap()
                .unwrap();
            assert!((back - kg).abs() < 0.002, "{kg} {back}");
        }
        assert_eq!(
            fish_units(&formatter(us(), Voice::Plain)).weight,
            WeightUnit::Pounds
        );
        assert_eq!(
            fish_units(&formatter(metric(), Voice::Plain)).length,
            LengthUnit::Centimetres
        );
    }

    #[test]
    fn parse_errors_have_words_in_the_locale() {
        let s = Strings::new(Voice::Plain);
        assert_eq!(
            ParseError::NotANumber.text(s),
            "Enter a number, such as 12.5."
        );
        let s = Strings::localized(Voice::Plain, comma());
        assert_eq!(
            ParseError::NotANumber.text(s),
            "Enter a number, such as 12,5."
        );
    }

    #[test]
    fn schema_snapshot() {
        let schema = serde_json::json!({
            "catch_list": schemars::schema_for!(CatchList),
            "catch_detail": schemars::schema_for!(CatchDetail),
            "insights_view": schemars::schema_for!(InsightsView),
            "fish_units": schemars::schema_for!(FishUnits),
            "pounds_ounces": schemars::schema_for!(PoundsOunces),
        });
        insta::assert_json_snapshot!(schema);
    }
}
