//! The bundled species catalogue: hand-curated agency values and `FishBase`-derived values, merged.
//!
//! Two families of files are embedded at build time (a missing file fails the build):
//! `data/species/*.json` (agency and published values) and `data/species/fishbase/*.json` (CC BY-NC
//! values, each with its record reference). When both know a field the agency value wins; `FishBase`
//! names are kept as aliases beside the anglers' names. The user's edits are not part of the
//! catalogue: [`Catalogue::profile`] lays a [`ProfileEdits`] over a profile when asked.
//!
//! A data file is `{"species": [ <SpeciesProfile>, ... ]}`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::profile::{Layer, ProfileEdits, ProfileError, Sourced, SpeciesProfile};
use crate::trip::SpeciesId;

/// The embedded agency files, `(file name, contents)`.
const AGENCY_FILES: &[(&str, &str)] = &[
    ("salt.json", include_str!("../../data/species/salt.json")),
    ("fresh.json", include_str!("../../data/species/fresh.json")),
];

/// The embedded `FishBase` files, `(file name, contents)`.
const FISHBASE_FILES: &[(&str, &str)] = &[
    (
        "fishbase/salt.json",
        include_str!("../../data/species/fishbase/salt.json"),
    ),
    (
        "fishbase/fresh.json",
        include_str!("../../data/species/fishbase/fresh.json"),
    ),
];

/// One data file.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileFile {
    pub species: Vec<SpeciesProfile>,
}

/// Why the catalogue could not be built.
#[derive(Debug, thiserror::Error)]
pub enum CatalogueError {
    #[error("{file}: {source}")]
    Parse {
        file: String,
        source: serde_json::Error,
    },
    #[error("{file}: {source}")]
    Invalid { file: String, source: ProfileError },
    #[error("{file}: species {id} appears twice in the same layer")]
    Duplicate { file: String, id: String },
}

/// The merged catalogue.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Catalogue {
    species: BTreeMap<SpeciesId, SpeciesProfile>,
}

impl Catalogue {
    /// The catalogue embedded in the app.
    ///
    /// # Errors
    /// A bundled file that does not parse or validate; a test over the real data keeps this from
    /// happening in a release.
    pub fn bundled() -> Result<Self, CatalogueError> {
        Self::from_files(AGENCY_FILES, FISHBASE_FILES)
    }

    /// Builds a catalogue from `(name, json)` pairs of agency files and `FishBase` files.
    ///
    /// # Errors
    /// A file that does not parse, a profile that fails [`SpeciesProfile::validate`] for its
    /// layer, or a species listed twice in one layer.
    pub fn from_files(
        agency: &[(&str, &str)],
        fishbase: &[(&str, &str)],
    ) -> Result<Self, CatalogueError> {
        let agency = load_layer(agency, Layer::Agency)?;
        let fishbase = load_layer(fishbase, Layer::FishBase)?;
        let mut species = BTreeMap::new();
        let ids: std::collections::BTreeSet<_> = agency.keys().chain(fishbase.keys()).collect();
        for id in ids {
            let merged = match (agency.get(id), fishbase.get(id)) {
                (Some(a), Some(f)) => merge(a.clone(), f.clone()),
                (Some(p), None) | (None, Some(p)) => p.clone(),
                (None, None) => continue,
            };
            species.insert(id.clone(), merged);
        }
        Ok(Self { species })
    }

    /// The profiles, in id order.
    pub fn iter(&self) -> impl Iterator<Item = &SpeciesProfile> {
        self.species.values()
    }

    pub fn len(&self) -> usize {
        self.species.len()
    }

    pub fn is_empty(&self) -> bool {
        self.species.is_empty()
    }

    /// The catalogue's profile, without the user's edits.
    pub fn get(&self, id: &SpeciesId) -> Option<&SpeciesProfile> {
        self.species.get(id)
    }

    /// The profile with the user's edits laid over it.
    pub fn profile(&self, id: &SpeciesId, edits: &ProfileEdits) -> Option<SpeciesProfile> {
        let p = self.species.get(id)?;
        Some(match edits.species.get(id) {
            Some(e) => p.with_edits(e),
            None => p.clone(),
        })
    }

    /// Profiles with a scientific name, common name or alias equal to `query`, ignoring case.
    pub fn find(&self, query: &str) -> Vec<&SpeciesProfile> {
        self.species
            .values()
            .filter(|p| p.has_name(query))
            .collect()
    }
}

fn load_layer(
    files: &[(&str, &str)],
    layer: Layer,
) -> Result<BTreeMap<SpeciesId, SpeciesProfile>, CatalogueError> {
    let mut out = BTreeMap::new();
    for (name, text) in files {
        let file: ProfileFile =
            serde_json::from_str(text).map_err(|source| CatalogueError::Parse {
                file: (*name).to_owned(),
                source,
            })?;
        for mut p in file.species {
            if layer == Layer::FishBase {
                // The catalogue shows anglers' names; FishBase's are aliases.
                if let Some(c) = p.common_names.take() {
                    p.aliases.extend(c.value.into_iter().map(|value| Sourced {
                        value,
                        source: c.source.clone(),
                    }));
                }
            }
            p.validate(layer)
                .map_err(|source| CatalogueError::Invalid {
                    file: (*name).to_owned(),
                    source,
                })?;
            let id = p.id.clone();
            if out.insert(id.clone(), p).is_some() {
                return Err(CatalogueError::Duplicate {
                    file: (*name).to_owned(),
                    id: id.as_str().to_owned(),
                });
            }
        }
    }
    Ok(out)
}

/// The agency profile wins field by field; `FishBase` fills what it lacks, and its aliases join.
fn merge(a: SpeciesProfile, f: SpeciesProfile) -> SpeciesProfile {
    let mut aliases = a.aliases;
    for alias in f.aliases {
        let dup = aliases
            .iter()
            .any(|x| x.value.eq_ignore_ascii_case(&alias.value));
        if !dup {
            aliases.push(alias);
        }
    }
    SpeciesProfile {
        id: a.id,
        scientific_name: a.scientific_name,
        common_names: a.common_names,
        aliases,
        water: a.water.or(f.water),
        preferred_temp_c: a.preferred_temp_c.or(f.preferred_temp_c),
        tolerated_temp_c: a.tolerated_temp_c.or(f.tolerated_temp_c),
        spawn_temp_c: a.spawn_temp_c.or(f.spawn_temp_c),
        depth_m: a.depth_m.or(f.depth_m),
        habitat: a.habitat.or(f.habitat),
        flow: a.flow.or(f.flow),
        light: a.light.or(f.light),
        lures: a.lures.or(f.lures),
        regions: a.regions.or(f.regions),
    }
}

#[cfg(test)]
mod tests {
    use super::super::profile::{Licence, Range, WaterKind};
    use super::*;

    const AGENCY: &str = include_str!("../../data/species/synthetic/agency.synthetic.json");
    const FISHBASE: &str = include_str!("../../data/species/synthetic/fishbase.synthetic.json");

    fn id(s: &str) -> SpeciesId {
        SpeciesId::new(s).unwrap()
    }

    fn synthetic() -> Catalogue {
        Catalogue::from_files(
            &[("agency.synthetic.json", AGENCY)],
            &[("fishbase/fishbase.synthetic.json", FISHBASE)],
        )
        .unwrap()
    }

    #[test]
    fn the_bundled_catalogue_builds() {
        // The data files exist at build time; this checks they parse and validate.
        let c = Catalogue::bundled().unwrap();
        for p in c.iter() {
            assert_ne!(p.sources().len(), 0);
        }
    }

    #[test]
    fn two_species_merge_from_both_layers() {
        let c = synthetic();
        assert_eq!(c.len(), 2);
        assert!(c.get(&id("morone-saxatilis")).is_some());
        assert!(c.get(&id("oncorhynchus-mykiss")).is_some());
    }

    #[test]
    fn every_present_field_has_a_source() {
        for p in synthetic().iter() {
            assert!(p.sources().len() >= 3, "{}", p.id.as_str());
            for s in p.sources() {
                assert!(!s.name.is_empty(), "{}", p.id.as_str());
            }
        }
    }

    #[test]
    fn values_from_the_fishbase_files_carry_cc_by_nc_and_their_reference() {
        let c = synthetic();
        let p = c.get(&id("morone-saxatilis")).unwrap();
        let spawn = p.spawn_temp_c.as_ref().unwrap();
        assert_eq!(spawn.source.licence, Licence::CcByNc4);
        assert!(spawn.source.reference.is_some());
        assert!(spawn.source.url.as_deref().unwrap().starts_with("https://"));
        assert!(spawn.source.citation.is_some());
        let alias = p
            .aliases
            .iter()
            .find(|a| a.value == "Synthetic striper")
            .unwrap();
        assert_eq!(alias.source.licence, Licence::CcByNc4);

        // Every FishBase-only species' values are CC BY-NC too.
        let q = c.get(&id("oncorhynchus-mykiss")).unwrap();
        assert_eq!(
            q.tolerated_temp_c.as_ref().unwrap().source.licence,
            Licence::CcByNc4
        );
    }

    #[test]
    fn the_agency_value_beats_fishbase_and_fishbase_fills_the_rest() {
        let c = synthetic();
        let p = c.get(&id("morone-saxatilis")).unwrap();
        // Both files give a preferred temperature; the agency's wins.
        let pref = p.preferred_temp_c.as_ref().unwrap();
        assert_eq!(
            pref.value,
            Range {
                low: 15.0,
                high: 20.0
            }
        );
        assert_eq!(pref.source.licence, Licence::Published);
        // Water is only in the FishBase file here.
        assert_eq!(p.water.as_ref().unwrap().value, WaterKind::Both);
        assert_eq!(p.water.as_ref().unwrap().source.licence, Licence::CcByNc4);
        // The anglers' name is the agency's; FishBase's name is an alias.
        assert_eq!(p.display_name(), "Striped bass");
        assert!(p.has_name("synthetic striper"));
    }

    #[test]
    fn steelhead_and_rainbow_trout_are_one_species() {
        let c = synthetic();
        let p = c.get(&id("oncorhynchus-mykiss")).unwrap();
        assert_eq!(p.scientific_name.value, "Oncorhynchus mykiss");
        assert_eq!(c.find("steelhead").len(), 1);
        assert_eq!(c.find("Rainbow Trout")[0].id, p.id);
        assert_eq!(c.find("synthetic redband")[0].id, p.id);
    }

    #[test]
    fn user_edits_override_and_survive_a_catalogue_change() {
        let c = synthetic();
        let sb = id("morone-saxatilis");
        let mut edits = ProfileEdits::default();
        edits.edit(&sb).preferred_temp_c = Some(Range {
            low: 10.0,
            high: 14.0,
        });
        let p = c.profile(&sb, &edits).unwrap();
        assert_eq!(
            p.preferred_temp_c.unwrap().source.licence,
            Licence::UserEdited
        );
        assert_eq!(
            c.get(&sb)
                .unwrap()
                .preferred_temp_c
                .as_ref()
                .unwrap()
                .value
                .low,
            15.0
        );

        // A different catalogue (only the agency file) still applies the same stored edits.
        let smaller = Catalogue::from_files(&[("a.json", AGENCY)], &[]).unwrap();
        let q = smaller.profile(&sb, &edits).unwrap();
        assert_eq!(
            q.preferred_temp_c.unwrap().value,
            Range {
                low: 10.0,
                high: 14.0
            }
        );
        assert!(smaller.profile(&id("salmo-salar"), &edits).is_none());
    }

    #[test]
    fn a_fishbase_value_without_the_notice_is_refused() {
        let bad = FISHBASE.replace("\"licence\": \"cc_by_nc4\"", "\"licence\": \"published\"");
        let r = Catalogue::from_files(&[], &[("bad.json", &bad)]);
        assert!(matches!(
            r,
            Err(CatalogueError::Invalid {
                source: ProfileError::WrongLicence { .. },
                ..
            })
        ));
    }

    #[test]
    fn a_cc_by_nc_value_in_an_agency_file_is_refused() {
        let r = Catalogue::from_files(&[("bad.json", FISHBASE)], &[]);
        assert!(matches!(r, Err(CatalogueError::Invalid { .. })));
    }

    #[test]
    fn duplicates_and_bad_json_are_errors() {
        let dup = Catalogue::from_files(&[("a.json", AGENCY), ("b.json", AGENCY)], &[]);
        assert!(matches!(dup, Err(CatalogueError::Duplicate { .. })));
        let bad = Catalogue::from_files(&[("a.json", "{")], &[]);
        assert!(matches!(bad, Err(CatalogueError::Parse { .. })));
    }
}
