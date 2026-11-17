//! Links to the official fishing regulations and licence pages of each region: links only, never limits or seasons as data.
//!
//! The catalogue is one CSV file per country under `data/regulations/`, loaded with `include_str!`.
//! Regions are ISO 3166-2 codes (`US-NY`). Federal waters are linked through the state's NOAA
//! Fisheries region. Every link carries the date it was last checked, for display.

use std::sync::OnceLock;

use serde::Deserialize;

/// Text shown beside regulation links: the app links to the official pages and states no limit itself.
/// Safety-adjacent wording, never in Luna's voice. Intended for a `SafetyKey` once the view takes it.
pub const REGULATIONS_DISCLAIMER: &str = "Fishing regulations change during the year. \
Underbite does not state limits, seasons or licence rules; open the official page and check it \
before you fish.";

/// The water a link is wanted for. The Great Lakes count as fresh water.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Water {
    Fresh,
    Salt,
}

/// What a link leads to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegKind {
    /// Freshwater regulations of the jurisdiction.
    Freshwater,
    /// Saltwater (marine) regulations of the jurisdiction.
    Saltwater,
    /// Where to buy or look up the licence.
    Licence,
    /// Federal-waters regulations of a NOAA Fisheries region.
    Federal,
}

/// One link to an official page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegLink {
    pub kind: RegKind,
    /// Display label, the agency's name.
    pub label: String,
    /// Always `https://`.
    pub url: String,
    /// The date the link was last checked, `YYYY-MM-DD`.
    pub checked: String,
    /// False when the page could not be confirmed when checked; open it in a browser view.
    pub verified: bool,
}

#[derive(Debug, Deserialize)]
struct Row {
    region: String,
    name: String,
    agency: String,
    noaa_region: String,
    kind: String,
    url: String,
    verified: String,
    checked: String,
    // `coastal` and `great_lakes` columns are informational and ignored.
}

/// Reasons a catalogue file is rejected (a test failure; the shipped file never hits them).
#[derive(Debug, thiserror::Error)]
pub enum CatalogueError {
    #[error("malformed row: {0}")]
    Csv(#[from] csv::Error),
    #[error("row {region}: unknown kind {kind:?}")]
    Kind { region: String, kind: String },
    #[error("row {region}: url is not https")]
    Url { region: String },
}

#[derive(Debug)]
struct Entry {
    region: String,
    noaa_region: String,
    kind: RegKind,
    link: RegLink,
}

fn parse(text: &str) -> Result<Vec<Entry>, CatalogueError> {
    let mut rdr = csv::ReaderBuilder::new().from_reader(text.as_bytes());
    let mut out = Vec::new();
    for row in rdr.deserialize::<Row>() {
        let r = row?;
        let kind = match r.kind.as_str() {
            "freshwater" => RegKind::Freshwater,
            "saltwater" => RegKind::Saltwater,
            "licence" => RegKind::Licence,
            "federal" => RegKind::Federal,
            _ => {
                return Err(CatalogueError::Kind {
                    region: r.region,
                    kind: r.kind,
                });
            }
        };
        if !r.url.starts_with("https://") {
            return Err(CatalogueError::Url { region: r.region });
        }
        let label = if kind == RegKind::Federal {
            r.name
        } else {
            r.agency
        };
        out.push(Entry {
            region: r.region,
            noaa_region: r.noaa_region,
            kind,
            link: RegLink {
                kind,
                label,
                url: r.url,
                checked: r.checked,
                verified: r.verified == "Y",
            },
        });
    }
    Ok(out)
}

fn catalogue() -> &'static [Entry] {
    static CATALOGUE: OnceLock<Vec<Entry>> = OnceLock::new();
    CATALOGUE.get_or_init(|| {
        // A malformed shipped file is caught by the tests; at run time it yields no links.
        parse(include_str!("../data/regulations/us.csv")).unwrap_or_default()
    })
}

fn push_unique(out: &mut Vec<RegLink>, link: &RegLink) {
    if !out.iter().any(|l| l.url == link.url) {
        out.push(link.clone());
    }
}

fn jurisdiction_links(region: &str, wanted: &[RegKind], federal: bool) -> Vec<RegLink> {
    let region = region.trim().to_ascii_uppercase();
    let cat = catalogue();
    let mut out = Vec::new();
    for kind in wanted {
        for e in cat.iter().filter(|e| e.region == region && e.kind == *kind) {
            push_unique(&mut out, &e.link);
        }
    }
    let noaa = cat
        .iter()
        .find(|e| e.region == region)
        .map_or("", |e| e.noaa_region.as_str());
    if federal && !noaa.is_empty() {
        for e in cat
            .iter()
            .filter(|e| e.kind == RegKind::Federal && e.noaa_region == noaa)
        {
            push_unique(&mut out, &e.link);
        }
        if matches!(noaa, "greater-atlantic" | "southeast") {
            for e in cat
                .iter()
                .filter(|e| e.kind == RegKind::Federal && e.noaa_region == "atlantic-hms")
            {
                push_unique(&mut out, &e.link);
            }
        }
    }
    out
}

/// The links for fishing `water` in `region`: the jurisdiction's regulations page for that water,
/// its licence page and, for salt water, the federal-waters pages of its NOAA Fisheries region.
/// An inland jurisdiction has no saltwater page: for `Water::Salt` it yields only the licence link.
/// An unknown region yields nothing.
pub fn links(region: &str, water: Water) -> Vec<RegLink> {
    match water {
        Water::Fresh => jurisdiction_links(region, &[RegKind::Freshwater, RegKind::Licence], false),
        Water::Salt => jurisdiction_links(region, &[RegKind::Saltwater, RegKind::Licence], true),
    }
}

/// Every link of `region`: freshwater, saltwater, licence and federal.
pub fn all_links(region: &str) -> Vec<RegLink> {
    jurisdiction_links(
        region,
        &[RegKind::Freshwater, RegKind::Saltwater, RegKind::Licence],
        true,
    )
}

/// The licence link of `region`.
pub fn licence_link(region: &str) -> Option<RegLink> {
    jurisdiction_links(region, &[RegKind::Licence], false)
        .into_iter()
        .next()
}

#[cfg(test)]
mod tests {
    use super::*;

    const US: &str = include_str!("../data/regulations/us.csv");
    const HEAD: &str =
        "region,name,agency,noaa_region,coastal,great_lakes,kind,url,verified,checked\n";

    fn licence_regions() -> Vec<String> {
        let mut v: Vec<String> = parse(US)
            .unwrap()
            .into_iter()
            .filter(|e| e.kind == RegKind::Licence)
            .map(|e| e.region)
            .collect();
        v.dedup();
        v
    }

    #[test]
    fn shipped_file_parses_with_https_urls() {
        let rows = parse(US).unwrap();
        assert!(rows.len() > 140);
        assert!(rows.iter().all(|e| e.link.url.starts_with("https://")));
        assert!(rows.iter().all(|e| e.link.checked.len() == 10));
    }

    #[test]
    fn malformed_rows_are_errors() {
        let bad_kind = format!("{HEAD}US-XX,X,X,,N,N,bogus,https://x.example,Y,2026-01-01\n");
        let bad_url = format!("{HEAD}US-XX,X,X,,N,N,licence,http://x.example,Y,2026-01-01\n");
        let short = format!("{HEAD}US-XX,X\n");
        assert!(parse(&bad_kind).is_err());
        assert!(parse(&bad_url).is_err());
        assert!(parse(&short).is_err());
    }

    #[test]
    fn every_region_has_a_licence_link() {
        let codes = licence_regions();
        assert_eq!(codes.len(), 56);
        for c in codes {
            assert!(licence_link(&c).is_some(), "{c}");
        }
        let regions: std::collections::BTreeSet<_> = parse(US)
            .unwrap()
            .into_iter()
            .filter(|e| e.kind != RegKind::Federal)
            .map(|e| e.region)
            .collect();
        assert_eq!(regions.len(), 56);
    }

    #[test]
    fn new_york_has_all_links() {
        let all = all_links("US-NY");
        let find = |k, needle: &str| all.iter().any(|l| l.kind == k && l.url.contains(needle));
        assert!(find(
            RegKind::Freshwater,
            "dec.ny.gov/things-to-do/freshwater-fishing"
        ));
        assert!(find(
            RegKind::Saltwater,
            "dec.ny.gov/things-to-do/saltwater-fishing"
        ));
        assert!(find(
            RegKind::Licence,
            "dec.ny.gov/regulatory/permits-licenses"
        ));
        assert!(find(RegKind::Federal, "new-england-mid-atlantic"));
        assert!(find(RegKind::Federal, "highly-migratory"));
        let salt = links("us-ny", Water::Salt);
        assert!(salt.iter().any(|l| l.url.contains("saltwater-fishing")));
        assert!(!salt.iter().any(|l| l.url.contains("freshwater-fishing")));
        let fresh = links("US-NY", Water::Fresh);
        assert_eq!(fresh.len(), 2);
        assert_eq!(
            fresh[0].label,
            "New York State Department of Environmental Conservation"
        );
        assert_eq!(fresh[0].checked, "2026-11-16");
    }

    #[test]
    fn inland_states_have_no_saltwater_page_or_federal_link() {
        let salt = links("US-CO", Water::Salt);
        assert_eq!(salt.len(), 1);
        assert!(salt.iter().all(|l| l.kind == RegKind::Licence));
        assert!(
            links("US-CO", Water::Fresh)
                .iter()
                .any(|l| l.kind == RegKind::Freshwater)
        );
        assert_eq!(links("XX-ZZ", Water::Fresh).len(), 0);
        assert!(licence_link("").is_none());
    }

    #[test]
    fn shared_page_is_listed_once() {
        let az = links("US-AZ", Water::Fresh);
        let mut urls: Vec<_> = az.iter().map(|l| &l.url).collect();
        urls.sort();
        urls.dedup();
        assert_eq!(urls.len(), az.len());
    }

    #[test]
    fn pacific_states_link_their_region_only() {
        let wa = links("US-WA", Water::Salt);
        assert!(wa.iter().any(|l| l.url.contains("west-coast")));
        assert!(!wa.iter().any(|l| l.url.contains("highly-migratory")));
    }
}
