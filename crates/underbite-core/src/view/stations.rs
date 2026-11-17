//! The station choices of a spot, as rows: per measurement, the stations discovery ranked and
//! those it left out, every word and distance written here.
//!
//! The input is a discovery's rankings by measurement ([`MeasurementRanking`]); a row holds the
//! station's name, its source, its distance in the user's unit, how it relates to the spot and
//! the reasons for its place (or for leaving it out) as text, with the key the pin calls take.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::discover::rank::{DroppedStation, MeasurementRanking, RankedStation, Reason};
use crate::model::Measurement;
use crate::provenance::{Relation, StationRef};
use crate::time::UnixTime;
use crate::view::format::Formatter;
use crate::view::strings::{Key, MEASUREMENTS};

/// One station of a measurement.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct StationRow {
    /// The station as the pin calls take it: `coops:8518750`, `coops:NYH1927:13`.
    pub key: String,
    /// The station's name as its source writes it.
    pub name: String,
    /// The source's id (`coops`).
    pub source: String,
    /// The source's name: "NOAA CO-OPS".
    pub source_name: String,
    /// The station's id with its bin: "8518750", "NYH1927 · Bin 13".
    pub station: String,
    /// "0.4 mi".
    pub distance: String,
    /// "Same water body", "Nearby", ...; a station left out has none.
    pub relation: Option<String>,
    /// The line under the name: "NOAA CO-OPS 8518750 · 0.4 mi · Nearby".
    pub detail: String,
    /// Why it has its place, or why it was left out: "Latest value 6 min ago", "Pinned by you".
    pub reasons: Vec<String>,
    /// The user always uses it.
    pub pinned: bool,
    /// The user never uses it.
    pub excluded: bool,
    /// The ranking uses it (in the order of the rows).
    pub used: bool,
}

/// The stations of one measurement.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct StationGroup {
    pub measurement: Measurement,
    /// "Tide", "River flow", ...
    pub title: String,
    /// The stations used, best first, then those the user excluded.
    pub rows: Vec<StationRow>,
    /// "3 not used", when stations other than the excluded ones were left out.
    pub left_out_title: Option<String>,
    /// Those stations, each with why it was left out.
    pub left_out: Vec<StationRow>,
}

/// The rows of `rankings` at Unix time `now`, one group per measurement with a station, in the
/// order of [`MEASUREMENTS`].
pub fn station_rows(
    rankings: &BTreeMap<Measurement, MeasurementRanking>,
    f: &Formatter,
    now: UnixTime,
) -> Vec<StationGroup> {
    MEASUREMENTS
        .iter()
        .filter_map(|m| rankings.get(m).map(|r| group(*m, r, f, now)))
        .filter(|g| !g.rows.is_empty() || !g.left_out.is_empty())
        .collect()
}

fn group(
    measurement: Measurement,
    ranking: &MeasurementRanking,
    f: &Formatter,
    now: UnixTime,
) -> StationGroup {
    let mut rows: Vec<StationRow> = ranking.stations.iter().map(|s| used(s, f, now)).collect();
    let (excluded, others): (Vec<&DroppedStation>, Vec<&DroppedStation>) = ranking
        .dropped
        .iter()
        .partition(|d| d.reason == Reason::Excluded);
    rows.extend(excluded.into_iter().map(|d| left_out(d, f, now)));
    let mut others: Vec<&DroppedStation> = others;
    others.sort_by(|a, b| a.distance_m.total_cmp(&b.distance_m));
    let left_out: Vec<StationRow> = others.into_iter().map(|d| left_out(d, f, now)).collect();
    StationGroup {
        measurement,
        title: f.strings.measurement(measurement).to_owned(),
        rows,
        left_out_title: (!left_out.is_empty()).then(|| {
            f.strings
                .fmt(Key::StationsNotUsed, &[("n", &left_out.len().to_string())])
        }),
        left_out,
    }
}

fn used(s: &RankedStation, f: &Formatter, now: UnixTime) -> StationRow {
    let relation = f.strings.relation(s.relation).to_owned();
    let reasons = s
        .reasons
        .iter()
        // The distance and the relation have their own places in the row.
        .filter(|r| !matches!(r, Reason::Distance { .. } | Reason::Relation { .. }))
        // Coverage is only in question while nothing says the station reports or not.
        .filter(|r| **r != Reason::Listed || !s.reasons.iter().any(reports_or_not))
        .map(|r| reason_text(f, r, now))
        .collect();
    row(&s.station, s.distance_m, Some(relation), reasons, f, |r| {
        r.pinned = s.pinned;
        r.used = true;
    })
}

fn left_out(d: &DroppedStation, f: &Formatter, now: UnixTime) -> StationRow {
    row(
        &d.station,
        d.distance_m,
        None,
        vec![reason_text(f, &d.reason, now)],
        f,
        |r| r.excluded = d.reason == Reason::Excluded,
    )
}

fn row(
    station: &StationRef,
    distance_m: f32,
    relation: Option<String>,
    reasons: Vec<String>,
    f: &Formatter,
    finish: impl FnOnce(&mut StationRow),
) -> StationRow {
    let source = f.strings.source_name(&station.source);
    let id = match station.bin {
        Some(bin) => format!(
            "{} · {}",
            station.id,
            f.strings.fmt(Key::BinOnly, &[("bin", &bin.to_string())])
        ),
        None => station.id.clone(),
    };
    let distance = f.distance(Some(distance_m)).full();
    let mut detail = vec![format!("{source} {id}"), distance.clone()];
    detail.extend(relation.clone());
    let mut row = StationRow {
        key: key(station),
        name: station.name.clone(),
        source: station.source.as_str().to_owned(),
        source_name: source,
        station: id,
        distance,
        relation,
        detail: detail.join(" · "),
        reasons,
        pinned: false,
        excluded: false,
        used: false,
    };
    finish(&mut row);
    row
}

/// `source:id` or `source:id:bin`, as the pin calls read it.
fn key(station: &StationRef) -> String {
    match station.bin {
        Some(bin) => format!("{}:{}:{bin}", station.source, station.id),
        None => format!("{}:{}", station.source, station.id),
    }
}

/// A reason that settles whether the station reports.
fn reports_or_not(reason: &Reason) -> bool {
    matches!(reason, Reason::Reported { .. } | Reason::NotReported { .. })
}

/// A relation in words, in the formatter's voice.
pub fn relation_text(f: &Formatter, relation: Relation) -> &'static str {
    f.strings.relation(relation)
}

/// A ranking reason in words; distances and ages come from the formatter.
pub fn reason_text(f: &Formatter, reason: &Reason, now: UnixTime) -> String {
    let s = f.strings;
    match reason {
        Reason::Pinned => s.get(Key::StationPinned).to_owned(),
        Reason::Excluded => s.get(Key::StationExcluded).to_owned(),
        Reason::TooFar {
            distance_m,
            limit_m,
        } => f.far_reason(*distance_m, *limit_m),
        Reason::Stale { age_s, limit_s } => f.stale_reason(*age_s, *limit_s),
        Reason::WeakAndVariable => s.get(Key::StationWeakAndVariable).to_owned(),
        Reason::OtherBin { kept } => s.fmt(Key::StationOtherBin, &[("bin", &kept.to_string())]),
        Reason::Reported { at } => s.fmt(Key::StationReported, &[("age", &f.age(now.0 - at.0))]),
        Reason::Listed => s.get(Key::StationListed).to_owned(),
        Reason::NotReported { checked_at } => s.fmt(
            Key::StationNotReported,
            &[("age", &f.age(now.0 - checked_at.0))],
        ),
        Reason::Relation { relation } => s.relation(*relation).to_owned(),
        Reason::OtherWaterBody { .. } => s.get(Key::StationOtherWater).to_owned(),
        Reason::NotPlaced => s.get(Key::StationNotPlaced).to_owned(),
        Reason::ForecastGaugeFor { site } => {
            s.fmt(Key::StationForecastGauge, &[("site", &site.id)])
        }
        Reason::Secondary => s.get(Key::StationSecondary).to_owned(),
        Reason::Distance { distance_m } => f.distance(Some(*distance_m)).full(),
    }
}

#[cfg(test)]
mod tests {
    use whether_core::presentation::TimeFormat;

    use super::*;
    use crate::discover::rank::{ChainSpec, CoverageState, Limits, StationKey};
    use crate::provenance::{Relation, SourceId, StationKind, UsgsSiteType};
    use crate::units::{
        DischargeUnit, DistanceUnit, HeightUnit, PressureUnit, SpeedUnit, TempUnit,
    };
    use crate::view::{DisplayUnits, Voice};

    const NOW: UnixTime = UnixTime(1_794_744_000);

    fn units(distance: DistanceUnit) -> DisplayUnits {
        DisplayUnits {
            temperature: TempUnit::F,
            air_temperature: TempUnit::F,
            height: HeightUnit::Ft,
            stage: HeightUnit::Ft,
            lake_level: HeightUnit::Ft,
            waves: HeightUnit::Ft,
            discharge: DischargeUnit::Cfs,
            current_speed: SpeedUnit::Kn,
            wind_speed: SpeedUnit::Mph,
            pressure: PressureUnit::InHg,
            distance,
        }
    }

    fn formatter(distance: DistanceUnit, voice: Voice) -> Formatter {
        Formatter::new(units(distance), voice, TimeFormat::H12, "America/New_York")
    }

    fn station(source: SourceId, id: &str, name: &str, kind: StationKind) -> StationRef {
        StationRef {
            source,
            id: id.into(),
            name: name.into(),
            kind,
            latitude: 40.7,
            longitude: -74.0,
            bin: None,
            reference_id: None,
        }
    }

    fn ranked(station: StationRef, distance_m: f32, reasons: Vec<Reason>) -> RankedStation {
        RankedStation {
            station,
            distance_m,
            relation: Relation::Nearby,
            coverage: CoverageState::Reported,
            last_value_at: None,
            age_s: None,
            primary: true,
            pinned: reasons.contains(&Reason::Pinned),
            reasons,
        }
    }

    fn ranking(stations: Vec<RankedStation>, dropped: Vec<DroppedStation>) -> MeasurementRanking {
        MeasurementRanking {
            chain: ChainSpec {
                sources: vec![SourceId::COOPS],
                limits: Limits::default(),
            },
            stations,
            dropped,
        }
    }

    fn battery() -> StationRef {
        station(
            SourceId::COOPS,
            "8518750",
            "The Battery",
            StationKind::TideHarmonic,
        )
    }

    fn sample() -> BTreeMap<Measurement, MeasurementRanking> {
        let mut narrows = station(
            SourceId::COOPS,
            "NYH1927",
            "The Narrows",
            StationKind::CurrentHarmonic,
        );
        narrows.bin = Some(13);
        let mut deeper = narrows.clone();
        deeper.bin = Some(15);
        let mut out = BTreeMap::new();
        out.insert(
            Measurement::WaterTemperature,
            ranking(
                vec![ranked(
                    battery(),
                    643.7,
                    vec![
                        Reason::Pinned,
                        Reason::Reported {
                            at: UnixTime(NOW.0 - 360),
                        },
                        Reason::Distance { distance_m: 643.7 },
                    ],
                )],
                vec![
                    DroppedStation {
                        station: station(SourceId::NDBC, "44025", "Long Island", StationKind::Buoy),
                        distance_m: 90_000.0,
                        reason: Reason::TooFar {
                            distance_m: 90_000.0,
                            limit_m: 50_000.0,
                        },
                    },
                    DroppedStation {
                        station: station(
                            SourceId::COOPS,
                            "8519483",
                            "Bergen Point",
                            StationKind::WaterLevel,
                        ),
                        distance_m: 9_000.0,
                        reason: Reason::Excluded,
                    },
                ],
            ),
        );
        out.insert(
            Measurement::TidalCurrent,
            ranking(
                vec![ranked(
                    narrows,
                    1_200.0,
                    vec![
                        Reason::Listed,
                        Reason::Distance {
                            distance_m: 1_200.0,
                        },
                    ],
                )],
                vec![DroppedStation {
                    station: deeper,
                    distance_m: 1_200.0,
                    reason: Reason::OtherBin { kept: 13 },
                }],
            ),
        );
        out.insert(
            Measurement::Tide,
            ranking(
                vec![ranked(
                    battery(),
                    643.7,
                    vec![Reason::Distance { distance_m: 643.7 }],
                )],
                vec![],
            ),
        );
        out.insert(Measurement::Waves, ranking(vec![], vec![]));
        out
    }

    #[test]
    fn groups_follow_the_list_order_and_skip_empty_measurements() {
        let groups = station_rows(&sample(), &formatter(DistanceUnit::Mi, Voice::Plain), NOW);
        let titles: Vec<&str> = groups.iter().map(|g| g.title.as_str()).collect();
        assert_eq!(titles, ["Tide", "Tidal current", "Water temperature"]);
    }

    #[test]
    fn a_used_row_has_its_distance_relation_and_reasons_in_words() {
        let groups = station_rows(&sample(), &formatter(DistanceUnit::Mi, Voice::Plain), NOW);
        let temperature = &groups[2];
        let battery = &temperature.rows[0];
        assert_eq!(battery.key, "coops:8518750");
        assert_eq!(battery.name, "The Battery");
        assert_eq!(battery.source_name, "NOAA CO-OPS");
        assert_eq!(battery.distance, "0.4 mi");
        assert_eq!(battery.relation.as_deref(), Some("Nearby"));
        assert_eq!(battery.detail, "NOAA CO-OPS 8518750 · 0.4 mi · Nearby");
        assert_eq!(battery.reasons, ["Pinned by you", "Latest value 6 min ago"]);
        assert!(battery.pinned && battery.used && !battery.excluded);

        let km = station_rows(&sample(), &formatter(DistanceUnit::Km, Voice::Plain), NOW);
        assert_eq!(km[2].rows[0].distance, "0.6 km");
    }

    #[test]
    fn an_exclusion_stays_a_row_and_the_others_left_out_say_why() {
        let groups = station_rows(&sample(), &formatter(DistanceUnit::Mi, Voice::Plain), NOW);
        let temperature = &groups[2];
        let keys: Vec<&str> = temperature.rows.iter().map(|r| r.key.as_str()).collect();
        assert_eq!(keys, ["coops:8518750", "coops:8519483"]);
        let bergen = &temperature.rows[1];
        assert!(bergen.excluded && !bergen.used && !bergen.pinned);
        assert_eq!(bergen.reasons, ["Excluded by you"]);
        assert_eq!(bergen.relation, None);
        assert_eq!(bergen.detail, "NOAA CO-OPS 8519483 · 5.6 mi");

        assert_eq!(temperature.left_out_title.as_deref(), Some("1 not used"));
        let far = &temperature.left_out[0];
        assert_eq!(far.key, "ndbc:44025");
        assert_eq!(
            far.reasons,
            ["56 mi from your spot, beyond your 31 mi limit"]
        );
        assert!(!far.excluded && !far.used);
    }

    #[test]
    fn a_current_bin_is_in_its_key_and_its_line() {
        let groups = station_rows(&sample(), &formatter(DistanceUnit::Mi, Voice::Plain), NOW);
        let current = &groups[1];
        assert_eq!(current.rows[0].key, "coops:NYH1927:13");
        assert_eq!(current.rows[0].station, "NYH1927 · Bin 13");
        assert_eq!(
            current.rows[0].reasons,
            ["Listed by the source; not checked yet"]
        );
        assert_eq!(current.left_out[0].key, "coops:NYH1927:15");
        assert_eq!(
            current.left_out[0].reasons,
            ["Bin 13 of this station is used instead"]
        );
        assert_eq!(groups[0].left_out_title, None);
    }

    #[test]
    fn rows_and_reasons_are_translated_in_the_pseudo_locale() {
        use crate::view::locale::Locale;
        let rankings = sample();
        // The data: station names and ids, source ids, measurement names.
        let mut data: Vec<String> = Vec::new();
        for (m, r) in &rankings {
            data.push(
                serde_json::to_value(m)
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .to_owned(),
            );
            data.extend(r.chain.sources.iter().map(|s| s.as_str().to_owned()));
            for s in r
                .stations
                .iter()
                .map(|s| &s.station)
                .chain(r.dropped.iter().map(|d| &d.station))
            {
                data.extend([s.name.clone(), s.id.clone(), s.source.as_str().to_owned()]);
            }
        }
        data.push("USGS-01375000".to_owned());
        data.sort_by_key(|t| std::cmp::Reverse(t.len()));
        for voice in [Voice::Plain, Voice::Luna] {
            let f = formatter(DistanceUnit::Mi, voice).with_locale(Locale::PSEUDO);
            let groups = station_rows(&rankings, &f, NOW);
            let site = StationKey::new(SourceId::USGS, "USGS-01375000");
            let reasons: Vec<String> = [
                Reason::Pinned,
                Reason::Excluded,
                Reason::TooFar {
                    distance_m: 9_000.0,
                    limit_m: 5_000.0,
                },
                Reason::Stale {
                    age_s: 7_300,
                    limit_s: 3_600,
                },
                Reason::WeakAndVariable,
                Reason::OtherBin { kept: 13 },
                Reason::Reported {
                    at: UnixTime(NOW.0 - 360),
                },
                Reason::Listed,
                Reason::NotReported {
                    checked_at: UnixTime(NOW.0 - 7_200),
                },
                Reason::Relation {
                    relation: Relation::Upstream,
                },
                Reason::OtherWaterBody {
                    relation: Relation::Nearby,
                },
                Reason::NotPlaced,
                Reason::ForecastGaugeFor { site },
                Reason::Secondary,
                Reason::Distance {
                    distance_m: 1_609.344,
                },
            ]
            .iter()
            .map(|r| reason_text(&f, r, NOW))
            .collect();
            let json = serde_json::json!({ "groups": groups, "reasons": reasons });
            let english = crate::view::pseudo_tests::english_in(&json, &data);
            assert!(english.is_empty(), "{}", english.join("\n"));
        }
    }

    #[test]
    fn every_reason_reads_as_words() {
        let f = formatter(DistanceUnit::Mi, Voice::Plain);
        let site = StationKey::new(SourceId::USGS, "USGS-01375000");
        let text = |r: Reason| reason_text(&f, &r, NOW);
        assert_eq!(
            text(Reason::Stale {
                age_s: 2 * 86_400,
                limit_s: 86_400
            }),
            "2 d old, beyond your 1 d limit"
        );
        assert_eq!(
            text(Reason::NotReported {
                checked_at: UnixTime(NOW.0 - 7_200)
            }),
            "Checked 2 h ago: no recent values"
        );
        assert_eq!(
            text(Reason::ForecastGaugeFor { site }),
            "Forecast gauge for USGS-01375000"
        );
        assert_eq!(
            text(Reason::OtherWaterBody {
                relation: Relation::Nearby
            }),
            "On another river or a tributary"
        );
        assert_eq!(
            text(Reason::Relation {
                relation: Relation::Upstream
            }),
            "Upstream"
        );
        assert_eq!(
            text(Reason::WeakAndVariable),
            "Weak and variable current: no predictions"
        );
        assert_eq!(text(Reason::Secondary), "Not the station's main product");
        assert_eq!(
            text(Reason::NotPlaced),
            "Not on the mapped streams, so maybe another river"
        );
        assert_eq!(
            text(Reason::Distance {
                distance_m: 1_609.344
            }),
            "1.0 mi"
        );
    }

    #[test]
    fn a_usgs_site_reads_from_the_cores_json() {
        // A USGS site's kind is an object in the JSON, not a string.
        let site = station(
            SourceId::USGS,
            "USGS-01375000",
            "Croton River at New Croton Dam",
            StationKind::UsgsSite(UsgsSiteType::Stream),
        );
        let mut rankings = BTreeMap::new();
        let mut kept = ranked(
            site,
            2_000.0,
            vec![
                Reason::Relation {
                    relation: Relation::Upstream,
                },
                Reason::Distance {
                    distance_m: 2_000.0,
                },
            ],
        );
        kept.relation = Relation::Upstream;
        rankings.insert(Measurement::Discharge, ranking(vec![kept], vec![]));
        let json = serde_json::to_string(&rankings).expect("json");
        assert!(json.contains(r#""kind":{"usgs_site":"stream"}"#), "{json}");
        let read: BTreeMap<Measurement, MeasurementRanking> =
            serde_json::from_str(&json).expect("read");
        let groups = station_rows(&read, &formatter(DistanceUnit::Mi, Voice::Plain), NOW);
        assert_eq!(groups[0].title, "River flow");
        assert_eq!(groups[0].rows[0].key, "usgs:USGS-01375000");
        assert_eq!(groups[0].rows[0].relation.as_deref(), Some("Upstream"));
        assert_eq!(groups[0].rows[0].reasons, Vec::<String>::new());
    }

    #[test]
    fn luna_words_the_count_her_way() {
        let groups = station_rows(&sample(), &formatter(DistanceUnit::Mi, Voice::Luna), NOW);
        assert_eq!(
            groups[2].left_out_title.as_deref(),
            Some("Luna passed over 1")
        );
        assert_eq!(groups[2].rows[0].reasons[0], "Pinned by you");
        assert_eq!(
            groups[1].rows[0].reasons,
            ["Listed by the source; Luna has not checked yet"]
        );
    }

    #[test]
    fn listed_says_nothing_beside_a_reading() {
        let f = formatter(DistanceUnit::Mi, Voice::Plain);
        let both = ranked(
            battery(),
            643.7,
            vec![
                Reason::Listed,
                Reason::Reported {
                    at: UnixTime(NOW.0 - 360),
                },
            ],
        );
        assert_eq!(used(&both, &f, NOW).reasons, ["Latest value 6 min ago"]);
    }
}
