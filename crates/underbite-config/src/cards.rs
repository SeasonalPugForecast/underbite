//! Card order and visibility, with an override per water type.
//!
//! Card ids this version does not know (a card a newer or older version had) are dropped when
//! the file is read and reported by validation as warnings, so an old or foreign file always
//! loads.

use schemars::JsonSchema;
use serde::de::value::{Error as ValueError, StrDeserializer};
use serde::{Deserialize, Serialize};
use underbite_core::model::WaterType;
use underbite_core::view::CardId;

use crate::schema::Issues;

/// A card order and the cards hidden from it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, JsonSchema)]
pub struct CardLayout {
    /// Cards in display order. In a water-type override, empty means the base order.
    pub order: Vec<CardId>,
    /// Cards never shown.
    pub hidden: Vec<CardId>,
    /// Set on a layout this version ships for a water type (rivers, ponds and ice): the user
    /// did not write it, so the base hidden list applies to it, and a base order the user
    /// changed replaces its order. Never written to a file and not part of the schema.
    #[serde(skip)]
    #[schemars(skip)]
    pub inherited: bool,
}

impl CardLayout {
    /// A layout this version ships.
    fn shipped(order: &[CardId]) -> Self {
        Self {
            order: order.to_vec(),
            hidden: Vec::new(),
            inherited: true,
        }
    }
}

// serde's `skip_serializing_if` hands over a reference to the field.
#[allow(clippy::ref_option)]
fn is_inherited(layout: &Option<CardLayout>) -> bool {
    layout.as_ref().is_none_or(|l| l.inherited)
}

/// `[cards]`; a `[cards.<water type>]` table overrides the base layout for that water type.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(from = "RawCards")]
pub struct CardsConfig {
    pub order: Vec<CardId>,
    pub hidden: Vec<CardId>,
    /// `river_stream`.
    #[serde(skip_serializing_if = "is_inherited")]
    pub river: Option<CardLayout>,
    /// `lake_reservoir`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lake: Option<CardLayout>,
    /// `tidal_river_estuary`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tidal: Option<CardLayout>,
    /// `bay_inshore`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bay: Option<CardLayout>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub surf: Option<CardLayout>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nearshore: Option<CardLayout>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offshore: Option<CardLayout>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub great_lakes: Option<CardLayout>,
    #[serde(skip_serializing_if = "is_inherited")]
    pub pond: Option<CardLayout>,
    #[serde(skip_serializing_if = "is_inherited")]
    pub ice: Option<CardLayout>,
    /// Ids dropped on reading, as (path, id); never written.
    #[serde(skip)]
    #[schemars(skip)]
    pub dropped: Vec<(String, String)>,
}

impl Default for CardsConfig {
    fn default() -> Self {
        use CardId::{
            Alerts, Clarity, Current, Flow, MarineForecast, Now, Sources, Stocking, StormAhead,
            SunMoon, Tide, WaterTemp, Waves, Wind, Windows,
        };
        // Alerts sit near the top everywhere: a card shows only when it has something to say.
        let fresh = CardLayout::shipped(&[
            Now, Alerts, Flow, WaterTemp, Clarity, Windows, Wind, SunMoon, Stocking, Sources,
        ]);
        Self {
            order: vec![
                Now,
                Alerts,
                StormAhead,
                Windows,
                Tide,
                Current,
                Wind,
                WaterTemp,
                Waves,
                MarineForecast,
                Flow,
                SunMoon,
                Sources,
            ],
            hidden: Vec::new(),
            river: Some(fresh.clone()),
            lake: None,
            tidal: None,
            bay: None,
            surf: None,
            nearshore: None,
            offshore: None,
            great_lakes: None,
            pond: Some(fresh.clone()),
            ice: Some(fresh),
            dropped: Vec::new(),
        }
    }
}

impl CardsConfig {
    /// The override for a water type, if there is one.
    pub fn layout(&self, water_type: WaterType) -> Option<&CardLayout> {
        match water_type {
            WaterType::RiverStream => &self.river,
            WaterType::LakeReservoir => &self.lake,
            WaterType::TidalRiverEstuary => &self.tidal,
            WaterType::BayInshore => &self.bay,
            WaterType::Surf => &self.surf,
            WaterType::Nearshore => &self.nearshore,
            WaterType::Offshore => &self.offshore,
            WaterType::GreatLakes => &self.great_lakes,
            WaterType::Pond => &self.pond,
            WaterType::Ice => &self.ice,
        }
        .as_ref()
    }

    /// The cards to show for a water type, in order, hidden ones removed. An override the user
    /// wrote replaces the base hidden list as well as the base order (an empty override order
    /// keeps the base order). A layout this version ships takes the base hidden list, and keeps
    /// its own order only while the base order is the shipped one.
    pub fn order_for(&self, water_type: WaterType) -> Vec<CardId> {
        let (order, hidden) = match self.layout(water_type) {
            Some(l) if l.inherited => {
                let base_is_shipped = self.order == Self::default().order;
                if base_is_shipped && !l.order.is_empty() {
                    (&l.order, &self.hidden)
                } else {
                    (&self.order, &self.hidden)
                }
            }
            Some(l) if l.order.is_empty() => (&self.order, &l.hidden),
            Some(l) => (&l.order, &l.hidden),
            None => (&self.order, &self.hidden),
        };
        order
            .iter()
            .filter(|c| !hidden.contains(c))
            .copied()
            .collect()
    }

    pub(crate) fn validate(&self, issues: &mut Issues) {
        for (path, id) in &self.dropped {
            issues.warning(path, format!("unknown card `{id}` was dropped"));
        }
        let layouts = [
            (
                "cards",
                Some(CardLayout {
                    order: self.order.clone(),
                    hidden: self.hidden.clone(),
                    inherited: false,
                }),
            ),
            ("cards.river", self.river.clone()),
            ("cards.lake", self.lake.clone()),
            ("cards.tidal", self.tidal.clone()),
            ("cards.bay", self.bay.clone()),
            ("cards.surf", self.surf.clone()),
            ("cards.nearshore", self.nearshore.clone()),
            ("cards.offshore", self.offshore.clone()),
            ("cards.great_lakes", self.great_lakes.clone()),
            ("cards.pond", self.pond.clone()),
            ("cards.ice", self.ice.clone()),
        ];
        for (path, layout) in layouts {
            let Some(layout) = layout else { continue };
            for (i, id) in layout.order.iter().enumerate() {
                if layout.order[..i].contains(id) {
                    issues.warning(format!("{path}.order[{i}]"), "card listed twice");
                }
            }
        }
    }
}

#[derive(Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
/// A card order and hidden cards, by card id; ids this version does not know are dropped.
struct RawLayout {
    order: Vec<String>,
    hidden: Vec<String>,
}

#[derive(Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
/// The card layout, by card id; ids this version does not know are dropped with a warning.
struct RawCards {
    order: Option<Vec<String>>,
    hidden: Vec<String>,
    river: Option<RawLayout>,
    lake: Option<RawLayout>,
    tidal: Option<RawLayout>,
    bay: Option<RawLayout>,
    surf: Option<RawLayout>,
    nearshore: Option<RawLayout>,
    offshore: Option<RawLayout>,
    great_lakes: Option<RawLayout>,
    pond: Option<RawLayout>,
    ice: Option<RawLayout>,
}

fn ids(raw: Vec<String>, path: &str, dropped: &mut Vec<(String, String)>) -> Vec<CardId> {
    let mut out = Vec::with_capacity(raw.len());
    for (i, s) in raw.into_iter().enumerate() {
        match CardId::deserialize(StrDeserializer::<ValueError>::new(&s)) {
            Ok(id) => out.push(id),
            Err(_) => dropped.push((format!("{path}[{i}]"), s)),
        }
    }
    out
}

impl From<RawCards> for CardsConfig {
    fn from(raw: RawCards) -> Self {
        let defaults = Self::default();
        let mut dropped = Vec::new();
        let order = match raw.order {
            Some(o) => ids(o, "cards.order", &mut dropped),
            None => defaults.order.clone(),
        };
        let hidden = ids(raw.hidden, "cards.hidden", &mut dropped);
        let mut layout = |name: &str, r: Option<RawLayout>, default: &Option<CardLayout>| match r {
            Some(r) => Some(CardLayout {
                order: ids(r.order, &format!("cards.{name}.order"), &mut dropped),
                hidden: ids(r.hidden, &format!("cards.{name}.hidden"), &mut dropped),
                inherited: false,
            }),
            None => default.clone(),
        };
        let river = layout("river", raw.river, &defaults.river);
        let lake = layout("lake", raw.lake, &defaults.lake);
        let tidal = layout("tidal", raw.tidal, &defaults.tidal);
        let bay = layout("bay", raw.bay, &defaults.bay);
        let surf = layout("surf", raw.surf, &defaults.surf);
        let nearshore = layout("nearshore", raw.nearshore, &defaults.nearshore);
        let offshore = layout("offshore", raw.offshore, &defaults.offshore);
        let great_lakes = layout("great_lakes", raw.great_lakes, &defaults.great_lakes);
        let pond = layout("pond", raw.pond, &defaults.pond);
        let ice = layout("ice", raw.ice, &defaults.ice);
        Self {
            order,
            hidden,
            river,
            lake,
            tidal,
            bay,
            surf,
            nearshore,
            offshore,
            great_lakes,
            pond,
            ice,
            dropped,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storm_ahead_is_on_for_salt_and_lakes_and_off_for_rivers() {
        let c = CardsConfig::default();
        for wt in [
            WaterType::BayInshore,
            WaterType::Surf,
            WaterType::Nearshore,
            WaterType::Offshore,
            WaterType::TidalRiverEstuary,
            WaterType::GreatLakes,
            WaterType::LakeReservoir,
        ] {
            assert!(c.order_for(wt).contains(&CardId::StormAhead), "{wt:?}");
        }
        assert!(
            !c.order_for(WaterType::RiverStream)
                .contains(&CardId::StormAhead)
        );
    }

    #[test]
    fn every_default_order_has_the_summary_alerts_and_sources() {
        let c = CardsConfig::default();
        for wt in [
            WaterType::RiverStream,
            WaterType::LakeReservoir,
            WaterType::TidalRiverEstuary,
            WaterType::BayInshore,
            WaterType::Surf,
            WaterType::Nearshore,
            WaterType::Offshore,
            WaterType::GreatLakes,
            WaterType::Pond,
            WaterType::Ice,
        ] {
            let order = c.order_for(wt);
            assert_eq!(order.first(), Some(&CardId::Now), "{wt:?}");
            assert_eq!(order.get(1), Some(&CardId::Alerts), "{wt:?}");
            assert_eq!(order.last(), Some(&CardId::Sources), "{wt:?}");
        }
        assert!(
            c.order_for(WaterType::BayInshore)
                .contains(&CardId::MarineForecast)
        );
    }

    #[test]
    fn unknown_ids_are_dropped_and_reported() {
        let c: CardsConfig =
            toml::from_str("order = [\"tide\", \"bogus\", \"wind\"]\n[river]\nhidden = [\"nope\"]")
                .unwrap();
        assert_eq!(c.order, [CardId::Tide, CardId::Wind]);
        assert_eq!(
            c.dropped[0],
            ("cards.order[1]".to_owned(), "bogus".to_owned())
        );
        let mut issues = Issues::default();
        c.validate(&mut issues);
        assert_eq!(issues.0.len(), 2);
        assert_eq!(issues.0[1].path, "cards.river.hidden[0]");
    }

    #[test]
    fn hidden_cards_and_inheriting_overrides() {
        let c: CardsConfig = toml::from_str(
            "order = [\"tide\", \"wind\"]\nhidden = [\"wind\"]\n[bay]\nhidden = [\"tide\"]",
        )
        .unwrap();
        assert_eq!(c.order_for(WaterType::Surf), [CardId::Tide]);
        assert_eq!(c.order_for(WaterType::BayInshore), [CardId::Wind]);
    }

    fn parsed(text: &str) -> CardsConfig {
        toml::from_str(text).unwrap()
    }

    #[test]
    fn a_hidden_card_stays_hidden_for_the_shipped_water_types() {
        let c = parsed("hidden = [\"wind\"]");
        for wt in [WaterType::RiverStream, WaterType::Pond, WaterType::Ice] {
            let order = c.order_for(wt);
            assert!(!order.contains(&CardId::Wind), "{wt:?}");
            // The shipped order is kept.
            assert_eq!(order.first(), Some(&CardId::Now), "{wt:?}");
            assert!(order.contains(&CardId::Clarity), "{wt:?}");
        }
        assert!(!c.order_for(WaterType::BayInshore).contains(&CardId::Wind));
    }

    #[test]
    fn the_shipped_orders_stay_when_the_user_sets_nothing() {
        let c = parsed("");
        assert_eq!(c, CardsConfig::default());
        assert_eq!(c.order_for(WaterType::RiverStream)[2], CardId::Flow);
        assert_eq!(c.order_for(WaterType::BayInshore)[2], CardId::StormAhead);
        // A user override of its own hidden list still replaces the base list.
        let c = parsed("hidden = [\"wind\"]\n[river]\nhidden = [\"flow\"]");
        let river = c.order_for(WaterType::RiverStream);
        assert!(river.contains(&CardId::Wind));
        assert!(!river.contains(&CardId::Flow));
    }

    #[test]
    fn a_user_base_order_reaches_water_types_without_their_own_override() {
        let c = parsed("order = [\"tide\", \"wind\", \"sources\"]");
        assert_eq!(
            c.order_for(WaterType::RiverStream),
            [CardId::Tide, CardId::Wind, CardId::Sources]
        );
        assert_eq!(c.order_for(WaterType::Pond), c.order_for(WaterType::Ice));
        // A user override order still wins.
        let c = parsed("order = [\"tide\"]\n[river]\norder = [\"flow\", \"wind\"]");
        assert_eq!(
            c.order_for(WaterType::RiverStream),
            [CardId::Flow, CardId::Wind]
        );
        assert_eq!(c.order_for(WaterType::Pond), [CardId::Tide]);
    }

    #[test]
    fn shipped_layouts_are_not_written_and_survive_a_round_trip() {
        let c = CardsConfig {
            hidden: vec![CardId::Wind],
            ..CardsConfig::default()
        };
        let text = toml::to_string(&c).unwrap();
        assert!(!text.contains("river"), "{text}");
        let back: CardsConfig = toml::from_str(&text).unwrap();
        assert!(
            !back
                .order_for(WaterType::RiverStream)
                .contains(&CardId::Wind)
        );
        // A layout the user wrote is written.
        let mine = parsed("[river]\norder = [\"flow\"]");
        assert!(toml::to_string(&mine).unwrap().contains("river"));
    }

    #[test]
    fn unknown_keys_are_rejected() {
        assert!(toml::from_str::<CardsConfig>("[ocean]\norder = []").is_err());
        assert!(toml::from_str::<CardsConfig>("[river]\nshow = []").is_err());
    }
}
