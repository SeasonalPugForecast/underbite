//! Shared files across the FFI: reading, writing and fuzzing `.underbite` documents, and merging
//! live-shared trips.
//!
//! A trip crosses as the JSON of the core's `Trip`, as everywhere in the app; a file is the text
//! the core writes (pretty JSON with sorted keys, at most 1,000,000 bytes). Location fuzz is
//! deterministic and only ever coarsens a place: a file made at "about 1 km" from a trip that
//! was itself received at "area only" stays at "area only".

use underbite_core::model::WaterType;
use underbite_core::share::merge::{Change, merge};
use underbite_core::share::{Document, ShareError as Problem, SpotShare, decode, encode, fuzz};
use underbite_core::time::LocalDate;
use underbite_core::trip::{Fuzz, Trip, TripLimits, TripPlace};
use underbite_core::view::strings::{Key, Strings};

use crate::settings::load;
use crate::view::SpotInfo;

/// How much of each place's location a share reveals.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum ShareFuzz {
    /// The coordinates as saved.
    Exact,
    /// Rounded to 0.01 degrees, about a kilometre: the default.
    Rounded1Km,
    /// No coordinates: name, area and region only.
    AreaOnly,
}

impl From<ShareFuzz> for Fuzz {
    fn from(f: ShareFuzz) -> Self {
        match f {
            ShareFuzz::Exact => Self::Exact,
            ShareFuzz::Rounded1Km => Self::Rounded1Km,
            ShareFuzz::AreaOnly => Self::AreaOnly,
        }
    }
}

/// Why a file or a shared trip cannot be used; `message` is the core's text for the person.
#[derive(Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum ShareError {
    /// A newer version of the app made the file.
    #[error("{message}")]
    NeedsNewerVersion {
        /// "This file needs a newer version of the app."
        message: String,
    },
    /// The trip's last day is before its first.
    #[error("{message}")]
    DatesBackwards {
        /// The core's text.
        message: String,
    },
    /// The trip covers more than 31 days.
    #[error("{message}")]
    TooLong {
        /// The core's text.
        message: String,
    },
    /// Not a file this app reads, or one with a place or payload that cannot exist.
    #[error("{message}")]
    Unreadable {
        /// The core's text.
        message: String,
    },
    /// Larger than 1,000,000 bytes.
    #[error("{message}")]
    TooLarge {
        /// The core's text.
        message: String,
    },
    /// A kind this version knows of but cannot open yet (`layers`, `profile`).
    #[error("{message}")]
    Unsupported {
        /// The file's kind.
        kind: String,
        /// Says so.
        message: String,
    },
    /// What the app passed in could not be read: its own trip or spot, or the settings.
    #[error("invalid {what}: {message}")]
    Invalid {
        /// What was being read.
        what: String,
        /// The parser's message.
        message: String,
    },
}

impl From<Problem> for ShareError {
    fn from(p: Problem) -> Self {
        let message = sentence(&p.to_string());
        match p {
            Problem::NeedsNewerVersion => Self::NeedsNewerVersion { message },
            Problem::DatesBackwards => Self::DatesBackwards { message },
            Problem::TooLong => Self::TooLong { message },
            Problem::Unreadable => Self::Unreadable { message },
            Problem::TooLarge => Self::TooLarge { message },
        }
    }
}

fn invalid(what: &str, message: &impl ToString) -> ShareError {
    ShareError::Invalid {
        what: what.to_owned(),
        message: message.to_string(),
    }
}

/// `text` with a capital first letter and a full stop.
fn sentence(text: &str) -> String {
    let mut chars = text.chars();
    let mut out: String = chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default();
    if !out.ends_with('.') {
        out.push('.');
    }
    out
}

fn read_trip(json: &str) -> Result<Trip, ShareError> {
    serde_json::from_str(json).map_err(|e| invalid("trip", &e))
}

fn to_json(what: &str, value: &impl serde::Serialize) -> Result<String, ShareError> {
    serde_json::to_string(value).map_err(|e| invalid(what, &e))
}

// ---------------------------------------------------------------------------------------------
// Writing and fuzzing
// ---------------------------------------------------------------------------------------------

/// The `.underbite` file of a trip (its JSON), its places reduced to `fuzz`: the text to write
/// to the file. The trip is checked as a received one would be.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn trip_file(trip_json: String, fuzz: ShareFuzz) -> Result<String, ShareError> {
    let mut doc = Document::Trip(read_trip(&trip_json)?);
    self::fuzz(&mut doc, fuzz.into());
    Ok(encode(&doc)?)
}

/// The `.underbite` file of a saved spot with what its owner wrote about it, reduced to `fuzz`.
/// `area` is the group it is saved under ("Lower Hudson") and `region` its ISO 3166-2
/// subdivision (`US-NY`), both kept at every fuzz level.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn spot_file(
    spot: SpotInfo,
    area: Option<String>,
    region: Option<String>,
    notes: String,
    fuzz: ShareFuzz,
) -> Result<String, ShareError> {
    let water_type: WaterType = serde_json::from_value(serde_json::Value::String(
        spot.water_type.clone(),
    ))
    .map_err(|_| {
        invalid(
            "spot",
            &format!("`{}` is not a water type", spot.water_type),
        )
    })?;
    let place = TripPlace {
        spot_id: spot.id,
        name: spot.name,
        area: area.filter(|a| !a.trim().is_empty()),
        latitude: Some(spot.latitude),
        longitude: Some(spot.longitude),
        time_zone: spot.time_zone,
        water_type,
        region: region.filter(|r| !r.trim().is_empty()),
        fuzz: Fuzz::Exact,
    };
    let mut doc = Document::Spot(SpotShare { place, notes });
    self::fuzz(&mut doc, fuzz.into());
    Ok(encode(&doc)?)
}

/// A trip (its JSON) with its places reduced to `fuzz`, as JSON: the copy a live share hands to
/// others. A place never gets more precise than it is.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn fuzz_trip(trip_json: String, fuzz: ShareFuzz) -> Result<String, ShareError> {
    let mut doc = Document::Trip(read_trip(&trip_json)?);
    self::fuzz(&mut doc, fuzz.into());
    match doc {
        Document::Trip(trip) => to_json("trip", &trip),
        // INVARIANT: fuzzing keeps the document's kind.
        _ => Err(invalid("trip", &"not a trip")),
    }
}

// ---------------------------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------------------------

/// What a `.underbite` file holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum SharedKind {
    /// A trip: `trip_json`.
    Trip,
    /// A spot: `spot`, `place_json` and `notes`.
    Spot,
}

/// A file as read, for the preview before adding it.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct SharedFile {
    /// Trip or spot.
    pub kind: SharedKind,
    /// The trip, as the app keeps trips (a trip file).
    pub trip_json: Option<String>,
    /// The place as the core's `TripPlace` JSON (a spot file).
    pub place_json: Option<String>,
    /// The place as a spot to add (a spot file with coordinates; none when shared by area only).
    pub spot: Option<SpotInfo>,
    /// What the sender wrote about the spot (a spot file).
    pub notes: Option<String>,
    /// "Approximate location", in the settings' voice, when any place was shared rounded or
    /// by area only.
    pub approximate: Option<String>,
}

/// Reads a `.underbite` file. Every error carries the core's text to show; a kind this version
/// cannot open yet is [`ShareError::Unsupported`].
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn read_file(bytes: Vec<u8>, settings_json: String) -> Result<SharedFile, ShareError> {
    let (config, _) = load(&settings_json).map_err(|e| invalid("settings", &e))?;
    let strings = Strings::new(config.presentation.voice);
    let approximate =
        |fuzzed: bool| fuzzed.then(|| strings.get(Key::TripApproximateLocation).to_owned());
    match decode(&bytes)? {
        Document::Trip(trip) => Ok(SharedFile {
            kind: SharedKind::Trip,
            approximate: approximate(trip.places.iter().any(|p| p.fuzz != Fuzz::Exact)),
            trip_json: Some(to_json("trip", &trip)?),
            place_json: None,
            spot: None,
            notes: None,
        }),
        Document::Spot(share) => Ok(SharedFile {
            kind: SharedKind::Spot,
            approximate: approximate(share.place.fuzz != Fuzz::Exact),
            trip_json: None,
            place_json: Some(to_json("place", &share.place)?),
            spot: spot_info(&share.place),
            notes: Some(share.notes),
        }),
        Document::Unsupported { kind } => Err(ShareError::Unsupported {
            message: sentence(&format!(
                "this version of the app cannot open a {} file yet",
                kind_words(&kind)
            )),
            kind,
        }),
    }
}

/// A reserved kind in words.
fn kind_words(kind: &str) -> &str {
    match kind {
        "layers" => "map layers",
        "profile" => "species profile",
        other => other,
    }
}

fn spot_info(place: &TripPlace) -> Option<SpotInfo> {
    Some(SpotInfo {
        id: place.spot_id.clone(),
        name: place.name.clone(),
        latitude: place.latitude?,
        longitude: place.longitude?,
        water_type: match serde_json::to_value(place.water_type) {
            Ok(serde_json::Value::String(s)) => s,
            _ => String::new(),
        },
        time_zone: place.time_zone.clone(),
    })
}

// ---------------------------------------------------------------------------------------------
// Merging
// ---------------------------------------------------------------------------------------------

/// One difference between this device's trip and the merged one, for the "what changed"
/// banner. Dates are `YYYY-MM-DD`; method, species and limits are as the trip writes them.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum TripChange {
    /// The trip was renamed.
    Renamed {
        /// The old name.
        from: String,
        /// The new name.
        to: String,
    },
    /// The dates moved.
    Dates {
        /// The old first day.
        from_first: String,
        /// The old last day.
        from_last: String,
        /// The new first day.
        to_first: String,
        /// The new last day.
        to_last: String,
    },
    /// The method changed (`"kayak"`).
    Method {
        /// The old method.
        from: String,
        /// The new method.
        to: String,
    },
    /// Species were added or removed (ids such as `morone-saxatilis`).
    Species {
        /// Added.
        added: Vec<String>,
        /// Removed.
        removed: Vec<String>,
    },
    /// The limits changed: the names of the profiles, none where there were no limits.
    Limits {
        /// The old profile's name.
        from: Option<String>,
        /// The new profile's name.
        to: Option<String>,
    },
    /// The notes changed.
    Notes {
        /// The old notes.
        from: String,
        /// The new notes.
        to: String,
    },
    /// A place was added.
    PlaceAdded {
        /// Its spot id.
        spot_id: String,
        /// Its name.
        name: String,
    },
    /// A place was removed.
    PlaceRemoved {
        /// Its spot id.
        spot_id: String,
        /// Its name.
        name: String,
    },
    /// A place's name, area, location, zone, water type or region changed.
    PlaceChanged {
        /// Its spot id.
        spot_id: String,
        /// Its new name.
        name: String,
    },
    /// The places are in another order.
    PlacesReordered,
    /// A checklist item was added.
    ChecklistAdded {
        /// Its id.
        id: String,
        /// Its text.
        text: String,
    },
    /// A checklist item was removed.
    ChecklistRemoved {
        /// Its id.
        id: String,
        /// Its text.
        text: String,
    },
    /// A checklist item was reworded.
    ChecklistEdited {
        /// Its id.
        id: String,
        /// The old text.
        from: String,
        /// The new text.
        to: String,
    },
    /// A checklist item was ticked or unticked.
    ChecklistTicked {
        /// Its id.
        id: String,
        /// Its text.
        text: String,
        /// Ticked.
        done: bool,
    },
}

fn iso(d: LocalDate) -> String {
    format!("{:04}-{:02}-{:02}", d.year, d.month, d.day)
}

fn name_of(value: &impl serde::Serialize) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(s)) => s,
        _ => String::new(),
    }
}

fn limits_name(limits: Option<&TripLimits>) -> Option<String> {
    limits.map(|l| l.name.clone())
}

impl From<Change> for TripChange {
    fn from(c: Change) -> Self {
        match c {
            Change::Renamed { from, to } => Self::Renamed { from, to },
            Change::Dates { from, to } => Self::Dates {
                from_first: iso(from.0),
                from_last: iso(from.1),
                to_first: iso(to.0),
                to_last: iso(to.1),
            },
            Change::Method { from, to } => Self::Method {
                from: name_of(&from),
                to: name_of(&to),
            },
            Change::Species { added, removed } => Self::Species {
                added: added.iter().map(|s| s.as_str().to_owned()).collect(),
                removed: removed.iter().map(|s| s.as_str().to_owned()).collect(),
            },
            Change::Limits { from, to } => Self::Limits {
                from: limits_name(from.as_ref()),
                to: limits_name(to.as_ref()),
            },
            Change::Notes { from, to } => Self::Notes { from, to },
            Change::PlaceAdded { spot_id, name } => Self::PlaceAdded { spot_id, name },
            Change::PlaceRemoved { spot_id, name } => Self::PlaceRemoved { spot_id, name },
            Change::PlaceChanged { spot_id, name } => Self::PlaceChanged { spot_id, name },
            Change::PlacesReordered => Self::PlacesReordered,
            Change::ChecklistAdded { id, text } => Self::ChecklistAdded { id, text },
            Change::ChecklistRemoved { id, text } => Self::ChecklistRemoved { id, text },
            Change::ChecklistEdited { id, from, to } => Self::ChecklistEdited { id, from, to },
            Change::ChecklistTicked { id, text, done } => Self::ChecklistTicked { id, text, done },
        }
    }
}

/// A merged trip and what the merge changed under this device.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct MergedTrip {
    /// The merged trip, to save and to send.
    pub trip_json: String,
    /// What changed relative to `mine`, for the banner; empty when nothing did.
    pub changes: Vec<TripChange>,
}

/// Merges a live-shared trip edited on more than one device: `base_json` is the trip as this
/// device last saw it on the server, `mine_json` as this device has it now, `theirs_json` what
/// the server holds. Field by field against the base; where both sides changed a field the
/// later `modified_at` wins (this device's on a tie). A place never becomes more precise than
/// this device holds it, so an owner's exact coordinates never reach a participant's copy and a
/// participant's rounded ones never replace the owner's.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn merge_trips(
    base_json: String,
    mine_json: String,
    theirs_json: String,
) -> Result<MergedTrip, ShareError> {
    let base = read_trip(&base_json)?;
    let mine = read_trip(&mine_json)?;
    let theirs = read_trip(&theirs_json)?;
    let merged = merge(&base, &mine, &theirs);
    Ok(MergedTrip {
        trip_json: to_json("trip", &merged.trip)?,
        changes: merged.changes.into_iter().map(TripChange::from).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_problems_keep_their_text_as_a_sentence() {
        let e = ShareError::from(Problem::NeedsNewerVersion);
        assert_eq!(
            e,
            ShareError::NeedsNewerVersion {
                message: "This file needs a newer version of the app.".into()
            }
        );
        assert_eq!(kind_words("layers"), "map layers");
    }
}
