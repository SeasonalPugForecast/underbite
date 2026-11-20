//! Whether the water at a point is tidal, from the wetland codes of the U.S. Fish and Wildlife
//! Service's National Wetlands Inventory.
//!
//! 3DHP types tidal rivers and estuaries as plain rivers, so the inventory's Cowardin codes
//! decide: the Riverine system's Tidal subsystem (`R1...`, from the head of tide down to the
//! estuary) and every Estuarine code (`E...`) are tidal; any other code is not.
//!
//! The question is asked of the polygons in a small box around the point, without geometry, so
//! the rule cannot prefer the polygon nearest the snapped water: a tidal creek that joins the
//! river inside the box can make a non-tidal reach read as tidal (a false positive). The
//! inventory's own caveats apply: polygons come from imagery of mixed ages, and a reach can be
//! coded non-tidal just above the true limit.

use serde::Serialize;

/// What the wetland polygons around a point say about tide.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub enum Tide {
    /// Nothing is known (no answer, no polygons, an answer that could not be read): the caller
    /// falls back on what it knows locally.
    #[default]
    Unknown,
    /// Polygons are mapped around the point and none of them is tidal.
    NotTidal,
    /// A tidal polygon is mapped around the point.
    Tidal {
        /// Only estuarine polygons: no freshwater tidal river among them.
        estuarine: bool,
    },
}

impl Tide {
    /// The inventory decided: tidal or not tidal.
    pub const fn is_known(self) -> bool {
        !matches!(self, Self::Unknown)
    }

    /// The water is tidal.
    pub const fn is_tidal(self) -> bool {
        matches!(self, Self::Tidal { .. })
    }
}

/// The Cowardin codes of the polygons in a box, as the service returned them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Wetlands {
    /// The `ATTRIBUTE` of each polygon (`R1UBV`, `E1UBL`, `PEM1C`).
    pub codes: Vec<String>,
    /// The service held more polygons than it returned.
    pub exceeded_transfer_limit: bool,
}

impl Wetlands {
    /// The rule: tidal when any code starts with `R1` or `E`; not tidal when there are
    /// polygons and none matches; unknown when there are none. A truncated answer that has
    /// shown no tidal code says nothing (the missing polygons may hold one), so it is unknown.
    pub fn tide(&self) -> Tide {
        let codes = || self.codes.iter().map(|c| c.trim());
        let river = codes().any(|c| starts_with_ci(c, "R1"));
        let estuary = codes().any(|c| starts_with_ci(c, "E"));
        if river || estuary {
            return Tide::Tidal { estuarine: !river };
        }
        if self.codes.is_empty() || self.exceeded_transfer_limit {
            Tide::Unknown
        } else {
            Tide::NotTidal
        }
    }
}

fn starts_with_ci(code: &str, prefix: &str) -> bool {
    code.get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
}

/// States, districts and territories with tidal water: every state on a coast, Pennsylvania
/// (the tidal Delaware) and the District of Columbia (the Potomac). The Great Lakes have no
/// tide.
const TIDAL_STATES: [&str; 25] = [
    "AK", "AL", "CA", "CT", "DC", "DE", "FL", "GA", "HI", "LA", "MA", "MD", "ME", "MS", "NC", "NH",
    "NJ", "NY", "OR", "PA", "RI", "SC", "TX", "VA", "WA",
];

/// Whether any of the point's regions (`US-NY`) can hold tidal water. Regions outside the
/// United States, and an empty list, cannot: the inventory covers the United States.
pub fn may_be_tidal(regions: &[String]) -> bool {
    regions.iter().any(|r| {
        r.trim()
            .strip_prefix("US-")
            .is_some_and(|s| TIDAL_STATES.iter().any(|t| t.eq_ignore_ascii_case(s)))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tide(codes: &[&str], exceeded: bool) -> Tide {
        Wetlands {
            codes: codes.iter().map(|c| (*c).to_owned()).collect(),
            exceeded_transfer_limit: exceeded,
        }
        .tide()
    }

    #[test]
    fn a_tidal_river_code_or_an_estuarine_code_is_tidal() {
        assert_eq!(
            tide(&["R3UBH", "R1UBV"], false),
            Tide::Tidal { estuarine: false }
        );
        assert_eq!(tide(&["E1UBL"], false), Tide::Tidal { estuarine: true });
        // A river with an estuarine reach beside it is still a tidal river.
        assert_eq!(
            tide(&["E1UBL6", "R1UBV"], false),
            Tide::Tidal { estuarine: false }
        );
    }

    #[test]
    fn other_codes_are_not_tidal() {
        assert_eq!(tide(&["R3UBH", "R5UBH", "PUBHh"], false), Tide::NotTidal);
        // The second character decides, not the first alone.
        assert_eq!(tide(&["R2UBH", "R4SBC"], false), Tide::NotTidal);
    }

    #[test]
    fn nothing_mapped_is_unknown() {
        assert_eq!(tide(&[], false), Tide::Unknown);
    }

    #[test]
    fn a_truncated_answer_is_known_only_when_it_shows_a_tidal_code() {
        assert_eq!(tide(&["R3UBH"], true), Tide::Unknown);
        assert_eq!(
            tide(&["R3UBH", "R1UBV"], true),
            Tide::Tidal { estuarine: false }
        );
    }

    #[test]
    fn coastal_states_may_be_tidal_and_inland_ones_not() {
        let r = |s: &[&str]| s.iter().map(|x| (*x).to_owned()).collect::<Vec<_>>();
        assert!(may_be_tidal(&r(&["US-NY"])));
        assert!(may_be_tidal(&r(&["US-VT", "US-PA"])));
        assert!(!may_be_tidal(&r(&["US-VT"])));
        assert!(!may_be_tidal(&r(&["US-OH", "US-MI"])));
        assert!(!may_be_tidal(&r(&["CA-ON"])));
        assert!(!may_be_tidal(&[]));
    }
}
