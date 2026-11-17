//! Merging live-shared trips edited on more than one device.
//!
//! [`merge`] is a three-way merge: `base` is the trip as this device last saw it on the server,
//! `mine` the trip as this device has it now, `theirs` what the server holds. It works field by
//! field against the base:
//!
//! - a field only one side changed takes that side's value;
//! - a field both sides changed to different values takes the value of the side with the later
//!   `modified_at` (this device's, on a tie, since it saves last);
//! - the first and last day move together, as do a place's coordinates and fuzz level;
//! - species merge as a set: an addition on either side stays, a removal on either side removes;
//! - places match by `spot_id` (by position when it is empty), checklist items by `id`; an item
//!   removed on one side and unchanged on the other is removed, one that was edited on the other
//!   side is kept; an item added on one side is kept;
//! - a merge never makes a place more precise than this device holds it: a copy of the place from
//!   `theirs` is rounded down to this device's fuzz level, and a difference that is only the fuzz
//!   is not a change, so a participant's rounded copy never overwrites the owner's exact
//!   coordinates and the owner's exact ones never reach a participant.
//!
//! The result lists what changed under this device as [`Change`] values, for a "what changed"
//! banner the view words.

use std::collections::BTreeMap;

use crate::time::{LocalDate, UnixTime};
use crate::trip::model::{ChecklistItem, Fuzz, Method, SpeciesId, Trip, TripLimits, TripPlace};

/// The merged trip and what the merge changed relative to `mine`.
#[derive(Clone, Debug, PartialEq)]
pub struct Merged {
    pub trip: Trip,
    pub changes: Vec<Change>,
}

/// One difference between this device's trip and the merged one.
#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    Renamed {
        from: String,
        to: String,
    },
    Dates {
        from: (LocalDate, LocalDate),
        to: (LocalDate, LocalDate),
    },
    Method {
        from: Method,
        to: Method,
    },
    Species {
        added: Vec<SpeciesId>,
        removed: Vec<SpeciesId>,
    },
    Limits {
        from: Option<TripLimits>,
        to: Option<TripLimits>,
    },
    Notes {
        from: String,
        to: String,
    },
    PlaceAdded {
        spot_id: String,
        name: String,
    },
    PlaceRemoved {
        spot_id: String,
        name: String,
    },
    /// Name, area, location, zone, water type or region changed; `name` is the new name.
    PlaceChanged {
        spot_id: String,
        name: String,
    },
    PlacesReordered,
    ChecklistAdded {
        id: String,
        text: String,
    },
    ChecklistRemoved {
        id: String,
        text: String,
    },
    ChecklistEdited {
        id: String,
        from: String,
        to: String,
    },
    /// An item was ticked (`done`) or unticked.
    ChecklistTicked {
        id: String,
        text: String,
        done: bool,
    },
}

/// Merges `mine` and `theirs` against `base`; see the module documentation for the rules.
///
/// The id comes from `theirs`, `modified_at` is the later of the two.
#[must_use]
pub fn merge(base: &Trip, mine: &Trip, theirs: &Trip) -> Merged {
    let mine_wins = mine.modified_at >= theirs.modified_at;
    let floor = fuzz_floor(mine, base);

    let (first, last) = pick(
        &(base.first, base.last),
        &(mine.first, mine.last),
        &(theirs.first, theirs.last),
        mine_wins,
    );
    let trip = Trip {
        id: theirs.id.clone(),
        name: pick(&base.name, &mine.name, &theirs.name, mine_wins),
        places: merge_list(
            &base.places,
            &mine.places,
            &theirs.places,
            &Rules {
                key: place_key,
                same: same_place,
                both: &|b, m, t| merge_place(b, m, t, mine_wins),
                theirs_only: &|t, b| coarsened(t, floor.max(b.map_or(Fuzz::Exact, |b| b.fuzz))),
            },
        ),
        first,
        last,
        species: merge_species(&base.species, &mine.species, &theirs.species),
        method: pick(&base.method, &mine.method, &theirs.method, mine_wins),
        limits: pick(&base.limits, &mine.limits, &theirs.limits, mine_wins),
        notes: pick(&base.notes, &mine.notes, &theirs.notes, mine_wins),
        checklist: merge_list(
            &base.checklist,
            &mine.checklist,
            &theirs.checklist,
            &Rules {
                key: item_key,
                same: |a, b| a == b,
                both: &|b, m, t| merge_item(b, m, t, mine_wins),
                theirs_only: &|t, _| t.clone(),
            },
        ),
        modified_at: UnixTime(mine.modified_at.0.max(theirs.modified_at.0)),
    };
    let changes = diff(mine, &trip);
    Merged { trip, changes }
}

/// The three-way choice for one value.
fn pick<T: PartialEq + Clone>(base: &T, mine: &T, theirs: &T, mine_wins: bool) -> T {
    if mine == base {
        theirs.clone()
    } else if theirs == base || mine == theirs || mine_wins {
        mine.clone()
    } else {
        theirs.clone()
    }
}

// ---- places ----

fn place_key(index: usize, p: &TripPlace) -> String {
    if p.spot_id.is_empty() {
        format!("#{index}")
    } else {
        p.spot_id.clone()
    }
}

fn item_key(index: usize, c: &ChecklistItem) -> String {
    if c.id.is_empty() {
        format!("#{index}")
    } else {
        c.id.clone()
    }
}

/// The coarsest level this device holds places at, so places that arrive from elsewhere are
/// brought to it. Falls back to the base's places when this device has none.
fn fuzz_floor(mine: &Trip, base: &Trip) -> Fuzz {
    let max = |t: &Trip| t.places.iter().map(|p| p.fuzz).max();
    max(mine).or_else(|| max(base)).unwrap_or(Fuzz::Exact)
}

/// `p` at `level` or coarser; never more precise. The same rounding as the file's fuzz.
fn coarsened(p: &TripPlace, level: Fuzz) -> TripPlace {
    let mut p = p.clone();
    if level <= p.fuzz {
        return p;
    }
    match level {
        Fuzz::Exact => {}
        Fuzz::Rounded1Km => {
            p.latitude = p.latitude.map(hundredth);
            p.longitude = p.longitude.map(hundredth);
        }
        Fuzz::AreaOnly => {
            p.latitude = None;
            p.longitude = None;
        }
    }
    p.fuzz = level;
    p
}

fn hundredth(x: f64) -> f64 {
    (x * 100.0).round() / 100.0 + 0.0
}

/// Whether two places are at the same location once both are brought to the coarser level: a
/// rounded copy of an exact place is the same location.
fn same_location(a: &TripPlace, b: &TripPlace) -> bool {
    let level = a.fuzz.max(b.fuzz);
    let (a, b) = (coarsened(a, level), coarsened(b, level));
    (a.fuzz, a.latitude, a.longitude) == (b.fuzz, b.latitude, b.longitude)
}

fn same_place(a: &TripPlace, b: &TripPlace) -> bool {
    same_location(a, b)
        && a.name == b.name
        && a.area == b.area
        && a.time_zone == b.time_zone
        && a.water_type == b.water_type
        && a.region == b.region
}

fn merge_place(
    base: Option<&TripPlace>,
    mine: &TripPlace,
    theirs: &TripPlace,
    mine_wins: bool,
) -> TripPlace {
    let Some(base) = base else {
        // Both sides added the place.
        return if same_place(mine, theirs) || mine_wins {
            mine.clone()
        } else {
            coarsened(theirs, mine.fuzz)
        };
    };
    let mine_moved = !same_location(mine, base);
    let theirs_moved = !same_location(theirs, base);
    let take_theirs = theirs_moved && (!mine_moved || !mine_wins) && !same_location(mine, theirs);
    let mut p = if take_theirs {
        coarsened(theirs, mine.fuzz)
    } else {
        mine.clone()
    };
    p.name = pick(&base.name, &mine.name, &theirs.name, mine_wins);
    p.area = pick(&base.area, &mine.area, &theirs.area, mine_wins);
    p.time_zone = pick(
        &base.time_zone,
        &mine.time_zone,
        &theirs.time_zone,
        mine_wins,
    );
    p.water_type = pick(
        &base.water_type,
        &mine.water_type,
        &theirs.water_type,
        mine_wins,
    );
    p.region = pick(&base.region, &mine.region, &theirs.region, mine_wins);
    p.spot_id.clone_from(&mine.spot_id);
    p
}

fn merge_item(
    base: Option<&ChecklistItem>,
    mine: &ChecklistItem,
    theirs: &ChecklistItem,
    mine_wins: bool,
) -> ChecklistItem {
    match base {
        None if mine_wins => mine.clone(),
        None => theirs.clone(),
        Some(b) => ChecklistItem {
            id: mine.id.clone(),
            text: pick(&b.text, &mine.text, &theirs.text, mine_wins),
            done: pick(&b.done, &mine.done, &theirs.done, mine_wins),
        },
    }
}

// ---- lists ----

struct Rules<'a, T> {
    key: fn(usize, &T) -> String,
    same: fn(&T, &T) -> bool,
    /// Both sides hold the item; the base may not (both added it).
    both: &'a dyn Fn(Option<&T>, &T, &T) -> T,
    /// Only the other side's copy survives; the base's copy, when there was one.
    theirs_only: &'a dyn Fn(&T, Option<&T>) -> T,
}

type Keyed<'a, T> = (Vec<String>, BTreeMap<String, &'a T>);

fn keyed<T>(items: &[T], key: fn(usize, &T) -> String) -> Keyed<'_, T> {
    let keys: Vec<String> = items.iter().enumerate().map(|(i, x)| key(i, x)).collect();
    let mut map = BTreeMap::new();
    for (k, x) in keys.iter().zip(items) {
        map.entry(k.clone()).or_insert(x);
    }
    (keys, map)
}

fn merge_list<T: Clone>(base: &[T], mine: &[T], theirs: &[T], r: &Rules<'_, T>) -> Vec<T> {
    let (bk, bm) = keyed(base, r.key);
    let (mk, mm) = keyed(mine, r.key);
    let (tk, tm) = keyed(theirs, r.key);
    // This device's order when it changed the list, else the other side's.
    let (primary, secondary) = if mk == bk { (&tk, &mk) } else { (&mk, &tk) };
    let mut order: Vec<&String> = Vec::new();
    for k in primary.iter().chain(secondary) {
        if !order.contains(&k) {
            order.push(k);
        }
    }
    let mut out = Vec::new();
    for k in order {
        let b = bm.get(k).copied();
        let item = match (b, mm.get(k).copied(), tm.get(k).copied()) {
            (_, Some(m), Some(t)) => Some((r.both)(b, m, t)),
            (None, Some(m), None) => Some(m.clone()),
            (None, None, Some(t)) => Some((r.theirs_only)(t, None)),
            (Some(b), Some(m), None) => (!(r.same)(m, b)).then(|| m.clone()),
            (Some(b), None, Some(t)) => (!(r.same)(t, b)).then(|| (r.theirs_only)(t, Some(b))),
            (Some(_) | None, None, None) => None,
        };
        out.extend(item);
    }
    out
}

fn merge_species(base: &[SpeciesId], mine: &[SpeciesId], theirs: &[SpeciesId]) -> Vec<SpeciesId> {
    let mut out: Vec<SpeciesId> = Vec::new();
    for s in mine.iter().chain(theirs) {
        let (b, m, t) = (base.contains(s), mine.contains(s), theirs.contains(s));
        // Kept unless one side removed it (it was in the base and is missing from a side).
        if (!b || (m && t)) && !out.contains(s) {
            out.push(s.clone());
        }
    }
    out
}

// ---- what changed ----

fn diff(mine: &Trip, merged: &Trip) -> Vec<Change> {
    let mut c = Vec::new();
    if mine.name != merged.name {
        c.push(Change::Renamed {
            from: mine.name.clone(),
            to: merged.name.clone(),
        });
    }
    if (mine.first, mine.last) != (merged.first, merged.last) {
        c.push(Change::Dates {
            from: (mine.first, mine.last),
            to: (merged.first, merged.last),
        });
    }
    if mine.method != merged.method {
        c.push(Change::Method {
            from: mine.method,
            to: merged.method,
        });
    }
    let added: Vec<_> = merged
        .species
        .iter()
        .filter(|s| !mine.species.contains(s))
        .cloned()
        .collect();
    let removed: Vec<_> = mine
        .species
        .iter()
        .filter(|s| !merged.species.contains(s))
        .cloned()
        .collect();
    if !added.is_empty() || !removed.is_empty() {
        c.push(Change::Species { added, removed });
    }
    if mine.limits != merged.limits {
        c.push(Change::Limits {
            from: mine.limits.clone(),
            to: merged.limits.clone(),
        });
    }
    if mine.notes != merged.notes {
        c.push(Change::Notes {
            from: mine.notes.clone(),
            to: merged.notes.clone(),
        });
    }

    let (mk, mm) = keyed(&mine.places, place_key);
    let (nk, nm) = keyed(&merged.places, place_key);
    for (k, p) in &nm {
        match mm.get(k) {
            None => c.push(Change::PlaceAdded {
                spot_id: p.spot_id.clone(),
                name: p.name.clone(),
            }),
            Some(old) if **old != **p => c.push(Change::PlaceChanged {
                spot_id: p.spot_id.clone(),
                name: p.name.clone(),
            }),
            Some(_) => {}
        }
    }
    for (k, p) in &mm {
        if !nm.contains_key(k) {
            c.push(Change::PlaceRemoved {
                spot_id: p.spot_id.clone(),
                name: p.name.clone(),
            });
        }
    }
    let kept = |ks: &[String], other: &BTreeMap<String, &TripPlace>| -> Vec<String> {
        ks.iter()
            .filter(|k| other.contains_key(*k))
            .cloned()
            .collect()
    };
    if kept(&mk, &nm) != kept(&nk, &mm) {
        c.push(Change::PlacesReordered);
    }

    diff_checklist(&mine.checklist, &merged.checklist, &mut c);
    c
}

fn diff_checklist(mine: &[ChecklistItem], merged: &[ChecklistItem], c: &mut Vec<Change>) {
    let (_, om) = keyed(mine, item_key);
    let (_, nm) = keyed(merged, item_key);
    for (k, n) in &nm {
        match om.get(k) {
            None => c.push(Change::ChecklistAdded {
                id: n.id.clone(),
                text: n.text.clone(),
            }),
            Some(o) => {
                if o.text != n.text {
                    c.push(Change::ChecklistEdited {
                        id: n.id.clone(),
                        from: o.text.clone(),
                        to: n.text.clone(),
                    });
                }
                if o.done != n.done {
                    c.push(Change::ChecklistTicked {
                        id: n.id.clone(),
                        text: n.text.clone(),
                        done: n.done,
                    });
                }
            }
        }
    }
    for (k, o) in &om {
        if !nm.contains_key(k) {
            c.push(Change::ChecklistRemoved {
                id: o.id.clone(),
                text: o.text.clone(),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trip::model::tests::{battery, trip};

    fn item(id: &str, text: &str, done: bool) -> ChecklistItem {
        ChecklistItem {
            id: id.into(),
            text: text.into(),
            done,
        }
    }

    fn croton() -> TripPlace {
        TripPlace {
            spot_id: "spot-croton".into(),
            name: "Croton River".into(),
            latitude: Some(41.2050),
            longitude: Some(-73.8830),
            ..battery()
        }
    }

    fn at(t: &mut Trip, secs: i64) {
        t.modified_at = UnixTime(secs);
    }

    #[test]
    fn nothing_changed_is_theirs_and_mine() {
        let b = trip();
        let m = merge(&b, &b, &b);
        assert_eq!(m.trip, b);
        assert_eq!(m.changes, vec![]);
    }

    #[test]
    fn different_fields_both_survive() {
        let b = trip();
        let mut mine = b.clone();
        mine.name = "Hudson long weekend".into();
        at(&mut mine, 10);
        let mut theirs = b.clone();
        theirs.notes = "Bring the thermos.".into();
        theirs.method = Method::Shore;
        at(&mut theirs, 20);
        let m = merge(&b, &mine, &theirs);
        assert_eq!(m.trip.name, "Hudson long weekend");
        assert_eq!(m.trip.notes, "Bring the thermos.");
        assert_eq!(m.trip.method, Method::Shore);
        assert_eq!(m.trip.modified_at, UnixTime(20));
        assert_eq!(
            m.changes,
            vec![
                Change::Method {
                    from: Method::Kayak,
                    to: Method::Shore
                },
                Change::Notes {
                    from: b.notes.clone(),
                    to: "Bring the thermos.".into()
                },
            ]
        );
    }

    #[test]
    fn same_field_later_modified_at_wins() {
        let b = trip();
        let mut mine = b.clone();
        mine.name = "Mine".into();
        at(&mut mine, 30);
        let mut theirs = b.clone();
        theirs.name = "Theirs".into();
        at(&mut theirs, 20);
        assert_eq!(merge(&b, &mine, &theirs).trip.name, "Mine");
        at(&mut theirs, 40);
        let m = merge(&b, &mine, &theirs);
        assert_eq!(m.trip.name, "Theirs");
        assert_eq!(
            m.changes,
            vec![Change::Renamed {
                from: "Mine".into(),
                to: "Theirs".into()
            }]
        );
        at(&mut theirs, 30);
        assert_eq!(
            merge(&b, &mine, &theirs).trip.name,
            "Mine",
            "a tie keeps this device's"
        );
    }

    #[test]
    fn dates_move_together() {
        let b = trip();
        let mut mine = b.clone();
        mine.first = LocalDate {
            year: 2027,
            month: 7,
            day: 13,
        };
        at(&mut mine, 10);
        let mut theirs = b.clone();
        theirs.last = LocalDate {
            year: 2027,
            month: 7,
            day: 20,
        };
        at(&mut theirs, 20);
        let m = merge(&b, &mine, &theirs);
        assert_eq!((m.trip.first, m.trip.last), (theirs.first, theirs.last));
        assert_eq!(m.trip.validate(), Ok(()));
    }

    #[test]
    fn species_merge_as_a_set() {
        let b = trip();
        let sp = |s: &str| SpeciesId::new(s).unwrap();
        let mut mine = b.clone();
        mine.species.push(sp("esox-lucius"));
        let mut theirs = b.clone();
        theirs.species = vec![sp("salvelinus-fontinalis")];
        let m = merge(&b, &mine, &theirs);
        assert_eq!(
            m.trip.species,
            vec![sp("esox-lucius"), sp("salvelinus-fontinalis")]
        );
    }

    #[test]
    fn place_removed_on_one_side_unchanged_on_the_other_is_removed() {
        let mut b = trip();
        b.places.push(croton());
        let mut mine = b.clone();
        mine.places.remove(1);
        let m = merge(&b, &mine, &b);
        assert_eq!(m.trip.places.len(), 1);
        assert_eq!(m.changes, vec![]);
        // And the other way round.
        let m = merge(&b, &b, &mine);
        assert_eq!(m.trip.places.len(), 1);
        assert_eq!(
            m.changes,
            vec![Change::PlaceRemoved {
                spot_id: "spot-croton".into(),
                name: "Croton River".into()
            }]
        );
    }

    #[test]
    fn place_removed_on_one_side_edited_on_the_other_is_kept() {
        let mut b = trip();
        b.places.push(croton());
        let mut mine = b.clone();
        mine.places.remove(1);
        let mut theirs = b.clone();
        theirs.places[1].name = "Croton River, lower".into();
        let m = merge(&b, &mine, &theirs);
        assert_eq!(m.trip.places.len(), 2);
        assert_eq!(m.trip.places[1].name, "Croton River, lower");
        assert_eq!(
            m.changes,
            vec![Change::PlaceAdded {
                spot_id: "spot-croton".into(),
                name: "Croton River, lower".into()
            }]
        );
        // Removed by them, edited here.
        let m = merge(&b, &theirs, &mine);
        assert_eq!(m.trip.places[1].name, "Croton River, lower");
        assert_eq!(m.changes, vec![]);
    }

    #[test]
    fn places_added_on_both_sides_and_matched_by_position_without_a_spot_id() {
        let b = trip();
        let mut mine = b.clone();
        mine.places.push(croton());
        let mut theirs = b.clone();
        theirs.places.push(TripPlace {
            spot_id: "spot-other".into(),
            name: "Other".into(),
            ..battery()
        });
        let m = merge(&b, &mine, &theirs);
        let ids: Vec<_> = m.trip.places.iter().map(|p| p.spot_id.as_str()).collect();
        assert_eq!(ids, ["spot-battery", "spot-croton", "spot-other"]);

        let mut b = trip();
        b.places[0].spot_id.clear();
        let mut mine = b.clone();
        mine.places[0].name = "Battery Park".into();
        let mut theirs = b.clone();
        theirs.places[0].area = Some("Harbor".into());
        let m = merge(&b, &mine, &theirs);
        assert_eq!(m.trip.places.len(), 1);
        assert_eq!(m.trip.places[0].name, "Battery Park");
        assert_eq!(m.trip.places[0].area.as_deref(), Some("Harbor"));
    }

    #[test]
    fn place_fields_merge_individually() {
        let b = trip();
        let mut mine = b.clone();
        mine.places[0].name = "Battery Park".into();
        let mut theirs = b.clone();
        theirs.places[0].latitude = Some(40.71);
        theirs.places[0].longitude = Some(-74.02);
        let m = merge(&b, &mine, &theirs);
        let p = &m.trip.places[0];
        assert_eq!(
            (p.name.as_str(), p.latitude, p.longitude),
            ("Battery Park", Some(40.71), Some(-74.02))
        );
    }

    #[test]
    fn checklist_ticks_on_both_sides_and_edits() {
        let mut b = trip();
        b.checklist = vec![item("c1", "Licence", false), item("c2", "Waders", false)];
        let mut mine = b.clone();
        mine.checklist[0].done = true;
        mine.checklist.push(item("c3", "Net", false));
        let mut theirs = b.clone();
        theirs.checklist[0].done = true;
        theirs.checklist[1].text = "Wading boots".into();
        theirs.checklist.push(item("c4", "Pliers", false));
        let m = merge(&b, &mine, &theirs);
        assert_eq!(
            m.trip.checklist,
            vec![
                item("c1", "Licence", true),
                item("c2", "Wading boots", false),
                item("c3", "Net", false),
                item("c4", "Pliers", false),
            ]
        );
        assert_eq!(
            m.changes,
            vec![
                Change::ChecklistEdited {
                    id: "c2".into(),
                    from: "Waders".into(),
                    to: "Wading boots".into()
                },
                Change::ChecklistAdded {
                    id: "c4".into(),
                    text: "Pliers".into()
                },
            ]
        );
    }

    #[test]
    fn checklist_tick_against_untick_and_delete_rules() {
        let mut b = trip();
        b.checklist = vec![item("c1", "Licence", true), item("c2", "Waders", false)];
        let mut mine = b.clone();
        mine.checklist[0].done = false;
        mine.checklist.remove(1);
        at(&mut mine, 5);
        let mut theirs = b.clone();
        theirs.checklist[0].text = "Licence (printed)".into();
        at(&mut theirs, 9);
        let m = merge(&b, &mine, &theirs);
        // Text: only theirs changed. Done: only mine changed. Waders removed by mine, untouched.
        assert_eq!(
            m.trip.checklist,
            vec![item("c1", "Licence (printed)", false)]
        );

        let mut theirs = b.clone();
        theirs.checklist[0].done = false;
        theirs.checklist[1].done = true;
        at(&mut theirs, 9);
        // Unticked on both sides stays unticked.
        let m = merge(&b, &mine, &theirs);
        assert!(!m.trip.checklist[0].done);
        // An item ticked there is kept against a delete here.
        assert_eq!(m.trip.checklist.len(), 2);
    }

    #[test]
    fn a_participant_keeps_rounded_and_the_owner_keeps_exact() {
        let b_owner = trip();
        let mut rounded = b_owner.clone();
        rounded.places[0] = coarsened(&rounded.places[0], Fuzz::Rounded1Km);
        // The participant edits the notes; the owner merges the participant's copy.
        let mut theirs = rounded.clone();
        theirs.notes = "Ice instead.".into();
        let m = merge(&b_owner, &b_owner, &theirs);
        assert_eq!(m.trip.places[0], b_owner.places[0], "exact stays exact");
        assert_eq!(m.trip.notes, "Ice instead.");
        assert!(
            m.changes
                .iter()
                .all(|c| !matches!(c, Change::PlaceChanged { .. }))
        );

        // The owner edits the notes and renames the place; the participant merges the owner's copy.
        let mut owner = b_owner.clone();
        owner.notes = "Moved.".into();
        owner.places[0].name = "Battery Park".into();
        owner.places.push(croton());
        let m = merge(&rounded, &rounded, &owner);
        assert!(
            m.trip.places.iter().all(|p| p.fuzz == Fuzz::Rounded1Km),
            "{:?}",
            m.trip.places
        );
        assert_eq!(m.trip.places[0].latitude, Some(40.70));
        assert_eq!(m.trip.places[0].name, "Battery Park");
        assert_eq!(m.trip.places[1].latitude, Some(41.21));
    }

    #[test]
    fn a_moved_place_is_never_finer_than_this_device_holds_it() {
        let exact = trip();
        let mut rounded = exact.clone();
        rounded.places[0] = coarsened(&rounded.places[0], Fuzz::Rounded1Km);
        let mut owner = exact.clone();
        owner.places[0].latitude = Some(40.6892);
        owner.places[0].longitude = Some(-74.0445);
        let m = merge(&rounded, &rounded, &owner);
        let p = &m.trip.places[0];
        assert_eq!(
            (p.latitude, p.longitude, p.fuzz),
            (Some(40.69), Some(-74.04), Fuzz::Rounded1Km)
        );
        // A place the owner edited that the participant had deleted comes back rounded too.
        let mut gone = rounded.clone();
        gone.places.push(coarsened(&croton(), Fuzz::Rounded1Km));
        let base = gone.clone();
        gone.places.remove(1);
        let mut edited = exact.clone();
        edited.places.push(croton());
        edited.places[1].name = "Croton".into();
        let m = merge(&base, &gone, &edited);
        assert_eq!(m.trip.places[1].fuzz, Fuzz::Rounded1Km);
        assert_eq!(m.trip.places[1].latitude, Some(41.21));
    }

    #[test]
    fn area_only_stays_area_only() {
        let exact = trip();
        let mut mine = exact.clone();
        mine.places[0] = coarsened(&exact.places[0], Fuzz::AreaOnly);
        let mut theirs = exact.clone();
        theirs.places[0].name = "Battery Park".into();
        let m = merge(&mine, &mine, &theirs);
        let p = &m.trip.places[0];
        assert_eq!(
            (p.latitude, p.longitude, p.fuzz),
            (None, None, Fuzz::AreaOnly)
        );
        assert_eq!(p.name, "Battery Park");
        assert_eq!(m.trip.validate(), Ok(()));
    }
}
