//! The go/no-go card: each day and hour judged against the user's limits, with the reasons.
//!
//! The verdicts come from [`crate::trip::gonogo`]; this file only words them. A reason about a
//! measured value reads in the user's voice and units ("gusts 25 kn over your 18 kn limit");
//! warnings, advisories, storms, hurricanes and lightning are safety text, the same in every voice.

use schemars::JsonSchema;
use serde::Serialize;

use crate::time::UnixTime;
use crate::trip::gonogo::{Check, DayInputs, DayVerdict, Reason, ReasonKind, Verdict, assess};
use crate::view::cards::sun_moon::day_label;
use crate::view::cards::tide::credits as source_credits;
use crate::view::cards::wind::weather_credit;
use crate::view::cards::windows::{CARD_DAYS, local_days};
use crate::view::format::Formatter;
use crate::view::strings::{Key, SafetyKey};
use crate::view::{CardChrome, CardId, CardState, Compact, CreditVm, Quantity, ViewInput, Voice};

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct GoNoGoCard {
    pub chrome: CardChrome,
    pub compact: Compact,
    pub full: GoNoGoFull,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct GoNoGoFull {
    /// The name of the limits profile the days are judged against.
    pub profile: String,
    pub days: Vec<GoNoGoDayVm>,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct GoNoGoDayVm {
    /// "Sun Nov 8"
    pub date: String,
    pub verdict: Verdict,
    /// "Go", "Marginal", "No-go" or "Unknown".
    pub verdict_label: String,
    /// The sentence for the day: "Luna approves.", "No-go: gusts 25 kn over your 18 kn limit".
    pub summary: String,
    /// One per check and outcome, failures first.
    pub reasons: Vec<GoNoGoReasonVm>,
    /// Every local hour of the day, in order.
    pub hours: Vec<GoNoGoHourVm>,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct GoNoGoReasonVm {
    pub check: Check,
    pub kind: ReasonKind,
    /// The reason with its numbers; the whole sentence, as in the summary.
    pub text: String,
    /// The official page for a hurricane.
    pub link: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct GoNoGoHourVm {
    pub start_t: i64,
    /// "6 AM" or "06:00"
    pub label: String,
    pub verdict: Verdict,
    /// The leading reason when the hour is not a plain go.
    pub reason: Option<String>,
}

const fn is_safety(check: Check) -> bool {
    matches!(
        check,
        Check::SmallCraftAdvisory
            | Check::MarineWarning
            | Check::Lightning
            | Check::StormAhead
            | Check::Hurricane
    )
}

/// The noun of a check, for "Missing: ...".
fn noun(f: &Formatter, check: Check) -> &'static str {
    let s = f.strings;
    match check {
        Check::Forecast => s.get(Key::CheckForecast),
        Check::Wind => s.get(Key::CheckWind),
        Check::Gust => s.get(Key::CheckGust),
        Check::Waves => s.get(Key::CheckWaves),
        Check::Current => s.get(Key::CheckCurrent),
        Check::AirTemperature => s.get(Key::CheckAirTemperature),
        Check::Visibility => s.get(Key::CheckVisibility),
        Check::Flow => s.get(Key::CheckFlow),
        Check::SmallCraftAdvisory => s.safety(SafetyKey::CheckSmallCraft),
        Check::MarineWarning => s.safety(SafetyKey::CheckMarineWarning),
        Check::Lightning => s.safety(SafetyKey::CheckLightning),
        Check::StormAhead => s.safety(SafetyKey::CheckStormAhead),
        Check::Hurricane => s.safety(SafetyKey::CheckHurricane),
    }
}

/// A measured value or its limit in the user's units, from canonical units.
fn value(f: &Formatter, check: Check, v: f64) -> String {
    let v = Some(v as f32);
    match check {
        Check::Wind | Check::Gust => f.wind_speed(v),
        Check::Waves => f.wave_height(v),
        Check::Current => f.current_speed(v),
        Check::AirTemperature => f.temperature(v),
        Check::Visibility => f.distance(v),
        Check::Flow => f.discharge(v),
        _ => Quantity::default(),
    }
    .full()
}

/// When an alert, storm or cone is in effect.
fn span(f: &Formatter, from: Option<UnixTime>, until: Option<UnixTime>) -> Option<String> {
    let s = f.strings;
    let when = |t: UnixTime| format!("{}, {}", f.date_of(t), f.clock(t));
    match (from.map(when), until.map(when)) {
        (Some(start), Some(end)) => {
            Some(s.safety_fmt(SafetyKey::AlertSpan, &[("start", &start), ("end", &end)]))
        }
        (Some(start), None) => Some(s.safety_fmt(SafetyKey::AlertFrom, &[("start", &start)])),
        (None, Some(end)) => Some(s.safety_fmt(SafetyKey::AlertUntil, &[("end", &end)])),
        (None, None) => None,
    }
}

/// A safety finding in words; the same in every voice.
fn safety_text(f: &Formatter, r: &Reason) -> Option<String> {
    let s = f.strings;
    let span = span(f, r.at, r.until);
    let alert = |fallback: SafetyKey| {
        let name = r
            .subject
            .clone()
            .unwrap_or_else(|| s.safety(fallback).to_owned());
        match &span {
            Some(span) => s.safety_fmt(
                SafetyKey::NoGoAlertSpan,
                &[("alert", &name), ("span", span)],
            ),
            None => s.safety_fmt(SafetyKey::NoGoAlert, &[("alert", &name)]),
        }
    };
    Some(match r.check {
        Check::MarineWarning => alert(SafetyKey::AlertNameMarine),
        Check::SmallCraftAdvisory => alert(SafetyKey::AlertNameSmallCraft),
        Check::Lightning => s.safety(SafetyKey::NoGoLightning).to_owned(),
        Check::StormAhead => match &span {
            Some(span) => s.safety_fmt(SafetyKey::NoGoStormSpan, &[("span", span)]),
            None => s.safety(SafetyKey::NoGoStorm).to_owned(),
        },
        Check::Hurricane => s.safety_fmt(
            SafetyKey::NoGoHurricane,
            &[
                ("name", r.subject.as_deref().unwrap_or_default()),
                ("span", span.as_deref().unwrap_or_default()),
            ],
        ),
        _ => return None,
    })
}

/// One finding as a sentence in the user's voice, with its numbers.
fn reason_text(f: &Formatter, r: &Reason) -> String {
    let s = f.strings;
    match r.kind {
        ReasonKind::OutsideForecast => s.get(Key::GoNoGoBeyondForecast).to_owned(),
        ReasonKind::Past => s.get(Key::GoNoGoDayOver).to_owned(),
        ReasonKind::NoData => s.fmt(Key::GoNoGoMissing, &[("checks", noun(f, r.check))]),
        ReasonKind::Fails | ReasonKind::Marginal => {
            if is_safety(r.check) {
                return safety_text(f, r).unwrap_or_default();
            }
            let (Some(measured), Some(limit)) = (r.measured, r.limit) else {
                return String::new();
            };
            let fails = r.kind == ReasonKind::Fails;
            let key = match (fails, r.check.limit_is_minimum()) {
                (true, false) => Key::GoNoGoOver,
                (true, true) => Key::GoNoGoUnder,
                (false, false) => Key::GoNoGoNearOver,
                (false, true) => Key::GoNoGoNearUnder,
            };
            let what = s.fmt(
                key,
                &[
                    ("what", noun(f, r.check)),
                    ("measured", &value(f, r.check, measured)),
                    ("limit", &value(f, r.check, limit)),
                ],
            );
            s.fmt(
                if fails {
                    Key::GoNoGoBad
                } else {
                    Key::GoNoGoMarginal
                },
                &[("reason", &what)],
            )
        }
    }
}

/// The reason that leads: a safety failure, else any failure, else a marginal value, else the
/// first.
fn lead(reasons: &[Reason]) -> Option<&Reason> {
    reasons
        .iter()
        .find(|r| r.kind == ReasonKind::Fails && is_safety(r.check))
        .or_else(|| {
            [ReasonKind::Fails, ReasonKind::Marginal]
                .into_iter()
                .find_map(|k| reasons.iter().find(|r| r.kind == k))
        })
        .or_else(|| reasons.first())
}

/// The day's sentence.
fn summary(f: &Formatter, day: &DayVerdict) -> String {
    let s = f.strings;
    match day.verdict {
        Verdict::Go => s.get(Key::GoNoGoGood).to_owned(),
        Verdict::NoGo | Verdict::Marginal => lead(&day.reasons)
            .map(|r| reason_text(f, r))
            .unwrap_or_default(),
        Verdict::Unknown => {
            if day.reasons.iter().any(|r| r.kind == ReasonKind::Past) {
                return s.get(Key::GoNoGoDayOver).to_owned();
            }
            let missing: Vec<&str> = day
                .reasons
                .iter()
                .filter(|r| r.kind == ReasonKind::NoData)
                .map(|r| noun(f, r.check))
                .collect();
            let mut text = s.get(Key::GoNoGoUnknown).to_owned();
            let mut more: Vec<String> = Vec::new();
            if !missing.is_empty() {
                more.push(s.fmt(Key::GoNoGoMissing, &[("checks", &missing.join(", "))]));
            }
            if day
                .reasons
                .iter()
                .any(|r| r.kind == ReasonKind::OutsideForecast)
            {
                more.push(s.get(Key::GoNoGoBeyondForecast).to_owned());
            }
            for m in more {
                text.push_str(". ");
                text.push_str(&m);
            }
            text
        }
    }
}

fn verdict_label(f: &Formatter, verdict: Verdict) -> &'static str {
    match verdict {
        Verdict::Go => f.strings.get(Key::VerdictGo),
        Verdict::Marginal => f.strings.get(Key::VerdictMarginal),
        Verdict::NoGo => f.strings.safety(SafetyKey::VerdictNoGo),
        Verdict::Unknown => f.strings.get(Key::VerdictUnknown),
    }
}

pub(in crate::view) fn day_vm(f: &Formatter, day: &DayVerdict) -> GoNoGoDayVm {
    GoNoGoDayVm {
        date: day_label(f, day.day),
        verdict: day.verdict,
        verdict_label: verdict_label(f, day.verdict).to_owned(),
        summary: summary(f, day),
        reasons: day
            .reasons
            .iter()
            .map(|r| GoNoGoReasonVm {
                check: r.check,
                kind: r.kind,
                text: reason_text(f, r),
                link: r.link.clone(),
            })
            .collect(),
        hours: day
            .hours
            .iter()
            .map(|h| GoNoGoHourVm {
                start_t: h.start.0,
                label: f.hour_label(h.start),
                verdict: h.verdict,
                reason: (h.verdict != Verdict::Go)
                    .then(|| lead(&h.reasons).map(|r| reason_text(f, r)))
                    .flatten(),
            })
            .collect(),
    }
}

/// The sources the verdicts rest on: the forecast, and each source a reason names.
fn credits(f: &Formatter, input: &ViewInput, days: &[DayVerdict]) -> Vec<CreditVm> {
    let snapshot = &input.snapshot;
    let mut out: Vec<CreditVm> = Vec::new();
    let mut push = |c: CreditVm| {
        if !out.contains(&c) {
            out.push(c);
        }
    };
    if let Some(w) = &snapshot.weather {
        push(weather_credit(w));
    }
    let mut ids: Vec<&crate::provenance::SourceId> = days
        .iter()
        .flat_map(|d| d.reasons.iter().filter_map(|r| r.source.as_ref()))
        .collect();
    ids.sort();
    ids.dedup();
    for id in ids {
        match id.as_str() {
            // The forecast's own credit is already there.
            other
                if snapshot
                    .weather
                    .as_ref()
                    .is_some_and(|w| w.provider.id() == other) => {}
            "open-meteo" => push(CreditVm {
                source: f.strings.safety(SafetyKey::SourceOpenMeteo).to_owned(),
                text: f
                    .strings
                    .safety(SafetyKey::CreditOpenMeteoModels)
                    .to_owned(),
                link: None,
                licence: None,
            }),
            "nhc" => push(CreditVm {
                source: f.strings.safety(SafetyKey::SourceNhc).to_owned(),
                text: f.strings.safety(SafetyKey::CreditNhc).to_owned(),
                link: None,
                licence: None,
            }),
            _ => source_credits(f, snapshot, id)
                .into_iter()
                .for_each(&mut push),
        }
    }
    out
}

pub fn build(input: &ViewInput) -> Option<GoNoGoCard> {
    let limits = input.trip_limits.as_ref()?;
    let dates = local_days(input, CARD_DAYS);
    if dates.is_empty() {
        return None;
    }
    let f = Formatter::for_input(input);
    let mut plain = f.clone();
    plain.strings.voice = Voice::Plain;
    let river = input.river_ranges.unwrap_or_default();
    let day_inputs = DayInputs::new(&input.snapshot).with_river(river);
    let verdicts: Vec<DayVerdict> = dates
        .iter()
        .map(|&d| assess(d, &day_inputs, limits, input.now))
        .collect();
    let days: Vec<GoNoGoDayVm> = verdicts.iter().map(|d| day_vm(&f, d)).collect();
    let plain_days: Vec<GoNoGoDayVm> = verdicts.iter().map(|d| day_vm(&plain, d)).collect();

    let today = days.first()?;
    let mut lines = vec![today.summary.clone()];
    if let Some(next) = days.get(1) {
        lines.push(f.strings.fmt(
            Key::GoNoGoDayLine,
            &[("day", &next.date), ("summary", &next.summary)],
        ));
    }
    let headline = Quantity {
        value: None,
        text: today.verdict_label.clone(),
        unit: String::new(),
    };
    let title = f.strings.card_title(CardId::GoNoGo);
    // The plain meaning first, then the same in the user's voice when it differs.
    let plain_today = plain_days.first()?;
    let mut accessibility = format!("{title}: {}", plain_today.summary);
    if plain_today.summary != today.summary {
        accessibility.push_str(". ");
        accessibility.push_str(&today.summary);
    }
    Some(GoNoGoCard {
        chrome: CardChrome {
            id: CardId::GoNoGo,
            title: title.to_owned(),
            footnote: None,
            credits: credits(&f, input, &verdicts),
            state: CardState::Fresh,
            notices: Vec::new(),
            accessibility,
        },
        compact: Compact {
            headline,
            lines,
            spark: Vec::new(),
        },
        full: GoNoGoFull {
            profile: limits.name.clone(),
            days,
        },
    })
}

#[cfg(test)]
pub(in crate::view) mod tests {
    use super::*;
    use crate::trip::gonogo::HourVerdict;
    use crate::view::tests::sample_input;
    use whether_core::presentation::TimeFormat;

    fn reason(check: Check, kind: ReasonKind, measured: f64, limit: f64) -> Reason {
        Reason {
            check,
            kind,
            measured: Some(measured),
            limit: Some(limit),
            at: Some(UnixTime(1_762_600_000)),
            until: None,
            source: None,
            subject: None,
            link: None,
        }
    }

    fn day(verdict: Verdict, reasons: Vec<Reason>) -> DayVerdict {
        DayVerdict {
            day: crate::time::LocalDate {
                year: 2026,
                month: 11,
                day: 8,
            },
            verdict,
            reasons,
            hours: vec![HourVerdict {
                start: UnixTime(1_762_600_000),
                verdict,
                reasons: Vec::new(),
            }],
        }
    }

    fn formatter(voice: Voice) -> Formatter {
        let mut input = sample_input();
        input.voice = voice;
        Formatter::for_input(&input)
    }

    #[test]
    fn a_measured_reason_has_its_numbers_in_the_users_units() {
        // 25 kn and 18 kn in m/s.
        let gusts = reason(Check::Gust, ReasonKind::Fails, 12.86, 9.26);
        let plain = reason_text(&formatter(Voice::Plain), &gusts);
        assert_eq!(plain, "No-go: gusts 29 mph over your 21 mph limit");
        let luna = reason_text(&formatter(Voice::Luna), &gusts);
        assert_eq!(
            luna,
            "Luna's not impressed: gusts to 29 mph, your limit is 21 mph"
        );
        let cold = reason(Check::AirTemperature, ReasonKind::Marginal, 2.0, 1.5);
        let text = reason_text(&formatter(Voice::Plain), &cold);
        assert!(
            text.starts_with("Marginal: air temperature 36 °F"),
            "{text}"
        );
    }

    #[test]
    fn safety_reasons_are_plain_in_both_voices() {
        let mut warning = Reason {
            check: Check::MarineWarning,
            kind: ReasonKind::Fails,
            measured: None,
            limit: None,
            at: Some(UnixTime(1_762_600_000)),
            until: Some(UnixTime(1_762_650_000)),
            source: None,
            subject: Some("Gale Warning".into()),
            link: None,
        };
        let a = reason_text(&formatter(Voice::Luna), &warning);
        let b = reason_text(&formatter(Voice::Plain), &warning);
        assert_eq!(a, b);
        assert!(a.starts_with("No-go: Gale Warning, "), "{a}");
        assert!(!a.contains("Luna"));
        warning.check = Check::Lightning;
        let l = reason_text(&formatter(Voice::Luna), &warning);
        assert_eq!(l, reason_text(&formatter(Voice::Plain), &warning));
        assert!(!l.contains("Luna"));
    }

    #[test]
    fn an_unknown_day_lists_what_is_missing() {
        let d = day(
            Verdict::Unknown,
            vec![
                Reason {
                    kind: ReasonKind::NoData,
                    measured: None,
                    limit: None,
                    ..reason(Check::Wind, ReasonKind::NoData, 0.0, 0.0)
                },
                Reason {
                    kind: ReasonKind::NoData,
                    measured: None,
                    limit: None,
                    ..reason(Check::Lightning, ReasonKind::NoData, 0.0, 0.0)
                },
            ],
        );
        let text = summary(&formatter(Voice::Plain), &d);
        assert_eq!(text, "Not enough data to judge. Missing: wind, lightning");
    }

    #[test]
    fn the_card_judges_three_days_and_reads_plain_meaning_first() {
        let card = build(&sample_input()).unwrap();
        assert_eq!(card.full.days.len(), 3);
        assert_eq!(card.full.profile, "Kayak");
        assert!(card.chrome.accessibility.starts_with("Luna's verdict: "));
        assert!(card.compact.lines.len() <= 2);
    }

    #[test]
    fn a_spot_without_a_forecast_reads_unknown_with_what_is_missing() {
        let mut input = sample_input();
        input.snapshot.weather = None;
        input.snapshot.marine = crate::model::MarineConditions::default();
        input.snapshot.storm = None;
        let card = build(&input).unwrap();
        let today = &card.full.days[0];
        assert_eq!(today.verdict, Verdict::Unknown);
        assert!(
            today
                .summary
                .starts_with("Not enough data to judge. Missing: "),
            "{}",
            today.summary
        );
        assert_eq!(card.compact.headline.text, "Unknown");
        insta::assert_json_snapshot!("go_no_go_unknown", card);
    }

    #[test]
    fn no_limits_no_card() {
        let mut input = sample_input();
        input.trip_limits = None;
        assert!(build(&input).is_none());
    }

    #[test]
    fn snapshots_in_both_voices_and_unit_systems() {
        let mut input = sample_input();
        insta::assert_json_snapshot!("go_no_go_luna_us", build(&input).unwrap());
        input.voice = Voice::Plain;
        input.clock = TimeFormat::H24;
        input.units = crate::view::cards::flow::fixtures::metric();
        insta::assert_json_snapshot!("go_no_go_plain_metric", build(&input).unwrap());
    }
}
