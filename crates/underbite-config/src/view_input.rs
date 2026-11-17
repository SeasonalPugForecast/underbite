//! The configuration as the view's input: what `build_view` takes, built the same way for the
//! command line and the app, and the per-measurement source groups and limits it draws on.

use underbite_core::model::{Measurement, SpotSnapshot};
use underbite_core::provenance::SourceId;
use underbite_core::time::UnixTime;
use underbite_core::view::{MeasurementLimit, RiverRanges, SolunarSettings, ViewInput};

use crate::rivers::RiverRange;
use crate::schema::Config;
use crate::sources::{SourceGroup, SourcesConfig};

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
        M::LakeLevel => "lake_level",
        M::Waves | M::WavesForecast => "waves",
        M::SeaSurfaceTemp | M::SeaSurfaceTempForecast => "sst",
        M::Clarity | M::Turbidity => "clarity",
        M::DissolvedOxygen
        | M::Conductivity
        | M::Salinity
        | M::Ph
        | M::Chlorophyll
        | M::Cyanobacteria => "water_quality",
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

/// What `build_view` takes, from the configuration: the cards of the spot's water type in the
/// user's order, units, voice and clock, the limits, the river ranges of the spot's gauge and
/// the thresholds.
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
}
