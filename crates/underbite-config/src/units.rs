//! Display units, one per measurement group.
//!
//! The base keys (`discharge`, `height`, `speed`, `water_temp`, `pressure`, `distance`) cover
//! everything; the optional keys override a single group. Units only choose how the view model
//! formats a value, so changing them never changes a stored measurement.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use underbite_core::units::{
    DischargeUnit, DistanceUnit, HeightUnit, PressureUnit, SpeedUnit, TempUnit,
};
use underbite_core::view::DisplayUnits;

/// `[units]`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct UnitsConfig {
    /// River discharge.
    pub discharge: DischargeUnit,
    /// Every height unless overridden: stage, lake level, tide and waves.
    pub height: HeightUnit,
    /// Every speed unless overridden: current and wind.
    pub speed: SpeedUnit,
    /// Water temperature, and air temperature unless `air_temp` is set.
    pub water_temp: TempUnit,
    /// Air temperature, if it differs from the water's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub air_temp: Option<TempUnit>,
    /// River stage (gage height).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<HeightUnit>,
    /// Lake level.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lake_level: Option<HeightUnit>,
    /// Tide heights.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tide: Option<HeightUnit>,
    /// Wave heights.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub waves: Option<HeightUnit>,
    /// Current speed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current: Option<SpeedUnit>,
    /// Wind speed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wind: Option<SpeedUnit>,
    /// Air pressure.
    pub pressure: PressureUnit,
    /// Station distances.
    pub distance: DistanceUnit,
}

impl Default for UnitsConfig {
    fn default() -> Self {
        Self {
            discharge: DischargeUnit::Cfs,
            height: HeightUnit::Ft,
            speed: SpeedUnit::Kn,
            water_temp: TempUnit::F,
            air_temp: None,
            stage: None,
            lake_level: None,
            tide: None,
            waves: None,
            current: None,
            wind: None,
            pressure: PressureUnit::InHg,
            distance: DistanceUnit::Mi,
        }
    }
}

impl UnitsConfig {
    /// The default units for a device whose locale names `region` (an ISO 3166 country code,
    /// any case). The United States and its territories, and an unknown region, keep
    /// [`Default`]; Canada and the rest of the world get metric units, with miles and mph in
    /// the United Kingdom. The app passes the locale's region in; the core never reads it.
    pub fn for_device_region(region: Option<&str>) -> Self {
        let region = region.map(str::to_ascii_uppercase);
        let mut u = Self::default();
        match region.as_deref() {
            None | Some("" | "US" | "PR" | "VI" | "GU" | "AS" | "MP" | "UM") => {}
            Some(code) => {
                u.discharge = DischargeUnit::M3s;
                u.height = HeightUnit::M;
                u.water_temp = TempUnit::C;
                u.distance = DistanceUnit::Km;
                match code {
                    "CA" => {
                        u.pressure = PressureUnit::Kpa;
                        u.wind = Some(SpeedUnit::Kmh);
                    }
                    "GB" => {
                        u.pressure = PressureUnit::Hpa;
                        u.wind = Some(SpeedUnit::Mph);
                        u.distance = DistanceUnit::Mi;
                    }
                    _ => {
                        u.pressure = PressureUnit::Hpa;
                        u.wind = Some(SpeedUnit::Kmh);
                    }
                }
            }
        }
        u
    }

    /// Air temperature unit.
    pub fn air_temp(&self) -> TempUnit {
        self.air_temp.unwrap_or(self.water_temp)
    }

    /// River stage unit.
    pub fn stage(&self) -> HeightUnit {
        self.stage.unwrap_or(self.height)
    }

    /// Lake level unit.
    pub fn lake_level(&self) -> HeightUnit {
        self.lake_level.unwrap_or(self.height)
    }

    /// Tide height unit.
    pub fn tide(&self) -> HeightUnit {
        self.tide.unwrap_or(self.height)
    }

    /// Wave height unit.
    pub fn waves(&self) -> HeightUnit {
        self.waves.unwrap_or(self.height)
    }

    /// Current speed unit.
    pub fn current(&self) -> SpeedUnit {
        self.current.unwrap_or(self.speed)
    }

    /// Wind speed unit.
    pub fn wind(&self) -> SpeedUnit {
        self.wind.unwrap_or(self.speed)
    }

    /// The units the view takes, one per measurement group.
    pub fn display_units(&self) -> DisplayUnits {
        DisplayUnits {
            temperature: self.water_temp,
            air_temperature: self.air_temp(),
            height: self.tide(),
            stage: self.stage(),
            lake_level: self.lake_level(),
            waves: self.waves(),
            discharge: self.discharge,
            current_speed: self.current(),
            wind_speed: self.wind(),
            pressure: self.pressure,
            distance: self.distance,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overrides_fall_back_to_the_base_keys() {
        let u: UnitsConfig =
            toml::from_str("height = \"m\"\nwind = \"mph\"\nstage = \"ft\"").unwrap();
        assert_eq!(u.tide(), HeightUnit::M);
        assert_eq!(u.stage(), HeightUnit::Ft);
        assert_eq!(u.wind(), SpeedUnit::Mph);
        assert_eq!(u.current(), SpeedUnit::Kn);
        assert_eq!(u.air_temp(), TempUnit::F);
        let d = u.display_units();
        assert_eq!((d.height, d.wind_speed), (HeightUnit::M, SpeedUnit::Mph));
    }

    #[test]
    fn device_regions_pick_default_units() {
        let default = UnitsConfig::default();
        assert_eq!(UnitsConfig::for_device_region(Some("us")), default);
        assert_eq!(UnitsConfig::for_device_region(None), default);
        assert_eq!(UnitsConfig::for_device_region(Some("PR")), default);

        let ca = UnitsConfig::for_device_region(Some("CA"));
        assert_eq!(ca.tide(), HeightUnit::M);
        assert_eq!(ca.water_temp, TempUnit::C);
        assert_eq!(ca.discharge, DischargeUnit::M3s);
        assert_eq!(ca.distance, DistanceUnit::Km);
        assert_eq!(ca.pressure, PressureUnit::Kpa);
        assert_eq!((ca.wind(), ca.current()), (SpeedUnit::Kmh, SpeedUnit::Kn));

        let pt = UnitsConfig::for_device_region(Some("PT"));
        assert_eq!(pt.pressure, PressureUnit::Hpa);
        assert_eq!((pt.wind(), pt.distance), (SpeedUnit::Kmh, DistanceUnit::Km));
        let gb = UnitsConfig::for_device_region(Some("GB"));
        assert_eq!((gb.wind(), gb.distance), (SpeedUnit::Mph, DistanceUnit::Mi));
        assert_eq!(gb.tide(), HeightUnit::M);
    }

    #[test]
    fn unknown_unit_is_rejected() {
        assert!(toml::from_str::<UnitsConfig>("discharge = \"gpm\"").is_err());
        assert!(toml::from_str::<UnitsConfig>("pressure_unit = \"hPa\"").is_err());
    }
}
