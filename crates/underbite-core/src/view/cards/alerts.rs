//! Active marine alerts.

use schemars::JsonSchema;
use serde::Serialize;
use whether_core::model::Alert;
use whether_core::scales::Level;

use crate::model::{AlertSeverity, MarineAlert};
use crate::provenance::SourceId;
use crate::time::UnixTime;
use crate::view::format::Formatter;
use crate::view::strings::{Key, SafetyKey, source_name};
use crate::view::{CardChrome, CardId, CardState, Compact, CreditVm, Quantity, ViewInput};

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct AlertsCard {
    pub chrome: CardChrome,
    pub compact: Compact,
    pub full: AlertsFull,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AlertOrigin {
    Marine,
    Weather,
}

/// One alert, safety text exactly as issued and never in Luna's voice.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct AlertVm {
    pub id: String,
    pub origin: AlertOrigin,
    pub event: String,
    pub headline: Option<String>,
    /// The source's description, whole and as issued, for the full form.
    pub description: Option<String>,
    /// What the source tells the reader to do, whole and as issued.
    pub instruction: Option<String>,
    pub severity: AlertSeverity,
    pub severity_label: String,
    pub onset: Option<String>,
    pub ends: Option<String>,
    pub expires: Option<String>,
    /// Zone names or ids the alert covers.
    pub zones: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct AlertsFull {
    /// Marine and weather alerts de-duplicated, most severe first.
    pub alerts: Vec<AlertVm>,
}

/// One alert on its way to the card, with what de-duplication compares.
struct Entry {
    vm: AlertVm,
    vtec_key: Option<String>,
    onset: Option<UnixTime>,
}

/// `KOKX.SC.Y.0100`: office, phenomenon, significance and number, the event key the providers
/// take from a VTEC string.
fn is_vtec_key(id: &str) -> bool {
    let parts: Vec<&str> = id.split('.').collect();
    let upper = |s: &str, n: usize| s.len() == n && s.bytes().all(|b| b.is_ascii_uppercase());
    matches!(parts.as_slice(), [office, phenomenon, significance, number]
        if upper(office, 4)
            && upper(phenomenon, 2)
            && upper(significance, 1)
            && number.len() == 4
            && number.bytes().all(|b| b.is_ascii_digit()))
}

/// Two reports of one advisory: the same VTEC key when both have one, else the same event, onset
/// and zone (a report without zones matches any zone).
fn same_alert(a: &Entry, b: &Entry) -> bool {
    if let (Some(x), Some(y)) = (&a.vtec_key, &b.vtec_key) {
        return x == y;
    }
    a.vm.event.eq_ignore_ascii_case(&b.vm.event)
        && a.onset == b.onset
        && (a.vm.zones.is_empty()
            || b.vm.zones.is_empty()
            || a.vm.zones.iter().any(|z| b.vm.zones.contains(z)))
}

/// The credit text of the marine alerts when the snapshot carries none.
const CREDIT_MARINE_ALERTS: &str = "Marine alerts: NOAA National Weather Service";

/// How the alert source grades the alert: safety text, the same in every voice.
const fn severity_label(severity: AlertSeverity) -> &'static str {
    SafetyKey::severity(severity).text()
}

const fn severity_rank(severity: AlertSeverity) -> u8 {
    match severity {
        AlertSeverity::Extreme => 4,
        AlertSeverity::Severe => 3,
        AlertSeverity::Moderate => 2,
        AlertSeverity::Minor => 1,
        AlertSeverity::Unknown => 0,
    }
}

const fn weather_severity(level: Level) -> AlertSeverity {
    match level {
        Level::Extreme => AlertSeverity::Extreme,
        Level::VeryPoor | Level::Poor => AlertSeverity::Severe,
        Level::Moderate => AlertSeverity::Moderate,
        Level::Fair => AlertSeverity::Minor,
        Level::Good => AlertSeverity::Unknown,
    }
}

/// "Nov 8, 1:48 AM" in the spot's zone.
fn when(f: &Formatter, t: UnixTime) -> String {
    format!("{}, {}", f.date_of(t), f.clock(t))
}

fn marine_entry(f: &Formatter, a: &MarineAlert) -> Entry {
    Entry {
        vm: AlertVm {
            id: a.id.clone(),
            origin: AlertOrigin::Marine,
            event: a.event.clone(),
            headline: a.headline.clone(),
            description: a.description.clone(),
            instruction: a.instruction.clone(),
            severity: a.severity,
            severity_label: severity_label(a.severity).to_owned(),
            onset: a.onset.map(|t| when(f, t)),
            ends: a.ends.map(|t| when(f, t)),
            expires: a.expires.map(|t| when(f, t)),
            zones: a.zones.clone(),
        },
        vtec_key: a.vtec_key.clone(),
        onset: a.onset,
    }
}

fn weather_entry(f: &Formatter, a: &Alert) -> Entry {
    let severity = weather_severity(a.severity);
    Entry {
        vm: AlertVm {
            id: a.id.clone().unwrap_or_else(|| a.title.clone()),
            origin: AlertOrigin::Weather,
            event: a.title.clone(),
            headline: None,
            // The source's full text, as written.
            description: (!a.description.is_empty()).then(|| a.description.clone()),
            // whether's alerts carry no separate instruction.
            instruction: None,
            severity,
            severity_label: severity_label(severity).to_owned(),
            onset: Some(when(f, a.start)),
            ends: Some(when(f, a.end)),
            expires: None,
            zones: Vec::new(),
        },
        // whether's alert id is the VTEC event key when the alert has one.
        vtec_key: a.id.clone().filter(|id| is_vtec_key(id)),
        onset: Some(a.start),
    }
}

/// Marine alerts and the weather provider's alerts, each advisory once, still in effect.
/// The alert text is the source's, verbatim, whatever the voice.
#[allow(clippy::too_many_lines)]
pub fn build(input: &ViewInput) -> Option<AlertsCard> {
    let snapshot = &input.snapshot;
    let now = input.now;
    let f = Formatter::for_input(input);

    let marine = snapshot
        .marine
        .marine_alerts
        .iter()
        .filter(|a| a.ends.or(a.expires).is_none_or(|end| end > now))
        .map(|a| marine_entry(&f, a));
    let weather = snapshot
        .weather
        .iter()
        .flat_map(|w| &w.alerts)
        .filter(|a| a.end > now)
        .map(|a| weather_entry(&f, a));
    let mut entries: Vec<Entry> = Vec::new();
    // Marine first: its reports carry the zones and the onset.
    for entry in marine.chain(weather) {
        match entries.iter_mut().find(|e| same_alert(e, &entry)) {
            Some(existing) => {
                for zone in entry.vm.zones {
                    if !existing.vm.zones.contains(&zone) {
                        existing.vm.zones.push(zone);
                    }
                }
            }
            None => entries.push(entry),
        }
    }
    if entries.is_empty() {
        return None;
    }
    // Most severe first, then the newest onset.
    entries.sort_by(|a, b| {
        severity_rank(b.vm.severity)
            .cmp(&severity_rank(a.vm.severity))
            .then(b.onset.cmp(&a.onset))
    });
    let alerts: Vec<AlertVm> = entries.into_iter().map(|e| e.vm).collect();
    let top = &alerts[0];

    let mut credits = Vec::new();
    if !snapshot.marine.marine_alerts.is_empty() {
        let found = snapshot
            .attribution
            .iter()
            .find(|a| a.source == SourceId::NWS);
        credits.push(CreditVm {
            source: source_name(&SourceId::NWS),
            text: found.map_or_else(|| CREDIT_MARINE_ALERTS.to_owned(), |a| a.credit.clone()),
            link: found.and_then(|a| a.url.clone()),
            licence: None,
        });
    }
    if let Some(w) = snapshot.weather.as_ref().filter(|w| !w.alerts.is_empty()) {
        credits.push(CreditVm {
            source: w.provider.display_name().to_owned(),
            text: w.provider.attribution().to_owned(),
            link: None,
            licence: None,
        });
    }

    let title = f.strings.card_title(CardId::Alerts).to_owned();
    let count = alerts.len();
    let events: Vec<&str> = alerts.iter().map(|a| a.event.as_str()).collect();
    let accessibility = f.strings.fmt(
        Key::AlertsAcc,
        &[
            ("title", &title),
            ("count", &count.to_string()),
            ("events", &events.join(", ")),
        ],
    );
    let lines = [
        top.ends
            .as_ref()
            .or(top.expires.as_ref())
            .map(|t| f.strings.fmt(Key::UntilTime, &[("time", t)])),
        (count > 1).then(|| {
            f.strings
                .fmt(Key::MoreCount, &[("n", &(count - 1).to_string())])
        }),
    ]
    .into_iter()
    .flatten()
    .collect();
    Some(AlertsCard {
        chrome: CardChrome {
            id: CardId::Alerts,
            title,
            footnote: None,
            credits,
            state: CardState::Fresh,
            notices: Vec::new(),
            accessibility,
        },
        compact: Compact {
            headline: Quantity {
                value: None,
                text: top.event.clone(),
                unit: String::new(),
            },
            lines,
            spark: Vec::new(),
        },
        full: AlertsFull { alerts },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::parse_rfc3339;
    use crate::view::Voice;
    use crate::view::tests::sample_input;
    use whether_core::testdata::{SAMPLE_NOW, sample_alerts};

    fn at(iso: &str) -> UnixTime {
        parse_rfc3339(iso).unwrap()
    }

    /// The recorded ANZ338 alert, as the provider would parse it.
    fn anz338_alerts() -> Vec<MarineAlert> {
        recorded_alerts("alerts-active-zone-ANZ338.geo.json", AlertSeverity::Minor)
    }

    /// A recorded NWS alert file, with the NWS text joined the way the provider joins it.
    fn recorded_alerts(file: &str, severity: AlertSeverity) -> Vec<MarineAlert> {
        let path = format!(
            "{}/../../tests/fixtures/raw/nws-marine/{file}",
            env!("CARGO_MANIFEST_DIR")
        );
        let unwrap = |s: &str| {
            s.split("\n\n")
                .map(|para| para.replace('\n', " "))
                .collect::<Vec<_>>()
                .join("\n\n")
        };
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        json["features"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| {
                let p = &f["properties"];
                let time = |key: &str| p[key].as_str().and_then(parse_rfc3339);
                let vtec = p["parameters"]["VTEC"][0].as_str().unwrap();
                let key: Vec<&str> = vtec.trim_matches('/').split('.').collect();
                MarineAlert {
                    id: p["id"].as_str().unwrap().into(),
                    event: p["event"].as_str().unwrap().into(),
                    headline: p["headline"].as_str().map(Into::into),
                    description: p["description"].as_str().map(unwrap),
                    instruction: p["instruction"].as_str().map(unwrap),
                    onset: time("onset"),
                    ends: time("ends"),
                    expires: time("expires"),
                    severity,
                    zones: p["geocode"]["UGC"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|z| z.as_str().unwrap().into())
                        .collect(),
                    vtec_key: Some(key[2..6].join(".")),
                }
            })
            .collect()
    }

    /// Harbor input on the recording day with the marine alert.
    fn marine_input(voice: Voice) -> ViewInput {
        let mut input = sample_input();
        input.now = at("2026-11-08T14:30:00Z");
        input.voice = voice;
        input.snapshot.marine.marine_alerts = anz338_alerts();
        input.snapshot.weather.as_mut().unwrap().alerts = Vec::new();
        input
    }

    /// The weather provider's report of the same advisory.
    fn weather_report(input: &ViewInput, id: &str) -> Alert {
        let marine = &input.snapshot.marine.marine_alerts[0];
        Alert {
            id: Some(id.into()),
            title: marine.event.clone(),
            description: "* WHAT...West winds 10 to 20 kt.".into(),
            severity: Level::Moderate,
            start: marine.onset.unwrap(),
            end: marine.ends.unwrap(),
            source: "NWS Upton NY".into(),
            checked_at: None,
            url: None,
        }
    }

    fn count(card: &AlertsCard, event: &str) -> usize {
        card.full.alerts.iter().filter(|a| a.event == event).count()
    }

    #[test]
    fn the_recorded_advisory_is_shown_verbatim() {
        let input = marine_input(Voice::Luna);
        let card = build(&input).unwrap();
        let a = &card.full.alerts[0];
        assert_eq!(card.full.alerts.len(), 1);
        assert_eq!(a.event, "Small Craft Advisory");
        assert_eq!(
            a.headline.as_deref(),
            input.snapshot.marine.marine_alerts[0].headline.as_deref()
        );
        assert_eq!(a.zones.len(), 6);
        assert_eq!(a.onset.as_deref(), Some("Nov 8, 1:48 AM"));
        assert_eq!(a.ends.as_deref(), Some("Nov 9, 6:00 PM"));
        insta::assert_json_snapshot!("alerts_marine_luna", card);
    }

    #[test]
    fn the_full_form_carries_the_whole_description_and_instruction() {
        for voice in [Voice::Luna, Voice::Plain] {
            let input = marine_input(voice);
            let source = &input.snapshot.marine.marine_alerts[0];
            let card = build(&input).unwrap();
            let a = &card.full.alerts[0];
            assert_eq!(a.description, source.description);
            assert_eq!(a.instruction, source.instruction);
            assert!(a.description.as_deref().unwrap().contains("* IMPACTS..."));
            assert!(
                a.instruction
                    .as_deref()
                    .unwrap()
                    .starts_with("Inexperienced mariners")
            );
        }
    }

    #[test]
    fn the_gale_warning_shows_its_description_and_instruction() {
        let mut input = marine_input(Voice::Plain);
        input.now = at("2026-11-15T12:00:00Z");
        input.snapshot.marine.marine_alerts = recorded_alerts(
            "alerts-active-zone-GMZ250-gale-warning.geo.json",
            AlertSeverity::Moderate,
        );
        let card = build(&input).unwrap();
        let a = &card.full.alerts[0];
        assert_eq!(a.event, "Gale Warning");
        assert!(a.description.as_deref().unwrap().contains("* IMPACTS..."));
        assert!(a.instruction.as_deref().unwrap().contains("Remain in port"));
        insta::assert_json_snapshot!("alerts_gale_plain", card);
    }

    #[test]
    fn the_advisory_appears_once_when_both_sessions_report_it() {
        let mut input = marine_input(Voice::Plain);
        let delta = input.now.0 - SAMPLE_NOW.0;
        let mut weather = sample_alerts();
        for a in &mut weather {
            a.start.0 += delta;
            a.end.0 += delta;
        }
        // whether's alert id is the VTEC event key.
        weather.push(weather_report(&input, "KOKX.SC.Y.0100"));
        input.snapshot.weather.as_mut().unwrap().alerts = weather;
        let card = build(&input).unwrap();
        assert_eq!(count(&card, "Small Craft Advisory"), 1);
        assert_eq!(count(&card, "Flood Warning"), 1);
        assert_eq!(count(&card, "Wind Advisory"), 1);
        assert_eq!(card.full.alerts.len(), 3);
        // The marine report is the one kept: it has the zones.
        let sca = card
            .full
            .alerts
            .iter()
            .find(|a| a.event == "Small Craft Advisory")
            .unwrap();
        assert_eq!(sca.origin, AlertOrigin::Marine);
        assert_eq!(sca.zones.len(), 6);
        // Most severe first.
        assert_eq!(card.full.alerts[0].event, "Flood Warning");
        insta::assert_json_snapshot!("alerts_both_plain", card);
    }

    #[test]
    fn without_a_key_the_event_onset_and_zone_decide() {
        let mut input = marine_input(Voice::Plain);
        let report = weather_report(&input, "urn:oid:not-a-vtec-key");
        input.snapshot.weather.as_mut().unwrap().alerts = vec![report];
        assert_eq!(build(&input).unwrap().full.alerts.len(), 1);
        // A different onset is a different advisory.
        input.snapshot.weather.as_mut().unwrap().alerts[0].start.0 += 3_600;
        assert_eq!(build(&input).unwrap().full.alerts.len(), 2);
        // Different VTEC keys are different advisories even with the same event and onset.
        let other = weather_report(&input, "KOKX.SC.Y.0101");
        input.snapshot.weather.as_mut().unwrap().alerts = vec![other];
        assert_eq!(build(&input).unwrap().full.alerts.len(), 2);
    }

    #[test]
    fn expired_alerts_are_dropped() {
        let mut input = marine_input(Voice::Plain);
        input.now = at("2026-11-09T23:30:00Z");
        assert!(build(&input).is_none());
        // Whether's sample alerts, at their own sample time.
        let mut input = sample_input();
        input.now = SAMPLE_NOW;
        input.snapshot.marine.marine_alerts = Vec::new();
        input.snapshot.weather.as_mut().unwrap().alerts = sample_alerts();
        let card = build(&input).unwrap();
        assert_eq!(card.full.alerts.len(), 2);
        assert_eq!(card.full.alerts[0].origin, AlertOrigin::Weather);
        input.now = UnixTime(SAMPLE_NOW.0 + 20 * 3_600);
        assert!(build(&input).is_none());
    }

    #[test]
    fn the_text_is_the_same_in_both_voices() {
        let luna = build(&marine_input(Voice::Luna)).unwrap();
        let plain = build(&marine_input(Voice::Plain)).unwrap();
        assert_eq!(luna.full, plain.full);
        assert!(!format!("{:?}", luna.chrome).contains("Luna"));
    }
}
