//! Links to the official fishing regulations and licence pages of each region: links only, never limits or seasons as data.
//!
//! The catalogue is one CSV file per country under `data/regulations/`, loaded with `include_str!`.
//! Regions are ISO 3166-2 codes (`US-NY`). Federal waters are linked through the state's NOAA
//! Fisheries region. Every link carries the date it was last checked, for display.

use std::sync::OnceLock;

use serde::Deserialize;

use crate::view::strings::SafetyKey;

/// Text shown beside regulation links: the app links to the official pages and states no limit itself.
/// It is the view's safety text [`SafetyKey::RegulationsDisclaimer`], never in Luna's voice;
/// displays take it from the view, which also translates it.
pub const REGULATIONS_DISCLAIMER: &str = SafetyKey::RegulationsDisclaimer.text();

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

/// The Canadian file: `dfo_regions` lists the DFO regional pages (`;`-separated codes) that cover
/// the tidal waters of a province or territory; `language` is `en` or `fr`.
#[derive(Debug, Deserialize)]
struct CaRow {
    region: String,
    name: String,
    agency: String,
    dfo_regions: String,
    kind: String,
    language: String,
    url: String,
    verified: String,
    checked: String,
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
    /// DFO regions (`DFO-PAC`, ...) whose tidal pages cover this jurisdiction.
    dfo_regions: Vec<String>,
    /// `en` or `fr`.
    language: String,
    kind: RegKind,
    link: RegLink,
}

fn parse_kind(region: &str, kind: &str) -> Result<RegKind, CatalogueError> {
    match kind {
        "freshwater" => Ok(RegKind::Freshwater),
        "saltwater" => Ok(RegKind::Saltwater),
        "licence" => Ok(RegKind::Licence),
        "federal" => Ok(RegKind::Federal),
        _ => Err(CatalogueError::Kind {
            region: region.to_owned(),
            kind: kind.to_owned(),
        }),
    }
}

fn parse_ca(text: &str) -> Result<Vec<Entry>, CatalogueError> {
    let mut rdr = csv::ReaderBuilder::new().from_reader(text.as_bytes());
    let mut out = Vec::new();
    for row in rdr.deserialize::<CaRow>() {
        let r = row?;
        let kind = parse_kind(&r.region, &r.kind)?;
        if !r.url.starts_with("https://") {
            return Err(CatalogueError::Url { region: r.region });
        }
        let label = if r.region.starts_with("DFO-") {
            format!("{} ({})", r.agency, r.name)
        } else {
            r.agency
        };
        out.push(Entry {
            region: r.region,
            noaa_region: String::new(),
            dfo_regions: r
                .dfo_regions
                .split(';')
                .filter(|c| !c.is_empty())
                .map(str::to_owned)
                .collect(),
            language: r.language,
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

fn parse(text: &str) -> Result<Vec<Entry>, CatalogueError> {
    let mut rdr = csv::ReaderBuilder::new().from_reader(text.as_bytes());
    let mut out = Vec::new();
    for row in rdr.deserialize::<Row>() {
        let r = row?;
        let kind = parse_kind(&r.region, &r.kind)?;
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
            dfo_regions: Vec::new(),
            language: "en".to_owned(),
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
        let mut all = parse(include_str!("../data/regulations/us.csv")).unwrap_or_default();
        all.extend(parse_ca(include_str!("../data/regulations/ca.csv")).unwrap_or_default());
        all
    })
}

fn push_unique(out: &mut Vec<RegLink>, link: &RegLink) {
    if !out.iter().any(|l| l.url == link.url) {
        out.push(link.clone());
    }
}

/// The rows of `region` and `kind` in `lang`, or in English when the agency has no such page.
fn in_language<'a>(cat: &'a [Entry], region: &str, kind: RegKind, lang: &str) -> Vec<&'a Entry> {
    let rows: Vec<&Entry> = cat
        .iter()
        .filter(|e| e.region == region && e.kind == kind)
        .collect();
    let wanted: Vec<&Entry> = rows
        .iter()
        .copied()
        .filter(|e| e.language == lang)
        .collect();
    if wanted.is_empty() {
        rows.into_iter().filter(|e| e.language == "en").collect()
    } else {
        wanted
    }
}

fn jurisdiction_links(region: &str, wanted: &[RegKind], federal: bool, lang: &str) -> Vec<RegLink> {
    let region = region.trim().to_ascii_uppercase();
    let lang = lang.trim().to_ascii_lowercase();
    let cat = catalogue();
    let mut out = Vec::new();
    for kind in wanted {
        for e in in_language(cat, &region, *kind, &lang) {
            push_unique(&mut out, &e.link);
        }
    }
    let first = cat.iter().find(|e| e.region == region);
    let noaa = first.map_or("", |e| e.noaa_region.as_str());
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
    // Canadian tidal waters are federal: the DFO regional pages of the province or territory.
    if federal && let Some(first) = first {
        for code in &first.dfo_regions {
            for e in in_language(cat, code, RegKind::Saltwater, &lang) {
                push_unique(&mut out, &e.link);
            }
        }
    }
    out
}

/// Like [`links`], with the pages in `lang` (`"en"` or `"fr"`) where the agency has them and in
/// English where it does not. For salt water in Canada the links are DFO's regional pages only:
/// provincial licences do not cover tidal waters.
pub fn links_in(region: &str, water: Water, lang: &str) -> Vec<RegLink> {
    let canadian = region.trim().to_ascii_uppercase().starts_with("CA-");
    match water {
        Water::Fresh => jurisdiction_links(
            region,
            &[RegKind::Freshwater, RegKind::Licence],
            false,
            lang,
        ),
        Water::Salt if canadian => jurisdiction_links(region, &[RegKind::Saltwater], true, lang),
        Water::Salt => {
            jurisdiction_links(region, &[RegKind::Saltwater, RegKind::Licence], true, lang)
        }
    }
}

/// The links for fishing `water` in `region`: the jurisdiction's regulations page for that water,
/// its licence page and, for salt water, the federal-waters pages of its NOAA Fisheries region
/// (in Canada, DFO's regional pages). English pages; see [`links_in`].
/// An inland jurisdiction has no saltwater page: for `Water::Salt` it yields only the licence link.
/// An unknown region yields nothing.
pub fn links(region: &str, water: Water) -> Vec<RegLink> {
    links_in(region, water, "en")
}

/// Like [`all_links`], in `lang` with an English fallback.
pub fn all_links_in(region: &str, lang: &str) -> Vec<RegLink> {
    jurisdiction_links(
        region,
        &[RegKind::Freshwater, RegKind::Saltwater, RegKind::Licence],
        true,
        lang,
    )
}

/// Every link of `region`: freshwater, saltwater, licence and federal.
pub fn all_links(region: &str) -> Vec<RegLink> {
    all_links_in(region, "en")
}

/// The licence link of `region` in `lang`, English when the agency has no such page.
pub fn licence_link_in(region: &str, lang: &str) -> Option<RegLink> {
    jurisdiction_links(region, &[RegKind::Licence], false, lang)
        .into_iter()
        .next()
}

/// The licence link of `region`.
pub fn licence_link(region: &str) -> Option<RegLink> {
    licence_link_in(region, "en")
}

/// The links for several regions at once, without duplicates: border waters (Lake Erie, the
/// St. Lawrence) pass both the US state and the Canadian province.
pub fn links_for_regions(regions: &[&str], water: Water, lang: &str) -> Vec<RegLink> {
    let mut out = Vec::new();
    for r in regions {
        for l in links_in(r, water, lang) {
            push_unique(&mut out, &l);
        }
    }
    out
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

    const CA: &str = include_str!("../data/regulations/ca.csv");

    #[test]
    fn canadian_file_parses_and_covers_every_jurisdiction() {
        let rows = parse_ca(CA).unwrap();
        assert_eq!(rows.len(), 51);
        assert!(rows.iter().all(|e| e.link.url.starts_with("https://")));
        assert!(rows.iter().all(|e| e.link.checked.len() == 10));
        let provinces: std::collections::BTreeSet<_> = rows
            .iter()
            .filter(|e| e.region.starts_with("CA-"))
            .map(|e| e.region.clone())
            .collect();
        assert_eq!(provinces.len(), 13);
        for p in provinces {
            assert!(licence_link(&p).is_some(), "{p}");
            assert!(!links(&p, Water::Fresh).is_empty(), "{p}");
        }
        let bad = "region,name,agency,dfo_regions,kind,language,url,verified,checked\nCA-XX,X,X,,tidal,en,https://x.example,Y,2026-01-01\n";
        assert!(parse_ca(bad).is_err());
    }

    #[test]
    fn british_columbia_has_provincial_and_pacific_links() {
        let fresh = links("CA-BC", Water::Fresh);
        assert!(fresh.iter().any(|l| l.kind == RegKind::Freshwater));
        assert!(fresh.iter().any(|l| l.kind == RegKind::Licence));
        let salt = links("ca-bc", Water::Salt);
        assert_eq!(salt.len(), 1);
        assert!(salt[0].url.contains("pac.dfo-mpo.gc.ca"));
        let all = all_links("CA-BC");
        assert!(all.iter().any(|l| l.url.contains("gov.bc.ca")));
        assert!(all.iter().any(|l| l.url.contains("pac.dfo-mpo.gc.ca")));
    }

    #[test]
    fn french_pages_are_chosen_with_an_english_fallback() {
        let on = links_in("CA-ON", Water::Fresh, "fr");
        assert!(on.iter().all(|l| l.url.contains("/fr/")));
        assert_eq!(
            links_in("CA-BC", Water::Fresh, "fr"),
            links("CA-BC", Water::Fresh)
        );
        assert!(
            links_in("CA-BC", Water::Salt, "FR")[0]
                .url
                .ends_with("index-fra.html")
        );
        assert!(
            licence_link_in("CA-QC", "fr")
                .unwrap()
                .url
                .contains("peche-sportive")
        );
        assert_eq!(
            links_in("CA-ON", Water::Fresh, "de"),
            links("CA-ON", Water::Fresh)
        );
    }

    #[test]
    fn tidal_waters_map_to_dfo_regions() {
        let has =
            |r: &str, needle: &str| links(r, Water::Salt).iter().any(|l| l.url.contains(needle));
        assert!(has("CA-NS", "/maritimes/"));
        assert!(has("CA-NS", "/gulf/"));
        assert!(has("CA-PE", "/gulf/"));
        assert!(has("CA-NL", "/newfoundland-labrador/"));
        assert!(has("CA-QC", "/quebec/"));
        assert!(has("CA-NU", "/arctic-arctique/"));
        assert_eq!(links("CA-AB", Water::Salt).len(), 0);
        assert_eq!(links("DFO-PAC", Water::Salt).len(), 1);
    }

    #[test]
    fn northern_pages_are_unverified() {
        for r in ["CA-YT", "CA-NU"] {
            let fresh = links(r, Water::Fresh);
            assert!(fresh.iter().all(|l| !l.verified), "{r}");
        }
        assert!(links("CA-ON", Water::Fresh).iter().all(|l| l.verified));
    }

    #[test]
    fn border_waters_return_both_countries() {
        let both = links_for_regions(&["US-NY", "CA-ON"], Water::Fresh, "en");
        assert!(both.iter().any(|l| l.url.contains("dec.ny.gov")));
        assert!(both.iter().any(|l| l.url.contains("ontario.ca")));
    }

    #[test]
    fn the_disclaimer_is_the_views_safety_text_in_every_voice() {
        use crate::view::Voice;
        use crate::view::strings::Strings;
        for voice in [Voice::Luna, Voice::Plain] {
            let s = Strings::new(voice);
            assert_eq!(
                s.safety(SafetyKey::RegulationsDisclaimer),
                REGULATIONS_DISCLAIMER
            );
        }
        assert!(REGULATIONS_DISCLAIMER.contains("official page"));
        assert!(!REGULATIONS_DISCLAIMER.contains("Luna"));
    }
}
