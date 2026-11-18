//! The catch log across the FFI: entries, conditions at the time of a catch, export and what worked.
//!
//! The log is the user's private data. The FFI does no I/O: the app keeps the log's bytes (the
//! core's JSON text, UTF-8) wherever it likes and passes them in; every call that changes the log
//! returns the new bytes for the app to store. Ids and `now` come from the app. Exports are made
//! only when asked for. Every word, size and time is written by the core's view; the editor's text
//! fields are parsed here in the user's fish units. Errors carry the core's words.

use std::sync::OnceLock;

use serde::Serialize;
use underbite_core::log::export::{export_csv, export_gpx};
use underbite_core::log::insights::{Filter, catch_density_for, summarise};
use underbite_core::log::{
    CatchConditions, CatchEntry, CatchLocation, Fate, LogBook, LogError as CoreLogError, PhotoRef,
};
use underbite_core::model::SpotSnapshot;
use underbite_core::species::catalogue::Catalogue;
use underbite_core::time::{UnixTime, zone};
use underbite_core::trip::SpeciesId;
use underbite_core::view::Voice;
use underbite_core::view::log::{
    FishUnits, LengthUnit, ParseError, SpotName, WeightUnit, catch_detail, catch_list, edit_length,
    edit_pounds_ounces, edit_weight, fish_units, insights_view, parse_length, parse_pounds_ounces,
    parse_weight,
};
use underbite_core::view::strings::{Key, SafetyKey, Strings};

use crate::map::formatter;
use crate::settings::{SettingsError, load};

// ---------------------------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------------------------

/// Why a log call failed.
#[derive(Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum LogError {
    /// The bytes are not a catch log, or are from a newer version of the app.
    #[error("{message}")]
    Book {
        /// The core's words.
        message: String,
    },
    /// One field of the editor is not acceptable.
    #[error("{message}")]
    Field {
        /// Which field: `length`, `weight`, `ounces`, `location`, `time_zone`, `species`, `photos`
        /// or `id`.
        field: String,
        /// What is wrong, in the user's voice and locale.
        message: String,
    },
    /// There is no catch with this id.
    #[error("there is no catch with the id {id}")]
    NotFound {
        /// The id asked for.
        id: String,
    },
    /// The settings, a snapshot or another input could not be read.
    #[error("{message}")]
    Invalid {
        /// What is wrong.
        message: String,
    },
}

impl From<CoreLogError> for LogError {
    fn from(e: CoreLogError) -> Self {
        Self::Book {
            message: e.to_string(),
        }
    }
}

impl From<SettingsError> for LogError {
    fn from(e: SettingsError) -> Self {
        Self::Invalid {
            message: e.to_string(),
        }
    }
}

fn invalid(message: impl Into<String>) -> LogError {
    LogError::Invalid {
        message: message.into(),
    }
}

fn field(field: &str, message: impl Into<String>) -> LogError {
    LogError::Field {
        field: field.to_owned(),
        message: message.into(),
    }
}

// ---------------------------------------------------------------------------------------------
// The book as bytes
// ---------------------------------------------------------------------------------------------

fn open(bytes: &[u8]) -> Result<LogBook, LogError> {
    let text = std::str::from_utf8(bytes).map_err(|e| LogError::Book {
        message: format!("the catch log is not valid: {e}"),
    })?;
    Ok(LogBook::from_json(text)?)
}

fn save(book: &LogBook) -> Result<Vec<u8>, LogError> {
    Ok(book.to_json()?.into_bytes())
}

fn catalogue() -> Result<&'static Catalogue, LogError> {
    static CATALOGUE: OnceLock<Result<Catalogue, String>> = OnceLock::new();
    CATALOGUE
        .get_or_init(|| Catalogue::bundled().map_err(|e| e.to_string()))
        .as_ref()
        .map_err(|e| invalid(e.clone()))
}

fn json<T: Serialize>(value: &T) -> Result<String, LogError> {
    serde_json::to_string(value).map_err(|e| invalid(e.to_string()))
}

/// An empty log, as the bytes to store.
#[uniffi::export]
pub fn log_new() -> Vec<u8> {
    LogBook::new().to_json().unwrap_or_default().into_bytes()
}

/// What a log holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Record)]
pub struct LogCounts {
    /// Catches that are not deleted.
    pub catches: u32,
    /// Records of deleted catches, kept so a sync can carry the deletion.
    pub deleted: u32,
}

/// A log read from stored bytes, in its canonical form.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct LogLoaded {
    /// The log as bytes to store (the same book always gives the same bytes).
    pub book: Vec<u8>,
    /// What is in it.
    pub counts: LogCounts,
}

fn counts(book: &LogBook) -> LogCounts {
    let live = book.live().count();
    LogCounts {
        catches: u32::try_from(live).unwrap_or(u32::MAX),
        deleted: u32::try_from(book.entries().len() - live).unwrap_or(u32::MAX),
    }
}

/// Reads a log from its stored bytes. Fields this version does not know are ignored; a log
/// written by a newer version of the app is refused.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn log_load(book: Vec<u8>) -> Result<LogLoaded, LogError> {
    let opened = open(&book)?;
    Ok(LogLoaded {
        counts: counts(&opened),
        book: save(&opened)?,
    })
}

/// A log after a change.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct LogChange {
    /// The log as bytes to store.
    pub book: Vec<u8>,
    /// Whether the log differs from before.
    pub changed: bool,
}

// ---------------------------------------------------------------------------------------------
// The editor
// ---------------------------------------------------------------------------------------------

/// What became of the fish.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FateId {
    Released,
    Kept,
    KeptForSashimi,
}

impl From<Fate> for FateId {
    fn from(f: Fate) -> Self {
        match f {
            Fate::Released => Self::Released,
            Fate::Kept => Self::Kept,
            Fate::KeptForSashimi => Self::KeptForSashimi,
        }
    }
}

impl From<FateId> for Fate {
    fn from(f: FateId) -> Self {
        match f {
            FateId::Released => Self::Released,
            FateId::Kept => Self::Kept,
            FateId::KeptForSashimi => Self::KeptForSashimi,
        }
    }
}

/// One choice of the editor's fate picker.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FateOption {
    pub id: FateId,
    /// The label in the user's voice.
    pub label: String,
}

/// The fate choices in the user's voice: "Released", "Kept" and, in Luna's voice, "Kept for
/// sashimi". The plain voice words that last one as "Kept", so it offers only the first two; a
/// catch already kept for sashimi keeps its fate unless the user picks another.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn fate_options(settings_json: String) -> Result<Vec<FateOption>, LogError> {
    let (config, _) = load(&settings_json)?;
    let s = Strings::new(config.presentation.voice);
    let mut out = vec![
        FateOption {
            id: FateId::Released,
            label: s.get(Key::LogReleased).to_owned(),
        },
        FateOption {
            id: FateId::Kept,
            label: s.get(Key::LogKept).to_owned(),
        },
    ];
    if config.presentation.voice == Voice::Luna {
        out.push(FateOption {
            id: FateId::KeptForSashimi,
            label: s.get(Key::LogKeptForSashimi).to_owned(),
        });
    }
    Ok(out)
}

/// The units the editor's size fields are typed in, with their labels.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FishUnitsInfo {
    /// `inches` or `centimetres`.
    pub length_unit: String,
    /// The unit symbol beside the length field.
    pub length_label: String,
    /// `pounds` or `kilograms`: with `pounds` the weight is typed as pounds and ounces.
    pub weight_unit: String,
    /// The unit symbol beside the weight field (pounds or kilograms).
    pub weight_label: String,
    /// The unit symbol beside the ounces field; empty when the weight is in kilograms.
    pub ounces_label: String,
}

fn units_of(units: FishUnits, s: Strings) -> FishUnitsInfo {
    let (length_unit, length_key) = match units.length {
        LengthUnit::Inches => ("inches", Key::UnitInches),
        LengthUnit::Centimetres => ("centimetres", Key::UnitCentimetres),
    };
    let pounds = units.weight == WeightUnit::Pounds;
    FishUnitsInfo {
        length_unit: length_unit.to_owned(),
        length_label: s.get(length_key).to_owned(),
        weight_unit: if pounds { "pounds" } else { "kilograms" }.to_owned(),
        weight_label: s
            .get(if pounds {
                Key::UnitPounds
            } else {
                Key::UnitKilograms
            })
            .to_owned(),
        ounces_label: if pounds {
            s.get(Key::UnitOunces).to_owned()
        } else {
            String::new()
        },
    }
}

/// The units and labels of the editor's size fields for these settings.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn fish_units_info(settings_json: String) -> Result<FishUnitsInfo, LogError> {
    let (config, _) = load(&settings_json)?;
    let f = formatter(&config, "UTC");
    Ok(units_of(fish_units(&f), f.strings))
}

/// The editor's fields: what the user types and picks, in their units.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct CatchInput {
    /// A new unique id for a new catch, the id of the catch being edited otherwise.
    pub id: String,
    /// When the fish was caught, Unix seconds.
    pub caught_at: i64,
    /// The IANA zone the catch time is read in.
    pub time_zone: String,
    /// Degrees north.
    pub latitude: f64,
    /// Degrees east.
    pub longitude: f64,
    /// The saved spot's id, if the catch is at one.
    pub spot_id: Option<String>,
    /// A place name the user typed.
    pub area: Option<String>,
    /// A catalogue species id.
    pub species: Option<String>,
    /// A species the user typed, for one not in the catalogue.
    pub species_text: Option<String>,
    /// The length as typed, in the length unit of [`fish_units_info`]; blank for none.
    pub length_text: String,
    /// The weight as typed: pounds, or kilograms.
    pub weight_text: String,
    /// The ounces as typed; used only when the weight is in pounds.
    pub ounces_text: String,
    pub fate: FateId,
    pub lure: Option<String>,
    pub method: Option<String>,
    pub notes: Option<String>,
    /// Photo file names in the app's container.
    pub photos: Vec<String>,
}

fn text(t: Option<String>) -> Option<String> {
    t.map(|t| t.trim().to_owned()).filter(|t| !t.is_empty())
}

/// The weight typed in the editor, in kilograms.
fn parse_weight_fields(
    units: FishUnits,
    weight: &str,
    ounces: &str,
    s: Strings,
) -> Result<Option<f32>, LogError> {
    match units.weight {
        WeightUnit::Pounds => parse_pounds_ounces(weight, ounces, s.locale).map_err(|e| {
            let name = if e == ParseError::OuncesTooMany {
                "ounces"
            } else {
                "weight"
            };
            field(name, e.text(s))
        }),
        unit => parse_weight(weight, unit, s.locale).map_err(|e| field("weight", e.text(s))),
    }
}

/// Whether the text of the length field is acceptable: `None` when it is (or is blank), the
/// message for the field when it is not. For checking as the user types.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn check_length_text(text: String, settings_json: String) -> Result<Option<String>, LogError> {
    let (config, _) = load(&settings_json)?;
    let f = formatter(&config, "UTC");
    Ok(parse_length(&text, fish_units(&f).length, f.strings.locale)
        .err()
        .map(|e| e.text(f.strings)))
}

/// Whether the weight fields are acceptable: `None` when they are (or are blank), the message
/// for the field when they are not.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn check_weight_text(
    weight: String,
    ounces: String,
    settings_json: String,
) -> Result<Option<String>, LogError> {
    let (config, _) = load(&settings_json)?;
    let f = formatter(&config, "UTC");
    Ok(
        match parse_weight_fields(fish_units(&f), &weight, &ounces, f.strings) {
            Err(LogError::Field { message, .. }) => Some(message),
            _ => None,
        },
    )
}

/// The editor's fields for the catch `id`, its size in the user's units.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn log_editor(
    book: Vec<u8>,
    id: String,
    settings_json: String,
) -> Result<CatchInput, LogError> {
    let book = open(&book)?;
    let (config, _) = load(&settings_json)?;
    let f = formatter(&config, "UTC");
    let units = fish_units(&f);
    let locale = f.strings.locale;
    let e = book
        .get(&id)
        .filter(|e| !e.deleted)
        .ok_or(LogError::NotFound { id })?;
    let (weight_text, ounces_text) = match (e.weight_kg, units.weight) {
        (None, _) => (String::new(), String::new()),
        (Some(kg), WeightUnit::Pounds) => {
            let po = edit_pounds_ounces(kg, locale);
            (po.pounds, po.ounces)
        }
        (Some(kg), unit) => (edit_weight(kg, unit, locale), String::new()),
    };
    Ok(CatchInput {
        id: e.id.clone(),
        caught_at: e.caught_at.0,
        time_zone: e.time_zone.clone(),
        latitude: e.location.latitude,
        longitude: e.location.longitude,
        spot_id: e.location.spot_id.clone(),
        area: e.location.area.clone(),
        species: e.species.as_ref().map(|s| s.as_str().to_owned()),
        species_text: e.species_text.clone(),
        length_text: e
            .length_m
            .map(|m| edit_length(m, units.length, locale))
            .unwrap_or_default(),
        weight_text,
        ounces_text,
        fate: e.fate.into(),
        lure: e.lure.clone(),
        method: e.method.clone(),
        notes: e.notes.clone(),
        photos: e.photos.iter().map(|p| p.as_str().to_owned()).collect(),
    })
}

/// Saves the editor's fields as catch `input.id`: a new catch, or the edit of an existing one
/// (which keeps its creation time and its conditions). Sizes are read in the user's units.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn log_save(
    book: Vec<u8>,
    input: CatchInput,
    settings_json: String,
    now: i64,
) -> Result<LogChange, LogError> {
    let mut book = open(&book)?;
    let (config, _) = load(&settings_json)?;
    let f = formatter(&config, "UTC");
    let s = f.strings;
    let units = fish_units(&f);
    let now = UnixTime(now);

    if input.id.trim().is_empty() {
        return Err(field("id", "a catch needs an id"));
    }
    if zone(&input.time_zone).is_err() {
        return Err(field(
            "time_zone",
            format!("`{}` is not a time zone", input.time_zone),
        ));
    }
    let (lat, lon) = (input.latitude, input.longitude);
    if !(lat.is_finite() && lon.is_finite() && lat.abs() <= 90.0 && lon.abs() <= 180.0) {
        return Err(field("location", "the position is not on the Earth"));
    }
    let length = parse_length(&input.length_text, units.length, s.locale)
        .map_err(|e| field("length", e.text(s)))?;
    let weight = parse_weight_fields(units, &input.weight_text, &input.ounces_text, s)?;
    let species = match text(input.species) {
        Some(id) => Some(SpeciesId::new(&id).map_err(|e| field("species", e.to_string()))?),
        None => None,
    };
    let photos = input
        .photos
        .iter()
        .map(|p| PhotoRef::new(p).map_err(|e| field("photos", e.to_string())))
        .collect::<Result<Vec<_>, _>>()?;

    let old = book.get(&input.id).cloned();
    let mut location = CatchLocation::new(lat, lon);
    location.spot_id = text(input.spot_id);
    location.area = text(input.area);
    let mut entry = CatchEntry::new(
        input.id,
        UnixTime(input.caught_at),
        input.time_zone,
        location,
        now,
    );
    entry.species = species;
    entry.species_text = text(input.species_text);
    entry.length_m = length;
    entry.weight_kg = weight;
    entry.fate = input.fate.into();
    entry.lure = text(input.lure);
    entry.method = text(input.method);
    entry.notes = text(input.notes);
    entry.photos = photos;
    if let Some(old) = old {
        entry.created_at = old.created_at;
        // An edit always supersedes the record it edits, even on a clock that ran backwards.
        entry.modified_at = UnixTime(now.0.max(old.modified_at.0.saturating_add(1)));
        if !old.deleted {
            entry.conditions = old.conditions;
        }
    }
    let changed = book.upsert(entry);
    Ok(LogChange {
        book: save(&book)?,
        changed,
    })
}

/// Attaches the conditions at `at` (the catch time when `None`) from the spot's snapshot to the
/// catch `id`, replacing any it had. Values more than an hour from `at` are left out; when the
/// snapshot holds nothing that near, the log is returned unchanged with `changed` false.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn log_attach_conditions(
    book: Vec<u8>,
    id: String,
    snapshot_json: String,
    at: Option<i64>,
    now: i64,
) -> Result<LogChange, LogError> {
    let mut book = open(&book)?;
    let snapshot: SpotSnapshot = serde_json::from_str(&snapshot_json)
        .map_err(|e| invalid(format!("the snapshot is not valid: {e}")))?;
    let mut entry = book
        .get(&id)
        .filter(|e| !e.deleted)
        .cloned()
        .ok_or(LogError::NotFound { id })?;
    let conditions =
        CatchConditions::from_snapshot(&snapshot, at.map_or(entry.caught_at, UnixTime));
    let changed = if conditions.is_empty() {
        false
    } else {
        entry.conditions = Some(conditions);
        entry.modified_at = UnixTime(now.max(entry.modified_at.0.saturating_add(1)));
        book.upsert(entry)
    };
    Ok(LogChange {
        book: save(&book)?,
        changed,
    })
}

/// A log after a deletion.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct LogDeleted {
    /// The log as bytes to store.
    pub book: Vec<u8>,
    /// Whether there was a catch to delete.
    pub deleted: bool,
    /// The catch's photo file names, for the app to delete.
    pub photos: Vec<String>,
}

/// Deletes the catch `id`. Its record stays as a tombstone with no position, notes or photos, so
/// a sync can carry the deletion.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn log_delete(book: Vec<u8>, id: String, now: i64) -> Result<LogDeleted, LogError> {
    let mut book = open(&book)?;
    let photos = book.delete(&id, UnixTime(now));
    Ok(LogDeleted {
        deleted: photos.is_some(),
        photos: photos
            .unwrap_or_default()
            .iter()
            .map(|p| p.as_str().to_owned())
            .collect(),
        book: save(&book)?,
    })
}

/// A log after a merge.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct LogMerged {
    /// The log as bytes to store.
    pub book: Vec<u8>,
    /// How many records changed.
    pub changed: u32,
}

/// Merges another log (from a sync or a backup) into this one, record by record: the later edit
/// of each catch wins, on every device the same.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn log_merge(book: Vec<u8>, other: Vec<u8>) -> Result<LogMerged, LogError> {
    let mut book = open(&book)?;
    let changed = book.merge(open(&other)?);
    Ok(LogMerged {
        changed: u32::try_from(changed).unwrap_or(u32::MAX),
        book: save(&book)?,
    })
}

// ---------------------------------------------------------------------------------------------
// Views
// ---------------------------------------------------------------------------------------------

/// A saved spot's name, to put a name to the catches that carry its id.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct PlaceName {
    pub spot_id: String,
    pub name: String,
}

fn places(spots: Vec<PlaceName>) -> Vec<SpotName> {
    spots
        .into_iter()
        .map(|p| SpotName {
            spot_id: p.spot_id,
            name: p.name,
        })
        .collect()
}

/// The catch list as JSON: the core's `CatchList`, newest first, every value in the user's units,
/// clock and voice, with the empty state when there are no catches.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn catch_list_view(
    book: Vec<u8>,
    settings_json: String,
    spots: Vec<PlaceName>,
) -> Result<String, LogError> {
    let book = open(&book)?;
    let (config, _) = load(&settings_json)?;
    let f = formatter(&config, "UTC");
    json(&catch_list(&book, &f, catalogue()?, &places(spots)))
}

/// One catch in full as JSON: the core's `CatchDetail`, its conditions each with its source.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn catch_detail_view(
    book: Vec<u8>,
    id: String,
    settings_json: String,
    spots: Vec<PlaceName>,
) -> Result<String, LogError> {
    let book = open(&book)?;
    let (config, _) = load(&settings_json)?;
    let f = formatter(&config, "UTC");
    let entry = book
        .get(&id)
        .filter(|e| !e.deleted)
        .ok_or(LogError::NotFound { id })?;
    json(&catch_detail(entry, &f, catalogue()?, &places(spots)))
}

/// Which catches to count; a field left `None` does not narrow.
#[derive(Clone, Debug, Default, PartialEq, Eq, uniffi::Record)]
pub struct CatchFilter {
    /// A catalogue species id.
    pub species: Option<String>,
    /// A saved spot's id.
    pub spot_id: Option<String>,
    /// A place name, compared ignoring case.
    pub area: Option<String>,
    /// Caught at or after this Unix time.
    pub from: Option<i64>,
    /// Caught before this Unix time.
    pub until: Option<i64>,
    /// The method, compared ignoring case.
    pub method: Option<String>,
}

fn filter_of(f: CatchFilter) -> Result<Filter, LogError> {
    Ok(Filter {
        species: text(f.species)
            .map(|s| SpeciesId::new(&s).map_err(|e| field("species", e.to_string())))
            .transpose()?,
        spot_id: text(f.spot_id),
        area: text(f.area),
        from: f.from.map(UnixTime),
        until: f.until.map(UnixTime),
        method: text(f.method),
    })
}

/// What worked, as JSON: the core's `InsightsView` over the catches that pass `filter`, with its
/// sentences, bars and the notice that a pattern is not a promise.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn insights_json(
    book: Vec<u8>,
    filter: CatchFilter,
    settings_json: String,
) -> Result<String, LogError> {
    let book = open(&book)?;
    let (config, _) = load(&settings_json)?;
    let f = formatter(&config, "UTC");
    let filter = filter_of(filter)?;
    let insights = summarise(&book, &filter);
    json(&insights_view(&insights, &filter, &f, catalogue()?))
}

/// A square of the local catch heat map.
#[derive(Clone, Copy, Debug, PartialEq, uniffi::Record)]
pub struct CatchCell {
    /// The cell's centre, degrees north.
    pub latitude: f64,
    /// The cell's centre, degrees east.
    pub longitude: f64,
    /// Catches in the cell.
    pub count: u32,
    /// All catches binned, so the app can say "3 of 5".
    pub total: u32,
}

/// The catches that pass `filter` binned onto a fixed grid of `cell_m`-metre squares. The grid is
/// fixed to the Earth, so a cell never moves when more catches are logged. A `cell_m` that is
/// not a positive number gives no cells.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn catch_cells(
    book: Vec<u8>,
    filter: CatchFilter,
    cell_m: f64,
) -> Result<Vec<CatchCell>, LogError> {
    let book = open(&book)?;
    let cells = catch_density_for(&book, &filter_of(filter)?, cell_m);
    Ok(cells
        .into_iter()
        .map(|c| CatchCell {
            latitude: c.latitude,
            longitude: c.longitude,
            count: u32::try_from(c.count).unwrap_or(u32::MAX),
            total: u32::try_from(c.total).unwrap_or(u32::MAX),
        })
        .collect())
}

/// A catch as the map pins it.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct CatchPoint {
    /// The catch's id, for [`catch_detail_view`] and [`log_editor`].
    pub id: String,
    /// Degrees north.
    pub latitude: f64,
    /// Degrees east.
    pub longitude: f64,
}

/// Where each catch was made, for the map's pins: the catches that are not deleted, in the
/// log's stable order. Deleted catches keep no position and are left out.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn catch_points(book: Vec<u8>) -> Result<Vec<CatchPoint>, LogError> {
    let book = open(&book)?;
    Ok(book
        .live()
        .map(|e| CatchPoint {
            id: e.id.clone(),
            latitude: e.location.latitude,
            longitude: e.location.longitude,
        })
        .collect())
}

// ---------------------------------------------------------------------------------------------
// Export
// ---------------------------------------------------------------------------------------------

/// The words of the log's export: its title, the two formats and the warning shown before the
/// file is shared (plain in every voice: the file holds exact places).
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct ExportWords {
    pub title: String,
    pub csv: String,
    pub gpx: String,
    pub warning: String,
}

/// The words of the log's export in the user's voice.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn log_export_words(settings_json: String) -> Result<ExportWords, LogError> {
    let (config, _) = load(&settings_json)?;
    let s = Strings::new(config.presentation.voice);
    Ok(ExportWords {
        title: s.get(Key::LogExportTitle).to_owned(),
        csv: s.get(Key::LogExportCsv).to_owned(),
        gpx: s.get(Key::LogExportGpx).to_owned(),
        warning: s.safety(SafetyKey::LogExportWarning).to_owned(),
    })
}

/// The catches as CSV text, for the user to save or share. Made only when asked for.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn log_export_csv(book: Vec<u8>) -> Result<String, LogError> {
    Ok(export_csv(&open(&book)?))
}

/// The catches as GPX waypoints, for the user to save or share. Made only when asked for.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn log_export_gpx(book: Vec<u8>) -> Result<String, LogError> {
    Ok(export_gpx(&open(&book)?))
}
