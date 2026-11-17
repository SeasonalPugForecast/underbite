//! Harmful algal blooms: the EPA Cyanobacteria Assessment Network (satellite) cell counts and NOAA HAB forecasts.

pub mod cyan;
pub mod hab;

use underbite_core::model::{BloomInfo, ExternalLink, HabForecast};

use crate::request::Coordinates;

/// The bloom block of a snapshot from what each source delivered. A source that failed softly
/// (not a lake, stale composites, no beach in range) is simply left out: the block then says
/// nothing about it, and the failure is recorded as a source error, never as "no bloom". Links
/// are added without repeats. `None` when there is neither a reading, a forecast nor a link.
pub fn bloom_info(
    spot: Coordinates,
    cyan: Option<&cyan::CyanReading>,
    hab: Option<HabForecast>,
    links: impl IntoIterator<Item = ExternalLink>,
) -> Option<BloomInfo> {
    let mut info = cyan.map(|c| c.bloom_info(spot)).unwrap_or_default();
    info.hab = hab;
    for link in links {
        if !info.links.contains(&link) {
            info.links.push(link);
        }
    }
    let empty = info.cyan_image_date.is_none() && info.hab.is_none() && info.links.is_empty();
    (!empty).then_some(info)
}

#[cfg(test)]
mod tests {
    use super::*;
    use underbite_core::time::{UnixTime, parse_rfc3339};

    use crate::testutil::fixture_bytes;

    const SPOT: Coordinates = Coordinates {
        latitude: 27.448,
        longitude: -82.688,
    };

    #[test]
    fn combines_both_sources_and_dedups_links() {
        let now = parse_rfc3339("2026-11-17T09:00:00Z").expect("a time");
        let zip = fixture_bytes("hab", "noaa-hab-gulf-rif-currentmodelrun-2026-11-17.zip");
        let hab = hab::parse_gulf_zip(&zip, SPOT, now).expect("a forecast");
        let body = fixture_bytes("cyan", "cyan-oneida-lake-weekly-2026-11-17.json");
        let cyan = cyan::parse(&body, UnixTime(1_774_670_400 + 86_400)).expect("fresh");
        let links = hab::links_near(SPOT).into_iter().chain([cyan::info_link()]);
        let info = bloom_info(SPOT, Some(&cyan), Some(hab), links).expect("a block");
        assert!(info.hab.is_some());
        assert!(info.cyan_image_date.is_some());
        assert_eq!(info.links.len(), 2, "the CyAN link once, plus Florida's");
    }

    #[test]
    fn a_failed_source_leaves_the_block_out() {
        assert_eq!(bloom_info(SPOT, None, None, []), None);
        let only_links = bloom_info(SPOT, None, None, hab::links_near(SPOT)).expect("links");
        assert!(only_links.hab.is_none() && only_links.cyan_image_date.is_none());
        assert_eq!(only_links.links.len(), 1);
    }
}
