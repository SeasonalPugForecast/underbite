//! The spot snapshot contract and the vocabulary around it: measurements, requirements,
//! station candidates and source errors.

use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use whether_core::model::WeatherSnapshot;
use whether_core::storm::{Hurricane, StormOutlook};

use super::marine::MarineConditions;
use super::sky::SkyConditions;
use super::spot::Spot;
use super::water::WaterConditions;
use crate::provenance::{Attribution, SourceId, StationRef};
use crate::time::UnixTime;

/// A kind of value the app can show; the key of source chains, discovery and requirements.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Measurement {
    WaterTemperature,
    Discharge,
    GageHeight,
    LakeLevel,
    Turbidity,
    DissolvedOxygen,
    Conductivity,
    Salinity,
    Ph,
    Chlorophyll,
    Clarity,
    Cyanobacteria,
    Tide,
    TidalCurrent,
    OceanCurrent,
    Waves,
    WavesForecast,
    SeaSurfaceTemp,
    SeaSurfaceTempForecast,
    MarineForecast,
    MarineAlerts,
    FloodCategories,
    FlowForecast,
    ObservedWind,
    ObservedPressure,
}

fn default_past_hours() -> u32 {
    24 * 7
}

fn default_future_days() -> u8 {
    7
}

/// What a refresh must fetch: the measurements wanted and how far back and ahead.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Requirements {
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub measurements: BTreeSet<Measurement>,
    /// Past window in hours, for sparklines (7 days by default).
    #[serde(default = "default_past_hours")]
    pub past_hours: u32,
    /// Future window in days, for predictions.
    #[serde(default = "default_future_days")]
    pub future_days: u8,
}

impl Default for Requirements {
    fn default() -> Self {
        Self {
            measurements: BTreeSet::new(),
            past_hours: default_past_hours(),
            future_days: default_future_days(),
        }
    }
}

/// What a station offers for one measurement.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Coverage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_value_at: Option<UnixTime>,
    /// The station's primary product for this measurement.
    pub primary: bool,
}

/// A station found near a spot and what it offers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StationCandidate {
    pub station: StationRef,
    pub offers: BTreeMap<Measurement, Coverage>,
}

/// Why a source did not deliver.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SourceErrorKind {
    RateLimited,
    NotFound,
    NoData,
    Stale,
    Parse,
    Transport,
    Other,
}

/// A source that failed, with a plain message ("USGS: rate limited, used NWPS stage").
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SourceError {
    pub source: SourceId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub measurement: Option<Measurement>,
    pub kind: SourceErrorKind,
    pub message: String,
}

/// The storm-ahead subset of whether's storm data and nearby NHC hurricanes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StormWatch {
    pub checked_at: UnixTime,
    /// Point series and run times only: no ensembles, grids or past runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub point: Option<StormOutlook>,
    /// Active NHC storms whose cone or track comes within the alert distance.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hurricanes: Vec<Hurricane>,
}

impl StormWatch {
    /// A watch from a full outlook, keeping only its point series and run times.
    pub fn from_outlook(
        checked_at: UnixTime,
        mut outlook: StormOutlook,
        hurricanes: Vec<Hurricane>,
    ) -> Self {
        outlook.ensembles.clear();
        outlook.grids.clear();
        outlook.past_runs.clear();
        outlook.hurricanes.clear();
        outlook.far_hurricanes.clear();
        Self {
            checked_at,
            point: Some(outlook),
            hurricanes,
        }
    }
}

fn is_default<T: Default + PartialEq>(value: &T) -> bool {
    *value == T::default()
}

/// Everything known about one spot at one moment.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SpotSnapshot {
    pub spot: Spot,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weather: Option<WeatherSnapshot>,
    #[serde(default, skip_serializing_if = "is_default")]
    pub water: WaterConditions,
    #[serde(default, skip_serializing_if = "is_default")]
    pub marine: MarineConditions,
    #[serde(default, skip_serializing_if = "is_default")]
    pub sky: SkyConditions,
    pub fetched_at: UnixTime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storm: Option<StormWatch>,
    /// The stations the values came from.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stations: Vec<StationRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attribution: Vec<Attribution>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<SourceError>,
}

impl SpotSnapshot {
    /// A snapshot with only the spot set.
    pub fn new(spot: Spot, fetched_at: UnixTime) -> Self {
        Self {
            spot,
            weather: None,
            water: WaterConditions::default(),
            marine: MarineConditions::default(),
            sky: SkyConditions::default(),
            fetched_at,
            storm: None,
            stations: Vec::new(),
            attribution: Vec::new(),
            errors: Vec::new(),
        }
    }
}

/// Sample snapshots and sections for tests, here and in other crates (feature `testdata`).
#[cfg(any(test, feature = "testdata"))]
#[allow(clippy::too_many_lines, clippy::wildcard_imports)]
pub mod testdata {
    use super::*;
    use crate::datum::{Height, Heights, VerticalDatum};
    use crate::model::marine::*;
    use crate::model::sky::*;
    use crate::model::spot::{WaterBodyRef, WaterType};
    use crate::model::water::*;
    use crate::provenance::{Provenance, Quality, Relation, StationKind, ValueKind};
    use crate::series::{Sample, Series, Vector};
    use crate::time::LocalDate;

    const T0: i64 = 1_794_000_000;

    pub fn station() -> StationRef {
        StationRef {
            source: SourceId::COOPS,
            id: "8518750".into(),
            name: "The Battery".into(),
            kind: StationKind::TideHarmonic,
            latitude: 40.7003,
            longitude: -74.0142,
            bin: None,
            reference_id: None,
        }
    }

    pub fn spot() -> Spot {
        Spot {
            id: "battery".into(),
            name: "The Battery".into(),
            latitude: 40.7003,
            longitude: -74.0142,
            water_type: WaterType::TidalRiverEstuary,
            water_body: Some(WaterBodyRef {
                source: SourceId::USGS_3DHP,
                id: "hudson-lower".into(),
                name: Some("Hudson River".into()),
            }),
            time_zone: "America/New_York".into(),
        }
    }

    fn prov(source: SourceId, kind: ValueKind) -> Provenance {
        Provenance {
            source,
            station: Some(station()),
            distance_m: Some(640.0),
            relation: Relation::SameWaterBody,
            observed_at: UnixTime(T0),
            kind,
            quality: Quality::Provisional,
        }
    }

    fn at(minutes: i64) -> UnixTime {
        UnixTime(T0 + minutes * 60)
    }

    fn series<T>(source: SourceId, kind: ValueKind, values: Vec<T>) -> Series<T> {
        let samples = values
            .into_iter()
            .enumerate()
            .map(|(i, v)| Sample::new(at(i64::try_from(i).unwrap_or_default() * 6), v))
            .collect();
        Series::new(prov(source, kind), samples)
    }

    fn f32s(source: SourceId, kind: ValueKind, values: &[f32]) -> Series<f32> {
        series(source, kind, values.to_vec())
    }

    fn percentile(basis: PercentileBasis) -> Percentile {
        Percentile {
            day_of_year: "11-08".into(),
            thresholds: PercentileThresholds {
                p05: Some(0.02),
                p10: Some(0.03),
                p25: Some(0.04),
                p50: Some(0.42),
                p75: Some(6.7),
                p90: Some(19.5),
                p95: Some(33.4),
            },
            min: Some(0.01),
            max: Some(80.0),
            mean: Some(5.0),
            value: 0.35,
            rank: Some(48.0),
            rank_clamped: false,
            class: Some(PercentileClass::Normal),
            basis,
            sample_count: 93,
            approved_only: true,
            datum: None,
            provenance: Some(prov(SourceId::USGS, ValueKind::Computed)),
        }
    }

    fn train(dir: WaveDirection) -> WaveTrain {
        WaveTrain {
            height_m: Some(0.6),
            period_s: Some(7.0),
            direction_from: Some(dir),
        }
    }

    fn waves(source: SourceId, kind: ValueKind) -> WaveSeries {
        let sample = WaveSample {
            significant_height_m: Some(0.9),
            dominant_period_s: Some(8.0),
            mean_period_s: Some(5.5),
            direction_from_deg: Some(120.0),
            wind_waves: Some(train(WaveDirection::Compass("SE".into()))),
            swells: vec![train(WaveDirection::Degrees(150.0))],
            steepness: Some("AVERAGE".into()),
        };
        series(source, kind, vec![sample.clone(), sample])
    }

    pub fn sample_water() -> WaterConditions {
        WaterConditions {
            temperature_c: Some(f32s(SourceId::USGS, ValueKind::Observed, &[14.5, 14.6])),
            discharge_m3s: Some(f32s(SourceId::USGS, ValueKind::Observed, &[0.35, 0.34])),
            gage_height: Some(Heights {
                datum: VerticalDatum::Navd88,
                series: f32s(SourceId::USGS, ValueKind::Observed, &[1.2, 1.21]),
            }),
            level: Some(Heights {
                datum: VerticalDatum::Ngvd29,
                series: f32s(SourceId::USGS, ValueKind::Observed, &[230.1, 230.2]),
            }),
            turbidity: Some(series(
                SourceId::USGS,
                ValueKind::Observed,
                vec![Turbidity {
                    value: 3.1,
                    unit: TurbidityUnit::Fnu,
                }],
            )),
            dissolved_oxygen_mg_l: Some(f32s(SourceId::USGS, ValueKind::Observed, &[8.2])),
            conductivity_us_cm: Some(f32s(SourceId::USGS, ValueKind::Observed, &[410.0])),
            salinity_psu: Some(f32s(SourceId::NDBC, ValueKind::Observed, &[24.0])),
            ph: Some(f32s(SourceId::USGS, ValueKind::Observed, &[7.8])),
            chlorophyll_ug_l: Some(f32s(SourceId::USGS, ValueKind::Observed, &[2.5])),
            clarity_kd490_per_m: Some(f32s(
                SourceId::new("copernicus"),
                ValueKind::Satellite,
                &[0.4],
            )),
            cyanobacteria_cells_ml: Some(f32s(
                SourceId::new("cyan"),
                ValueKind::Satellite,
                &[1500.0],
            )),
            discharge_forecast_m3s: Some(f32s(SourceId::NWPS, ValueKind::Predicted, &[0.5, 0.6])),
            stage_forecast: Some(Heights {
                datum: VerticalDatum::Gauge,
                series: f32s(SourceId::NWPS, ValueKind::Predicted, &[1.3, 1.4]),
            }),
            flood_categories: Some(FloodCategories {
                action: Some(Height {
                    datum: VerticalDatum::Gauge,
                    metres: 2.4,
                }),
                minor: Some(Height {
                    datum: VerticalDatum::Gauge,
                    metres: 2.4,
                }),
                moderate: None,
                major: Some(Height {
                    datum: VerticalDatum::Gauge,
                    metres: 3.0,
                }),
            }),
            discharge_percentile_for_date: Some(percentile(PercentileBasis::StatisticsApi)),
            level_percentile_for_date: Some(Percentile {
                datum: Some(VerticalDatum::Ngvd29),
                ..percentile(PercentileBasis::ComputedFromDaily { years: 16 })
            }),
        }
    }

    pub fn sample_marine() -> MarineConditions {
        MarineConditions {
            tide: Some(TideSeries {
                source_kind: TideSourceKind::HarmonicStation,
                datum: VerticalDatum::Mllw,
                events: vec![
                    TideEvent {
                        time: at(10),
                        kind: TideKind::High,
                        height_m: Some(1.71),
                    },
                    TideEvent {
                        time: at(400),
                        kind: TideKind::Low,
                        height_m: None,
                    },
                ],
                curve: Some(f32s(
                    SourceId::COOPS,
                    ValueKind::Predicted,
                    &[1.5, 1.6, 1.7],
                )),
                observed: Some(f32s(SourceId::COOPS, ValueKind::Observed, &[1.4, 1.5])),
                provenance: prov(SourceId::COOPS, ValueKind::Predicted),
            }),
            tidal_current: Some(CurrentSeries {
                events: vec![
                    CurrentEvent {
                        time: at(30),
                        kind: CurrentKind::MaxEbb,
                        speed_ms: -0.8,
                    },
                    CurrentEvent {
                        time: at(200),
                        kind: CurrentKind::Slack,
                        speed_ms: 0.0,
                    },
                ],
                curve: Some(f32s(SourceId::COOPS, ValueKind::Predicted, &[-0.8, 0.0])),
                flood_dir_deg: Some(20.0),
                ebb_dir_deg: Some(200.0),
                bin: Some(2),
                depth_m: Some(4.5),
                observed: Some(series(
                    SourceId::COOPS,
                    ValueKind::Observed,
                    vec![Vector {
                        speed_ms: 0.5,
                        toward_deg: 190.0,
                    }],
                )),
                provenance: prov(SourceId::COOPS, ValueKind::Predicted),
            }),
            ocean_current: Some(series(
                SourceId::new("copernicus"),
                ValueKind::Modelled,
                vec![Vector {
                    speed_ms: 0.2,
                    toward_deg: 45.0,
                }],
            )),
            waves: Some(waves(SourceId::NDBC, ValueKind::Observed)),
            sea_surface_temp_c: Some(f32s(SourceId::NDBC, ValueKind::Observed, &[15.0])),
            waves_forecast: Some(waves(SourceId::OPEN_METEO_MARINE, ValueKind::Modelled)),
            sea_surface_temp_forecast_c: Some(f32s(
                SourceId::OPEN_METEO_MARINE,
                ValueKind::Modelled,
                &[15.2, 15.3],
            )),
            observed_wind: Some(series(
                SourceId::NDBC,
                ValueKind::Observed,
                vec![Wind {
                    speed_ms: 6.0,
                    gust_ms: Some(8.0),
                    from_deg: Some(250.0),
                }],
            )),
            observed_pressure_hpa: Some(f32s(SourceId::NDBC, ValueKind::Observed, &[1013.2])),
            marine_forecast: Some(ZoneForecast {
                id: "ANZ338".into(),
                name: "New York Harbor".into(),
                kind: ZoneKind::Coastal,
                issued: at(-60),
                product: ZoneProduct::ZoneApi,
                periods: vec![ForecastPeriod {
                    name: "Tonight".into(),
                    text: "W winds 10 to 15 kt. Waves 1 ft or less.".into(),
                }],
            }),
            marine_alerts: vec![MarineAlert {
                id: "urn:oid:example".into(),
                event: "Small Craft Advisory".into(),
                headline: Some("Small Craft Advisory in effect".into()),
                description: None,
                instruction: None,
                onset: Some(at(0)),
                ends: Some(at(600)),
                expires: Some(at(300)),
                severity: AlertSeverity::Minor,
                zones: vec!["ANZ338".into()],
                vtec_key: Some("KOKX.SC.Y.0100".into()),
            }],
        }
    }

    pub fn sample_sky() -> SkyConditions {
        SkyConditions {
            days: vec![SkyDay {
                date: LocalDate {
                    year: 2026,
                    month: 11,
                    day: 15,
                },
                sun: SunTimes {
                    polar: None,
                    sunrise: Some(at(20)),
                    sunset: Some(at(560)),
                    civil_dawn: Some(at(-10)),
                    civil_dusk: Some(at(590)),
                    nautical_dawn: Some(at(-45)),
                    nautical_dusk: Some(at(625)),
                    golden_morning_end: Some(at(80)),
                    golden_evening_start: Some(at(500)),
                },
                moon: MoonDay {
                    rise: Some(at(100)),
                    set: None,
                    upper_transit: Some(at(400)),
                    lower_transit: Some(at(800)),
                    elongation_deg: 120.0,
                    illuminated_fraction: 0.75,
                    phase: MoonPhase::WaxingGibbous,
                    nearest_principal: Some(PrincipalPhaseTime {
                        phase: PrincipalPhase::Full,
                        time: at(3000),
                    }),
                },
                solunar: vec![SolunarPeriod {
                    kind: SolunarKind::Major,
                    start: at(340),
                    end: at(460),
                    cause: SolunarCause::UpperTransit,
                }],
            }],
        }
    }

    pub fn sample_snapshot() -> SpotSnapshot {
        let storm = StormWatch::from_outlook(
            at(0),
            whether_core::storm::testdata::noreaster(),
            vec![whether_core::storm::testdata::hurricane()],
        );

        SpotSnapshot {
            spot: spot(),
            weather: Some(whether_core::testdata::sample_snapshot()),
            water: sample_water(),
            marine: sample_marine(),
            sky: sample_sky(),
            fetched_at: at(0),
            storm: Some(storm),
            stations: vec![station()],
            attribution: vec![Attribution {
                source: SourceId::COOPS,
                credit: "NOAA CO-OPS".into(),
                url: Some("https://tidesandcurrents.noaa.gov".into()),
            }],
            errors: vec![SourceError {
                source: SourceId::USGS,
                measurement: Some(Measurement::Discharge),
                kind: SourceErrorKind::RateLimited,
                message: "USGS: rate limited, used NWPS stage".into(),
            }],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testdata::*;
    use super::*;

    #[test]
    fn full_snapshot_round_trips() {
        let snap = sample_snapshot();
        let json = serde_json::to_string(&snap).unwrap();
        let back: SpotSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(back, snap);
        assert_eq!(serde_json::to_string(&back).unwrap(), json);
    }

    #[test]
    fn storm_watch_drops_ensembles_grids_and_past_runs() {
        let snap = sample_snapshot();
        let point = snap.storm.unwrap().point.unwrap();
        assert_ne!(point.points.len(), 0);
        assert!(point.ensembles.is_empty() && point.grids.is_empty() && point.past_runs.is_empty());
    }

    #[test]
    fn spot_only_snapshot_is_compact() {
        let snap = SpotSnapshot::new(spot(), UnixTime(1_794_000_000));
        let value = serde_json::to_value(&snap).unwrap();
        let keys: Vec<&str> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(keys, ["fetched_at", "spot"]);
        let back: SpotSnapshot = serde_json::from_value(value).unwrap();
        assert_eq!(back, snap);
    }

    #[test]
    fn old_files_without_new_fields_decode() {
        let json = r#"{"spot":{"id":"a","name":"A","latitude":1.0,"longitude":2.0,
            "water_type":"pond","time_zone":"UTC"},"fetched_at":5}"#;
        let snap: SpotSnapshot = serde_json::from_str(json).unwrap();
        assert!(snap.errors.is_empty() && snap.storm.is_none());
    }

    #[test]
    fn requirements_default_to_seven_days_each_way() {
        let r: Requirements = serde_json::from_str("{}").unwrap();
        assert_eq!(r, Requirements::default());
        assert_eq!((r.past_hours, r.future_days), (168, 7));
    }

    #[test]
    fn station_candidate_round_trips_with_ordered_offers() {
        let mut offers = BTreeMap::new();
        offers.insert(
            Measurement::Tide,
            Coverage {
                last_value_at: Some(UnixTime(10)),
                primary: true,
            },
        );
        offers.insert(
            Measurement::WaterTemperature,
            Coverage {
                last_value_at: None,
                primary: false,
            },
        );
        let c = StationCandidate {
            station: station(),
            offers,
        };
        let json = serde_json::to_string(&c).unwrap();
        assert!(json.find("water_temperature").unwrap() < json.find("\"tide\"").unwrap());
        assert_eq!(serde_json::from_str::<StationCandidate>(&json).unwrap(), c);
    }

    #[test]
    fn schema_snapshot() {
        let schemas = serde_json::json!({
            "SpotSnapshot": schemars::schema_for!(SpotSnapshot),
            "Requirements": schemars::schema_for!(Requirements),
            "StationCandidate": schemars::schema_for!(StationCandidate),
        });
        insta::assert_json_snapshot!(schemas);
    }
}
