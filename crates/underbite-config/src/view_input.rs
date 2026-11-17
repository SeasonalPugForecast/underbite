//! The configuration as the view's input: what `build_view` takes, built the same way for the
//! command line and the app, and the per-measurement source groups and limits it draws on.

use underbite_core::model::{Measurement, SpotSnapshot};
use underbite_core::provenance::SourceId;
use underbite_core::time::UnixTime;
use underbite_core::trip::TripLimits;
use underbite_core::trip::windows::{FactorWeights, TideRule, WindowParams};
use underbite_core::view::{MeasurementLimit, RiverRanges, SolunarSettings, ViewInput};

use crate::limits::LimitsProfile;
use crate::rivers::RiverRange;
use crate::schema::Config;
use crate::sources::{SourceGroup, SourcesConfig};
use crate::windows::WindowsConfig;

/// Station observations of wind and pressure have no `[sources]` group: they come from CO-OPS
/// and NDBC stations, nearest first, and count only while fresh.
pub const OBSERVATION_CHAIN: [SourceId; 2] = [SourceId::COOPS, SourceId::NDBC];
/// How old a station's wind or pressure may be, minutes.
pub const OBSERVATION_MAX_AGE_MINUTES: u32 = 180;

/// The `[sources]` table that configures `measurement`, by name; `None` for station
/// observations of wind and pressure, which have a fixed chain.
pub fn group_name(measurement: Measurement) -> Option<&'static str> {
    use Measurement as M;
    Some(match measurement {
        M::WaterTemperature => "water_temperature",
        M::Tide => "tide",
        M::TidalCurrent | M::OceanCurrent => "currents",
        M::Discharge | M::GageHeight | M::FlowForecast | M::FloodCategories => "flow",
        M::LakeLevel | M::Reservoir => "lake_level",
        M::Waves | M::WavesForecast => "waves",
        M::SeaSurfaceTemp | M::SeaSurfaceTempForecast => "sst",
        M::Clarity | M::Turbidity => "clarity",
        M::DissolvedOxygen
        | M::Conductivity
        | M::Salinity
        | M::Ph
        | M::Chlorophyll
        | M::Cyanobacteria
        | M::Bloom => "water_quality",
        M::MarineForecast | M::MarineAlerts => "marine_forecast",
        M::ObservedWind | M::ObservedPressure => return None,
    })
}

/// The configured group of `measurement`, or the fixed observation chain.
pub fn group_of(sources: &SourcesConfig, measurement: Measurement) -> SourceGroup {
    let configured = group_name(measurement).and_then(|name| {
        sources
            .groups()
            .into_iter()
            .find(|(n, _)| *n == name)
            .map(|(_, g)| g.clone())
    });
    configured.unwrap_or_else(|| SourceGroup {
        chain: OBSERVATION_CHAIN.to_vec(),
        max_distance_km: None,
        max_age_minutes: Some(OBSERVATION_MAX_AGE_MINUTES),
    })
}

/// The user's distance and age limits per measurement, for greying values in the view.
pub fn measurement_limits(sources: &SourcesConfig) -> Vec<MeasurementLimit> {
    use Measurement as M;
    let all = [
        M::WaterTemperature,
        M::Discharge,
        M::GageHeight,
        M::LakeLevel,
        M::Turbidity,
        M::Tide,
        M::TidalCurrent,
        M::Waves,
        M::SeaSurfaceTemp,
        M::ObservedWind,
        M::ObservedPressure,
    ];
    all.into_iter()
        .filter_map(|m| {
            let group = group_of(sources, m);
            let limit = MeasurementLimit {
                measurement: m,
                max_distance_m: group.max_distance_km.map(|km| (km * 1_000.0) as f32),
                max_age_s: group.max_age_minutes.map(|min| i64::from(min) * 60),
            };
            (limit.max_distance_m.is_some() || limit.max_age_s.is_some()).then_some(limit)
        })
        .collect()
}

/// The user's ranges for the gauge the spot's discharge comes from: the entry for that USGS
/// site, and none when the spot has no USGS discharge or no entry names its gauge.
pub fn river_ranges(rivers: &[RiverRange], snapshot: &SpotSnapshot) -> Option<RiverRanges> {
    let station = snapshot
        .water
        .discharge_m3s
        .as_ref()?
        .provenance()
        .station
        .as_ref()
        .filter(|s| s.source == SourceId::USGS)?;
    gauge_ranges(rivers, &station.id)
}

/// The ranges of the `[[rivers]]` entry for the USGS site `station_id`, as the USGS provider
/// writes it (`USGS-01375000`).
fn gauge_ranges(rivers: &[RiverRange], station_id: &str) -> Option<RiverRanges> {
    RiverRange::find(rivers, station_id).map(RiverRange::to_ranges)
}

/// The settings fishing windows are computed with, from `[windows]`. The tide rule is the core's
/// default: the configuration has no key for it.
pub fn window_params(windows: &WindowsConfig) -> WindowParams {
    let w = &windows.factors;
    WindowParams {
        weights: FactorWeights {
            tide_stage: w.tide_stage,
            current_strength: w.current_strength,
            low_light: w.low_light,
            solunar: w.solunar,
            pressure_falling: w.pressure_falling,
            water_temp: w.water_temp,
            flow_in_range: w.flow_in_range,
        },
        tide: TideRule::default(),
        current_range_kn: windows.current_range_kn,
        show_solunar: windows.show_solunar,
        solunar_major_minutes: windows.solunar_major_minutes,
        solunar_minor_minutes: windows.solunar_minor_minutes,
    }
}

/// A `[[limits]]` profile as the core's copy of the limits, field for field.
pub fn trip_limits(profile: &LimitsProfile) -> TripLimits {
    TripLimits {
        name: profile.name.clone(),
        max_wind_kn: profile.max_wind_kn,
        max_gust_kn: profile.max_gust_kn,
        max_wave_m: profile.max_wave_m,
        max_current_kn: profile.max_current_kn,
        min_air_temp_c: profile.min_air_temp_c,
        min_visibility_km: profile.min_visibility_km,
        no_small_craft_advisory: profile.no_small_craft_advisory,
        no_lightning: profile.no_lightning,
    }
}

/// What `build_view` takes, from the configuration: the cards of the spot's water type in the
/// user's order, units, voice and clock, the limits, the river ranges of the spot's gauge, the
/// thresholds, the fishing-window settings and the first go/no-go profile (the card is left out
/// when there is none).
pub fn view_input(config: &Config, snapshot: SpotSnapshot, now: UnixTime) -> ViewInput {
    let river_ranges = river_ranges(&config.rivers, &snapshot);
    ViewInput {
        card_order: config.cards.order_for(snapshot.spot.water_type),
        snapshot,
        now,
        units: config.units.display_units(),
        voice: config.presentation.voice,
        clock: config.presentation.clock,
        locale: underbite_core::view::locale::Locale::ENGLISH,
        timeline_span: config.presentation.timeline_span(),
        limits: measurement_limits(&config.sources),
        river_ranges,
        #[allow(clippy::cast_possible_truncation)]
        trout_stress_c: Some(config.thresholds.trout_stress_c as f32),
        solunar: SolunarSettings {
            show: config.windows.show_solunar,
        },
        windows: window_params(&config.windows),
        trip_limits: config.limits.first().map(trip_limits),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use underbite_core::model::snapshot::testdata::sample_snapshot;

    fn range(site: &str, wadeable: f64) -> RiverRange {
        RiverRange {
            site: site.into(),
            wadeable_below_cfs: Some(wadeable),
            best_cfs: None,
            blown_out_above_cfs: None,
        }
    }

    #[test]
    fn a_river_entry_applies_only_to_its_own_gauge() {
        let croton = range("usgs:01375000", 300.0);
        let other = range("usgs:01420500", 1200.0);
        // One entry, for the spot's gauge.
        let got = gauge_ranges(std::slice::from_ref(&croton), "USGS-01375000").unwrap();
        assert_eq!(got, croton.to_ranges());
        // One entry, for another gauge: nothing applies.
        assert!(gauge_ranges(std::slice::from_ref(&other), "USGS-01375000").is_none());
        // Two entries: the matching one applies, and none when neither matches.
        let both = [other.clone(), croton.clone()];
        assert_eq!(
            gauge_ranges(&both, "USGS-01375000").unwrap(),
            croton.to_ranges()
        );
        assert_eq!(
            gauge_ranges(&both, "USGS-01420500").unwrap(),
            other.to_ranges()
        );
        assert!(gauge_ranges(&both, "USGS-01372058").is_none());
        assert!(gauge_ranges(&[], "USGS-01375000").is_none());
    }

    #[test]
    fn reservoirs_and_blooms_follow_the_level_and_quality_groups() {
        assert_eq!(group_name(Measurement::Reservoir), Some("lake_level"));
        assert_eq!(group_name(Measurement::Bloom), Some("water_quality"));
        let sources = &Config::default().sources;
        assert_eq!(
            group_of(sources, Measurement::Reservoir),
            group_of(sources, Measurement::LakeLevel)
        );
        assert_eq!(
            group_of(sources, Measurement::Bloom),
            group_of(sources, Measurement::Cyanobacteria)
        );
    }

    #[test]
    fn limits_come_from_the_groups_that_set_them() {
        let limits = measurement_limits(&Config::default().sources);
        let temp = limits
            .iter()
            .find(|l| l.measurement == Measurement::WaterTemperature)
            .unwrap();
        assert_eq!(temp.max_distance_m, Some(25_000.0));
        assert_eq!(temp.max_age_s, Some(10_800));
        assert!(limits.iter().all(|l| l.measurement != Measurement::Tide));
        // Station wind has no group of its own: the fixed observation age applies.
        let wind = limits
            .iter()
            .find(|l| l.measurement == Measurement::ObservedWind)
            .unwrap();
        assert_eq!(
            wind.max_age_s,
            Some(i64::from(OBSERVATION_MAX_AGE_MINUTES) * 60)
        );
    }

    #[test]
    fn the_configuration_sets_the_views_units_voice_and_cards() {
        let snapshot = sample_snapshot();
        let config = Config::default();
        let input = view_input(&config, snapshot.clone(), UnixTime(0));
        assert_eq!(
            input.card_order,
            config.cards.order_for(snapshot.spot.water_type)
        );
        assert_eq!(input.voice, config.presentation.voice);
        assert_eq!(input.limits, measurement_limits(&config.sources));
        assert!(input.river_ranges.is_none());
    }

    #[test]
    fn windows_and_limits_are_the_configurations() {
        let mut config = Config::default();
        config.windows.factors.solunar = 0.25;
        config.windows.current_range_kn = [0.8, 3.0];
        config.windows.solunar_major_minutes = 90;
        let params = window_params(&config.windows);
        assert!((params.weights.solunar - 0.25).abs() < f64::EPSILON);
        assert_eq!(params.current_range_kn, [0.8, 3.0]);
        assert_eq!(params.solunar_major_minutes, 90);
        assert_eq!(params.tide, TideRule::default());
        assert_eq!(
            window_params(&WindowsConfig::default()),
            WindowParams::default()
        );

        let kayak = &config.limits[0];
        let limits = trip_limits(kayak);
        assert_eq!(limits.name, "Kayak");
        assert_eq!(limits.max_gust_kn, kayak.max_gust_kn);
        assert_eq!(limits.max_current_kn, Some(2.0));
        assert!(limits.no_small_craft_advisory && limits.no_lightning);

        let snapshot = sample_snapshot();
        let input = view_input(&config, snapshot.clone(), UnixTime(0));
        assert_eq!(input.trip_limits, Some(limits));
        assert_eq!(input.windows, params);
        // Without a profile the go/no-go card has nothing to judge against.
        config.limits.clear();
        assert!(
            view_input(&config, snapshot, UnixTime(0))
                .trip_limits
                .is_none()
        );
    }
}
