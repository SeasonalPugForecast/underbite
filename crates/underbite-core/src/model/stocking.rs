//! Fish stocking records for a spot's waterbody.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::provenance::SourceId;

/// When a stocking took place, as precisely as the list says.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StockingWhen {
    /// A calendar month (`month` 1 to 12), as the actual-stocking list gives it.
    Month { year: u16, month: u8 },
    /// Text the list gives for a planned date ("Week of April 14"); never parsed.
    DateText(String),
}

/// A length range in metres.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LengthRange {
    pub min_m: f32,
    pub max_m: f32,
}

/// One stocking of a waterbody. Numbers are the agency's estimates.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StockingEvent {
    /// The waterbody as the list names it.
    pub waterbody: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub county: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub town: Option<String>,
    pub when: StockingWhen,
    /// The species text as the list writes it ("Brown Trout"); not normalised.
    pub species_text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number_estimate: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_m: Option<LengthRange>,
    /// True for a planned stocking, false for one that took place.
    pub planned: bool,
    pub source: SourceId,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(when: StockingWhen, planned: bool) -> StockingEvent {
        StockingEvent {
            waterbody: "Croton River".into(),
            county: Some("Westchester".into()),
            town: None,
            when,
            species_text: "Brown Trout".into(),
            number_estimate: Some(1200),
            size_m: Some(LengthRange {
                min_m: 0.2,
                max_m: 0.3,
            }),
            planned,
            source: SourceId::NY_DEC_STOCKING,
        }
    }

    #[test]
    fn events_round_trip() {
        for e in [
            event(
                StockingWhen::Month {
                    year: 2026,
                    month: 4,
                },
                false,
            ),
            event(StockingWhen::DateText("April 14".into()), true),
        ] {
            let json = serde_json::to_string(&e).unwrap();
            assert_eq!(serde_json::from_str::<StockingEvent>(&json).unwrap(), e);
        }
    }

    #[test]
    fn optional_fields_may_be_absent() {
        let json = r#"{"waterbody":"Pond","when":{"date_text":"May"},"species_text":"Trout",
            "planned":true,"source":"ny-dec-stocking"}"#;
        let e: StockingEvent = serde_json::from_str(json).unwrap();
        assert!(e.county.is_none() && e.number_estimate.is_none() && e.size_m.is_none());
    }
}
