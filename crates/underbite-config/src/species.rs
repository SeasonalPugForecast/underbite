//! `[species]`: the species the user follows, from the bundled catalogue.

use std::sync::OnceLock;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use underbite_core::species::catalogue::Catalogue;
use underbite_core::species::profile::{ProfileEdits, SpeciesProfile};
use underbite_core::trip::SpeciesId;

use crate::schema::Issues;

/// `[species]`: the species the user follows. The user's edits of their profiles are not kept
/// here: the app stores them apart and they are laid over the catalogue's values when a view
/// is built.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct SpeciesConfig {
    /// Species ids from the bundled catalogue (`morone-saxatilis`), in the order the species
    /// card lists them. Ids the catalogue lacks are dropped on import with a warning.
    pub followed: Vec<SpeciesId>,
}

impl SpeciesConfig {
    /// The followed species' profiles in the user's order, `edits` laid over the catalogue's
    /// values. Ids the catalogue lacks are skipped.
    pub fn profiles(&self, edits: &ProfileEdits) -> Vec<SpeciesProfile> {
        let Some(catalogue) = catalogue() else {
            return Vec::new();
        };
        self.followed
            .iter()
            .filter_map(|id| catalogue.profile(id, edits))
            .collect()
    }

    /// Warnings for ids the catalogue lacks and ids listed twice.
    pub(crate) fn validate(&self, issues: &mut Issues) {
        for (i, id) in self.followed.iter().enumerate() {
            let path = format!("species.followed[{i}]");
            if !is_known_species(id.as_str()) {
                issues.warning(
                    path,
                    format!("`{}` is not a species this version knows", id.as_str()),
                );
            } else if self.followed[..i].contains(id) {
                issues.warning(path, "species listed twice");
            }
        }
    }
}

/// The bundled species catalogue, read once. `None` only if the bundled data does not load,
/// which the core's tests over that data rule out.
pub fn catalogue() -> Option<&'static Catalogue> {
    static CATALOGUE: OnceLock<Option<Catalogue>> = OnceLock::new();
    CATALOGUE.get_or_init(|| Catalogue::bundled().ok()).as_ref()
}

/// Whether `id` names a species of the bundled catalogue.
pub fn is_known_species(id: &str) -> bool {
    SpeciesId::new(id).is_ok_and(|id| catalogue().is_some_and(|c| c.get(&id).is_some()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(s: &str) -> SpeciesId {
        SpeciesId::new(s).unwrap()
    }

    #[test]
    fn species_default_to_none_and_parse_in_order() {
        assert_eq!(SpeciesConfig::default().followed, []);
        let s: SpeciesConfig =
            toml::from_str("followed = [\"salmo-trutta\", \"micropterus-dolomieu\"]").unwrap();
        assert_eq!(s.followed, [id("salmo-trutta"), id("micropterus-dolomieu")]);
        assert!(toml::from_str::<SpeciesConfig>("followed = [\"Brown Trout\"]").is_err());
        assert!(toml::from_str::<SpeciesConfig>("edits = {}").is_err());
    }

    #[test]
    fn followed_profiles_keep_the_order_and_take_the_edits() {
        let s = SpeciesConfig {
            followed: vec![
                id("salmo-trutta"),
                id("no-such-fish"),
                id("micropterus-dolomieu"),
            ],
        };
        let plain = s.profiles(&ProfileEdits::default());
        let ids: Vec<_> = plain.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["salmo-trutta", "micropterus-dolomieu"]);

        let mut edits = ProfileEdits::default();
        edits.edit(&id("salmo-trutta")).lures = Some(vec!["Woolly bugger".into()]);
        let edited = s.profiles(&edits);
        assert_eq!(
            edited[0].lures.as_ref().map(|l| l.value.clone()),
            Some(vec!["Woolly bugger".to_owned()])
        );
        assert_eq!(edited[1], plain[1]);
    }

    #[test]
    fn unknown_and_repeated_species_are_warnings() {
        let s = SpeciesConfig {
            followed: vec![id("salmo-trutta"), id("no-such-fish"), id("salmo-trutta")],
        };
        let mut issues = Issues::default();
        s.validate(&mut issues);
        let found: Vec<_> = issues
            .0
            .iter()
            .map(|i| (i.severity, i.path.as_str()))
            .collect();
        assert_eq!(
            found,
            [
                (crate::schema::Severity::Warning, "species.followed[1]"),
                (crate::schema::Severity::Warning, "species.followed[2]"),
            ]
        );
        assert!(is_known_species("morone-saxatilis"));
        assert!(!is_known_species("Morone saxatilis"));
        assert!(!is_known_species("no-such-fish"));
    }
}
