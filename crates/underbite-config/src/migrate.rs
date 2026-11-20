//! Upgrades from older configuration versions, and the checks every import goes through.
//!
//! Imported text is first decoded into a generic value. Everything that needs the document's
//! shape (refusing anything key-like, upgrading older versions, naming unknown keys by their full
//! path, clamping the coordinate precision, dropping unknown source, map layer and species ids
//! and alert rules of unknown kinds) happens on that value, before the typed decode, so a file
//! from an older version always decodes and every change made on the way is reported.

use serde_json::{Map, Value};

use crate::map::{KNOWN_BASES, KNOWN_LAYERS};
use crate::schema::{CURRENT_VERSION, Issue, Issues, json_schema};
use crate::sources::{KNOWN_SOURCES, MAX_COORDINATE_PRECISION};
use crate::species::is_known_species;

/// Importing a configuration file failed.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum ImportError {
    /// Not valid TOML or JSON, or not a document at all.
    #[error("not a valid configuration file: {0}")]
    Syntax(String),
    /// The file holds something that looks like a secret; secrets never travel in files.
    #[error(
        "`{path}` looks like a key or password; remove it and keep keys in the keychain or the environment"
    )]
    KeyLike { path: String },
    /// The file was written by a newer version.
    #[error(
        "this file was made by a newer Underbite (configuration version {found}; this one reads up to {supported})"
    )]
    Newer { found: u64, supported: u32 },
    /// The file has a key this version does not have.
    #[error("unknown key `{path}`")]
    UnknownKey { path: String },
    /// A value has the wrong type or is out of range.
    #[error("{0}")]
    Invalid(String),
}

/// One upgrade step, from version `n` (its index in the list) to `n + 1`.
pub type Step = fn(&mut Map<String, Value>);

/// The upgrades from every older version. Version 1 is the first real one; a version-0 file only
/// needs its version number raised.
pub const STEPS: [Step; 1] = [|_| {}];

/// Raises `doc` to [`CURRENT_VERSION`] with the built-in steps.
pub fn migrate(doc: &mut Map<String, Value>) -> Result<(), ImportError> {
    migrate_with(doc, &STEPS, CURRENT_VERSION)
}

/// Raises `doc` to `current` by running `steps` one version at a time. A missing `version` means 1.
pub fn migrate_with(
    doc: &mut Map<String, Value>,
    steps: &[Step],
    current: u32,
) -> Result<(), ImportError> {
    let found = match doc.get("version") {
        None => 1,
        Some(v) => v
            .as_u64()
            .ok_or_else(|| ImportError::Invalid("`version` must be a whole number".to_owned()))?,
    };
    if found > u64::from(current) {
        return Err(ImportError::Newer {
            found,
            supported: current,
        });
    }
    // INVARIANT: found <= current, which is a u32.
    let mut version = u32::try_from(found).unwrap_or(current);
    while version < current {
        if let Some(step) = steps.get(version as usize) {
            step(doc);
        }
        version += 1;
    }
    doc.insert("version".to_owned(), Value::from(current));
    Ok(())
}

/// Whether a key's name suggests a secret.
fn is_key_like(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n == "key"
        || n.ends_with("_key")
        || n.ends_with("-key")
        || n.contains("apikey")
        || n.contains("token")
        || n.contains("password")
        || n.contains("secret")
}

fn join(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_owned()
    } else {
        format!("{path}.{key}")
    }
}

/// The path of the first key-like key anywhere in `value`.
pub fn find_key_like(value: &Value) -> Option<String> {
    fn walk(value: &Value, path: &str) -> Option<String> {
        match value {
            Value::Object(map) => map.iter().find_map(|(k, v)| {
                let at = join(path, k);
                if is_key_like(k) {
                    Some(at)
                } else {
                    walk(v, &at)
                }
            }),
            Value::Array(items) => items
                .iter()
                .enumerate()
                .find_map(|(i, v)| walk(v, &format!("{path}[{i}]"))),
            _ => None,
        }
    }
    walk(value, "")
}

/// The path of the first key in `doc` that the schema does not allow.
fn find_unknown_key(doc: &Value) -> Option<String> {
    let schema = json_schema();
    check(doc, &schema, &schema, "")
}

fn resolve<'a>(schema: &'a Value, root: &'a Value) -> &'a Value {
    schema
        .get("$ref")
        .and_then(Value::as_str)
        .and_then(|r| r.strip_prefix('#'))
        .and_then(|p| root.pointer(p))
        .unwrap_or(schema)
}

fn describes_object(schema: &Value) -> bool {
    schema.get("properties").is_some() || schema.get("additionalProperties").is_some()
}

fn describes_array(schema: &Value) -> bool {
    schema.get("items").is_some()
}

/// Whether `value` carries the tag a variant of an internally tagged enum fixes with `const`
/// (`kind = "spot"`).
fn matches_tag(value: &Value, schema: &Value) -> bool {
    schema
        .get("properties")
        .and_then(Value::as_object)
        .is_some_and(|props| {
            props.iter().any(|(k, s)| {
                s.get("const")
                    .is_some_and(|c| value.get(k).is_some_and(|v| v == c))
            })
        })
}

fn check(value: &Value, schema: &Value, root: &Value, path: &str) -> Option<String> {
    let schema = resolve(schema, root);
    for alts in ["anyOf", "oneOf"] {
        if let Some(list) = schema.get(alts).and_then(Value::as_array) {
            let fits: Vec<&Value> = list
                .iter()
                .map(|s| resolve(s, root))
                .filter(|s| match value {
                    Value::Object(_) => describes_object(s),
                    Value::Array(_) => describes_array(s),
                    _ => false,
                })
                .collect();
            // Several shapes fit: a tagged variant is picked by its tag, else nothing is judged.
            let tagged: Vec<&Value> = if fits.len() > 1 {
                fits.iter()
                    .copied()
                    .filter(|s| matches_tag(value, s))
                    .collect()
            } else {
                fits
            };
            return match tagged.as_slice() {
                [only] => check(value, only, root, path),
                _ => None,
            };
        }
    }
    match value {
        Value::Object(map) => {
            let props = schema.get("properties").and_then(Value::as_object);
            let extra = schema.get("additionalProperties");
            for (k, v) in map {
                let at = join(path, k);
                let found = match (props.and_then(|p| p.get(k)), extra) {
                    (Some(s), _) => check(v, s, root, &at),
                    (None, Some(Value::Bool(false))) => Some(at),
                    (None, Some(s)) if s.is_object() => check(v, s, root, &at),
                    _ => None,
                };
                if found.is_some() {
                    return found;
                }
            }
            None
        }
        Value::Array(items) => {
            let s = schema.get("items")?;
            items
                .iter()
                .enumerate()
                .find_map(|(i, v)| check(v, s, root, &format!("{path}[{i}]")))
        }
        _ => None,
    }
}

/// Clamps the coordinate precision and drops source ids this version does not know, recording a
/// warning for each.
fn tidy_sources(doc: &mut Value, issues: &mut Issues) {
    let Some(sources) = doc.get_mut("sources").and_then(Value::as_object_mut) else {
        return;
    };
    if let Some(p) = sources.get_mut("coordinate_precision")
        && let Some(n) = p.as_u64()
        && n > u64::from(MAX_COORDINATE_PRECISION)
    {
        issues.warning(
            "sources.coordinate_precision",
            format!("{n} decimals is too precise to send; using {MAX_COORDINATE_PRECISION}"),
        );
        *p = Value::from(MAX_COORDINATE_PRECISION);
    }
    for (group, body) in sources.iter_mut() {
        let Some(chain) = body.get_mut("chain").and_then(Value::as_array_mut) else {
            continue;
        };
        let mut kept = Vec::with_capacity(chain.len());
        for (i, id) in chain.drain(..).enumerate() {
            match id.as_str() {
                Some(s) if !KNOWN_SOURCES.contains(&s) => issues.warning(
                    format!("sources.{group}.chain[{i}]"),
                    format!("`{s}` is not a known source and was dropped"),
                ),
                _ => kept.push(id),
            }
        }
        *chain = kept;
    }
}

/// The map section's `preset` key was renamed `active_preset`. Runs before the unknown-key check.
fn rename_map_keys(doc: &mut Value) {
    let Some(map) = doc.get_mut("map").and_then(Value::as_object_mut) else {
        return;
    };
    if let Some(old) = map.remove("preset") {
        map.entry("active_preset").or_insert(old);
    }
}

/// Keeps the entries of a layer list whose id is known, once each. The map's own layer list
/// (`complete`) then gets every known layer it lacks, appended hidden.
fn tidy_layer_list(list: &mut Vec<Value>, path: &str, complete: bool, issues: &mut Issues) {
    let id_of = |v: &Value| v.get("id").and_then(Value::as_str).map(str::to_owned);
    let mut kept: Vec<Value> = Vec::with_capacity(list.len());
    for (i, entry) in list.drain(..).enumerate() {
        match id_of(&entry) {
            Some(id) if !KNOWN_LAYERS.contains(&id.as_str()) => issues.warning(
                format!("{path}[{i}]"),
                format!("`{id}` is not a known map layer and was dropped"),
            ),
            Some(id)
                if kept
                    .iter()
                    .any(|k| id_of(k).as_deref() == Some(id.as_str())) =>
            {
                issues.warning(
                    format!("{path}[{i}]"),
                    format!("`{id}` was listed twice; the first entry was kept"),
                );
            }
            _ => kept.push(entry),
        }
    }
    if complete {
        for id in KNOWN_LAYERS {
            if !kept.iter().any(|k| id_of(k).as_deref() == Some(id)) {
                kept.push(serde_json::json!({ "id": id, "shown": false }));
            }
        }
    }
    *list = kept;
}

/// Drops an unknown base map, and layer ids this version does not know from the layer list and the
/// presets, recording a warning for each; appends the layers the list lacks, hidden.
fn tidy_map(doc: &mut Value, issues: &mut Issues) {
    let Some(map) = doc.get_mut("map").and_then(Value::as_object_mut) else {
        return;
    };
    if let Some(base) = map.get("base").and_then(Value::as_str)
        && !KNOWN_BASES.contains(&base)
    {
        issues.warning(
            "map.base",
            format!("`{base}` is not a known base map; using the default"),
        );
        map.remove("base");
    }
    if let Some(layers) = map.get_mut("layers").and_then(Value::as_array_mut) {
        tidy_layer_list(layers, "map.layers", true, issues);
    }
    if let Some(presets) = map.get_mut("presets").and_then(Value::as_array_mut) {
        for (i, preset) in presets.iter_mut().enumerate() {
            if let Some(layers) = preset.get_mut("layers").and_then(Value::as_array_mut) {
                tidy_layer_list(layers, &format!("map.presets[{i}].layers"), false, issues);
            }
        }
    }
}

/// Drops followed species the bundled catalogue does not have, and repeats, recording a warning
/// for each. An entry that is not a string is left for the typed decode to refuse.
fn tidy_species(doc: &mut Value, issues: &mut Issues) {
    let Some(followed) = doc
        .get_mut("species")
        .and_then(|s| s.get_mut("followed"))
        .and_then(Value::as_array_mut)
    else {
        return;
    };
    let mut kept: Vec<Value> = Vec::with_capacity(followed.len());
    for (i, entry) in followed.drain(..).enumerate() {
        let path = format!("species.followed[{i}]");
        match entry.as_str() {
            Some(id) if !is_known_species(id) => issues.warning(
                path,
                format!("`{id}` is not a species this version knows and was dropped"),
            ),
            Some(_) if kept.contains(&entry) => issues.warning(
                path,
                "this species was listed twice; the first entry was kept",
            ),
            _ => kept.push(entry),
        }
    }
    *followed = kept;
}

/// Checks and upgrades a decoded document in place, ready for the typed decode. Returns the
/// warnings for what was changed.
pub fn prepare(doc: &mut Value) -> Result<Vec<Issue>, ImportError> {
    if !doc.is_object() {
        return Err(ImportError::Syntax(
            "the top level must be a table of settings".to_owned(),
        ));
    }
    if let Some(path) = find_key_like(doc) {
        return Err(ImportError::KeyLike { path });
    }
    if let Some(map) = doc.as_object_mut() {
        migrate(map)?;
    }
    rename_map_keys(doc);
    if let Some(path) = find_unknown_key(doc) {
        return Err(ImportError::UnknownKey { path });
    }
    let mut issues = Issues::default();
    tidy_sources(doc, &mut issues);
    tidy_map(doc, &mut issues);
    crate::alerts::tidy(doc, &mut issues);
    tidy_species(doc, &mut issues);
    Ok(issues.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use underbite_core::trip::SpeciesId;

    fn obj(v: &Value) -> Map<String, Value> {
        v.as_object().cloned().unwrap()
    }

    #[test]
    fn missing_version_means_one() {
        let mut d = obj(&json!({}));
        migrate(&mut d).unwrap();
        assert_eq!(d["version"], 1);
    }

    #[test]
    fn newer_versions_are_refused() {
        let mut d = obj(&json!({ "version": 2 }));
        let e = migrate(&mut d).unwrap_err();
        assert_eq!(
            e,
            ImportError::Newer {
                found: 2,
                supported: 1
            }
        );
        assert!(e.to_string().contains("newer Underbite"));
    }

    #[test]
    fn a_synthetic_v0_file_is_upgraded_step_by_step() {
        fn v0_to_v1(d: &mut Map<String, Value>) {
            if let Some(v) = d.remove("speed_unit") {
                d.insert("units".to_owned(), json!({ "speed": v }));
            }
        }
        fn v1_to_v2(d: &mut Map<String, Value>) {
            if let Some(u) = d.get_mut("units").and_then(Value::as_object_mut) {
                u.insert("height".to_owned(), json!("m"));
            }
        }
        let mut d = obj(&json!({ "version": 0, "speed_unit": "kn" }));
        migrate_with(&mut d, &[v0_to_v1, v1_to_v2], 2).unwrap();
        assert_eq!(
            Value::Object(d),
            json!({ "version": 2, "units": { "speed": "kn", "height": "m" } })
        );
    }

    #[test]
    fn key_like_names_are_found_with_their_path() {
        let v = json!({ "providers": { "usgs": { "api_key": "x" } } });
        assert_eq!(find_key_like(&v).as_deref(), Some("providers.usgs.api_key"));
        let v = json!({ "limits": [{ "name": "a" }, { "Token": 1 }] });
        assert_eq!(find_key_like(&v).as_deref(), Some("limits[1].Token"));
        assert_eq!(
            find_key_like(&json!({ "max_gust_kn": 1, "monkey": 2 })),
            None
        );
    }

    #[test]
    fn unknown_keys_get_their_full_path() {
        for (doc, path) in [
            (json!({ "colour": 1 }), "colour"),
            (json!({ "units": { "speeed": "kn" } }), "units.speeed"),
            (
                json!({ "sources": { "tide": { "chain": [], "bogus": 1 } } }),
                "sources.tide.bogus",
            ),
            (
                json!({ "limits": [{ "name": "a" }, { "name": "b", "max_knots": 1 }] }),
                "limits[1].max_knots",
            ),
            (json!({ "map": { "zoom": 3 } }), "map.zoom"),
        ] {
            let mut d = doc;
            assert_eq!(
                prepare(&mut d),
                Err(ImportError::UnknownKey { path: path.into() })
            );
        }
    }

    #[test]
    fn precision_is_clamped_and_unknown_sources_dropped() {
        let mut d = json!({ "sources": {
            "coordinate_precision": 4,
            "tide": { "chain": ["coops", "mystery", "coops-harmonics"] }
        }});
        let issues = prepare(&mut d).unwrap();
        assert_eq!(d["sources"]["coordinate_precision"], 2);
        assert_eq!(
            d["sources"]["tide"]["chain"],
            json!(["coops", "coops-harmonics"])
        );
        let paths: Vec<_> = issues.iter().map(|i| i.path.as_str()).collect();
        assert_eq!(
            paths,
            ["sources.coordinate_precision", "sources.tide.chain[1]"]
        );
    }

    #[test]
    fn an_old_map_section_is_renamed_and_still_decodes() {
        let mut d = json!({ "map": { "preset": "Offshore", "offline_max_mb": 800 } });
        assert_eq!(prepare(&mut d), Ok(vec![]));
        assert_eq!(d["map"]["active_preset"], "Offshore");
        assert!(d["map"].get("preset").is_none());
        let c: crate::schema::Config = serde_json::from_value(d).unwrap();
        assert_eq!(c.map.active_preset, "Offshore");
        assert_eq!(c.map.offline_max_mb, 800);
        assert_eq!(c.map.layers.len(), KNOWN_LAYERS.len());
    }

    #[test]
    fn a_document_without_a_map_section_gets_the_defaults() {
        let mut d = json!({ "units": { "speed": "kn" } });
        assert_eq!(prepare(&mut d), Ok(vec![]));
        let c: crate::schema::Config = serde_json::from_value(d).unwrap();
        assert_eq!(c.map, crate::map::MapConfig::default());
    }

    #[test]
    fn unknown_layers_are_dropped_and_missing_ones_appended_hidden() {
        let mut d = json!({ "map": {
            "base": "mystery-map",
            "layers": [
                { "id": "radar", "shown": true, "opacity": 0.5 },
                { "id": "sonar", "shown": true },
                { "id": "radar" },
                { "id": "sst", "shown": true }
            ],
            "presets": [{ "name": "Mine", "layers": [{ "id": "radar", "shown": true }, { "id": "sonar" }] }]
        }});
        let issues = prepare(&mut d).unwrap();
        let paths: Vec<_> = issues.iter().map(|i| i.path.as_str()).collect();
        assert_eq!(
            paths,
            [
                "map.base",
                "map.layers[1]",
                "map.layers[2]",
                "map.presets[0].layers[1]"
            ]
        );
        let c: crate::schema::Config = serde_json::from_value(d).unwrap();
        let ids: Vec<_> = c.map.layers.iter().map(|l| l.id.as_str()).collect();
        assert_eq!(&ids[..2], ["radar", "sst"]);
        assert_eq!(ids.len(), KNOWN_LAYERS.len());
        assert_eq!(
            (c.map.layers[0].shown, c.map.layers[0].opacity),
            (true, Some(0.5))
        );
        assert!(c.map.layers[2..].iter().all(|l| !l.shown));
        assert_eq!(c.map.base.as_str(), "apple-standard");
        assert_eq!(c.map.presets[0].layers.len(), 1);
        assert_eq!(c.validate(), []);
    }

    #[test]
    fn layers_from_before_the_access_own_depth_breaks_and_storms_ones_come_hidden_at_the_top() {
        let added = [
            "public-land",
            "access-points",
            "stocking-points",
            "species-records",
            "hab-beaches",
            "beach-forecast",
            "catches",
            "tracks",
            "catch-heat",
            "depth-shading",
            "depth-contours",
            "sst-breaks",
            "chlorophyll-breaks",
            "storms",
        ];
        let old: Vec<_> = KNOWN_LAYERS
            .iter()
            .filter(|id| !added.contains(id) && !id.starts_with("copernicus-"))
            .map(|id| json!({ "id": id, "shown": *id == "spots" }))
            .collect();
        assert_eq!(old.len(), 29);
        let mut d = json!({ "map": { "layers": old } });
        assert_eq!(prepare(&mut d), Ok(vec![]));
        let c: crate::schema::Config = serde_json::from_value(d).unwrap();
        let ids: Vec<_> = c.map.layers.iter().map(|l| l.id.as_str()).collect();
        assert_eq!(ids, KNOWN_LAYERS);
        assert!(c.map.layers[29..].iter().all(|l| !l.shown));
        assert!(
            c.map
                .layers
                .iter()
                .any(|l| l.id.as_str() == "spots" && l.shown)
        );
        assert_eq!(c.validate(), []);
    }

    #[test]
    fn a_layer_list_from_before_the_depth_layers_gets_them_hidden_at_the_end() {
        let old: Vec<_> = KNOWN_LAYERS
            .iter()
            .filter(|id| !id.starts_with("depth-"))
            .map(|id| json!({ "id": id, "shown": *id == "gebco-relief" }))
            .collect();
        assert_eq!(old.len(), KNOWN_LAYERS.len() - 2);
        let mut d = json!({ "map": { "layers": old } });
        assert_eq!(prepare(&mut d), Ok(vec![]));
        let c: crate::schema::Config = serde_json::from_value(d).unwrap();
        let tail: Vec<_> = c.map.layers[KNOWN_LAYERS.len() - 2..]
            .iter()
            .map(|l| (l.id.as_str(), l.shown))
            .collect();
        assert_eq!(tail, [("depth-shading", false), ("depth-contours", false)]);
        assert_eq!(c.validate(), []);
    }

    #[test]
    fn a_layer_list_from_before_the_breaks_layers_gets_them_hidden_at_the_end() {
        let old: Vec<_> = KNOWN_LAYERS
            .iter()
            .filter(|id| {
                !id.ends_with("-breaks") && **id != "storms" && !id.starts_with("copernicus-")
            })
            .map(|id| json!({ "id": id, "shown": *id == "sst" }))
            .collect();
        assert_eq!(old.len(), KNOWN_LAYERS.len() - 6);
        let mut d = json!({ "map": { "layers": old } });
        assert_eq!(prepare(&mut d), Ok(vec![]));
        let c: crate::schema::Config = serde_json::from_value(d).unwrap();
        let ids: Vec<_> = c.map.layers.iter().map(|l| l.id.as_str()).collect();
        assert_eq!(ids, KNOWN_LAYERS);
        let tail: Vec<_> = c.map.layers[KNOWN_LAYERS.len() - 6..KNOWN_LAYERS.len() - 3]
            .iter()
            .map(|l| (l.id.as_str(), l.shown))
            .collect();
        assert_eq!(
            tail,
            [
                ("sst-breaks", false),
                ("chlorophyll-breaks", false),
                ("storms", false)
            ]
        );
        assert_eq!(c.validate(), []);
    }

    #[test]
    fn a_layer_list_from_before_the_storms_layer_gets_it_hidden_at_the_end() {
        let old: Vec<_> = KNOWN_LAYERS
            .iter()
            .filter(|id| **id != "storms" && !id.starts_with("copernicus-"))
            .map(|id| json!({ "id": id, "shown": *id == "radar" }))
            .collect();
        assert_eq!(old.len(), KNOWN_LAYERS.len() - 4);
        let mut d = json!({ "map": { "layers": old } });
        assert_eq!(prepare(&mut d), Ok(vec![]));
        let c: crate::schema::Config = serde_json::from_value(d).unwrap();
        let ids: Vec<_> = c.map.layers.iter().map(|l| l.id.as_str()).collect();
        assert_eq!(ids, KNOWN_LAYERS);
        let storms = c.map.layers.get(KNOWN_LAYERS.len() - 4);
        assert_eq!(
            storms.map(|l| (l.id.as_str(), l.shown)),
            Some(("storms", false))
        );
        assert_eq!(c.validate(), []);
    }

    #[test]
    fn a_layer_list_from_before_the_copernicus_layers_gets_them_hidden_at_the_end() {
        let old: Vec<_> = KNOWN_LAYERS
            .iter()
            .filter(|id| !id.starts_with("copernicus-"))
            .map(|id| json!({ "id": id, "shown": *id == "radar" }))
            .collect();
        assert_eq!(old.len(), KNOWN_LAYERS.len() - 3);
        let mut d = json!({ "map": { "layers": old } });
        assert_eq!(prepare(&mut d), Ok(vec![]));
        let c: crate::schema::Config = serde_json::from_value(d).unwrap();
        let ids: Vec<_> = c.map.layers.iter().map(|l| l.id.as_str()).collect();
        assert_eq!(ids, KNOWN_LAYERS);
        let tail: Vec<_> = c.map.layers[KNOWN_LAYERS.len() - 3..]
            .iter()
            .map(|l| (l.id.as_str(), l.shown))
            .collect();
        assert_eq!(
            tail,
            [
                ("copernicus-currents", false),
                ("copernicus-sst", false),
                ("copernicus-sea-level", false)
            ]
        );
        assert_eq!(c.validate(), []);
    }

    #[test]
    fn an_unknown_key_in_a_layer_entry_is_named_by_path() {
        let mut d = json!({ "map": { "layers": [{ "id": "sst", "glow": 1 }] } });
        assert_eq!(
            prepare(&mut d),
            Err(ImportError::UnknownKey {
                path: "map.layers[0].glow".into()
            })
        );
    }

    #[test]
    fn alerts_unknown_keys_get_their_full_path_through_tagged_variants() {
        let rule = |target: Value, condition: Value| {
            json!({ "alerts": [
                { "id": "a", "name": "A", "target": { "kind": "spot", "id": "s" }, "all_of": [] },
                { "id": "b", "name": "B", "target": target, "all_of": [{ "kind": "storm_ahead" }, condition] }
            ]})
        };
        for (doc, path) in [
            (
                rule(
                    json!({ "kind": "spot", "id": "s", "lat": 40.7 }),
                    json!({ "kind": "trip_go" }),
                ),
                "alerts[1].target.lat",
            ),
            (
                rule(
                    json!({ "kind": "box", "south": 40, "west": -74, "north": 40.5, "east": -73.5, "zoom": 2 }),
                    json!({ "kind": "trip_go" }),
                ),
                "alerts[1].target.zoom",
            ),
            (
                rule(
                    json!({ "kind": "trip", "id": "t" }),
                    json!({ "kind": "measurement", "metric": "discharge", "below": 1, "unit": "cfs" }),
                ),
                "alerts[1].all_of[1].unit",
            ),
            (
                rule(
                    json!({ "kind": "trip", "id": "t" }),
                    json!({ "kind": "trip_go", "day": 2 }),
                ),
                "alerts[1].all_of[1].day",
            ),
            (
                json!({ "alerts": [{ "id": "a", "name": "A", "target": { "kind": "spot", "id": "s" },
                    "all_of": [], "quiet_hours": { "start_minute": 0, "end_minute": 60, "zone": "UTC" } }] }),
                "alerts[0].quiet_hours.zone",
            ),
        ] {
            let mut d = doc;
            assert_eq!(
                prepare(&mut d),
                Err(ImportError::UnknownKey { path: path.into() })
            );
        }
    }

    #[test]
    fn alerts_a_document_without_them_decodes_and_unknown_kinds_are_dropped() {
        let mut d = json!({ "version": 1, "units": { "speed": "kn" } });
        assert_eq!(prepare(&mut d), Ok(vec![]));
        let c: crate::schema::Config = serde_json::from_value(d).unwrap();
        assert_eq!(c.alerts, []);

        let mut d = json!({ "alerts": [
            { "id": "a", "name": "A", "target": { "kind": "geofence", "id": "s" }, "all_of": [] },
            { "id": "b", "name": "B", "target": { "kind": "spot", "id": "deleted-spot" },
              "all_of": [{ "kind": "marine_alert", "event": "Gale Warning" }] }
        ]});
        let issues = prepare(&mut d).unwrap();
        let paths: Vec<_> = issues.iter().map(|i| i.path.as_str()).collect();
        assert_eq!(paths, ["alerts[0].target.kind"]);
        let c: crate::schema::Config = serde_json::from_value(d).unwrap();
        assert_eq!(c.alerts.len(), 1);
        assert_eq!(c.alerts[0].id, "b");
        assert_eq!(c.validate(), []);
    }

    #[test]
    fn species_a_document_without_them_follows_none() {
        let mut d = json!({ "version": 1, "windows": { "show_solunar": false } });
        assert_eq!(prepare(&mut d), Ok(vec![]));
        let c: crate::schema::Config = serde_json::from_value(d).unwrap();
        assert_eq!(c.species, crate::species::SpeciesConfig::default());
        assert_eq!(c.validate(), []);
    }

    #[test]
    fn species_unknown_and_repeated_ids_are_dropped_with_warnings() {
        let mut d = json!({ "species": { "followed": [
            "micropterus-dolomieu", "no-such-fish", "Salmo trutta",
            "salmo-trutta", "micropterus-dolomieu"
        ] } });
        let issues = prepare(&mut d).unwrap();
        let paths: Vec<_> = issues.iter().map(|i| i.path.as_str()).collect();
        assert_eq!(
            paths,
            [
                "species.followed[1]",
                "species.followed[2]",
                "species.followed[4]"
            ]
        );
        assert!(issues[0].message.contains("no-such-fish"));
        let c: crate::schema::Config = serde_json::from_value(d).unwrap();
        let ids: Vec<_> = c.species.followed.iter().map(SpeciesId::as_str).collect();
        assert_eq!(ids, ["micropterus-dolomieu", "salmo-trutta"]);
        assert_eq!(c.validate(), []);
    }

    #[test]
    fn species_an_unknown_key_is_named_by_path() {
        let mut d = json!({ "species": { "followed": [], "favourite": "salmo-trutta" } });
        assert_eq!(
            prepare(&mut d),
            Err(ImportError::UnknownKey {
                path: "species.favourite".into()
            })
        );
    }

    #[test]
    fn a_top_level_that_is_not_a_table_is_refused() {
        assert!(matches!(
            prepare(&mut json!([1])),
            Err(ImportError::Syntax(_))
        ));
    }
}
