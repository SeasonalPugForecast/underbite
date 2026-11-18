//! Active marine alerts.

use schemars::JsonSchema;
use serde::Serialize;
use whether_core::model::Alert;
use whether_core::scales::Level;

use crate::model::{AlertSeverity, MarineAlert};
use crate::provenance::SourceId;
use crate::time::UnixTime;
use crate::view::alerts::rule_row;
use crate::view::format::Formatter;
use crate::view::strings::{Key, SafetyKey};
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
    /// When the alert is in effect, in words: "Nov 8, 1:48 AM – Nov 9, 6:00 PM", "From …"
    /// without an end, "Until …" without an onset (the end, else the expiry).
    pub span: Option<String>,
    /// Zone names or ids the alert covers.
    pub zones: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct AlertsFull {
    /// Marine and weather alerts de-duplicated, most severe first.
    pub alerts: Vec<AlertVm>,
    /// The spot's alert rules and what their last check found; `None` when it has none.
    pub rules: Option<AlertRulesVm>,
}

/// The rules section of the full form.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct AlertRulesVm {
    /// "Your alert rules".
    pub title: String,
    /// In the order the caller gave them.
    pub rules: Vec<AlertRuleVm>,
}

/// How a rule's last check came out, for its styling; the words are in `state_label`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RuleStateKind {
    /// Every condition holds.
    Met,
    NotMet,
    /// A condition's data was too old or too far, or its source failed.
    Unchecked,
    /// A condition had no data.
    NoData,
    /// Its spot or trip was deleted.
    Orphaned,
    /// Not checked since it was made.
    NotChecked,
    /// Turned off.
    Off,
}

/// One alert rule: its name, its conditions in the user's units and its state.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct AlertRuleVm {
    pub id: String,
    /// The user's name for it.
    pub name: String,
    /// "River flow under 800 cfs · Water temperature under 66 °F".
    pub summary: String,
    pub state: RuleStateKind,
    /// "Met since 6:10 AM", "River flow: couldn't check, data 5 h old".
    pub state_label: String,
    /// "Checked 6:10 AM"; `None` before the first check and when off.
    pub checked: Option<String>,
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

/// How the alert source grades the alert: safety text, the same in every voice.
fn severity_label(f: &Formatter, severity: AlertSeverity) -> &'static str {
    f.strings.safety(SafetyKey::severity(severity))
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

/// When an alert is in effect, from its onset to its end (else its expiry): safety text, the
/// same in every voice. A bare end reads "Until …", never as a start.
pub(in crate::view) fn span(
    f: &Formatter,
    onset: Option<UnixTime>,
    end: Option<UnixTime>,
) -> Option<String> {
    let s = f.strings;
    match (onset.map(|t| when(f, t)), end.map(|t| when(f, t))) {
        (Some(start), Some(end)) => {
            Some(s.safety_fmt(SafetyKey::AlertSpan, &[("start", &start), ("end", &end)]))
        }
        (Some(start), None) => Some(s.safety_fmt(SafetyKey::AlertFrom, &[("start", &start)])),
        (None, Some(end)) => Some(s.safety_fmt(SafetyKey::AlertUntil, &[("end", &end)])),
        (None, None) => None,
    }
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
            severity_label: severity_label(f, a.severity).to_owned(),
            onset: a.onset.map(|t| when(f, t)),
            ends: a.ends.map(|t| when(f, t)),
            expires: a.expires.map(|t| when(f, t)),
            span: span(f, a.onset, a.ends.or(a.expires)),
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
            severity_label: severity_label(f, severity).to_owned(),
            onset: Some(when(f, a.start)),
            ends: Some(when(f, a.end)),
            expires: None,
            span: span(f, Some(a.start), Some(a.end)),
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
    let rules = rules_section(&f, input);
    if entries.is_empty() && rules.is_none() {
        return None;
    }
    // Most severe first, then the newest onset.
    entries.sort_by(|a, b| {
        severity_rank(b.vm.severity)
            .cmp(&severity_rank(a.vm.severity))
            .then(b.onset.cmp(&a.onset))
    });
    let alerts: Vec<AlertVm> = entries.into_iter().map(|e| e.vm).collect();
    let title = f.strings.card_title(CardId::Alerts).to_owned();
    let Some(top) = alerts.first() else {
        // Without alerts the card is built only for its rules.
        let rules = rules?;
        return Some(rules_only_card(&f, title, rules));
    };

    let mut credits = Vec::new();
    if !snapshot.marine.marine_alerts.is_empty() {
        let found = snapshot
            .attribution
            .iter()
            .find(|a| a.source == SourceId::NWS);
        credits.push(CreditVm {
            source: f.strings.source_name(&SourceId::NWS),
            // The credit text of the marine alerts when the snapshot carries none.
            text: found.map_or_else(
                || f.strings.safety(SafetyKey::CreditMarineAlerts).to_owned(),
                |a| a.credit.clone(),
            ),
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

    let count = alerts.len();
    let events: Vec<&str> = alerts.iter().map(|a| a.event.as_str()).collect();
    let mut accessibility = f.strings.fmt(
        Key::AlertsAcc,
        &[
            ("title", &title),
            ("count", &count.to_string()),
            ("events", &events.join(", ")),
        ],
    );
    if let Some(rules) = &rules {
        accessibility.push(' ');
        accessibility.push_str(&rules_accessibility(&f, rules));
    }
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
        full: AlertsFull { alerts, rules },
    })
}

/// The spot's rules with their states, when it has any.
fn rules_section(f: &Formatter, input: &ViewInput) -> Option<AlertRulesVm> {
    (!input.alert_rules.is_empty()).then(|| AlertRulesVm {
        title: f.strings.get(Key::AlertRulesTitle).to_owned(),
        rules: input
            .alert_rules
            .iter()
            .map(|r| rule_row(f, r, input.now))
            .collect(),
    })
}

/// "Rules met: 1 of 2".
fn rules_met(f: &Formatter, rules: &AlertRulesVm) -> (usize, String) {
    let met = rules
        .rules
        .iter()
        .filter(|r| r.state == RuleStateKind::Met)
        .count();
    let text = f.strings.fmt(
        Key::AlertRulesMet,
        &[
            ("met", &met.to_string()),
            ("count", &rules.rules.len().to_string()),
        ],
    );
    (met, text)
}

/// What `VoiceOver` reads for the rules: how many are met, then each with its state.
fn rules_accessibility(f: &Formatter, rules: &AlertRulesVm) -> String {
    let (_, met) = rules_met(f, rules);
    let list: Vec<String> = rules
        .rules
        .iter()
        .map(|r| {
            f.strings.fmt(
                Key::AlertRuleAcc,
                &[("name", &r.name), ("state", &r.state_label)],
            )
        })
        .collect();
    f.strings.fmt(
        Key::AlertRulesAcc,
        &[("rules", &met), ("list", &list.join(". "))],
    )
}

/// The card with rules and no alert in effect: the headline counts the rules met, the lines
/// name the first one met. It claims nothing about alerts.
fn rules_only_card(f: &Formatter, title: String, rules: AlertRulesVm) -> AlertsCard {
    let (met, headline) = rules_met(f, &rules);
    let first_met = rules.rules.iter().find(|r| r.state == RuleStateKind::Met);
    let lines = [
        first_met.map(|r| r.name.clone()),
        (met > 1).then(|| {
            f.strings
                .fmt(Key::MoreCount, &[("n", &(met - 1).to_string())])
        }),
    ]
    .into_iter()
    .flatten()
    .collect();
    let accessibility = f.strings.fmt(
        Key::AlertsRulesOnlyAcc,
        &[
            ("title", &title),
            ("rules", &rules_accessibility(f, &rules)),
        ],
    );
    AlertsCard {
        chrome: CardChrome {
            id: CardId::Alerts,
            title,
            footnote: None,
            credits: Vec::new(),
            state: CardState::Fresh,
            notices: Vec::new(),
            accessibility,
        },
        compact: Compact {
            headline: Quantity {
                value: None,
                text: headline,
                unit: String::new(),
            },
            lines,
            spark: Vec::new(),
        },
        full: AlertsFull {
            alerts: Vec::new(),
            rules: Some(rules),
        },
    }
}

#[cfg(test)]
pub(in crate::view) mod tests {
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
    pub(in crate::view) fn marine_input(voice: Voice) -> ViewInput {
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
        assert_eq!(a.span.as_deref(), Some("Nov 8, 1:48 AM – Nov 9, 6:00 PM"));
        insta::assert_json_snapshot!("alerts_marine_luna", card);
    }

    #[test]
    fn the_span_says_from_and_until_when_one_end_is_missing() {
        for voice in [Voice::Luna, Voice::Plain] {
            let mut input = marine_input(voice);
            let alert = &mut input.snapshot.marine.marine_alerts[0];
            let (onset, ends) = (alert.onset, alert.ends);
            // No end: the expiry closes it.
            alert.ends = None;
            alert.expires = Some(at("2026-11-08T23:00:00Z"));
            let card = build(&input).unwrap();
            assert_eq!(
                card.full.alerts[0].span.as_deref(),
                Some("Nov 8, 1:48 AM – Nov 8, 6:00 PM")
            );
            // No end and no expiry: from the onset.
            input.snapshot.marine.marine_alerts[0].expires = None;
            let card = build(&input).unwrap();
            assert_eq!(
                card.full.alerts[0].span.as_deref(),
                Some("From Nov 8, 1:48 AM")
            );
            // No onset: a bare end reads as an end, not a start.
            let alert = &mut input.snapshot.marine.marine_alerts[0];
            alert.onset = None;
            alert.ends = ends;
            let card = build(&input).unwrap();
            assert_eq!(
                card.full.alerts[0].span.as_deref(),
                Some("Until Nov 9, 6:00 PM")
            );
            let alert = &mut input.snapshot.marine.marine_alerts[0];
            alert.ends = None;
            assert!(build(&input).unwrap().full.alerts[0].span.is_none());
            assert!(onset.is_some());
        }
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

    // -- the rules section --------------------------------------------------------------------

    use crate::alerts::{AlertMetric, AlertRule, CheckProblem, Condition, RuleState, RuleStatus};
    use crate::view::alerts::RuleInput;
    use crate::view::alerts::tests::{COOL_C, CROTON_LOW_M3S, below, croton_rule, rule, spot};
    use crate::view::cards::flow::fixtures::{metric, us};
    use std::collections::BTreeMap;

    fn state(status: RuleStatus, met_since: Option<&str>) -> RuleState {
        RuleState {
            status,
            checked_at: at("2026-11-08T14:10:00Z"),
            met_since: met_since.map(at),
            notified_at: None,
            pending: None,
            marine: BTreeMap::new(),
            window: None,
        }
    }

    fn named(name: &str, all_of: Vec<Condition>) -> AlertRule {
        AlertRule {
            name: name.into(),
            ..rule(
                &name.to_lowercase().replace(' ', "-"),
                spot("battery"),
                all_of,
            )
        }
    }

    /// The Battery's rules in every state a check can leave.
    pub(in crate::view) fn battery_rules() -> Vec<RuleInput> {
        let advisory = || Condition::MarineAlert {
            event: "Small Craft Advisory".into(),
        };
        let gusts = || below(AlertMetric::Gusts, 9.0);
        let water = || below(AlertMetric::WaterTemperature, COOL_C);
        let with = |rule: AlertRule, state: RuleState| RuleInput {
            rule,
            state: Some(state),
        };
        vec![
            with(
                named("Harbor advisory", vec![advisory()]),
                state(RuleStatus::Met, Some("2026-11-08T11:10:00Z")),
            ),
            with(
                named("Calm and cool", vec![gusts(), water()]),
                state(RuleStatus::Met, Some("2026-11-07T22:40:00Z")),
            ),
            with(
                named("Light wind", vec![gusts()]),
                state(
                    RuleStatus::Unchecked {
                        condition: 0,
                        problem: CheckProblem::Stale {
                            age_s: 5 * 3_600 + 600,
                            limit_s: 3 * 3_600,
                        },
                    },
                    None,
                ),
            ),
            with(
                named("Cool water", vec![water()]),
                state(
                    RuleStatus::Unchecked {
                        condition: 0,
                        problem: CheckProblem::Far {
                            distance_m: 41_000.0,
                            limit_m: 25_000.0,
                        },
                    },
                    None,
                ),
            ),
            with(
                named("Advisory and calm", vec![advisory(), gusts()]),
                state(
                    RuleStatus::Unchecked {
                        condition: 0,
                        problem: CheckProblem::SourceFailed,
                    },
                    None,
                ),
            ),
            with(
                named("Low tide", vec![below(AlertMetric::TideHeight, 0.2)]),
                state(RuleStatus::NoData { condition: 0 }, None),
            ),
            with(
                named("Storms", vec![Condition::StormAhead {}]),
                state(RuleStatus::NotMet, None),
            ),
            with(croton_rule(), state(RuleStatus::Orphaned, None)),
            RuleInput {
                rule: named(
                    "New rule",
                    vec![Condition::WindowStarts { within_minutes: 30 }],
                ),
                state: None,
            },
            with(
                AlertRule {
                    enabled: false,
                    ..named(
                        "Old rule",
                        vec![below(AlertMetric::Discharge, CROTON_LOW_M3S)],
                    )
                },
                state(RuleStatus::Met, Some("2026-11-01T10:00:00Z")),
            ),
        ]
    }

    #[test]
    fn the_rules_section_shows_each_rules_state() {
        let mut input = marine_input(Voice::Plain);
        input.alert_rules = battery_rules();
        let card = build(&input).unwrap();
        // The alerts are as before.
        assert_eq!(
            card.full.alerts,
            build(&marine_input(Voice::Plain)).unwrap().full.alerts
        );
        let rules = card.full.rules.as_ref().unwrap();
        assert_eq!(rules.title, "Your alert rules");
        let states: Vec<(RuleStateKind, &str)> = rules
            .rules
            .iter()
            .map(|r| (r.state, r.state_label.as_str()))
            .collect();
        assert_eq!(
            states,
            [
                (RuleStateKind::Met, "Met since 6:10 AM"),
                (RuleStateKind::Met, "Met since Nov 7, 5:40 PM"),
                (
                    RuleStateKind::Unchecked,
                    "Gusts: couldn't check, data 5 h old"
                ),
                (
                    RuleStateKind::Unchecked,
                    "Water temperature: couldn't check, the station is 25 mi away"
                ),
                (
                    RuleStateKind::Unchecked,
                    "Marine alerts: couldn't check, the source didn't answer"
                ),
                (RuleStateKind::NoData, "Tide height: no data"),
                (RuleStateKind::NotMet, "Not met"),
                (RuleStateKind::Orphaned, "Its spot or trip was deleted"),
                (RuleStateKind::NotChecked, "Not checked yet"),
                (RuleStateKind::Off, "Off"),
            ]
        );
        assert_eq!(rules.rules[0].checked.as_deref(), Some("Checked 9:10 AM"));
        assert_eq!(rules.rules[8].checked, None);
        assert_eq!(rules.rules[9].checked, None);
        assert_eq!(
            rules.rules[1].summary,
            "Gusts under 20 mph · Water temperature under 66 °F"
        );
        // The compact form still leads with the alert in effect.
        assert_eq!(card.compact.headline.text, "Small Craft Advisory");
        assert!(
            card.chrome
                .accessibility
                .contains("Rules met: 2 of 10. Harbor advisory: Met since 6:10 AM."),
            "{}",
            card.chrome.accessibility
        );
        let mut all = BTreeMap::new();
        for (units, u) in [("us", us()), ("metric", metric())] {
            for voice in [Voice::Luna, Voice::Plain] {
                let mut input = marine_input(voice);
                input.units = u;
                input.alert_rules = battery_rules();
                all.insert(
                    format!("{units} {voice:?}"),
                    build(&input).unwrap().full.rules,
                );
            }
        }
        insta::assert_json_snapshot!("alerts_rules_section", all);
    }

    #[test]
    fn rules_without_an_alert_in_effect_still_make_the_card() {
        let mut input = marine_input(Voice::Luna);
        input.snapshot.marine.marine_alerts = Vec::new();
        assert!(build(&input).is_none());
        input.alert_rules = battery_rules();
        let card = build(&input).unwrap();
        assert_eq!(card.full.alerts, Vec::<AlertVm>::new());
        assert_eq!(card.chrome.credits, Vec::<CreditVm>::new());
        assert_eq!(card.compact.headline.text, "Rules met: 2 of 10");
        assert_eq!(card.compact.lines, ["Harbor advisory", "1 more"]);
        assert!(
            card.chrome
                .accessibility
                .starts_with("Alerts. Rules met: 2 of 10.")
        );
        insta::assert_json_snapshot!("alerts_rules_only_luna", card);
    }

    #[test]
    fn the_advisory_and_the_rules_safety_words_are_the_same_in_both_voices() {
        let card = |voice| {
            let mut input = marine_input(voice);
            input.alert_rules = battery_rules();
            build(&input).unwrap()
        };
        let (luna, plain) = (card(Voice::Luna), card(Voice::Plain));
        assert_eq!(luna.full.alerts, plain.full.alerts);
        let summary =
            |c: &AlertsCard, i: usize| c.full.rules.as_ref().unwrap().rules[i].summary.clone();
        // The advisory and the storm conditions.
        assert_eq!(summary(&luna, 0), "Small Craft Advisory in effect");
        assert_eq!(summary(&luna, 0), summary(&plain, 0));
        assert_eq!(summary(&luna, 6), summary(&plain, 6));
        // A fishing window follows the voice.
        assert_eq!(summary(&plain, 8), "A fishing window within 30 min");
        assert_eq!(summary(&luna, 8), "A snack time within 30 min");
    }
}
