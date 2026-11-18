//! Species across the FFI: the catalogue, a profile, the user's edits, seasons and sharing a profile.
//!
//! The catalogue is bundled in the core. The user's edits are the app's to keep: it stores the
//! JSON text the calls here return (an empty text is no edits yet) and passes it back in; every
//! call that changes the edits returns the new text. Views cross as the core's JSON (the species
//! list, the profile page, the season view and the species card), every word, unit and licence
//! written by the core. A shared profile is a `.underbite` file of kind `profile`; importing one
//! never replaces an edit the user made unless the user asks for the sender's value.

use std::sync::OnceLock;

use serde::Serialize;
use underbite_config::schema::Config;
use underbite_config::view_input::view_input;
use underbite_core::model::SpotSnapshot;
use underbite_core::share::{
    Document, OnConflict, ProfileImport, ProfileShare, ShareError as Problem, decode, encode,
};
use underbite_core::species::catalogue::Catalogue;
use underbite_core::species::profile::{Field, ProfileEdits, SpeciesProfile};
use underbite_core::time::UnixTime;
use underbite_core::trip::SpeciesId;
use underbite_core::view::Card;
use underbite_core::view::cards::species::build as build_card;
use underbite_core::view::format::Formatter;
use underbite_core::view::species::{
    EditedBy, SeasonInput, apply_edit, edit_choices, field_label, profile_page, season_view,
    species_list as list_view, water_reading,
};
use underbite_core::view::strings::Strings;

use crate::settings::{SettingsError, load};
use crate::share::ShareError;

// ---------------------------------------------------------------------------------------------
// Errors and inputs
// ---------------------------------------------------------------------------------------------

/// Why a species call failed.
#[derive(Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum SpeciesError {
    /// The stored edits are not edits this version reads.
    #[error("the species edits cannot be read: {message}")]
    Edits {
        /// The parser's message.
        message: String,
    },
    /// The catalogue has no species with this id.
    #[error("there is no species with the id {id}")]
    NotFound {
        /// The id asked for.
        id: String,
    },
    /// The editor's input for one field is not acceptable.
    #[error("{message}")]
    Field {
        /// The field (`spawn_temp`, `lures`, ...).
        field: String,
        /// What is wrong, in the user's voice and locale.
        message: String,
    },
    /// The settings, a snapshot or another input could not be read.
    #[error("{message}")]
    Invalid {
        /// What is wrong.
        message: String,
    },
}

impl From<SettingsError> for SpeciesError {
    fn from(e: SettingsError) -> Self {
        Self::Invalid {
            message: e.to_string(),
        }
    }
}

fn invalid(message: impl Into<String>) -> SpeciesError {
    SpeciesError::Invalid {
        message: message.into(),
    }
}

fn catalogue() -> Result<&'static Catalogue, SpeciesError> {
    static CATALOGUE: OnceLock<Result<Catalogue, String>> = OnceLock::new();
    CATALOGUE
        .get_or_init(|| Catalogue::bundled().map_err(|e| e.to_string()))
        .as_ref()
        .map_err(|e| invalid(e.clone()))
}

/// The stored edits; blank text is no edits.
fn read_edits(text: &str) -> Result<ProfileEdits, SpeciesError> {
    if text.trim().is_empty() {
        return Ok(ProfileEdits::default());
    }
    ProfileEdits::from_json(text).map_err(|e| SpeciesError::Edits {
        message: e.to_string(),
    })
}

fn write_edits(edits: &ProfileEdits) -> Result<String, SpeciesError> {
    edits.to_json().map_err(|e| invalid(e.to_string()))
}

fn json<T: Serialize>(value: &T) -> Result<String, SpeciesError> {
    serde_json::to_string(value).map_err(|e| invalid(e.to_string()))
}

fn species_id(id: &str) -> Result<SpeciesId, SpeciesError> {
    SpeciesId::new(id).map_err(|_| SpeciesError::NotFound { id: id.to_owned() })
}

fn field_of(field: &str) -> Result<Field, SpeciesError> {
    serde_json::from_value(serde_json::Value::String(field.to_owned()))
        .map_err(|_| invalid(format!("`{field}` is not a species field")))
}

fn field_id(field: Field) -> String {
    match serde_json::to_value(field) {
        Ok(serde_json::Value::String(s)) => s,
        _ => String::new(),
    }
}

/// The profile of `id` with the user's edits laid over it.
fn profile_of(id: &str, edits: &ProfileEdits) -> Result<SpeciesProfile, SpeciesError> {
    let sid = species_id(id)?;
    catalogue()?
        .profile(&sid, edits)
        .ok_or_else(|| SpeciesError::NotFound { id: id.to_owned() })
}

/// The formatter of the settings, for words that depend on no place (UTC).
fn formatter(config: &Config, time_zone: &str) -> Formatter {
    crate::map::formatter(config, time_zone)
}

fn read_snapshot(json: &str) -> Result<SpotSnapshot, SpeciesError> {
    serde_json::from_str(json).map_err(|e| invalid(format!("invalid snapshot: {e}")))
}

// ---------------------------------------------------------------------------------------------
// The catalogue and a profile
// ---------------------------------------------------------------------------------------------

/// Every species of the catalogue by name, as the core's `[SpeciesRow]` JSON (`id`, `name`,
/// `scientific_name`, `water`, `water_label`, `edited`).
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn species_list(edits: String, settings_json: String) -> Result<String, SpeciesError> {
    let (config, _) = load(&settings_json)?;
    let edits = read_edits(&edits)?;
    json(&list_view(
        catalogue()?,
        &edits,
        Strings::new(config.presentation.voice),
    ))
}

/// The profile page of a species as the core's `ProfilePage` JSON: every field in the user's
/// units with its source and licence, the edited ones marked, and the editor's pre-filled values.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn species_profile(
    id: String,
    edits: String,
    settings_json: String,
) -> Result<String, SpeciesError> {
    let (config, _) = load(&settings_json)?;
    let profile = profile_of(&id, &read_edits(&edits)?)?;
    json(&profile_page(
        &profile,
        &formatter(&config, "UTC"),
        EditedBy::You,
    ))
}

// ---------------------------------------------------------------------------------------------
// Editing
// ---------------------------------------------------------------------------------------------

/// One choice of a picker field.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct SpeciesChoice {
    /// What [`species_edit`] takes.
    pub id: String,
    /// The label in the user's voice.
    pub label: String,
}

/// The choices of the `water`, `flow` and `light` fields; none for the others.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn species_edit_choices(
    field: String,
    settings_json: String,
) -> Result<Vec<SpeciesChoice>, SpeciesError> {
    let (config, _) = load(&settings_json)?;
    Ok(
        edit_choices(field_of(&field)?, Strings::new(config.presentation.voice))
            .into_iter()
            .map(|c| SpeciesChoice {
                id: c.id,
                label: c.label,
            })
            .collect(),
    )
}

/// The edits with one field of a species set from the editor, as the text to store. `values`
/// are in the form of the profile page's `edit` (temperatures and depths in the user's units and
/// decimal mark). An unacceptable value is [`SpeciesError::Field`] with the words to show.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn species_edit(
    edits: String,
    id: String,
    field: String,
    values: Vec<String>,
    settings_json: String,
) -> Result<String, SpeciesError> {
    let (config, _) = load(&settings_json)?;
    let mut edits = read_edits(&edits)?;
    let sid = species_id(&id)?;
    if catalogue()?.get(&sid).is_none() {
        return Err(SpeciesError::NotFound { id });
    }
    let which = field_of(&field)?;
    let f = formatter(&config, "UTC");
    let mut edit = edits.species.get(&sid).cloned().unwrap_or_default();
    apply_edit(&mut edit, which, &values, &f).map_err(|e| SpeciesError::Field {
        field: field.clone(),
        message: e.text(f.strings),
    })?;
    edits.species.insert(sid, edit);
    write_edits(&edits)
}

/// The edits with one field of a species reset to the catalogue's value, or every field when
/// `field` is `None`, as the text to store.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn species_reset(
    edits: String,
    id: String,
    field: Option<String>,
) -> Result<String, SpeciesError> {
    let mut edits = read_edits(&edits)?;
    let sid = species_id(&id)?;
    match field {
        Some(field) => edits.reset_field(&sid, field_of(&field)?),
        None => edits.reset(&sid),
    }
    write_edits(&edits)
}

// ---------------------------------------------------------------------------------------------
// Seasons and the card
// ---------------------------------------------------------------------------------------------

/// The season view of a species as the core's `SeasonView` JSON: the stage now from the spot's
/// water temperature (`snapshot_json`, when there is one), the typical months, and the official
/// regulation pages of `region` (ISO 3166-2, `US-NY`) with the user's `notes` and the regulations
/// disclaimer. Typical water temperatures by month are not passed in yet, so the months say so.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn species_seasons(
    id: String,
    edits: String,
    snapshot_json: Option<String>,
    region: Option<String>,
    notes: String,
    settings_json: String,
    now: i64,
) -> Result<String, SpeciesError> {
    let (config, _) = load(&settings_json)?;
    let profile = profile_of(&id, &read_edits(&edits)?)?;
    let snapshot = snapshot_json.as_deref().map(read_snapshot).transpose()?;
    let zone = snapshot
        .as_ref()
        .map_or("UTC", |s| s.spot.time_zone.as_str());
    let f = formatter(&config, zone);
    let water = snapshot
        .as_ref()
        .and_then(|s| water_reading(s, UnixTime(now), &f));
    let input = SeasonInput {
        water_c: water.map(|w| w.0),
        trend: water.map(|w| w.1),
        region: region.as_deref(),
        notes: &notes,
        trout_stress_c: Some(config.thresholds.trout_stress_c as f32),
        ..SeasonInput::default()
    };
    json(&season_view(&profile, &input, &f, EditedBy::You))
}

/// The species card for the chosen species (`ids`, in the user's order) at a spot, as the
/// core's card JSON (`{"card": "species", ...}`, as in the spot view); `None` when no species is
/// chosen. An id the catalogue lacks is [`SpeciesError::NotFound`].
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn species_card(
    snapshot_json: String,
    ids: Vec<String>,
    edits: String,
    settings_json: String,
    now: i64,
) -> Result<Option<String>, SpeciesError> {
    let (config, _) = load(&settings_json)?;
    let edits = read_edits(&edits)?;
    let profiles = ids
        .iter()
        .map(|id| profile_of(id, &edits))
        .collect::<Result<Vec<_>, _>>()?;
    let input = view_input(&config, read_snapshot(&snapshot_json)?, UnixTime(now));
    build_card(&input, &profiles)
        .map(|card| json(&Card::Species(card)))
        .transpose()
}

// ---------------------------------------------------------------------------------------------
// Sharing
// ---------------------------------------------------------------------------------------------

fn share_invalid(what: &str, e: &impl ToString) -> ShareError {
    ShareError::Invalid {
        what: what.to_owned(),
        message: e.to_string(),
    }
}

fn edits_for_share(edits: &str) -> Result<ProfileEdits, ShareError> {
    read_edits(edits).map_err(|e| share_invalid("species edits", &e))
}

fn catalogue_for_share() -> Result<&'static Catalogue, ShareError> {
    catalogue().map_err(|e| share_invalid("catalogue", &e))
}

/// The `.underbite` file of a species profile with the user's edits of it: every value with its
/// source and licence (a `FishBase` value with its CC BY-NC citation), the edits kept apart.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn species_share_file(id: String, edits: String) -> Result<String, ShareError> {
    let edits = edits_for_share(&edits)?;
    let sid = SpeciesId::new(&id).map_err(|e| share_invalid("species", &e))?;
    let share =
        ProfileShare::from_catalogue(catalogue_for_share()?, &sid, &edits).ok_or_else(|| {
            share_invalid("species", &format!("there is no species with the id {id}"))
        })?;
    Ok(encode(&Document::Profile(share))?)
}

/// One field of an import, with its label.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct ProfileField {
    /// `spawn_temp`, `lures`, ...
    pub field: String,
    /// "Spawning temperature", in the user's voice.
    pub label: String,
}

/// What importing a shared profile does to the user's edits.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct ProfileImportPlan {
    /// The species id.
    pub id: String,
    /// The species is in this version's catalogue; when not, nothing can be imported.
    pub known: bool,
    /// The sender edited these and the user has not: they are imported.
    pub added: Vec<ProfileField>,
    /// The sender's value is already the user's.
    pub same: Vec<ProfileField>,
    /// Both edited these differently: the user's edit stays unless they take the sender's.
    pub conflicts: Vec<ProfileField>,
}

fn plan_record(plan: &ProfileImport, s: Strings) -> ProfileImportPlan {
    let fields = |list: &[Field]| {
        list.iter()
            .map(|f| ProfileField {
                field: field_id(*f),
                label: field_label(*f, s).to_owned(),
            })
            .collect()
    };
    ProfileImportPlan {
        id: plan.id.as_str().to_owned(),
        known: plan.known,
        added: fields(&plan.added),
        same: fields(&plan.same),
        conflicts: fields(&plan.conflicts),
    }
}

/// A shared profile as read, for the preview before importing it.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct SharedProfile {
    /// The species id.
    pub id: String,
    /// The name the sender's profile shows.
    pub name: String,
    /// The sender's profile page (the core's `ProfilePage` JSON), the sender's edits marked and
    /// credited to them, with every source and licence of the file.
    pub page_json: String,
    /// What importing it would do.
    pub plan: ProfileImportPlan,
}

fn read_share(bytes: &[u8]) -> Result<ProfileShare, ShareError> {
    match decode(bytes)? {
        Document::Profile(share) => Ok(share),
        _ => Err(Problem::Unreadable.into()),
    }
}

/// Reads a shared profile file for its preview. Another kind of file is
/// [`ShareError::Unreadable`]; read those with `read_file`.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn species_read_file(
    bytes: Vec<u8>,
    edits: String,
    settings_json: String,
) -> Result<SharedProfile, ShareError> {
    let (config, _) = load(&settings_json).map_err(|e| share_invalid("settings", &e))?;
    let share = read_share(&bytes)?;
    let edits = edits_for_share(&edits)?;
    let shown = share.shown();
    let f = formatter(&config, "UTC");
    let page = profile_page(&shown, &f, EditedBy::Sender);
    Ok(SharedProfile {
        id: shown.id.as_str().to_owned(),
        name: shown.display_name().to_owned(),
        page_json: serde_json::to_string(&page).map_err(|e| share_invalid("profile", &e))?,
        plan: plan_record(
            &share.plan_import(catalogue_for_share()?, &edits),
            f.strings,
        ),
    })
}

/// The user's edits after importing a shared profile.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct ProfileImported {
    /// The edits to store (unchanged when nothing was imported).
    pub edits: String,
    /// What the import did; with `take_theirs` the conflicts were replaced by the sender's
    /// values, otherwise they were kept.
    pub plan: ProfileImportPlan,
}

/// Imports a shared profile into the user's edits. Fields the user has not edited take the
/// sender's values; fields both edited differently keep the user's unless `take_theirs`.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn species_import(
    bytes: Vec<u8>,
    edits: String,
    take_theirs: bool,
    settings_json: String,
) -> Result<ProfileImported, ShareError> {
    let (config, _) = load(&settings_json).map_err(|e| share_invalid("settings", &e))?;
    let share = read_share(&bytes)?;
    let mut edits = edits_for_share(&edits)?;
    let on_conflict = if take_theirs {
        OnConflict::TakeTheirs
    } else {
        OnConflict::KeepMine
    };
    let plan = share.import_into(catalogue_for_share()?, &mut edits, on_conflict);
    Ok(ProfileImported {
        edits: write_edits(&edits).map_err(|e| share_invalid("species edits", &e))?,
        plan: plan_record(&plan, Strings::new(config.presentation.voice)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::default_settings_json;
    use serde_json::Value;
    use underbite_core::model::snapshot::testdata::sample_snapshot;

    const BASS: &str = "micropterus-dolomieu";

    fn settings() -> String {
        default_settings_json()
    }

    fn value(json: &str) -> Value {
        serde_json::from_str(json).unwrap()
    }

    fn edited() -> String {
        let e = species_edit(
            String::new(),
            BASS.into(),
            "lures".into(),
            vec!["Tube jig".into()],
            settings(),
        )
        .unwrap();
        species_edit(
            e,
            BASS.into(),
            "spawn_temp".into(),
            vec!["59".into(), "64".into()],
            settings(),
        )
        .unwrap()
    }

    #[test]
    fn species_list_and_profile_carry_sources_and_marks() {
        let list = value(&species_list(edited(), settings()).unwrap());
        let rows = list.as_array().unwrap();
        assert!(rows.len() >= 30);
        let bass = rows.iter().find(|r| r["id"] == BASS).unwrap();
        assert_eq!(bass["edited"], true);
        let page = value(&species_profile(BASS.into(), edited(), settings()).unwrap());
        let fields = page["fields"].as_array().unwrap();
        let lures = fields.iter().find(|f| f["field"] == "lures").unwrap();
        assert_eq!(lures["edited"], true);
        assert_eq!(lures["source"]["name"], "Your edit");
        let tolerated = fields
            .iter()
            .find(|f| f["field"] == "tolerated_temp")
            .unwrap();
        assert_eq!(tolerated["source"]["licence_label"], "CC BY-NC 4.0");
        assert_eq!(page["notices"].as_array().unwrap().len(), 1);
        assert!(matches!(
            species_profile("nemo-imaginarius".into(), String::new(), settings()),
            Err(SpeciesError::NotFound { .. })
        ));
        assert!(matches!(
            species_list("{not edits".into(), settings()),
            Err(SpeciesError::Edits { .. })
        ));
    }

    #[test]
    fn species_edits_are_checked_in_the_users_words_and_reset() {
        let bad = species_edit(
            String::new(),
            BASS.into(),
            "spawn_temp".into(),
            vec!["64".into(), "59".into()],
            settings(),
        );
        assert_eq!(
            bad,
            Err(SpeciesError::Field {
                field: "spawn_temp".into(),
                message: "The first number must not be above the second.".into()
            })
        );
        assert!(matches!(
            species_edit(
                String::new(),
                BASS.into(),
                "colour".into(),
                vec![],
                settings()
            ),
            Err(SpeciesError::Invalid { .. })
        ));
        let e = edited();
        let one = species_reset(e.clone(), BASS.into(), Some("lures".into())).unwrap();
        let v = value(&one);
        assert!(v["species"][BASS].get("lures").is_none());
        assert!(v["species"][BASS].get("spawn_temp_c").is_some());
        let none = species_reset(e, BASS.into(), None).unwrap();
        assert_eq!(value(&none)["species"], serde_json::json!({}));
        let choices = species_edit_choices("water".into(), settings()).unwrap();
        assert_eq!(
            choices.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
            ["fresh", "salt", "both"]
        );
    }

    #[test]
    fn species_seasons_keep_three_sections_and_the_disclaimer() {
        let snap = serde_json::to_string(&sample_snapshot()).unwrap();
        let at = sample_snapshot().fetched_at.0;
        let v = value(
            &species_seasons(
                BASS.into(),
                String::new(),
                Some(snap),
                Some("US-NY".into()),
                "My notes".into(),
                settings(),
                at,
            )
            .unwrap(),
        );
        assert_eq!(v["now"]["stage"], "spawning");
        assert_eq!(v["typical"]["months"].as_array().unwrap().len(), 12);
        assert!(v["typical"]["note"].is_string());
        assert!(
            v["legal"]["disclaimer"]
                .as_str()
                .unwrap()
                .starts_with("Fishing regulations change")
        );
        assert_eq!(v["legal"]["notes"], "My notes");
        assert_ne!(v["legal"]["links"].as_array().unwrap().len(), 0);
        let bare = value(
            &species_seasons(
                BASS.into(),
                String::new(),
                None,
                None,
                String::new(),
                settings(),
                at,
            )
            .unwrap(),
        );
        assert_eq!(bare["now"]["stage"], "unknown");
        assert!(bare["legal"]["empty"].is_string());
    }

    #[test]
    fn species_card_is_the_view_cards_json() {
        let snap = serde_json::to_string(&sample_snapshot()).unwrap();
        let at = sample_snapshot().fetched_at.0;
        let card = species_card(
            snap.clone(),
            vec![BASS.into(), "salmo-trutta".into()],
            edited(),
            settings(),
            at,
        )
        .unwrap()
        .unwrap();
        let v = value(&card);
        assert_eq!(v["card"], "species");
        assert_eq!(v["chrome"]["id"], "species");
        assert_eq!(v["full"]["species"][0]["edited"], true);
        assert_eq!(v["full"]["species"].as_array().unwrap().len(), 2);
        assert_eq!(
            species_card(snap.clone(), vec![], String::new(), settings(), at),
            Ok(None)
        );
        assert!(matches!(
            species_card(
                snap,
                vec!["nemo-imaginarius".into()],
                String::new(),
                settings(),
                at
            ),
            Err(SpeciesError::NotFound { .. })
        ));
    }

    #[test]
    fn species_share_and_import_never_overwrite_silently() {
        let file = species_share_file(BASS.into(), edited()).unwrap();
        assert!(file.contains("\"kind\": \"profile\""));
        assert!(file.contains("cc_by_nc4"));
        // The receiver typed their own lures.
        let mine = species_edit(
            String::new(),
            BASS.into(),
            "lures".into(),
            vec!["Ned rig".into()],
            settings(),
        )
        .unwrap();
        let preview =
            species_read_file(file.clone().into_bytes(), mine.clone(), settings()).unwrap();
        assert_eq!(preview.name, "Smallmouth bass");
        assert!(preview.plan.known);
        assert_eq!(preview.plan.added[0].field, "spawn_temp");
        assert_eq!(preview.plan.added[0].label, "Spawning temperature");
        assert_eq!(preview.plan.conflicts[0].field, "lures");
        let page = value(&preview.page_json);
        assert!(
            page["credits"]
                .as_array()
                .unwrap()
                .iter()
                .any(|c| c["source"] == "Edited by the sender")
        );

        let kept =
            species_import(file.clone().into_bytes(), mine.clone(), false, settings()).unwrap();
        let v = value(&kept.edits);
        assert_eq!(v["species"][BASS]["lures"], serde_json::json!(["Ned rig"]));
        assert!(v["species"][BASS].get("spawn_temp_c").is_some());
        let taken = species_import(file.into_bytes(), mine, true, settings()).unwrap();
        assert_eq!(
            value(&taken.edits)["species"][BASS]["lures"],
            serde_json::json!(["Tube jig"])
        );

        let trip = br#"{"format":"underbite","v":9,"kind":"profile"}"#.to_vec();
        assert!(matches!(
            species_read_file(trip, String::new(), settings()),
            Err(ShareError::NeedsNewerVersion { .. })
        ));
        assert!(matches!(
            species_share_file("nemo-imaginarius".into(), String::new()),
            Err(ShareError::Invalid { .. })
        ));
    }
}
