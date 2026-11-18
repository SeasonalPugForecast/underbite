//! Every card, the timeline and the map and offline words built in the pseudo-locale. A text of
//! the table comes out accented, so a word the view writes outside the table shows up as plain
//! ASCII letters, and a number written with an English decimal point as a '.' between digits.
//! What the data itself says (station names, forecast text, a provider's own credit) is taken
//! out first: it is shown as the source writes it.

use std::collections::BTreeSet;

use serde_json::Value;

use crate::species::profile::SpeciesProfile;
use crate::view::cards::flow::fixtures::{croton_input, metric, us};
use crate::view::format::Formatter;
use crate::view::locale::Locale;
use crate::view::{CardId, SpotView, ViewInput, Voice, build_view_with_species};

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
        ("alert rules", {
            let mut input = cards::alerts::tests::marine_input(Voice::Plain);
            input.units = metric();
            input.alert_rules = cards::alerts::tests::battery_rules();
            input
        }),
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
        (
            "species",
            cards::water_temp::tests::river(metric(), Voice::Plain, 21.0, 22.0),
        ),
    ]
}

/// What species profiles say, as written: names, sources, citations and free text, longest first.
pub(in crate::view) fn profile_texts(profiles: &[SpeciesProfile]) -> Vec<String> {
    let mut found = Vec::new();
    for p in profiles {
        strings(&serde_json::to_value(p).unwrap(), "", &mut found);
    }
    let mut texts: BTreeSet<String> = found
        .into_iter()
        .filter(|(path, s)| field(path) != "licence" && !identifier(s))
        .map(|(_, s)| s)
        .collect();
    texts.insert("FishBase".to_owned());
    // A licence's version is part of its name.
    texts.insert(crate::view::packs::pseudo("CC BY-NC 4.0"));
    let mut texts: Vec<String> = texts.into_iter().filter(|t| t.len() > 1).collect();
    texts.sort_by_key(|t| std::cmp::Reverse(t.len()));
    texts
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
    // So are the names of their alert rules, and the alert events they watch for are the NWS's.
    for r in &input.alert_rules {
        texts.insert(r.rule.name.clone());
        for c in &r.rule.all_of {
            if let crate::alerts::Condition::MarineAlert { event } = c {
                texts.insert(event.clone());
            }
        }
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
    let species = crate::view::cards::species::tests::chosen();
    for (name, mut input) in inputs() {
        let mut data = data_texts(&input);
        data.extend(profile_texts(&species));
        data.sort_by_key(|t| std::cmp::Reverse(t.len()));
        input.locale = Locale::PSEUDO;
        input.card_order = CardId::ALL.to_vec();
        for voice in [Voice::Luna, Voice::Plain] {
            input.voice = voice;
            let view = build_view_with_species(&input, &species);
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
    // Every card was built at least once.
    let built: BTreeSet<CardId> = CardId::ALL.into_iter().collect();
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
        MapQuantity, break_strength, break_threshold_stops, breaks_few_clear, breaks_from,
        breaks_needs_source, contour_interval_text, depth_band_text, depth_datums_line,
        depth_label, depth_reading, map_current, map_measure, map_time, map_value, reef_depths,
        structure_category, structure_title,
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
            depth_band_text(&f, Some((4.572, 7.62))),
            depth_band_text(&f, None),
            contour_interval_text(&f, 3.048),
            depth_datums_line(&f, &["NAVD 88".to_owned()]),
            depth_datums_line(&f, &[]),
            break_strength(&f, BreakKind::Sst, 0.5),
            break_strength(&f, BreakKind::Chlorophyll, 0.0375),
            break_threshold_stops(&f, BreakKind::Sst)[4].1.clone(),
            break_threshold_stops(&f, BreakKind::Chlorophyll)[4]
                .1
                .clone(),
            breaks_from(
                &f,
                f.strings.layer_title("sst").unwrap_or_default(),
                input.now,
            ),
            breaks_few_clear(&f, input.now),
            breaks_needs_source(&f, f.strings.layer_title("chlorophyll").unwrap_or_default()),
            f.strings
                .layer_title("sst-breaks")
                .unwrap_or_default()
                .to_owned(),
            f.strings
                .layer_legend("chlorophyll-breaks")
                .unwrap_or_default()
                .to_owned(),
            structure_title(&f, StructureKind::Wreck, false).to_owned(),
            structure_category(&f, "dangerous wreck", true),
            reef_depths(&f, Some(6.1), Some(21.3)).unwrap_or_default(),
            f.strings.pack_size(48_260_000),
            f.strings.pack_duration(4_800),
        ];
        let data = ["dangerous wreck".to_owned(), "NAVD 88".to_owned()];
        let english: Vec<String> = texts
            .iter()
            .filter_map(|t| untranslated(t, &data).map(|rest| format!("{t:?} ({rest:?})")))
            .collect();
        assert!(english.is_empty(), "{}", english.join("\n"));
    }
}

/// Texts that are not translated, among `texts`, leaving out what `data` says.
fn english_texts(texts: &[String], data: &[String]) -> Vec<String> {
    texts
        .iter()
        .filter_map(|t| untranslated(t, data).map(|rest| format!("{t:?} ({rest:?})")))
        .collect()
}

#[test]
fn storm_layer_words_are_translated_in_the_pseudo_locale() {
    use crate::time::UnixTime;
    use crate::view::strings::{
        SafetyKey, advisory_wind, hurricane_movement, hurricane_name, storm_advisory,
        storm_cone_note, storm_day_time, storm_knots, storm_model_run, storm_span, storm_step,
    };
    const HOUR: i64 = 3_600;
    let now = crate::view::tests::sample_input().now;
    let zone = "America/New_York";
    let mut english = Vec::new();
    for units in [us(), metric()] {
        for voice in [Voice::Plain, Voice::Luna] {
            let f = Formatter::new(
                units,
                voice,
                whether_core::presentation::TimeFormat::H12,
                zone,
            )
            .with_locale(Locale::PSEUDO);
            let s = f.strings;
            let ahead = UnixTime(now.0 + 30 * HOUR);
            let mut texts = vec![
                storm_day_time(&f, zone, ahead),
                storm_span(&f, zone, ahead, UnixTime(ahead.0 + HOUR)),
                storm_span(&f, zone, ahead, UnixTime(ahead.0 + 9 * HOUR)),
                storm_span(&f, zone, ahead, UnixTime(ahead.0 + 50 * HOUR)),
                storm_step(&f, zone, now, now),
                storm_step(&f, zone, UnixTime(now.0 + 72 * HOUR), now),
                advisory_wind(&f, Some(85)).full(),
                advisory_wind(&f, None).full(),
                storm_knots(&f, Some(12.5)).full(),
                hurricane_movement(&f, Some(45.0), Some(13.0)),
                hurricane_movement(&f, Some(45.0), Some(0.5)),
                hurricane_movement(&f, None, None),
                storm_advisory(&f, zone, now),
                storm_model_run(&f, "ECMWF", Some(now)),
                storm_model_run(&f, "ECMWF", None),
                storm_cone_note(&f, 2026),
                s.safety_fmt(
                    SafetyKey::HurricaneCategory,
                    &[
                        ("name", &hurricane_name(&f, "HU", "Josephine")),
                        ("category", "2"),
                    ],
                ),
                s.safety_fmt(
                    SafetyKey::StormStrength,
                    &[
                        ("wind", &advisory_wind(&f, Some(85)).full()),
                        ("pressure", &f.pressure(Some(975.0)).full()),
                        ("movement", &hurricane_movement(&f, Some(45.0), Some(13.0))),
                    ],
                ),
                s.safety_fmt(SafetyKey::StormLayerNone, &[("n", "5")]),
                s.safety_fmt(
                    SafetyKey::StormLayerEvent,
                    &[(
                        "window",
                        &storm_span(&f, zone, ahead, UnixTime(ahead.0 + HOUR)),
                    )],
                ),
                s.safety_fmt(
                    SafetyKey::StormLayerLowest,
                    &[
                        ("pressure", &f.pressure(Some(990.0)).full()),
                        ("when", &storm_span(&f, zone, ahead, UnixTime(ahead.0 + 1))),
                        ("model", "ECMWF"),
                    ],
                ),
                s.safety_fmt(SafetyKey::StormLayerGuidance, &[("model", "HWRF")]),
            ];
            for classification in ["HU", "TS", "TD", "STS", "STD", "PTC", "PC", "TY", "XX"] {
                texts.push(hurricane_name(&f, classification, "Josephine"));
            }
            for key in [
                SafetyKey::StormLayerLoading,
                SafetyKey::StormLayerFailed,
                SafetyKey::StormLayerPartial,
                SafetyKey::StormLayerStormBand,
                SafetyKey::StormLayerOfficial,
                SafetyKey::StormLayerObserved,
                SafetyKey::StormLayerConsensus,
                SafetyKey::StormLayerConeLink,
                SafetyKey::StormLayerAdvisoryLink,
                SafetyKey::StormForecastsChangeText,
                SafetyKey::CreditOpenMeteoModels,
                SafetyKey::CreditNhc,
                SafetyKey::NotForNavigation,
            ] {
                texts.push(s.safety(key).to_owned());
            }
            let data = [
                "Josephine".to_owned(),
                "ECMWF".to_owned(),
                "HWRF".to_owned(),
            ];
            english.extend(english_texts(&texts, &data));
        }
    }
    assert!(english.is_empty(), "{}", english.join("\n"));
}

#[test]
fn depth_breaks_and_structure_words_are_translated_in_the_pseudo_locale() {
    use crate::map::breaks::BreakKind;
    use crate::view::strings::{
        Key, SafetyKey, break_strength, break_threshold_stops, contour_interval_text,
        depth_band_text, depth_label, structure_group_title,
    };
    let mut english = Vec::new();
    for units in [us(), metric()] {
        for voice in [Voice::Plain, Voice::Luna] {
            let f = Formatter::new(
                units,
                voice,
                whether_core::presentation::TimeFormat::H12,
                "America/New_York",
            )
            .with_locale(Locale::PSEUDO);
            let s = f.strings;
            let mut texts = vec![
                contour_interval_text(&f, 3.048),
                contour_interval_text(&f, 0.5),
                depth_band_text(&f, Some((1.5, 2.25))),
                depth_band_text(&f, None),
                structure_group_title(&f, 1),
                structure_group_title(&f, 1_200),
            ];
            texts.extend([0.0, 2.5, 10.0, 55.5, 100.0].map(|m| depth_label(&f, m)));
            for id in [
                "depth-shading",
                "depth-contours",
                "sst-breaks",
                "chlorophyll-breaks",
            ] {
                texts.push(s.layer_title(id).unwrap().to_owned());
                texts.push(s.layer_legend(id).unwrap().to_owned());
                texts.push(s.layer_credit(id).unwrap().to_owned());
            }
            for kind in [BreakKind::Sst, BreakKind::Chlorophyll] {
                for (_, label) in break_threshold_stops(&f, kind) {
                    texts.push(label);
                }
            }
            for per_km in [0.1, 0.25, 0.6, 1.25] {
                texts.push(break_strength(&f, BreakKind::Sst, per_km));
                texts.push(break_strength(&f, BreakKind::Chlorophyll, per_km / 10.0));
            }
            for key in [
                Key::MapDepthTitle,
                Key::MapDepthHighlight,
                Key::MapDepthFrom,
                Key::MapDepthTo,
                Key::MapDepthOff,
                Key::MapDepthHighlightFrom,
                Key::MapDepthHighlightTo,
                Key::MapDepthHighlightClear,
                Key::MapDepthZoomIn,
                Key::MapDepthLoading,
                Key::MapDepthFailed,
                Key::MapBreaksThreshold,
                Key::MapBreaksThresholdLabel,
                Key::MapBreaksZoomIn,
                Key::MapBreaksLoading,
                Key::MapBreaksNone,
                Key::MapStructureLoading,
                Key::MapStructureFailed,
                Key::MapStructureZoomIn,
            ] {
                texts.push(s.get(key).to_owned());
            }
            for key in [
                SafetyKey::MapCreditBathymetry,
                SafetyKey::MapReferenceBathymetry,
                SafetyKey::MapLegendBathymetry,
                SafetyKey::NotForNavigation,
            ] {
                texts.push(s.safety(key).to_owned());
            }
            english.extend(english_texts(&texts, &[]));
        }
    }
    assert!(english.is_empty(), "{}", english.join("\n"));
}

#[test]
fn preset_share_words_are_translated_in_the_pseudo_locale() {
    use crate::view::strings::{Key, Strings};
    let mut english = Vec::new();
    for voice in [Voice::Plain, Voice::Luna] {
        let s = Strings::localized(voice, Locale::PSEUDO);
        let texts: Vec<String> = [
            Key::PresetShare,
            Key::PresetShareDetail,
            Key::PresetShareFile,
            Key::PresetCancel,
            Key::PresetDone,
            Key::ReceivePreset,
            Key::ReceivePresetExists,
            Key::ReceiveAdd,
            Key::ReceivePresetRename,
            Key::ReceiveReplace,
            Key::ReceivePresetNameTitle,
            Key::ReceivePresetNameDetail,
            Key::ReceivePresetNameField,
            Key::ReceivePresetAdded,
        ]
        .into_iter()
        // The app fills in the received preset's name.
        .map(|key| s.fmt(key, &[("name", "Stripers")]))
        .collect();
        english.extend(english_texts(&texts, &["Stripers".to_owned()]));
    }
    assert!(english.is_empty(), "{}", english.join("\n"));
}

#[test]
fn offline_pack_words_are_translated_in_the_pseudo_locale() {
    use crate::map::pack::{DatePolicy, PlanError, Stale, ZoomReach};
    use crate::time::{LocalDate, UnixTime};
    use crate::view::strings::{Key, pack_refreshed};
    let day = LocalDate {
        year: 2026,
        month: 11,
        day: 20,
    };
    let at = UnixTime(1_794_944_400);
    let mut english = Vec::new();
    for voice in [Voice::Plain, Voice::Luna] {
        let f = Formatter::new(
            us(),
            voice,
            whether_core::presentation::TimeFormat::H12,
            "America/New_York",
        )
        .with_locale(Locale::PSEUDO);
        let s = f.strings;
        let mut texts: Vec<String> = [
            Key::PackWifiOnly,
            Key::PackWifiOnlyDetail,
            Key::PackWaitingForWifi,
            Key::PackBackgroundRefresh,
            Key::PackBaseMapNote,
            Key::PackStatusDownloading,
            Key::PackStatusPaused,
            Key::PackStatusDone,
            Key::PackStatusFailed,
            Key::PackStatusCancelled,
            Key::PackStatusNotDownloaded,
        ]
        .into_iter()
        .map(|key| s.get(key).to_owned())
        .collect();
        for reach in ZoomReach::ALL {
            texts.push(s.pack_zoom_title(reach));
            texts.push(s.pack_zoom_detail(reach).to_owned());
        }
        for dates in [
            DatePolicy::Latest,
            DatePolicy::LastDays(1),
            DatePolicy::LastDays(3),
            DatePolicy::Days(vec![day]),
            DatePolicy::Days(vec![day, LocalDate { day: 21, ..day }]),
        ] {
            texts.push(s.pack_dates(&dates));
            texts.push(s.pack_dates_title(&dates));
            texts.push(s.pack_dates_detail(&dates));
        }
        texts.extend([
            s.pack_stale(Stale::NewestOverADay).to_owned(),
            s.pack_stale(Stale::OverAWeek).to_owned(),
            s.pack_error(&PlanError::InvalidId),
            s.pack_error(&PlanError::InvalidName),
            s.pack_counts(2_340, 12, 1),
            s.pack_counts(1, 1, 3),
            s.pack_counts(0, 0, 0),
            s.pack_progress(312, 1_024, 3, 3_100_000),
            pack_refreshed(&f, Some(at), at),
            pack_refreshed(&f, Some(at), UnixTime(at.0 + 86_400)),
            pack_refreshed(&f, None, at),
        ]);
        english.extend(english_texts(&texts, &[]));
    }
    assert!(english.is_empty(), "{}", english.join("\n"));
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

#[test]
fn the_log_views_are_translated_in_the_pseudo_locale() {
    use crate::log::insights::{Filter, summarise};
    use crate::view::log::{
        CatchDetail, CatchList, InsightsView, ParseError, catch_detail, catch_list, insights_view,
        tests as log,
    };
    use crate::view::strings::Strings;
    let mut ids = BTreeSet::new();
    for schema in [
        schemars::schema_for!(CatchList),
        schemars::schema_for!(CatchDetail),
        schemars::schema_for!(InsightsView),
    ] {
        identifiers(&serde_json::to_value(schema).unwrap(), &mut ids);
    }
    // What the angler and the data wrote: shown as written.
    let mut data: Vec<String> = [
        "Striped bass",
        "Hickory shad",
        "Perch",
        "The Battery",
        "Lower Hudson",
        "Hudson River entrance",
        "Croton River",
        "Bucktail jig",
        "Casting from the seawall",
        "Second fish of the morning",
        "c1.jpg",
    ]
    .map(str::to_owned)
    .to_vec();
    data.sort_by_key(|t| std::cmp::Reverse(t.len()));
    let catalogue = log::catalogue();
    let mut book = log::bass_book();
    book.upsert(log::full_entry());
    book.upsert(log::bare_entry());
    let filter = Filter::default();
    let mut problems = Vec::new();
    for units in [us(), metric()] {
        for voice in [Voice::Luna, Voice::Plain] {
            let f = log::formatter(units, voice).with_locale(Locale::PSEUDO);
            let places = log::places();
            let empty = crate::log::LogBook::new();
            let mut found = Vec::new();
            for view in [
                serde_json::to_value(catch_detail(&log::full_entry(), &f, &catalogue, &places)),
                serde_json::to_value(catch_detail(&log::bare_entry(), &f, &catalogue, &places)),
                serde_json::to_value(catch_list(&book, &f, &catalogue, &places)),
                serde_json::to_value(catch_list(&empty, &f, &catalogue, &places)),
                serde_json::to_value(insights_view(
                    &summarise(&book, &filter),
                    &filter,
                    &f,
                    &catalogue,
                )),
                serde_json::to_value(insights_view(
                    &summarise(&empty, &filter),
                    &filter,
                    &f,
                    &catalogue,
                )),
            ] {
                strings(&view.unwrap(), "", &mut found);
            }
            for e in [
                ParseError::NotANumber,
                ParseError::NotPositive,
                ParseError::TooLarge,
                ParseError::PoundsNotWhole,
                ParseError::OuncesTooMany,
            ] {
                let words = Strings::localized(voice, Locale::PSEUDO);
                found.push(("error".to_owned(), e.text(words)));
            }
            problems.extend(
                found
                    .into_iter()
                    .filter(|(path, s)| {
                        !matches!(field(path), "key" | "id") && !ids.contains(s.as_str())
                    })
                    .filter_map(|(path, s)| {
                        untranslated(&s, &data)
                            .map(|rest| format!("({voice:?}){path}: {s:?} ({rest:?})"))
                    }),
            );
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn the_species_views_are_translated_in_the_pseudo_locale() {
    use crate::species::profile::Field;
    use crate::species::seasons::MonthObservations;
    use crate::view::species::{
        EditError, EditedBy, ProfilePage, SeasonInput, SeasonView, SpeciesRow, edit_choices,
        profile_page, season_view, species_list, tests as sp,
    };
    use crate::view::strings::Strings;
    let mut ids = BTreeSet::new();
    for schema in [
        schemars::schema_for!(SpeciesRow),
        schemars::schema_for!(ProfilePage),
        schemars::schema_for!(SeasonView),
    ] {
        identifiers(&serde_json::to_value(schema).unwrap(), &mut ids);
    }
    let catalogue = sp::catalogue();
    let (bass, edits) = sp::edited_bass();
    let all: Vec<SpeciesProfile> = catalogue.iter().cloned().collect();
    // What the profiles, the agencies and the angler wrote: shown as written.
    let mut data = profile_texts(&all);
    data.extend(profile_texts(std::slice::from_ref(&bass)));
    data.extend(
        crate::regulations::all_links("US-NY")
            .into_iter()
            .map(|l| l.label),
    );
    data.push("Check the Croton special rules.".to_owned());
    data.sort_by_key(|t| std::cmp::Reverse(t.len()));
    let normals = sp::croton_normals();
    let obs = [MonthObservations {
        month: 5,
        count: 12,
    }];
    let trout = catalogue.get(&sp::id("salmo-trutta")).unwrap();
    let mut problems = Vec::new();
    for units in [us(), metric()] {
        for voice in [Voice::Luna, Voice::Plain] {
            let f = sp::formatter(units, voice).with_locale(Locale::PSEUDO);
            let words = Strings::localized(voice, Locale::PSEUDO);
            let input = SeasonInput {
                water_c: Some(21.5),
                trend: Some(crate::view::TrendDirection::Rising),
                normals: &normals,
                observations: &obs,
                region: Some("US-NY"),
                notes: "Check the Croton special rules.",
                trout_stress_c: Some(20.0),
            };
            let mut found = Vec::new();
            for view in [
                serde_json::to_value(species_list(&catalogue, &edits, words)),
                serde_json::to_value(profile_page(&bass, &f, EditedBy::You)),
                serde_json::to_value(profile_page(&bass, &f, EditedBy::Sender)),
                serde_json::to_value(season_view(&bass, &input, &f, EditedBy::You)),
                serde_json::to_value(season_view(trout, &input, &f, EditedBy::You)),
                serde_json::to_value(season_view(
                    trout,
                    &SeasonInput::default(),
                    &f,
                    EditedBy::You,
                )),
                serde_json::to_value(edit_choices(Field::Light, words)),
                serde_json::to_value(edit_choices(Field::Flow, words)),
                serde_json::to_value(edit_choices(Field::Water, words)),
            ] {
                strings(&view.unwrap(), "", &mut found);
            }
            for e in [
                EditError::NotANumber,
                EditError::BothEnds,
                EditError::Backwards,
                EditError::OutOfRange,
                EditError::Empty,
                EditError::NotAChoice,
            ] {
                found.push(("error".to_owned(), e.text(words)));
            }
            problems.extend(
                found
                    .into_iter()
                    // Links, ids, region codes and the editor's raw values are not text.
                    .filter(|(path, s)| {
                        !matches!(field(path), "link" | "id" | "region")
                            && !ids.contains(s.as_str())
                    })
                    .filter(|(path, _)| !path.contains(".edit["))
                    .filter_map(|(path, s)| {
                        untranslated(&s, &data)
                            .map(|rest| format!("({voice:?}){path}: {s:?} ({rest:?})"))
                    }),
            );
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn the_alert_words_are_translated_in_the_pseudo_locale() {
    use crate::alerts::AlertMetric;
    use crate::view::alerts::{
        ThresholdError, describe_rule, edit_threshold, notification, tests as al,
    };
    // What the user and the sources wrote: shown as written.
    let names = al::names();
    let mut data: Vec<String> = names
        .spots
        .values()
        .chain(names.trips.values())
        .cloned()
        .collect();
    data.extend(
        [
            "Croton R at New Croton Dam",
            "Small Craft Advisory",
            "Hurricane Melissa",
        ]
        .map(str::to_owned),
    );
    let events = al::events();
    data.extend(events.iter().map(|(_, rule)| rule.name.clone()));
    data.sort_by_key(|t| std::cmp::Reverse(t.len()));
    let mut problems = Vec::new();
    for units in [us(), metric()] {
        for voice in [Voice::Luna, Voice::Plain] {
            let f = al::formatter(units, voice).with_locale(Locale::PSEUDO);
            let mut texts = Vec::new();
            for (event, rule) in &events {
                for show_name in [true, false] {
                    let rule = crate::alerts::AlertRule {
                        show_name,
                        ..rule.clone()
                    };
                    let n = notification(&f, event, &rule, &names);
                    texts.extend([n.title, n.body]);
                }
                let d = describe_rule(&f, rule, &names);
                texts.extend(d.conditions);
                texts.extend([d.target, d.summary]);
                texts.extend(d.repeat.into_iter().chain(d.quiet));
            }
            for metric in AlertMetric::ALL {
                texts.push(ThresholdError::NotANumber.text(&f, metric));
                texts.push(ThresholdError::OutOfRange.text(&f, metric));
                texts.push(edit_threshold(&f, metric, 1.25));
            }
            problems.extend(texts.iter().filter_map(|t| {
                untranslated(t, &data).map(|rest| format!("({voice:?}) {t:?} ({rest:?})"))
            }));
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}
