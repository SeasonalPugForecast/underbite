//! `underbite trip`: each day of a trip at each place, with what every value is based on.
//!
//! Each place is fetched as `underbite spot` fetches it; then the tide station's harmonic
//! constants make the tides certain for every day, and past years' values (USGS normals for
//! river flow and water temperature, Open-Meteo Marine sea surface temperature from 2023 on)
//! stand in where no forecast reaches. A request that fails only leaves its cells
//! "unavailable". With `--offline` nothing but long-lived station metadata is asked for (station
//! lists, constants, datums, offsets, station records, answered by the cache or `--replay`), so
//! only tides, the sun, the moon and the solunar periods are known.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, anyhow, bail};
use serde::Deserialize;
use serde_json::Value;
use underbite_config::export::parse_toml;
use underbite_config::schema::{Config, Severity, has_errors};
use underbite_config::view_input::{river_ranges, view_input};
use underbite_core::datum::{DatumTable, VerticalDatum};
use underbite_core::model::{Measurement, Requirements, Spot, SpotSnapshot, WaterType};
use underbite_core::offline::cache::MetadataKind;
use underbite_core::provenance::{
    Provenance, Quality, Relation, SourceId, StationKind, StationRef, ValueKind,
};
use underbite_core::series::Series;
use underbite_core::time::{self, LocalDate, UnixTime, UtcOffset, day_number, parse_date};
use underbite_core::trip::grid::{self, GridConfig, PlaceInputs, PlaceKey, TideConstants};
use underbite_core::trip::history::{
    DayThresholds, GriddedInput, HistoryInputs, HistoryKind, SeriesInput, TripHistory, daily_means,
};
use underbite_core::trip::{Fuzz, MAX_TRIP_DAYS, Method, Trip, TripError, TripPlace};
use underbite_core::view::trip::{TripAppState, TripView, build_trip_view};
use underbite_fetch::{Call, FetchHost, FetchOptions, Fetched, TransportError, fetch_spot};
use underbite_providers::coops::mdapi;
use underbite_providers::open_meteo_marine::{HistoryKind as MarineHistoryKind, OpenMeteoMarine};
use underbite_providers::request::{CacheHint, Coordinates, Place, Request, Target, Window};
use underbite_providers::source::{FetchContext, SourceError};
use underbite_providers::usgs::Usgs;

use crate::cli::Global;
use crate::commands::spot::point_id;
use crate::runtime::env::Env;
use crate::runtime::host::CliHost;
use crate::runtime::transport::{Replay, Transport};

/// Arguments of `underbite trip`.
#[derive(Debug, clap::Args)]
pub(crate) struct Args {
    /// A place, as `lat,lon` in decimal degrees; repeat it for each place.
    #[arg(
        long = "place",
        value_name = "LAT,LON",
        required = true,
        allow_hyphen_values = true
    )]
    pub(crate) places: Vec<String>,
    /// The first day, as `YYYY-MM-DD`.
    #[arg(long, value_name = "DATE")]
    pub(crate) from: String,
    /// The last day, as `YYYY-MM-DD` (at most 31 days in all).
    #[arg(long, value_name = "DATE")]
    pub(crate) to: String,
    /// The trip's name.
    #[arg(long)]
    pub(crate) name: Option<String>,
    /// How you fish: `boat`, `kayak`, `shore`, `wade` or `ice`.
    #[arg(long, value_name = "METHOD", default_value = "shore")]
    pub(crate) method: String,
    /// The kind of water at every place, such as `tidal_river_estuary` (default: guessed from
    /// what the sources report).
    #[arg(long, value_name = "TYPE")]
    pub(crate) water_type: Option<String>,
    /// The places' IANA time zone, such as `America/New_York` (default: the forecast's, or the
    /// tide station's with `--offline`).
    #[arg(long, value_name = "ZONE")]
    pub(crate) tz: Option<String>,
    /// The places' ISO 3166-2 region, such as `US-NY`, for the licence and regulations links.
    #[arg(long, value_name = "CODE")]
    pub(crate) region: Option<String>,
    /// Read this configuration file instead of the defaults.
    #[arg(long, value_name = "FILE")]
    pub(crate) config: Option<PathBuf>,
    /// Ask for nothing but station metadata: tides from harmonic constants, the sun and the
    /// moon; everything else is unavailable.
    #[arg(long)]
    pub(crate) offline: bool,
    /// Print the view as JSON.
    #[arg(long)]
    pub(crate) json: bool,
}

/// The first year Open-Meteo's best-match sea surface temperature covers whole.
const SST_FIRST_YEAR: i32 = 2023;
/// The fewest years a date needs for a typical sea surface temperature.
const SST_MIN_YEARS: u32 = 2;
/// The fewest years the daily-record fallback of the normals needs.
const NORMALS_MIN_YEARS: u32 = 10;
/// How long a CO-OPS station record stays valid, seconds.
const STATION_AGE_S: u32 = 30 * 86_400;

/// Runs `underbite trip`.
pub(crate) fn run(args: &Args, global: &Global) -> anyhow::Result<ExitCode> {
    let env = environment(global)?;
    let view = plan(args, &env)?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&view)?);
    } else {
        print!("{}", render(&view)?);
    }
    Ok(ExitCode::SUCCESS)
}

/// The trip's view: every place fetched (or, offline, its tide station found), its tides
/// computed and its history asked for, then the grid and the view.
fn plan(args: &Args, env: &Env) -> anyhow::Result<TripView> {
    let config = load_config(args.config.as_deref())?;
    let first = parse_date(&args.from).map_err(|e| anyhow!("--from {:?}: {e}", args.from))?;
    let last = parse_date(&args.to).map_err(|e| anyhow!("--to {:?}: {e}", args.to))?;
    let span = day_number(last) - day_number(first) + 1;
    if span < 1 {
        bail!("{}", TripError::DatesBackwards);
    }
    if span > MAX_TRIP_DAYS {
        bail!("{}", TripError::TooLong);
    }
    let method = parse_enum::<Method>("method", &args.method)?;
    let water_type = args
        .water_type
        .as_deref()
        .map(|t| parse_enum::<WaterType>("water type", t))
        .transpose()?;
    let now = env.now;
    let host = env.host();
    let options = FetchOptions {
        infer_water_type: water_type.is_none(),
        ..env.options.clone()
    };
    let ctx = FetchContext::new(now, options.user_agent.clone())
        .with_coordinate_precision(config.sources.coordinate_precision)
        .with_secrets(options.secrets.clone());

    let mut found = Vec::new();
    for text in &args.places {
        let (latitude, longitude) = parse_location(text)?;
        let spot = Spot {
            id: point_id(latitude, longitude),
            name: text.trim().to_owned(),
            latitude,
            longitude,
            water_type: water_type.unwrap_or(WaterType::RiverStream),
            water_body: None,
            time_zone: args.tz.clone().unwrap_or_default(),
        };
        let place = if args.offline {
            offline_place(&host, &spot, &config, &options, args, now)
        } else {
            online_place(&host, spot, &config, &options, args, now)
        };
        found.push(place);
    }

    let trip = Trip {
        id: "cli".into(),
        name: args.name.clone().unwrap_or_else(|| "Trip".into()),
        places: found.iter().map(|f| f.place.clone()).collect(),
        first,
        last,
        species: Vec::new(),
        method,
        limits: None,
        notes: String::new(),
        checklist: Vec::new(),
        modified_at: now,
    };
    trip.validate().map_err(|e| anyhow!("{e}"))?;

    let mut inputs = BTreeMap::new();
    for f in found {
        let history = match &f.snapshot {
            Some(s) if !args.offline => history(&host, &trip, &f.place, s, &ctx),
            _ => None,
        };
        let river = f
            .snapshot
            .as_ref()
            .and_then(|s| river_ranges(&config.rivers, s))
            .unwrap_or_default();
        inputs.insert(
            PlaceKey::of(&f.place),
            PlaceInputs {
                snapshot: f.snapshot,
                tide_constants: f.tides,
                history,
                river,
                species_water_temp_c: None,
            },
        );
    }
    let fallback = trip
        .places
        .first()
        .and_then(|p| inputs.get(&PlaceKey::of(p)))
        .and_then(|i| i.snapshot.clone())
        .unwrap_or_else(|| SpotSnapshot::new(fallback_spot(&trip), now));
    let input = view_input(&config, fallback, now);
    let cfg = GridConfig {
        windows: input.windows,
        limits: input.trip_limits.clone(),
    };
    let grid = grid::build(&trip, &inputs, &cfg, now);
    Ok(build_trip_view(
        &trip,
        &grid,
        &inputs,
        &TripAppState::default(),
        &input,
    ))
}

/// A place of the trip with what is known about it.
struct Found {
    place: TripPlace,
    snapshot: Option<SpotSnapshot>,
    tides: Option<TideConstants>,
}

fn trip_place(spot: &Spot, zone: String, region: Option<&str>) -> TripPlace {
    TripPlace {
        spot_id: spot.id.clone(),
        name: spot.name.clone(),
        area: None,
        latitude: Some(spot.latitude),
        longitude: Some(spot.longitude),
        time_zone: zone,
        water_type: spot.water_type,
        region: region.map(str::to_owned),
        fuzz: Fuzz::Exact,
    }
}

/// A place fetched as `underbite spot` fetches it, with its station's constants.
fn online_place(
    host: &CliHost<'_>,
    spot: Spot,
    config: &Config,
    options: &FetchOptions,
    args: &Args,
    now: UnixTime,
) -> Found {
    let fetched = pollster::block_on(fetch_spot(host, &spot, config, now, options));
    let snapshot = fetched.snapshot;
    for e in &snapshot.errors {
        eprintln!(
            "underbite: {}: {}: {}",
            spot.name,
            e.source.as_str(),
            e.message
        );
    }
    let zone = [snapshot.spot.time_zone.as_str(), spot.time_zone.as_str()]
        .into_iter()
        .find(|z| !z.is_empty())
        .map_or_else(|| utc_note(&spot), str::to_owned);
    let fetched_spot = Spot {
        water_type: snapshot.spot.water_type,
        ..spot
    };
    let tides = tide_station(&snapshot).and_then(|(station, provenance)| {
        constants(host, station, provenance, now)
            .map_err(|e| eprintln!("underbite: {}: tide constants: {e:#}", fetched_spot.name))
            .ok()
    });
    Found {
        place: trip_place(&fetched_spot, zone, args.region.as_deref()),
        snapshot: Some(snapshot),
        tides,
    }
}

/// A place without a fetch: the nearest tide station from the station lists alone, its
/// constants and its time zone, all long-lived metadata.
fn offline_place(
    host: &CliHost<'_>,
    spot: &Spot,
    config: &Config,
    options: &FetchOptions,
    args: &Args,
    now: UnixTime,
) -> Found {
    let host = MetadataOnly(*host);
    let requirements = Requirements {
        measurements: BTreeSet::from([Measurement::Tide]),
        ..Requirements::default()
    };
    let options = FetchOptions {
        allow_metered: false,
        infer_water_type: false,
        ..options.clone()
    };
    let discovery = pollster::block_on(underbite_fetch::stations_near(
        &host,
        spot,
        config,
        &requirements,
        now,
        &options,
    ));
    let ranked = discovery
        .ranking
        .measurements
        .get(&Measurement::Tide)
        .and_then(|m| {
            m.stations.iter().find(|r| {
                r.station.source == SourceId::COOPS
                    && matches!(
                        r.station.kind,
                        StationKind::TideHarmonic | StationKind::TideSubordinate
                    )
            })
        });
    let zone = match (&args.tz, ranked) {
        (Some(tz), _) => tz.clone(),
        (None, Some(r)) => station_zone(&host, &r.station.id).unwrap_or_else(|| utc_note(spot)),
        (None, None) => utc_note(spot),
    };
    let tides = ranked.and_then(|r| {
        let provenance = Provenance {
            source: SourceId::COOPS,
            station: Some(r.station.clone()),
            distance_m: Some(r.distance_m),
            relation: r.relation,
            observed_at: now,
            kind: ValueKind::Predicted,
            quality: Quality::Approved,
        };
        constants(&host, &r.station, &provenance, now)
            .map_err(|e| eprintln!("underbite: {}: tide constants: {e:#}", spot.name))
            .ok()
    });
    if ranked.is_none() {
        eprintln!(
            "underbite: {}: no tide station in the saved station lists",
            spot.name
        );
    }
    if args.water_type.is_none() {
        eprintln!(
            "underbite: {}: water type {} (offline, nothing is inferred; --water-type sets \
             another)",
            spot.name,
            serde_json::to_value(spot.water_type)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_default()
        );
    }
    Found {
        place: trip_place(spot, zone, args.region.as_deref()),
        snapshot: None,
        tides,
    }
}

fn utc_note(spot: &Spot) -> String {
    eprintln!(
        "underbite: {}: the time zone is unknown, so the days are UTC days (--tz sets one)",
        spot.name
    );
    "UTC".into()
}

fn fallback_spot(trip: &Trip) -> Spot {
    let place = trip.places.first();
    Spot {
        id: place.map(|p| p.spot_id.clone()).unwrap_or_default(),
        name: place.map(|p| p.name.clone()).unwrap_or_default(),
        latitude: place.and_then(|p| p.latitude).unwrap_or_default(),
        longitude: place.and_then(|p| p.longitude).unwrap_or_default(),
        water_type: place.map_or(WaterType::RiverStream, |p| p.water_type),
        water_body: None,
        time_zone: place.map(|p| p.time_zone.clone()).unwrap_or_default(),
    }
}

// ---------------------------------------------------------------------------------------------
// The host: recorded scenarios, and station metadata only
// ---------------------------------------------------------------------------------------------

/// The command's environment. A `--replay` directory without routes of its own answers from
/// every scenario under it (`battery/`, `croton/`, …), the first scenario in name order with a
/// matching route winning, so one trip can cross several recorded places.
fn environment(global: &Global) -> anyhow::Result<Env> {
    let Some(dir) = global
        .replay
        .as_deref()
        .filter(|d| !d.join("routes.toml").exists())
    else {
        return Env::from_global(global);
    };
    let mut scenarios: Vec<PathBuf> = std::fs::read_dir(dir)
        .with_context(|| format!("--replay: cannot read {}", dir.display()))?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.join("routes.toml").is_file())
        .collect();
    if scenarios.is_empty() {
        return Env::from_global(global);
    }
    scenarios.sort();
    let replays = scenarios
        .iter()
        .map(|path| Replay::load(path).context("--replay"))
        .collect::<anyhow::Result<Vec<_>>>()?;
    let mut env = Env::from_global(&Global {
        verbose: global.verbose,
        replay: None,
        now: global.now.clone(),
    })?;
    env.transport = Box::new(Scenarios(replays));
    Ok(env)
}

/// Several recorded scenarios as one transport.
struct Scenarios(Vec<Replay>);

impl Transport for Scenarios {
    fn send(&self, call: &Call) -> Result<Fetched, TransportError> {
        for replay in &self.0 {
            match replay.send(call) {
                Err(TransportError::NoRoute(_)) => {}
                answered => return answered,
            }
        }
        Err(TransportError::NoRoute(call.redacted_url()))
    }
}

/// The host with every request refused except long-lived station metadata.
struct MetadataOnly<'a>(CliHost<'a>);

/// Whether `url` asks for long-lived station metadata: what the offline cache keeps, and a
/// CO-OPS station's own record (its time zone).
fn is_metadata(url: &str) -> bool {
    if MetadataKind::of_url(url).is_some() {
        return true;
    }
    let path = url.split(['?', '#']).next().unwrap_or(url);
    path.strip_prefix(&format!("{}/stations/", mdapi::BASE))
        .and_then(|rest| rest.strip_suffix(".json"))
        .is_some_and(|id| !id.is_empty() && !id.contains('/'))
}

impl FetchHost for MetadataOnly<'_> {
    async fn send(&self, call: &Call) -> Result<Fetched, TransportError> {
        if is_metadata(&call.url) {
            self.0.send(call).await
        } else {
            Err(TransportError::Failed(
                "offline: only station metadata is asked for".into(),
            ))
        }
    }

    fn cached(&self, call: &Call) -> impl Future<Output = Option<Fetched>> + Send {
        self.0.cached(call)
    }

    fn load(&self, namespace: &str, key: &str) -> impl Future<Output = Option<String>> + Send {
        self.0.load(namespace, key)
    }

    fn store(&self, namespace: &str, key: &str, value: String) -> impl Future<Output = ()> + Send {
        self.0.store(namespace, key, value)
    }

    fn save_usage(&self, ledger: String) -> impl Future<Output = Result<(), String>> + Send {
        self.0.save_usage(ledger)
    }
}

/// One request through `host`, as bytes; a status other than 2xx is an error.
fn get(host: &impl FetchHost, request: &Request) -> anyhow::Result<Fetched> {
    let call = Call::from(request);
    let fetched = pollster::block_on(host.send(&call))
        .with_context(|| format!("request to {}", call.redacted_url()))?;
    if !(200..300).contains(&fetched.status) {
        bail!("HTTP {} from {}", fetched.status, call.redacted_url());
    }
    Ok(fetched)
}

// ---------------------------------------------------------------------------------------------
// Tides from the station's constants
// ---------------------------------------------------------------------------------------------

/// The CO-OPS tide station of a snapshot's tide.
fn tide_station(snapshot: &SpotSnapshot) -> Option<(&StationRef, &Provenance)> {
    let tide = snapshot.marine.tide.as_ref()?;
    let station = tide
        .provenance
        .station
        .as_ref()
        .filter(|s| s.source == SourceId::COOPS)?;
    matches!(
        station.kind,
        StationKind::TideHarmonic | StationKind::TideSubordinate
    )
    .then_some((station, &tide.provenance))
}

fn station_target(station: StationRef) -> Target {
    Target {
        place: Place::Station {
            station,
            distance_m: None,
            relation: Relation::Nearby,
        },
        measurements: BTreeSet::from([Measurement::Tide]),
        window: Window {
            from: UnixTime(0),
            to: UnixTime(0),
        },
        datum: None,
    }
}

/// The station's harmonic constants, heights above MLLW; a subordinate station's are its
/// reference's with its own offsets.
fn constants(
    host: &impl FetchHost,
    station: &StationRef,
    provenance: &Provenance,
    now: UnixTime,
) -> anyhow::Result<TideConstants> {
    let harmonics = |id: &str| -> anyhow::Result<_> {
        let target = station_target(StationRef {
            id: id.to_owned(),
            name: id.to_owned(),
            kind: StationKind::TideHarmonic,
            bin: None,
            reference_id: None,
            ..station.clone()
        });
        let datums = get(host, &mdapi::datums_request(target.clone())?)?;
        let table = mdapi::parse_datums(id, &datums.body)?;
        let none = DatumTable::new(id, BTreeMap::new());
        let harcon = get(host, &mdapi::tide_harcon_request(target, &none)?)?;
        Ok(mdapi::parse_tide_harmonics(
            id,
            &harcon.body,
            &table,
            VerticalDatum::Mllw,
        )?)
    };
    let base = Provenance {
        source: SourceId::COOPS_HARMONICS,
        station: Some(station.clone()),
        distance_m: provenance.distance_m,
        relation: provenance.relation,
        observed_at: now,
        kind: ValueKind::Computed,
        quality: Quality::Approved,
    };
    if station.kind == StationKind::TideSubordinate {
        let body = get(
            host,
            &mdapi::tide_offsets_request(station_target(station.clone()))?,
        )?;
        let offsets =
            mdapi::parse_tide_offsets(&body.body)?.context("a reference station has no offsets")?;
        let reference = harmonics(&offsets.reference_id)?;
        return Ok(TideConstants::Subordinate {
            reference,
            offsets,
            base,
        });
    }
    Ok(TideConstants::Harmonic {
        harmonics: harmonics(&station.id)?,
        base,
    })
}

#[derive(Deserialize)]
struct Details {
    stations: Vec<Detail>,
}

#[derive(Deserialize)]
struct Detail {
    timezone: Option<String>,
}

/// The IANA zone of a CO-OPS station, from its record's standard-time abbreviation.
fn station_zone(host: &impl FetchHost, id: &str) -> Option<String> {
    let call = Call::get(format!("{}/stations/{id}.json", mdapi::BASE)).with_cache(CacheHint {
        max_age: Some(STATION_AGE_S),
        ..CacheHint::default()
    });
    let fetched = pollster::block_on(host.send(&call)).ok()?;
    if !(200..300).contains(&fetched.status) {
        return None;
    }
    let details: Details = serde_json::from_slice(&fetched.body).ok()?;
    let abbreviation = details.stations.into_iter().find_map(|d| d.timezone)?;
    Some(
        match abbreviation.as_str() {
            "EST" | "EDT" => "America/New_York",
            "CST" | "CDT" => "America/Chicago",
            "MST" | "MDT" => "America/Denver",
            "PST" | "PDT" => "America/Los_Angeles",
            "AKST" | "AKDT" => "America/Anchorage",
            "HST" => "Pacific/Honolulu",
            "AST" => "America/Puerto_Rico",
            "ChST" => "Pacific/Guam",
            "SST" => "Pacific/Pago_Pago",
            _ => return None,
        }
        .to_owned(),
    )
}

// ---------------------------------------------------------------------------------------------
// History: what the dates looked like in past years
// ---------------------------------------------------------------------------------------------

/// The dates from `first` to `last`; none when reversed or too long.
fn dates(first: LocalDate, last: LocalDate) -> Vec<LocalDate> {
    let (a, b) = (day_number(first), day_number(last));
    if b < a || b - a >= MAX_TRIP_DAYS {
        return Vec::new();
    }
    (a..=b)
        .map(|n| {
            let (year, month, day) = whether_core::time::civil_from_days(n);
            LocalDate { year, month, day }
        })
        .collect()
}

/// Whether some day of the trip not over yet at the place is past the end of `series`.
fn beyond(trip: &Trip, zone: &str, now: UnixTime) -> Option<impl Fn(Option<&Series<f32>>) -> bool> {
    let tz = time::zone(zone).ok()?;
    let today = day_number(time::local_date(now, &tz).ok()?);
    let starts: Vec<UnixTime> = dates(trip.first, trip.last)
        .into_iter()
        .filter(|&d| day_number(d) >= today)
        .filter_map(|d| time::local_date_bounds(d, &tz).ok().map(|(start, _)| start))
        .collect();
    Some(move |series: Option<&Series<f32>>| {
        let end = series.and_then(Series::latest).map(|s| s.time);
        starts
            .iter()
            .any(|&start| end.is_none_or(|end| end < start))
    })
}

/// Past years' values for the trip's dates where no forecast reaches: the normals of the
/// snapshot's USGS gauges, and sea surface temperature on salt and tidal water. Failures are
/// reported and leave the quantity without a band.
fn history(
    host: &CliHost<'_>,
    trip: &Trip,
    place: &TripPlace,
    snapshot: &SpotSnapshot,
    ctx: &FetchContext,
) -> Option<TripHistory> {
    let beyond = beyond(trip, &place.time_zone, ctx.now)?;
    let report = |source: &str, e: &dyn std::fmt::Display| {
        eprintln!("underbite: {}: {source}: {e}", place.name);
    };
    let usgs = Usgs::new();
    let mut normals: [Option<Vec<DayThresholds>>; 2] = [None, None];
    let wanted = [
        (
            Measurement::Discharge,
            snapshot.water.discharge_m3s.as_ref(),
            { beyond(snapshot.water.discharge_forecast_m3s.as_ref()) },
        ),
        (
            Measurement::WaterTemperature,
            snapshot.water.temperature_c.as_ref(),
            beyond(snapshot.marine.sea_surface_temp_forecast_c.as_ref()),
        ),
    ];
    for (slot, (measurement, series, needed)) in normals.iter_mut().zip(wanted) {
        let Some(target) = series.and_then(|s| usgs_target(s, measurement, ctx.now)) else {
            continue;
        };
        if !needed {
            continue;
        }
        let Ok(requests) =
            usgs.statistics_range_requests(&target, measurement, None, trip.first, trip.last, ctx)
        else {
            continue;
        };
        let mut days = Vec::new();
        for request in &requests {
            let parsed = get(host, request)
                .and_then(|f| Ok(usgs.parse_statistics_range(request, &f.as_response(), ctx)?));
            match parsed {
                Ok(range) => days.extend(range.days.into_iter().map(|d| DayThresholds {
                    day_of_year: d.day_of_year,
                    thresholds: d.thresholds,
                    sample_count: d.sample_count,
                })),
                Err(e) => report("usgs", &format!("{e:#}")),
            }
        }
        *slot = Some(days);
    }
    let sst = if beyond(snapshot.marine.sea_surface_temp_forecast_c.as_ref())
        && is_salt(place.water_type)
    {
        Some(sst_history(host, trip, place, ctx, &report))
    } else {
        None
    };
    let tz = time::zone(&place.time_zone).ok()?;
    let sst_daily = sst
        .as_ref()
        .map(|s| daily_means(&s.iter().collect::<Vec<_>>(), &tz))
        .unwrap_or_default();
    let (usgs_id, marine_id) = (SourceId::USGS, SourceId::OPEN_METEO_MARINE);
    let inputs = HistoryInputs {
        flow: series_input(normals[0].as_deref(), &usgs_id),
        water_temp: series_input(normals[1].as_deref(), &usgs_id),
        sst: sst.is_some().then(|| GriddedInput {
            daily: &sst_daily,
            source: &marine_id,
            kind: HistoryKind::Model,
            min_years: SST_MIN_YEARS,
        }),
        waves: None,
        weather: None,
    };
    TripHistory::build(trip.first, trip.last, &inputs).ok()
}

/// The normals of a gauge, without the daily-record fallback.
fn series_input<'a>(
    normals: Option<&'a [DayThresholds]>,
    source: &'a SourceId,
) -> Option<SeriesInput<'a>> {
    normals.map(|normals| SeriesInput {
        normals,
        normals_source: source,
        daily: &[],
        daily_source: source,
        min_years: NORMALS_MIN_YEARS,
    })
}

/// The sea surface temperature of the trip's dates in each past year from 2023 on whose copy
/// of the dates is over, at the place's rounded point.
fn sst_history(
    host: &CliHost<'_>,
    trip: &Trip,
    place: &TripPlace,
    ctx: &FetchContext,
    report: &dyn Fn(&str, &dyn std::fmt::Display),
) -> Vec<Series<f32>> {
    let (Some(latitude), Some(longitude)) = (place.latitude, place.longitude) else {
        return Vec::new();
    };
    let target = Target {
        place: Place::Point(ctx.round(Coordinates {
            latitude,
            longitude,
        })),
        measurements: BTreeSet::from([Measurement::SeaSurfaceTempForecast]),
        window: Window {
            from: ctx.now,
            to: ctx.now,
        },
        datum: None,
    };
    let today = day_number(ctx.now.to_local(UtcOffset(0)).date);
    let span = trip.last.year - trip.first.year;
    let mut out = Vec::new();
    for year in SST_FIRST_YEAR..=trip.last.year.max(SST_FIRST_YEAR) {
        let (from, to) = (in_year(trip.first, year), in_year(trip.last, year + span));
        if day_number(to) >= today {
            continue;
        }
        let Ok(request) = OpenMeteoMarine.history_request(
            &target,
            MarineHistoryKind::SeaSurfaceTemp,
            from,
            to,
            ctx,
        ) else {
            continue;
        };
        let parsed = get(host, &request).and_then(|f| {
            OpenMeteoMarine
                .parse_history(&request, &f.as_response(), ctx)?
                .sea_surface_temp
                .ok_or_else(|| {
                    SourceError::NoData(Some("no sea surface temperature".into())).into()
                })
        });
        match parsed {
            Ok(series) => out.push(series),
            Err(e) => report("open-meteo-marine", &format!("{year}: {e:#}")),
        }
    }
    out
}

/// `date` in `year`; 29 February is 28 February in a year without one.
fn in_year(date: LocalDate, year: i32) -> LocalDate {
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let day = if date.month == 2 && date.day == 29 && !leap {
        28
    } else {
        date.day
    };
    LocalDate {
        year,
        month: date.month,
        day,
    }
}

fn is_salt(water: WaterType) -> bool {
    matches!(
        water,
        WaterType::TidalRiverEstuary
            | WaterType::BayInshore
            | WaterType::Surf
            | WaterType::Nearshore
            | WaterType::Offshore
    )
}

/// The normals target of the USGS gauge `series` comes from.
fn usgs_target(series: &Series<f32>, measurement: Measurement, now: UnixTime) -> Option<Target> {
    let provenance = series.provenance();
    let station = provenance
        .station
        .as_ref()
        .filter(|s| s.source == SourceId::USGS)?;
    Some(Target {
        place: Place::Station {
            station: station.clone(),
            distance_m: provenance.distance_m,
            relation: provenance.relation,
        },
        measurements: BTreeSet::from([measurement]),
        window: Window { from: now, to: now },
        datum: None,
    })
}

// ---------------------------------------------------------------------------------------------
// Arguments and configuration
// ---------------------------------------------------------------------------------------------

/// The configuration in `path`, or the defaults. Warnings go to stderr; errors stop the command.
fn load_config(path: Option<&Path>) -> anyhow::Result<Config> {
    let Some(path) = path else {
        return Ok(Config::default());
    };
    let text =
        std::fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    let (config, issues) = parse_toml(&text).map_err(|e| anyhow!("{}: {e}", path.display()))?;
    for issue in &issues {
        eprintln!("underbite: {}: {issue}", path.display());
    }
    if has_errors(&issues) {
        let first = issues.iter().find(|i| i.severity == Severity::Error);
        bail!(
            "{} has errors{}",
            path.display(),
            first.map_or_else(String::new, |i| format!(": {i}"))
        );
    }
    Ok(config)
}

/// Parses `lat,lon`.
fn parse_location(text: &str) -> anyhow::Result<(f64, f64)> {
    let (lat, lon) = text
        .split_once(',')
        .ok_or_else(|| anyhow!("--place {text:?}: expected LAT,LON, such as 40.7033,-74.0170"))?;
    let lat: f64 = lat.trim().parse().context("latitude")?;
    let lon: f64 = lon.trim().parse().context("longitude")?;
    if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
        bail!("latitude is -90 to 90 and longitude -180 to 180");
    }
    Ok((lat, lon))
}

/// A serde enum from its name, with `-` for `_`.
fn parse_enum<T: serde::de::DeserializeOwned>(what: &str, text: &str) -> anyhow::Result<T> {
    serde_json::from_value(Value::String(text.replace('-', "_")))
        .map_err(|_| anyhow!("unknown {what} `{text}`"))
}

// ---------------------------------------------------------------------------------------------
// Text
// ---------------------------------------------------------------------------------------------

/// The view as plain text: every string is the view's own.
fn render(view: &TripView) -> anyhow::Result<String> {
    let mut out = String::new();
    writeln!(out, "{} ({}; {})", view.title, view.dates, view.status)?;
    for p in &view.places {
        write!(out, "- {} ({})", p.name, p.water_type_label)?;
        if let Some(a) = &p.approximate {
            write!(out, ", {a}")?;
        }
        writeln!(out)?;
        if let Some(problem) = &p.problem {
            writeln!(out, "  ! {problem}")?;
        }
    }
    for n in &view.notices {
        writeln!(out, "! {}", n.text)?;
    }
    for day in &view.days {
        writeln!(out, "\n== {} ==", day.label)?;
        for (column, place) in view.places.iter().zip(&day.places) {
            let Some(place) = place else { continue };
            if view.places.len() > 1 {
                writeln!(out, "{}", column.name)?;
            }
            for cell in &place.cells {
                writeln!(
                    out,
                    "  {}: {} [{}]",
                    cell.label, cell.value, cell.basis_label
                )?;
                for line in &cell.lines {
                    writeln!(out, "    {line}")?;
                }
            }
            for reason in &place.reasons {
                writeln!(out, "  - {}", reason.text)?;
            }
        }
    }
    if !view.checklist.is_empty() {
        writeln!(out, "\nChecklist")?;
        for row in &view.checklist {
            write!(out, "- {}", row.text)?;
            if let Some(detail) = &row.detail {
                write!(out, " ({detail})")?;
            }
            if let Some(link) = &row.link {
                write!(out, " {link}")?;
            }
            writeln!(out)?;
        }
    }
    for c in &view.attribution {
        match &c.link {
            Some(link) => writeln!(out, "Credit: {} ({link})", c.text)?,
            None => writeln!(out, "Credit: {}", c.text)?,
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use clap::Parser as _;

    use super::*;
    use crate::cli::{Cli, Cmd};

    fn replay_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/replay")
    }

    fn args(extra: &[&str]) -> Args {
        let mut command_line = vec![
            "underbite",
            "trip",
            "--place",
            "40.7033,-74.0170",
            "--from",
            "2026-11-08",
            "--to",
            "2026-11-10",
            "--region",
            "US-NY",
        ];
        command_line.extend_from_slice(extra);
        match Cli::try_parse_from(command_line)
            .expect("arguments")
            .command
        {
            Cmd::Trip(args) => args,
            other => panic!("{other:?}"),
        }
    }

    /// The recorded scenarios at 2026-11-08 12:00 UTC, without a cache.
    fn env() -> Env {
        let global = Global {
            verbose: 0,
            replay: Some(replay_dir()),
            now: Some("2026-11-08T12:00:00Z".into()),
        };
        let mut env = environment(&global).expect("environment");
        env.cache = None;
        env
    }

    fn cells(view: &TripView, day: usize) -> &[underbite_core::view::trip::TripCell] {
        &view.days[day].places[0].as_ref().expect("place day").cells
    }

    fn basis(view: &TripView, day: usize, kind: &str) -> String {
        let cell = cells(view, day)
            .iter()
            .find(|c| serde_json::to_value(c.kind).ok() == Some(Value::from(kind)))
            .unwrap_or_else(|| panic!("no {kind} cell"));
        serde_json::to_value(cell.basis)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default()
    }

    #[test]
    fn arguments_take_several_places_and_negative_longitudes() {
        let a = args(&["--place", "41.2257,-73.8618", "--offline", "--json"]);
        assert_eq!(a.places, ["40.7033,-74.0170", "41.2257,-73.8618"]);
        assert!(a.offline && a.json);
        assert_eq!(a.method, "shore");
        assert!(Cli::try_parse_from(["underbite", "trip", "--from", "2026-11-08"]).is_err());
    }

    #[test]
    fn a_battery_weekend_has_certain_tides_and_forecast_cells() {
        let view = plan(&args(&[]), &env()).expect("view");
        assert_eq!(view.days.len(), 3);
        for day in 0..3 {
            assert_eq!(basis(&view, day, "tides"), "certain", "day {day}");
            assert_eq!(basis(&view, day, "sun"), "certain");
        }
        assert_eq!(basis(&view, 0, "windows"), "forecast");
        insta::assert_snapshot!("battery_text", render(&view).expect("text"));
    }

    #[test]
    fn offline_every_cell_but_the_certain_ones_is_unavailable() {
        let view = plan(&args(&["--offline"]), &env()).expect("view");
        assert_eq!(view.days.len(), 3);
        for day in 0..3 {
            for cell in cells(&view, day) {
                let basis = serde_json::to_value(cell.basis).unwrap();
                assert!(
                    basis == "certain" || basis == "unavailable",
                    "day {day}: {cell:?}"
                );
            }
            assert_eq!(basis(&view, day, "tides"), "certain", "day {day}");
            assert_eq!(basis(&view, day, "verdict"), "unavailable");
        }
        assert_eq!(view.places[0].water_type_label, "River or stream");
        insta::assert_snapshot!("battery_offline_text", render(&view).expect("text"));
    }

    #[test]
    fn offline_asks_for_station_metadata_only() {
        assert!(is_metadata(
            "https://api.tidesandcurrents.noaa.gov/mdapi/prod/webapi/stations/8518750/harcon.json?units=metric"
        ));
        assert!(is_metadata(
            "https://api.tidesandcurrents.noaa.gov/mdapi/prod/webapi/stations/8518750.json"
        ));
        assert!(is_metadata(
            "https://api.tidesandcurrents.noaa.gov/mdapi/prod/webapi/stations.json?type=tidepredictions"
        ));
        assert!(!is_metadata(
            "https://api.tidesandcurrents.noaa.gov/api/prod/datagetter?product=predictions"
        ));
        assert!(!is_metadata(
            "https://marine-api.open-meteo.com/v1/marine?latitude=40.7"
        ));
    }

    #[test]
    fn bad_dates_and_places_are_refused() {
        let mut a = args(&[]);
        a.to = "2026-11-01".into();
        assert!(plan(&a, &env()).is_err());
        a.to = "2026-13-01".into();
        assert!(plan(&a, &env()).is_err());
        assert!(parse_location("40.7").is_err());
        assert!(parse_location("91,0").is_err());
        assert!(parse_enum::<Method>("method", "canoe").is_err());
        assert_eq!(
            parse_enum::<Method>("method", "kayak").ok(),
            Some(Method::Kayak)
        );
    }
}
