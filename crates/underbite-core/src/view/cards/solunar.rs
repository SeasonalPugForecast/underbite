//! Solunar periods.

use schemars::JsonSchema;
use serde::Serialize;
use whether_core::presentation::TimeFormat;

use crate::model::{SolunarCause, SolunarKind, SolunarPeriod};
use crate::time::UnixTime;
use crate::view::cards::sun_moon::{computed_credit, day_label, today_and_after};
use crate::view::format::{Formatter, number};
use crate::view::{
    CardChrome, CardId, CardState, Compact, NoticeKind, Quantity, Row, Section, ViewInput, Voice,
};

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct SolunarCard {
    pub chrome: CardChrome,
    pub compact: Compact,
    pub full: SolunarFull,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct SolunarPeriodVm {
    pub kind: SolunarKind,
    pub cause: SolunarCause,
    /// "Major" or "Minor" (the playful label is the voice's, set by the formatter).
    pub label: String,
    pub start_t: i64,
    pub end_t: i64,
    /// "6:10 to 8:10 AM"
    pub range: String,
    /// "Moon overhead", "Moonrise"
    pub cause_label: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct SolunarDayVm {
    pub date: String,
    pub periods: Vec<SolunarPeriodVm>,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct SolunarFull {
    pub days: Vec<SolunarDayVm>,
    pub sections: Vec<Section>,
}

/// The period's name in the voice: "Major", or Luna's "Big nibble".
pub(crate) fn kind_label(voice: Voice, kind: SolunarKind) -> &'static str {
    match (voice, kind) {
        (Voice::Plain, SolunarKind::Major) => "Major",
        (Voice::Plain, SolunarKind::Minor) => "Minor",
        (Voice::Luna, SolunarKind::Major) => "Big nibble",
        (Voice::Luna, SolunarKind::Minor) => "Small nibble",
    }
}

fn cause_label(cause: SolunarCause) -> &'static str {
    match cause {
        SolunarCause::UpperTransit => "Moon overhead",
        SolunarCause::LowerTransit => "Moon underfoot",
        SolunarCause::Rise => "Moonrise",
        SolunarCause::Set => "Moonset",
    }
}

/// "6:10 to 8:10 AM": a shared AM or PM is written once.
pub(crate) fn range_text(f: &Formatter, start: UnixTime, end: UnixTime) -> String {
    let (a, b) = (f.clock(start), f.clock(end));
    if f.clock == TimeFormat::H12
        && let (Some((ta, sa)), Some((_, sb))) = (a.rsplit_once(' '), b.rsplit_once(' '))
        && sa == sb
    {
        return format!("{ta} to {b}");
    }
    format!("{a} to {b}")
}

fn period_vm(f: &Formatter, p: &SolunarPeriod) -> SolunarPeriodVm {
    SolunarPeriodVm {
        kind: p.kind,
        cause: p.cause,
        label: kind_label(f.strings.voice, p.kind).to_owned(),
        start_t: p.start.0,
        end_t: p.end.0,
        range: range_text(f, p.start, p.end),
        cause_label: cause_label(p.cause).to_owned(),
    }
}

/// The soonest period of a kind that has not ended, across the snapshot's days.
fn next_of(periods: &[&SolunarPeriod], kind: SolunarKind, now: UnixTime) -> Option<SolunarPeriod> {
    periods
        .iter()
        .filter(|p| p.kind == kind && p.end > now)
        .min_by_key(|p| (p.start, p.end))
        .map(|p| **p)
}

pub fn build(input: &ViewInput) -> Option<SolunarCard> {
    if !input.solunar.show {
        return None;
    }
    let days = today_and_after(input, &input.snapshot.sky.days);
    let all: Vec<&SolunarPeriod> = days.iter().flat_map(|d| &d.solunar).collect();
    if all.is_empty() {
        return None;
    }
    let f = Formatter::for_input(input);
    let now = input.now;

    let major = next_of(&all, SolunarKind::Major, now);
    let minor = next_of(&all, SolunarKind::Minor, now);
    let line = |p: &Option<SolunarPeriod>| {
        p.map(|p| {
            let label = kind_label(f.strings.voice, p.kind);
            if p.start <= now {
                format!("{label} now, until {}", f.clock(p.end))
            } else {
                format!(
                    "Next {} {}",
                    label.to_lowercase(),
                    range_text(&f, p.start, p.end)
                )
            }
        })
    };
    let lines: Vec<String> = [line(&major), line(&minor)].into_iter().flatten().collect();

    // The headline counts down to the nearest period of either kind.
    let nearest = [major, minor].into_iter().flatten().min_by_key(|p| p.start);
    let headline = nearest.map_or_else(Quantity::default, |p| {
        let minutes = ((p.start.0 - now.0).max(0) / 60) as f32;
        if p.start <= now {
            Quantity {
                value: Some(0.0),
                text: "now".to_owned(),
                unit: String::new(),
            }
        } else if minutes < 120.0 {
            Quantity {
                value: Some(minutes),
                text: number(minutes, 0),
                unit: "min".to_owned(),
            }
        } else {
            let hours = minutes / 60.0;
            Quantity {
                value: Some(hours),
                text: number(hours, 1),
                unit: "h".to_owned(),
            }
        }
    });

    let day_vms: Vec<SolunarDayVm> = days
        .iter()
        .map(|d| SolunarDayVm {
            date: day_label(&f, d.date),
            periods: d.solunar.iter().map(|p| period_vm(&f, p)).collect(),
        })
        .collect();

    let mut rows = Vec::new();
    for (p, name) in [(&major, "Next major"), (&minor, "Next minor")] {
        if let Some(p) = p {
            let vm = period_vm(&f, p);
            rows.push(Row {
                label: name.to_owned(),
                value: format!("{} ({})", vm.range, vm.cause_label.to_lowercase()),
                detail: Some(f.date_of(p.start)),
            });
        }
    }
    let accessibility = if lines.is_empty() {
        "Solunar: no more periods in the forecast. A traditional cue, not a promise.".to_owned()
    } else {
        format!(
            "Solunar: {}. A traditional cue, not a promise.",
            lines.join(". ")
        )
    };
    Some(SolunarCard {
        chrome: CardChrome {
            id: CardId::Solunar,
            title: f.strings.card_title(CardId::Solunar).to_owned(),
            footnote: None,
            credits: vec![computed_credit(f.strings)],
            state: CardState::Fresh,
            notices: vec![f.notice(NoticeKind::TraditionalCue)],
            accessibility,
        },
        compact: Compact {
            headline,
            lines,
            spark: Vec::new(),
        },
        full: SolunarFull {
            days: day_vms,
            sections: vec![Section {
                title: "Next".to_owned(),
                rows,
            }],
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::parse_rfc3339;
    use crate::view::cards::sun_moon::testing::battery_input;
    use whether_core::presentation::TimeFormat;

    #[test]
    fn periods_come_from_the_moon_events_with_the_traditional_cue() {
        let input = battery_input();
        let card = build(&input).unwrap();
        let kinds: Vec<_> = card.chrome.notices.iter().map(|n| n.kind).collect();
        assert_eq!(kinds, [NoticeKind::TraditionalCue]);
        assert_eq!(card.full.days.len(), 3);
        assert_eq!(card.full.days[0].date, "Sun Nov 8");
        let first = &card.full.days[0].periods[0];
        assert!(first.start_t < first.end_t);
        // Majors last two hours and minors one.
        for p in card.full.days.iter().flat_map(|d| &d.periods) {
            let minutes = (p.end_t - p.start_t) / 60;
            assert_eq!(
                minutes,
                if p.kind == SolunarKind::Major {
                    120
                } else {
                    60
                }
            );
        }
        assert!(card.compact.lines.len() <= 2);
        assert!(card.compact.lines[0].starts_with("Next big nibble"));
    }

    #[test]
    fn hidden_when_solunar_is_off() {
        let mut input = battery_input();
        input.solunar.show = false;
        assert!(build(&input).is_none());
    }

    #[test]
    fn a_period_in_progress_reads_now() {
        let mut input = battery_input();
        let p = input.snapshot.sky.days[0].solunar[0];
        input.now = UnixTime(p.start.0 + 600);
        let card = build(&input).unwrap();
        assert_eq!(card.compact.headline.text, "now");
        assert!(card.compact.lines.iter().any(|l| l.contains("now, until")));
    }

    #[test]
    fn ranges_share_a_meridiem_and_the_24_hour_clock_does_not() {
        let mut input = battery_input();
        input.voice = Voice::Plain;
        let f = Formatter::for_input(&input);
        let base = parse_rfc3339("2026-11-08T00:00:00Z").unwrap().0;
        let t = |h: i64, m: i64| UnixTime(base + h * 3_600 + m * 60);
        // 11:10 UTC is 6:10 AM EST in November.
        assert_eq!(range_text(&f, t(11, 10), t(13, 10)), "6:10 to 8:10 AM");
        assert_eq!(range_text(&f, t(16, 40), t(18, 10)), "11:40 AM to 1:10 PM");
        input.clock = TimeFormat::H24;
        let f = Formatter::for_input(&input);
        assert_eq!(range_text(&f, t(11, 10), t(13, 10)), "06:10 to 08:10");
    }

    #[test]
    fn snapshots_in_both_voices() {
        let mut input = battery_input();
        insta::assert_json_snapshot!("solunar_luna", build(&input).unwrap());
        input.voice = Voice::Plain;
        input.clock = TimeFormat::H24;
        let card = build(&input).unwrap();
        assert!(card.full.days[0].periods.iter().any(|p| p.label == "Major"));
        insta::assert_json_snapshot!("solunar_plain_24h", card);
    }
}
