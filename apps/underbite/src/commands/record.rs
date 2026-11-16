//! `underbite record`: write a provider's live response into the fixtures directory and print
//! the index line for it. Run by hand; the requests are built by the providers the app uses, so
//! a recording is what the app would be sent.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, anyhow, bail};
use underbite_core::model::{Measurement, Requirements};
use underbite_core::provenance::{Relation, SourceId, StationKind, StationRef};
use underbite_core::time::{UnixTime, day_number, parse_date};
use underbite_fetch::Call;
use underbite_providers::coops::{Coops, mdapi};
use underbite_providers::ndbc::Ndbc;
use underbite_providers::request::{
    Place, Request, Resource, Target, Window, is_secret_header, redact_url, secret_values,
};
use underbite_providers::source::{FetchContext, Source};
use underbite_providers::usgs::Usgs;

use crate::cli::Global;
use crate::runtime::env::Env;

/// Arguments of `underbite record`.
#[derive(Debug, clap::Args)]
pub(crate) struct Args {
    /// Provider to record: `coops`, `usgs` or `ndbc`.
    #[arg(value_name = "PROVIDER")]
    pub(crate) provider: String,
    /// What to record: `predictions`, `water-level` or `datums` (coops); `latest-continuous`
    /// (usgs); `txt` (ndbc). Without one, the resources of the provider are listed.
    #[arg(value_name = "RESOURCE")]
    pub(crate) resource: Option<String>,
    /// Station id (coops, ndbc).
    #[arg(long, value_name = "ID")]
    pub(crate) station: Option<String>,
    /// Site id (usgs), with or without the `USGS-` prefix.
    #[arg(long, value_name = "ID")]
    pub(crate) site: Option<String>,
    /// First day, `YYYY-MM-DD` (UTC).
    #[arg(long, value_name = "DATE")]
    pub(crate) begin: Option<String>,
    /// Last day, `YYYY-MM-DD` (UTC); the first day by default.
    #[arg(long, value_name = "DATE")]
    pub(crate) end: Option<String>,
    /// `hilo` (default) or `6` for coops predictions.
    #[arg(long, value_name = "INTERVAL", default_value = "hilo")]
    pub(crate) interval: String,
    /// Directory holding one folder per provider.
    #[arg(long, value_name = "DIR", default_value = "tests/fixtures/raw")]
    pub(crate) out: PathBuf,
    /// File name to write; a default follows the fixtures' naming.
    #[arg(long, value_name = "NAME")]
    pub(crate) name: Option<String>,
    /// Replace a fixture that exists.
    #[arg(long)]
    pub(crate) force: bool,
}

const RESOURCES: &[(&str, &[&str])] = &[
    ("coops", &["predictions", "water-level", "datums"]),
    ("usgs", &["latest-continuous"]),
    ("ndbc", &["txt"]),
];

/// What one recording is: the request, the folder and the file name.
struct Plan {
    request: Request,
    folder: &'static str,
    name: String,
    extension: &'static str,
}

fn need<'a>(value: Option<&'a String>, flag: &str, resource: &str) -> anyhow::Result<&'a str> {
    value
        .map(String::as_str)
        .with_context(|| format!("{resource} needs {flag}"))
}

fn day(text: &str, flag: &str) -> anyhow::Result<UnixTime> {
    let date = parse_date(text).map_err(|e| anyhow!("{flag} {text:?}: {e}"))?;
    Ok(UnixTime(day_number(date) * 86_400))
}

fn station(source: SourceId, id: &str, kind: StationKind) -> Place {
    Place::Station {
        station: StationRef {
            source,
            id: id.to_owned(),
            name: id.to_owned(),
            kind,
            latitude: 0.0,
            longitude: 0.0,
            bin: None,
            reference_id: None,
        },
        distance_m: None,
        relation: Relation::Nearby,
    }
}

fn target(place: Place, measurement: Measurement, window: Window) -> Target {
    Target {
        place,
        measurements: BTreeSet::from([measurement]),
        window,
        datum: None,
    }
}

/// The single request of `resource` that `source` plans for `target`.
fn planned(
    source: &dyn Source,
    target: &Target,
    ctx: &FetchContext,
    resource: Resource,
) -> anyhow::Result<Request> {
    let mut found: Vec<Request> = source
        .plan(target, &Requirements::default(), ctx)
        .map_err(|e| anyhow!("{e}"))?
        .into_iter()
        .filter(|r| r.resource == resource)
        .collect();
    match found.len() {
        1 => Ok(found.remove(0)),
        0 => bail!("the provider plans no such request"),
        _ => bail!("that range takes more than one request; record a shorter one"),
    }
}

/// The days `--begin` and `--end` name, and the first day's text; without `--begin` a
/// request that needs no dates gets the epoch day.
fn days(args: &Args, resource: &str, required: bool) -> anyhow::Result<(Window, String)> {
    let from_text = match args.begin.as_deref() {
        Some(b) => b,
        None if required => bail!("{resource} needs --begin"),
        None => "1970-01-01",
    };
    let from = day(from_text, "--begin")?;
    let to = match &args.end {
        Some(e) => day(e, "--end")?,
        None => from,
    };
    Ok((
        Window {
            from,
            to: UnixTime(to.0 + 86_399),
        },
        from_text.to_owned(),
    ))
}

fn plan(args: &Args, resource: &str, ctx: &FetchContext) -> anyhow::Result<Plan> {
    let window = |required: bool| days(args, resource, required);
    match (args.provider.as_str(), resource) {
        ("coops", "predictions") => {
            let id = need(args.station.as_ref(), "--station", resource)?;
            let (window, date) = window(true)?;
            let t = target(
                station(SourceId::COOPS, id, StationKind::TideHarmonic),
                Measurement::Tide,
                window,
            );
            let (wanted, label) = match args.interval.as_str() {
                "hilo" => (Resource::CoopsTideHighLow, "hilo"),
                "6" => (Resource::CoopsTideCurve, "predictions-6min"),
                other => bail!("--interval is `hilo` or `6`, not {other:?}"),
            };
            Ok(Plan {
                request: planned(&Coops, &t, ctx, wanted)?,
                folder: "coops",
                name: format!("coops-{id}-{label}-{date}"),
                extension: "json",
            })
        }
        ("coops", "water-level") => {
            let id = need(args.station.as_ref(), "--station", resource)?;
            let (window, date) = window(true)?;
            let t = target(
                station(SourceId::COOPS, id, StationKind::WaterLevel),
                Measurement::Tide,
                window,
            );
            Ok(Plan {
                request: planned(&Coops, &t, ctx, Resource::CoopsWaterLevel)?,
                folder: "coops",
                name: format!("coops-{id}-water-level-{date}"),
                extension: "json",
            })
        }
        ("coops", "datums") => {
            let id = need(args.station.as_ref(), "--station", resource)?;
            let (window, _) = window(false)?;
            let t = target(
                station(SourceId::COOPS, id, StationKind::TideHarmonic),
                Measurement::Tide,
                window,
            );
            Ok(Plan {
                request: mdapi::datums_request(t).map_err(|e| anyhow!("{e}"))?,
                folder: "coops",
                name: format!("coops-mdapi-{id}-datums-metric"),
                extension: "json",
            })
        }
        ("usgs", "latest-continuous") => {
            let site = need(args.site.as_ref(), "--site", resource)?;
            let site = site.strip_prefix("USGS-").unwrap_or(site);
            let (window, _) = window(false)?;
            let t = target(
                station(
                    SourceId::USGS,
                    &format!("USGS-{site}"),
                    StationKind::UsgsSite(underbite_core::provenance::UsgsSiteType::Stream),
                ),
                Measurement::Discharge,
                window,
            );
            Ok(Plan {
                request: planned(&Usgs::new(), &t, ctx, Resource::UsgsLatestContinuous)?,
                folder: "usgs",
                name: format!("usgs-latest-continuous-{site}"),
                extension: "json",
            })
        }
        ("ndbc", "txt") => {
            let id = need(args.station.as_ref(), "--station", resource)?;
            let (window, _) = window(false)?;
            let t = target(
                station(SourceId::NDBC, id, StationKind::Buoy),
                Measurement::ObservedWind,
                window,
            );
            Ok(Plan {
                request: planned(&Ndbc, &t, ctx, Resource::NdbcStandard)?,
                folder: "ndbc",
                name: format!("ndbc-{id}"),
                extension: "txt",
            })
        }
        (provider, resource) => bail!(
            "cannot record {resource} of {provider}; {}",
            listing(provider)
        ),
    }
}

fn listing(provider: &str) -> String {
    match RESOURCES.iter().find(|(p, _)| *p == provider) {
        Some((_, resources)) => format!("{provider} offers {}", resources.join(", ")),
        None => format!(
            "providers are {}",
            RESOURCES
                .iter()
                .map(|(p, _)| *p)
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// Whether the request carries a key, in its URL or a header.
fn carries_secret(request: &Request) -> bool {
    !secret_values(&request.url).is_empty()
        || request
            .headers
            .iter()
            .any(|(name, _)| is_secret_header(name))
}

/// The call that records `request`: the request as the app sends it, asking for a fresh copy,
/// so a recording is never answered from the cache nor stored in it.
fn recording_call(request: &Request) -> Call {
    let mut call = Call::from(request);
    call.cache.fresh = true;
    call
}

/// The file a recording is written to; an existing one is kept unless `force`.
fn destination(
    out: &Path,
    plan: &Plan,
    name: Option<&str>,
    force: bool,
) -> anyhow::Result<PathBuf> {
    let file = match name {
        Some(n) => n.to_owned(),
        None => format!("{}.{}", plan.name, plan.extension),
    };
    if file.contains(['/', '\\']) || file.starts_with('.') {
        bail!("--name is a file name, not a path");
    }
    let path = out.join(plan.folder).join(file);
    if path.exists() && !force {
        bail!("{} exists; use --force to replace it", path.display());
    }
    Ok(path)
}

/// Runs `underbite record`.
pub(crate) fn run(args: &Args, global: &Global) -> anyhow::Result<ExitCode> {
    let Some(resource) = args.resource.as_deref() else {
        println!("{}", listing(&args.provider));
        return Ok(ExitCode::SUCCESS);
    };
    let env = Env::from_global(global)?;
    let ctx = FetchContext::new(env.now, env.options.user_agent.clone())
        .with_secrets(env.options.secrets.clone());
    let plan = plan(args, resource, &ctx)?;
    if carries_secret(&plan.request) {
        bail!(
            "refusing to record {}: the request carries a key (unset USGS_API_KEY to record it \
             without)",
            redact_url(&plan.request.url)
        );
    }
    let path = destination(&args.out, &plan, args.name.as_deref(), args.force)?;
    let call = recording_call(&plan.request);
    let fetched = pollster::block_on(underbite_fetch::send(&env.host(), &call))
        .with_context(|| format!("request to {}", call.redacted_url()))?;
    if !(200..300).contains(&fetched.status) {
        bail!(
            "{} answered HTTP {}; nothing written",
            call.redacted_url(),
            fetched.status
        );
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    std::fs::write(&path, &fetched.body).with_context(|| format!("writing {}", path.display()))?;
    let date = jiff::Timestamp::from_second(env.now.0)
        .map(|t| t.strftime("%Y-%m-%d").to_string())
        .unwrap_or_default();
    let file = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    eprintln!("wrote {} ({} bytes)", path.display(), fetched.body.len());
    println!("- `{file}`: `{}` ({date})", call.redacted_url());
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Parser)]
    struct Cli {
        #[command(flatten)]
        args: Args,
    }

    fn args(extra: &[&str]) -> Args {
        let mut argv = vec!["record"];
        argv.extend_from_slice(extra);
        Cli::try_parse_from(argv).unwrap().args
    }

    fn ctx() -> FetchContext {
        FetchContext::new(UnixTime(1_794_000_000), "underbite/test")
    }

    #[test]
    fn coops_predictions_are_the_apps_own_request() {
        let a = args(&[
            "coops",
            "predictions",
            "--station",
            "8518750",
            "--begin",
            "2026-11-08",
            "--end",
            "2026-11-14",
        ]);
        let p = plan(&a, "predictions", &ctx()).unwrap();
        assert!(p.request.url.contains("interval=hilo"));
        assert!(
            p.request
                .url
                .contains("begin_date=20261108&end_date=20261114")
        );
        assert_eq!(p.name, "coops-8518750-hilo-2026-11-08");
    }

    #[test]
    fn a_range_over_the_limit_is_refused() {
        let a = args(&[
            "coops",
            "predictions",
            "--station",
            "8518750",
            "--begin",
            "2024-01-01",
            "--end",
            "2026-01-01",
        ]);
        assert!(plan(&a, "predictions", &ctx()).is_err());
    }

    #[test]
    fn missing_arguments_and_unknown_resources_say_so() {
        let a = args(&["coops", "predictions", "--begin", "2026-11-08"]);
        assert!(
            plan(&a, "predictions", &ctx())
                .err()
                .unwrap()
                .to_string()
                .contains("--station")
        );
        let a = args(&["usgs", "bogus"]);
        let err = plan(&a, "bogus", &ctx()).err().unwrap().to_string();
        assert!(err.contains("latest-continuous"), "{err}");
    }

    #[test]
    fn a_keyed_request_is_recognised() {
        let a = args(&["usgs", "latest-continuous", "--site", "01375000"]);
        let keyed = FetchContext::new(UnixTime(0), "underbite/test").with_secrets(
            underbite_providers::source::Secrets {
                usgs_api_key: Some("k".into()),
            },
        );
        assert!(carries_secret(
            &plan(&a, "latest-continuous", &keyed).unwrap().request
        ));
        assert!(!carries_secret(
            &plan(&a, "latest-continuous", &ctx()).unwrap().request
        ));
    }

    #[test]
    fn a_recording_is_the_apps_request_and_never_answered_from_the_cache() {
        use crate::runtime::cache::Cache;
        use underbite_fetch::Fetched;

        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::at(dir.path());
        let a = args(&["ndbc", "txt", "--station", "44025"]);
        let p = plan(&a, "txt", &ctx()).unwrap();
        let plain = Call::from(&p.request);
        let old = Fetched {
            cache_max_age: Some(3_600),
            ..Fetched::new(200, "old")
        };
        assert!(cache.store_response(&plain, &old, ctx().now).unwrap());
        let call = recording_call(&p.request);
        assert_eq!((&call.url, &call.headers), (&plain.url, &plain.headers));
        assert!(cache.load_response(&call, ctx().now).is_none());
        assert!(!cache.store_response(&call, &old, ctx().now).unwrap());
    }

    #[test]
    fn an_existing_fixture_is_kept_without_force() {
        let dir = tempfile::tempdir().unwrap();
        let a = args(&["ndbc", "txt", "--station", "44025"]);
        let p = plan(&a, "txt", &ctx()).unwrap();
        let path = destination(dir.path(), &p, None, false).unwrap();
        assert!(path.ends_with("ndbc/ndbc-44025.txt"));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"x").unwrap();
        assert!(destination(dir.path(), &p, None, false).is_err());
        assert!(destination(dir.path(), &p, None, true).is_ok());
        assert!(destination(dir.path(), &p, Some("../x"), true).is_err());
    }
}
