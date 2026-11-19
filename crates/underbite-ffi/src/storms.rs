//! The map's Storms layer across the FFI: the forecast models' low-pressure tracks and NHC's
//! active hurricanes around the centre of the map's view, fetched through the app's host and
//! worded by the core's view.
//!
//! The storm outlook is whether's (`whether_core::storm`): the four deterministic models' hours
//! and their sea-level pressure on a 2.5° grid around a point, from Open-Meteo, and NHC's
//! active storms with their tracks. Here it is fetched for the view's centre, snapped to the
//! grid's quarter degree and rounded to the settings' precision before it leaves the device, and
//! kept in the host's store (one outlook for the map). A call asks for nothing while the centre
//! stays within one grid cell ([`RECENTRE_DEG`]) of the kept outlook's and it was checked less
//! than [`CHECK_EVERY_S`] ago. Past that it reads the models' run times (`meta.json`, small
//! static files) and NHC's active storms; the models' data (two metered calls, counted in the
//! shared usage ledger with every other Open-Meteo call) is fetched again only when a model has
//! a newer run, or the centre moved more than one grid cell. NHC's files carry no coordinates;
//! a storm's tracks are read once per forecast advisory (`underbite_providers::storms`).
//!
//! The view re-expresses whether's storms view through Underbite's formatter: every number,
//! time and word is the core's, in the user's units and the device's time zone. The app draws
//! each model's lows as a translucent band (the storm near the centre wider), NHC's official
//! track, the guidance tracks and the observed track, and NHC's forecast cone, which is labelled
//! as NHC's cone built from NHC's yearly cone sizes and never as an impact area. Hurricanes are
//! absent when none are active. Longitudes stay in `-180..180`: the app draws across the
//! antimeridian the short way. Every word about storms is safety text, never in Luna's voice.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use underbite_config::schema::Config;
use underbite_core::model::{SourceError as ModelSourceError, Spot, StormWatch, WaterType};
use underbite_core::time::UnixTime;
use underbite_core::view::format::Formatter;
use underbite_core::view::strings::{
    SafetyKey, advisory_wind, hurricane_movement, hurricane_name, storm_advisory, storm_cone_note,
    storm_day_time, storm_model_run, storm_span, storm_step,
};
use underbite_fetch::store::{self, Stored};
use underbite_fetch::{Call, Fetched};
use underbite_providers::request::CacheHint;
use underbite_providers::request::Coordinates;
use underbite_providers::session::usage::UsageLedger;
use underbite_providers::source::{FetchContext, SourceError};
use underbite_providers::storms::{OPEN_METEO_STORM, POINT_BUDGET, StormOptions, StormWatchPlan};
use whether_core::model::ResolvedLocation;
use whether_core::models::short_name;
use whether_core::scales::saffir_simpson;
use whether_core::storm::analysis::{self, StormEvent};
use whether_core::storm::hurricane::{CONE_SEASON, CONE_SOURCE, DRAWN_AIDS, cone};
use whether_core::storm::track::{self, Track};
use whether_core::storm::{
    DETERMINISTIC_MODELS, GRID_MAX_LATITUDE, GRID_SNAP_DEG, GRID_SPACING_DEG, Hurricane, ModelRole,
    ModelRun, StormOutlook, TrackFix, wrap_longitude,
};
use whether_providers::open_meteo::storm as om;
use whether_providers::{HttpRequest, HttpResponse, check_status};

use crate::fetch::{FetchHost, SourceIssue, fetched, host_request, user_agent};
use crate::map::{MapColour, MapError, formatter, invalid};
use crate::settings::load;

// ---------------------------------------------------------------------------------------------
// Limits
// ---------------------------------------------------------------------------------------------

/// The catalogue id of the layer.
pub const STORMS_LAYER: &str = "storms";

/// How far the view's centre may move from the kept outlook's before the models' data is
/// fetched again, degrees: one cell of the pressure grid.
pub const RECENTRE_DEG: f64 = GRID_SPACING_DEG as f64;

/// How long a kept outlook stands without asking anything, seconds. After that a call reads the
/// run times and NHC's active storms again.
pub const CHECK_EVERY_S: i64 = 15 * 60;

/// How long a kept outlook is trusted when the run times cannot be read: one model update.
const TRUSTED_WITHOUT_META_S: i64 = 6 * 3_600;

/// How long the host may reuse a `meta.json`, which is the same for everyone, seconds.
const META_MAX_AGE_S: u32 = 600;

/// Besides the storm, the most other lows drawn per model.
const OTHER_LOWS: usize = 2;

/// The host store's namespace and key of the map's outlook.
const STORE_NAMESPACE: &str = "storms";
const STORE_KEY: &str = "map-outlook";

/// A coordinate rounded to 3 decimals (about 100 m) for drawing.
fn coordinate(x: f64) -> f64 {
    (x * 1_000.0).round() / 1_000.0
}

// ---------------------------------------------------------------------------------------------
// The view
// ---------------------------------------------------------------------------------------------

/// A position of a track, with when the centre is (or was) there, Unix seconds.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct StormPoint {
    pub latitude: f64,
    /// In `-180..180`.
    pub longitude: f64,
    pub time: i64,
}

/// A corner of NHC's forecast cone.
#[derive(Clone, Copy, Debug, PartialEq, uniffi::Record)]
pub struct StormCoordinate {
    pub latitude: f64,
    pub longitude: f64,
}

/// One of the forecast models whose lows are drawn.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct StormModel {
    /// The Open-Meteo model id (`ecmwf_ifs025`).
    pub model: String,
    /// "ECMWF".
    pub short: String,
    /// The model family, which picks the band's dash: `ecmwf`, `gfs`, `gem`, `icon`, `other`.
    pub role: String,
    pub colour: MapColour,
    /// "ECMWF lows · 12Z run".
    pub label: String,
}

/// A low's position at one time, with its labels.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct StormTrackPoint {
    pub latitude: f64,
    pub longitude: f64,
    pub time: i64,
    /// The low's mark: "ECMWF 978 hPa".
    pub label: String,
    /// As read aloud: "ECMWF low, 978 hPa".
    pub spoken: String,
}

/// One model's low, drawn as a band.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct StormTrack {
    pub model: String,
    pub short: String,
    pub role: String,
    pub colour: MapColour,
    /// The low that brings the storm near the map's centre: drawn wider and on top.
    pub is_storm: bool,
    /// Oldest first.
    pub points: Vec<StormTrackPoint>,
}

/// A point of NHC's official forecast track.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct StormForecastPoint {
    pub latitude: f64,
    pub longitude: f64,
    pub time: i64,
    /// When the centre is forecast there: "Fri 2 AM".
    pub label: String,
    /// The forecast sustained wind as NHC rounds it: "75 mph".
    pub wind: String,
    /// "Hurricane Josephine, official forecast, Fri 2 AM, 75 mph".
    pub spoken: String,
}

/// A guidance model's track for a cyclone, drawn as a narrower band.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct StormAid {
    /// The ATCF aid code (`AVNI`).
    pub aid: String,
    /// "GFS", "UKMET", "Consensus".
    pub short: String,
    /// `gfs`, `gem`, `ecmwf`, `ukmet`, `hafs`, `consensus` or `other`.
    pub role: String,
    pub colour: MapColour,
    pub points: Vec<StormPoint>,
}

/// One active tropical cyclone from NHC: its advisory, tracks and forecast cone.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct StormHurricane {
    /// NHC's id (`al102026`).
    pub id: String,
    pub name: String,
    /// "Hurricane Josephine · Category 1", "Subtropical Storm Josephine".
    pub title: String,
    /// NHC's classification code (`HU`, `TS`, `STS`, ...).
    pub classification: String,
    pub category: Option<u8>,
    /// `watch`, `strong`, `severe` or `extreme`, as the storm-ahead card colours it.
    pub severity: String,
    /// The colour of its symbol.
    pub colour: MapColour,
    /// The colour of its official and observed tracks.
    pub track_colour: MapColour,
    /// The colour of its forecast cone.
    pub cone_colour: MapColour,
    pub latitude: f64,
    pub longitude: f64,
    pub wind: String,
    pub pressure: String,
    pub movement: String,
    /// "Winds 65 mph · 29.18 inHg · NE at 13 mph".
    pub strength: String,
    /// "Advisory Thu 5:00 PM".
    pub advisory: String,
    /// NHC's advisory page.
    pub advisory_url: Option<String>,
    /// As read aloud: the title and the strength.
    pub spoken: String,
    /// NHC's official track, from the advisory's position on.
    pub official: Vec<StormForecastPoint>,
    /// The guidance models, one per family, in drawing order.
    pub aids: Vec<StormAid>,
    /// The observed track, oldest first.
    pub past: Vec<StormPoint>,
    /// NHC's forecast cone around the official track: a ring, counter-clockwise, not closed.
    pub cone: Vec<StormCoordinate>,
}

/// A time step of the layer: which point of each low is marked.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct StormStep {
    pub time: i64,
    /// "Sun 7 PM · +72 h".
    pub label: String,
}

/// What a legend row shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum StormLegendKind {
    /// A model's lows: a band in its colour and dash.
    ModelBand,
    /// The wider band of the storm near the centre.
    StormBand,
    /// NHC's official forecast track: a solid line.
    OfficialTrack,
    /// The observed track: a dotted line.
    ObservedTrack,
    /// A guidance model's track: a narrow band.
    GuidanceBand,
    /// NHC's forecast cone.
    Cone,
}

/// One row of the layer's legend.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct StormLegendItem {
    pub kind: StormLegendKind,
    /// The model family, for the dash (empty for the cone and NHC's tracks).
    pub role: String,
    pub colour: MapColour,
    /// What it is (safety text).
    pub label: String,
}

/// Everything the Storms layer draws and says.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct StormLayer {
    /// The point the outlook is for (the view's centre, snapped and rounded).
    pub latitude: f64,
    pub longitude: f64,
    /// "A storm passes near the map's centre · Sat night – Mon night", or "No storm in the next
    /// 10 days near the map's centre. Lowest pressure: …"; none without the models' hours.
    pub summary: Option<String>,
    pub models: Vec<StormModel>,
    /// Per model, the storm's low first, then the others, deepest first.
    pub tracks: Vec<StormTrack>,
    /// Active cyclones; empty when NHC lists none near.
    pub hurricanes: Vec<StormHurricane>,
    /// The time steps, from the one at or before now.
    pub steps: Vec<StormStep>,
    pub legend: Vec<StormLegendItem>,
    /// What NHC's forecast cone is, when a cone is drawn (safety text).
    pub cone_note: Option<String>,
    /// NHC's page on how the cone is made, and its title.
    pub cone_link: String,
    pub cone_link_title: String,
    /// The title of a hurricane's advisory link ("NHC advisory").
    pub advisory_link_title: String,
    /// "Storm forecasts change from one run to the next …" (safety text).
    pub safety: String,
    /// "Storm outlook checked Nov 18, 5:00 PM · modelled"; none for an outlook never checked.
    pub checked: Option<String>,
    /// Why something is missing: part of the data, or all of it, could not be loaded.
    pub status: Option<String>,
    /// The credits of what is drawn (safety text).
    pub credits: Vec<String>,
    /// "Not for navigation." (safety text).
    pub notice: String,
}

/// What [`fetch_storm_layer`] did.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct StormLayerOutcome {
    pub layer: StormLayer,
    /// Requests sent through the host (none when the kept outlook stood).
    pub requests: u32,
    /// Whether the models' data was fetched now.
    pub refreshed: bool,
    /// What failed; the layer keeps what it had.
    pub errors: Vec<SourceIssue>,
}

/// The words of the layer before any outlook is there.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct StormLayerWords {
    /// "Loading the storm outlook…".
    pub loading: String,
    /// "The storm outlook could not be loaded. Try again later."
    pub failed: String,
    pub safety: String,
    pub notice: String,
}

fn rgb(hex: u32) -> MapColour {
    MapColour {
        red: ((hex >> 16) & 0xff) as u8,
        green: ((hex >> 8) & 0xff) as u8,
        blue: (hex & 0xff) as u8,
        alpha: 255,
    }
}

/// A model family's colour: the Okabe-Ito palette, told apart by people with colour vision
/// deficiencies, and the app draws each with its own dash besides.
fn role_colour(role: ModelRole) -> MapColour {
    rgb(match role {
        ModelRole::Ecmwf => 0x00_72_B2,
        ModelRole::Gfs => 0xD5_5E_00,
        ModelRole::Gem => 0x00_9E_73,
        ModelRole::Icon => 0xCC_79_A7,
        ModelRole::Ukmet => 0xE6_9F_00,
        ModelRole::Hafs => 0x56_B4_E9,
        ModelRole::Consensus => 0x8C_56_4B,
        ModelRole::Official => 0x1A_1A_1A,
        ModelRole::Other => 0x8E_8E_93,
    })
}

/// NHC's cone, drawn grey under a crisp edge.
const CONE_COLOUR: u32 = 0x6B_6B_6B;

const fn role_name(role: ModelRole) -> &'static str {
    match role {
        ModelRole::Ecmwf => "ecmwf",
        ModelRole::Gfs => "gfs",
        ModelRole::Gem => "gem",
        ModelRole::Icon => "icon",
        ModelRole::Ukmet => "ukmet",
        ModelRole::Hafs => "hafs",
        ModelRole::Consensus => "consensus",
        ModelRole::Official => "official",
        ModelRole::Other => "other",
    }
}

fn is_hurricane(h: &Hurricane) -> bool {
    matches!(h.classification.as_str(), "HU" | "TY")
}

fn category(h: &Hurricane) -> Option<u8> {
    if is_hurricane(h) {
        h.wind_kt.and_then(saffir_simpson)
    } else {
        None
    }
}

/// The severity name and colour of a cyclone, as the storm-ahead card has it.
fn severity(h: &Hurricane) -> (&'static str, MapColour) {
    let name = match category(h) {
        Some(3..) => "extreme",
        Some(_) => "severe",
        None if is_hurricane(h) => "severe",
        None if matches!(h.classification.as_str(), "TS" | "STS") => "strong",
        None => "watch",
    };
    let colour = match name {
        "extreme" => 0x7B_1F_A2,
        "severe" => 0xC6_28_28,
        "strong" => 0xD5_5E_00,
        _ => 0xB2_81_00,
    };
    (name, rgb(colour))
}

/// The short name of a guidance aid: its model family's, else its code.
fn aid_short(f: &Formatter, aid: &str) -> String {
    match ModelRole::of_aid(aid) {
        ModelRole::Gfs => "GFS".to_owned(),
        ModelRole::Gem => "GEM".to_owned(),
        ModelRole::Ecmwf => "ECMWF".to_owned(),
        ModelRole::Ukmet => "UKMET".to_owned(),
        ModelRole::Hafs => "HAFS".to_owned(),
        ModelRole::Consensus => f.strings.safety(SafetyKey::StormLayerConsensus).to_owned(),
        ModelRole::Official => "NHC".to_owned(),
        ModelRole::Icon | ModelRole::Other => aid.to_owned(),
    }
}

fn point(fix: &TrackFix) -> StormPoint {
    StormPoint {
        latitude: coordinate(fix.latitude),
        longitude: coordinate(fix.longitude),
        time: fix.time.0,
    }
}

/// The guidance tracks: whether's drawn aids in order, then any other, one per model family (so
/// ECMWF's raw run is drawn only without its interpolated one).
fn aids(f: &Formatter, h: &Hurricane) -> Vec<StormAid> {
    let order = DRAWN_AIDS.iter().copied().chain(
        h.aids
            .keys()
            .map(String::as_str)
            .filter(|a| !DRAWN_AIDS.contains(a)),
    );
    let mut out: Vec<StormAid> = Vec::new();
    for aid in order {
        let Some(fixes) = h.aids.get(aid).filter(|fixes| fixes.len() > 1) else {
            continue;
        };
        let role = ModelRole::of_aid(aid);
        if role != ModelRole::Other && out.iter().any(|t| t.role == role_name(role)) {
            continue;
        }
        out.push(StormAid {
            aid: aid.to_owned(),
            short: aid_short(f, aid),
            role: role_name(role).to_owned(),
            colour: role_colour(role),
            points: fixes.iter().map(point).collect(),
        });
    }
    out
}

fn hurricane(f: &Formatter, zone: &str, h: &Hurricane) -> StormHurricane {
    let name = hurricane_name(f, &h.classification, &h.name);
    let category = category(h);
    let title = category.map_or_else(
        || name.clone(),
        |n| {
            f.strings.safety_fmt(
                SafetyKey::HurricaneCategory,
                &[("name", &name), ("category", &n.to_string())],
            )
        },
    );
    let wind = advisory_wind(f, h.wind_kt).full();
    let pressure = f.pressure(h.pressure_hpa).full();
    let movement = hurricane_movement(f, h.movement_deg, h.movement_kt);
    let strength = f.strings.safety_fmt(
        SafetyKey::StormStrength,
        &[
            ("wind", &wind),
            ("pressure", &pressure),
            ("movement", &movement),
        ],
    );
    let (severity, colour) = severity(h);
    let official = h
        .official
        .iter()
        .map(|fix| {
            let label = storm_day_time(f, zone, fix.time);
            let wind = advisory_wind(f, fix.wind_kt).full();
            StormForecastPoint {
                latitude: coordinate(fix.latitude),
                longitude: coordinate(fix.longitude),
                time: fix.time.0,
                spoken: f.strings.safety_fmt(
                    SafetyKey::StormLayerForecastSpoken,
                    &[("storm", &name), ("time", &label), ("wind", &wind)],
                ),
                label,
                wind,
            }
        })
        .collect();
    StormHurricane {
        id: h.id.clone(),
        name: h.name.clone(),
        spoken: f.strings.safety_fmt(
            SafetyKey::StormLayerHurricaneSpoken,
            &[("storm", &title), ("strength", &strength)],
        ),
        title,
        classification: h.classification.clone(),
        category,
        severity: severity.to_owned(),
        colour,
        track_colour: role_colour(ModelRole::Official),
        cone_colour: rgb(CONE_COLOUR),
        latitude: coordinate(h.latitude),
        longitude: coordinate(h.longitude),
        wind,
        pressure,
        movement,
        strength,
        advisory: storm_advisory(f, zone, h.advisory_time),
        advisory_url: h.advisory_url.clone(),
        official,
        aids: aids(f, h),
        past: h.past.iter().map(point).collect(),
        cone: cone(&h.official, h.basin)
            .into_iter()
            .map(|(lat, lon)| StormCoordinate {
                latitude: coordinate(lat),
                longitude: coordinate(lon),
            })
            .collect(),
    }
}

/// When the rain or snow of `event` falls: its window without the padding the analysis adds,
/// except where the window meets the edge of the data.
fn wet_span(outlook: &StormOutlook, event: &StormEvent) -> (UnixTime, UnixTime) {
    let first = outlook.points.iter().filter_map(|p| p.times.first()).min();
    let last = outlook.points.iter().filter_map(|p| p.times.last()).max();
    let pad = analysis::EVENT_PAD_SECS;
    let start = match first {
        Some(first) if event.start.0 > first.0 - 3_600 => event.start.0 + pad,
        _ => event.start.0,
    };
    let end = match last {
        Some(last) if event.end.0 < last.0 => event.end.0 - pad,
        _ => event.end.0,
    };
    if end > start {
        (UnixTime(start), UnixTime(end))
    } else {
        (event.start, event.end)
    }
}

/// The storm near the centre, or that there is none and the deepest low.
fn summary(
    f: &Formatter,
    zone: &str,
    outlook: &StormOutlook,
    events: &[StormEvent],
) -> Option<String> {
    if let Some(event) = events.first() {
        let (start, end) = wet_span(outlook, event);
        return Some(f.strings.safety_fmt(
            SafetyKey::StormLayerEvent,
            &[("window", &storm_span(f, zone, start, end))],
        ));
    }
    let first = outlook
        .points
        .iter()
        .filter_map(|p| p.times.first())
        .min()?;
    let last = outlook.points.iter().filter_map(|p| p.times.last()).max()?;
    // Hourly amounts cover the hour before their time.
    let days = ((last.0 - first.0 + 3_600 + 43_200) / 86_400).max(1);
    let mut text = f
        .strings
        .safety_fmt(SafetyKey::StormLayerNone, &[("n", &days.to_string())]);
    if let Some((model, hpa, at)) = analysis::deepest_low(outlook) {
        text.push(' ');
        text.push_str(&f.strings.safety_fmt(
            SafetyKey::StormLayerLowest,
            &[
                ("pressure", &f.pressure(Some(hpa)).full()),
                ("when", &storm_span(f, zone, at, UnixTime(at.0 + 1))),
                ("model", short_name(&model)),
            ],
        ));
    }
    Some(text)
}

/// The lows drawn: per grid, the storm of the first event first, then the others.
fn tracks(f: &Formatter, outlook: &StormOutlook, event: Option<&StormEvent>) -> Vec<StormTrack> {
    let place = (outlook.place.latitude, outlook.place.longitude);
    let mut out = Vec::new();
    for grid in &outlook.grids {
        let lows: Vec<Track> = track::track_lows(grid);
        let storm = event.and_then(|e| track::storm_track(&lows, place, (e.start, e.end)));
        let role = ModelRole::of(&grid.model);
        let short = short_name(&grid.model);
        for (low, is_storm) in track::keep_tracks(&lows, storm, OTHER_LOWS) {
            let points = low
                .points
                .iter()
                .map(|p| {
                    let pressure = f.pressure(Some(p.pressure_hpa)).full();
                    let args = [("model", short), ("pressure", pressure.as_str())];
                    StormTrackPoint {
                        latitude: coordinate(p.latitude),
                        longitude: coordinate(p.longitude),
                        time: p.time.0,
                        label: f.strings.safety_fmt(SafetyKey::StormLayerLow, &args),
                        spoken: f.strings.safety_fmt(SafetyKey::StormLayerLowSpoken, &args),
                    }
                })
                .collect();
            out.push(StormTrack {
                model: grid.model.clone(),
                short: short.to_owned(),
                role: role_name(role).to_owned(),
                colour: role_colour(role),
                is_storm,
                points,
            });
        }
    }
    out
}

/// The models whose hours or lows the outlook holds, in its order.
fn models(f: &Formatter, outlook: &StormOutlook) -> Vec<StormModel> {
    let ids = outlook
        .points
        .iter()
        .map(|p| &p.model)
        .chain(outlook.grids.iter().map(|g| &g.model));
    let mut out: Vec<StormModel> = Vec::new();
    for model in ids {
        if out.iter().any(|m| &m.model == model) {
            continue;
        }
        // The point's run, else the grid's.
        let run = outlook
            .points
            .iter()
            .filter(|p| &p.model == model)
            .find_map(|p| p.run.as_ref())
            .or_else(|| {
                outlook
                    .grids
                    .iter()
                    .filter(|g| &g.model == model)
                    .find_map(|g| g.run.as_ref())
            });
        let role = ModelRole::of(model);
        let short = short_name(model);
        out.push(StormModel {
            model: model.clone(),
            short: short.to_owned(),
            role: role_name(role).to_owned(),
            colour: role_colour(role),
            label: storm_model_run(f, short, run.map(|r| r.init)),
        });
    }
    out
}

/// The time steps: the grids' times (else the hours' on 6-hour boundaries), from the last one
/// at or before `now`.
fn steps(f: &Formatter, zone: &str, outlook: &StormOutlook, now: UnixTime) -> Vec<StormStep> {
    const SIX_HOURS: i64 = 6 * 3_600;
    let mut times: Vec<UnixTime> = outlook
        .grids
        .iter()
        .flat_map(|g| g.times.iter().copied())
        .collect();
    if times.is_empty() {
        times = outlook
            .points
            .iter()
            .flat_map(|p| p.times.iter().copied())
            .filter(|t| t.0.rem_euclid(SIX_HOURS) == 0)
            .collect();
    }
    times.sort_unstable();
    times.dedup();
    let first = times.iter().rposition(|t| *t <= now).unwrap_or(0);
    times
        .iter()
        .skip(first)
        .map(|t| StormStep {
            time: t.0,
            label: storm_step(f, zone, *t, now),
        })
        .collect()
}

fn legend(
    f: &Formatter,
    models: &[StormModel],
    tracks: &[StormTrack],
    hurricanes: &[StormHurricane],
) -> Vec<StormLegendItem> {
    let s = f.strings;
    let mut out: Vec<StormLegendItem> = models
        .iter()
        .filter(|m| tracks.iter().any(|t| t.model == m.model))
        .map(|m| StormLegendItem {
            kind: StormLegendKind::ModelBand,
            role: m.role.clone(),
            colour: m.colour,
            label: m.label.clone(),
        })
        .collect();
    if tracks.iter().any(|t| t.is_storm) {
        out.push(StormLegendItem {
            kind: StormLegendKind::StormBand,
            role: String::new(),
            colour: role_colour(ModelRole::Other),
            label: s.safety(SafetyKey::StormLayerStormBand).to_owned(),
        });
    }
    if hurricanes.is_empty() {
        return out;
    }
    out.push(StormLegendItem {
        kind: StormLegendKind::OfficialTrack,
        role: String::new(),
        colour: role_colour(ModelRole::Official),
        label: s.safety(SafetyKey::StormLayerOfficial).to_owned(),
    });
    if hurricanes.iter().any(|h| h.past.len() > 1) {
        out.push(StormLegendItem {
            kind: StormLegendKind::ObservedTrack,
            role: String::new(),
            colour: role_colour(ModelRole::Official),
            label: s.safety(SafetyKey::StormLayerObserved).to_owned(),
        });
    }
    let mut seen: Vec<&str> = Vec::new();
    for aid in hurricanes.iter().flat_map(|h| &h.aids) {
        if seen.contains(&aid.role.as_str()) && aid.role != "other" {
            continue;
        }
        seen.push(&aid.role);
        out.push(StormLegendItem {
            kind: StormLegendKind::GuidanceBand,
            role: aid.role.clone(),
            colour: aid.colour,
            label: s.safety_fmt(SafetyKey::StormLayerGuidance, &[("model", &aid.short)]),
        });
    }
    out.push(StormLegendItem {
        kind: StormLegendKind::Cone,
        role: String::new(),
        colour: rgb(CONE_COLOUR),
        label: storm_cone_note(f, CONE_SEASON),
    });
    out
}

/// The layer for `outlook` at `now`, in the settings' units and `zone`'s times.
fn layer_view(
    config: &Config,
    zone: &str,
    outlook: &StormOutlook,
    checked_at: Option<UnixTime>,
    failed: bool,
    now: UnixTime,
) -> StormLayer {
    let f = formatter(config, zone);
    let s = f.strings;
    let events = analysis::find_events(outlook, now);
    let tracks = tracks(&f, outlook, events.first());
    let models = models(&f, outlook);
    let hurricanes: Vec<StormHurricane> = outlook
        .hurricanes
        .iter()
        .map(|h| hurricane(&f, zone, h))
        .collect();
    let has_models = !outlook.points.is_empty() || !outlook.grids.is_empty();
    let mut credits = Vec::new();
    if has_models {
        credits.push(s.safety(SafetyKey::CreditOpenMeteoModels).to_owned());
    }
    if !hurricanes.is_empty() {
        credits.push(s.safety(SafetyKey::CreditNhc).to_owned());
    }
    let status = match (failed, has_models || !hurricanes.is_empty()) {
        (false, _) => None,
        (true, true) => Some(s.safety(SafetyKey::StormLayerPartial).to_owned()),
        (true, false) => Some(s.safety(SafetyKey::StormLayerFailed).to_owned()),
    };
    StormLayer {
        latitude: outlook.place.latitude,
        longitude: outlook.place.longitude,
        summary: summary(&f, zone, outlook, &events),
        legend: legend(&f, &models, &tracks, &hurricanes),
        cone_note: (!hurricanes.is_empty()).then(|| storm_cone_note(&f, CONE_SEASON)),
        cone_link: CONE_SOURCE.to_owned(),
        cone_link_title: s.safety(SafetyKey::StormLayerConeLink).to_owned(),
        advisory_link_title: s.safety(SafetyKey::StormLayerAdvisoryLink).to_owned(),
        steps: steps(&f, zone, outlook, now),
        models,
        tracks,
        hurricanes,
        safety: s.safety(SafetyKey::StormForecastsChangeText).to_owned(),
        checked: checked_at.map(|t| {
            s.safety_fmt(
                SafetyKey::StormChecked,
                &[("date", &f.date_of(t)), ("time", &f.clock(t))],
            )
        }),
        status,
        credits,
        notice: s.safety(SafetyKey::NotForNavigation).to_owned(),
    }
}

// ---------------------------------------------------------------------------------------------
// The kept outlook
// ---------------------------------------------------------------------------------------------

/// The map's outlook as the host keeps it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct MapOutlook {
    /// When the run times and NHC were last read for it.
    checked_at: UnixTime,
    outlook: StormOutlook,
}

fn kept(host: &dyn FetchHost) -> Option<MapOutlook> {
    let text = host.load(STORE_NAMESPACE.to_owned(), STORE_KEY.to_owned())?;
    serde_json::from_str(&text).ok()
}

fn keep(host: &dyn FetchHost, value: &MapOutlook) {
    if let Ok(json) = serde_json::to_string(value) {
        host.store(STORE_NAMESPACE.to_owned(), STORE_KEY.to_owned(), json);
    }
}

/// The view's centre as it may leave the device: snapped to the pressure grid's quarter degree,
/// within the grid's latitudes, and rounded to the settings' precision.
fn centre(latitude: f64, longitude: f64, ctx: &FetchContext) -> Result<(f64, f64), MapError> {
    if !(latitude.is_finite() && longitude.is_finite()) || latitude.abs() > 90.0 {
        return Err(invalid(format!(
            "{latitude}, {longitude} is not a point on the map"
        )));
    }
    let snap = |v: f64| (v / GRID_SNAP_DEG).round() * GRID_SNAP_DEG;
    let limit = f64::from(GRID_MAX_LATITUDE);
    let lat = snap(latitude.clamp(-limit, limit));
    let lon = wrap_longitude(snap(wrap_longitude(longitude)));
    let rounded = ctx
        .round(Coordinates {
            latitude: lat,
            longitude: lon,
        })
        .coordinates();
    Ok((rounded.latitude, rounded.longitude))
}

/// Whether `outlook` stands for the area around `centre`: within one grid cell.
fn same_area(outlook: &StormOutlook, centre: (f64, f64)) -> bool {
    (outlook.place.latitude - centre.0).abs() <= RECENTRE_DEG
        && wrap_longitude(outlook.place.longitude - centre.1).abs() <= RECENTRE_DEG
}

/// Whether the models' data in `outlook` is from their latest runs in `runs`. A model missing
/// from a part the outlook has is current when the outlook was fetched after its latest run
/// was out (Open-Meteo leaves some models out); a part that is missing altogether is not. When
/// some run times could not be read, the outlook stands for [`TRUSTED_WITHOUT_META_S`].
fn models_current(
    outlook: &StormOutlook,
    runs: &BTreeMap<&'static str, ModelRun>,
    now: UnixTime,
) -> bool {
    if outlook.points.is_empty() && outlook.grids.is_empty() {
        return false;
    }
    if runs.len() < om::RUN_DOMAINS.len() && now.0 - outlook.fetched_at.0 >= TRUSTED_WITHOUT_META_S
    {
        return false;
    }
    DETERMINISTIC_MODELS.iter().all(|m| {
        let latest = om::run_domain(m).and_then(|d| runs.get(d));
        let fresh = |run: Option<Option<&ModelRun>>, answered: bool| match (run, latest) {
            (Some(run), Some(latest)) => run.is_some_and(|r| r.init >= latest.init),
            (None, Some(latest)) => answered && outlook.fetched_at >= latest.available,
            (run, None) => run.is_some(),
        };
        let point = outlook.points.iter().find(|p| p.model == *m);
        let grid = outlook.grids.iter().find(|g| g.model == *m);
        fresh(point.map(|p| p.run.as_ref()), !outlook.points.is_empty())
            && fresh(grid.map(|g| g.run.as_ref()), !outlook.grids.is_empty())
    })
}

// ---------------------------------------------------------------------------------------------
// The fetch
// ---------------------------------------------------------------------------------------------

/// One call's way out: requests one at a time through the host, and the call ledger, which
/// counts and saves a metered request before it goes out.
struct Link<'h> {
    host: &'h dyn FetchHost,
    now: UnixTime,
    ledger: Option<UsageLedger>,
    requests: u32,
    errors: Vec<ModelSourceError>,
    /// Whether data the layer draws (the models' hours or grid, NHC's storms) failed; the run
    /// times are not counted, as a kept outlook stands without them.
    data_failed: bool,
}

impl Link<'_> {
    fn ledger(&mut self) -> &mut UsageLedger {
        let host = self.host;
        let now = self.now;
        self.ledger.get_or_insert_with(|| {
            let mut ledger = host
                .load(store::USAGE.to_owned(), store::LEDGER_KEY.to_owned())
                .and_then(|text| serde_json::from_str::<Stored<UsageLedger>>(&text).ok())
                .map(|stored| stored.value)
                .unwrap_or_default();
            ledger.drop_distant_pauses(now);
            ledger
        })
    }

    fn save_ledger(&mut self) -> Result<(), String> {
        let saved_at = self.now.0;
        let json = serde_json::to_string(&Stored {
            saved_at,
            value: self.ledger(),
        })
        .map_err(|e| e.to_string())?;
        self.host.save_usage(json).map_err(|e| e.to_string())
    }

    /// Counts a metered Open-Meteo call and saves the ledger; an error when the budget is spent
    /// or the ledger could not be saved, and the call must not go out.
    fn spend(&mut self) -> Result<(), SourceError> {
        let now = self.now;
        self.ledger()
            .try_spend(&OPEN_METEO_STORM, POINT_BUDGET, now)
            .map_err(|quota| SourceError::RateLimited {
                retry_after_s: Some(quota.retry_after_s(now)),
            })?;
        self.save_ledger()
            .map_err(|e| SourceError::Transport(format!("the call ledger was not saved: {e}")))
    }

    /// Sends `call`; a response or why there is none.
    async fn send(&mut self, call: &Call) -> Result<Fetched, SourceError> {
        self.requests += 1;
        let response = self
            .host
            .send(host_request(call))
            .await
            .map_err(|e| SourceError::Transport(e.to_string()))?;
        fetched(response).map_err(|e| SourceError::Transport(e.to_string()))
    }

    /// Sends an Open-Meteo request and parses its body, recording what failed. A metered one
    /// goes out only once counted, and what it says of the budget is kept.
    async fn open_meteo<T>(
        &mut self,
        request: &HttpRequest,
        metered: bool,
        max_age: Option<u32>,
        parse: impl FnOnce(&[u8]) -> Result<T, whether_providers::ProviderError>,
    ) -> Result<T, SourceError> {
        let result = self.exchange(request, metered, max_age, parse).await;
        if let Err(e) = &result {
            self.errors.push(e.to_model(OPEN_METEO_STORM, None));
        }
        result
    }

    async fn exchange<T>(
        &mut self,
        request: &HttpRequest,
        metered: bool,
        max_age: Option<u32>,
        parse: impl FnOnce(&[u8]) -> Result<T, whether_providers::ProviderError>,
    ) -> Result<T, SourceError> {
        if metered {
            self.spend()?;
        }
        let call = Call::from(request).with_cache(CacheHint {
            max_age,
            ..CacheHint::default()
        });
        let result = self.send(&call).await;
        if metered && let Ok(response) = &result {
            let now = self.now;
            let ledger = self.ledger();
            if let Some(left) = response.rate_limit_remaining {
                ledger.observe_remaining(&OPEN_METEO_STORM, POINT_BUDGET, left, now);
            }
            if response.status == 429 {
                ledger.rate_limited(&OPEN_METEO_STORM, response.retry_after, now);
            }
            // The call was counted before it went out; this only keeps what the response said.
            let _ = self.save_ledger();
        }
        result.and_then(|response| {
            body(&response).and_then(|b| parse(b).map_err(|e| SourceError::Parse(e.to_string())))
        })
    }
}

/// The body of a 2xx response, or the error the status means.
fn body(response: &Fetched) -> Result<&[u8], SourceError> {
    match response.status {
        200..=299 if response.body.trim_ascii().is_empty() => {
            Err(SourceError::NoData(Some("empty response".into())))
        }
        200..=299 => Ok(&response.body),
        404 | 410 => Err(SourceError::NotFound(None)),
        429 => Err(SourceError::RateLimited {
            retry_after_s: response.retry_after,
        }),
        status => Err(SourceError::Status {
            status,
            message: check_status(&HttpResponse::new(status, &response.body))
                .err()
                .map(|e| e.to_string()),
        }),
    }
}

/// The latest run of every model whose `meta.json` answered.
async fn latest_runs(link: &mut Link<'_>) -> BTreeMap<&'static str, ModelRun> {
    let mut runs = BTreeMap::new();
    for (_, domain) in om::RUN_DOMAINS {
        let request = om::meta_request(domain);
        if let Ok(run) = link
            .open_meteo(&request, false, Some(META_MAX_AGE_S), |b| {
                om::parse_run_meta(domain, b)
            })
            .await
        {
            runs.insert(domain, run);
        }
    }
    runs
}

/// The models' hours at `centre` and their pressure grid around it, each tagged with its run.
/// `None` when neither came.
async fn models_data(
    link: &mut Link<'_>,
    centre: (f64, f64),
    runs: &BTreeMap<&'static str, ModelRun>,
) -> Option<StormOutlook> {
    let loc = Coordinates {
        latitude: centre.0,
        longitude: centre.1,
    };
    let point = link
        .open_meteo(
            &om::point_request(&loc, &DETERMINISTIC_MODELS),
            true,
            None,
            |b| om::parse_points(b, &DETERMINISTIC_MODELS),
        )
        .await;
    // A spent budget or a pause holds for the grid too: it is not asked.
    let grids = match &point {
        Err(SourceError::RateLimited { .. }) => None,
        _ => link
            .open_meteo(
                &om::grid_request(&loc, &DETERMINISTIC_MODELS),
                true,
                None,
                |b| om::parse_grids(b, &loc, &DETERMINISTIC_MODELS),
            )
            .await
            .ok(),
    };
    let point = point.ok();
    link.data_failed |= point.is_none() || grids.is_none();
    if point.is_none() && grids.is_none() {
        return None;
    }
    let run_of = |model: &str| om::run_domain(model).and_then(|d| runs.get(d)).cloned();
    let (location, mut points) = point.map_or((None, Vec::new()), |p| (Some(p.location), p.series));
    for p in &mut points {
        p.run = run_of(&p.model);
    }
    let mut grids = grids.unwrap_or_default();
    for g in &mut grids {
        g.run = run_of(&g.model);
    }
    Some(StormOutlook {
        place: ResolvedLocation {
            name: String::new(),
            latitude: centre.0,
            longitude: centre.1,
            ..location.unwrap_or_default()
        },
        points,
        grids,
        fetched_at: link.now,
        ..StormOutlook::default()
    })
}

/// NHC's active storms near `centre`, through the provider's storm plan (no coordinates leave
/// the device), keeping the tracks of `known` storms for their forecast advisory.
async fn hurricanes(
    link: &mut Link<'_>,
    ctx: &FetchContext,
    centre: (f64, f64),
    known: &[Hurricane],
    distance_km: f64,
) -> Vec<Hurricane> {
    let spot = Spot {
        id: STORMS_LAYER.to_owned(),
        name: String::new(),
        latitude: centre.0,
        longitude: centre.1,
        water_type: WaterType::Offshore,
        water_body: None,
        time_zone: "UTC".to_owned(),
    };
    let previous = StormWatch {
        checked_at: link.now,
        point: None,
        hurricanes: known.to_vec(),
    };
    let options = StormOptions {
        hurricane_distance_km: distance_km,
        allow_metered: false,
    };
    let mut plan = StormWatchPlan::new(&spot, Some(&previous), &options, ctx, None);
    while let Some(request) = plan.next_request() {
        let call = Call::from(&request.http).with_cache(request.cache);
        match link.send(&call).await {
            Ok(response) => plan.on_response(&response.as_response()),
            Err(e) => plan.on_transport_error(&e.to_string()),
        }
    }
    let outcome = plan.finish();
    link.data_failed |= !outcome.errors.is_empty();
    link.errors.extend(outcome.errors);
    outcome.watch.hurricanes
}

/// The Storms layer around the view's centre (`latitude`, `longitude`) at `now`, in the settings'
/// units and `time_zone`'s times, through `host`.
///
/// Asks for nothing while the kept outlook is within one grid cell of the centre and was checked
/// less than [`CHECK_EVERY_S`] ago. Otherwise reads the models' run times and NHC's active
/// storms, and fetches the models' hours and pressure grid (two metered Open-Meteo calls, only
/// with `allow_metered`, counted in the usage ledger) when a model has a newer run or the
/// centre moved more than one grid cell. NHC's tracks are read once per forecast advisory. What
/// fails is listed in the outcome and the layer keeps what it had.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub async fn fetch_storm_layer(
    latitude: f64,
    longitude: f64,
    settings_json: String,
    time_zone: String,
    allow_metered: bool,
    now: i64,
    host: Arc<dyn FetchHost>,
) -> Result<StormLayerOutcome, MapError> {
    let (config, _) = load(&settings_json)?;
    let now = UnixTime(now);
    let ctx = FetchContext::new(now, user_agent())
        .with_coordinate_precision(config.sources.coordinate_precision);
    let wanted = centre(latitude, longitude, &ctx)?;
    let previous = kept(host.as_ref());
    let stored = previous.as_ref().filter(|k| same_area(&k.outlook, wanted));
    if let Some(k) = stored
        && (0..CHECK_EVERY_S).contains(&(now.0 - k.checked_at.0))
    {
        return Ok(StormLayerOutcome {
            layer: layer_view(
                &config,
                &time_zone,
                &k.outlook,
                Some(k.checked_at),
                false,
                now,
            ),
            requests: 0,
            refreshed: false,
            errors: Vec::new(),
        });
    }
    let mut link = Link {
        host: host.as_ref(),
        now,
        ledger: None,
        requests: 0,
        errors: Vec::new(),
        data_failed: false,
    };
    // The area keeps its centre while the view stays within a cell of it.
    let area = stored.map_or(wanted, |k| {
        (k.outlook.place.latitude, k.outlook.place.longitude)
    });
    let mut outlook = stored.map(|k| k.outlook.clone());
    let mut refreshed = false;
    if allow_metered {
        let runs = latest_runs(&mut link).await;
        let current = outlook
            .as_ref()
            .is_some_and(|o| models_current(o, &runs, now));
        if !current && let Some(fresh) = models_data(&mut link, area, &runs).await {
            outlook = Some(fresh);
            refreshed = true;
        }
    }
    let mut outlook = outlook.unwrap_or_else(|| StormOutlook {
        place: ResolvedLocation {
            latitude: area.0,
            longitude: area.1,
            ..ResolvedLocation::default()
        },
        fetched_at: now,
        ..StormOutlook::default()
    });
    // The storms known from any area keep their tracks for their forecast advisory.
    let known = previous.map(|k| k.outlook.hurricanes).unwrap_or_default();
    outlook.hurricanes = hurricanes(
        &mut link,
        &ctx,
        area,
        &known,
        config.storms.hurricane_distance_km,
    )
    .await;
    let layer = layer_view(
        &config,
        &time_zone,
        &outlook,
        Some(now),
        link.data_failed,
        now,
    );
    keep(
        host.as_ref(),
        &MapOutlook {
            checked_at: now,
            outlook,
        },
    );
    Ok(StormLayerOutcome {
        layer,
        requests: link.requests,
        refreshed,
        errors: link.errors.iter().map(SourceIssue::from).collect(),
    })
}

/// The Storms layer of the kept outlook at `now`, without asking anything: for a change of units
/// or time zone, and to draw at once on opening. `None` when no outlook is kept.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn kept_storm_layer(
    settings_json: String,
    time_zone: String,
    now: i64,
    host: Arc<dyn FetchHost>,
) -> Result<Option<StormLayer>, MapError> {
    let (config, _) = load(&settings_json)?;
    Ok(kept(host.as_ref()).map(|k| {
        layer_view(
            &config,
            &time_zone,
            &k.outlook,
            Some(k.checked_at),
            false,
            UnixTime(now),
        )
    }))
}

/// The Storms layer of a storm outlook given as JSON (whether's `StormOutlook`), at `now`: for
/// outlooks kept elsewhere, such as the app's built-in samples.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn storm_layer_of(
    outlook_json: String,
    settings_json: String,
    time_zone: String,
    now: i64,
) -> Result<StormLayer, MapError> {
    let (config, _) = load(&settings_json)?;
    let outlook: StormOutlook =
        serde_json::from_str(&outlook_json).map_err(|e| invalid(format!("storm outlook: {e}")))?;
    Ok(layer_view(
        &config,
        &time_zone,
        &outlook,
        None,
        false,
        UnixTime(now),
    ))
}

/// The layer's words before an outlook is there, in the settings' voice.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn storm_layer_words(settings_json: String) -> Result<StormLayerWords, MapError> {
    let (config, _) = load(&settings_json)?;
    let s = formatter(&config, "UTC").strings;
    Ok(StormLayerWords {
        loading: s.safety(SafetyKey::StormLayerLoading).to_owned(),
        failed: s.safety(SafetyKey::StormLayerFailed).to_owned(),
        safety: s.safety(SafetyKey::StormForecastsChangeText).to_owned(),
        notice: s.safety(SafetyKey::NotForNavigation).to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use serde_json::{Value, json};
    use underbite_providers::session::usage::OPEN_METEO;
    use whether_core::storm::testdata;

    use super::*;
    use crate::fetch::{HostError, HostRequest, HostResponse};
    use crate::settings::default_settings_json;

    macro_rules! raw {
        ($file:literal) => {
            include_bytes!(concat!("../../../tests/fixtures/raw/storms/", $file))
        };
    }

    const METAS: [(&str, &[u8]); 4] = [
        (
            "ecmwf_ifs025",
            raw!("open-meteo-meta-ecmwf_ifs025-2026-11-15.json"),
        ),
        (
            "ncep_gfs025",
            raw!("open-meteo-meta-ncep_gfs025-2026-11-15.json"),
        ),
        (
            "cmc_gem_gdps_15km",
            raw!("open-meteo-meta-cmc_gem_gdps_15km-2026-11-15.json"),
        ),
        ("dwd_icon", raw!("open-meteo-meta-dwd_icon-2026-11-15.json")),
    ];
    const POINT: &[u8] = raw!("whether-open-meteo-storm-point-10001.json");
    const GRID: &[u8] = raw!("whether-open-meteo-storm-grid-10001.json");
    const NO_STORMS: &[u8] = raw!("nhc-current-storms-2026-11-15.json");
    const JOSEPHINE: &[u8] = raw!("whether-nhc-current-storms-josephine.synthetic.json");
    const ADECK: &[u8] = raw!("whether-nhc-aal102026.dat.gz");
    const BDECK: &[u8] = raw!("whether-nhc-bal102026.dat");

    const CURRENT_STORMS: &str = "https://www.nhc.noaa.gov/CurrentStorms.json";
    const ADECK_URL: &str = "https://ftp.nhc.noaa.gov/atcf/aid_public/aal102026.dat.gz";
    const BDECK_URL: &str = "https://ftp.nhc.noaa.gov/atcf/btk/bal102026.dat";
    const FORECAST: &str = "https://api.open-meteo.com/v1/forecast";

    /// After the recorded run times were out (2026-11-15 22:00 UTC).
    const NOW: i64 = 1_794_780_000;
    /// An hour after Josephine's recorded advisory (2026-10-22 21:00 UTC).
    const JOSEPHINE_NOW: i64 = 1_792_702_800 + 3_600;
    const MINUTE: i64 = 60;
    /// The Battery, at the tip of Manhattan.
    const BATTERY: (f64, f64) = (40.7033, -74.0170);
    /// A point on Josephine's official track, at her 36-hour position.
    const ON_HER_TRACK: (f64, f64) = (43.5, -55.9);

    /// Answers from the recordings and keeps what the fetch stores.
    #[derive(Default)]
    struct Net {
        asked: Mutex<Vec<HostRequest>>,
        values: Mutex<BTreeMap<(String, String), String>>,
        /// NHC's active storms; none when `None`, as recorded.
        storms: Option<&'static [u8]>,
        /// `meta.json` bodies in place of the recorded ones, by domain.
        metas: Vec<(&'static str, String)>,
        /// URLs containing one of these answer 503.
        down: Vec<&'static str>,
        /// URLs containing one of these answer 429 with the longest `Retry-After`.
        limited: Vec<&'static str>,
    }

    impl Net {
        fn with_storms(storms: &'static [u8]) -> Arc<Self> {
            Arc::new(Self {
                storms: Some(storms),
                ..Self::default()
            })
        }

        /// A host that keeps what `other` kept.
        fn after(other: &Self) -> Self {
            Self {
                values: Mutex::new(other.values.lock().unwrap().clone()),
                ..Self::default()
            }
        }

        /// The URLs asked, without their queries.
        fn paths(&self) -> Vec<String> {
            self.urls()
                .iter()
                .map(|u| u.split('?').next().unwrap_or_default().to_owned())
                .collect()
        }

        fn urls(&self) -> Vec<String> {
            self.asked
                .lock()
                .unwrap()
                .iter()
                .map(|r| r.url.clone())
                .collect()
        }

        fn forget_asked(&self) {
            self.asked.lock().unwrap().clear();
        }

        fn ledger(&self) -> UsageLedger {
            self.load(store::USAGE.to_owned(), store::LEDGER_KEY.to_owned())
                .and_then(|text| serde_json::from_str::<Stored<UsageLedger>>(&text).ok())
                .map(|s| s.value)
                .unwrap_or_default()
        }

        fn body(&self, url: &str) -> Option<Vec<u8>> {
            let fixed: Option<&[u8]> = match url {
                CURRENT_STORMS => Some(self.storms.unwrap_or(NO_STORMS)),
                ADECK_URL => Some(ADECK),
                BDECK_URL => Some(BDECK),
                _ => None,
            };
            if let Some(body) = fixed {
                return Some(body.to_vec());
            }
            if let Some(rest) = url.strip_prefix("https://api.open-meteo.com/data/") {
                let domain = rest.strip_suffix("/static/meta.json")?;
                if let Some((_, body)) = self.metas.iter().find(|(d, _)| *d == domain) {
                    return Some(body.clone().into_bytes());
                }
                return METAS
                    .iter()
                    .find(|(d, _)| *d == domain)
                    .map(|(_, b)| b.to_vec());
            }
            if url.starts_with("https://api.open-meteo.com/v1/forecast?") {
                let grid = url.contains("&temporal_resolution=hourly_6");
                return Some(if grid { GRID } else { POINT }.to_vec());
            }
            None
        }
    }

    #[async_trait::async_trait]
    impl FetchHost for Net {
        async fn send(&self, request: HostRequest) -> Result<HostResponse, HostError> {
            let url = request.url.clone();
            self.asked.lock().unwrap().push(request);
            if self.limited.iter().any(|d| url.contains(d)) {
                return Ok(HostResponse {
                    status: 429,
                    body: Vec::new(),
                    content_type: None,
                    date: None,
                    expires: None,
                    last_modified: None,
                    etag: None,
                    retry_after: Some("4294967295".into()),
                    rate_limit_remaining: None,
                });
            }
            let reply = |status: u16, body: Vec<u8>| HostResponse {
                status,
                body,
                content_type: Some("application/json".into()),
                date: None,
                expires: None,
                last_modified: None,
                etag: None,
                retry_after: None,
                rate_limit_remaining: None,
            };
            if self.down.iter().any(|d| url.contains(d)) {
                return Ok(reply(503, Vec::new()));
            }
            let body = self.body(&url).ok_or_else(|| HostError::Failed {
                message: "no route".into(),
            })?;
            Ok(reply(200, body))
        }

        fn load(&self, namespace: String, key: String) -> Option<String> {
            self.values.lock().unwrap().get(&(namespace, key)).cloned()
        }

        fn store(&self, namespace: String, key: String, value: String) {
            self.values.lock().unwrap().insert((namespace, key), value);
        }

        fn save_usage(&self, ledger_json: String) -> Result<(), HostError> {
            self.store(
                store::USAGE.to_owned(),
                store::LEDGER_KEY.to_owned(),
                ledger_json,
            );
            Ok(())
        }
    }

    fn settings(edit: impl FnOnce(&mut Value)) -> String {
        let mut v: Value = serde_json::from_str(&default_settings_json()).unwrap();
        v["presentation"]["voice"] = json!("plain");
        edit(&mut v);
        v.to_string()
    }

    fn imperial() -> String {
        settings(|_| {})
    }

    fn metric() -> String {
        settings(|v| {
            v["units"] = json!({ "discharge": "m3s", "height": "m", "speed": "ms",
                "water_temp": "C", "pressure": "hPa", "distance": "km" });
        })
    }

    fn fetch_at(
        net: &Arc<Net>,
        (latitude, longitude): (f64, f64),
        metered: bool,
        now: i64,
    ) -> StormLayerOutcome {
        let host: Arc<dyn FetchHost> = net.clone();
        pollster::block_on(fetch_storm_layer(
            latitude,
            longitude,
            imperial(),
            "America/New_York".into(),
            metered,
            now,
            host,
        ))
        .unwrap()
    }

    #[test]
    fn a_storm_429_pauses_the_pool_for_a_day_at_most() {
        let net = Arc::new(Net {
            limited: vec!["/v1/forecast"],
            ..Net::default()
        });
        fetch_at(&net, BATTERY, true, NOW);
        let until = net.ledger().paused_until(&OPEN_METEO, UnixTime(NOW));
        assert_eq!(until, Some(UnixTime(NOW + 86_400)));
    }

    #[test]
    fn a_far_future_pause_in_the_stored_ledger_is_dropped() {
        let net = Arc::new(Net::default());
        let mut ledger = UsageLedger::default();
        ledger.pause_until(&OPEN_METEO, UnixTime(NOW + 4_000_000_000));
        net.save_usage(
            serde_json::to_string(&Stored {
                saved_at: NOW,
                value: &ledger,
            })
            .unwrap(),
        )
        .unwrap();
        fetch_at(&net, BATTERY, true, NOW);
        assert!(net.paths().iter().any(|p| p == FORECAST));
        assert_eq!(net.ledger().paused_until(&OPEN_METEO, UnixTime(NOW)), None);
    }

    fn meta_paths() -> Vec<String> {
        METAS
            .iter()
            .map(|(d, _)| format!("https://api.open-meteo.com/data/{d}/static/meta.json"))
            .collect()
    }

    fn kept_outlook(net: &Net) -> MapOutlook {
        let text = net
            .load(STORE_NAMESPACE.into(), STORE_KEY.into())
            .expect("an outlook is kept");
        serde_json::from_str(&text).unwrap()
    }

    #[test]
    fn a_first_look_reads_the_runs_the_models_and_nhc_and_counts_two_calls() {
        let net = Arc::new(Net::default());
        let outcome = fetch_at(&net, BATTERY, true, NOW);
        let mut expected = meta_paths();
        expected.extend([FORECAST.into(), FORECAST.into(), CURRENT_STORMS.into()]);
        assert_eq!(net.paths(), expected);
        assert_eq!(outcome.requests, 7);
        assert!(outcome.refreshed);
        assert_eq!(outcome.errors, []);
        // The centre leaves the device snapped to the grid's quarter degree.
        let urls = net.urls();
        assert!(
            urls[4].contains("latitude=40.75&longitude=-74"),
            "{}",
            urls[4]
        );
        assert!(urls[5].contains("&temporal_resolution=hourly_6&"));
        // The two Open-Meteo data calls count in the shared pool; the run times do not.
        assert_eq!(net.ledger().used(&OPEN_METEO, UnixTime(NOW)), 2);

        let layer = &outcome.layer;
        assert_eq!((layer.latitude, layer.longitude), (40.75, -74.0));
        let shorts: Vec<_> = layer.models.iter().map(|m| m.short.as_str()).collect();
        assert_eq!(shorts, ["ECMWF", "GFS", "GEM", "ICON"]);
        assert!(layer.models[0].label.starts_with("ECMWF lows · "));
        assert!(layer.models[0].label.ends_with("Z run"));
        assert_ne!(layer.tracks, []);
        for t in &layer.tracks {
            assert!(t.points.len() >= 3, "{}", t.model);
            assert!(t.points.iter().all(|p| p.label.ends_with(" inHg")));
            assert!(
                t.points
                    .iter()
                    .all(|p| (-180.0..180.0).contains(&p.longitude))
            );
        }
        assert_eq!(layer.hurricanes, []);
        assert_eq!(layer.cone_note, None);
        assert!(layer.legend.iter().all(|l| l.kind != StormLegendKind::Cone));
        assert!(layer.steps[0].label.ends_with(" · now"));
        assert!(layer.steps[1].label.contains(" · +"));
        assert!(layer.steps[0].time <= NOW && layer.steps[1].time > NOW);
        assert!(layer.summary.is_some());
        assert_eq!(layer.status, None);
        assert_eq!(layer.notice, "Not for navigation.");
        assert_eq!(layer.credits.len(), 1);
        assert!(layer.credits[0].contains("Open-Meteo.com"));
        assert!(layer.checked.as_deref().unwrap().contains("modelled"));
        assert_eq!(kept_outlook(&net).checked_at, UnixTime(NOW));
    }

    #[test]
    fn within_a_cell_and_a_quarter_hour_nothing_is_asked() {
        let net = Arc::new(Net::default());
        let first = fetch_at(&net, BATTERY, true, NOW);
        net.forget_asked();
        // Two degrees north east: within one cell of the kept outlook's centre.
        let again = fetch_at(&net, (42.5, -72.2), true, NOW + 10 * MINUTE);
        assert_eq!(net.paths(), Vec::<String>::new());
        assert_eq!(again.requests, 0);
        assert!(!again.refreshed);
        assert_eq!(again.layer.tracks, first.layer.tracks);
        assert_eq!(net.ledger().used(&OPEN_METEO, UnixTime(NOW)), 2);
    }

    #[test]
    fn later_the_run_times_and_nhc_are_read_and_current_models_are_kept() {
        let net = Arc::new(Net::default());
        fetch_at(&net, BATTERY, true, NOW);
        net.forget_asked();
        let again = fetch_at(&net, BATTERY, true, NOW + 20 * MINUTE);
        let mut expected = meta_paths();
        expected.push(CURRENT_STORMS.into());
        assert_eq!(net.paths(), expected);
        assert!(!again.refreshed);
        assert_eq!(net.ledger().used(&OPEN_METEO, UnixTime(NOW)), 2);
        assert_eq!(kept_outlook(&net).checked_at, UnixTime(NOW + 20 * MINUTE));
    }

    #[test]
    fn a_newer_run_fetches_the_models_again() {
        let net = Arc::new(Net::default());
        fetch_at(&net, BATTERY, true, NOW);
        let mut meta: Value = serde_json::from_slice(METAS[0].1).unwrap();
        let six_hours = 6 * 3_600;
        for field in ["last_run_initialisation_time", "last_run_availability_time"] {
            meta[field] = json!(meta[field].as_i64().unwrap() + six_hours);
        }
        let later = Arc::new(Net {
            metas: vec![("ecmwf_ifs025", meta.to_string())],
            ..Net::after(&net)
        });
        // Half an hour on, past the quarter hour: the new run is read, and its data fetched.
        let again = fetch_at(&later, BATTERY, true, NOW + 30 * MINUTE);
        let mut expected = meta_paths();
        expected.extend([FORECAST.into(), FORECAST.into(), CURRENT_STORMS.into()]);
        assert_eq!(later.paths(), expected);
        assert!(again.refreshed);
        assert_eq!(later.ledger().used(&OPEN_METEO, UnixTime(NOW)), 4);
    }

    #[test]
    fn moving_more_than_a_grid_cell_fetches_for_the_new_centre() {
        let net = Arc::new(Net::default());
        fetch_at(&net, BATTERY, true, NOW);
        net.forget_asked();
        let moved = fetch_at(&net, (43.4, -74.0), true, NOW + 5 * MINUTE);
        assert!(moved.refreshed);
        let urls = net.urls();
        assert!(
            urls[4].contains("latitude=43.5&longitude=-74"),
            "{}",
            urls[4]
        );
        assert_eq!((moved.layer.latitude, moved.layer.longitude), (43.5, -74.0));
    }

    #[test]
    fn without_metered_calls_only_nhc_is_asked() {
        let net = Arc::new(Net::default());
        let outcome = fetch_at(&net, BATTERY, false, NOW);
        assert_eq!(net.paths(), [CURRENT_STORMS]);
        assert!(!outcome.refreshed);
        let layer = &outcome.layer;
        assert!(layer.models.is_empty() && layer.tracks.is_empty() && layer.steps.is_empty());
        assert_eq!(
            (layer.summary.as_ref(), layer.status.as_ref()),
            (None, None)
        );
        assert_eq!(layer.credits, Vec::<String>::new());
        assert_eq!(net.ledger().used(&OPEN_METEO, UnixTime(NOW)), 0);
    }

    #[test]
    fn a_spent_budget_sends_no_data_call_and_says_so() {
        let net = Arc::new(Net::default());
        let mut ledger = UsageLedger::default();
        for _ in 0..POINT_BUDGET.limit {
            ledger
                .try_spend(&OPEN_METEO_STORM, POINT_BUDGET, UnixTime(NOW))
                .unwrap();
        }
        let saved = serde_json::to_string(&Stored {
            saved_at: NOW,
            value: ledger,
        })
        .unwrap();
        net.save_usage(saved).unwrap();
        let outcome = fetch_at(&net, BATTERY, true, NOW);
        let mut expected = meta_paths();
        expected.push(CURRENT_STORMS.into());
        assert_eq!(net.paths(), expected);
        assert_eq!(outcome.errors.len(), 1);
        assert_eq!(outcome.errors[0].kind, "rate_limited");
        assert_eq!(outcome.errors[0].source, "open-meteo-storm");
        assert_eq!(
            outcome.layer.status.as_deref(),
            Some("The storm outlook could not be loaded. Try again later.")
        );
    }

    #[test]
    fn a_failed_grid_keeps_the_hours_and_says_part_is_missing() {
        let net = Arc::new(Net {
            down: vec!["temporal_resolution=hourly_6"],
            ..Net::default()
        });
        let outcome = fetch_at(&net, BATTERY, true, NOW);
        assert!(outcome.refreshed);
        assert_eq!(outcome.errors.len(), 1);
        let layer = &outcome.layer;
        assert!(layer.tracks.is_empty() && !layer.models.is_empty());
        assert_eq!(
            layer.status.as_deref(),
            Some("Some storm data could not be loaded; what is shown may be incomplete.")
        );
        // Without the grid the outlook is not current: the next check asks for it again.
        let fixed = Arc::new(Net::after(&net));
        let again = fetch_at(&fixed, BATTERY, true, NOW + 20 * MINUTE);
        assert!(again.refreshed && !again.layer.tracks.is_empty());
    }

    #[test]
    fn josephine_near_the_centre_brings_her_tracks_and_nhc_s_cone() {
        let net = Net::with_storms(JOSEPHINE);
        let outcome = fetch_at(&net, ON_HER_TRACK, false, JOSEPHINE_NOW);
        assert_eq!(net.paths(), [CURRENT_STORMS, ADECK_URL, BDECK_URL]);
        // NHC's files carry no coordinates.
        assert!(
            net.urls()
                .iter()
                .all(|u| !u.contains("43.5") && !u.contains("55."))
        );
        assert_eq!(outcome.errors, []);
        let layer = &outcome.layer;
        assert_eq!(layer.hurricanes.len(), 1);
        let h = &layer.hurricanes[0];
        assert_eq!((h.id.as_str(), h.name.as_str()), ("al102026", "Josephine"));
        assert_eq!(h.title, "Subtropical Storm Josephine");
        assert_eq!(h.severity, "strong");
        assert!(h.strength.starts_with("Winds "), "{}", h.strength);
        assert!(h.advisory.starts_with("Advisory "), "{}", h.advisory);
        assert!(
            h.advisory_url
                .as_deref()
                .unwrap()
                .starts_with("https://www.nhc.noaa.gov/")
        );
        assert!(h.official.len() > 2 && h.past.len() > 2 && h.cone.len() > 10);
        assert!(
            h.official
                .iter()
                .all(|p| !p.label.is_empty() && p.spoken.contains("Josephine"))
        );
        let roles: Vec<_> = h.aids.iter().map(|a| a.role.as_str()).collect();
        let mut unique = roles.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(
            roles.len(),
            unique.len(),
            "one guidance track per model family"
        );
        assert!(roles.contains(&"consensus"), "{roles:?}");
        let cone = layer.cone_note.as_deref().unwrap();
        assert!(cone.starts_with("NHC's forecast cone, drawn from NHC's 2026 cone sizes"));
        assert!(cone.contains("not where its impacts reach"));
        assert_eq!(layer.cone_link, "https://www.nhc.noaa.gov/aboutcone.shtml");
        let kinds: Vec<_> = layer.legend.iter().map(|l| l.kind).collect();
        assert_eq!(kinds.first(), Some(&StormLegendKind::OfficialTrack));
        assert_eq!(kinds.last(), Some(&StormLegendKind::Cone));
        assert!(kinds.contains(&StormLegendKind::ObservedTrack));
        assert_eq!(layer.credits.len(), 1);
        assert!(layer.credits[0].contains("National Hurricane Center"));
        assert_eq!(layer.advisory_link_title, "NHC advisory");
    }

    #[test]
    fn a_known_advisory_reads_no_decks_again() {
        let net = Net::with_storms(JOSEPHINE);
        let first = fetch_at(&net, ON_HER_TRACK, false, JOSEPHINE_NOW);
        net.forget_asked();
        let again = fetch_at(&net, ON_HER_TRACK, false, JOSEPHINE_NOW + 20 * MINUTE);
        assert_eq!(net.paths(), [CURRENT_STORMS]);
        assert_eq!(again.layer.hurricanes, first.layer.hurricanes);
    }

    #[test]
    fn hurricanes_are_absent_when_none_are_active() {
        let net = Net::with_storms(JOSEPHINE);
        fetch_at(&net, ON_HER_TRACK, false, JOSEPHINE_NOW);
        let quiet = Arc::new(Net::after(&net));
        let after = fetch_at(&quiet, ON_HER_TRACK, false, JOSEPHINE_NOW + 20 * MINUTE);
        assert_eq!(quiet.paths(), [CURRENT_STORMS]);
        assert_eq!(after.layer.hurricanes, []);
        assert_eq!(after.layer.cone_note, None);
        assert_eq!(after.layer.legend, []);
    }

    #[test]
    fn units_follow_the_settings() {
        let net = Net::with_storms(JOSEPHINE);
        fetch_at(&net, ON_HER_TRACK, false, JOSEPHINE_NOW);
        let host: Arc<dyn FetchHost> = net.clone();
        let view = |json: String| {
            kept_storm_layer(json, "UTC".into(), JOSEPHINE_NOW, host.clone())
                .unwrap()
                .unwrap()
        };
        let (us, si) = (view(imperial()), view(metric()));
        let (us, si) = (&us.hurricanes[0], &si.hurricanes[0]);
        assert!(us.wind.ends_with(" kn"), "{}", us.wind);
        assert!(si.wind.ends_with(" m/s"), "{}", si.wind);
        assert!(us.pressure.ends_with(" inHg"), "{}", us.pressure);
        assert!(si.pressure.ends_with(" hPa"), "{}", si.pressure);
        assert!(si.strength.contains(" hPa"));
    }

    #[test]
    fn the_noreaster_sample_marks_the_storm_s_low() {
        let outlook = testdata::noreaster();
        let json = serde_json::to_string(&outlook).unwrap();
        let layer = storm_layer_of(
            json,
            imperial(),
            "America/New_York".into(),
            testdata::STORM_NOW.0,
        )
        .unwrap();
        assert!(
            layer
                .summary
                .as_deref()
                .unwrap()
                .starts_with("A storm passes near the map's centre · "),
            "{:?}",
            layer.summary
        );
        let storms = layer.tracks.iter().filter(|t| t.is_storm).count();
        assert_eq!(storms, 4, "each model's low that brings the storm");
        assert!(
            layer
                .legend
                .iter()
                .any(|l| l.kind == StormLegendKind::StormBand)
        );
        assert_eq!(layer.checked, None);
        assert!(layer.steps[0].label.ends_with("now"));
        let lows: Vec<_> = layer.tracks[0].points.iter().map(|p| &p.label).collect();
        assert!(lows.iter().all(|l| l.starts_with("ECMWF ")), "{lows:?}");
    }

    #[test]
    fn centres_snap_round_and_wrap() {
        let ctx =
            |decimals| FetchContext::new(UnixTime(NOW), "test").with_coordinate_precision(decimals);
        assert_eq!(centre(40.7033, -74.017, &ctx(2)).unwrap(), (40.75, -74.0));
        assert_eq!(centre(40.7033, -74.017, &ctx(0)).unwrap(), (41.0, -74.0));
        assert_eq!(centre(89.0, 10.0, &ctx(2)).unwrap(), (80.0, 10.0));
        assert_eq!(centre(10.0, 179.95, &ctx(2)).unwrap(), (10.0, -180.0));
        assert!(centre(f64::NAN, 0.0, &ctx(2)).is_err());
        assert!(centre(91.0, 0.0, &ctx(2)).is_err());
        // Across the antimeridian, the short way round.
        let at = |latitude, longitude| StormOutlook {
            place: ResolvedLocation {
                latitude,
                longitude,
                ..ResolvedLocation::default()
            },
            ..StormOutlook::default()
        };
        assert!(same_area(&at(10.0, 179.0), (10.0, -179.0)));
        assert!(same_area(&at(10.0, 179.0), (12.5, 178.0)));
        assert!(!same_area(&at(10.0, 179.0), (10.0, -177.0)));
        assert!(!same_area(&at(10.0, 0.0), (12.75, 0.0)));
    }

    #[test]
    fn bad_input_is_an_error_not_a_panic() {
        let net = Arc::new(Net::default());
        let host: Arc<dyn FetchHost> = net.clone();
        let fetch = |latitude: f64, settings: String| {
            pollster::block_on(fetch_storm_layer(
                latitude,
                0.0,
                settings,
                "UTC".into(),
                true,
                NOW,
                host.clone(),
            ))
        };
        assert!(matches!(
            fetch(f64::NAN, imperial()),
            Err(MapError::Invalid { .. })
        ));
        assert!(matches!(
            fetch(0.0, "{".into()),
            Err(MapError::Invalid { .. })
        ));
        assert_eq!(net.paths(), Vec::<String>::new());
        assert!(storm_layer_of("[]".into(), imperial(), "UTC".into(), NOW).is_err());
        // A kept outlook that does not read is no outlook.
        net.store(STORE_NAMESPACE.into(), STORE_KEY.into(), "{".into());
        assert_eq!(
            kept_storm_layer(imperial(), "UTC".into(), NOW, host.clone()).unwrap(),
            None
        );
        let words = storm_layer_words(imperial()).unwrap();
        assert_eq!(words.notice, "Not for navigation.");
        assert!(words.safety.contains("National Hurricane Center"));
    }

    /// The outlooks the app's debug builds draw for screenshots: whether's nor'easter sample
    /// (without the ensembles the map does not draw) and Josephine as the recorded NHC files give
    /// her. `UNDERBITE_WRITE_SAMPLES=1 cargo test -p underbite-ffi storms` rewrites them.
    #[test]
    fn storm_samples_are_current() {
        let noreaster = StormOutlook {
            ensembles: Vec::new(),
            ..testdata::noreaster()
        };
        let net = Net::with_storms(JOSEPHINE);
        fetch_at(&net, ON_HER_TRACK, false, JOSEPHINE_NOW);
        let josephine = kept_outlook(&net).outlook;
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../ios/Underbite/Underbite/Map/Storms");
        for (name, outlook) in [
            ("storms-sample-noreaster.json", noreaster),
            ("storms-sample-josephine.json", josephine),
        ] {
            let json = serde_json::to_string(&outlook).unwrap() + "\n";
            let path = dir.join(name);
            if std::env::var_os("UNDERBITE_WRITE_SAMPLES").is_some() {
                std::fs::create_dir_all(&dir).unwrap();
                std::fs::write(&path, &json).unwrap();
            }
            let kept = std::fs::read_to_string(&path).unwrap_or_default();
            assert!(
                kept == json,
                "{name}: run `UNDERBITE_WRITE_SAMPLES=1 cargo test -p underbite-ffi storms`"
            );
        }
    }
}
