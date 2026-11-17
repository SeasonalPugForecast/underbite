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
    /// Set when several map features in the county carry the name: how the one placed was chosen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// A stocked waterbody the build could not place.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Unmatched {
    /// County, title case.
    pub county: String,
    /// The waterbody as the state writes it.
    pub waterbody: String,
    /// Why it is not placed.
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
    const TIGERWEB: &str = "tigerweb-ny-counties-generalised-2026-11-17.json";

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

    /// A point, `[longitude, latitude]`.
    type Pt = [f64; 2];

    /// One recorded 3DHP feature: its rings (layer 60) or paths (layer 50).
    struct Feature {
        id: String,
        name: String,
        layer: u8,
        parts: Vec<Vec<Pt>>,
    }

    fn pt(v: &serde_json::Value) -> Pt {
        [
            v[0].as_f64().expect("longitude"),
            v[1].as_f64().expect("latitude"),
        ]
    }

    fn parts(v: &serde_json::Value) -> Vec<Vec<Pt>> {
        v.as_array()
            .expect("array of parts")
            .iter()
            .map(|part| part.as_array().expect("part").iter().map(pt).collect())
            .collect()
    }

    /// The county rings of the generalised Census layer, by canonical county name.
    fn counties() -> BTreeMap<String, Vec<Vec<Pt>>> {
        let doc: serde_json::Value =
            serde_json::from_str(&fixture_str("stocking-ny", TIGERWEB)).expect("TIGERweb JSON");
        doc["features"]
            .as_array()
            .expect("features")
            .iter()
            .map(|f| {
                (
                    canonical_county(f["attributes"]["BASENAME"].as_str().expect("county")),
                    parts(&f["geometry"]["rings"]),
                )
            })
            .collect()
    }

    /// The file-name form of a county: `St. Lawrence` is `st-lawrence`.
    fn slug(county: &str) -> String {
        county
            .to_lowercase()
            .replace('.', "")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join("-")
    }

    /// Every recorded 3DHP feature of a county, one per layer and id (the name files and the
    /// spelling-variant files can overlap), in file-name then response order.
    fn features(county: &str) -> Vec<Feature> {
        let dir = crate::testutil::fixture_path("stocking-ny", "");
        let prefixes: Vec<String> = [60, 50]
            .iter()
            .flat_map(|layer| {
                ["names", "spelling-variants"]
                    .map(|kind| format!("3dhp-layer{layer}-county-{}-{kind}-", slug(county)))
            })
            .collect();
        let mut files: Vec<String> = std::fs::read_dir(dir)
            .expect("fixture directory")
            .filter_map(|e| e.ok()?.file_name().into_string().ok())
            .filter(|n| prefixes.iter().any(|p| n.starts_with(p)))
            .collect();
        files.sort();
        let mut seen = std::collections::BTreeSet::new();
        let mut out = Vec::new();
        for file in files {
            let layer: u8 = if file.starts_with("3dhp-layer60") {
                60
            } else {
                50
            };
            let doc: serde_json::Value =
                serde_json::from_str(&fixture_str("stocking-ny", &file)).expect("3DHP JSON");
            for f in doc["features"].as_array().expect("features") {
                let id = f["attributes"]["id3dhp"].as_str().expect("id").to_owned();
                if !seen.insert((layer, id.clone())) {
                    continue;
                }
                let key = if layer == 60 { "rings" } else { "paths" };
                out.push(Feature {
                    id,
                    name: f["attributes"]["gnisidlabel"]
                        .as_str()
                        .expect("name")
                        .to_owned(),
                    layer,
                    parts: parts(&f["geometry"][key]),
                });
            }
        }
        out
    }

    /// Even-odd point in polygon over all rings (holes and islands fall out of the rule).
    fn inside(p: Pt, rings: &[Vec<Pt>]) -> bool {
        let mut odd = false;
        for ring in rings {
            for w in ring.windows(2) {
                let (a, b) = (w[0], w[1]);
                if (a[1] > p[1]) != (b[1] > p[1])
                    && p[0] < a[0] + (p[1] - a[1]) * (b[0] - a[0]) / (b[1] - a[1])
                {
                    odd = !odd;
                }
            }
        }
        odd
    }

    /// Degrees rounded to 4 decimals, as recorded.
    fn r4(x: f64) -> f64 {
        (x * 1e4).round() / 1e4
    }

    /// Interior point candidates of a polygon, best first: the midpoints of the interior runs of 19
    /// horizontal scan lines, widest run first (ties: the lower line).
    fn interior_candidates(rings: &[Vec<Pt>]) -> Vec<Pt> {
        let (mut lo, mut hi) = (f64::MAX, f64::MIN);
        for p in rings.iter().flatten() {
            lo = lo.min(p[1]);
            hi = hi.max(p[1]);
        }
        let mut runs: Vec<(f64, Pt)> = Vec::new();
        for k in 1..20 {
            let y = lo + (hi - lo) * f64::from(k) / 20.0;
            let mut xs: Vec<f64> = Vec::new();
            for ring in rings {
                for w in ring.windows(2) {
                    let (a, b) = (w[0], w[1]);
                    if (a[1] > y) != (b[1] > y) {
                        xs.push(a[0] + (y - a[1]) * (b[0] - a[0]) / (b[1] - a[1]));
                    }
                }
            }
            xs.sort_by(f64::total_cmp);
            for run in xs.as_chunks::<2>().0 {
                runs.push((run[1] - run[0], [r4(run[0].midpoint(run[1])), r4(y)]));
            }
        }
        runs.sort_by(|a, b| b.0.total_cmp(&a.0));
        runs.into_iter().map(|(_, p)| p).collect()
    }

    /// Net area of rings, in square degrees scaled by the cosine of the latitude.
    fn area(rings: &[Vec<Pt>]) -> f64 {
        let mut sum = 0.0;
        let mut cos = 1.0;
        for ring in rings {
            for w in ring.windows(2) {
                sum += w[0][0] * w[1][1] - w[1][0] * w[0][1];
                cos = w[0][1].to_radians().cos();
            }
        }
        (sum / 2.0 * cos).abs()
    }

    fn path_length(path: &[Pt]) -> f64 {
        path.windows(2)
            .map(|w| {
                let cos = w[0][1].to_radians().cos();
                ((w[1][0] - w[0][0]) * cos).hypot(w[1][1] - w[0][1])
            })
            .sum()
    }

    /// The vertex halfway along the longest path of a line feature.
    fn river_point(paths: &[Vec<Pt>]) -> Option<Pt> {
        let longest = paths
            .iter()
            .filter(|p| !p.is_empty())
            .max_by(|a, b| path_length(a).total_cmp(&path_length(b)))?;
        let half = path_length(longest) / 2.0;
        let mut walked = 0.0;
        for (i, w) in longest.windows(2).enumerate() {
            walked += path_length(w);
            if walked >= half {
                return Some(longest[i + 1]);
            }
        }
        longest.first().copied()
    }

    /// A same-name feature with the point it would be placed at, inside its county.
    struct Candidate<'a> {
        feature: &'a Feature,
        point: Pt,
        /// Area (layer 60) or total length (layer 50): the larger is chosen.
        size: f64,
    }

    /// The candidates among same-name features in rank order, or why there are none.
    fn candidates<'a>(
        found: &[&'a Feature],
        county: &[Vec<Pt>],
    ) -> Result<Vec<Candidate<'a>>, &'static str> {
        let mut out = Vec::new();
        let mut no_point = false;
        for &feature in found {
            let (point, size) = if feature.layer == 60 {
                let options = interior_candidates(&feature.parts);
                no_point |= options.is_empty();
                let point = options
                    .into_iter()
                    .find(|&p| inside(p, &feature.parts) && inside(p, county));
                (point, area(&feature.parts))
            } else {
                let point = river_point(&feature.parts).filter(|&p| inside(p, county));
                (point, feature.parts.iter().map(|p| path_length(p)).sum())
            };
            if let Some(point) = point {
                out.push(Candidate {
                    feature,
                    point,
                    size,
                });
            }
        }
        if out.is_empty() {
            return Err(if no_point {
                "a same-name 3DHP feature has no interior point at the recorded generalisation"
            } else {
                "same-name 3DHP features lie outside the county"
            });
        }
        out.sort_by(|a, b| {
            b.size
                .total_cmp(&a.size)
                .then_with(|| a.feature.id.cmp(&b.feature.id))
        });
        Ok(out)
    }

    /// The build: a pair is placed when a recorded 3DHP feature carries its name (`name_matches`, in
    /// the layer its type word names; a name without a type word tries lakes, then lines) and a point
    /// chosen on the feature lies inside the county polygon; a feature whose point is outside is
    /// rejected, never moved. With several such features the largest waterbody or the river with the
    /// most recorded length wins (ties: the smaller id) and the others are named in the note.
    fn build() -> CatalogueFile {
        let counties = counties();
        let mut by_county: BTreeMap<String, Vec<Feature>> = BTreeMap::new();
        let mut placed = Vec::new();
        let mut unmatched = Vec::new();
        for ((county, _), waterbody) in pairs() {
            let Some(rings) = counties.get(&county) else {
                unmatched.push(Unmatched {
                    county,
                    waterbody,
                    note: Some("county not in the Census layer".to_owned()),
                });
                continue;
            };
            let all = by_county
                .entry(county.clone())
                .or_insert_with(|| features(&county));
            let kind = Kind::from_name(&waterbody);
            let layers: &[u8] = match kind {
                Some(Kind::Lake | Kind::Pond) => &[60],
                Some(Kind::River) => &[50],
                None => &[60, 50],
            };
            let mut reason = "no same-name feature in the recorded 3DHP responses";
            let mut chosen = None;
            for &layer in layers {
                let found: Vec<&Feature> = all
                    .iter()
                    .filter(|f| f.layer == layer && name_matches(&waterbody, &f.name))
                    .collect();
                if found.is_empty() {
                    continue;
                }
                match candidates(&found, rings) {
                    Ok(list) => {
                        chosen = Some(list);
                        break;
                    }
                    Err(why) => reason = why,
                }
            }
            match chosen {
                Some(list) => {
                    let best = &list[0];
                    let note = (list.len() > 1).then(|| {
                        let others: Vec<&str> = list[1..]
                            .iter()
                            .take(2)
                            .map(|c| c.feature.id.as_str())
                            .collect();
                        format!(
                            "{} features share the name in the county; placed the {}; next: {}",
                            list.len(),
                            if best.feature.layer == 60 {
                                "largest"
                            } else {
                                "longest"
                            },
                            others.join(", ")
                        )
                    });
                    placed.push(Placed {
                        county,
                        waterbody,
                        name: best.feature.name.clone(),
                        latitude: best.point[1],
                        longitude: best.point[0],
                        kind: kind.unwrap_or(if best.feature.layer == 60 {
                            Kind::Lake
                        } else {
                            Kind::River
                        }),
                        source: best.feature.id.clone(),
                        note,
                    });
                }
                None => unmatched.push(Unmatched {
                    county,
                    waterbody,
                    note: Some(reason.to_owned()),
                }),
            }
        }
        CatalogueFile {
            source: "NYS DEC stocking lists e52k-ymww and d9y2-n436".to_owned(),
            recorded: "2026-11-17".to_owned(),
            placed,
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
        assert!(
            (1000..1200).contains(&f.placed.len()),
            "{} placed",
            f.placed.len()
        );
        assert!(f.unmatched.iter().all(|u| u.note.is_some()));
        let placed: Vec<_> = f
            .placed
            .iter()
            .map(|u| key(&u.county, &u.waterbody))
            .collect();
        let unmatched: Vec<_> = f
            .unmatched
            .iter()
            .map(|u| key(&u.county, &u.waterbody))
            .collect();
        for keys in [placed, unmatched] {
            assert!(
                keys.windows(2).all(|w| w[0] < w[1]),
                "sorted, one entry per pair"
            );
        }
        assert_eq!(c.placed.len(), f.placed.len());
    }

    #[test]
    fn every_placed_point_is_inside_its_county_and_its_feature() {
        let counties = counties();
        let mut by_county: BTreeMap<String, Vec<Feature>> = BTreeMap::new();
        for p in &Catalogue::bundled().expect("loads").file().placed {
            assert!(
                inside([p.longitude, p.latitude], &counties[&p.county]),
                "{} {} is outside its county",
                p.county,
                p.waterbody
            );
            let all = by_county
                .entry(p.county.clone())
                .or_insert_with(|| features(&p.county));
            let f = all.iter().find(|f| f.id == p.source).expect("feature");
            assert!(name_matches(&p.waterbody, &f.name));
            if f.layer == 60 {
                assert!(
                    inside([p.longitude, p.latitude], &f.parts),
                    "{} {}",
                    p.county,
                    p.waterbody
                );
            } else {
                assert!(
                    f.parts
                        .iter()
                        .flatten()
                        .any(|v| (v[0] - p.longitude).abs() < 1e-9
                            && (v[1] - p.latitude).abs() < 1e-9),
                    "{} {}",
                    p.county,
                    p.waterbody
                );
            }
            for x in [p.latitude, p.longitude] {
                assert_eq!(x, r4(x));
            }
        }
    }

    #[test]
    fn an_interior_point_of_a_concave_polygon_is_inside() {
        // A "C" shape whose bounding-box centre (2, 2) is in the notch.
        let c: Vec<Vec<Pt>> = vec![vec![
            [0.0, 0.0],
            [4.0, 0.0],
            [4.0, 1.0],
            [1.0, 1.0],
            [1.0, 3.0],
            [4.0, 3.0],
            [4.0, 4.0],
            [0.0, 4.0],
            [0.0, 0.0],
        ]];
        assert!(!inside([2.0, 2.0], &c));
        let options = interior_candidates(&c);
        assert!(options.iter().all(|&p| inside(p, &c)));
        assert_eq!(options[0], [2.0, 0.2]);
    }

    #[test]
    fn a_river_point_is_the_middle_vertex_of_its_longest_path() {
        let paths: Vec<Vec<Pt>> = vec![
            vec![[0.0, 40.0], [0.1, 40.0]],
            vec![[1.0, 40.0], [1.1, 40.0], [1.2, 40.0], [1.3, 40.0]],
        ];
        assert_eq!(river_point(&paths), Some([1.2, 40.0]));
        assert_eq!(river_point(&[]), None);
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
        let Resolution::Placed(k) = c.resolve("Westchester", "Kensico Reservoir") else {
            panic!("Kensico Reservoir is placed")
        };
        assert_eq!(k.source, "JLOQT");
        assert_eq!(k.kind, Kind::Lake);
        assert!((41.05..41.14).contains(&k.latitude) && (-73.78..-73.7).contains(&k.longitude));
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
