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
use underbite_core::view::format::Formatter;
use underbite_core::view::strings::{Key, Strings, source_name};
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
    let group = |id: &str,
                 label: &str,
                 options: Vec<UnitOption>,
                 selected: String,
                 follows: Option<&str>,
                 overridden: bool| UnitGroup {
        id: id.to_owned(),
        label: label.to_owned(),
        options,
        selected,
        follows: follows.map(str::to_owned),
        overridden,
    };
    Ok(vec![
        group(
            "discharge",
            "River flow",
            options(&DischargeUnit::ALL, DischargeUnit::symbol),
            unit_id(u.discharge),
            None,
            false,
        ),
        group(
            "height",
            "Heights",
            heights(),
            unit_id(u.height),
            None,
            false,
        ),
        group(
            "stage",
            "River stage",
            heights(),
            unit_id(u.stage()),
            Some("height"),
            u.stage.is_some(),
        ),
        group(
            "lake_level",
            "Lake level",
            heights(),
            unit_id(u.lake_level()),
            Some("height"),
            u.lake_level.is_some(),
        ),
        group(
            "tide",
            "Tide height",
            heights(),
            unit_id(u.tide()),
            Some("height"),
            u.tide.is_some(),
        ),
        group(
            "waves",
            "Wave height",
            heights(),
            unit_id(u.waves()),
            Some("height"),
            u.waves.is_some(),
        ),
        group("speed", "Speeds", speeds(), unit_id(u.speed), None, false),
        group(
            "current",
            "Current speed",
            speeds(),
            unit_id(u.current()),
            Some("speed"),
            u.current.is_some(),
        ),
        group(
            "wind",
            "Wind speed",
            speeds(),
            unit_id(u.wind()),
            Some("speed"),
            u.wind.is_some(),
        ),
        group(
            "water_temp",
            "Water temperature",
            temps(),
            unit_id(u.water_temp),
            None,
            false,
        ),
        group(
            "air_temp",
            "Air temperature",
            temps(),
            unit_id(u.air_temp()),
            Some("water_temp"),
            u.air_temp.is_some(),
        ),
        group(
            "pressure",
            "Air pressure",
            options(&PressureUnit::ALL, PressureUnit::symbol),
            unit_id(u.pressure),
            None,
            false,
        ),
        group(
            "distance",
            "Distances",
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

/// Metres per kilometre and per statute mile, as the formatter's distances use.
const M_PER_KM: f64 = 1_000.0;
const M_PER_MI: f64 = 1_609.344;

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
            Self::Distance(u) => {
                return match u {
                    DistanceUnit::Km => v * M_PER_KM,
                    DistanceUnit::Mi => v * M_PER_MI,
                };
            }
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
            Self::Distance(u) => {
                return match u {
                    DistanceUnit::Km => c / M_PER_KM,
                    DistanceUnit::Mi => c / M_PER_MI,
                };
            }
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
    let mut unit = Unit::selected(kind, &units);
    let mut number = text.trim();
    let lower = number.to_lowercase();
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
    if let Some((symbol, found)) = candidates.iter().find(|(s, _)| lower.ends_with(s.as_str())) {
        unit = *found;
        number = number[..number.len() - symbol.len()].trim_end();
    }
    let cleaned: String = number.chars().filter(|c| *c != ',').collect();
    let value: f64 = cleaned
        .trim()
        .parse()
        .map_err(|_| invalid(format!("`{}` is not a number", text.trim())))?;
    if !value.is_finite() {
        return Err(invalid(format!("`{}` is not a number", text.trim())));
    }
    Ok(unit.to_canonical(value))
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
    let (low, high) = (unit.to_display(min), unit.to_display(max));
    if !(low.is_finite() && high.is_finite() && high > low) {
        return Err(invalid("the range needs a minimum below its maximum"));
    }
    let raw = (high - low) / 4.0;
    let magnitude = 10f64.powf(raw.log10().floor());
    let normal = raw / magnitude;
    let step = magnitude
        * if normal <= 1.0 {
            1.0
        } else if normal <= 2.0 {
            2.0
        } else if normal <= 5.0 {
            5.0
        } else {
            10.0
        };
    let formatter = Formatter::new(units, Voice::Plain, TimeFormat::H12, "UTC");
    let mut stops = Vec::new();
    let mut k = (low / step).ceil();
    while k * step <= high + step * 1e-9 && stops.len() < 64 {
        let value = unit.to_canonical(k * step);
        stops.push(DisplayStop {
            value,
            label: label(&formatter, kind, value),
        });
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

/// What a measurement is called, in the core's words.
pub(crate) fn measurement_label(m: Measurement) -> &'static str {
    Strings::new(Voice::Plain).get(match m {
        Measurement::WaterTemperature => Key::MeasWaterTemperature,
        Measurement::Discharge => Key::MeasDischarge,
        Measurement::GageHeight => Key::MeasGageHeight,
        Measurement::LakeLevel => Key::MeasLakeLevel,
        Measurement::Turbidity => Key::MeasTurbidity,
        Measurement::DissolvedOxygen => Key::MeasDissolvedOxygen,
        Measurement::Conductivity => Key::MeasConductivity,
        Measurement::Salinity => Key::MeasSalinity,
        Measurement::Ph => Key::MeasPh,
        Measurement::Chlorophyll => Key::MeasChlorophyll,
        Measurement::Clarity => Key::MeasClarity,
        Measurement::Cyanobacteria => Key::MeasCyanobacteria,
        Measurement::Tide => Key::MeasTide,
        Measurement::TidalCurrent => Key::MeasTidalCurrent,
        Measurement::OceanCurrent => Key::MeasOceanCurrent,
        Measurement::Waves => Key::MeasWaves,
        Measurement::WavesForecast => Key::MeasWavesForecast,
        Measurement::SeaSurfaceTemp => Key::MeasSeaSurfaceTemp,
        Measurement::SeaSurfaceTempForecast => Key::MeasSeaSurfaceTempForecast,
        Measurement::MarineForecast => Key::MeasMarineForecast,
        Measurement::MarineAlerts => Key::MeasMarineAlerts,
        Measurement::FloodCategories => Key::MeasFloodCategories,
        Measurement::FlowForecast => Key::MeasFlowForecast,
        Measurement::ObservedWind => Key::MeasWind,
        Measurement::ObservedPressure => Key::MeasPressure,
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
                    .map(|m| measurement_label(m).to_owned())
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
