//! What a species prefers, with the source and licence of every value, and the user's own edits.
//!
//! A profile holds canonical units (°C, metres) and no display text. Every value is a
//! [`Sourced`] pair: the value and a [`SourceRef`] naming where it came from, under which licence
//! and, for `FishBase`, the record reference and the citation the licence asks for. The user's edits
//! are a separate [`ProfileEdits`] value, applied on top when a profile is read, so replacing the
//! bundled catalogue never loses them.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::trip::SpeciesId;

/// Under which terms a value may be used and shown.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Licence {
    /// Work of a government agency, free of copyright.
    PublicDomain,
    /// Creative Commons Attribution 4.0.
    CcBy4,
    /// Creative Commons Attribution-NonCommercial 4.0 (`FishBase`).
    CcByNc4,
    /// A published paper or page whose own terms apply; the value is a cited fact.
    Published,
    /// Typed in by the user.
    UserEdited,
}

/// Where a value came from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceRef {
    /// Who says so, for example "`FishBase`" or "NOAA Fisheries".
    pub name: String,
    /// A link back to the page or record. Required for CC BY-NC values (the licence asks for it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub licence: Licence,
    /// The record's own reference (author and year), which `FishBase` asks to be cited.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    /// The citation of the database or work as a whole.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub citation: Option<String>,
}

impl SourceRef {
    /// The source of a value the user typed.
    pub fn user() -> Self {
        Self {
            name: "Your edit".to_owned(),
            url: None,
            licence: Licence::UserEdited,
            reference: None,
            citation: None,
        }
    }
}

/// A value and its source.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Sourced<T> {
    pub value: T,
    pub source: SourceRef,
}

impl<T> Sourced<T> {
    fn user(value: T) -> Self {
        Self {
            value,
            source: SourceRef::user(),
        }
    }
}

/// What kind of water a species lives in.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum WaterKind {
    Fresh,
    Salt,
    /// Both, such as the striped bass or the steelhead.
    Both,
}

/// A closed range of a quantity, low to high, in canonical units.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Range {
    pub low: f32,
    pub high: f32,
}

impl Range {
    fn valid(self, min: f32, max: f32) -> bool {
        self.low.is_finite()
            && self.high.is_finite()
            && self.low <= self.high
            && self.low >= min
            && self.high <= max
    }
}

/// A season of the year in the northern hemisphere's calendar.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Season {
    Spring,
    Summer,
    Autumn,
    Winter,
}

/// Which tide or current a species is said to feed on.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum FlowPreference {
    Incoming,
    Outgoing,
    SlackHigh,
    SlackLow,
    /// Any moving water, as against slack.
    Moving,
    Slack,
}

/// The light a species is said to feed in.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum LightPreference {
    Dawn,
    Day,
    Dusk,
    Night,
    Overcast,
}

/// A species profile. Everything but the id and the scientific name is optional: a source may
/// know only some of it, and an empty field is absent, never guessed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SpeciesProfile {
    pub id: SpeciesId,
    pub scientific_name: Sourced<String>,
    /// Anglers' names, the first being the one shown ("Steelhead", "Rainbow trout").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub common_names: Option<Sourced<Vec<String>>>,
    /// Other names, each with its own source (`FishBase`'s English name is kept here).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<Sourced<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub water: Option<Sourced<WaterKind>>,
    /// Water temperature the fish favours, °C.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_temp_c: Option<Sourced<Range>>,
    /// Water temperature the fish tolerates, °C.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tolerated_temp_c: Option<Sourced<Range>>,
    /// Water temperature at which it spawns, °C.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spawn_temp_c: Option<Sourced<Range>>,
    /// Where in the water column it is found, metres, by season.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth_m: Option<Sourced<BTreeMap<Season, Range>>>,
    /// Habitat tags such as "rocky", "estuary" or "weed edge".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub habitat: Option<Sourced<Vec<String>>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flow: Option<Sourced<Vec<FlowPreference>>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub light: Option<Sourced<Vec<LightPreference>>>,
    /// Lures and baits, free text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lures: Option<Sourced<Vec<String>>>,
    /// Regions it is fished in, free text such as "Mid-Atlantic".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub regions: Option<Sourced<Vec<String>>>,
}

/// Which part of a profile.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Field {
    CommonNames,
    Water,
    PreferredTemp,
    ToleratedTemp,
    SpawnTemp,
    Depth,
    Habitat,
    Flow,
    Light,
    Lures,
    Regions,
}

/// Why a profile is not acceptable.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ProfileError {
    #[error("{id}: {field} has no source name")]
    NoSource { id: String, field: &'static str },
    #[error("{id}: {field} is empty")]
    Empty { id: String, field: &'static str },
    #[error("{id}: {field} is out of range")]
    BadRange { id: String, field: &'static str },
    #[error(
        "{id}: {field} is licensed CC BY-NC but lacks a link, a record reference or a citation"
    )]
    IncompleteNonCommercial { id: String, field: &'static str },
    #[error("{id}: {field} has licence {found:?}, which does not belong in {layer}")]
    WrongLicence {
        id: String,
        field: &'static str,
        found: Licence,
        layer: &'static str,
    },
}

/// Which family of data files a profile came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layer {
    /// Hand-curated values from agencies and published work (`data/species/*.json`).
    Agency,
    /// Values derived from `FishBase` (`data/species/fishbase/*.json`), all CC BY-NC.
    FishBase,
}

impl Layer {
    fn name(self) -> &'static str {
        match self {
            Layer::Agency => "the agency files",
            Layer::FishBase => "the FishBase files",
        }
    }
}

impl SpeciesProfile {
    /// A profile with a name and nothing else.
    pub fn new(id: SpeciesId, scientific_name: Sourced<String>) -> Self {
        Self {
            id,
            scientific_name,
            common_names: None,
            aliases: Vec::new(),
            water: None,
            preferred_temp_c: None,
            tolerated_temp_c: None,
            spawn_temp_c: None,
            depth_m: None,
            habitat: None,
            flow: None,
            light: None,
            lures: None,
            regions: None,
        }
    }

    /// Every source named in the profile, in field order, for the credits.
    pub fn sources(&self) -> Vec<&SourceRef> {
        let mut out = vec![&self.scientific_name.source];
        macro_rules! add {
            ($($f:ident),*) => { $( if let Some(s) = &self.$f { out.push(&s.source); } )* };
        }
        add!(
            common_names,
            water,
            preferred_temp_c,
            tolerated_temp_c,
            spawn_temp_c,
            depth_m,
            habitat,
            flow,
            light,
            lures,
            regions
        );
        out.extend(self.aliases.iter().map(|a| &a.source));
        out
    }

    /// The name to show: the first common name, else the first alias, else the scientific name.
    pub fn display_name(&self) -> &str {
        self.common_names
            .as_ref()
            .and_then(|c| c.value.first())
            .or_else(|| self.aliases.first().map(|a| &a.value))
            .unwrap_or(&self.scientific_name.value)
    }

    /// Whether `query` is one of the profile's names, ignoring case.
    pub fn has_name(&self, query: &str) -> bool {
        let q = query.trim().to_lowercase();
        self.scientific_name.value.to_lowercase() == q
            || self
                .common_names
                .iter()
                .flat_map(|c| &c.value)
                .any(|n| n.to_lowercase() == q)
            || self.aliases.iter().any(|a| a.value.to_lowercase() == q)
    }

    /// Checks that every value has a source and is sane, and that licences match the layer.
    ///
    /// # Errors
    /// The first problem found.
    #[allow(clippy::too_many_lines)]
    pub fn validate(&self, layer: Layer) -> Result<(), ProfileError> {
        let id = self.id.as_str();
        let check_source = |field: &'static str, s: &SourceRef| -> Result<(), ProfileError> {
            if s.name.trim().is_empty() {
                return Err(ProfileError::NoSource {
                    id: id.to_owned(),
                    field,
                });
            }
            let ok = match layer {
                Layer::FishBase => s.licence == Licence::CcByNc4,
                Layer::Agency => !matches!(s.licence, Licence::CcByNc4 | Licence::UserEdited),
            };
            if !ok {
                return Err(ProfileError::WrongLicence {
                    id: id.to_owned(),
                    field,
                    found: s.licence,
                    layer: layer.name(),
                });
            }
            if s.licence == Licence::CcByNc4
                && [&s.url, &s.reference, &s.citation]
                    .iter()
                    .any(|o| o.as_deref().is_none_or(|t| t.trim().is_empty()))
            {
                return Err(ProfileError::IncompleteNonCommercial {
                    id: id.to_owned(),
                    field,
                });
            }
            Ok(())
        };
        let empty = |field: &'static str| ProfileError::Empty {
            id: id.to_owned(),
            field,
        };
        let bad = |field: &'static str| ProfileError::BadRange {
            id: id.to_owned(),
            field,
        };
        check_source("scientific_name", &self.scientific_name.source)?;
        if self.scientific_name.value.trim().is_empty() {
            return Err(empty("scientific_name"));
        }
        if let Some(c) = &self.common_names {
            check_source("common_names", &c.source)?;
            if c.value.is_empty() || c.value.iter().any(|n| n.trim().is_empty()) {
                return Err(empty("common_names"));
            }
        }
        for a in &self.aliases {
            check_source("aliases", &a.source)?;
            if a.value.trim().is_empty() {
                return Err(empty("aliases"));
            }
        }
        if let Some(w) = &self.water {
            check_source("water", &w.source)?;
        }
        for (field, r, lo, hi) in [
            ("preferred_temp_c", &self.preferred_temp_c, -5.0, 45.0),
            ("tolerated_temp_c", &self.tolerated_temp_c, -5.0, 50.0),
            ("spawn_temp_c", &self.spawn_temp_c, -5.0, 45.0),
        ] {
            if let Some(r) = r {
                check_source(field, &r.source)?;
                if !r.value.valid(lo, hi) {
                    return Err(bad(field));
                }
            }
        }
        if let Some(d) = &self.depth_m {
            check_source("depth_m", &d.source)?;
            if d.value.is_empty() {
                return Err(empty("depth_m"));
            }
            if d.value.values().any(|r| !r.valid(0.0, 12_000.0)) {
                return Err(bad("depth_m"));
            }
        }
        for (field, v) in [
            ("habitat", &self.habitat),
            ("lures", &self.lures),
            ("regions", &self.regions),
        ] {
            if let Some(v) = v {
                check_source(field, &v.source)?;
                if v.value.is_empty() || v.value.iter().any(|t| t.trim().is_empty()) {
                    return Err(empty(field));
                }
            }
        }
        if let Some(f) = &self.flow {
            check_source("flow", &f.source)?;
            if f.value.is_empty() {
                return Err(empty("flow"));
            }
        }
        if let Some(l) = &self.light {
            check_source("light", &l.source)?;
            if l.value.is_empty() {
                return Err(empty("light"));
            }
        }
        Ok(())
    }

    /// This profile with the user's edits laid over it; an edited field is `UserEdited`.
    #[must_use]
    pub fn with_edits(&self, edits: &SpeciesEdit) -> Self {
        let mut p = self.clone();
        macro_rules! over {
            ($($f:ident),*) => {
                $( if let Some(v) = &edits.$f { p.$f = Some(Sourced::user(v.clone())); } )*
            };
        }
        over!(
            common_names,
            water,
            preferred_temp_c,
            tolerated_temp_c,
            spawn_temp_c,
            depth_m,
            habitat,
            flow,
            light,
            lures,
            regions
        );
        p
    }
}

/// The user's overrides for one species. A field that is `None` is not overridden.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SpeciesEdit {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub common_names: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub water: Option<WaterKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_temp_c: Option<Range>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tolerated_temp_c: Option<Range>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spawn_temp_c: Option<Range>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth_m: Option<BTreeMap<Season, Range>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub habitat: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flow: Option<Vec<FlowPreference>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub light: Option<Vec<LightPreference>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lures: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub regions: Option<Vec<String>>,
}

impl SpeciesEdit {
    /// No field is overridden.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Drops the override of one field, so the catalogue's value shows again.
    pub fn reset(&mut self, field: Field) {
        match field {
            Field::CommonNames => self.common_names = None,
            Field::Water => self.water = None,
            Field::PreferredTemp => self.preferred_temp_c = None,
            Field::ToleratedTemp => self.tolerated_temp_c = None,
            Field::SpawnTemp => self.spawn_temp_c = None,
            Field::Depth => self.depth_m = None,
            Field::Habitat => self.habitat = None,
            Field::Flow => self.flow = None,
            Field::Light => self.light = None,
            Field::Lures => self.lures = None,
            Field::Regions => self.regions = None,
        }
    }

    /// The fields that are overridden.
    pub fn fields(&self) -> Vec<Field> {
        [
            (self.common_names.is_some(), Field::CommonNames),
            (self.water.is_some(), Field::Water),
            (self.preferred_temp_c.is_some(), Field::PreferredTemp),
            (self.tolerated_temp_c.is_some(), Field::ToleratedTemp),
            (self.spawn_temp_c.is_some(), Field::SpawnTemp),
            (self.depth_m.is_some(), Field::Depth),
            (self.habitat.is_some(), Field::Habitat),
            (self.flow.is_some(), Field::Flow),
            (self.light.is_some(), Field::Light),
            (self.lures.is_some(), Field::Lures),
            (self.regions.is_some(), Field::Regions),
        ]
        .into_iter()
        .filter(|(on, _)| *on)
        .map(|(_, f)| f)
        .collect()
    }
}

/// The user's edits for every species, kept apart from the catalogue and stored by the host.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfileEdits {
    #[serde(default)]
    pub species: BTreeMap<SpeciesId, SpeciesEdit>,
}

impl ProfileEdits {
    /// The edit of one species, created empty when there is none.
    pub fn edit(&mut self, id: &SpeciesId) -> &mut SpeciesEdit {
        self.species.entry(id.clone()).or_default()
    }

    /// Drops one field's override, and the species' entry once nothing is left.
    pub fn reset_field(&mut self, id: &SpeciesId, field: Field) {
        if let Some(e) = self.species.get_mut(id) {
            e.reset(field);
            if e.is_empty() {
                self.species.remove(id);
            }
        }
    }

    /// Drops every override of a species.
    pub fn reset(&mut self, id: &SpeciesId) {
        self.species.remove(id);
    }

    /// Parses the stored form.
    ///
    /// # Errors
    /// When the text is not valid edits JSON.
    pub fn from_json(text: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(text)
    }

    /// The stored form.
    ///
    /// # Errors
    /// Only if serialisation fails, which it cannot for these types.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn src(licence: Licence) -> SourceRef {
        SourceRef {
            name: "Synthetic test source".into(),
            url: Some("https://example.invalid/species".into()),
            licence,
            reference: (licence == Licence::CcByNc4).then(|| "Synthetic Author, 2000".into()),
            citation: (licence == Licence::CcByNc4).then(|| "Synthetic citation".into()),
        }
    }

    fn sourced<T>(value: T, licence: Licence) -> Sourced<T> {
        Sourced {
            value,
            source: src(licence),
        }
    }

    fn id(s: &str) -> SpeciesId {
        SpeciesId::new(s).unwrap()
    }

    fn range(low: f32, high: f32) -> Range {
        Range { low, high }
    }

    fn base() -> SpeciesProfile {
        let mut p = SpeciesProfile::new(
            id("morone-saxatilis"),
            sourced("Morone saxatilis".into(), Licence::Published),
        );
        p.common_names = Some(sourced(vec!["Striped bass".into()], Licence::Published));
        p.preferred_temp_c = Some(sourced(range(15.0, 20.0), Licence::Published));
        p
    }

    #[test]
    fn valid_profile_passes_and_lists_sources() {
        let p = base();
        p.validate(Layer::Agency).unwrap();
        assert_eq!(p.sources().len(), 3);
        assert_eq!(p.display_name(), "Striped bass");
        assert!(p.has_name("MORONE SAXATILIS"));
        assert!(!p.has_name("bluefish"));
    }

    #[test]
    fn a_value_without_a_source_name_is_refused() {
        let mut p = base();
        p.preferred_temp_c.as_mut().unwrap().source.name = " ".into();
        assert_eq!(
            p.validate(Layer::Agency),
            Err(ProfileError::NoSource {
                id: "morone-saxatilis".into(),
                field: "preferred_temp_c"
            })
        );
    }

    #[test]
    fn bad_ranges_and_empty_lists_are_refused() {
        let mut p = base();
        p.preferred_temp_c = Some(sourced(range(20.0, 15.0), Licence::Published));
        assert!(matches!(
            p.validate(Layer::Agency),
            Err(ProfileError::BadRange { .. })
        ));
        let mut p = base();
        p.lures = Some(sourced(vec![], Licence::Published));
        assert!(matches!(
            p.validate(Layer::Agency),
            Err(ProfileError::Empty { .. })
        ));
        let mut p = base();
        p.depth_m = Some(sourced(
            BTreeMap::from([(Season::Summer, range(-1.0, 3.0))]),
            Licence::Published,
        ));
        assert!(matches!(
            p.validate(Layer::Agency),
            Err(ProfileError::BadRange { .. })
        ));
    }

    #[test]
    fn licences_must_match_the_layer() {
        let p = base();
        assert!(matches!(
            p.validate(Layer::FishBase),
            Err(ProfileError::WrongLicence { .. })
        ));
        let mut q = base();
        q.scientific_name = sourced("Morone saxatilis".into(), Licence::CcByNc4);
        assert!(matches!(
            q.validate(Layer::Agency),
            Err(ProfileError::WrongLicence { .. })
        ));
    }

    #[test]
    fn non_commercial_values_need_link_reference_and_citation() {
        let mut p = SpeciesProfile::new(
            id("morone-saxatilis"),
            sourced("Morone saxatilis".into(), Licence::CcByNc4),
        );
        p.validate(Layer::FishBase).unwrap();
        p.scientific_name.source.reference = None;
        assert!(matches!(
            p.validate(Layer::FishBase),
            Err(ProfileError::IncompleteNonCommercial { .. })
        ));
    }

    #[test]
    fn edits_override_with_the_user_source_and_reset() {
        let p = base();
        let mut edits = ProfileEdits::default();
        edits.edit(&p.id).preferred_temp_c = Some(range(12.0, 18.0));
        edits.edit(&p.id).lures = Some(vec!["Bucktail".into()]);
        let e = p.with_edits(&edits.species[&p.id]);
        let t = e.preferred_temp_c.as_ref().unwrap();
        assert_eq!(t.value, range(12.0, 18.0));
        assert_eq!(t.source.licence, Licence::UserEdited);
        assert_eq!(
            e.lures.as_ref().unwrap().source.licence,
            Licence::UserEdited
        );
        assert_eq!(e.common_names, p.common_names);
        assert_eq!(
            edits.species[&p.id].fields(),
            vec![Field::PreferredTemp, Field::Lures]
        );

        edits.reset_field(&p.id, Field::PreferredTemp);
        assert_eq!(
            p.with_edits(&edits.species[&p.id]).preferred_temp_c,
            p.preferred_temp_c
        );
        edits.reset_field(&p.id, Field::Lures);
        assert!(edits.species.is_empty());
    }

    #[test]
    fn edits_round_trip_as_json() {
        let mut edits = ProfileEdits::default();
        edits.edit(&id("morone-saxatilis")).water = Some(WaterKind::Both);
        let text = edits.to_json().unwrap();
        assert_eq!(ProfileEdits::from_json(&text).unwrap(), edits);
        assert!(ProfileEdits::from_json(r#"{"species":{"Bad Id":{}}}"#).is_err());
    }

    #[test]
    fn a_profile_with_a_stray_field_is_refused() {
        let text = r#"{"id":"x","scientific_name":{"value":"X y","source":{"name":"n","licence":"published"}},"colour":"red"}"#;
        assert!(serde_json::from_str::<SpeciesProfile>(text).is_err());
    }
}
