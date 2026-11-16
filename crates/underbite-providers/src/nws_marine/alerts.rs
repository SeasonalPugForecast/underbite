//! NWS active marine alerts for a zone.
//!
//! Always queried by zone: the unfiltered list is hundreds of kilobytes. Event names, headlines
//! and zone ids are copied as the NWS wrote them.

use serde::Deserialize;
use underbite_core::model::{AlertSeverity, MarineAlert};
use underbite_core::time::{UnixTime, parse_rfc3339};

use super::zones::BASE;
use crate::source::SourceError;

/// `/alerts/active?zone={id}`.
pub fn url(zone_id: &str) -> String {
    format!("{BASE}/alerts/active?zone={zone_id}")
}

#[derive(Deserialize)]
struct Collection {
    #[serde(default)]
    features: Vec<Feature>,
}

#[derive(Deserialize)]
struct Feature {
    properties: Properties,
}

#[derive(Deserialize)]
struct Properties {
    id: String,
    event: String,
    headline: Option<String>,
    description: Option<String>,
    instruction: Option<String>,
    onset: Option<String>,
    effective: Option<String>,
    ends: Option<String>,
    expires: Option<String>,
    severity: Option<String>,
    status: Option<String>,
    #[serde(rename = "messageType")]
    message_type: Option<String>,
    #[serde(default)]
    geocode: Geocode,
    #[serde(default)]
    parameters: Parameters,
}

#[derive(Default, Deserialize)]
struct Geocode {
    #[serde(rename = "UGC", default)]
    ugc: Vec<String>,
}

#[derive(Default, Deserialize)]
struct Parameters {
    #[serde(rename = "VTEC", default)]
    vtec: Vec<String>,
}

/// The event's identity across updates from its VTEC string
/// (`/O.EXT.KOKX.SC.Y.0100.…/` gives `KOKX.SC.Y.0100`): office, phenomenon, significance, number.
pub fn vtec_key(vtec: &str) -> Option<String> {
    let parts: Vec<&str> = vtec.trim().trim_matches('/').split('.').collect();
    (parts.len() >= 6 && parts[2..6].iter().all(|p| !p.is_empty())).then(|| parts[2..6].join("."))
}

fn severity(text: Option<&str>) -> AlertSeverity {
    match text {
        Some("Extreme") => AlertSeverity::Extreme,
        Some("Severe") => AlertSeverity::Severe,
        Some("Moderate") => AlertSeverity::Moderate,
        Some("Minor") => AlertSeverity::Minor,
        _ => AlertSeverity::Unknown,
    }
}

fn time(text: Option<&str>) -> Option<UnixTime> {
    text.and_then(parse_rfc3339)
}

/// NWS text is hard-wrapped at about 65 columns: a single line break inside a paragraph becomes a
/// space, paragraph breaks (two or more line breaks) stay, and nothing else changes.
fn unwrap_lines(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\n' {
            out.push(c);
            continue;
        }
        let mut run = 1;
        while chars.next_if_eq(&'\n').is_some() {
            run += 1;
        }
        if run == 1 {
            out.push(' ');
        } else {
            out.extend(std::iter::repeat_n('\n', run));
        }
    }
    out
}

/// The active alerts in a `/alerts/active` body: actual alerts only (not tests or exercises, not
/// cancellations), one per VTEC key (the first, with the zones of all its copies), in the order
/// the NWS listed them. An empty list is normal.
pub fn parse(body: &[u8]) -> Result<Vec<MarineAlert>, SourceError> {
    let collection: Collection =
        serde_json::from_slice(body).map_err(|e| SourceError::Parse(format!("alerts: {e}")))?;
    let mut alerts: Vec<MarineAlert> = Vec::new();
    for p in collection.features.into_iter().map(|f| f.properties) {
        if p.status.as_deref().is_some_and(|s| s != "Actual")
            || p.message_type.as_deref() == Some("Cancel")
        {
            continue;
        }
        let key = p.parameters.vtec.iter().find_map(|v| vtec_key(v));
        let duplicate = alerts.iter_mut().find(|a| match (&key, &a.vtec_key) {
            (Some(k), Some(other)) => k == other,
            _ => a.id == p.id,
        });
        if let Some(existing) = duplicate {
            for zone in p.geocode.ugc {
                if !existing.zones.contains(&zone) {
                    existing.zones.push(zone);
                }
            }
            continue;
        }
        alerts.push(MarineAlert {
            id: p.id,
            event: p.event,
            headline: p.headline,
            description: p.description.as_deref().map(unwrap_lines),
            instruction: p.instruction.as_deref().map(unwrap_lines),
            onset: time(p.onset.as_deref()).or(time(p.effective.as_deref())),
            ends: time(p.ends.as_deref()),
            expires: time(p.expires.as_deref()),
            severity: severity(p.severity.as_deref()),
            zones: p.geocode.ugc,
            vtec_key: key,
        });
    }
    Ok(alerts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::fixture_bytes;

    fn alerts(file: &str) -> Vec<MarineAlert> {
        parse(&fixture_bytes("nws-marine", file)).unwrap()
    }

    #[test]
    fn nws_alerts_url_filters_by_zone() {
        assert_eq!(
            url("ANZ338"),
            "https://api.weather.gov/alerts/active?zone=ANZ338"
        );
    }

    #[test]
    fn nws_alerts_vtec_key_is_office_phenomenon_significance_number() {
        assert_eq!(
            vtec_key("/O.EXT.KOKX.SC.Y.0100.000000T0000Z-261109T2300Z/").as_deref(),
            Some("KOKX.SC.Y.0100")
        );
        assert_eq!(vtec_key("/O.EXT.KOKX/"), None);
    }

    #[test]
    fn nws_alerts_small_craft_advisory_for_the_harbor() {
        let a = alerts("alerts-active-zone-ANZ338.geo.json");
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].event, "Small Craft Advisory");
        assert_eq!(a[0].vtec_key.as_deref(), Some("KOKX.SC.Y.0100"));
        assert_eq!(a[0].severity, AlertSeverity::Minor);
        assert_eq!(a[0].ends, parse_rfc3339("2026-11-09T18:00:00-05:00"));
        assert!(a[0].zones.contains(&"ANZ338".to_string()));
        insta::assert_json_snapshot!("alerts_anz338", a);
    }

    #[test]
    fn nws_alerts_lake_erie_and_gulf_gale() {
        let a = alerts("alerts-active-zone-LEZ146.geo.json");
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].event, "Small Craft Advisory");
        assert!(
            a[0].vtec_key
                .as_deref()
                .is_some_and(|k| k.starts_with("KCLE.SC.Y."))
        );
        let g = alerts("alerts-active-zone-GMZ250-gale-warning.geo.json");
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].event, "Gale Warning");
        assert_eq!(g[0].severity, AlertSeverity::Moderate);
        assert_eq!(g[0].vtec_key.as_deref(), Some("KCRP.GL.W.0007"));
        assert_eq!(g[0].zones.len(), 4);
        assert_eq!(g[0].expires, parse_rfc3339("2026-11-15T22:00:00-06:00"));
    }

    #[test]
    fn nws_alerts_none_is_normal() {
        let none = alerts("alerts-active-zone-ANZ338-event-gale-warning-empty.geo.json");
        assert_eq!(none.len(), 0);
    }

    #[test]
    fn nws_alerts_copies_of_one_event_merge_and_cancellations_drop() {
        let one = |id: &str, zone: &str, message: &str| {
            format!(
                r#"{{"id":"{id}","event":"Gale Warning","status":"Actual","messageType":"{message}",
                "geocode":{{"UGC":["{zone}"]}},"parameters":{{"VTEC":["/O.NEW.KCRP.GL.W.0007.1/"]}}}}"#
            )
        };
        let body = format!(
            r#"{{"features":[{{"properties":{}}},{{"properties":{}}},{{"properties":{}}}]}}"#,
            one("a", "GMZ250", "Alert"),
            one("b", "GMZ255", "Update"),
            one("c", "GMZ270", "Cancel"),
        );
        let a = parse(body.as_bytes()).unwrap();
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].id, "a");
        assert_eq!(a[0].zones, ["GMZ250", "GMZ255"]);
    }

    #[test]
    fn nws_alerts_description_and_instruction_keep_paragraphs_and_join_wrapped_lines() {
        let a = alerts("alerts-active-zone-ANZ338.geo.json");
        let description = a[0].description.as_deref().unwrap();
        assert!(description.starts_with(
            "* WHAT...West winds 10 to 20 kt with gusts up to 30 kt.\n\n* WHERE...Long Island Sound, New York Harbor, Peconic and Gardiners Bays"
        ));
        assert!(description.ends_with("* IMPACTS...Conditions will be hazardous to small craft."));
        assert_eq!(description.matches("\n\n").count(), 3);
        assert!(!description.contains("and\nGardiners"));
        assert_eq!(
            a[0].instruction.as_deref(),
            Some(
                "Inexperienced mariners, especially those operating smaller vessels, should avoid navigating in hazardous conditions."
            )
        );
        let g = alerts("alerts-active-zone-GMZ250-gale-warning.geo.json");
        // Spacing inside the text is the NWS's own.
        assert!(
            g[0].instruction
                .as_deref()
                .unwrap()
                .contains("hazardous conditions.  Remain in port")
        );
        insta::assert_json_snapshot!("alerts_gmz250", g);
    }

    #[test]
    fn nws_alerts_unwrap_lines_only_touches_single_breaks() {
        assert_eq!(unwrap_lines("a\nb\n\nc\n\n\nd "), "a b\n\nc\n\n\nd ");
    }

    #[test]
    fn alerts_decode_without_description_or_instruction() {
        let old = r#"{"id":"x","event":"Gale Warning","severity":"moderate"}"#;
        let a: MarineAlert = serde_json::from_str(old).unwrap();
        assert_eq!(a.description, None);
        assert_eq!(a.instruction, None);
        assert!(!serde_json::to_string(&a).unwrap().contains("description"));
    }
}
