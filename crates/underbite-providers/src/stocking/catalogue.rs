//! The bundled catalogue of stocked waterbodies: each list's county and waterbody name, placed on the map.
//!
//! The catalogue is `data/stocking/ny.json`, built offline from recorded responses (method in
//! `data/stocking/README.md`). A waterbody is either placed (a representative point and a kind) or
//! listed as unmatched; nothing is guessed. A river or creek is a line, so its point is a representative
//! point on that line, not a location of the stocking.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

const NY_JSON: &str = include_str!("../../data/stocking/ny.json");

/// What a stocked waterbody is, as far as a spot search cares.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    /// A lake or reservoir.
    Lake,
    /// A river, creek, brook, stream or kill (a line).
    River,
    /// A pond.
    Pond,
}

impl Kind {
    /// The kind a waterbody's own type word gives, if it has one.
    pub fn from_name(name: &str) -> Option<Self> {
        let normalised = normalise(name);
        match normalised.split(' ').next_back()? {
            "pond" => Some(Self::Pond),
            "lake" | "reservoir" => Some(Self::Lake),
            "creek" | "brook" | "river" | "stream" | "kill" => Some(Self::River),
            _ => None,
        }
    }
}

/// A waterbody placed on the map.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Placed {
    /// County, title case (`St. Lawrence`).
    pub county: String,
    /// The waterbody as the state writes it.
    pub waterbody: String,
    /// The name of the matched map feature.
    pub name: String,
    /// Representative point, degrees (WGS 84). For a river, a point on its line.
    pub latitude: f64,
    /// Representative point, degrees (WGS 84).
    pub longitude: f64,
    /// Kind of waterbody.
    pub kind: Kind,
    /// 3DHP feature id, or `manual`.
    pub source: String,
}

/// A stocked waterbody the build could not place.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Unmatched {
    /// County, title case.
    pub county: String,
    /// The waterbody as the state writes it.
    pub waterbody: String,
    /// What is known about it, when something is (a feature identified without a recorded point).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// The outcome of looking a `(county, waterbody)` pair up.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Resolution<'a> {
    /// The waterbody has a point.
    Placed(&'a Placed),
    /// The waterbody is stocked but its location is unknown.
    Unmatched(&'a Unmatched),
    /// Not in the catalogue.
    Unknown,
}

/// The catalogue file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CatalogueFile {
    /// Which lists the pairs come from.
    pub source: String,
    /// Date the lists and map responses were recorded (UTC).
    pub recorded: String,
    /// Waterbodies with a point, sorted by county and waterbody.
    pub placed: Vec<Placed>,
    /// Waterbodies without one, sorted by county and waterbody.
    pub unmatched: Vec<Unmatched>,
}

/// A loaded catalogue with its lookup indexes.
#[derive(Debug, Clone)]
pub struct Catalogue {
    file: CatalogueFile,
    placed: BTreeMap<(String, String), usize>,
    unmatched: BTreeMap<(String, String), usize>,
}

/// The catalogue could not be read.
#[derive(Debug, thiserror::Error)]
pub enum CatalogueError {
    /// The JSON is malformed.
    #[error("catalogue is not valid JSON: {0}")]
    Json(#[from] serde_json::Error),
}

impl Catalogue {
    /// The catalogue bundled with the library.
    pub fn bundled() -> Result<Self, CatalogueError> {
        Self::from_json(NY_JSON)
    }

    /// Reads a catalogue from its JSON text.
    pub fn from_json(text: &str) -> Result<Self, CatalogueError> {
        let file: CatalogueFile = serde_json::from_str(text)?;
        let placed = file
            .placed
            .iter()
            .enumerate()
            .map(|(i, p)| (key(&p.county, &p.waterbody), i))
            .collect();
        let unmatched = file
            .unmatched
            .iter()
            .enumerate()
            .map(|(i, u)| (key(&u.county, &u.waterbody), i))
            .collect();
        Ok(Self {
            file,
            placed,
            unmatched,
        })
    }

    /// The catalogue file as read.
    pub fn file(&self) -> &CatalogueFile {
        &self.file
    }

    /// Looks a pair up as the state's lists write it: county in any case, waterbody ignoring case and
    /// surrounding or repeated white space. The same name in two counties is two entries.
    pub fn resolve(&self, county: &str, waterbody: &str) -> Resolution<'_> {
        let k = key(county, waterbody);
        if let Some(&i) = self.placed.get(&k) {
            Resolution::Placed(&self.file.placed[i])
        } else if let Some(&i) = self.unmatched.get(&k) {
            Resolution::Unmatched(&self.file.unmatched[i])
        } else {
            Resolution::Unknown
        }
    }

    /// Placed waterbodies within `radius_m` metres of a point, nearest first, with their distance.
    pub fn near(&self, latitude: f64, longitude: f64, radius_m: f64) -> Vec<(&Placed, f64)> {
        let mut found: Vec<_> = self
            .file
            .placed
            .iter()
            .map(|p| (p, distance_m(latitude, longitude, p.latitude, p.longitude)))
            .filter(|(_, d)| *d <= radius_m)
            .collect();
        found.sort_by(|a, b| a.1.total_cmp(&b.1));
        found
    }
}

fn key(county: &str, waterbody: &str) -> (String, String) {
    (canonical_county(county), canonical_waterbody(waterbody))
}

/// The county as the Census writes it: title case, `St.` with its full stop, one space between words.
pub fn canonical_county(county: &str) -> String {
    let words: Vec<String> = county
        .split_whitespace()
        .map(|w| {
            let lower = w.to_lowercase();
            if lower == "st" || lower == "st." {
                return "St.".to_owned();
            }
            let mut chars = lower.chars();
            chars
                .next()
                .map_or_else(String::new, |c| c.to_uppercase().chain(chars).collect())
        })
        .collect();
    words.join(" ")
}

/// The waterbody lower-cased with white space collapsed: the catalogue's lookup form.
pub fn canonical_waterbody(waterbody: &str) -> String {
    waterbody
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// The waterbody with agency qualifiers and abbreviations removed, for comparing with a map name:
/// lower case; no parenthetical text or `#n`; nothing after ` - `; `&` as `and`, `.` as a space; no
/// trailing numbering or `AS`; `Res`, `Cr`, `Ck`, `Br`, `Pd` and `Mt` spelled out.
pub fn normalise(waterbody: &str) -> String {
    let mut text = waterbody.to_lowercase();
    // Parenthetical text, including an unclosed one.
    while let Some(open) = text.find('(') {
        let end = text[open..].find(')').map_or(text.len(), |c| open + c + 1);
        text.replace_range(open..end, " ");
    }
    if let Some(dash) = text.find(" - ") {
        text.truncate(dash);
    }
    let text = text.replace('&', " and ").replace('.', " ");
    let mut words: Vec<&str> = text
        .split_whitespace()
        .filter(|w| {
            !(w.starts_with('#') && w.len() > 1 && w[1..].chars().all(|c| c.is_ascii_digit()))
        })
        .collect();
    if words.last() == Some(&"as") {
        words.pop();
    }
    while words.len() > 1
        && words
            .last()
            .is_some_and(|w| w.chars().all(|c| c.is_ascii_digit()))
    {
        words.pop();
    }
    let words: Vec<&str> = words
        .into_iter()
        .map(|w| match w {
            "res" => "reservoir",
            "cr" | "ck" => "creek",
            "br" => "brook",
            "pd" => "pond",
            "mt" => "mount",
            _ => w,
        })
        .collect();
    words.join(" ")
}

/// Whether a state waterbody name and a map feature name are the same waterbody: equal once normalised,
/// in any word order (`Fish Creek East Branch` is `East Branch Fish Creek`). A shared name core with a
/// different type word (`Moose Pond`, `Moose Creek`) is not a match.
pub fn name_matches(waterbody: &str, feature_name: &str) -> bool {
    let sorted = |s: &str| {
        let mut words: Vec<String> = normalise(s).split(' ').map(str::to_owned).collect();
        words.sort();
        words
    };
    let a = sorted(waterbody);
    !a.iter().all(String::is_empty) && a == sorted(feature_name)
}

fn distance_m(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    const EARTH_RADIUS_M: f64 = 6_371_008.8;
    let (p1, p2) = (lat1.to_radians(), lat2.to_radians());
    let dphi = p2 - p1;
    let dlambda = (lon2 - lon1).to_radians();
    let a = (dphi / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dlambda / 2.0).sin().powi(2);
    2.0 * EARTH_RADIUS_M * a.sqrt().asin()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::fixture_str;

    const E52K: &str = "socrata-ny-e52k-ymww-county-waterbody-pairs-2026-11-17.json";
    const D9Y2: &str = "socrata-ny-d9y2-n436-county-waterbody-pairs-2026-11-17.json";
    const KENSICO: &str = "3dhp-waterbody-layer60-name-query-kensico-2026-11-17.json";

    /// The pairs of both lists, one per county and lower-cased waterbody, with the most frequent
    /// published spelling (ties: the smallest).
    fn pairs() -> BTreeMap<(String, String), String> {
        let mut spellings: BTreeMap<(String, String), BTreeMap<String, u64>> = BTreeMap::new();
        for name in [E52K, D9Y2] {
            let rows: Vec<BTreeMap<String, String>> =
                serde_json::from_str(&fixture_str("stocking-ny", name)).expect("pairs JSON");
            for row in rows {
                let n: u64 = row["n"].parse().expect("a count");
                let shown = row["waterbody"]
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ");
                let entry = spellings
                    .entry(key(&row["county"], &row["waterbody"]))
                    .or_default();
                *entry.entry(shown).or_default() += n;
            }
        }
        spellings
            .into_iter()
            .map(|(k, s)| {
                let best = s
                    .into_iter()
                    .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0)))
                    .map(|(name, _)| name)
                    .expect("one spelling");
                (k, best)
            })
            .collect()
    }

    /// The build: every pair is unmatched unless a recorded response places it. The recorded responses
    /// carry no coordinates, so nothing is placed; the one feature they identify is noted.
    fn build() -> CatalogueFile {
        let kensico: serde_json::Value =
            serde_json::from_str(&fixture_str("stocking-ny", KENSICO)).expect("3DHP JSON");
        let attrs = &kensico["features"][0]["attributes"];
        let id = attrs["id3dhp"].as_str().expect("id");
        let feature = attrs["gnisidlabel"].as_str().expect("label");
        let unmatched = pairs()
            .into_iter()
            .map(|((county, _), waterbody)| {
                // The envelope queried for this feature lies in Westchester County.
                let note = (county == "Westchester" && name_matches(&waterbody, feature))
                    .then(|| format!("3DHP waterbody {id} ({feature}); no point recorded"));
                Unmatched {
                    county,
                    waterbody,
                    note,
                }
            })
            .collect();
        CatalogueFile {
            source: "NYS DEC stocking lists e52k-ymww and d9y2-n436".to_owned(),
            recorded: "2026-11-17".to_owned(),
            placed: Vec::new(),
            unmatched,
        }
    }

    /// One entry per line, so a review reads as a list.
    fn render(file: &CatalogueFile) -> String {
        fn list<T: Serialize>(items: &[T]) -> String {
            if items.is_empty() {
                return "[]".to_owned();
            }
            let lines: Vec<String> = items
                .iter()
                .map(|i| serde_json::to_string(i).expect("serialise"))
                .collect();
            format!("[\n  {}\n ]", lines.join(",\n  "))
        }
        format!(
            "{{\n \"source\": {},\n \"recorded\": {},\n \"placed\": {},\n \"unmatched\": {}\n}}\n",
            serde_json::to_string(&file.source).expect("serialise"),
            serde_json::to_string(&file.recorded).expect("serialise"),
            list(&file.placed),
            list(&file.unmatched)
        )
    }

    /// Rewrites `data/stocking/ny.json` from the recorded responses:
    /// `cargo test -p underbite-providers regenerate_ny_catalogue -- --ignored`.
    #[test]
    #[ignore = "rewrites data/stocking/ny.json"]
    fn regenerate_ny_catalogue() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("data/stocking/ny.json");
        std::fs::write(path, render(&build())).expect("write ny.json");
    }

    #[test]
    fn the_bundled_file_is_what_the_build_produces() {
        assert_eq!(
            NY_JSON,
            render(&build()),
            "run the regenerate_ny_catalogue test"
        );
        assert_eq!(Catalogue::bundled().expect("loads").file(), &build());
    }

    #[test]
    fn counts_and_order() {
        let c = Catalogue::bundled().expect("loads");
        let f = c.file();
        assert_eq!(f.placed.len() + f.unmatched.len(), 1586);
        let keys: Vec<_> = f
            .unmatched
            .iter()
            .map(|u| key(&u.county, &u.waterbody))
            .collect();
        assert!(
            keys.windows(2).all(|w| w[0] < w[1]),
            "sorted, one entry per pair"
        );
    }

    #[test]
    fn westchester_waterbodies_resolve_or_are_unmatched() {
        let c = Catalogue::bundled().expect("loads");
        for name in [
            "Kensico Reservoir",
            "Pocantico River",
            "Dogwood Pond",
            "Cross River Reservoir",
            "Cross River Reservoir Outlet",
            "Stone Hill River",
        ] {
            assert_ne!(
                c.resolve("Westchester", name),
                Resolution::Unknown,
                "{name}"
            );
            let shouted = format!("  {} ", name.to_uppercase());
            assert_eq!(
                c.resolve("westchester", &shouted),
                c.resolve("Westchester", name)
            );
        }
        let Resolution::Unmatched(k) = c.resolve("Westchester", "Kensico Reservoir") else {
            panic!("Kensico Reservoir is unmatched")
        };
        assert!(k.note.as_deref().is_some_and(|n| n.contains("JLOQT")));
        assert_eq!(
            c.resolve("Putnam", "Kensico Reservoir"),
            Resolution::Unknown
        );
    }

    #[test]
    fn county_spellings_collapse() {
        assert_eq!(canonical_county("ST LAWRENCE"), "St. Lawrence");
        assert_eq!(canonical_county("St. Lawrence"), "St. Lawrence");
        assert_eq!(canonical_county("clinton"), "Clinton");
        let c = Catalogue::bundled().expect("loads");
        assert!(
            c.file()
                .unmatched
                .iter()
                .all(|u| u.county == canonical_county(&u.county))
        );
    }

    #[test]
    fn normalisation_follows_the_documented_rules() {
        assert_eq!(normalise("Patterson Pond (#1)"), "patterson pond");
        assert_eq!(normalise("Clear Pond (P594- Upper Hudson)"), "clear pond");
        assert_eq!(normalise("Palmer Pond 2 AS"), "palmer pond");
        assert_eq!(normalise("Bog Brook Res"), "bog brook reservoir");
        assert_eq!(
            normalise("Peru Fish & Game Club Pond"),
            "peru fish and game club pond"
        );
        assert_eq!(normalise("Mt. Morris Pd - North"), "mount morris pond");
        assert_eq!(normalise("Oak Pond (unclosed"), "oak pond");
    }

    #[test]
    fn a_core_match_with_another_type_word_is_rejected() {
        assert!(name_matches("Moose Pond", "Moose Pond"));
        assert!(!name_matches("Moose Pond", "Moose Creek"));
        assert!(!name_matches("Rock Pond", "Rock Pond Brook"));
        assert!(!name_matches("Oneida Lake", "Oneida River"));
        assert!(name_matches(
            "Fish Creek East Branch",
            "East Branch Fish Creek"
        ));
        assert!(name_matches("Cross River Res", "Cross River Reservoir"));
        assert!(!name_matches("", ""));
    }

    #[test]
    fn kinds_come_from_the_type_word() {
        assert_eq!(Kind::from_name("Kensico Reservoir"), Some(Kind::Lake));
        assert_eq!(Kind::from_name("Dogwood Pond"), Some(Kind::Pond));
        assert_eq!(Kind::from_name("Pocantico River"), Some(Kind::River));
        assert_eq!(Kind::from_name("County Fair"), None);
    }

    #[test]
    fn nearby_search_orders_by_distance() {
        let json = r#"{"source":"t","recorded":"2026-11-17","unmatched":[],"placed":[
            {"county":"A","waterbody":"Far Pond","name":"Far Pond","latitude":41.2,"longitude":-73.8,"kind":"Pond","source":"manual"},
            {"county":"A","waterbody":"Near Lake","name":"Near Lake","latitude":41.101,"longitude":-73.8,"kind":"Lake","source":"X1"}]}"#;
        let c = Catalogue::from_json(json).expect("loads");
        let found = c.near(41.1, -73.8, 20_000.0);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].0.name, "Near Lake");
        assert!((found[0].1 - 111.2).abs() < 1.0);
        assert_eq!(c.near(41.1, -73.8, 1_000.0).len(), 1);
        assert!(matches!(c.resolve("a", "near lake"), Resolution::Placed(_)));
    }
}
