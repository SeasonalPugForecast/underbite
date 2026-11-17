//! The fishing windows card ("snack times"): when the chosen factors line up, and why.
//!
//! The windows come from [`crate::trip::windows`]; this file only words them. Each factor that
//! matched is listed with the numbers behind it in the user's units, and the solunar factor
//! carries the traditional-cue notice.

use schemars::JsonSchema;
use serde::Serialize;

use crate::model::SpotSnapshot;
use crate::provenance::Provenance;
use crate::time::{LocalDate, UnixTime, local_date, local_date_bounds, zone};
use crate::trip::windows::{
    Evidence, Factor, FishingWindow, LightBand, Ranges, TideFlow, WindowInputs, compute,
};
use crate::view::cards::flow::credits as station_credits;
use crate::view::cards::sun_moon::{computed_credit, day_label};
use crate::view::cards::tide::credits as tide_credits;
use crate::view::cards::wind::weather_credit;
use crate::view::format::Formatter;
use crate::view::strings::Key;
use crate::view::{
    CardChrome, CardId, CardState, Compact, CreditVm, Notice, NoticeKind, Quantity, Section,
    ViewInput,
};

use super::solunar::kind_label;

/// Local days from today that the card and the go/no-go card look at.
pub(in crate::view) const CARD_DAYS: usize = 3;

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct WindowsCard {
    pub chrome: CardChrome,
    pub compact: Compact,
    pub full: WindowsFull,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct WindowsFull {
    /// The days that have a window, in order.
    pub days: Vec<WindowDayVm>,
    pub sections: Vec<Section>,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct WindowDayVm {
    /// "Sun Nov 8"
    pub date: String,
    pub windows: Vec<WindowVm>,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct WindowVm {
    pub start_t: i64,
    pub end_t: i64,
    /// "6:10 to 8:00 AM"
    pub range: String,
    /// The factors that matched, with their numbers.
    pub factors: Vec<WindowFactorVm>,
    /// Enabled factors that had no data in the window, by name.
    pub no_data: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct WindowFactorVm {
    pub factor: Factor,
    /// "Incoming tide"
    pub label: String,
    /// "1 h before the High at 6:52 AM"
    pub detail: String,
}

/// The spot's local days from today, `n` of them.
pub(in crate::view) fn local_days(input: &ViewInput, n: usize) -> Vec<LocalDate> {
    let Ok(tz) = zone(&input.snapshot.spot.time_zone) else {
        return Vec::new();
    };
    let Ok(mut date) = local_date(input.now, &tz) else {
        return Vec::new();
    };
    let mut days = Vec::new();
    while days.len() < n {
        days.push(date);
        let next = local_date_bounds(date, &tz)
            .and_then(|(_, end)| local_date(end, &tz))
            .ok();
        match next {
            Some(next) if next != date => date = next,
            _ => break,
        }
    }
    days
}

/// The windows for the local days `first` to `last` that end after `since`, with the user's
/// settings and the spot's best river flow.
pub(in crate::view) fn windows_between(
    input: &ViewInput,
    first: LocalDate,
    last: LocalDate,
    since: UnixTime,
) -> Vec<FishingWindow> {
    let flow = input
        .river_ranges
        .and_then(|r| Some([f64::from(r.best_min_m3s?), f64::from(r.best_max_m3s?)]));
    let ranges = Ranges {
        water_temp_c: None,
        flow_m3s: flow,
    };
    let inputs = WindowInputs::from_snapshot(&input.snapshot, first, last, ranges);
    compute(&inputs, &input.windows, since)
}

/// The windows of the next [`CARD_DAYS`] local days.
fn upcoming(input: &ViewInput) -> Vec<FishingWindow> {
    let days = local_days(input, CARD_DAYS);
    match (days.first(), days.last()) {
        (Some(&first), Some(&last)) => windows_between(input, first, last, input.now),
        _ => Vec::new(),
    }
}

fn factor_label(f: &Formatter, factor: Factor) -> &'static str {
    f.strings.get(match factor {
        Factor::TideStage => Key::FactorTideStage,
        Factor::CurrentStrength => Key::FactorCurrentStrength,
        Factor::LowLight => Key::FactorLowLight,
        Factor::Solunar => Key::CardSolunar,
        Factor::PressureFalling => Key::FactorPressureFalling,
        Factor::WaterTemp => Key::FactorWaterTemp,
        Factor::FlowInRange => Key::FactorFlowInRange,
    })
}

/// The factor's name and the numbers behind it, in the user's units.
#[allow(clippy::too_many_lines)]
fn describe(f: &Formatter, evidence: &Evidence) -> (&'static str, String) {
    let s = f.strings;
    match *evidence {
        Evidence::TideStage {
            flow,
            next_kind,
            next_at,
            seconds_to_next,
        } => (
            s.get(match flow {
                TideFlow::Incoming => Key::FactorTideIncoming,
                TideFlow::Outgoing => Key::FactorTideOutgoing,
            }),
            s.fmt(
                Key::WindowTideNext,
                &[
                    ("duration", &f.duration(seconds_to_next)),
                    ("kind", &s.tide_kind(next_kind).to_lowercase()),
                    ("time", &f.clock(next_at)),
                ],
            ),
        ),
        Evidence::CurrentStrength { speed_ms, flooding } => (
            s.get(Key::FactorCurrentStrength),
            s.fmt(
                if flooding {
                    Key::WindowCurrentFlood
                } else {
                    Key::WindowCurrentEbb
                },
                &[("speed", &f.current_speed(Some(speed_ms as f32)).full())],
            ),
        ),
        Evidence::LowLight {
            band,
            sun_event_at,
            minutes_from_event,
        } => {
            let event = match band {
                LightBand::Morning => Key::Sunrise,
                LightBand::Evening => Key::Sunset,
            };
            (
                s.get(Key::FactorLowLight),
                s.fmt(
                    if minutes_from_event < 0 {
                        Key::WindowLightBefore
                    } else {
                        Key::WindowLightAfter
                    },
                    &[
                        ("duration", &f.duration(minutes_from_event.abs() * 60)),
                        ("event", &s.get(event).to_lowercase()),
                        ("time", &f.clock(sun_event_at)),
                    ],
                ),
            )
        }
        Evidence::Solunar { kind, centre, .. } => (
            s.get(Key::CardSolunar),
            s.fmt(
                Key::WindowSolunar,
                &[
                    ("label", &kind_label(f, kind).to_lowercase()),
                    ("time", &f.clock(centre)),
                ],
            ),
        ),
        Evidence::PressureFalling { change_hpa, hours } => (
            s.get(Key::FactorPressureFalling),
            s.fmt(
                Key::WindowPressure,
                &[
                    ("amount", &f.pressure(Some(change_hpa.abs() as f32)).full()),
                    ("hours", &hours.to_string()),
                ],
            ),
        ),
        Evidence::WaterTemp {
            celsius,
            min_c,
            max_c,
        } => (
            s.get(Key::FactorWaterTemp),
            s.fmt(
                Key::WindowInRange,
                &[
                    ("value", &f.temperature(Some(celsius as f32)).full()),
                    ("min", &f.temperature(Some(min_c as f32)).full()),
                    ("max", &f.temperature(Some(max_c as f32)).full()),
                ],
            ),
        ),
        Evidence::FlowInRange {
            m3s,
            min_m3s,
            max_m3s,
        } => (
            s.get(Key::FactorFlowInRange),
            s.fmt(
                Key::WindowInRange,
                &[
                    ("value", &f.discharge(Some(m3s as f32)).full()),
                    ("min", &f.discharge(Some(min_m3s as f32)).full()),
                    ("max", &f.discharge(Some(max_m3s as f32)).full()),
                ],
            ),
        ),
    }
}

fn window_vm(f: &Formatter, w: &FishingWindow) -> WindowVm {
    WindowVm {
        start_t: w.start.0,
        end_t: w.end.0,
        range: f.clock_range(w.start, w.end),
        factors: w
            .matched
            .iter()
            .map(|m| {
                let (label, detail) = describe(f, &m.evidence);
                WindowFactorVm {
                    factor: m.factor,
                    label: label.to_owned(),
                    detail,
                }
            })
            .collect(),
        no_data: w
            .no_data
            .iter()
            .map(|factor| factor_label(f, *factor).to_owned())
            .collect(),
    }
}

/// The traditional-cue notice when a solunar period is among the reasons.
pub(in crate::view) fn notices(f: &Formatter, windows: &[FishingWindow]) -> Vec<Notice> {
    let solunar = windows
        .iter()
        .any(|w| w.matched.iter().any(|m| m.factor == Factor::Solunar));
    if solunar {
        vec![f.notice(NoticeKind::TraditionalCue)]
    } else {
        Vec::new()
    }
}

/// The sources of the factors that matched, and the on-device computation.
pub(in crate::view) fn credits(input: &ViewInput, windows: &[FishingWindow]) -> Vec<CreditVm> {
    let f = Formatter::for_input(input);
    let snapshot: &SpotSnapshot = &input.snapshot;
    let used = |factor: Factor| {
        windows
            .iter()
            .any(|w| w.matched.iter().any(|m| m.factor == factor))
    };
    let mut out = vec![computed_credit(f.strings)];
    let mut push = |credits: Vec<CreditVm>| {
        for c in credits {
            if !out.contains(&c) {
                out.push(c);
            }
        }
    };
    if used(Factor::TideStage)
        && let Some(tide) = &snapshot.marine.tide
    {
        push(tide_credits(&f, snapshot, &tide.provenance.source));
    }
    if used(Factor::CurrentStrength)
        && let Some(current) = &snapshot.marine.tidal_current
    {
        push(tide_credits(&f, snapshot, &current.provenance.source));
    }
    if used(Factor::PressureFalling) {
        if let Some(p) = &snapshot.marine.observed_pressure_hpa {
            push(station_credits(input, &[p.provenance()]));
        }
        if let Some(w) = &snapshot.weather {
            push(vec![weather_credit(w)]);
        }
    }
    let mut provs: Vec<&Provenance> = Vec::new();
    if used(Factor::WaterTemp)
        && let Some(t) = snapshot
            .water
            .temperature_c
            .as_ref()
            .or(snapshot.marine.sea_surface_temp_c.as_ref())
    {
        provs.push(t.provenance());
    }
    if used(Factor::FlowInRange)
        && let Some(d) = &snapshot.water.discharge_m3s
    {
        provs.push(d.provenance());
    }
    if !provs.is_empty() {
        push(station_credits(input, &provs));
    }
    out
}

pub fn build(input: &ViewInput) -> Option<WindowsCard> {
    if local_days(input, 1).is_empty() {
        return None;
    }
    let f = Formatter::for_input(input);
    let now = input.now;
    let windows = upcoming(input);
    let vms: Vec<(LocalDate, WindowVm)> =
        windows.iter().map(|w| (w.date, window_vm(&f, w))).collect();

    let first = windows.first();
    let mut lines = Vec::new();
    let mut headline = Quantity::default();
    if let Some((w, vm)) = first.zip(vms.first().map(|(_, vm)| vm)) {
        lines.push(if w.start <= now {
            f.strings.fmt(Key::WindowNow, &[("time", &f.clock(w.end))])
        } else {
            f.strings.fmt(Key::NextWindow, &[("range", &vm.range)])
        });
        let names: Vec<&str> = vm.factors.iter().map(|x| x.label.as_str()).collect();
        if !names.is_empty() {
            lines.push(names.join(", "));
        }
        headline = Quantity {
            value: None,
            text: if w.start <= now {
                f.strings.get(Key::NowText).to_owned()
            } else {
                f.clock(w.start)
            },
            unit: String::new(),
        };
    } else {
        lines.push(f.strings.get(Key::NoWindows).to_owned());
    }

    // Days in order, each with its windows.
    let mut days: Vec<WindowDayVm> = Vec::new();
    for (date, vm) in vms {
        match days.last_mut() {
            Some(day) if day.date == day_label(&f, date) => day.windows.push(vm),
            _ => days.push(WindowDayVm {
                date: day_label(&f, date),
                windows: vec![vm],
            }),
        }
    }

    let notices = notices(&f, &windows);
    let title = f.strings.card_title(CardId::Windows);
    let mut accessibility = format!("{title}: {}", lines.join(". "));
    for n in &notices {
        accessibility.push_str(". ");
        accessibility.push_str(&n.text);
    }
    Some(WindowsCard {
        chrome: CardChrome {
            id: CardId::Windows,
            title: title.to_owned(),
            footnote: None,
            credits: credits(input, &windows),
            state: CardState::Fresh,
            notices,
            accessibility,
        },
        compact: Compact {
            headline,
            lines,
            spark: Vec::new(),
        },
        full: WindowsFull {
            days,
            sections: Vec::new(),
        },
    })
}

#[cfg(test)]
pub(in crate::view) mod tests {
    use super::*;
    use crate::view::Voice;
    use crate::view::tests::sample_input;
    use whether_core::presentation::TimeFormat;

    /// The Battery with its recorded tide predictions and the computed sky.
    pub(in crate::view) fn battery() -> ViewInput {
        crate::view::timeline::tests::input()
    }

    #[test]
    fn windows_list_their_factors_with_numbers() {
        let card = build(&battery()).unwrap();
        assert_ne!(card.full.days.len(), 0);
        let w = &card.full.days[0].windows[0];
        assert!(w.factors.len() >= 2);
        assert!(card.compact.lines[0].starts_with("Next snack time"));
        assert!(w.start_t < w.end_t);
        // Nothing is a bare factor name: every detail carries its numbers.
        assert!(
            w.factors
                .iter()
                .all(|x| x.detail.chars().any(|c| c.is_ascii_digit()))
        );
    }

    #[test]
    fn solunar_reasons_carry_the_traditional_cue() {
        let mut input = battery();
        input.windows.weights.solunar = 5.0;
        let card = build(&input).unwrap();
        let has = card
            .full
            .days
            .iter()
            .flat_map(|d| &d.windows)
            .flat_map(|w| &w.factors)
            .any(|x| x.factor == Factor::Solunar);
        assert!(has);
        assert!(
            card.chrome
                .notices
                .iter()
                .any(|n| n.kind == NoticeKind::TraditionalCue)
        );
        assert!(card.chrome.accessibility.contains("traditional cue"));
    }

    #[test]
    fn a_spot_with_no_windows_says_so() {
        let mut input = sample_input();
        input.windows.weights = crate::trip::windows::FactorWeights {
            tide_stage: 0.0,
            current_strength: 0.0,
            low_light: 0.0,
            solunar: 0.0,
            pressure_falling: 0.0,
            water_temp: 0.0,
            flow_in_range: 0.0,
        };
        let card = build(&input).unwrap();
        assert_eq!(card.compact.lines, ["No snack times in this period."]);
        assert_eq!(card.full.days.len(), 0);
        assert_eq!(card.compact.headline, Quantity::default());
    }

    #[test]
    fn snapshots_in_both_voices_and_unit_systems() {
        let mut input = battery();
        insta::assert_json_snapshot!("windows_luna_us", build(&input).unwrap());
        input.voice = Voice::Plain;
        input.clock = TimeFormat::H24;
        input.units = crate::view::cards::flow::fixtures::metric();
        let card = build(&input).unwrap();
        assert!(card.compact.lines[0].starts_with("Next fishing window"));
        insta::assert_json_snapshot!("windows_plain_metric", card);
    }

    #[test]
    fn a_window_in_progress_reads_now() {
        let mut input = battery();
        let first = build(&input).unwrap().full.days[0].windows[0].start_t;
        input.now = UnixTime(first + 600);
        let card = build(&input).unwrap();
        assert_eq!(card.compact.headline.text, "now");
        assert!(card.compact.lines[0].starts_with("Snack time now"));
    }
}
