//! Every card, the timeline and the map and offline words built in the pseudo-locale. A text of
//! the table comes out accented, so a word the view writes outside the table shows up as plain
//! ASCII letters, and a number written with an English decimal point as a '.' between digits.
//! What the data itself says (station names, forecast text, a provider's own credit) is taken
//! out first: it is shown as the source writes it.

use std::collections::BTreeSet;

use serde_json::Value;

use crate::view::cards::flow::fixtures::{croton_input, metric, us};
use crate::view::format::Formatter;
use crate::view::locale::Locale;
use crate::view::{CardId, SpotView, ViewInput, Voice, build_view};

/// The inputs of the card tests: between them every card, a timeline and both unit systems.
fn inputs() -> Vec<(&'static str, ViewInput)> {
    use crate::view::cards;
    vec![
        ("sample", crate::view::tests::sample_input()),
        ("tide", cards::tide::testing::input()),
        ("sun and moon", cards::sun_moon::testing::battery_input()),
        (
            "bloom forecast",
            cards::bloom::tests::lake(
                us(),
                crate::model::BloomInfo {
                    hab: Some(cards::bloom::tests::forecast(
                        crate::model::HabLevel::Moderate,
                        crate::view::cards::flow::fixtures::NOW,
                    )),
                    ..crate::model::BloomInfo::default()
                },
            ),
        ),
        ("croton", croton_input(us(), Voice::Luna)),
        ("cohn6", cards::flow::tests::cohn6_input(metric())),
        ("current", cards::current::tests::nyh1927()),
        ("pressure", cards::pressure::tests::forecast_only()),
        ("wind", cards::wind::tests::forecast_only()),
        (
            "clarity",
            cards::clarity::tests::bronx(metric(), Voice::Plain),
        ),
        (
            "water temperature",
            cards::water_temp::tests::river(us(), Voice::Luna, 12.0, 24.0),
        ),
        ("noreaster", cards::storm_ahead::tests::noreaster()),
        ("hurricane", cards::storm_ahead::tests::hurricane()),
        (
            "marine forecast",
            cards::marine_forecast::tests::input(20, Voice::Plain),
        ),
        ("alerts", cards::alerts::tests::marine_input(Voice::Luna)),
        ("waves", cards::waves::tests::buoy_input()),
        ("reservoir", cards::flow::tests::reservoir_input()),
        (
            "bloom satellite",
            cards::bloom::tests::lake(metric(), cards::bloom::tests::recent_composite()),
        ),
        (
            "bloom stale",
            cards::bloom::tests::lake(us(), cards::bloom::tests::stale_composite()),
        ),
        ("stocking", cards::stocking::tests::with_events(us())),
        ("timeline", crate::view::timeline::tests::input()),
    ]
}

/// The last field name of a path: "title" of ".cards[0].chrome.title".
fn field(path: &str) -> &str {
    path.rsplit('.').next().unwrap_or_default()
}

/// Every string in a JSON value, with its path.
fn strings(value: &Value, path: &str, out: &mut Vec<(String, String)>) {
    match value {
        Value::String(s) => out.push((path.to_owned(), s.clone())),
        Value::Array(items) => {
            for (i, v) in items.iter().enumerate() {
                strings(v, &format!("{path}[{i}]"), out);
            }
        }
        Value::Object(map) => {
            for (k, v) in map {
                strings(v, &format!("{path}.{k}"), out);
            }
        }
        _ => {}
    }
}

/// The enum names and tags in a schema: identifiers, not text.
fn identifiers(schema: &Value, out: &mut BTreeSet<String>) {
    match schema {
        Value::Object(map) => {
            for (k, v) in map {
                match (k.as_str(), v) {
                    ("enum", Value::Array(names)) => {
                        out.extend(names.iter().filter_map(|n| n.as_str().map(str::to_owned)));
                    }
                    ("const", Value::String(name)) => {
                        out.insert(name.clone());
                    }
                    _ => identifiers(v, out),
                }
            }
        }
        Value::Array(items) => items.iter().for_each(|v| identifiers(v, out)),
        _ => {}
    }
}

/// An enum name or an id in the snapshot's JSON ("observed", "coops"): not text the data shows,
/// and kept out of the data so it cannot hide an English word the view writes.
fn identifier(s: &str) -> bool {
    s.chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// What the data says, longest first: every string of the snapshot but its identifiers, as
/// written and lower case, with the names whether gives its providers and models.
fn data_texts(input: &ViewInput) -> Vec<String> {
    let mut found = Vec::new();
    strings(
        &serde_json::to_value(&input.snapshot).unwrap(),
        "",
        &mut found,
    );
    let mut texts: BTreeSet<String> = BTreeSet::new();
    for (path, s) in found {
        let model = whether_core::models::short_name(&s);
        if model != s {
            texts.insert(model.to_owned());
        }
        // A source without a name in the table is shown by its id.
        if !identifier(&s) || field(&path) == "source" {
            texts.insert(s.to_lowercase());
            texts.insert(s);
        }
    }
    if let Some(w) = &input.snapshot.weather {
        texts.insert(w.provider.display_name().to_owned());
        texts.insert(w.provider.attribution().to_owned());
        for p in &w.supplemented_by {
            texts.insert(p.display_name().to_owned());
            texts.insert(p.short_credit().to_owned());
        }
    }
    // The name of the user's limits profile is theirs.
    if let Some(limits) = &input.trip_limits {
        texts.insert(limits.name.clone());
    }
    // A licence's version is part of its name, not a number to write in the locale.
    texts.insert(crate::view::packs::pseudo("CC BY 4.0"));
    let mut texts: Vec<String> = texts.into_iter().filter(|t| t.len() > 1).collect();
    texts.sort_by_key(|t| std::cmp::Reverse(t.len()));
    texts
}

/// What is left of `text` once the data is taken out, when that still has English in it.
fn untranslated(text: &str, data: &[String]) -> Option<String> {
    let mut rest = text.to_owned();
    for d in data {
        if rest.contains(d.as_str()) {
            rest = rest.replace(d.as_str(), " ");
        }
    }
    let letters = rest.chars().any(|c| c.is_ascii_alphabetic());
    let chars: Vec<char> = rest.chars().collect();
    let point = chars
        .windows(3)
        .any(|w| w[0].is_ascii_digit() && w[1] == '.' && w[2].is_ascii_digit());
    (letters || point).then_some(rest)
}

/// The paths and texts of `value` that are not translated, leaving out links and identifiers.
pub(in crate::view) fn english_in(value: &Value, data: &[String]) -> Vec<String> {
    let schema = serde_json::to_value(schemars::schema_for!(SpotView)).unwrap();
    let mut ids = BTreeSet::new();
    identifiers(&schema, &mut ids);
    let mut found = Vec::new();
    strings(value, "", &mut found);
    found
        .into_iter()
        .filter(|(path, s)| {
            !matches!(field(path), "link" | "key" | "id") && !ids.contains(s.as_str())
        })
        .filter_map(|(path, s)| {
            untranslated(&s, data).map(|rest| format!("{path}: {s:?} ({rest:?})"))
        })
        .collect()
}

#[test]
fn the_check_finds_english_words_and_decimal_points() {
    let data = ["Hudson R at Battery, NY".to_owned()];
    assert_eq!(
        untranslated("[Ŧíđé ] Hudson R at Battery, NY 4,2", &data),
        None
    );
    assert!(untranslated("[Ŧíđé ] high", &data).is_some());
    assert!(untranslated("4.2 [ƒŧ ]", &data).is_some());
    assert!(untranslated("12:03 · 2026-11-08", &data).is_none());
}

#[test]
fn every_card_and_the_timeline_are_translated_in_the_pseudo_locale() {
    let mut problems = Vec::new();
    let mut cards = BTreeSet::new();
    for (name, mut input) in inputs() {
        let data = data_texts(&input);
        input.locale = Locale::PSEUDO;
        input.card_order = CardId::ALL.to_vec();
        for voice in [Voice::Luna, Voice::Plain] {
            input.voice = voice;
            let view = build_view(&input);
            cards.extend(view.cards.iter().map(|c| c.chrome().id));
            let json = serde_json::to_value(&view).unwrap();
            problems.extend(
                english_in(&json, &data)
                    .into_iter()
                    .map(|p| format!("{name} ({voice:?}){p}")),
            );
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
    // Every card the view builds was built at least once.
    let built: BTreeSet<CardId> = CardId::ALL
        .into_iter()
        .filter(|id| !matches!(id, CardId::Species))
        .collect();
    assert_eq!(cards, built);
}

#[test]
fn the_pseudo_locale_writes_numbers_and_dates_its_own_way() {
    let mut input = crate::view::cards::tide::testing::input();
    input.locale = Locale::PSEUDO;
    let f = Formatter::for_input(&input);
    assert_eq!(f.current_speed(Some(0.72)).text, "1,4");
    assert_eq!(f.discharge(Some(34.0)).text, "1\u{202F}201");
    let date = f.date_of(input.now);
    assert!(date.starts_with("8 [Ñöṽ"), "{date}");
    // English is untouched by the pseudo pack.
    input.locale = Locale::ENGLISH;
    assert_eq!(Formatter::for_input(&input).date_of(input.now), "Nov 8");
}

#[test]
fn map_depth_and_offline_words_are_translated_in_the_pseudo_locale() {
    use crate::map::breaks::BreakKind;
    use crate::map::catalogue::DateMode;
    use crate::map::dem::{ApproxDepth, DepthReading};
    use crate::map::enc::StructureKind;
    use crate::view::strings::{
        MapQuantity, break_strength, depth_label, depth_reading, map_current, map_measure,
        map_time, map_value, reef_depths, structure_category, structure_title,
    };
    let input = crate::view::tests::sample_input();
    for units in [us(), metric()] {
        let f = Formatter::new(
            units,
            Voice::Plain,
            whether_core::presentation::TimeFormat::H12,
            "America/New_York",
        )
        .with_locale(Locale::PSEUDO);
        let reading = ApproxDepth {
            reading: DepthReading::Depth { depth_m: 29.6 },
            vertical_datum: None,
            source: Some("crm_vol1_2023".into()),
        };
        let (distance, bearing, line) = map_measure(&f, 3_862.4, 90.0);
        let texts = [
            map_value(&f, MapQuantity::Temperature, 15.04),
            map_value(&f, MapQuantity::Chlorophyll, 2.46),
            map_value(&f, MapQuantity::Kd490, 0.254),
            map_current(&f, 0.5144, 45.0),
            map_time(&f, input.now, DateMode::Hourly),
            distance,
            bearing,
            line,
            depth_label(&f, 29.6),
            depth_reading(&f, &reading),
            break_strength(&f, BreakKind::Sst, 0.5),
            break_strength(&f, BreakKind::Chlorophyll, 0.0375),
            structure_title(&f, StructureKind::Wreck, false).to_owned(),
            structure_category(&f, "dangerous wreck", true),
            reef_depths(&f, Some(6.1), Some(21.3)).unwrap_or_default(),
            f.strings.pack_size(48_260_000),
            f.strings.pack_duration(4_800),
        ];
        let data = ["dangerous wreck".to_owned()];
        let english: Vec<String> = texts
            .iter()
            .filter_map(|t| untranslated(t, &data).map(|rest| format!("{t:?} ({rest:?})")))
            .collect();
        assert!(english.is_empty(), "{}", english.join("\n"));
    }
}

#[test]
fn the_trip_view_is_translated_in_the_pseudo_locale() {
    use crate::view::trip::{TripView, tests as trip};
    type Build = fn(&ViewInput) -> trip::Built;
    let schema = serde_json::to_value(schemars::schema_for!(TripView)).unwrap();
    let mut ids = BTreeSet::new();
    identifiers(&schema, &mut ids);
    let builds: [(&str, Build); 3] = [
        ("two places", trip::two_places),
        ("beyond the forecast", trip::beyond_the_forecast),
        ("battery", trip::battery_three_days),
    ];
    let mut problems = Vec::new();
    for (name, build) in builds {
        for units in [us(), metric()] {
            for voice in [Voice::Luna, Voice::Plain] {
                let mut input = trip::view_input();
                input.units = units;
                input.voice = voice;
                let built = build(&input);
                // What the data says: the snapshots, the trip's own words, the agencies' names.
                let mut data: BTreeSet<String> = BTreeSet::new();
                for place in built.inputs.values() {
                    if let Some(snapshot) = &place.snapshot {
                        let mut each = input.clone();
                        each.snapshot = snapshot.clone();
                        data.extend(data_texts(&each));
                    }
                }
                let t = &built.trip;
                data.extend([t.name.clone(), t.notes.clone()]);
                data.extend(t.checklist.iter().map(|c| c.text.clone()));
                for place in &t.places {
                    data.insert(place.name.clone());
                    data.extend(place.area.clone());
                    if let Some(region) = &place.region {
                        data.extend(
                            crate::regulations::all_links(region)
                                .into_iter()
                                .map(|l| l.label),
                        );
                    }
                }
                let mut data: Vec<String> = data.into_iter().filter(|t| t.len() > 1).collect();
                data.sort_by_key(|t| std::cmp::Reverse(t.len()));
                input.locale = Locale::PSEUDO;
                let json = serde_json::to_value(built.view(&input)).unwrap();
                let mut found = Vec::new();
                strings(&json, "", &mut found);
                problems.extend(
                    found
                        .into_iter()
                        .filter(|(path, s)| {
                            !matches!(field(path), "link" | "key" | "id" | "place")
                                && !ids.contains(s.as_str())
                        })
                        .filter_map(|(path, s)| {
                            untranslated(&s, &data)
                                .map(|rest| format!("{name} ({voice:?}){path}: {s:?} ({rest:?})"))
                        }),
                );
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}
