//! The settings: reading, editing and writing the configuration.
//!
//! The app keeps its settings as the configuration crate's versioned JSON and passes that text
//! in with every call; nothing here holds state. Keys are never part of it: a document with a
//! key-like field is refused, and the keychain (iOS) holds the keys apart.

use std::sync::Arc;

use serde::Serialize;
use underbite_config::export::{parse_json, parse_toml, to_json, to_toml};
use underbite_config::schema::{Config, Issue, Severity, has_errors};
use underbite_config::sources::KNOWN_SOURCES;
use underbite_core::model::{Measurement, WaterType};
use underbite_core::provenance::SourceId;
use underbite_core::units::{
    DischargeUnit, DistanceUnit, HeightUnit, PressureUnit, SpeedUnit, TempUnit, pressure_from_hpa,
    pressure_to_hpa,
};
use underbite_core::view::format::{Formatter, distance_from_metres, distance_to_metres};
use underbite_core::view::strings::{MEASUREMENTS, Strings, WATER_TYPES, source_name};
use underbite_core::view::{CardId, DisplayUnits, Voice};
use underbite_providers::coops::{Coops, CoopsHarmonics};
use underbite_providers::ndbc::Ndbc;
use underbite_providers::nwps::Nwps;
use underbite_providers::nws_marine::NwsMarine;
use underbite_providers::open_meteo_marine::OpenMeteoMarine;
use underbite_providers::source::Source;
use underbite_providers::usgs::Usgs;
use whether_core::presentation::TimeFormat;

/// Why a settings call failed.
#[derive(Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum SettingsError {
    /// The settings or a settings file cannot be used; `message` says why in plain words.
    #[error("{message}")]
    Invalid {
        /// What is wrong and, where it helps, the path of the field.
        message: String,
    },
}

fn invalid(message: impl Into<String>) -> SettingsError {
    SettingsError::Invalid {
        message: message.into(),
    }
}

/// How serious a settings issue is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum IssueSeverity {
    /// The settings cannot be used as they are.
    Error,
    /// The settings work, perhaps not as intended.
    Warning,
}

/// One problem found in the settings.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct SettingsIssue {
    /// How serious.
    pub severity: IssueSeverity,
    /// The dotted path of the field, such as `limits[0].max_wave_m`; empty for the document.
    pub path: String,
    /// What is wrong and how to fix it.
    pub message: String,
}

impl From<Issue> for SettingsIssue {
    fn from(i: Issue) -> Self {
        Self {
            severity: match i.severity {
                Severity::Error => IssueSeverity::Error,
                Severity::Warning => IssueSeverity::Warning,
            },
            path: i.path,
            message: i.message,
        }
    }
}

/// Settings brought up to date, with what was changed on the way.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct MigratedSettings {
    /// The settings as the current version's JSON.
    pub json: String,
    /// What was dropped or clamped, and any remaining problems.
    pub issues: Vec<SettingsIssue>,
}

/// The settings in `json`, upgraded (the schema keeps card order and hidden lists, not
/// `{id, shown}` lists, so there is nothing to complete), with the warnings and validation issues found.
pub(crate) fn load(json: &str) -> Result<(Config, Vec<Issue>), SettingsError> {
    parse_json(json).map_err(|e| invalid(e.to_string()))
}

/// The default settings as JSON.
///
/// # Panics
/// Never: the defaults always serialize.
#[uniffi::export]
pub fn default_settings_json() -> String {
    // INVARIANT: the default configuration holds no key-like field and always serializes.
    #[allow(clippy::expect_used)]
    to_json(&Config::default()).expect("the default settings serialize")
}

/// Settings saved by this or an older version, as the current version's JSON. Older settings
/// always decode: fields they lack take their defaults, card and source ids this version does
/// not know are dropped (and reported), and the version number is raised. Settings from a newer
/// version, with a key this version does not have or with a key-like field are refused.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn migrate_settings(json: String) -> Result<MigratedSettings, SettingsError> {
    let (config, issues) = load(&json)?;
    Ok(MigratedSettings {
        json: to_json(&config).map_err(|e| invalid(e.to_string()))?,
        issues: issues.into_iter().map(Into::into).collect(),
    })
}

/// Every problem in the settings, errors and warnings. Settings that cannot be read at all come
/// back as one error.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn validate_settings(json: String) -> Vec<SettingsIssue> {
    match load(&json) {
        Ok((_, issues)) => issues.into_iter().map(Into::into).collect(),
        Err(SettingsError::Invalid { message }) => vec![SettingsIssue {
            severity: IssueSeverity::Error,
            path: String::new(),
            message,
        }],
    }
}

/// The settings as the TOML file the user shares. No key is ever written.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn export_settings_toml(json: String) -> Result<String, SettingsError> {
    let (config, issues) = load(&json)?;
    refuse_errors(&issues)?;
    to_toml(&config).map_err(|e| invalid(e.to_string()))
}

/// The settings in a TOML file, as JSON. A key this version does not have is refused by its
/// path, so is a key-like field; older files are upgraded.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn import_settings_toml(text: String) -> Result<MigratedSettings, SettingsError> {
    let (config, issues) = parse_toml(&text).map_err(|e| invalid(e.to_string()))?;
    refuse_errors(&issues)?;
    Ok(MigratedSettings {
        json: to_json(&config).map_err(|e| invalid(e.to_string()))?,
        issues: issues.into_iter().map(Into::into).collect(),
    })
}

fn refuse_errors(issues: &[Issue]) -> Result<(), SettingsError> {
    if has_errors(issues)
        && let Some(first) = issues.iter().find(|i| i.severity == Severity::Error)
    {
        return Err(invalid(first.to_string()));
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Units
// ---------------------------------------------------------------------------------------------

/// One choice in a unit picker.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct UnitOption {
    /// The value the settings hold (`"cfs"`, `"F"`, `"inHg"`).
    pub id: String,
    /// The symbol shown (`"cfs"`, `"°F"`).
    pub label: String,
}

/// One picker of the units screen: a measurement group and its choices.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct UnitGroup {
    /// The key in the settings' `[units]` table (`"discharge"`, `"stage"`, `"wind"`).
    pub id: String,
    /// What the picker is called.
    pub label: String,
    /// The choices, in the order to list them.
    pub options: Vec<UnitOption>,
    /// The id of the unit in use.
    pub selected: String,
    /// The group that decides the unit while this one is not set; none for a base group.
    pub follows: Option<String>,
    /// The choice that makes the group follow `follows` again ("Same as river flow"); none for a
    /// base group.
    pub same_as_label: Option<String>,
    /// Whether this group has its own unit rather than following another.
    pub overridden: bool,
}

fn unit_id<T: Serialize>(unit: T) -> String {
    serde_json::to_value(unit)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn options<T: Copy + Serialize>(all: &[T], symbol: impl Fn(T) -> &'static str) -> Vec<UnitOption> {
    all.iter()
        .map(|&u| UnitOption {
            id: unit_id(u),
            label: symbol(u).to_owned(),
        })
        .collect()
}

/// The unit pickers for the settings, by measurement group, with the unit in use in each.
/// Labels are the core's symbols.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value, clippy::too_many_lines)] // UniFFI passes arguments by value
pub fn unit_options(settings_json: String) -> Result<Vec<UnitGroup>, SettingsError> {
    let (config, _) = load(&settings_json)?;
    let u = &config.units;
    let heights = || options(&HeightUnit::ALL, HeightUnit::symbol);
    let speeds = || options(&SpeedUnit::ALL, SpeedUnit::symbol);
    let temps = || options(&TempUnit::ALL, TempUnit::symbol);
    let strings = Strings::new(config.presentation.voice);
    let group = |id: &str,
                 options: Vec<UnitOption>,
                 selected: String,
                 follows: Option<&str>,
                 overridden: bool| UnitGroup {
        id: id.to_owned(),
        label: strings.unit_group(id).unwrap_or(id).to_owned(),
        options,
        selected,
        follows: follows.map(str::to_owned),
        same_as_label: follows.map(|f| strings.unit_same_as(strings.unit_group(f).unwrap_or(f))),
        overridden,
    };
    Ok(vec![
        group(
            "discharge",
            options(&DischargeUnit::ALL, DischargeUnit::symbol),
            unit_id(u.discharge),
            None,
            false,
        ),
        group("height", heights(), unit_id(u.height), None, false),
        group(
            "stage",
            heights(),
            unit_id(u.stage()),
            Some("height"),
            u.stage.is_some(),
        ),
        group(
            "lake_level",
            heights(),
            unit_id(u.lake_level()),
            Some("height"),
            u.lake_level.is_some(),
        ),
        group(
            "tide",
            heights(),
            unit_id(u.tide()),
            Some("height"),
            u.tide.is_some(),
        ),
        group(
            "waves",
            heights(),
            unit_id(u.waves()),
            Some("height"),
            u.waves.is_some(),
        ),
        group("speed", speeds(), unit_id(u.speed), None, false),
        group(
            "current",
            speeds(),
            unit_id(u.current()),
            Some("speed"),
            u.current.is_some(),
        ),
        group(
            "wind",
            speeds(),
            unit_id(u.wind()),
            Some("speed"),
            u.wind.is_some(),
        ),
        group("water_temp", temps(), unit_id(u.water_temp), None, false),
        group(
            "air_temp",
            temps(),
            unit_id(u.air_temp()),
            Some("water_temp"),
            u.air_temp.is_some(),
        ),
        group(
            "pressure",
            options(&PressureUnit::ALL, PressureUnit::symbol),
            unit_id(u.pressure),
            None,
            false,
        ),
        group(
            "distance",
            options(&DistanceUnit::ALL, DistanceUnit::symbol),
            unit_id(u.distance),
            None,
            false,
        ),
    ])
}

// ---------------------------------------------------------------------------------------------
// Value entry
// ---------------------------------------------------------------------------------------------

/// What a value is a measure of, for entry and display stops.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Discharge,
    Stage,
    LakeLevel,
    Tide,
    Waves,
    WaterTemp,
    AirTemp,
    CurrentSpeed,
    WindSpeed,
    Pressure,
    Distance,
}

impl Kind {
    /// The kind a measurement id or a unit group id names.
    fn of(name: &str) -> Option<Self> {
        Some(match name {
            "discharge" => Self::Discharge,
            "gage_height" | "stage" => Self::Stage,
            "lake_level" => Self::LakeLevel,
            "tide" | "height" | "depth" => Self::Tide,
            "waves" | "waves_forecast" => Self::Waves,
            "water_temperature" | "water_temp" | "sea_surface_temp" => Self::WaterTemp,
            "air_temperature" | "air_temp" => Self::AirTemp,
            "tidal_current" | "ocean_current" | "current" | "current_speed" => Self::CurrentSpeed,
            "observed_wind" | "wind" | "wind_speed" => Self::WindSpeed,
            "observed_pressure" | "pressure" => Self::Pressure,
            "distance" => Self::Distance,
            _ => return None,
        })
    }
}

/// A unit of any kind.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Unit {
    Discharge(DischargeUnit),
    Height(HeightUnit),
    Speed(SpeedUnit),
    Temp(TempUnit),
    Pressure(PressureUnit),
    Distance(DistanceUnit),
}

impl Unit {
    /// Every unit of a kind, in picker order.
    fn all(kind: Kind) -> Vec<Self> {
        match kind {
            Kind::Discharge => DischargeUnit::ALL.map(Self::Discharge).to_vec(),
            Kind::Stage | Kind::LakeLevel | Kind::Tide | Kind::Waves => {
                HeightUnit::ALL.map(Self::Height).to_vec()
            }
            Kind::CurrentSpeed | Kind::WindSpeed => SpeedUnit::ALL.map(Self::Speed).to_vec(),
            Kind::WaterTemp | Kind::AirTemp => TempUnit::ALL.map(Self::Temp).to_vec(),
            Kind::Pressure => PressureUnit::ALL.map(Self::Pressure).to_vec(),
            Kind::Distance => DistanceUnit::ALL.map(Self::Distance).to_vec(),
        }
    }

    /// The unit the settings choose for a kind.
    fn selected(kind: Kind, u: &DisplayUnits) -> Self {
        match kind {
            Kind::Discharge => Self::Discharge(u.discharge),
            Kind::Stage => Self::Height(u.stage),
            Kind::LakeLevel => Self::Height(u.lake_level),
            Kind::Tide => Self::Height(u.height),
            Kind::Waves => Self::Height(u.waves),
            Kind::WaterTemp => Self::Temp(u.temperature),
            Kind::AirTemp => Self::Temp(u.air_temperature),
            Kind::CurrentSpeed => Self::Speed(u.current_speed),
            Kind::WindSpeed => Self::Speed(u.wind_speed),
            Kind::Pressure => Self::Pressure(u.pressure),
            Kind::Distance => Self::Distance(u.distance),
        }
    }

    fn symbol(self) -> &'static str {
        match self {
            Self::Discharge(u) => u.symbol(),
            Self::Height(u) => u.symbol(),
            Self::Speed(u) => u.symbol(),
            Self::Temp(u) => u.symbol(),
            Self::Pressure(u) => u.symbol(),
            Self::Distance(u) => u.symbol(),
        }
    }

    /// A value in this unit as the canonical unit (the core's conversions; distance in metres).
    fn to_canonical(self, v: f64) -> f64 {
        let v32 = v as f32;
        f64::from(match self {
            Self::Discharge(u) => u.to_canonical(v32),
            Self::Height(u) => u.to_canonical(v32),
            Self::Speed(u) => u.to_canonical(v32),
            Self::Temp(u) => u.to_canonical(v32),
            Self::Pressure(u) => pressure_to_hpa(u, v32),
            Self::Distance(u) => return distance_to_metres(u, v),
        })
    }

    /// A canonical value in this unit.
    fn to_display(self, c: f64) -> f64 {
        let c32 = c as f32;
        f64::from(match self {
            Self::Discharge(u) => u.from_canonical(c32),
            Self::Height(u) => u.from_canonical(c32),
            Self::Speed(u) => u.from_canonical(c32),
            Self::Temp(u) => u.from_canonical(c32),
            Self::Pressure(u) => pressure_from_hpa(u, c32),
            Self::Distance(u) => return distance_from_metres(u, c),
        })
    }
}

/// The settings' display units and a kind from a measurement id.
fn units_and_kind(
    measurement: &str,
    settings_json: &str,
) -> Result<(DisplayUnits, Kind), SettingsError> {
    let kind = Kind::of(measurement)
        .ok_or_else(|| invalid(format!("`{measurement}` is not a measurement with units")))?;
    let (config, _) = load(settings_json)?;
    Ok((config.units.display_units(), kind))
}

/// A number typed in the user's unit for `measurement` (`"discharge"`, `"gage_height"`,
/// `"observed_wind"`, `"water_temperature"`, ...), as the canonical value: m³/s, metres, m/s,
/// °C, hPa; distances in metres. The text may end with a unit symbol of the same kind, which
/// then overrides the setting ("1,200 cfs"). The core converts; Swift never does.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn parse_user_value(
    measurement: String,
    text: String,
    settings_json: String,
) -> Result<f64, SettingsError> {
    let (units, kind) = units_and_kind(&measurement, &settings_json)?;
    let (value, unit) = parse_typed(kind, Unit::selected(kind, &units), &text, false)?;
    Ok(unit.to_canonical(value))
}

/// The number in `text` and the unit it is in: a trailing symbol of the kind's units wins over
/// `default`. With `decimal_comma` a lone comma that does not group thousands is a decimal point.
fn parse_typed(
    kind: Kind,
    default: Unit,
    text: &str,
    decimal_comma: bool,
) -> Result<(f64, Unit), SettingsError> {
    let mut unit = default;
    let mut number = text.trim();
    // The longest symbol first, so "m³/s" is not read as ending in "s".
    let mut candidates: Vec<(String, Unit)> = Unit::all(kind)
        .into_iter()
        .flat_map(|u| {
            let symbol = u.symbol().to_lowercase();
            let bare = symbol.trim_start_matches('°').to_owned();
            [(symbol.clone(), u), (bare, u)]
        })
        .collect();
    candidates.sort_by_key(|(s, _)| std::cmp::Reverse(s.len()));
    // The symbol is looked for on char boundaries of the original text: lowercasing can change
    // a character's byte length (KELVIN SIGN becomes "k"), so lengths never carry over.
    let found = candidates.iter().find_map(|(symbol, u)| {
        number
            .char_indices()
            .find(|&(at, _)| number[at..].to_lowercase() == *symbol)
            .map(|(at, _)| (at, *u))
    });
    if let Some((at, found)) = found {
        unit = found;
        number = number[..at].trim_end();
    }
    let value = parse_number(number, decimal_comma)
        .ok_or_else(|| invalid(format!("`{}` is not a number", text.trim())))?;
    Ok((value, unit))
}

/// A finite number from text: thousands separators dropped, and with `decimal_comma` a comma
/// that is not a thousands group ("1,5", "0,500", "1.200,5") read as the decimal point.
fn parse_number(text: &str, decimal_comma: bool) -> Option<f64> {
    let text = text.trim();
    let comma_is_decimal = decimal_comma
        && match (text.rfind(','), text.rfind('.')) {
            (Some(c), Some(d)) => c > d,
            (Some(c), None) => {
                text.matches(',').count() == 1
                    && (text.len() - c - 1 != 3 || text[..c].trim_start_matches('-') == "0")
            }
            _ => false,
        };
    let cleaned: String = if comma_is_decimal {
        text.chars()
            .filter(|c| *c != '.')
            .map(|c| if c == ',' { '.' } else { c })
            .collect()
    } else {
        text.chars().filter(|c| *c != ',').collect()
    };
    cleaned.trim().parse::<f64>().ok().filter(|v| v.is_finite())
}

/// One stop of a slider or a range editor: a canonical value and its label.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct DisplayStop {
    /// The value in canonical units (as [`parse_user_value`] returns).
    pub value: f64,
    /// The value in the user's unit, written by the core: `"1,200 cfs"`.
    pub label: String,
}

/// Round stops between `min` and `max` (canonical values) for `measurement`: multiples of a
/// round step in the user's unit, each with the value to store and the label to show.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn display_stops(
    measurement: String,
    min: f64,
    max: f64,
    settings_json: String,
) -> Result<Vec<DisplayStop>, SettingsError> {
    let (units, kind) = units_and_kind(&measurement, &settings_json)?;
    let unit = Unit::selected(kind, &units);
    Ok(stops_between(kind, &units, unit, min, max)?
        .into_iter()
        .map(|(value, label)| DisplayStop { value, label })
        .collect())
}

/// The step of round stops across `low..high`: 1, 2, 5 or 10 times a power of ten, giving about
/// four intervals.
fn round_step(low: f64, high: f64) -> f64 {
    let raw = (high - low) / 4.0;
    let magnitude = 10f64.powf(raw.log10().floor());
    let normal = raw / magnitude;
    magnitude
        * if normal <= 1.0 {
            1.0
        } else if normal <= 2.0 {
            2.0
        } else if normal <= 5.0 {
            5.0
        } else {
            10.0
        }
}

/// Round stops of `unit` between two canonical values, as canonical values with their labels.
fn stops_between(
    kind: Kind,
    units: &DisplayUnits,
    unit: Unit,
    min: f64,
    max: f64,
) -> Result<Vec<(f64, String)>, SettingsError> {
    let (low, high) = (unit.to_display(min), unit.to_display(max));
    if !(low.is_finite() && high.is_finite() && high > low) {
        return Err(invalid("the range needs a minimum below its maximum"));
    }
    let step = round_step(low, high);
    let formatter = Formatter::new(*units, Voice::Plain, TimeFormat::H12, "UTC");
    let mut stops = Vec::new();
    let mut k = (low / step).ceil();
    while k * step <= high + step * 1e-9 && stops.len() < 64 {
        let value = unit.to_canonical(k * step);
        stops.push((value, label(&formatter, kind, value)));
        k += 1.0;
    }
    Ok(stops)
}

/// "1,200 cfs": the formatter's text and unit for a canonical value.
fn label(f: &Formatter, kind: Kind, canonical: f64) -> String {
    let v = Some(canonical as f32);
    let q = match kind {
        Kind::Discharge => f.discharge(v),
        Kind::Stage => f.stage_length(v),
        Kind::LakeLevel => f.lake_level_length(v),
        Kind::Tide => f.length(v),
        Kind::Waves => f.wave_height(v),
        Kind::WaterTemp => f.temperature(v),
        Kind::AirTemp => {
            let mut air = f.clone();
            air.units.temperature = f.units.air_temperature;
            air.temperature(v)
        }
        Kind::CurrentSpeed => f.current_speed(v),
        Kind::WindSpeed => f.wind_speed(v),
        Kind::Pressure => f.pressure(v),
        Kind::Distance => f.distance(v),
    };
    format!("{} {}", q.text, q.unit)
}

// ---------------------------------------------------------------------------------------------
// Settings fields
// ---------------------------------------------------------------------------------------------
//
// A settings field keeps its value in the unit its name says (`max_wind_kn` in knots,
// `wadeable_below_cfs` in cfs, `max_distance_km` in km, `max_age_minutes` in minutes). The
// functions below take such a value and the field's key, and speak the user's unit in text:
// Swift holds the stored value, shows the core's label and never converts.
//
// A key is the field's path in the settings with list positions written `[]`:
// `limits[].max_wind_kn`, `rivers[].best_cfs`, `thresholds.trout_stress_c`,
// `storms.hurricane_distance_km`, `sources.<group>.max_distance_km` and
// `sources.<group>.max_age_minutes` for every source group. A position (`limits[2].max_wind_kn`)
// is accepted and ignored. [`setting_field_keys`] lists every key.

/// The source groups of `[sources]` that carry the distance and age limits.
const SOURCE_GROUPS: [&str; 12] = [
    "water_temperature",
    "tide",
    "currents",
    "flow",
    "lake_level",
    "waves",
    "sst",
    "clarity",
    "water_quality",
    "marine_forecast",
    "weather",
    "storm",
];

/// The unit family of a settings field.
#[derive(Clone, Copy, Debug, PartialEq)]
enum FieldUnit {
    /// A measure of `kind`, stored in `stored`.
    Measure(Kind, Unit),
    /// Minutes, shown as a duration.
    Minutes,
}

const WIND: FieldUnit = FieldUnit::Measure(Kind::WindSpeed, Unit::Speed(SpeedUnit::Kn));
const CURRENT: FieldUnit = FieldUnit::Measure(Kind::CurrentSpeed, Unit::Speed(SpeedUnit::Kn));
const FLOW: FieldUnit = FieldUnit::Measure(Kind::Discharge, Unit::Discharge(DischargeUnit::Cfs));
const KM: FieldUnit = FieldUnit::Measure(Kind::Distance, Unit::Distance(DistanceUnit::Km));

/// The keys that do not depend on a source group, with their units.
const FIXED_FIELDS: [(&str, FieldUnit); 10] = [
    ("limits[].max_wind_kn", WIND),
    ("limits[].max_gust_kn", WIND),
    (
        "limits[].max_wave_m",
        FieldUnit::Measure(Kind::Waves, Unit::Height(HeightUnit::M)),
    ),
    ("limits[].max_current_kn", CURRENT),
    (
        "limits[].min_air_temp_c",
        FieldUnit::Measure(Kind::AirTemp, Unit::Temp(TempUnit::C)),
    ),
    ("limits[].min_visibility_km", KM),
    ("rivers[].wadeable_below_cfs", FLOW),
    ("rivers[].best_cfs", FLOW),
    ("rivers[].blown_out_above_cfs", FLOW),
    ("storms.hurricane_distance_km", KM),
];

/// `thresholds.trout_stress_c`, a water temperature.
const TROUT_STRESS: (&str, FieldUnit) = (
    "thresholds.trout_stress_c",
    FieldUnit::Measure(Kind::WaterTemp, Unit::Temp(TempUnit::C)),
);

/// Every settings field key the functions here take: the limits, river ranges, thresholds, the
/// storm distance and, for each source group, `sources.<group>.max_distance_km` and
/// `sources.<group>.max_age_minutes`.
#[uniffi::export]
pub fn setting_field_keys() -> Vec<String> {
    let mut keys: Vec<String> = FIXED_FIELDS.iter().map(|(k, _)| (*k).to_owned()).collect();
    keys.push(TROUT_STRESS.0.to_owned());
    for group in SOURCE_GROUPS {
        keys.push(format!("sources.{group}.max_distance_km"));
        keys.push(format!("sources.{group}.max_age_minutes"));
    }
    keys
}

/// The unit family of a field key, list positions ignored.
fn field_unit(field: &str) -> Result<FieldUnit, SettingsError> {
    let mut key = String::with_capacity(field.len());
    let mut rest = field.trim();
    while let Some(open) = rest.find('[') {
        key.push_str(&rest[..open]);
        let Some(close) = rest[open..].find(']') else {
            break;
        };
        key.push_str("[]");
        rest = &rest[open + close + 1..];
    }
    key.push_str(rest);
    if let Some((_, unit)) = FIXED_FIELDS.iter().find(|(k, _)| *k == key) {
        return Ok(*unit);
    }
    if key == TROUT_STRESS.0 {
        return Ok(TROUT_STRESS.1);
    }
    if let Some(rest) = key.strip_prefix("sources.")
        && let Some((group, name)) = rest.split_once('.')
        && SOURCE_GROUPS.contains(&group)
    {
        match name {
            "max_distance_km" => return Ok(KM),
            "max_age_minutes" => return Ok(FieldUnit::Minutes),
            _ => {}
        }
    }
    Err(invalid(format!(
        "`{field}` is not a settings field with a value to enter"
    )))
}

/// Six significant digits: undoes the single-precision round trip of a unit conversion.
fn tidy(v: f64) -> f64 {
    if v == 0.0 || !v.is_finite() {
        return v;
    }
    let scale = 10f64.powf(5.0 - v.abs().log10().floor());
    (v * scale).round() / scale
}

/// A value of the field's stored unit in the unit `selected`.
fn stored_to_selected(stored: Unit, selected: Unit, v: f64) -> f64 {
    if stored == selected {
        v
    } else {
        tidy(selected.to_display(stored.to_canonical(v)))
    }
}

/// Minutes from text: a number, optionally with `min`, `h` or `d` after it.
fn parse_minutes(text: &str) -> Result<f64, SettingsError> {
    let lower = text.trim().to_lowercase();
    let units: [(&[&str], f64); 3] = [
        (&["days", "day", "d"], 1_440.0),
        (&["hours", "hour", "hrs", "hr", "h"], 60.0),
        (&["minutes", "minute", "mins", "min", "m"], 1.0),
    ];
    let mut number = lower.as_str();
    let mut factor = 1.0;
    'found: for (suffixes, f) in units {
        for suffix in suffixes {
            if let Some(n) = lower.strip_suffix(suffix) {
                number = n;
                factor = f;
                break 'found;
            }
        }
    }
    let v = parse_number(number, true)
        .ok_or_else(|| invalid(format!("`{}` is not a number", text.trim())))?;
    Ok(tidy(v * factor))
}

/// Steps for minute stops that a clock reads easily: minutes, quarter hours, hours, days.
const MINUTE_STEPS: [f64; 14] = [
    1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 180.0, 360.0, 720.0, 1_440.0, 2_880.0, 10_080.0,
];

/// What the user typed for a settings field, as the value in the field's stored unit. The text
/// is a number in the user's unit for that kind of value, with a decimal point or comma, and may
/// end with a unit symbol that then overrides it: "15" with knots chosen reads 15 for
/// `limits[].max_wind_kn`, "30 mph" reads 26.07; "1,5" is 1.5. For a field in minutes the text
/// may end with `min`, `h` or `d`: "2 h" reads 120.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn parse_setting_value(
    field: String,
    text: String,
    settings_json: String,
) -> Result<f64, SettingsError> {
    match field_unit(&field)? {
        FieldUnit::Minutes => parse_minutes(&text),
        FieldUnit::Measure(kind, stored) => {
            let (config, _) = load(&settings_json)?;
            let units = config.units.display_units();
            let (value, used) = parse_typed(kind, Unit::selected(kind, &units), &text, true)?;
            Ok(if used == stored {
                value
            } else {
                tidy(stored.to_display(used.to_canonical(value)))
            })
        }
    }
}

/// A value of a settings field (in the field's stored unit) as the core writes it in the user's
/// unit: "15 kn", "1,200 cfs", "3.3 ft", "54 °F", "2 h".
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn format_setting_value(
    field: String,
    value: f64,
    settings_json: String,
) -> Result<String, SettingsError> {
    let (config, _) = load(&settings_json)?;
    match field_unit(&field)? {
        FieldUnit::Minutes => Ok(duration_text(&config, value * 60.0)),
        FieldUnit::Measure(kind, stored) => {
            let units = config.units.display_units();
            let formatter = Formatter::new(units, Voice::Plain, TimeFormat::H12, "UTC");
            Ok(label(&formatter, kind, stored.to_canonical(value)))
        }
    }
}

/// Round stops for a slider or range editor on a settings field, between `min` and `max` (in the
/// field's stored unit): the value to store (in the stored unit) and the label in the user's unit.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn setting_stops(
    field: String,
    min: f64,
    max: f64,
    settings_json: String,
) -> Result<Vec<DisplayStop>, SettingsError> {
    let (config, _) = load(&settings_json)?;
    match field_unit(&field)? {
        FieldUnit::Minutes => {
            if !(min.is_finite() && max.is_finite() && max > min) {
                return Err(invalid("the range needs a minimum below its maximum"));
            }
            let raw = (max - min) / 4.0;
            let step = MINUTE_STEPS
                .into_iter()
                .find(|s| *s >= raw)
                .unwrap_or(MINUTE_STEPS[MINUTE_STEPS.len() - 1]);
            let mut stops = Vec::new();
            let mut k = (min / step).ceil();
            while k * step <= max + 1e-9 && stops.len() < 64 {
                let value = k * step;
                stops.push(DisplayStop {
                    value,
                    label: duration_text(&config, value * 60.0),
                });
                k += 1.0;
            }
            Ok(stops)
        }
        FieldUnit::Measure(kind, stored) => {
            let units = config.units.display_units();
            let selected = Unit::selected(kind, &units);
            let stops = stops_between(
                kind,
                &units,
                selected,
                stored.to_canonical(min),
                stored.to_canonical(max),
            )?;
            Ok(stops
                .into_iter()
                .map(|(canonical, label)| DisplayStop {
                    value: stored_to_selected(selected, stored, selected.to_display(canonical)),
                    label,
                })
                .collect())
        }
    }
}

fn duration_text(config: &Config, seconds: f64) -> String {
    let units = config.units.display_units();
    Formatter::new(units, config.presentation.voice, TimeFormat::H12, "UTC")
        .duration(seconds.round() as i64)
}

/// A time span as the core writes it: "45 min", "6 h", "2 d". The largest unit that divides the
/// span evenly, minutes otherwise, in the user's voice.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn format_duration(seconds: i64, settings_json: String) -> Result<String, SettingsError> {
    let (config, _) = load(&settings_json)?;
    Ok(duration_text(&config, seconds as f64))
}

// ---------------------------------------------------------------------------------------------
// Cards
// ---------------------------------------------------------------------------------------------

/// One card of the customize screen.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct CardInfo {
    /// The id the settings use (`"water_temp"`).
    pub id: String,
    /// The card's title in the user's voice.
    pub title: String,
    /// Whether the card shows for this water type.
    pub shown: bool,
    /// Whether this version builds the card; the others are placeholders for later cards.
    pub available: bool,
}

/// Every card, the shown ones first in the user's order for `water_type` (`"river_stream"`,
/// `"tidal_river_estuary"`, ...), then the hidden ones.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn card_catalogue(
    water_type: String,
    settings_json: String,
) -> Result<Vec<CardInfo>, SettingsError> {
    let water: WaterType = serde_json::from_value(serde_json::Value::String(water_type.clone()))
        .map_err(|_| invalid(format!("`{water_type}` is not a water type")))?;
    let (config, _) = load(&settings_json)?;
    let strings = Strings::new(config.presentation.voice);
    let order = config.cards.order_for(water);
    let info = |id: CardId, shown: bool| CardInfo {
        id: unit_id(id),
        title: strings.card_title(id).to_owned(),
        shown,
        available: !matches!(
            id,
            CardId::Windows | CardId::GoNoGo | CardId::Stocking | CardId::Species
        ),
    };
    let mut cards: Vec<CardInfo> = order.iter().map(|&id| info(id, true)).collect();
    cards.extend(
        CardId::ALL
            .into_iter()
            .filter(|id| !order.contains(id))
            .map(|id| info(id, false)),
    );
    Ok(cards)
}

// ---------------------------------------------------------------------------------------------
// Water types and measurements
// ---------------------------------------------------------------------------------------------

/// A kind of water, for a picker.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct WaterTypeOption {
    /// The id spots and settings use (`"river_stream"`).
    pub id: String,
    /// Its name in the user's voice: "River or stream".
    pub label: String,
    /// Its name in the plain voice, for `VoiceOver`, when the user's voice words it differently.
    pub plain_label: Option<String>,
}

/// Every kind of water, in the order a picker lists them, named in the settings' voice.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn water_type_options(settings_json: String) -> Result<Vec<WaterTypeOption>, SettingsError> {
    let (config, _) = load(&settings_json)?;
    let strings = Strings::new(config.presentation.voice);
    let plain = Strings::new(Voice::Plain);
    Ok(WATER_TYPES
        .into_iter()
        .map(|w| {
            let label = strings.water_type(w).to_owned();
            let plain_label = Some(plain.water_type(w).to_owned()).filter(|p| *p != label);
            WaterTypeOption {
                id: unit_id(w),
                label,
                plain_label,
            }
        })
        .collect())
}

/// A measurement's name.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct MeasurementTitle {
    /// The id the core writes (`"water_temperature"`).
    pub id: String,
    /// "Water temperature", in the user's voice.
    pub title: String,
}

/// Every measurement with its name in the settings' voice, in the order lists of them read.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn measurement_titles(settings_json: String) -> Result<Vec<MeasurementTitle>, SettingsError> {
    let (config, _) = load(&settings_json)?;
    let strings = Strings::new(config.presentation.voice);
    Ok(MEASUREMENTS
        .into_iter()
        .map(|m| MeasurementTitle {
            id: unit_id(m),
            title: strings.measurement(m).to_owned(),
        })
        .collect())
}

/// A source group's name.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct SourceGroupTitle {
    /// The key in the settings' `[sources]` table (`"water_temperature"`, `"sst"`).
    pub id: String,
    /// "Water temperature", in the user's voice.
    pub title: String,
}

/// Every source group with its name in the settings' voice, in the order the sources screen
/// lists them.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn source_group_titles(settings_json: String) -> Result<Vec<SourceGroupTitle>, SettingsError> {
    let (config, _) = load(&settings_json)?;
    let strings = Strings::new(config.presentation.voice);
    Ok(SOURCE_GROUPS
        .into_iter()
        .map(|id| SourceGroupTitle {
            id: id.to_owned(),
            title: strings.source_group(id).unwrap_or(id).to_owned(),
        })
        .collect())
}

/// A source group's limits in words: "within 25 km, under 3 h old", "any distance, any age".
/// `group` is the id of [`source_group_titles`]; the distance is in kilometres and the age in
/// minutes, as the settings hold them.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn source_limits_text(
    group: String,
    max_distance_km: Option<f64>,
    max_age_minutes: Option<i64>,
    settings_json: String,
) -> Result<String, SettingsError> {
    if !SOURCE_GROUPS.contains(&group.as_str()) {
        return Err(invalid(format!("{group} is not a source group")));
    }
    let (config, _) = load(&settings_json)?;
    let distance = match max_distance_km {
        Some(km) => Some(format_setting_value(
            format!("sources.{group}.max_distance_km"),
            km,
            settings_json.clone(),
        )?),
        None => None,
    };
    let age = max_age_minutes.map(|m| duration_text(&config, m as f64 * 60.0));
    Ok(Strings::new(config.presentation.voice).source_limits(distance.as_deref(), age.as_deref()))
}

/// What a go/no-go profile checks, in words: "2 limits, no lightning"; "No limits set".
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn limit_summary(
    set_count: u32,
    no_small_craft_advisory: bool,
    no_lightning: bool,
    settings_json: String,
) -> Result<String, SettingsError> {
    let (config, _) = load(&settings_json)?;
    Ok(Strings::new(config.presentation.voice).limit_summary(
        set_count as usize,
        no_small_craft_advisory,
        no_lightning,
    ))
}

/// The label of the band a flow chart shades, from the labels of its two edges ("5th
/// percentile", "95th percentile"): "Shaded: 5th percentile to 95th percentile". The band is
/// not the USGS "normal" range, so it carries its own words.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn percentile_band_label(
    low: String,
    high: String,
    settings_json: String,
) -> Result<String, SettingsError> {
    let (config, _) = load(&settings_json)?;
    Ok(Strings::new(config.presentation.voice).percentile_band(&low, &high))
}

// ---------------------------------------------------------------------------------------------
// Sources
// ---------------------------------------------------------------------------------------------

/// A data source, for the sources screen.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
#[allow(clippy::struct_excessive_bools)] // independent facts about a source
pub struct SourceInfo {
    /// The id the settings use (`"coops"`).
    pub id: String,
    /// The name shown.
    pub name: String,
    /// What it supplies, in words.
    pub measurements: Vec<String>,
    /// The credit its data are shown with.
    pub credit: String,
    /// The licence its data are under, when it states one.
    pub license: Option<String>,
    /// Its home page.
    pub url: Option<String>,
    /// It serves only with a key or an account of the user's own.
    pub needs_key: bool,
    /// It takes a key but also serves without one.
    pub key_optional: bool,
    /// It is off until the user turns it on.
    pub opt_in: bool,
    /// This version can fetch from it; the others are listed and skipped.
    pub implemented: bool,
}

/// What one source row holds besides what its provider says about itself.
struct Row {
    id: &'static str,
    /// A name for sources the core's names do not cover.
    name: Option<&'static str>,
    credit: Option<&'static str>,
    license: Option<&'static str>,
    url: Option<&'static str>,
    /// What a source supplies that this version cannot fetch yet.
    measurements: &'static [Measurement],
    needs_key: bool,
    key_optional: bool,
    opt_in: bool,
}

impl Row {
    const fn new(id: &'static str) -> Self {
        Self {
            id,
            name: None,
            credit: None,
            license: None,
            url: None,
            measurements: &[],
            needs_key: false,
            key_optional: false,
            opt_in: false,
        }
    }
}

const PUBLIC_DOMAIN: &str = "Public domain";
const CC_BY_4: &str = "CC BY 4.0";

/// The sources the configuration knows, in the order of [`KNOWN_SOURCES`].
fn rows() -> Vec<Row> {
    use Measurement as M;
    let weather = |id, name| Row {
        name: Some(name),
        ..Row::new(id)
    };
    vec![
        Row {
            key_optional: true,
            license: Some(PUBLIC_DOMAIN),
            ..Row::new("usgs")
        },
        Row {
            license: Some(PUBLIC_DOMAIN),
            ..Row::new("coops")
        },
        Row {
            license: Some(PUBLIC_DOMAIN),
            ..Row::new("coops-harmonics")
        },
        Row {
            license: Some(PUBLIC_DOMAIN),
            ..Row::new("ndbc")
        },
        Row {
            license: Some(PUBLIC_DOMAIN),
            ..Row::new("nwps")
        },
        Row {
            license: Some(PUBLIC_DOMAIN),
            ..Row::new("nws")
        },
        Row {
            name: Some("NOAA GLERL"),
            url: Some("https://www.glerl.noaa.gov/"),
            measurements: &[M::WaterTemperature, M::LakeLevel, M::Waves],
            ..Row::new("glerl")
        },
        Row {
            name: Some("NASA JPL MUR sea surface temperature"),
            credit: Some("SST: NASA JPL MUR (MEaSUREs), via NOAA CoastWatch ERDDAP"),
            license: Some("JPL PO.DAAC data policy"),
            measurements: &[M::SeaSurfaceTemp],
            ..Row::new("mur-sst")
        },
        Row {
            name: Some("HF radar surface currents"),
            measurements: &[M::OceanCurrent],
            ..Row::new("hf-radar")
        },
        Row {
            name: Some("Copernicus Marine"),
            url: Some("https://marine.copernicus.eu/"),
            measurements: &[M::OceanCurrent, M::Clarity, M::Chlorophyll],
            needs_key: true,
            opt_in: true,
            ..Row::new("copernicus")
        },
        Row {
            license: Some(CC_BY_4),
            ..Row::new("open-meteo-marine")
        },
        Row {
            name: Some("Open-Meteo storm tracks"),
            credit: Some("Weather data by Open-Meteo.com"),
            license: Some(CC_BY_4),
            url: Some("https://open-meteo.com/"),
            ..Row::new("open-meteo-storm")
        },
        Row {
            name: Some("NOAA National Hurricane Center"),
            credit: Some("National Hurricane Center (NOAA)"),
            license: Some(PUBLIC_DOMAIN),
            url: Some("https://www.nhc.noaa.gov/"),
            ..Row::new("nhc")
        },
        Row {
            needs_key: true,
            ..weather("openweather", "OpenWeather")
        },
        Row {
            credit: Some("Weather data by Open-Meteo.com"),
            license: Some(CC_BY_4),
            url: Some("https://open-meteo.com/"),
            ..weather("open-meteo", "Open-Meteo")
        },
        Row {
            license: Some(CC_BY_4),
            url: Some("https://api.met.no/"),
            ..weather("met-norway", "MET Norway")
        },
        weather("acis", "NOAA ACIS climate normals"),
        Row {
            needs_key: true,
            ..weather("pirate-weather", "Pirate Weather")
        },
        Row::new("dfo-iwls"),
        Row::new("eccc-hydrometric"),
    ]
}

/// The provider this version has for a source id.
fn provider(id: &str) -> Option<Arc<dyn Source>> {
    Some(match id {
        "usgs" => Arc::new(Usgs::new()),
        "coops" => Arc::new(Coops),
        "coops-harmonics" => Arc::new(CoopsHarmonics),
        "ndbc" => Arc::new(Ndbc),
        "nwps" => Arc::new(Nwps),
        "nws" => Arc::new(NwsMarine),
        "open-meteo-marine" => Arc::new(OpenMeteoMarine),
        _ => return None,
    })
}

/// Every source the settings can name, with what the sources screen shows for it.
#[uniffi::export]
pub fn source_catalogue() -> Vec<SourceInfo> {
    let infos: Vec<SourceInfo> = rows()
        .into_iter()
        .map(|row| {
            let source = provider(row.id);
            let attribution = source.as_ref().map(|s| s.attribution());
            let measurements: Vec<Measurement> = match &source {
                Some(s) => s.measurements().to_vec(),
                None => row.measurements.to_vec(),
            };
            let name = row.name.map_or_else(
                || source_name(&SourceId::new(row.id)),
                std::borrow::ToOwned::to_owned,
            );
            SourceInfo {
                id: row.id.to_owned(),
                credit: attribution
                    .as_ref()
                    .map(|a| a.credit.clone())
                    .or_else(|| row.credit.map(str::to_owned))
                    .unwrap_or_else(|| name.clone()),
                url: attribution
                    .and_then(|a| a.url)
                    .or_else(|| row.url.map(str::to_owned)),
                name,
                measurements: measurements
                    .into_iter()
                    .map(|m| Strings::new(Voice::Plain).measurement(m).to_owned())
                    .collect(),
                license: row.license.map(str::to_owned),
                needs_key: row.needs_key,
                key_optional: row.key_optional,
                opt_in: row.opt_in,
                implemented: source.is_some(),
            }
        })
        .collect();
    debug_assert_eq!(infos.len(), KNOWN_SOURCES.len());
    infos
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_catalogue_follows_the_known_sources() {
        let ids: Vec<String> = source_catalogue().into_iter().map(|s| s.id).collect();
        assert_eq!(ids, KNOWN_SOURCES);
    }
}
