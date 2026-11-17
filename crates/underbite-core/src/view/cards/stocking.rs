//! The stocking card: recent and planned fish stockings of waterbodies near the spot.
//!
//! The numbers are the agency's estimates and the list of completed stockings lags about a year;
//! a planned stocking is a schedule whose date is the list's own text (it carries no year).

use schemars::JsonSchema;
use serde::Serialize;

use crate::model::{StockingEvent, StockingWhen};
use crate::units::HeightUnit;
use crate::view::format::Formatter;
use crate::view::strings::Key;
use crate::view::{CardChrome, CardId, CardState, Compact, CreditVm, Quantity, Section, ViewInput};

use super::flow::row;

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct StockingCard {
    pub chrome: CardChrome,
    pub compact: Compact,
    pub full: StockingFull,
}

/// One stocking, ready to show.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct StockingVm {
    pub waterbody: String,
    /// "Yorktown, Westchester".
    pub place: Option<String>,
    /// The species as the list writes it.
    pub species: String,
    /// "Apr 2026", or "planned: Week of April 14".
    pub when: String,
    /// "Brown Trout · Apr 2026 · about 1,200 · 8 to 12 in".
    pub summary: String,
    pub planned: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct StockingFull {
    /// Newest first.
    pub recent: Vec<StockingVm>,
    pub planned: Vec<StockingVm>,
    /// "No recent stocking reported nearby" when there is none of either.
    pub empty: Option<String>,
    /// The estimate and lag notes, and the planned note when there are planned rows.
    pub notes: Vec<String>,
    pub sections: Vec<Section>,
}

/// Fish lengths are read in inches with feet and in centimetres with metres.
fn size_text(f: &Formatter, e: &StockingEvent) -> Option<String> {
    let r = e.size_m?;
    let (factor, key) = match f.units.height {
        HeightUnit::Ft => (100.0 / 2.54, Key::UnitInches),
        HeightUnit::M => (100.0, Key::UnitCentimetres),
    };
    let unit = f.strings.get(key);
    let (lo, hi) = (r.min_m * factor, r.max_m * factor);
    let (lo, hi) = (f.strings.number(lo, 0), f.strings.number(hi, 0));
    let range = if lo == hi {
        lo
    } else {
        f.strings.fmt(Key::Range, &[("from", &lo), ("to", &hi)])
    };
    Some(format!("{range} {unit}"))
}

fn vm(f: &Formatter, e: &StockingEvent) -> StockingVm {
    let when = match &e.when {
        StockingWhen::Month { year, month } => f.strings.fmt(
            Key::StockingMonthYear,
            &[
                ("month", f.strings.month(*month)),
                ("year", &year.to_string()),
            ],
        ),
        StockingWhen::DateText(t) => f.strings.fmt(Key::StockingPlannedWhen, &[("when", t)]),
    };
    let mut details = vec![when.clone()];
    if let Some(n) = e.number_estimate {
        details.push(f.strings.fmt(
            Key::StockingAbout,
            &[("n", &f.strings.grouped(n as f32, 0))],
        ));
    }
    details.extend(size_text(f, e));
    let place = match (&e.town, &e.county) {
        (Some(t), Some(c)) => Some(format!("{t}, {c}")),
        (Some(p), None) | (None, Some(p)) => Some(p.clone()),
        (None, None) => None,
    };
    StockingVm {
        waterbody: e.waterbody.clone(),
        place,
        species: e.species_text.clone(),
        when,
        summary: f.strings.fmt(
            Key::StockingSummary,
            &[
                ("species", &e.species_text),
                ("details", &details.join(" · ")),
            ],
        ),
        planned: e.planned,
    }
}

/// Months since year 0, to order the actual stockings newest first.
fn month_index(e: &StockingEvent) -> i32 {
    match e.when {
        StockingWhen::Month { year, month } => i32::from(year) * 12 + i32::from(month),
        StockingWhen::DateText(_) => 0,
    }
}

#[allow(clippy::too_many_lines)]
pub fn build(input: &ViewInput) -> Option<StockingCard> {
    let f = Formatter::for_input(input);
    let events = &input.snapshot.stocking;

    let mut done: Vec<&StockingEvent> = events.iter().filter(|e| !e.planned).collect();
    done.sort_by_key(|e| std::cmp::Reverse(month_index(e)));
    let planned: Vec<&StockingEvent> = events.iter().filter(|e| e.planned).collect();
    let recent: Vec<StockingVm> = done.iter().map(|e| vm(&f, e)).collect();
    let planned_vm: Vec<StockingVm> = planned.iter().map(|e| vm(&f, e)).collect();

    let none = f.strings.get(Key::StockingNone).to_owned();
    let empty = (events.is_empty()).then(|| none.clone());
    let mut notes = Vec::new();
    if !events.is_empty() {
        notes.push(f.strings.get(Key::StockingEstimateNote).to_owned());
    }
    if !recent.is_empty() {
        notes.push(f.strings.get(Key::StockingLagNote).to_owned());
    }
    if !planned_vm.is_empty() {
        notes.push(f.strings.get(Key::StockingPlannedNote).to_owned());
    }

    let section = |key: Key, items: &[StockingVm]| Section {
        title: f.strings.get(key).to_owned(),
        rows: items
            .iter()
            .map(|s| {
                let mut r = row(&s.waterbody, &s.summary);
                r.detail.clone_from(&s.place);
                r
            })
            .collect(),
    };
    let mut sections = Vec::new();
    if !recent.is_empty() {
        sections.push(section(Key::StockingSectionRecent, &recent));
    }
    if !planned_vm.is_empty() {
        sections.push(section(Key::StockingSectionPlanned, &planned_vm));
    }

    let count = events.len();
    let headline = if count == 0 {
        Quantity {
            value: None,
            text: f.strings.get(Key::Missing).to_owned(),
            unit: String::new(),
        }
    } else {
        Quantity {
            value: Some(count as f32),
            text: count.to_string(),
            unit: f
                .strings
                .get(if count == 1 {
                    Key::StockingUnitOne
                } else {
                    Key::StockingUnitMany
                })
                .to_owned(),
        }
    };
    let mut lines: Vec<String> = Vec::new();
    match (recent.first(), planned_vm.first()) {
        (None, None) => lines.push(none.clone()),
        (r, p) => {
            lines.extend(r.map(|s| s.summary.clone()));
            lines.extend(p.map(|s| s.summary.clone()));
        }
    }
    lines.truncate(2);

    // The credit: the snapshot's own attribution, else the source's name.
    let mut credits: Vec<CreditVm> = Vec::new();
    for e in events {
        let name = f.strings.source_name(&e.source);
        let found = input
            .snapshot
            .attribution
            .iter()
            .find(|a| a.source == e.source);
        let credit = CreditVm {
            source: name.clone(),
            text: found.map_or(name, |a| a.credit.clone()),
            link: found.and_then(|a| a.url.clone()),
            licence: None,
        };
        if !credits.contains(&credit) {
            credits.push(credit);
        }
    }

    let title = f.strings.card_title(CardId::Stocking).to_owned();
    let accessibility = format!(
        "{title}: {}",
        lines.first().map_or(none.as_str(), String::as_str)
    );
    Some(StockingCard {
        chrome: CardChrome {
            id: CardId::Stocking,
            title,
            footnote: None,
            credits,
            state: CardState::Fresh,
            notices: Vec::new(),
            accessibility,
        },
        compact: Compact {
            headline,
            lines,
            spark: Vec::new(),
        },
        full: StockingFull {
            recent,
            planned: planned_vm,
            empty,
            notes,
            sections,
        },
    })
}

#[cfg(test)]
pub(in crate::view) mod tests {
    use super::*;
    use crate::model::LengthRange;
    use crate::provenance::{Attribution, SourceId};
    use crate::view::Voice;
    use crate::view::cards::flow::fixtures::{input, metric, us};

    fn event(when: StockingWhen, species: &str, planned: bool) -> StockingEvent {
        StockingEvent {
            waterbody: "Croton River".into(),
            county: Some("Westchester".into()),
            town: Some("Yorktown".into()),
            when,
            species_text: species.into(),
            number_estimate: Some(1_200),
            size_m: Some(LengthRange {
                min_m: 0.203,
                max_m: 0.305,
            }),
            planned,
            source: SourceId::NY_DEC_STOCKING,
        }
    }

    fn month(year: u16, month: u8) -> StockingWhen {
        StockingWhen::Month { year, month }
    }

    pub(in crate::view) fn with_events(units: crate::view::DisplayUnits) -> ViewInput {
        let mut input = input(units, Voice::Luna);
        input.snapshot.attribution.push(Attribution {
            source: SourceId::NY_DEC_STOCKING,
            credit: "NYS DEC fish stocking lists".into(),
            url: Some("https://data.ny.gov".into()),
        });
        input.snapshot.stocking = vec![
            event(month(2025, 10), "Brook Trout", false),
            event(month(2026, 4), "Brown Trout", false),
            StockingEvent {
                number_estimate: None,
                size_m: None,
                ..event(
                    StockingWhen::DateText("Week of April 14".into()),
                    "Rainbow Trout",
                    true,
                )
            },
        ];
        input
    }

    fn card(input: &ViewInput) -> StockingCard {
        build(input).expect("a stocking card")
    }

    #[test]
    fn recent_stockings_come_newest_first_with_the_estimate_and_lag_notes() {
        let c = card(&with_events(us()));
        assert_eq!(c.full.recent.len(), 2);
        assert_eq!(
            c.full.recent[0].summary,
            "Brown Trout · Apr 2026 · about 1,200 · 8 to 12 in"
        );
        assert_eq!(c.full.recent[1].when, "Oct 2025");
        assert_eq!(
            c.full.recent[0].place.as_deref(),
            Some("Yorktown, Westchester")
        );
        assert_eq!(c.full.notes.len(), 3);
        assert!(c.full.notes[0].contains("estimates"));
        assert!(c.full.notes[1].contains("lags about a year"));
        assert_eq!(c.compact.headline.full(), "3 stockings");
        assert_eq!(c.chrome.credits.len(), 1);
        assert_eq!(c.chrome.credits[0].text, "NYS DEC fish stocking lists");
    }

    #[test]
    fn a_planned_stocking_says_so_and_keeps_the_lists_own_date_text() {
        let c = card(&with_events(us()));
        assert_eq!(c.full.planned.len(), 1);
        assert!(c.full.planned[0].planned);
        assert_eq!(c.full.planned[0].when, "planned: Week of April 14");
        assert_eq!(c.compact.lines.len(), 2);
        assert!(c.compact.lines[1].starts_with("Rainbow Trout · planned:"));
        assert!(c.full.notes[2].contains("Planned"));
    }

    #[test]
    fn sizes_are_centimetres_in_metric() {
        let c = card(&with_events(metric()));
        assert!(
            c.full.recent[0]
                .summary
                .ends_with("about 1,200 · 20 to 31 cm")
        );
    }

    #[test]
    fn no_stocking_is_a_message_not_an_empty_card() {
        let mut i = with_events(us());
        i.snapshot.stocking.clear();
        let c = card(&i);
        assert_eq!(
            c.full.empty.as_deref(),
            Some("No recent stocking reported nearby")
        );
        assert_eq!(c.compact.lines, ["No recent stocking reported nearby"]);
        assert_eq!(c.full.notes.len(), 0);
        assert_eq!(c.chrome.credits.len(), 0);
    }

    #[test]
    fn snapshots_with_stockings_and_with_none() {
        insta::assert_json_snapshot!("stocking_us", card(&with_events(us())));
        insta::assert_json_snapshot!("stocking_metric", card(&with_events(metric())));
        let mut i = with_events(us());
        i.snapshot.stocking.clear();
        insta::assert_json_snapshot!("stocking_none", card(&i));
    }
}
