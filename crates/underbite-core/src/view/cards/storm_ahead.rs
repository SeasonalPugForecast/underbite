//! A storm or hurricane ahead of the spot.
//!
//! Safety text: the card has no Luna variant (nothing here takes a voice for its words) and is
//! never shortened. It is built from whether's storm analysis (events, thresholds, hurricane cone
//! windows) and every value is expressed again through Underbite's formatter and units.

use schemars::JsonSchema;
use serde::Serialize;
use whether_core::models::short_name;
use whether_core::scales::saffir_simpson;
use whether_core::storm::analysis::{
    self, ModelEventStats, SIGNIFICANT_GUST_KMH, SIGNIFICANT_HORIZON_SECS, SIGNIFICANT_PRECIP_MM,
    SIGNIFICANT_SNOW_CM, StormEvent,
};
use whether_core::storm::hurricane::hurricane_window;
use whether_core::storm::{Agreement, Hurricane, StormOutlook};
use whether_core::time::Weekday;
use whether_core::units::{Mps, WindUnit};

use crate::model::SpotSnapshot;
use crate::time::UnixTime;
use crate::units::{HeightUnit, SpeedUnit};
use crate::view::cards::sun_moon::day_label;
use crate::view::chart::Zoned;
use crate::view::format::Formatter;
use crate::view::strings::{Key, SafetyKey};
use crate::view::{
    CardChrome, CardId, CardState, Compact, CreditVm, NoticeKind, Quantity, Row, Section, ViewInput,
};

/// A check older than this is shown as stale: the storm check runs every three hours.
const STALE_AFTER_S: i64 = 9 * 3_600;
const HOUR: i64 = 3_600;
const DAY: i64 = 24 * HOUR;

const OPEN_METEO_LINK: &str = "https://open-meteo.com/";
const NHC_LINK: &str = "https://www.nhc.noaa.gov/";

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct StormAheadCard {
    pub chrome: CardChrome,
    pub compact: Compact,
    pub full: StormAheadFull,
}

/// What made the weather event significant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StormHeadlineKind {
    Snow,
    Rain,
    Gusts,
}

/// How strong a coming storm is, for colouring: a model storm is a watch-level signal; a
/// cyclone follows NHC's classification.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StormSeverity {
    Watch,
    Strong,
    Severe,
    Extreme,
}

/// The figure that goes furthest past its threshold, as the most any model brings.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct StormHeadlineVm {
    pub kind: StormHeadlineKind,
    /// "Up to 14 in of snow", "Up to 1.7 in of rain" or "Gusts to 52 mph".
    pub text: String,
    pub amount: Quantity,
}

/// One model's numbers for the event.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct StormModelRowVm {
    /// The model's short name: "ECMWF".
    pub model: String,
    pub snow: Quantity,
    pub precipitation: Quantity,
    pub gust: Quantity,
    /// The lowest pressure at the spot.
    pub low: Quantity,
    /// "Sun 1 PM"
    pub low_at: String,
    /// When the precipitation starts: "Sat 9 PM".
    pub onset: String,
}

/// How well the models agree on the event's totals.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct StormAgreementVm {
    pub agreement: Agreement,
    /// "High agreement", "Medium agreement", "Low agreement".
    pub label: String,
    /// "ECMWF brings 14 in of snow; GFS 3 in." Empty when the models cannot be told apart.
    pub sentence: String,
}

/// The coming storm from the forecast models.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct StormEventVm {
    /// "Storm ahead · Sat night – Mon night"
    pub title: String,
    /// When the rain or snow falls: "Sat night – Mon night".
    pub window: String,
    pub start: i64,
    pub end: i64,
    pub severity: StormSeverity,
    pub headline: StormHeadlineVm,
    /// Absent when fewer than two models have a total.
    pub agreement: Option<StormAgreementVm>,
    pub models: Vec<StormModelRowVm>,
}

/// A hurricane whose forecast cone reaches the spot.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct HurricaneAheadVm {
    pub id: String,
    pub name: String,
    /// "Hurricane Josephine · Category 1"
    pub title: String,
    pub classification: String,
    pub category: Option<u8>,
    pub severity: StormSeverity,
    pub wind: Quantity,
    pub pressure: Quantity,
    pub movement: String,
    /// NHC's strength line: "Winds 65 mph · 29.18 inHg · NE at 13 mph".
    pub strength: String,
    /// "Advisory Thu 5:00 PM"
    pub advisory: String,
    pub advisory_url: Option<String>,
    /// "The forecast cone reaches this spot on Thu Jan 21", or "This spot is in the forecast cone".
    pub reaches: String,
    /// When the cone reaches the spot (now when it is there already).
    pub reaches_at: i64,
    /// When the cone leaves the spot.
    pub leaves_at: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct StormAheadFull {
    /// The storm from the forecast models, when one is coming.
    pub event: Option<StormEventVm>,
    /// Hurricanes whose cone reaches the spot, the soonest first.
    pub hurricanes: Vec<HurricaneAheadVm>,
    /// Why the card shows, and the safety text.
    pub sections: Vec<Section>,
}

// -- display units ----------------------------------------------------------------------------

/// Snow, precipitation and the missing placeholder in the user's length unit: inches for feet,
/// else centimetres and millimetres.
struct Amounts<'a> {
    f: &'a Formatter,
    imperial: bool,
}

impl Amounts<'_> {
    fn missing(&self, unit: &str) -> Quantity {
        Quantity {
            value: None,
            text: self.f.strings.get(Key::Missing).to_owned(),
            unit: unit.to_owned(),
        }
    }

    /// The unit of snow (`snow`) or rain amounts: inches, else centimetres or millimetres.
    fn unit(&self, snow: bool) -> &'static str {
        self.f.strings.get(match (self.imperial, snow) {
            (true, _) => Key::UnitInches,
            (false, true) => Key::UnitCentimetres,
            (false, false) => Key::UnitMillimetres,
        })
    }

    fn snow(&self, cm: Option<f32>) -> Quantity {
        let unit = self.unit(true);
        let factor = if self.imperial { 1.0 / 2.54 } else { 1.0 };
        cm.map_or_else(
            || self.missing(unit),
            |cm| self.f.quantity(cm * factor, 1, unit),
        )
    }

    fn precipitation(&self, mm: Option<f32>) -> Quantity {
        let unit = self.unit(false);
        let (factor, decimals) = if self.imperial {
            (1.0 / 25.4, 2)
        } else {
            (1.0, 1)
        };
        mm.map_or_else(
            || self.missing(unit),
            |mm| self.f.quantity(mm * factor, decimals, unit),
        )
    }

    fn gust(&self, kmh: Option<f32>) -> Quantity {
        self.f.wind_speed(kmh.map(|v| v / 3.6))
    }
}

/// An amount rounded for a sentence: whole numbers from 1 up, one decimal below, one more
/// decimal for inches of rain.
fn rough(a: &Amounts, q: &Quantity, snow: bool) -> Quantity {
    let Some(value) = q.value else {
        return q.clone();
    };
    let extra = u8::from(!snow && a.imperial);
    a.f.quantity(value, u8::from(value < 1.0) + extra, &q.unit)
}

/// A speed in knots in the user's wind unit.
fn knots(f: &Formatter, kt: Option<f32>) -> Quantity {
    f.wind_speed(
        kt.and_then(|kt| Mps::from_unit(kt, WindUnit::Knots))
            .map(|m| m.0),
    )
}

/// A sustained wind as NHC's advisories give it: rounded to the nearest 5 in knots, mph and km/h.
fn advisory_wind(f: &Formatter, kt: Option<u16>) -> Quantity {
    let wind = knots(f, kt.map(f32::from));
    match wind.value {
        Some(v) if f.units.wind_speed != SpeedUnit::Ms => {
            f.quantity((v / 5.0).round() * 5.0, 0, &wind.unit)
        }
        _ => wind,
    }
}

// -- time -------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Part {
    Morning,
    Afternoon,
    Night,
}

/// The day and part of it a forecaster would name for `t`: morning (06-12), afternoon (12-18) or
/// night (18-06, named after its evening).
fn part_of_day(z: &Zoned, t: UnixTime) -> (Weekday, Part) {
    let local = z.local(t);
    match local.hour {
        0..=5 => (z.local(UnixTime(t.0 - 6 * HOUR)).weekday, Part::Night),
        6..=11 => (local.weekday, Part::Morning),
        12..=17 => (local.weekday, Part::Afternoon),
        _ => (local.weekday, Part::Night),
    }
}

/// "Sun afternoon", "Tue night".
fn day_part_label(f: &Formatter, z: &Zoned, t: UnixTime) -> String {
    let (day, part) = part_of_day(z, t);
    let key = match part {
        Part::Morning => SafetyKey::StormPartMorning,
        Part::Afternoon => SafetyKey::StormPartAfternoon,
        Part::Night => SafetyKey::StormPartNight,
    };
    f.strings
        .safety_fmt(key, &[("day", f.strings.weekday(day))])
}

/// "Sun 1 PM" or "Sun 13:00".
fn day_time(z: &Zoned, f: &Formatter, t: UnixTime) -> String {
    format!(
        "{} {}",
        f.strings.weekday(z.local(t).weekday),
        f.hour_label(t)
    )
}

/// The span in parts of days: "Sun afternoon", "Sun morning – Sun night", or for a day or more
/// "Sat night – Mon" (the end's part is kept when it is a night).
fn span_label(f: &Formatter, z: &Zoned, start: UnixTime, end: UnixTime) -> String {
    let last = UnixTime((end.0 - 1).max(start.0));
    let long = end.0 - start.0 >= DAY;
    let from = day_part_label(f, z, start);
    let (start_day, start_part) = part_of_day(z, start);
    let (end_day, end_part) = part_of_day(z, last);
    if (start_day, start_part) == (end_day, end_part) {
        return from;
    }
    let to = if long && end_part != Part::Night {
        f.strings.weekday(end_day).to_owned()
    } else {
        day_part_label(f, z, last)
    };
    format!("{from} – {to}")
}

/// When the rain or snow of `event` falls: its window without the padding the analysis adds,
/// except where the window meets the edge of the data.
fn wet_span(outlook: &StormOutlook, event: &StormEvent) -> (UnixTime, UnixTime) {
    let first = outlook.points.iter().filter_map(|p| p.times.first()).min();
    let last = outlook.points.iter().filter_map(|p| p.times.last()).max();
    let pad = analysis::EVENT_PAD_SECS;
    let start = match first {
        Some(first) if event.start.0 > first.0 - HOUR => event.start.0 + pad,
        _ => event.start.0,
    };
    let end = match last {
        Some(last) if event.end.0 < last.0 => event.end.0 - pad,
        _ => event.end.0,
    };
    if end > start {
        (UnixTime(start), UnixTime(end))
    } else {
        (event.start, event.end)
    }
}

// -- the weather event ------------------------------------------------------------------------

/// The largest of an event's quantities relative to its threshold, as the most any model brings.
fn headline(a: &Amounts, event: &StormEvent) -> Option<StormHeadlineVm> {
    let most = |pick: fn(&ModelEventStats) -> Option<f32>| {
        event.models.iter().filter_map(pick).reduce(f32::max)
    };
    let mut best: Option<(f32, StormHeadlineVm)> = None;
    let mut consider = |ratio: f32, vm: StormHeadlineVm| {
        if best.as_ref().is_none_or(|(r, _)| ratio > *r) {
            best = Some((ratio, vm));
        }
    };
    if let Some(cm) = most(|m| m.total_snow_cm) {
        let amount = rough(a, &a.snow(Some(cm)), true);
        consider(
            event.mean_snow_cm / SIGNIFICANT_SNOW_CM,
            StormHeadlineVm {
                kind: StormHeadlineKind::Snow,
                text: a
                    .f
                    .strings
                    .safety_fmt(SafetyKey::StormSnow, &[("amount", &amount.full())]),
                amount,
            },
        );
    }
    if let Some(mm) = most(|m| m.total_mm) {
        let amount = rough(a, &a.precipitation(Some(mm)), false);
        consider(
            event.mean_total_mm / SIGNIFICANT_PRECIP_MM,
            StormHeadlineVm {
                kind: StormHeadlineKind::Rain,
                text: a
                    .f
                    .strings
                    .safety_fmt(SafetyKey::StormRain, &[("amount", &amount.full())]),
                amount,
            },
        );
    }
    if let Some(kmh) = most(|m| m.peak_gust_kmh) {
        let amount = a.gust(Some(kmh));
        consider(
            analysis::mean_peak_gust_kmh(event).unwrap_or(0.0) / SIGNIFICANT_GUST_KMH,
            StormHeadlineVm {
                kind: StormHeadlineKind::Gusts,
                text: a
                    .f
                    .strings
                    .safety_fmt(SafetyKey::StormGusts, &[("amount", &amount.full())]),
                amount,
            },
        );
    }
    best.map(|(_, vm)| vm)
}

fn agreement_vm(a: &Amounts, event: &StormEvent, spot: (f64, f64)) -> Option<StormAgreementVm> {
    // The watch carries no pressure grids, so the verdict rests on the totals alone.
    let result = analysis::agreement(&event.models, &[], spot.0, spot.1);
    let verdict = result.verdict?;
    let label = a.f.strings.safety(match verdict {
        Agreement::High => SafetyKey::StormAgreementHigh,
        Agreement::Medium => SafetyKey::StormAgreementMedium,
        Agreement::Low => SafetyKey::StormAgreementLow,
    });
    let mut sentence = String::new();
    if let (Some(wet), Some(dry)) = (&result.wettest, &result.driest)
        && wet.model != dry.model
    {
        let snow_of = |model: &str| {
            event
                .models
                .iter()
                .find(|m| m.model == model)
                .and_then(|m| m.total_snow_cm)
        };
        let amount = |snow: Option<f32>, mm: f32| match snow {
            Some(cm) if event.has_snow() => rough(a, &a.snow(Some(cm)), true),
            _ => rough(a, &a.precipitation(Some(mm)), false),
        };
        let (wet_snow, dry_snow) = (snow_of(&wet.model), snow_of(&dry.model));
        let snowy = event.has_snow() && wet_snow.is_some() && dry_snow.is_some();
        let (w, d) = if snowy {
            (
                amount(wet_snow, wet.total_mm),
                amount(dry_snow, dry.total_mm),
            )
        } else {
            (amount(None, wet.total_mm), amount(None, dry.total_mm))
        };
        let noun = a.f.strings.safety(if snowy {
            SafetyKey::StormOfSnow
        } else {
            SafetyKey::StormOfRain
        });
        sentence = a.f.strings.safety_fmt(
            SafetyKey::StormBrings,
            &[
                ("wet", short_name(&wet.model)),
                ("wet_amount", &w.full()),
                ("noun", noun),
                ("dry", short_name(&dry.model)),
                ("dry_amount", &d.full()),
            ],
        );
    }
    Some(StormAgreementVm {
        agreement: verdict,
        label: label.to_owned(),
        sentence,
    })
}

fn model_row(a: &Amounts, z: &Zoned, stats: &ModelEventStats) -> StormModelRowVm {
    let f = a.f;
    let at = |t: Option<UnixTime>| {
        t.map_or_else(
            || f.strings.get(Key::Missing).to_owned(),
            |t| day_time(z, f, t),
        )
    };
    StormModelRowVm {
        model: short_name(&stats.model).to_owned(),
        snow: a.snow(stats.total_snow_cm),
        precipitation: a.precipitation(stats.total_mm),
        gust: a.gust(stats.peak_gust_kmh),
        low: f.pressure(stats.lowest_hpa),
        low_at: at(stats.lowest_at),
        onset: at(stats.onset),
    }
}

fn event_vm(
    a: &Amounts,
    z: &Zoned,
    outlook: &StormOutlook,
    event: &StormEvent,
    spot: (f64, f64),
) -> Option<StormEventVm> {
    let (wet_start, wet_end) = wet_span(outlook, event);
    let window = span_label(a.f, z, wet_start, wet_end);
    Some(StormEventVm {
        title: a
            .f
            .strings
            .safety_fmt(SafetyKey::StormTitle, &[("window", &window)]),
        window,
        start: event.start.0,
        end: event.end.0,
        severity: StormSeverity::Watch,
        headline: headline(a, event)?,
        agreement: agreement_vm(a, event, spot),
        models: event.models.iter().map(|m| model_row(a, z, m)).collect(),
    })
}

// -- hurricanes -------------------------------------------------------------------------------

fn is_hurricane(h: &Hurricane) -> bool {
    matches!(h.classification.as_str(), "HU" | "TY")
}

fn category(h: &Hurricane) -> Option<u8> {
    if is_hurricane(h) {
        h.wind_kt.and_then(saffir_simpson)
    } else {
        None
    }
}

fn severity(h: &Hurricane) -> StormSeverity {
    match category(h) {
        Some(3..) => StormSeverity::Extreme,
        Some(_) => StormSeverity::Severe,
        None if is_hurricane(h) => StormSeverity::Severe,
        None if matches!(h.classification.as_str(), "TS" | "STS") => StormSeverity::Strong,
        None => StormSeverity::Watch,
    }
}

/// "Hurricane Josephine", "Tropical Storm Josephine".
fn kind_name(f: &Formatter, h: &Hurricane) -> String {
    let kind = match h.classification.as_str() {
        "HU" => SafetyKey::HurricaneHurricane,
        "TS" => SafetyKey::HurricaneTropicalStorm,
        "TD" => SafetyKey::HurricaneTropicalDepression,
        "STS" => SafetyKey::HurricaneSubtropicalStorm,
        "STD" => SafetyKey::HurricaneSubtropicalDepression,
        "PTC" => SafetyKey::HurricanePostTropical,
        "PC" => SafetyKey::HurricanePotentialTropical,
        "TY" => SafetyKey::HurricaneTyphoon,
        _ => SafetyKey::HurricaneCyclone,
    };
    f.strings.safety_fmt(kind, &[("name", &h.name)])
}

/// "NE at 13 mph", "Stationary" below 1 kt, or the missing placeholder.
fn movement(f: &Formatter, h: &Hurricane) -> String {
    match (h.movement_deg, h.movement_kt) {
        (_, Some(kt)) if kt < 1.0 => f.strings.safety(SafetyKey::StormStationary).to_owned(),
        (Some(deg), Some(kt)) => f.strings.safety_fmt(
            SafetyKey::StormMovement,
            &[
                ("direction", f.strings.compass(deg)),
                ("speed", &knots(f, Some(kt)).full()),
            ],
        ),
        _ => f.strings.get(Key::Missing).to_owned(),
    }
}

fn hurricane_vm(
    f: &Formatter,
    z: &Zoned,
    h: &Hurricane,
    window: (UnixTime, UnixTime),
    now: UnixTime,
) -> HurricaneAheadVm {
    let name = kind_name(f, h);
    let category = category(h);
    let wind = advisory_wind(f, h.wind_kt);
    let pressure = f.pressure(h.pressure_hpa);
    let movement = movement(f, h);
    let advisory_day = f.strings.weekday(z.local(h.advisory_time).weekday);
    let reaches = if window.0 <= now {
        f.strings.safety(SafetyKey::StormInCone).to_owned()
    } else {
        f.strings.safety_fmt(
            SafetyKey::StormConeReaches,
            &[("date", &day_label(f, z.local(window.0).date))],
        )
    };
    HurricaneAheadVm {
        id: h.id.clone(),
        name: h.name.clone(),
        title: category.map_or_else(
            || name.clone(),
            |n| {
                f.strings.safety_fmt(
                    SafetyKey::HurricaneCategory,
                    &[("name", &name), ("category", &n.to_string())],
                )
            },
        ),
        classification: h.classification.clone(),
        category,
        severity: severity(h),
        strength: f.strings.safety_fmt(
            SafetyKey::StormStrength,
            &[
                ("wind", &wind.full()),
                ("pressure", &pressure.full()),
                ("movement", &movement),
            ],
        ),
        wind,
        pressure,
        movement,
        advisory: f.strings.safety_fmt(
            SafetyKey::StormAdvisory,
            &[("day", advisory_day), ("time", &f.clock(h.advisory_time))],
        ),
        advisory_url: h.advisory_url.clone(),
        reaches,
        reaches_at: window.0.0,
        leaves_at: window.1.0,
    }
}

/// The hurricanes whose forecast cone covers the spot within five days, soonest first.
fn hurricanes_ahead(
    f: &Formatter,
    z: &Zoned,
    snapshot: &SpotSnapshot,
    now: UnixTime,
) -> Vec<HurricaneAheadVm> {
    let Some(storm) = &snapshot.storm else {
        return Vec::new();
    };
    let spot = (snapshot.spot.latitude, snapshot.spot.longitude);
    let mut found: Vec<(UnixTime, &Hurricane, (UnixTime, UnixTime))> = storm
        .hurricanes
        .iter()
        .filter_map(|h| {
            hurricane_window(h, spot, now, SIGNIFICANT_HORIZON_SECS).map(|w| (w.0, h, w))
        })
        .collect();
    found.sort_by_key(|(start, _, _)| *start);
    found
        .into_iter()
        .map(|(_, h, w)| hurricane_vm(f, z, h, w, now))
        .collect()
}

// -- the card ---------------------------------------------------------------------------------

fn credits(f: &Formatter, event: bool, hurricanes: &[HurricaneAheadVm]) -> Vec<CreditVm> {
    let s = f.strings;
    let mut credits = Vec::new();
    if event {
        credits.push(CreditVm {
            source: s.safety(SafetyKey::SourceOpenMeteo).to_owned(),
            text: s.safety(SafetyKey::CreditOpenMeteoModels).to_owned(),
            link: Some(OPEN_METEO_LINK.to_owned()),
            licence: Some(s.safety(SafetyKey::LicenceCcBy4).to_owned()),
        });
    }
    if !hurricanes.is_empty() {
        credits.push(CreditVm {
            source: s.safety(SafetyKey::SourceNhc).to_owned(),
            text: s.safety(SafetyKey::CreditNhc).to_owned(),
            link: Some(
                hurricanes[0]
                    .advisory_url
                    .clone()
                    .unwrap_or_else(|| NHC_LINK.to_owned()),
            ),
            licence: Some(s.safety(SafetyKey::LicencePublicDomain).to_owned()),
        });
    }
    credits
}

/// Why the card shows, and the safety text.
fn sections(a: &Amounts, hurricanes: bool) -> Vec<Section> {
    let (snow, precip) = if a.imperial {
        (SIGNIFICANT_SNOW_CM / 2.54, SIGNIFICANT_PRECIP_MM / 25.4)
    } else {
        (SIGNIFICANT_SNOW_CM, SIGNIFICANT_PRECIP_MM)
    };
    let snow = a.f.quantity(snow, 0, a.unit(true)).full();
    let precip = a.f.quantity(precip, 0, a.unit(false)).full();
    let gust = a.gust(Some(SIGNIFICANT_GUST_KMH)).full();
    let days = SIGNIFICANT_HORIZON_SECS / DAY;
    let s = a.f.strings;
    let mut rows = vec![Row {
        label: s.safety(SafetyKey::StormShownWhen).to_owned(),
        value: s.safety_fmt(
            SafetyKey::StormShownWhenText,
            &[
                ("snow", &snow),
                ("precipitation", &precip),
                ("gust", &gust),
                ("days", &days.to_string()),
            ],
        ),
        detail: None,
    }];
    if hurricanes {
        rows.push(Row {
            label: s.safety(SafetyKey::StormHurricanesLabel).to_owned(),
            value: s.safety(SafetyKey::StormHurricanesText).to_owned(),
            detail: None,
        });
    }
    vec![
        Section {
            title: s.safety(SafetyKey::StormSectionWhy).to_owned(),
            rows,
        },
        Section {
            title: s.safety(SafetyKey::StormSectionSafety).to_owned(),
            rows: vec![Row {
                label: s.safety(SafetyKey::StormForecastsChange).to_owned(),
                value: s.safety(SafetyKey::StormForecastsChangeText).to_owned(),
                detail: None,
            }],
        },
    ]
}

/// The card, or `None` when no storm is significant within five days and no hurricane's forecast
/// cone reaches the spot.
#[allow(clippy::many_single_char_names, clippy::too_many_lines)]
pub fn build(input: &ViewInput) -> Option<StormAheadCard> {
    let snapshot = &input.snapshot;
    let storm = snapshot.storm.as_ref()?;
    let now = input.now;
    let f = Formatter::for_input(input);
    let z = Zoned::new(&snapshot.spot.time_zone);
    let a = Amounts {
        f: &f,
        imperial: input.units.height == HeightUnit::Ft,
    };
    let spot = (snapshot.spot.latitude, snapshot.spot.longitude);

    let event = storm.point.as_ref().and_then(|outlook| {
        let found = analysis::significant_event(outlook, now)?;
        event_vm(&a, &z, outlook, &found, spot)
    });
    let hurricanes = hurricanes_ahead(&f, &z, snapshot, now);
    if event.is_none() && hurricanes.is_empty() {
        return None;
    }

    let (headline, lines, accessibility) = match (hurricanes.first(), &event) {
        (Some(h), e) => {
            let second = e.as_ref().map_or_else(
                || h.strength.clone(),
                |e| {
                    f.strings
                        .safety_fmt(SafetyKey::StormAlso, &[("window", &e.window)])
                },
            );
            (
                Quantity {
                    value: None,
                    text: h.title.clone(),
                    unit: String::new(),
                },
                vec![h.reaches.clone(), second.clone()],
                f.strings.safety_fmt(
                    SafetyKey::StormHurricaneAcc,
                    &[
                        ("title", &h.title),
                        ("reaches", &h.reaches),
                        ("second", &second),
                    ],
                ),
            )
        }
        (None, Some(e)) => {
            let differ = match e.agreement.as_ref().map(|g| g.agreement) {
                Some(Agreement::Medium) => Some(SafetyKey::StormModelsDiffer),
                Some(Agreement::Low) => Some(SafetyKey::StormModelsDisagree),
                _ => None,
            };
            let line = match differ {
                Some(key) => format!("{} · {}", e.headline.text, f.strings.safety(key)),
                None => e.headline.text.clone(),
            };
            (
                e.headline.amount.clone(),
                vec![e.title.clone(), line.clone()],
                format!("{}. {line}.", e.title),
            )
        }
        (None, None) => return None,
    };

    let age = now.0 - storm.checked_at.0;
    let state = if age > STALE_AFTER_S {
        CardState::Stale {
            reason: f.strings.safety_fmt(
                SafetyKey::StormStale,
                &[("hours", &(age / HOUR).to_string())],
            ),
        }
    } else {
        CardState::Fresh
    };
    let footnote = f.strings.safety_fmt(
        SafetyKey::StormChecked,
        &[
            ("date", &f.date_of(storm.checked_at)),
            ("time", &f.clock(storm.checked_at)),
        ],
    );
    let full_sections = sections(&a, !hurricanes.is_empty());
    let credits = credits(&f, event.is_some(), &hurricanes);

    Some(StormAheadCard {
        chrome: CardChrome {
            id: CardId::StormAhead,
            title: f.strings.card_title(CardId::StormAhead).to_owned(),
            footnote: Some(footnote),
            credits,
            state,
            notices: vec![f.notice(NoticeKind::Approximate)],
            accessibility,
        },
        compact: Compact {
            headline,
            lines,
            spark: Vec::new(),
        },
        full: StormAheadFull {
            event,
            hurricanes,
            sections: full_sections,
        },
    })
}

#[cfg(test)]
pub(in crate::view) mod tests {
    use super::*;
    use crate::model::StormWatch;
    use crate::units::{DistanceUnit, PressureUnit, TempUnit};
    use crate::view::tests::sample_input;
    use crate::view::{DisplayUnits, Voice};
    use whether_core::storm::testdata::{self, STORM_NOW};

    fn metric(input: &mut ViewInput) {
        input.units = DisplayUnits {
            temperature: TempUnit::C,
            air_temperature: TempUnit::C,
            height: HeightUnit::M,
            stage: HeightUnit::M,
            lake_level: HeightUnit::M,
            waves: HeightUnit::M,
            discharge: crate::units::DischargeUnit::M3s,
            current_speed: SpeedUnit::Ms,
            wind_speed: SpeedUnit::Kmh,
            pressure: PressureUnit::Hpa,
            distance: DistanceUnit::Km,
        };
    }

    fn input_with(outlook: StormOutlook, hurricanes: Vec<Hurricane>) -> ViewInput {
        let mut input = sample_input();
        input.now = STORM_NOW;
        input.snapshot.storm = Some(StormWatch::from_outlook(STORM_NOW, outlook, hurricanes));
        input
    }

    pub(in crate::view) fn noreaster() -> ViewInput {
        input_with(testdata::noreaster(), Vec::new())
    }

    pub(in crate::view) fn hurricane() -> ViewInput {
        input_with(testdata::dry_week(), vec![testdata::hurricane()])
    }

    /// Both voices say the same thing: the card has no Luna variant.
    fn same_in_both_voices(mut input: ViewInput) -> StormAheadCard {
        input.voice = Voice::Luna;
        let luna = build(&input).unwrap();
        input.voice = Voice::Plain;
        let plain = build(&input).unwrap();
        assert_eq!(luna, plain);
        luna
    }

    #[test]
    fn a_quiet_week_has_no_card() {
        let input = input_with(testdata::dry_week(), Vec::new());
        assert_eq!(build(&input), None);
        let mut none = sample_input();
        none.snapshot.storm = None;
        assert_eq!(build(&none), None);
    }

    #[test]
    fn a_noreaster_is_ahead_in_us_units() {
        let card = same_in_both_voices(noreaster());
        let event = card.full.event.as_ref().unwrap();
        assert_eq!(event.headline.kind, StormHeadlineKind::Snow);
        assert_eq!(card.full.hurricanes.len(), 0);
        assert_eq!(card.chrome.credits.len(), 1);
        assert_eq!(card.chrome.credits[0].licence.as_deref(), Some("CC BY 4.0"));
        insta::assert_json_snapshot!("noreaster_us", card);
    }

    #[test]
    fn a_noreaster_is_ahead_in_metric() {
        let mut input = noreaster();
        metric(&mut input);
        input.voice = Voice::Plain;
        let card = build(&input).unwrap();
        assert_eq!(card.compact.headline.unit, "cm");
        insta::assert_json_snapshot!("noreaster_metric", card);
    }

    #[test]
    fn a_hurricane_cone_over_the_spot_is_ahead() {
        let card = same_in_both_voices(hurricane());
        let h = &card.full.hurricanes[0];
        assert!(
            h.reaches
                .starts_with("The forecast cone reaches this spot on ")
        );
        assert_eq!(h.category, Some(2));
        assert!(h.advisory_url.is_some());
        assert_eq!(
            card.chrome.credits[0].licence.as_deref(),
            Some("Public domain")
        );
        insta::assert_json_snapshot!("hurricane_us", card);
    }

    #[test]
    fn a_hurricane_in_metric_rounds_winds_like_nhc() {
        let mut input = hurricane();
        metric(&mut input);
        input.voice = Voice::Plain;
        let card = build(&input).unwrap();
        // 90 kt is 165 km/h in NHC's advisories.
        assert_eq!(card.full.hurricanes[0].wind.full(), "165 km/h");
        insta::assert_json_snapshot!("hurricane_metric", card);
    }

    #[test]
    fn a_hurricane_leads_a_storm_from_the_models() {
        let input = input_with(testdata::noreaster(), vec![testdata::hurricane()]);
        let card = same_in_both_voices(input);
        assert!(card.full.event.is_some());
        assert_eq!(card.full.hurricanes.len(), 1);
        assert!(card.compact.lines[1].starts_with("Also a storm ahead"));
    }

    #[test]
    fn a_cone_that_has_passed_is_not_ahead() {
        let mut input = hurricane();
        input.now = UnixTime(STORM_NOW.0 + 12 * DAY);
        assert_eq!(build(&input), None);
    }

    #[test]
    fn an_old_check_is_stale() {
        let mut input = noreaster();
        input.now = UnixTime(STORM_NOW.0 + 10 * HOUR);
        let card = build(&input).unwrap();
        assert!(matches!(card.chrome.state, CardState::Stale { .. }));
    }
}
