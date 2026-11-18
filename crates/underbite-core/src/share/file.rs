//! The `.underbite` document: envelope, payloads, encoding, decoding and fuzz.
//!
//! A file is JSON: `{"format":"underbite","v":1,"kind":"trip"|"spot"|"layers"|"profile", <kind>: {…}}`.
//! One file type carries every kind, so the OS registers a single document type. Fields this build
//! does not know are ignored within a known version, except inside a species profile, whose types
//! refuse fields they do not know. A file never carries keys, the catch log or photos.
//!
//! A shared map layer preset (`layers`) is a name and layer states in drawing order: no places,
//! keys or dates. Reading one leaves out the layers this build does not have and says which
//! ([`Document::Layers`]); a receiver who has not opted in to Copernicus Marine gets that
//! source's layers switched off ([`LayersShare::hide_copernicus`]).
//!
//! A shared species profile carries every value with its source and licence (a `FishBase` value
//! keeps its link, record reference and CC BY-NC citation) and the sender's edits apart from the
//! catalogue's values. Importing one adds the sender's edits to this device's; where both edited a
//! field differently the device's edit stays unless the user takes the sender's
//! ([`ProfileShare::import_into`]).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::map::{LayerId, catalogue};
use crate::species::catalogue::Catalogue;
use crate::species::profile::{
    Field, Layer, Licence, ProfileEdits, SourceRef, SpeciesEdit, SpeciesProfile,
};
use crate::trip::model::{Fuzz, PlaceProblem, SpeciesId, Trip, TripError, TripPlace};

/// The `format` member of every file.
pub const FORMAT: &str = "underbite";
/// The payload version this build reads and writes.
pub const VERSION: u32 = 1;
/// The most bytes [`decode`] reads and [`encode`] writes.
pub const MAX_BYTES: usize = 1_000_000;

/// The `kind: "spot"` payload: one place and what its owner wrote about it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SpotShare {
    pub place: TripPlace,
    #[serde(default)]
    pub notes: String,
}

/// The `kind: "profile"` payload: one species profile as its sender has it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ProfileShare {
    /// The profile as the sender's catalogue holds it: every value with its source and licence,
    /// and no edits.
    pub species: SpeciesProfile,
    /// The sender's own values, laid over `species` when shown; absent when there are none.
    #[serde(default, skip_serializing_if = "SpeciesEdit::is_empty")]
    pub edits: SpeciesEdit,
}

/// One layer of a shared map preset: its state as the map settings keep it in a preset.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SharedLayer {
    pub id: LayerId,
    /// Whether the preset shows the layer.
    #[serde(default)]
    pub shown: bool,
    /// 0 (clear) to 1 (solid); none means the layer's own.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "decimal_opacity"
    )]
    pub opacity: Option<f32>,
    /// The colour ramp by id; none means the layer's own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ramp: Option<String>,
    /// The value range the ramp spans, low and high, in canonical units; none means the layer's
    /// own.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "decimal_range"
    )]
    pub range: Option<(f32, f32)>,
}

/// `x` as the `f64` of its shortest decimal form, so that a file says 0.7 where a widening
/// conversion would say 0.699999988079071.
fn decimal(x: f32) -> f64 {
    x.to_string().parse().unwrap_or_else(|_| f64::from(x))
}

#[allow(clippy::ref_option, clippy::trivially_copy_pass_by_ref)] // serde's `serialize_with` form
fn decimal_opacity<S: serde::Serializer>(o: &Option<f32>, s: S) -> Result<S::Ok, S::Error> {
    o.map(decimal).serialize(s)
}

#[allow(clippy::ref_option, clippy::trivially_copy_pass_by_ref)] // serde's `serialize_with` form
fn decimal_range<S: serde::Serializer>(r: &Option<(f32, f32)>, s: S) -> Result<S::Ok, S::Error> {
    r.map(|(low, high)| (decimal(low), decimal(high)))
        .serialize(s)
}

/// The `kind: "layers"` payload: a map layer preset, its name and its layers in drawing order,
/// first at the bottom.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LayersShare {
    /// The preset's name as the sender sees it.
    pub name: String,
    #[serde(default)]
    pub layers: Vec<SharedLayer>,
}

/// The start of the id of every map layer Copernicus Marine serves: those layers need the
/// user's opt-in to that source.
pub const COPERNICUS_LAYER_PREFIX: &str = "copernicus-";

impl LayersShare {
    /// Switches off every shown layer that needs the Copernicus Marine opt-in, for a receiver
    /// who has not opted in, and returns their ids. The layers stay in the preset, hidden.
    pub fn hide_copernicus(&mut self) -> Vec<LayerId> {
        let mut hidden = Vec::new();
        for layer in &mut self.layers {
            if layer.shown && layer.id.as_str().starts_with(COPERNICUS_LAYER_PREFIX) {
                layer.shown = false;
                hidden.push(layer.id.clone());
            }
        }
        hidden
    }

    /// Keeps the first state of each layer `known` accepts and returns the ids of the others,
    /// each once, in file order.
    fn keep_known(&mut self, known: impl Fn(&str) -> bool) -> Vec<LayerId> {
        let mut left_out: Vec<LayerId> = Vec::new();
        let mut kept: Vec<SharedLayer> = Vec::with_capacity(self.layers.len());
        for layer in self.layers.drain(..) {
            if !known(layer.id.as_str()) {
                if !left_out.contains(&layer.id) {
                    left_out.push(layer.id);
                }
            } else if !kept.iter().any(|k| k.id == layer.id) {
                kept.push(layer);
            }
        }
        self.layers = kept;
        left_out
    }

    /// Checks what a file may carry: a name, and only opacities and ranges the map can use.
    fn validate(&self) -> Result<(), ShareError> {
        let usable = self.layers.iter().all(|l| {
            !l.id.as_str().trim().is_empty()
                && l.opacity
                    .is_none_or(|o| o.is_finite() && (0.0..=1.0).contains(&o))
                && l.range
                    .is_none_or(|(low, high)| low.is_finite() && high.is_finite() && low < high)
        });
        if self.name.trim().is_empty() || !usable {
            return Err(ShareError::Unreadable);
        }
        Ok(())
    }
}

/// Whether this build's map has an overlay with this id.
fn is_known_layer(id: &str) -> bool {
    catalogue::LAYERS.iter().any(|l| l.id == id)
}

/// What importing a shared profile does to this device's edits of the same species.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileImport {
    pub id: SpeciesId,
    /// This build's catalogue has the species. When it has not, nothing is imported.
    pub known: bool,
    /// Fields the sender edited and this device has not: they become this device's edits.
    pub added: Vec<Field>,
    /// Fields where the sender's value is already this device's (its edit or the catalogue's).
    pub same: Vec<Field>,
    /// Fields both edited to different values: this device's edit stays unless the user takes
    /// the sender's.
    pub conflicts: Vec<Field>,
}

/// What to do with a field both sides edited differently.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnConflict {
    /// Keep this device's edit.
    KeepMine,
    /// Take the sender's value, as the user asked.
    TakeTheirs,
}

/// Calls `$m!(Field, field_name)` for every editable field of a profile.
macro_rules! each_field {
    ($m:ident) => {
        $m!(CommonNames, common_names);
        $m!(Water, water);
        $m!(PreferredTemp, preferred_temp_c);
        $m!(ToleratedTemp, tolerated_temp_c);
        $m!(SpawnTemp, spawn_temp_c);
        $m!(Depth, depth_m);
        $m!(Habitat, habitat);
        $m!(Flow, flow);
        $m!(Light, light);
        $m!(Lures, lures);
        $m!(Regions, regions);
    };
}

impl ProfileShare {
    /// The share of one species of `catalogue` with this device's `edits` of it; `None` when the
    /// catalogue has no such species.
    pub fn from_catalogue(
        catalogue: &Catalogue,
        id: &SpeciesId,
        edits: &ProfileEdits,
    ) -> Option<Self> {
        Some(Self {
            species: catalogue.get(id)?.clone(),
            edits: edits.species.get(id).cloned().unwrap_or_default(),
        })
    }

    /// The profile as the sender reads it: the edits laid over it, each edited field
    /// `user_edited`.
    pub fn shown(&self) -> SpeciesProfile {
        self.species.with_edits(&self.edits)
    }

    /// What [`Self::import_into`] would do, without doing it.
    pub fn plan_import(&self, catalogue: &Catalogue, edits: &ProfileEdits) -> ProfileImport {
        let id = &self.species.id;
        let mut plan = ProfileImport {
            id: id.clone(),
            known: false,
            added: Vec::new(),
            same: Vec::new(),
            conflicts: Vec::new(),
        };
        let Some(base) = catalogue.get(id) else {
            return plan;
        };
        plan.known = true;
        let mine = edits.species.get(id);
        let theirs = &self.edits;
        macro_rules! compare {
            ($field:ident, $name:ident) => {
                if let Some(value) = &theirs.$name {
                    match mine.and_then(|m| m.$name.as_ref()) {
                        Some(own) if own == value => plan.same.push(Field::$field),
                        Some(_) => plan.conflicts.push(Field::$field),
                        None if base.$name.as_ref().is_some_and(|b| &b.value == value) => {
                            plan.same.push(Field::$field);
                        }
                        None => plan.added.push(Field::$field),
                    }
                }
            };
        }
        each_field!(compare);
        plan
    }

    /// Adds the sender's edits to `edits`: fields this device has not edited are taken, fields
    /// it edited differently stay as they are unless `on_conflict` is
    /// [`OnConflict::TakeTheirs`]. A species this build's catalogue lacks is not imported.
    /// Returns the plan that was carried out.
    pub fn import_into(
        &self,
        catalogue: &Catalogue,
        edits: &mut ProfileEdits,
        on_conflict: OnConflict,
    ) -> ProfileImport {
        let plan = self.plan_import(catalogue, edits);
        let take: Vec<Field> = match on_conflict {
            OnConflict::KeepMine => plan.added.clone(),
            OnConflict::TakeTheirs => plan.added.iter().chain(&plan.conflicts).copied().collect(),
        };
        if !plan.known || take.is_empty() {
            return plan;
        }
        let theirs = &self.edits;
        let mine = edits.edit(&self.species.id);
        macro_rules! copy {
            ($field:ident, $name:ident) => {
                if take.contains(&Field::$field) {
                    mine.$name.clone_from(&theirs.$name);
                }
            };
        }
        each_field!(copy);
        plan
    }

    /// Checks what a file may carry: every value sourced and sane, no edit posing as a source,
    /// and every CC BY-NC value with its link, record reference and citation.
    fn validate(&self) -> Result<(), ShareError> {
        let filled = |o: &Option<String>| o.as_deref().is_some_and(|t| !t.trim().is_empty());
        let sources_ok = self.species.sources().iter().all(|src| {
            !src.name.trim().is_empty()
                && src.licence != Licence::UserEdited
                && (src.licence != Licence::CcByNc4
                    || (filled(&src.url) && filled(&src.reference) && filled(&src.citation)))
        });
        if !sources_ok {
            return Err(ShareError::Unreadable);
        }
        // The values, edited or not, as the catalogue checks them; the sources were checked above.
        neutral_sources(self.shown())
            .validate(Layer::Agency)
            .map_err(|_| ShareError::Unreadable)
    }
}

/// `p` with every source replaced by one that any layer accepts, to check only the values.
fn neutral_sources(mut p: SpeciesProfile) -> SpeciesProfile {
    let neutral = || SourceRef {
        name: "check".to_owned(),
        url: None,
        licence: Licence::Published,
        reference: None,
        citation: None,
    };
    p.scientific_name.source = neutral();
    for a in &mut p.aliases {
        a.source = neutral();
    }
    macro_rules! clear {
        ($field:ident, $name:ident) => {
            if let Some(v) = &mut p.$name {
                v.source = neutral();
            }
        };
    }
    each_field!(clear);
    p
}

/// What a `.underbite` file holds.
#[derive(Clone, Debug, PartialEq)]
// A document is read or written once and moved whole; boxing would only complicate callers.
#[allow(clippy::large_enum_variant)]
pub enum Document {
    /// `kind: "trip"`.
    Trip(Trip),
    /// `kind: "spot"`.
    Spot(SpotShare),
    /// `kind: "profile"`.
    Profile(ProfileShare),
    /// `kind: "layers"`: a map layer preset.
    Layers {
        share: LayersShare,
        /// The layer ids [`decode`] left out because this build does not have them, each once,
        /// in file order. [`encode`] ignores it.
        left_out: Vec<LayerId>,
    },
}

/// Why a file cannot be used.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ShareError {
    /// Its `v` is above this build's: a newer version of the app made it.
    #[error("this file needs a newer version of the app")]
    NeedsNewerVersion,
    /// The last day is before the first.
    #[error("the dates are the wrong way round")]
    DatesBackwards,
    /// The trip covers more than 31 days.
    #[error("the trip is longer than 31 days")]
    TooLong,
    /// Not an `.underbite` file, or one with a place or payload that cannot exist.
    #[error("that is not a file this app can read")]
    Unreadable,
    /// The input is larger than [`MAX_BYTES`].
    #[error("the file is too large")]
    TooLarge,
}

impl From<TripError> for ShareError {
    fn from(e: TripError) -> Self {
        match e {
            TripError::DatesBackwards => Self::DatesBackwards,
            TripError::TooLong => Self::TooLong,
            TripError::NoPlaces | TripError::Place { .. } => Self::Unreadable,
        }
    }
}

impl From<PlaceProblem> for ShareError {
    fn from(_: PlaceProblem) -> Self {
        Self::Unreadable
    }
}

/// Reads a file.
///
/// The version is checked before the payload, so a file from a newer app reports
/// [`ShareError::NeedsNewerVersion`] whatever its payload looks like.
///
/// # Errors
/// [`ShareError`]: too large, not JSON or not this format, a newer `v`, an unknown `kind`, a
/// payload that does not parse, a trip whose dates or places do not validate, a profile with
/// a value that has no source or cannot be right, or a layer preset without a name or with an
/// opacity or range the map cannot use.
pub fn decode(bytes: &[u8]) -> Result<Document, ShareError> {
    if bytes.len() > MAX_BYTES {
        return Err(ShareError::TooLarge);
    }
    let Ok(Value::Object(mut file)) = serde_json::from_slice::<Value>(bytes) else {
        return Err(ShareError::Unreadable);
    };
    if file.get("format").and_then(Value::as_str) != Some(FORMAT) {
        return Err(ShareError::Unreadable);
    }
    match file.get("v").and_then(Value::as_u64) {
        Some(v) if v > u64::from(VERSION) => return Err(ShareError::NeedsNewerVersion),
        Some(v) if v >= 1 => {}
        _ => return Err(ShareError::Unreadable),
    }
    let kind = file
        .get("kind")
        .and_then(Value::as_str)
        .ok_or(ShareError::Unreadable)?
        .to_owned();
    match kind.as_str() {
        "trip" => {
            let trip: Trip = payload(&mut file, "trip")?;
            trip.validate()?;
            Ok(Document::Trip(trip))
        }
        "spot" => {
            let spot: SpotShare = payload(&mut file, "spot")?;
            spot.place.validate()?;
            Ok(Document::Spot(spot))
        }
        "profile" => {
            let share: ProfileShare = payload(&mut file, "profile")?;
            share.validate()?;
            Ok(Document::Profile(share))
        }
        "layers" => {
            let mut share: LayersShare = payload(&mut file, "layers")?;
            share.validate()?;
            let left_out = share.keep_known(is_known_layer);
            Ok(Document::Layers { share, left_out })
        }
        _ => Err(ShareError::Unreadable),
    }
}

fn payload<T: serde::de::DeserializeOwned>(
    file: &mut Map<String, Value>,
    key: &str,
) -> Result<T, ShareError> {
    let value = file.remove(key).ok_or(ShareError::Unreadable)?;
    serde_json::from_value(value).map_err(|_| ShareError::Unreadable)
}

/// Writes a file: pretty-printed JSON with sorted keys, so the same document always gives the same
/// bytes.
///
/// # Errors
/// The same validation errors as [`decode`], and [`ShareError::TooLarge`] when the output would
/// exceed [`MAX_BYTES`].
pub fn encode(doc: &Document) -> Result<String, ShareError> {
    let file = match doc {
        Document::Trip(trip) => {
            trip.validate()?;
            json!({"format": FORMAT, "v": VERSION, "kind": "trip", "trip": trip})
        }
        Document::Spot(spot) => {
            spot.place.validate()?;
            json!({"format": FORMAT, "v": VERSION, "kind": "spot", "spot": spot})
        }
        Document::Profile(share) => {
            share.validate()?;
            json!({"format": FORMAT, "v": VERSION, "kind": "profile", "profile": share})
        }
        Document::Layers { share, .. } => {
            share.validate()?;
            json!({"format": FORMAT, "v": VERSION, "kind": "layers", "layers": share})
        }
    };
    let text = serde_json::to_string_pretty(&file).map_err(|_| ShareError::Unreadable)?;
    if text.len() > MAX_BYTES {
        return Err(ShareError::TooLarge);
    }
    Ok(text)
}

/// Reduces the location of every place in `doc` to `level`, deterministically (never by random
/// offsets, which two shares could average back).
///
/// A place never gets more precise: asking for a level at or above what a place already carries
/// leaves it alone. The level is recorded on the place so the receiver can say "approximate
/// location". `Exact` changes nothing; `Rounded1Km` rounds to 0.01 degrees; `AreaOnly` drops the
/// coordinates and keeps name, area and region. A profile and a layer preset have no places.
pub fn fuzz(doc: &mut Document, level: Fuzz) {
    match doc {
        Document::Trip(trip) => {
            for place in &mut trip.places {
                fuzz_place(place, level);
            }
        }
        Document::Spot(spot) => fuzz_place(&mut spot.place, level),
        Document::Profile(_) | Document::Layers { .. } => {}
    }
}

fn fuzz_place(place: &mut TripPlace, level: Fuzz) {
    if level <= place.fuzz {
        return;
    }
    match level {
        Fuzz::Exact => {}
        Fuzz::Rounded1Km => {
            place.latitude = place.latitude.map(hundredth);
            place.longitude = place.longitude.map(hundredth);
        }
        Fuzz::AreaOnly => {
            place.latitude = None;
            place.longitude = None;
        }
    }
    place.fuzz = level;
}

/// `x` rounded to 0.01, with no negative zero.
fn hundredth(x: f64) -> f64 {
    (x * 100.0).round() / 100.0 + 0.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trip::model::Method;
    use crate::trip::model::tests::{battery, trip};

    fn spot() -> Document {
        Document::Spot(SpotShare {
            place: battery(),
            notes: "Park at the Battery Place garage.".into(),
        })
    }

    fn encoded(doc: &Document) -> Value {
        serde_json::from_str(&encode(doc).unwrap()).unwrap()
    }

    #[test]
    fn trip_round_trips() {
        let doc = Document::Trip(trip());
        let text = encode(&doc).unwrap();
        assert_eq!(decode(text.as_bytes()), Ok(doc.clone()));
        let v = encoded(&doc);
        assert_eq!(v["format"], "underbite");
        assert_eq!(v["v"], 1);
        assert_eq!(v["kind"], "trip");
        assert_eq!(v["trip"]["first"], "2027-07-12");
        assert_eq!(encode(&doc).unwrap(), text, "deterministic");
    }

    #[test]
    fn spot_round_trips() {
        let doc = spot();
        let text = encode(&doc).unwrap();
        assert_eq!(decode(text.as_bytes()), Ok(doc.clone()));
        assert_eq!(encoded(&doc)["kind"], "spot");
        assert_eq!(encoded(&doc)["spot"]["place"]["name"], "The Battery");
    }

    #[test]
    fn a_newer_version_is_refused_before_the_payload_is_read() {
        let doc = Document::Trip(trip());
        let mut v = encoded(&doc);
        v["v"] = 2.into();
        assert_eq!(
            decode(v.to_string().as_bytes()),
            Err(ShareError::NeedsNewerVersion)
        );
        let odd = br#"{"format":"underbite","v":2,"kind":"cairn","cairn":[1]}"#;
        assert_eq!(decode(odd), Err(ShareError::NeedsNewerVersion));
    }

    #[test]
    fn a_trip_over_31_days_is_too_long() {
        let mut t = trip();
        t.first = crate::time::LocalDate {
            year: 2027,
            month: 7,
            day: 1,
        };
        t.last = crate::time::LocalDate {
            year: 2027,
            month: 8,
            day: 1,
        };
        assert_eq!(encode(&Document::Trip(t.clone())), Err(ShareError::TooLong));
        t.last.day = 31;
        t.last.month = 7;
        let text = encode(&Document::Trip(t.clone())).unwrap();
        let mut v: Value = serde_json::from_str(&text).unwrap();
        assert!(decode(text.as_bytes()).is_ok());
        v["trip"]["last"] = "2027-08-01".into();
        assert_eq!(decode(v.to_string().as_bytes()), Err(ShareError::TooLong));
    }

    #[test]
    fn dates_backwards() {
        let mut v = encoded(&Document::Trip(trip()));
        v["trip"]["last"] = "2027-07-11".into();
        assert_eq!(
            decode(v.to_string().as_bytes()),
            Err(ShareError::DatesBackwards)
        );
        let mut t = trip();
        t.last = crate::time::LocalDate {
            year: 2027,
            month: 7,
            day: 11,
        };
        assert_eq!(encode(&Document::Trip(t)), Err(ShareError::DatesBackwards));
    }

    #[test]
    fn unreadable_inputs() {
        let good = encoded(&Document::Trip(trip()));
        let with = |f: &dyn Fn(&mut Value)| {
            let mut v = good.clone();
            f(&mut v);
            decode(v.to_string().as_bytes())
        };
        assert_eq!(decode(b""), Err(ShareError::Unreadable));
        assert_eq!(decode(b"not json"), Err(ShareError::Unreadable));
        assert_eq!(decode(b"[1,2]"), Err(ShareError::Unreadable));
        assert_eq!(decode(&[0xff, 0xfe]), Err(ShareError::Unreadable));
        assert_eq!(
            with(&|v| v["format"] = "whetherplan".into()),
            Err(ShareError::Unreadable)
        );
        assert_eq!(
            with(&|v| {
                v.as_object_mut().unwrap().remove("format");
            }),
            Err(ShareError::Unreadable)
        );
        assert_eq!(with(&|v| v["v"] = 0.into()), Err(ShareError::Unreadable));
        assert_eq!(with(&|v| v["v"] = "1".into()), Err(ShareError::Unreadable));
        assert_eq!(
            with(&|v| v["kind"] = "cairn".into()),
            Err(ShareError::Unreadable)
        );
        assert_eq!(
            with(&|v| v["kind"] = "spot".into()),
            Err(ShareError::Unreadable),
            "no spot payload"
        );
        assert_eq!(
            with(&|v| {
                v.as_object_mut().unwrap().remove("trip");
            }),
            Err(ShareError::Unreadable)
        );
        assert_eq!(
            with(&|v| v["trip"]["method"] = "swim".into()),
            Err(ShareError::Unreadable)
        );
        assert_eq!(
            with(&|v| v["trip"]["species"] = json!(["Striped Bass"])),
            Err(ShareError::Unreadable)
        );
        assert_eq!(
            with(&|v| v["trip"]["first"] = "2027-02-30".into()),
            Err(ShareError::Unreadable)
        );
        assert_eq!(
            with(&|v| v["trip"]["places"] = json!([])),
            Err(ShareError::Unreadable)
        );
        assert_eq!(
            with(&|v| v["trip"]["places"][0]["latitude"] = 123.0.into()),
            Err(ShareError::Unreadable)
        );
        assert_eq!(
            with(&|v| v["trip"]["places"][0]["time_zone"] = "Nowhere/Land".into()),
            Err(ShareError::Unreadable)
        );
    }

    #[test]
    fn input_over_a_megabyte_is_too_large() {
        let mut text = encode(&Document::Trip(trip())).unwrap();
        text.push_str(&" ".repeat(MAX_BYTES));
        assert_eq!(decode(text.as_bytes()), Err(ShareError::TooLarge));
        let mut t = trip();
        t.notes = "x".repeat(MAX_BYTES);
        assert_eq!(encode(&Document::Trip(t)), Err(ShareError::TooLarge));
        let at_limit = format!(
            "{}{}",
            encode(&Document::Trip(trip())).unwrap(),
            " ".repeat(MAX_BYTES - encode(&Document::Trip(trip())).unwrap().len())
        );
        assert_eq!(at_limit.len(), MAX_BYTES);
        assert!(decode(at_limit.as_bytes()).is_ok());
    }

    #[test]
    fn unknown_fields_in_a_known_version_are_ignored() {
        let mut v = encoded(&Document::Trip(trip()));
        v["future"] = json!({"a": 1});
        v["trip"]["rating"] = 5.into();
        v["trip"]["places"][0]["depth_m"] = 3.0.into();
        v["trip"]["limits"]["max_swell_m"] = 2.0.into();
        assert_eq!(decode(v.to_string().as_bytes()), Ok(Document::Trip(trip())));
    }

    #[test]
    fn payloads_that_do_not_parse_are_unreadable() {
        let text = r#"{"format":"underbite","v":1,"kind":"layers","layers":{"x":1}}"#;
        assert_eq!(decode(text.as_bytes()), Err(ShareError::Unreadable));
        let text = r#"{"format":"underbite","v":1,"kind":"layers"}"#;
        assert_eq!(decode(text.as_bytes()), Err(ShareError::Unreadable));
        let text = r#"{"format":"underbite","v":1,"kind":"profile","profile":{"x":1}}"#;
        assert_eq!(decode(text.as_bytes()), Err(ShareError::Unreadable));
    }

    #[test]
    fn rounded_fuzz_rounds_to_a_hundredth_of_a_degree() {
        let mut doc = Document::Trip(trip());
        fuzz(&mut doc, Fuzz::Rounded1Km);
        let Document::Trip(t) = &doc else { panic!() };
        let p = &t.places[0];
        assert_eq!(
            (p.latitude, p.longitude, p.fuzz),
            (Some(40.70), Some(-74.02), Fuzz::Rounded1Km)
        );
        assert_eq!(p.name, "The Battery");
        let before = doc.clone();
        fuzz(&mut doc, Fuzz::Rounded1Km);
        assert_eq!(doc, before, "idempotent");
        assert_eq!(decode(encode(&doc).unwrap().as_bytes()), Ok(doc));
    }

    #[test]
    fn fuzz_is_the_same_every_time() {
        let a = {
            let mut d = spot();
            fuzz(&mut d, Fuzz::Rounded1Km);
            d
        };
        let b = {
            let mut d = spot();
            fuzz(&mut d, Fuzz::Rounded1Km);
            d
        };
        assert_eq!(encode(&a), encode(&b));
        // The Croton and a buoy-side point round the same way every time too.
        let mut croton = battery();
        croton.latitude = Some(41.2087);
        croton.longitude = Some(-73.8586);
        fuzz_place(&mut croton, Fuzz::Rounded1Km);
        assert_eq!(
            (croton.latitude, croton.longitude),
            (Some(41.21), Some(-73.86))
        );
    }

    #[test]
    fn no_negative_zero_after_rounding() {
        assert!(hundredth(-0.001).is_sign_positive());
        assert!((hundredth(-0.004)).abs() < f64::EPSILON);
    }

    #[test]
    fn area_only_drops_coordinates_and_keeps_area_and_region() {
        let mut doc = Document::Trip(trip());
        fuzz(&mut doc, Fuzz::AreaOnly);
        let Document::Trip(t) = &doc else { panic!() };
        let p = &t.places[0];
        assert_eq!((p.latitude, p.longitude), (None, None));
        assert_eq!(p.fuzz, Fuzz::AreaOnly);
        assert_eq!(p.area.as_deref(), Some("Lower Hudson"));
        assert_eq!(p.region.as_deref(), Some("US-NY"));
        let v = encoded(&doc);
        assert!(v["trip"]["places"][0].get("latitude").is_none());
        assert!(v["trip"]["places"][0].get("longitude").is_none());
        assert_eq!(v["trip"]["places"][0]["fuzz"], "area_only");
        assert_eq!(decode(encode(&doc).unwrap().as_bytes()), Ok(doc));
    }

    #[test]
    fn fuzz_never_refines_and_exact_changes_nothing() {
        let mut doc = spot();
        fuzz(&mut doc, Fuzz::Exact);
        assert_eq!(doc, spot());
        fuzz(&mut doc, Fuzz::AreaOnly);
        let area = doc.clone();
        fuzz(&mut doc, Fuzz::Rounded1Km);
        assert_eq!(doc, area);
        fuzz(&mut doc, Fuzz::Exact);
        assert_eq!(doc, area);
    }

    #[test]
    fn fuzz_reaches_every_place_and_the_spot_payload() {
        let mut t = trip();
        let mut croton = battery();
        croton.spot_id = "spot-croton".into();
        croton.latitude = Some(41.2087);
        croton.longitude = Some(-73.8586);
        t.places.push(croton);
        let mut doc = Document::Trip(t);
        fuzz(&mut doc, Fuzz::Rounded1Km);
        let Document::Trip(t) = &doc else { panic!() };
        assert!(t.places.iter().all(|p| p.fuzz == Fuzz::Rounded1Km));
        let mut s = spot();
        fuzz(&mut s, Fuzz::AreaOnly);
        let Document::Spot(s) = &s else { panic!() };
        assert_eq!(s.place.fuzz, Fuzz::AreaOnly);
        let mut l = layers();
        fuzz(&mut l, Fuzz::AreaOnly);
        assert_eq!(l, layers(), "a preset has no places");
    }

    #[test]
    fn the_file_holds_no_display_strings_or_user_units() {
        let text = encode(&Document::Trip(trip())).unwrap();
        for word in ["°F", "mph", "feet", "api_key", "token"] {
            assert!(!text.contains(word), "{word}");
        }
    }

    #[test]
    fn the_wire_shape_and_schemas_are_stable() {
        /// The envelope as a type, for the schema: exactly one payload member matches `kind`.
        #[derive(JsonSchema)]
        #[allow(dead_code)]
        struct File {
            format: String,
            v: u32,
            kind: Kind,
            trip: Option<Trip>,
            spot: Option<SpotShare>,
            profile: Option<ProfileShare>,
            layers: Option<LayersShare>,
        }
        #[derive(JsonSchema)]
        #[serde(rename_all = "snake_case")]
        #[allow(dead_code)]
        enum Kind {
            Trip,
            Spot,
            Layers,
            Profile,
        }
        let mut t = trip();
        t.species
            .push(SpeciesId::new("pomatomus-saltatrix").unwrap());
        t.method = Method::Boat;
        let schemas = json!({
            "File": schemars::schema_for!(File),
            "Fuzz": schemars::schema_for!(Fuzz),
        });
        insta::assert_json_snapshot!("schema", schemas);
        insta::assert_snapshot!("trip_file", encode(&Document::Trip(t)).unwrap());
        insta::assert_snapshot!("spot_file", encode(&spot()).unwrap());
        insta::assert_snapshot!("profile_file", encode(&profile()).unwrap());
        insta::assert_snapshot!("layers_file", encode(&layers()).unwrap());
    }

    // -- map layer presets ------------------------------------------------------------------

    fn layer(id: &str, shown: bool) -> SharedLayer {
        SharedLayer {
            id: LayerId::new(id),
            shown,
            opacity: None,
            ramp: None,
            range: None,
        }
    }

    /// "Inshore flats" as a sender saved it: clarity at 70 % on a set range, then the stations.
    fn inshore_flats() -> LayersShare {
        LayersShare {
            name: "Inshore flats".into(),
            layers: vec![
                layer("radar", false),
                SharedLayer {
                    opacity: Some(0.7),
                    ramp: Some("turbidity".into()),
                    range: Some((0.05, 1.5)),
                    ..layer("clarity", true)
                },
                layer("tide-stations", true),
                layer("current-stations", true),
                layer("spots", true),
            ],
        }
    }

    fn layers() -> Document {
        Document::Layers {
            share: inshore_flats(),
            left_out: Vec::new(),
        }
    }

    #[test]
    fn a_layer_preset_round_trips_in_order_with_every_state() {
        let doc = layers();
        let text = encode(&doc).unwrap();
        assert_eq!(decode(text.as_bytes()), Ok(doc.clone()));
        assert_eq!(encode(&doc).unwrap(), text, "deterministic");
        let v = encoded(&doc);
        assert_eq!(v["kind"], "layers");
        assert_eq!(v["layers"]["name"], "Inshore flats");
        let ids: Vec<&str> = v["layers"]["layers"]
            .as_array()
            .unwrap()
            .iter()
            .map(|l| l["id"].as_str().unwrap())
            .collect();
        assert_eq!(
            ids,
            [
                "radar",
                "clarity",
                "tide-stations",
                "current-stations",
                "spots"
            ]
        );
        let clarity = &v["layers"]["layers"][1];
        assert_eq!(clarity["opacity"], json!(0.7));
        assert_eq!(clarity["ramp"], "turbidity");
        assert_eq!(clarity["range"], json!([0.05, 1.5]));
        // A layer's own opacity, ramp and range are left unsaid.
        assert_eq!(
            v["layers"]["layers"][0],
            json!({"id": "radar", "shown": false})
        );
    }

    #[test]
    fn a_layer_preset_holds_no_place_key_or_date() {
        let text = encode(&layers()).unwrap();
        let v: Value = serde_json::from_str(&text).unwrap();
        let mut keys: Vec<&str> = v["layers"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(keys, ["layers", "name"]);
        for word in [
            "latitude",
            "longitude",
            "api_key",
            "token",
            "date",
            "modified",
        ] {
            assert!(!text.contains(word), "{word}");
        }
    }

    #[test]
    fn layers_this_build_lacks_are_left_out_and_named() {
        let mut v = encoded(&layers());
        let list = v["layers"]["layers"].as_array_mut().unwrap();
        list.insert(
            1,
            json!({"id": "glow-worms", "shown": true, "opacity": 0.4}),
        );
        list.push(json!({"id": "apple-satellite", "shown": true}));
        list.push(json!({"id": "glow-worms", "shown": false}));
        list.push(json!({"id": "spots", "shown": false}));
        let Ok(Document::Layers { share, left_out }) = decode(v.to_string().as_bytes()) else {
            panic!()
        };
        // The base map is not an overlay; each id is named once, in file order.
        assert_eq!(
            left_out,
            [LayerId::new("glow-worms"), LayerId::new("apple-satellite")]
        );
        // The rest keep their order and states; a layer listed twice keeps its first state.
        assert_eq!(share, inshore_flats());
        let note = crate::view::strings::Strings::new(crate::view::Voice::Plain)
            .layers_left_out(&left_out)
            .unwrap();
        assert!(note.contains("glow-worms, apple-satellite"), "{note}");
    }

    #[test]
    fn a_newer_layer_preset_needs_a_newer_version() {
        let mut v = encoded(&layers());
        v["v"] = (VERSION + 1).into();
        v["layers"] = json!({"layout": "anything a later version writes"});
        assert_eq!(
            decode(v.to_string().as_bytes()),
            Err(ShareError::NeedsNewerVersion)
        );
    }

    #[test]
    fn unknown_fields_in_a_layer_preset_are_ignored() {
        let mut v = encoded(&layers());
        v["layers"]["colour"] = "teal".into();
        v["layers"]["layers"][1]["glow"] = 3.into();
        assert_eq!(decode(v.to_string().as_bytes()), Ok(layers()));
    }

    #[test]
    fn a_layer_preset_the_map_cannot_use_is_unreadable() {
        let good = encoded(&layers());
        let with = |f: &dyn Fn(&mut Value)| {
            let mut v = good.clone();
            f(&mut v);
            decode(v.to_string().as_bytes())
        };
        assert!(with(&|_| {}).is_ok());
        let bad: [&dyn Fn(&mut Value); 9] = [
            &|v| v["layers"]["name"] = "  ".into(),
            &|v| {
                v["layers"].as_object_mut().unwrap().remove("name");
            },
            &|v| v["layers"]["layers"][1]["opacity"] = 1.5.into(),
            &|v| v["layers"]["layers"][1]["opacity"] = (-0.1).into(),
            &|v| v["layers"]["layers"][1]["range"] = json!([2.0, 1.0]),
            &|v| v["layers"]["layers"][1]["range"] = json!([1.0]),
            &|v| v["layers"]["layers"][1]["id"] = "".into(),
            &|v| v["layers"]["layers"][1]["shown"] = "yes".into(),
            &|v| v["layers"]["layers"] = json!({"clarity": true}),
        ];
        for (i, f) in bad.into_iter().enumerate() {
            assert_eq!(with(f), Err(ShareError::Unreadable), "{i}");
        }
        // A preset that shows nothing is still a preset.
        assert!(with(&|v| v["layers"]["layers"] = json!([])).is_ok());
        let mut share = inshore_flats();
        share.layers[1].opacity = Some(f32::NAN);
        assert_eq!(
            encode(&Document::Layers {
                share,
                left_out: Vec::new()
            }),
            Err(ShareError::Unreadable)
        );
    }

    #[test]
    fn copernicus_layers_are_switched_off_and_kept() {
        let mut share = inshore_flats();
        share.layers.push(layer("copernicus-currents", true));
        share.layers.push(layer("copernicus-sst", false));
        let hidden = share.hide_copernicus();
        assert_eq!(hidden, [LayerId::new("copernicus-currents")]);
        assert_eq!(share.layers.len(), 7, "kept, hidden");
        assert!(
            share
                .layers
                .iter()
                .all(|l| !l.id.as_str().starts_with("copernicus-") || !l.shown)
        );
        assert!(share.layers[2].shown, "other layers are untouched");
        assert_eq!(share.hide_copernicus(), [], "nothing left to switch off");
    }

    // -- profiles ---------------------------------------------------------------------------

    fn catalogue() -> Catalogue {
        Catalogue::bundled().unwrap()
    }

    fn bass_id() -> SpeciesId {
        SpeciesId::new("micropterus-dolomieu").unwrap()
    }

    fn range(low: f32, high: f32) -> crate::species::profile::Range {
        crate::species::profile::Range { low, high }
    }

    /// Smallmouth bass as the sender has it: the user's lures and spawning range.
    fn sender_edits() -> ProfileEdits {
        let mut edits = ProfileEdits::default();
        let e = edits.edit(&bass_id());
        e.lures = Some(vec!["Tube jig".into()]);
        e.spawn_temp_c = Some(range(15.0, 18.0));
        edits
    }

    fn share() -> ProfileShare {
        ProfileShare::from_catalogue(&catalogue(), &bass_id(), &sender_edits()).unwrap()
    }

    fn profile() -> Document {
        Document::Profile(share())
    }

    #[test]
    fn a_profile_round_trips_with_its_sources_licences_and_edits() {
        let doc = profile();
        let text = encode(&doc).unwrap();
        assert_eq!(decode(text.as_bytes()), Ok(doc.clone()));
        let v = encoded(&doc);
        assert_eq!(v["kind"], "profile");
        let species = &v["profile"]["species"];
        assert_eq!(species["id"], "micropterus-dolomieu");
        // The FishBase attribution travels with each value.
        let tolerated = &species["tolerated_temp_c"]["source"];
        assert_eq!(tolerated["licence"], "cc_by_nc4");
        assert!(tolerated["citation"].as_str().unwrap().contains("FishBase"));
        assert!(tolerated["url"].as_str().unwrap().starts_with("https://"));
        assert!(tolerated["reference"].is_string());
        // The edits are kept apart from the catalogue's values.
        assert_eq!(v["profile"]["edits"]["lures"], json!(["Tube jig"]));
        assert!(!text.contains("user_edited"));
        let Document::Profile(back) = decode(text.as_bytes()).unwrap() else {
            panic!()
        };
        let shown = back.shown();
        assert_eq!(
            shown.spawn_temp_c.unwrap().source.licence,
            Licence::UserEdited
        );
        assert_eq!(
            shown.tolerated_temp_c.unwrap().source.licence,
            Licence::CcByNc4
        );
        // A profile has no places to fuzz.
        let mut fuzzed = doc.clone();
        fuzz(&mut fuzzed, Fuzz::AreaOnly);
        assert_eq!(fuzzed, doc);
    }

    #[test]
    fn a_profile_without_edits_has_no_edits_member() {
        let s = ProfileShare::from_catalogue(&catalogue(), &bass_id(), &ProfileEdits::default())
            .unwrap();
        let v = encoded(&Document::Profile(s));
        assert!(v["profile"].get("edits").is_none());
        assert!(
            ProfileShare::from_catalogue(
                &catalogue(),
                &SpeciesId::new("nemo-imaginarius").unwrap(),
                &ProfileEdits::default()
            )
            .is_none()
        );
    }

    #[test]
    fn a_profile_that_hides_or_drops_its_sources_is_refused() {
        let good = encoded(&profile());
        let with = |f: &dyn Fn(&mut Value)| {
            let mut v = good.clone();
            f(&mut v);
            decode(v.to_string().as_bytes())
        };
        assert!(with(&|_| {}).is_ok());
        // An edit posing as a source.
        assert_eq!(
            with(
                &|v| v["profile"]["species"]["tolerated_temp_c"]["source"]["licence"] =
                    "user_edited".into()
            ),
            Err(ShareError::Unreadable)
        );
        // A CC BY-NC value without its citation.
        assert_eq!(
            with(&|v| {
                v["profile"]["species"]["tolerated_temp_c"]["source"]
                    .as_object_mut()
                    .unwrap()
                    .remove("citation");
            }),
            Err(ShareError::Unreadable)
        );
        assert_eq!(
            with(&|v| v["profile"]["species"]["scientific_name"]["source"]["name"] = " ".into()),
            Err(ShareError::Unreadable)
        );
        // Values that cannot be right, in the profile or the edits.
        assert_eq!(
            with(&|v| v["profile"]["species"]["tolerated_temp_c"]["value"]["low"] = 40.0.into()),
            Err(ShareError::Unreadable)
        );
        assert_eq!(
            with(&|v| v["profile"]["edits"]["spawn_temp_c"] = json!({"low": 30.0, "high": 10.0})),
            Err(ShareError::Unreadable)
        );
        assert_eq!(
            with(&|v| v["profile"]["edits"]["lures"] = json!([])),
            Err(ShareError::Unreadable)
        );
        assert_eq!(
            with(&|v| v["profile"]["species"]["id"] = "Not An Id".into()),
            Err(ShareError::Unreadable)
        );
        let mut bad = share();
        bad.edits.lures = Some(vec![" ".into()]);
        assert_eq!(encode(&Document::Profile(bad)), Err(ShareError::Unreadable));
    }

    #[test]
    fn importing_adds_the_senders_edits_where_this_device_has_none() {
        let mut mine = ProfileEdits::default();
        let plan = share().import_into(&catalogue(), &mut mine, OnConflict::KeepMine);
        assert!(plan.known);
        assert_eq!(plan.added, vec![Field::SpawnTemp, Field::Lures]);
        assert_eq!(plan.conflicts, []);
        assert_eq!(mine, sender_edits());
        // Importing again changes nothing.
        let again = share().import_into(&catalogue(), &mut mine, OnConflict::TakeTheirs);
        assert_eq!(again.added, []);
        assert_eq!(again.same, vec![Field::SpawnTemp, Field::Lures]);
        assert_eq!(mine, sender_edits());
    }

    #[test]
    fn importing_never_overwrites_this_devices_edits_unless_asked() {
        let mut mine = ProfileEdits::default();
        mine.edit(&bass_id()).lures = Some(vec!["Ned rig".into()]);
        mine.edit(&bass_id()).regions = Some(vec!["Lower Hudson".into()]);
        let before = mine.clone();
        let plan = share().plan_import(&catalogue(), &mine);
        assert_eq!(plan.added, vec![Field::SpawnTemp]);
        assert_eq!(plan.conflicts, vec![Field::Lures]);
        assert_eq!(mine, before, "planning changes nothing");

        let kept = share().import_into(&catalogue(), &mut mine, OnConflict::KeepMine);
        assert_eq!(kept.conflicts, vec![Field::Lures]);
        let e = &mine.species[&bass_id()];
        assert_eq!(e.lures, Some(vec!["Ned rig".to_owned()]));
        assert_eq!(e.spawn_temp_c, Some(range(15.0, 18.0)));
        assert_eq!(e.regions, Some(vec!["Lower Hudson".to_owned()]));

        share().import_into(&catalogue(), &mut mine, OnConflict::TakeTheirs);
        let e = &mine.species[&bass_id()];
        assert_eq!(e.lures, Some(vec!["Tube jig".to_owned()]));
        assert_eq!(e.regions, Some(vec!["Lower Hudson".to_owned()]));
    }

    #[test]
    fn a_senders_value_equal_to_the_catalogue_is_not_an_edit_here() {
        let base = catalogue().get(&bass_id()).unwrap().clone();
        let mut theirs = ProfileEdits::default();
        theirs.edit(&bass_id()).tolerated_temp_c = base.tolerated_temp_c.map(|t| t.value);
        let s = ProfileShare::from_catalogue(&catalogue(), &bass_id(), &theirs).unwrap();
        let mut mine = ProfileEdits::default();
        let plan = s.import_into(&catalogue(), &mut mine, OnConflict::KeepMine);
        assert_eq!(plan.same, vec![Field::ToleratedTemp]);
        assert!(mine.species.is_empty());
    }

    #[test]
    fn a_species_this_build_lacks_is_not_imported() {
        let mut s = share();
        s.species.id = SpeciesId::new("nemo-imaginarius").unwrap();
        let mut mine = ProfileEdits::default();
        let plan = s.import_into(&catalogue(), &mut mine, OnConflict::TakeTheirs);
        assert!(!plan.known);
        assert_eq!(plan.added, []);
        assert!(mine.species.is_empty());
    }
}
