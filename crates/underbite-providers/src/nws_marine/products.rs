//! NWS marine text products for Great Lakes and offshore zones.
//!
//! These zones have no forecast endpoint. Their forecasts are bulletins in
//! `/products/types/{type}/locations/{location}/latest`: `NSH` (nearshore, one per office),
//! `GLF` (open lake, one per lake) and `OFF` (offshore, one per area). A bulletin holds one
//! section per group of zones; a section starts at a UGC line, ends at `$$` and carries dot
//! prefixed periods. The text is kept as the office wrote it, with hard wraps joined, and is
//! never read for numbers.

use serde::Deserialize;
use underbite_core::model::{ForecastPeriod, ZoneForecast, ZoneKind, ZoneProduct};
use underbite_core::time::{UnixTime, parse_rfc3339};

use crate::request::{Place, Request, Resource, Target, ZoneTarget};
use crate::source::{FetchContext, SourceError};

use underbite_core::model::Measurement;

/// Seconds a product may be reused (the API sets one minute on `latest`).
const PRODUCT_AGE_S: u32 = 60;

/// The product and location that answer a zone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProductRef {
    /// `Nsh`, `Glf` or `Off`.
    pub product: ZoneProduct,
    /// The location id in the products API: an office, a lake or an offshore area.
    pub location: String,
}

impl ProductRef {
    /// The product's type code in the URL.
    pub fn code(&self) -> &'static str {
        match self.product {
            ZoneProduct::Nsh => "NSH",
            ZoneProduct::Glf => "GLF",
            ZoneProduct::Off => "OFF",
            ZoneProduct::Cwf => "CWF",
            ZoneProduct::ZoneApi => "",
        }
    }

    /// `/products/types/{type}/locations/{location}/latest`.
    pub fn url(&self) -> String {
        format!(
            "{}/products/types/{}/locations/{}/latest",
            super::zones::BASE,
            self.code(),
            self.location
        )
    }
}

/// The zone's prefix letters and number: `LEZ146` is `("LEZ", 146)`.
fn split_id(id: &str) -> Option<(&str, u32)> {
    let prefix = id.get(..3)?;
    let number = id.get(3..)?;
    if number.len() != 3 || !prefix.bytes().all(|b| b.is_ascii_uppercase()) {
        return None;
    }
    Some((prefix, number.parse().ok()?))
}

/// The text product that holds the zone's forecast, or why none is known.
///
/// Verified for Lake Erie and Lake Ontario (nearshore `NSH` 142 to 149 of the zone's office, open
/// lake `GLF` 160 to 169 and 061) and for the Atlantic offshore zones (`NT1` and `NT2`). Other
/// lakes and ranges are unverified and say so rather than guess.
pub fn product_for(zone: &ZoneTarget) -> Result<ProductRef, SourceError> {
    let unknown = || {
        SourceError::Unsupported(format!(
            "no known text product for zone {}; see the National Weather Service forecast",
            zone.id
        ))
    };
    let (prefix, number) = split_id(&zone.id).ok_or_else(unknown)?;
    let (product, location) = match (zone.kind, prefix) {
        (ZoneKind::GreatLakesNearshore, "LEZ") if (142..=149).contains(&number) => {
            (ZoneProduct::Nsh, zone.office.clone().ok_or_else(unknown)?)
        }
        (ZoneKind::GreatLakesOpen, "LEZ") if (160..=169).contains(&number) || number == 61 => {
            (ZoneProduct::Glf, "LE".to_string())
        }
        (ZoneKind::GreatLakesOpen, "LOZ") if (61..=65).contains(&number) => {
            (ZoneProduct::Glf, "LO".to_string())
        }
        (ZoneKind::Offshore, "ANZ") if (800..=815).contains(&number) || number == 900 => {
            (ZoneProduct::Off, "NT1".to_string())
        }
        (ZoneKind::Offshore, "ANZ")
            if (820..=835).contains(&number) || (905..=935).contains(&number) =>
        {
            (ZoneProduct::Off, "NT2".to_string())
        }
        _ => return Err(unknown()),
    };
    Ok(ProductRef { product, location })
}

/// The follow-ups that read the text product of a zone: none for a zone with no known product.
pub fn requests(zone: &ZoneTarget, target: &Target, ctx: &FetchContext) -> Vec<Request> {
    let Ok(product) = product_for(zone) else {
        return Vec::new();
    };
    let target = Target {
        place: Place::Zone(zone.clone()),
        measurements: [Measurement::MarineForecast].into(),
        ..target.clone()
    };
    vec![
        super::get(
            product.url(),
            Resource::NwsProduct,
            target,
            super::LD_JSON,
            ctx,
            PRODUCT_AGE_S,
        )
        .optional(),
    ]
}

#[derive(Deserialize)]
struct ProductBody {
    #[serde(rename = "issuanceTime")]
    issuance_time: String,
    #[serde(rename = "productText")]
    text: String,
}

/// One section of a bulletin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Section {
    /// Every zone id the UGC line names, ranges expanded.
    pub zones: Vec<String>,
    /// When the section expires.
    pub expires: UnixTime,
    /// The periods, with any closing paragraphs on the last one.
    pub periods: Vec<ForecastPeriod>,
    /// Whether the section is a coded MAFOR block rather than a forecast.
    pub coded: bool,
}

/// Expands a UGC zone list (`LEZ142-143`, `LEZ146>148`, `LEZ061-169`) and returns the ids and the
/// `DDHHMM` expiry token. `ugc` is the whole line, wraps joined, with its closing `-`.
pub fn expand_ugc(ugc: &str) -> Option<(Vec<String>, &str)> {
    let mut tokens: Vec<&str> = ugc.trim().trim_end_matches('-').split('-').collect();
    let expiry = tokens.pop()?;
    if expiry.len() != 6 || !expiry.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let mut zones = Vec::new();
    let mut prefix = "";
    for token in tokens {
        let (first, last) = match token.split_once('>') {
            Some((a, b)) => (a, Some(b)),
            None => (token, None),
        };
        let digits_at = first.find(|c: char| c.is_ascii_digit())?;
        if digits_at > 0 {
            prefix = &first[..digits_at];
        } else if prefix.is_empty() {
            return None;
        }
        let from: u32 = first[digits_at..].parse().ok()?;
        let to: u32 = match last {
            Some(b) => b
                .trim_start_matches(|c: char| c.is_ascii_uppercase())
                .parse()
                .ok()?,
            None => from,
        };
        if to < from || to - from > 99 {
            return None;
        }
        zones.extend((from..=to).map(|n| format!("{prefix}{n:03}")));
    }
    (!zones.is_empty()).then_some((zones, expiry))
}

/// Whether a line begins a UGC line (`LEZ146>148-`, `ANZ330-335-`).
fn starts_ugc(line: &str) -> bool {
    let b = line.as_bytes();
    b.len() >= 7
        && b[..2].iter().all(u8::is_ascii_uppercase)
        && matches!(b[2], b'Z' | b'C')
        && b[3..6].iter().all(u8::is_ascii_digit)
        && matches!(b[6], b'-' | b'>')
}

/// Whether a line ends a UGC line: `-DDHHMM-`.
fn ends_ugc(line: &str) -> bool {
    let b = line.trim_end().as_bytes();
    b.len() >= 8
        && b[b.len() - 1] == b'-'
        && b[b.len() - 7..b.len() - 1].iter().all(u8::is_ascii_digit)
        && b[b.len() - 8] == b'-'
}

/// Whether a line is the local issue time (`236 AM EST Sun Nov 8 2026`).
fn is_issue_time(line: &str) -> bool {
    let mut words = line.split_whitespace();
    let (Some(clock), Some(half)) = (words.next(), words.next()) else {
        return false;
    };
    (3..=4).contains(&clock.len())
        && clock.bytes().all(|b| b.is_ascii_digit())
        && matches!(half, "AM" | "PM")
}

/// `.TONIGHT...text` as (label, rest of the line).
fn period_start(line: &str) -> Option<(&str, &str)> {
    let body = line.strip_prefix('.')?;
    let end = body.find("...")?;
    let label = &body[..end];
    let valid = !label.is_empty()
        && label
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b' ');
    // `....` after a label is the label's dots plus a period of text.
    valid.then(|| (label, body[end + 3..].trim_start_matches('.')))
}

/// `MONDAY NIGHT` as `Monday Night`.
fn title_case(label: &str) -> String {
    label
        .split(' ')
        .map(|word| {
            let mut chars = word.chars();
            chars
                .next()
                .map(|c| c.to_string() + &chars.as_str().to_lowercase())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The expiry of a `DDHHMM` token: the first such time (UTC) in the issuance month or the next.
fn expiry_time(token: &str, issuance: &str) -> Option<UnixTime> {
    if !(issuance.ends_with("+00:00") || issuance.ends_with('Z')) {
        return None;
    }
    let year: u32 = issuance.get(0..4)?.parse().ok()?;
    let month: u32 = issuance.get(5..7)?.parse().ok()?;
    let issued = parse_rfc3339(issuance)?;
    let at = |year: u32, month: u32| {
        parse_rfc3339(&format!(
            "{year:04}-{month:02}-{}T{}:{}:00+00:00",
            &token[0..2],
            &token[2..4],
            &token[4..6]
        ))
    };
    let (next_year, next_month) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    [at(year, month), at(next_year, next_month)]
        .into_iter()
        .flatten()
        .find(|t| t.0 >= issued.0 - 86_400)
}

/// Splits a bulletin into its sections.
pub fn sections(text: &str, issuance: &str) -> Result<Vec<Section>, SourceError> {
    let lines: Vec<&str> = text.lines().map(str::trim_end).collect();
    let mut sections = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if !starts_ugc(lines[i]) {
            i += 1;
            continue;
        }
        let mut ugc = lines[i].to_string();
        while !ends_ugc(&ugc) && i + 1 < lines.len() && !lines[i + 1].is_empty() {
            i += 1;
            ugc.push_str(lines[i]);
        }
        i += 1;
        let end = lines[i..]
            .iter()
            .position(|l| l.trim() == "$$")
            .map_or(lines.len(), |p| i + p);
        let body = &lines[i..end];
        i = end + 1;
        let Some((zones, token)) = expand_ugc(&ugc) else {
            continue;
        };
        let expires = expiry_time(token, issuance)
            .ok_or_else(|| SourceError::Parse(format!("product expiry {token:?}")))?;
        sections.push(read_section(zones, expires, body));
    }
    Ok(sections)
}

/// Reads a section's lines after its UGC line.
fn read_section(zones: Vec<String>, expires: UnixTime, body: &[&str]) -> Section {
    let first = body.iter().position(|l| is_issue_time(l));
    let rest = first.map_or(body, |p| &body[p + 1..]);
    let coded = rest.iter().any(|l| l.trim_start().starts_with("MAFOR"))
        || body.iter().any(|l| l.trim_start().starts_with("MAFOR"));
    let mut periods: Vec<(String, String)> = Vec::new();
    let mut open = false;
    let mut closing: Vec<String> = Vec::new();
    let mut headline = false;
    for line in rest {
        let line = line.trim();
        if let Some((label, text)) = period_start(line) {
            periods.push((title_case(label), text.trim().to_string()));
            open = true;
            headline = false;
        } else if line.is_empty() {
            open = false;
            headline = false;
        } else if line.starts_with("...") && periods.is_empty() {
            // An advisory headline: the alerts carry it.
            headline = !line[3..].contains("...");
        } else if headline {
            headline = !line.contains("...");
        } else if open {
            if let Some((_, text)) = periods.last_mut() {
                if !text.is_empty() {
                    text.push(' ');
                }
                text.push_str(line);
            }
        } else if !periods.is_empty() {
            // A closing paragraph (water temperatures, a pointer to the open lake forecast).
            match closing.last_mut() {
                Some(last) if !last.ends_with(['.', '!', '?']) => {
                    last.push(' ');
                    last.push_str(line);
                }
                _ => closing.push(line.to_string()),
            }
        }
    }
    if let (Some((_, text)), false) = (periods.last_mut(), closing.is_empty()) {
        for paragraph in &closing {
            text.push('\n');
            text.push_str(paragraph);
        }
    }
    let periods = periods
        .into_iter()
        .filter(|(name, _)| !name.starts_with("Synopsis"))
        .map(|(name, text)| ForecastPeriod { name, text })
        .collect();
    Section {
        zones,
        expires,
        periods,
        coded,
    }
}

/// The forecast of `zone` from a product body.
pub fn parse_forecast(
    body: &[u8],
    zone: &ZoneTarget,
    now: UnixTime,
) -> Result<ZoneForecast, SourceError> {
    let product = product_for(zone)?;
    let parsed: ProductBody = serde_json::from_slice(body)
        .map_err(|e| SourceError::Parse(format!("text product: {e}")))?;
    let issued = parse_rfc3339(&parsed.issuance_time)
        .ok_or_else(|| SourceError::Parse(format!("product time {:?}", parsed.issuance_time)))?;
    let sections = sections(&parsed.text, &parsed.issuance_time)?;
    let section = sections
        .iter()
        .find(|s| s.zones.contains(&zone.id))
        .ok_or_else(|| {
            SourceError::NoData(Some(format!(
                "zone {} is not in the {} bulletin",
                zone.id,
                product.code()
            )))
        })?;
    if section.coded && section.periods.is_empty() {
        return Err(SourceError::Unsupported(format!(
            "zone {} has only a coded (MAFOR) forecast",
            zone.id
        )));
    }
    if section.periods.is_empty() {
        return Err(SourceError::NoData(Some("section has no periods".into())));
    }
    if section.expires.0 < now.0 {
        return Err(SourceError::Stale { latest: issued });
    }
    Ok(ZoneForecast {
        id: zone.id.clone(),
        name: zone.name.clone(),
        kind: zone.kind,
        issued,
        product: product.product,
        periods: section.periods.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::{Coordinates, Window};
    use crate::testutil::fixture_bytes;

    /// Just after the NSH bulletin was issued.
    const NSH_NOW: UnixTime = UnixTime(1_794_124_800);
    /// 2026-11-15 20:00 UTC.
    const GLF_NOW: UnixTime = UnixTime(1_794_772_800);

    fn zone(id: &str, name: &str, kind: ZoneKind, office: Option<&str>) -> ZoneTarget {
        ZoneTarget {
            id: id.into(),
            name: name.into(),
            kind,
            office: office.map(Into::into),
        }
    }

    fn forecast(file: &str, zone: &ZoneTarget, now: UnixTime) -> Result<ZoneForecast, SourceError> {
        parse_forecast(&fixture_bytes("nws-marine", file), zone, now)
    }

    fn target() -> Target {
        let ctx = FetchContext::new(NSH_NOW, "underbite-test/0");
        Target {
            place: Place::Point(ctx.round(Coordinates {
                latitude: 41.55,
                longitude: -81.72,
            })),
            measurements: [Measurement::MarineForecast, Measurement::MarineAlerts].into(),
            window: Window {
                from: NSH_NOW,
                to: NSH_NOW,
            },
            datum: None,
        }
    }

    #[test]
    fn nws_products_ugc_lines_expand() {
        let ids = |s: &str| expand_ugc(s).map(|(z, e)| (z, e.to_string()));
        assert_eq!(
            ids("LEZ146>148-081515-"),
            Some((
                vec!["LEZ146".into(), "LEZ147".into(), "LEZ148".into()],
                "081515".into()
            ))
        );
        assert_eq!(ids("LEZ061-169-160315-").unwrap().0, ["LEZ061", "LEZ169"]);
        assert_eq!(ids("ANZ338-081930-").unwrap().0, ["ANZ338"]);
        // Wrapped and mixed prefixes.
        assert_eq!(
            ids("LEZ142-143-LOZ044>045-081515-").unwrap().0,
            ["LEZ142", "LEZ143", "LOZ044", "LOZ045"]
        );
        assert_eq!(ids("LEZ146>148-"), None);
        assert_eq!(ids("LEZ148>146-081515-"), None);
    }

    #[test]
    fn nws_products_multi_line_ugc_lines_join() {
        let text = "FZUS51 KCLE 080736\n\nANZ330-331-332-333-334-335-336-\n337-338-339-081930-\nNames-\n157 AM EST Sun Nov 8 2026\n\n.TODAY...W winds\n15 kt. \n\n$$\n";
        let sections = sections(text, "2026-11-08T07:36:00+00:00").unwrap();
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].zones.len(), 10);
        assert_eq!(sections[0].periods[0].text, "W winds 15 kt.");
    }

    #[test]
    fn nws_products_expiry_month_comes_from_the_issue_time() {
        let t = expiry_time("010300", "2026-11-30T21:00:00+00:00").unwrap();
        assert_eq!(Some(t), parse_rfc3339("2026-12-01T03:00:00+00:00"));
        let t = expiry_time("010300", "2026-12-31T21:00:00+00:00").unwrap();
        assert_eq!(Some(t), parse_rfc3339("2027-01-01T03:00:00+00:00"));
        assert!(expiry_time("010300", "2026-11-30T21:00:00-05:00").is_none());
    }

    #[test]
    fn nws_products_nsh_cle_serves_lez146_with_its_shared_section() {
        let z = zone(
            "LEZ146",
            "Avon Point to Willowick OH",
            ZoneKind::GreatLakesNearshore,
            Some("CLE"),
        );
        let f = forecast("products-nsh-cle-latest.json", &z, NSH_NOW).unwrap();
        assert_eq!(f.product, ZoneProduct::Nsh);
        assert_eq!(f.periods.len(), 4);
        assert_eq!(f.periods[0].name, "Today");
        assert!(
            f.periods[0]
                .text
                .starts_with("Southwest winds 15 to 25 knots becoming west to 30 knots. Sunny.")
        );
        assert!(
            f.periods[3]
                .text
                .contains("The water temperature off Toledo is 58 degrees")
        );
        insta::assert_json_snapshot!("nsh_cle_lez146", f);
        // LEZ147 and LEZ148 share the section; LEZ142 and LEZ143 have their own.
        for id in ["LEZ147", "LEZ148"] {
            let other = zone(id, "x", ZoneKind::GreatLakesNearshore, Some("CLE"));
            let g = forecast("products-nsh-cle-latest.json", &other, NSH_NOW).unwrap();
            assert_eq!(g.periods, f.periods, "{id}");
        }
        let two = zone("LEZ143", "x", ZoneKind::GreatLakesNearshore, Some("CLE"));
        let g = forecast("products-nsh-cle-latest.json", &two, NSH_NOW).unwrap();
        assert_ne!(g.periods, f.periods);
        assert!(
            g.periods[0]
                .text
                .contains("Waves 1 to 3 feet building to 2 to 4 feet.")
        );
    }

    #[test]
    fn nws_products_glf_le_serves_lez166_from_the_range_block() {
        let z = zone(
            "LEZ166",
            "Avon Point to Willowick OH beyond 5 nm",
            ZoneKind::GreatLakesOpen,
            Some("CLE"),
        );
        let f = forecast("products-glf-le-latest.json", &z, GLF_NOW).unwrap();
        assert_eq!(f.product, ZoneProduct::Glf);
        assert_eq!(f.periods.len(), 8);
        assert_eq!(f.periods[0].name, "Tonight");
        assert_eq!(f.periods[1].name, "Monday");
        assert_eq!(f.periods[2].name, "Monday Night");
        assert_eq!(
            f.periods[0].text,
            "Northwest winds 5 to 15 knots. Waves 1 to 3 feet."
        );
        assert_eq!(
            f.periods[7].text,
            "West winds 5 to 15 knots becoming south 10 knots or less. Waves 2 to 4 feet subsiding to 1 foot or less."
        );
        insta::assert_json_snapshot!("glf_le_lez166", f);
        // LEZ061 is Buffalo's zone inside the same Lake Erie bulletin.
        let buf = zone("LEZ061", "x", ZoneKind::GreatLakesOpen, Some("BUF"));
        let g = forecast("products-glf-le-latest.json", &buf, GLF_NOW).unwrap();
        assert_eq!(
            g.periods[0].text,
            "Northwest winds 10 to 15 knots. Waves 1 to 2 feet."
        );
    }

    #[test]
    fn nws_products_the_mafor_block_is_not_a_forecast() {
        let z = zone("LEZ161", "x", ZoneKind::GreatLakesOpen, Some("CLE"));
        // LEZ161 is outside the open-lake range of a forecast product.
        let err = forecast("products-glf-le-latest.json", &z, GLF_NOW).unwrap_err();
        assert!(
            matches!(err, SourceError::Unsupported(m) if m.contains("MAFOR")),
            "mafor"
        );
    }

    #[test]
    fn nws_products_glf_lo_serves_lake_ontario() {
        let z = zone("LOZ064", "x", ZoneKind::GreatLakesOpen, Some("BUF"));
        let f = forecast("products-glf-lo-latest.json", &z, GLF_NOW).unwrap();
        assert_eq!(f.periods.len(), 8);
        assert_eq!(f.periods[3].name, "Tuesday");
        assert_eq!(
            f.periods[3].text,
            "West winds 15 to 20 knots. Waves 3 to 6 feet."
        );
        let z = zone("LOZ062", "x", ZoneKind::GreatLakesOpen, Some("BUF"));
        let g = forecast("products-glf-lo-latest.json", &z, GLF_NOW).unwrap();
        assert_ne!(f.periods, g.periods);
        let coded = zone("LOZ061", "x", ZoneKind::GreatLakesOpen, Some("BUF"));
        assert!(matches!(
            forecast("products-glf-lo-latest.json", &coded, GLF_NOW),
            Err(SourceError::Unsupported(_))
        ));
    }

    #[test]
    fn nws_products_off_nt2_serves_offshore_zones() {
        let z = zone(
            "ANZ820",
            "Hudson Canyon to Baltimore Canyon to 1000 FM",
            ZoneKind::Offshore,
            Some("ONA"),
        );
        let now = NSH_NOW;
        let f = forecast("products-off-nt2-latest.json", &z, now).unwrap();
        assert_eq!(f.product, ZoneProduct::Off);
        assert_eq!(f.periods.len(), 10);
        assert_eq!(f.periods[3].name, "Mon Night");
        assert_eq!(
            f.periods[0].text,
            "W to NW winds 20 to 30 kt, becoming W 15 to 20 kt. Seas 6 to 10 ft."
        );
        insta::assert_json_snapshot!("off_nt2_anz820", f);
        // The synopsis section is not a zone forecast.
        let body = fixture_bytes("nws-marine", "products-off-nt2-latest.json");
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let all = sections(
            parsed["productText"].as_str().unwrap(),
            parsed["issuanceTime"].as_str().unwrap(),
        )
        .unwrap();
        assert_eq!(all.len(), 14);
        let synopsis = all.iter().find(|s| s.zones == ["ANZ899"]).unwrap();
        assert_eq!(synopsis.periods.len(), 0);
    }

    #[test]
    fn nws_products_expired_sections_are_stale() {
        let z = zone("LEZ146", "x", ZoneKind::GreatLakesNearshore, Some("CLE"));
        // After 2026-11-08 15:15 UTC.
        let err =
            forecast("products-nsh-cle-latest.json", &z, UnixTime(1_794_153_600)).unwrap_err();
        assert!(matches!(err, SourceError::Stale { .. }), "{err:?}");
    }

    #[test]
    fn nws_products_zone_to_product_rule() {
        let nsh = zone("LEZ146", "x", ZoneKind::GreatLakesNearshore, Some("CLE"));
        assert_eq!(
            product_for(&nsh).unwrap().url(),
            "https://api.weather.gov/products/types/NSH/locations/CLE/latest"
        );
        let glf = zone("LEZ166", "x", ZoneKind::GreatLakesOpen, Some("CLE"));
        assert_eq!(
            product_for(&glf).unwrap().url(),
            "https://api.weather.gov/products/types/GLF/locations/LE/latest"
        );
        let glf = zone("LOZ063", "x", ZoneKind::GreatLakesOpen, Some("BUF"));
        assert_eq!(product_for(&glf).unwrap().location, "LO");
        let off = zone("ANZ815", "x", ZoneKind::Offshore, Some("ONA"));
        assert_eq!(
            product_for(&off).unwrap().url(),
            "https://api.weather.gov/products/types/OFF/locations/NT1/latest"
        );
        let off = zone("ANZ920", "x", ZoneKind::Offshore, Some("ONA"));
        assert_eq!(product_for(&off).unwrap().location, "NT2");
        // Unverified lakes, ranges and areas are unsupported, with a message.
        for (id, kind) in [
            ("LHZ361", ZoneKind::GreatLakesOpen),
            ("LEZ444", ZoneKind::GreatLakesNearshore),
            ("LOZ043", ZoneKind::GreatLakesNearshore),
            ("PZZ810", ZoneKind::Offshore),
            ("ANZ338", ZoneKind::Coastal),
        ] {
            let err = product_for(&zone(id, "x", kind, Some("XXX"))).unwrap_err();
            assert!(
                matches!(&err, SourceError::Unsupported(m) if m.contains(id)),
                "{id}"
            );
        }
        let no_office = zone("LEZ146", "x", ZoneKind::GreatLakesNearshore, None);
        assert!(product_for(&no_office).is_err());
    }

    #[test]
    fn nws_products_requests_carry_the_zone() {
        let ctx = FetchContext::new(NSH_NOW, "underbite-test/0");
        let z = zone("LEZ146", "x", ZoneKind::GreatLakesNearshore, Some("CLE"));
        let r = requests(&z, &target(), &ctx);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].resource, Resource::NwsProduct);
        assert!(!r[0].required);
        assert_eq!(r[0].cache.max_age, Some(60));
        assert_eq!(
            r[0].target.measurements,
            [Measurement::MarineForecast].into()
        );
        assert!(matches!(&r[0].target.place, Place::Zone(t) if t.id == "LEZ146"));
        let unknown = zone("LHZ361", "x", ZoneKind::GreatLakesOpen, None);
        assert_eq!(requests(&unknown, &target(), &ctx).len(), 0);
    }

    #[test]
    fn nws_products_nt1_serves_anz815() {
        // A trimmed bulletin in the recorded layout (NT1 was observed, not recorded).
        let body = serde_json::json!({
            "issuanceTime": "2026-11-15T20:25:00+00:00",
            "productText": "\n000\nFZNT21 KWBC 152025\nOFFNT1\n\nOffshore Waters Forecast\n\nANZ815-160745-\nSouth of Long Island-\n325 PM EST Sun Nov 15 2026\n\n.TONIGHT...W winds 10 to 15 kt. Seas 3 to 4\nft. \n.MON...NW winds 10 kt. Seas 2 to 3 ft. \n\n$$\n"
        });
        let z = zone(
            "ANZ815",
            "South of Long Island",
            ZoneKind::Offshore,
            Some("ONA"),
        );
        let f = parse_forecast(body.to_string().as_bytes(), &z, GLF_NOW).unwrap();
        assert_eq!(f.periods.len(), 2);
        assert_eq!(f.periods[0].text, "W winds 10 to 15 kt. Seas 3 to 4 ft.");
        // The NT2 bulletin does not hold it.
        let err = forecast("products-off-nt2-latest.json", &z, NSH_NOW).unwrap_err();
        assert!(matches!(err, SourceError::NoData(_)));
    }
}
