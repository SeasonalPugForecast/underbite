//! The current conditions at a glance.

use schemars::JsonSchema;
use serde::Serialize;

use crate::model::WaterType;
use crate::view::cards::{current, flow, solunar, sun_moon, tide, water_temp, waves, wind};
use crate::view::format::Formatter;
use crate::view::strings::{Key, Strings};
use crate::view::{
    CardChrome, CardId, CardState, Compact, CreditVm, Notice, Quantity, ViewInput, Voice,
};

/// The most items the card summarises.
const MAX_ITEMS: usize = 5;

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct NowCard {
    pub chrome: CardChrome,
    pub compact: Compact,
    pub full: NowFull,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct NowFull {
    pub items: Vec<NowItem>,
}

/// The headline of another card.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct NowItem {
    pub card: CardId,
    pub label: String,
    pub headline: Quantity,
    pub line: Option<String>,
    pub state: CardState,
}

/// The cards that matter first for the water, most important first.
const fn priority(water_type: WaterType) -> &'static [CardId] {
    match water_type {
        WaterType::RiverStream | WaterType::LakeReservoir | WaterType::Pond | WaterType::Ice => &[
            CardId::WaterTemp,
            CardId::Flow,
            CardId::Wind,
            CardId::SunMoon,
            CardId::Solunar,
        ],
        WaterType::TidalRiverEstuary | WaterType::BayInshore => &[
            CardId::Tide,
            CardId::WaterTemp,
            CardId::Current,
            CardId::Wind,
            CardId::SunMoon,
            CardId::Solunar,
        ],
        WaterType::Surf | WaterType::Nearshore => &[
            CardId::Waves,
            CardId::Tide,
            CardId::WaterTemp,
            CardId::Wind,
            CardId::SunMoon,
            CardId::Solunar,
        ],
        WaterType::Offshore | WaterType::GreatLakes => &[
            CardId::Waves,
            CardId::WaterTemp,
            CardId::Wind,
            CardId::Current,
            CardId::SunMoon,
            CardId::Solunar,
        ],
    }
}

/// The compact form of a card, its title, state, credits and notices, if the card is built.
struct Part {
    id: CardId,
    chrome: CardChrome,
    compact: Compact,
}

fn part(id: CardId, input: &ViewInput) -> Option<Part> {
    macro_rules! of {
        ($card:expr) => {
            $card.map(|c| Part {
                id,
                chrome: c.chrome,
                compact: c.compact,
            })
        };
    }
    match id {
        CardId::Tide => of!(tide::build(input)),
        CardId::Current => of!(current::build(input)),
        CardId::Flow => of!(flow::build(input)),
        CardId::WaterTemp => of!(water_temp::build(input)),
        CardId::Waves => of!(waves::build(input)),
        CardId::Wind => of!(wind::build(input)),
        CardId::SunMoon => of!(sun_moon::build(input)),
        CardId::Solunar => of!(solunar::build(input)),
        _ => None,
    }
}

/// What matters now, gathered from the other cards in a fixed order for the water type. Values
/// are the cards' own; a card that is not built is skipped and nothing is filled in for it.
pub fn build(input: &ViewInput) -> Option<NowCard> {
    let f = Formatter::for_input(input);
    let title = f.strings.card_title(CardId::Now).to_owned();
    let parts: Vec<Part> = priority(input.snapshot.spot.water_type)
        .iter()
        .filter_map(|id| part(*id, input))
        .take(MAX_ITEMS)
        .collect();

    let Some(first) = parts.first() else {
        let reason = f.strings.get(Key::NoData).to_owned();
        return Some(NowCard {
            chrome: CardChrome {
                id: CardId::Now,
                title,
                footnote: None,
                credits: Vec::new(),
                state: CardState::Unavailable {
                    reason: reason.clone(),
                },
                notices: Vec::new(),
                accessibility: Strings::new(Voice::Plain).get(Key::NoData).to_owned(),
            },
            compact: Compact {
                headline: Quantity {
                    value: None,
                    text: f.strings.get(Key::Missing).to_owned(),
                    unit: String::new(),
                },
                lines: Vec::new(),
                spark: Vec::new(),
            },
            full: NowFull { items: Vec::new() },
        });
    };

    let items: Vec<NowItem> = parts
        .iter()
        .map(|p| NowItem {
            card: p.id,
            label: p.chrome.title.clone(),
            headline: p.compact.headline.clone(),
            line: p.compact.lines.first().cloned(),
            state: p.chrome.state.clone(),
        })
        .collect();

    let mut credits: Vec<CreditVm> = Vec::new();
    let mut notices: Vec<Notice> = Vec::new();
    for p in &parts {
        for c in &p.chrome.credits {
            if !credits.contains(c) {
                credits.push(c.clone());
            }
        }
        for n in &p.chrome.notices {
            if !notices.iter().any(|x| x.kind == n.kind) {
                notices.push(n.clone());
            }
        }
    }

    // The compact form leads with the first item and names the next two with their values.
    let lines = parts
        .iter()
        .skip(1)
        .take(2)
        .map(|p| format!("{} {}", p.chrome.title, p.compact.headline.full()))
        .collect();
    // Plain words only: the lines under the headlines are in the user's voice.
    let accessibility = std::iter::once(format!("{title}."))
        .chain(
            parts
                .iter()
                .map(|p| format!("{} {}.", p.chrome.title, p.compact.headline.full())),
        )
        .collect::<Vec<_>>()
        .join(" ");
    let state = first.chrome.state.clone();
    Some(NowCard {
        chrome: CardChrome {
            id: CardId::Now,
            title,
            footnote: None,
            credits,
            state,
            notices,
            accessibility,
        },
        compact: Compact {
            headline: first.compact.headline.clone(),
            lines,
            spark: Vec::new(),
        },
        full: NowFull { items },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{MarineConditions, WaterConditions};
    use crate::view::tests::sample_input;

    #[test]
    fn a_tidal_spot_leads_with_the_tide() {
        let input = sample_input();
        let card = build(&input).unwrap();
        let ids: Vec<CardId> = card.full.items.iter().map(|i| i.card).collect();
        assert_eq!(ids.first(), Some(&CardId::Tide));
        assert!(ids.len() <= MAX_ITEMS);
        // Each item is the other card's own headline, not a new value.
        let tide = tide::build(&input).unwrap();
        assert_eq!(card.full.items[0].headline, tide.compact.headline);
        assert_eq!(card.compact.headline, tide.compact.headline);
        assert!(card.compact.lines.len() <= 2);
        insta::assert_json_snapshot!("now_harbor_luna", card);
    }

    #[test]
    fn a_river_leads_with_water_temperature_and_flow() {
        let mut input = sample_input();
        input.snapshot.spot.water_type = WaterType::RiverStream;
        input.voice = Voice::Plain;
        let card = build(&input).unwrap();
        let ids: Vec<CardId> = card.full.items.iter().map(|i| i.card).collect();
        assert_eq!(&ids[..2], [CardId::WaterTemp, CardId::Flow]);
        insta::assert_json_snapshot!("now_river_plain", card);
    }

    #[test]
    fn an_empty_spot_gets_an_empty_state_card() {
        let mut input = sample_input();
        input.snapshot.water = WaterConditions::default();
        input.snapshot.marine = MarineConditions::default();
        input.snapshot.weather = None;
        input.snapshot.sky = crate::model::SkyConditions::default();
        for voice in [Voice::Luna, Voice::Plain] {
            input.voice = voice;
            let card = build(&input).unwrap();
            assert_eq!(card.full.items.len(), 0);
            assert_eq!(card.compact.headline.value, None);
            assert!(matches!(card.chrome.state, CardState::Unavailable { .. }));
        }
        input.voice = Voice::Luna;
        insta::assert_json_snapshot!("now_empty_luna", build(&input).unwrap());
    }
}
