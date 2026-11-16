//! `underbite tides`: highs and lows for a CO-OPS tide station, from NOAA's predictions or
//! computed on the device from the station's harmonic constants.

use std::collections::BTreeSet;
use std::process::ExitCode;
use std::sync::Arc;

use anyhow::{Context, anyhow, bail};
use jiff::tz::TimeZone;
use serde::Deserialize;
use serde_json::json;
use underbite_config::schema::Config;
use underbite_core::datum::Height;
use underbite_core::model::{Measurement, Requirements, TideEvent, TideKind, TideSeries};
use underbite_core::provenance::{Relation, SourceId, StationKind, StationRef};
use underbite_core::time::{UnixTime, day_number, parse_date, zone};
use underbite_core::view::Quantity;
use underbite_core::view::format::Formatter;
use underbite_core::view::strings::Key;
use underbite_providers::coops::mdapi::{self, Metadata, StationList};
use underbite_providers::coops::{Coops, CoopsHarmonics};
use underbite_providers::request::{
    CacheHint, Coordinates, Place, Request, Resource, Response, Target, Window,
};
use underbite_providers::session::{Chain, Link, Session};
use underbite_providers::source::{FetchContext, FragmentValue, Fragments, Source};

use crate::cli::Global;
use crate::runtime::env::Env;
use crate::runtime::fetch::Host;
use crate::runtime::transport::{Call, Fetched};

/// Arguments of `underbite tides`.
#[derive(Debug, clap::Args)]
pub(crate) struct Args {
    /// Station id, for example 8518750.
    #[arg(value_name = "STATION")]
    pub(crate) station: String,
    /// First day, as `YYYY-MM-DD` in UTC, the days CO-OPS counts in; today by default.
    #[arg(long, value_name = "DATE")]
    pub(crate) from: Option<String>,
    /// How many days to show.
    #[arg(long, value_name = "N", default_value_t = 3)]
    pub(crate) days: u16,
    /// Compute the predictions on this device from the station's harmonic constants.
    #[arg(long)]
    pub(crate) offline: bool,
    /// Print the events (and the curve) as JSON.
    #[arg(long)]
    pub(crate) json: bool,
}

/// The most days one run covers.
const MAX_DAYS: u16 = 366;
/// Seconds in a day.
const DAY_S: i64 = 86_400;
/// How long station details stay valid, seconds.
const DETAILS_AGE_S: u32 = 30 * 86_400;

/// The IANA zone of a CO-OPS station's standard-time abbreviation. CO-OPS names only the
/// standard time (`EST`), which stands for the zone the station observes.
fn zone_for(abbreviation: &str) -> Option<&'static str> {
    Some(match abbreviation {
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

/// One request through the host, the URL (redacted) named when it fails.
fn send(host: &Host<'_>, call: &Call) -> anyhow::Result<Fetched> {
    host.send(call)
        .with_context(|| format!("request to {}", call.redacted_url()))
}

fn target_for(place: Place, window: Window) -> Target {
    Target {
        place,
        measurements: BTreeSet::from([Measurement::Tide]),
        window,
        datum: None,
    }
}

/// The tide prediction station `id` from CO-OPS's list.
fn find_station(host: &Host<'_>, ctx: &FetchContext, id: &str) -> anyhow::Result<StationRef> {
    let point = ctx.round(Coordinates {
        latitude: 0.0,
        longitude: 0.0,
    });
    let window = Window {
        from: ctx.now,
        to: ctx.now,
    };
    let request = mdapi::stations_request(
        StationList::TidePredictions,
        target_for(Place::Point(point), window),
    );
    let fetched = send(host, &Call::from(&request))?;
    let Metadata::Stations(list) = mdapi::parse(&request, &fetched.as_response())
        .map_err(|e| anyhow!("CO-OPS station list: {e}"))?
    else {
        bail!("CO-OPS station list: unexpected answer");
    };
    list.into_iter()
        .map(|c| c.station)
        .find(|s| s.id == id)
        .with_context(|| format!("{id} is not a CO-OPS tide prediction station"))
}

/// The zone the station's clock shows; UTC (and a note) when CO-OPS does not say.
fn station_zone(host: &Host<'_>, id: &str) -> (TimeZone, String) {
    let found = (|| -> anyhow::Result<String> {
        let url = format!("{}/stations/{id}.json", mdapi::BASE);
        let call = Call::get(url).with_cache(CacheHint {
            max_age: Some(DETAILS_AGE_S),
            ..CacheHint::default()
        });
        let fetched = send(host, &call)?;
        if !(200..300).contains(&fetched.status) {
            bail!("HTTP {}", fetched.status);
        }
        let details: Details = serde_json::from_slice(&fetched.body)?;
        let abbreviation = details
            .stations
            .into_iter()
            .find_map(|d| d.timezone)
            .context("no time zone")?;
        zone_for(&abbreviation)
            .map(str::to_owned)
            .with_context(|| format!("unknown time zone {abbreviation}"))
    })();
    match found {
        Ok(name) => match zone(&name) {
            Ok(tz) => (tz, name),
            Err(_) => (TimeZone::UTC, "UTC".into()),
        },
        Err(e) => {
            tracing::warn!(error = %e, "station time zone");
            eprintln!("note: the station's time zone is unknown; times are UTC");
            (TimeZone::UTC, "UTC".into())
        }
    }
}

/// NOAA's published predictions, one request per range the API allows.
fn online(
    host: &Host<'_>,
    ctx: &FetchContext,
    place: Place,
    window: Window,
    curve: bool,
) -> anyhow::Result<TideSeries> {
    let target = target_for(place, window);
    let requests = Coops
        .plan(&target, &Requirements::default(), ctx)
        .map_err(|e| anyhow!("{e}"))?;
    let mut series: Option<TideSeries> = None;
    let mut samples = Vec::new();
    let mut ran = 0;
    for request in requests.iter().filter(|r| {
        r.resource == Resource::CoopsTideHighLow
            || (curve && r.resource == Resource::CoopsTideCurve)
    }) {
        ran += 1;
        for found in parse_with(&Coops, request, &send(host, &Call::from(request))?, ctx)? {
            match (request.resource, found) {
                (Resource::CoopsTideCurve, t) => {
                    if let Some(c) = &t.curve {
                        samples.extend(c.samples().iter().map(|s| (s.time, s.value)));
                    }
                }
                (_, t) => match &mut series {
                    Some(all) => all.events.extend(t.events),
                    None => series = Some(t),
                },
            }
        }
    }
    if ran == 0 {
        bail!("CO-OPS offers no predictions for this station");
    }
    let mut tide = series.context("CO-OPS returned no highs and lows")?;
    tide.events.sort_by_key(|e| e.time);
    tide.events.dedup_by_key(|e| e.time);
    if !samples.is_empty() {
        samples.sort_by_key(|(t, _)| *t);
        samples.dedup_by_key(|(t, _)| *t);
        let provenance = tide.provenance.clone();
        tide.curve = Some(underbite_core::series::Series::new(
            provenance,
            samples
                .into_iter()
                .map(|(t, v)| underbite_core::series::Sample::new(t, v))
                .collect(),
        ));
    }
    Ok(tide)
}

/// Parses one response into the tide series it holds.
fn parse_with(
    source: &dyn Source,
    request: &Request,
    fetched: &Fetched,
    ctx: &FetchContext,
) -> anyhow::Result<Vec<TideSeries>> {
    let mut out = Fragments::new();
    let response: Response<'_> = fetched.as_response();
    source
        .parse(request, &response, ctx, &mut out)
        .map_err(|e| anyhow!("{}: {e}", Call::from(request).redacted_url()))?;
    Ok(out
        .fragments()
        .iter()
        .filter_map(|f| match &f.value {
            FragmentValue::TideSeries(t) => Some(t.clone()),
            _ => None,
        })
        .collect())
}

/// Predictions computed on the device from the station's harmonic constants (and, for a
/// subordinate station, its reference's and its offsets), through the session that follows the
/// metadata requests.
fn offline(
    host: &Host<'_>,
    ctx: &FetchContext,
    place: Place,
    window: Window,
) -> anyhow::Result<TideSeries> {
    let target = target_for(place, window);
    let chain = Chain {
        links: vec![Link {
            source: Arc::new(CoopsHarmonics),
            target,
        }],
        max_age_s: None,
    };
    let requirements = Requirements {
        measurements: BTreeSet::from([Measurement::Tide]),
        ..Requirements::default()
    };
    let mut session = Session::new(
        std::iter::once((Measurement::Tide, chain)).collect(),
        requirements,
        ctx.clone(),
    );
    while let Some(request) = session.next_request() {
        let call = Call::from(&request.clone());
        match send(host, &call) {
            Ok(fetched) => session.on_response(&fetched.as_response()),
            Err(e) => session.on_transport_error(&format!("{e:#}")),
        }
    }
    let outcome = session.finish(ctx.now);
    outcome.marine.tide.ok_or_else(|| {
        let why: Vec<String> = outcome
            .errors
            .iter()
            .map(|e| format!("{}: {}", e.source, e.message))
            .collect();
        anyhow!(
            "no computed predictions for this station ({}); the constants are fetched once and \
             cached, so the first run needs the network",
            why.join("; ")
        )
    })
}

/// Runs `underbite tides`.
pub(crate) fn run(args: &Args, global: &Global) -> anyhow::Result<ExitCode> {
    if args.days == 0 || args.days > MAX_DAYS {
        bail!("--days is 1 to {MAX_DAYS}");
    }
    let env = Env::from_global(global)?;
    let host = env.host();
    let config = Config::default();
    let ctx = FetchContext::new(env.now, env.options.user_agent.clone())
        .with_coordinate_precision(config.sources.coordinate_precision);

    let first = match &args.from {
        Some(text) => {
            let date = parse_date(text).map_err(|e| anyhow!("--from {text:?}: {e}"))?;
            UnixTime(day_number(date) * DAY_S)
        }
        None => UnixTime(env.now.0.div_euclid(DAY_S) * DAY_S),
    };
    let window = Window {
        from: first,
        to: UnixTime(first.0 + i64::from(args.days) * DAY_S - 1),
    };

    let station = find_station(&host, &ctx, &args.station)?;
    let place = Place::Station {
        station: station.clone(),
        distance_m: None,
        relation: Relation::Nearby,
    };
    let (tz, zone_name) = station_zone(&host, &station.id);
    let (tide, computed, credit) = if args.offline {
        (
            offline(&host, &ctx, place, window)?,
            true,
            CoopsHarmonics.attribution(),
        )
    } else {
        (
            online(&host, &ctx, place, window, args.json)?,
            false,
            Coops.attribution(),
        )
    };

    let f = Formatter::new(
        config.units.display_units(),
        config.presentation.voice,
        config.presentation.clock,
        &zone_name,
    );
    let shown = Shown {
        f,
        tz,
        zone_name,
        station,
        tide,
        window,
        computed,
        credit: credit.credit,
    };
    if args.json {
        println!("{}", serde_json::to_string_pretty(&shown.json())?);
    } else {
        shown.print();
    }
    Ok(ExitCode::SUCCESS)
}

/// A tide table ready to print: the formatter and everything the output names.
struct Shown {
    f: Formatter,
    tz: TimeZone,
    zone_name: String,
    station: StationRef,
    tide: TideSeries,
    window: Window,
    computed: bool,
    credit: String,
}

impl Shown {
    fn height(&self, metres: Option<f32>) -> Quantity {
        self.f.height(metres.map(|metres| Height {
            datum: self.tide.datum,
            metres,
        }))
    }

    fn abbreviation(&self, t: UnixTime) -> String {
        jiff::Timestamp::from_second(t.0)
            .map(|ts| ts.to_zoned(self.tz.clone()).strftime("%Z").to_string())
            .unwrap_or_default()
    }

    fn label(&self, kind: TideKind) -> &'static str {
        self.f.strings.get(match kind {
            TideKind::High => Key::TideHigh,
            TideKind::Low => Key::TideLow,
        })
    }

    fn events(&self) -> impl Iterator<Item = &TideEvent> {
        self.tide
            .events
            .iter()
            .filter(|e| e.time >= self.window.from && e.time <= self.window.to)
    }

    fn json(&self) -> serde_json::Value {
        let rows: Vec<_> = self
            .events()
            .map(|e| {
                json!({
                    "t": e.time.0,
                    "time": format!(
                        "{} {} {}",
                        self.f.date_of(e.time),
                        self.f.clock(e.time),
                        self.abbreviation(e.time)
                    ),
                    "kind": match e.kind { TideKind::High => "high", TideKind::Low => "low" },
                    "height_m": e.height_m,
                    "height": self.height(e.height_m).full(),
                })
            })
            .collect();
        let curve: Vec<_> = self
            .tide
            .curve
            .as_ref()
            .map(|c| {
                c.samples()
                    .iter()
                    .filter(|s| s.time >= self.window.from && s.time <= self.window.to)
                    .map(|s| json!({ "t": s.time.0, "height_m": s.value }))
                    .collect()
            })
            .unwrap_or_default();
        json!({
            "station": {
                "id": self.station.id,
                "name": self.station.name,
                "kind": self.station.kind,
            },
            "zone": self.zone_name,
            "datum": self.height(Some(0.0)).unit,
            "source": if self.computed { SourceId::COOPS_HARMONICS } else { SourceId::COOPS },
            "computed_on_device": self.computed,
            "credit": self.credit,
            "events": rows,
            "curve": curve,
        })
    }

    fn print(&self) {
        let kind = match self.station.kind {
            StationKind::TideSubordinate => "subordinate station",
            _ => "harmonic station",
        };
        println!("{} ({}), {kind}", self.station.name, self.station.id);
        println!("Times in {}", self.zone_name);
        if self.computed {
            println!(
                "Computed on this device from the station's harmonic constants, not NOAA's \
                 published predictions."
            );
        } else {
            println!("Predictions published by NOAA CO-OPS.");
        }
        println!();
        let mut day = String::new();
        let mut any = false;
        for e in self.events() {
            any = true;
            let date = self.f.date_of(e.time);
            if date != day {
                println!("{date}");
                date.clone_into(&mut day);
            }
            println!(
                "  {} {}  {:<4} {}",
                self.f.clock(e.time),
                self.abbreviation(e.time),
                self.label(e.kind),
                self.height(e.height_m).full(),
            );
        }
        if !any {
            println!("No highs or lows in that range.");
        }
        println!();
        println!("{}", self.credit);
    }
}
