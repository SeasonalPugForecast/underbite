//! The two sessions of a spot fetch: Underbite's water and marine session over the discovery
//! plan, and whether's weather session, one request at a time through the same host.

use std::collections::{BTreeMap, BTreeSet};

use underbite_core::discover::rank::{LinkPlace, Plan};
use underbite_core::model::{
    Measurement, Requirements, SourceError as ModelSourceError, SourceErrorKind,
};
use underbite_core::provenance::{Attribution, SourceId};
use underbite_providers::request::{Coordinates, Place, Target, Window};
use underbite_providers::session::usage::OPEN_METEO;
use underbite_providers::session::{Chain, Link, Session, SessionOutcome};
use underbite_providers::source::{FetchContext, Period};
use whether_core::model::{ProviderId, WeatherSnapshot};
use whether_providers::{
    Attempt as WeatherAttempt, FetchContext as WeatherContext, FetchError, FetchSession,
    HttpResponse, Provider, ProviderError,
};

use crate::host::{Call, Driver, FetchHost, Fetched};
use crate::registry::{WeatherChain, water_source, weather_requirements};

// ---------------------------------------------------------------------------------------------
// The water session
// ---------------------------------------------------------------------------------------------

/// The session's chains from the discovery plan: each link's source from the registry and its
/// place (a point link is the spot rounded by the context).
pub(crate) fn session_chains(
    plan: &Plan,
    requirements: &Requirements,
    ctx: &FetchContext,
) -> BTreeMap<Measurement, Chain> {
    let window = Window::around(ctx.now, requirements);
    plan.chains
        .iter()
        .map(|(&m, planned)| {
            let links = planned
                .links
                .iter()
                .filter_map(|link| {
                    let source = water_source(&link.source)?;
                    let place = match &link.place {
                        LinkPlace::Station {
                            station,
                            distance_m,
                            relation,
                        } => Place::Station {
                            station: station.clone(),
                            distance_m: Some(*distance_m),
                            relation: *relation,
                        },
                        LinkPlace::Point {
                            latitude,
                            longitude,
                            ..
                        } => Place::Point(ctx.round(Coordinates {
                            latitude: *latitude,
                            longitude: *longitude,
                        })),
                    };
                    let target = Target {
                        place,
                        measurements: BTreeSet::from([m]),
                        window,
                        datum: None,
                    };
                    Some(Link { source, target })
                })
                .collect();
            let chain = Chain {
                links,
                max_age_s: Some(planned.max_age_s),
            };
            (m, chain)
        })
        .collect()
}

/// Runs the water session to its end, one request at a time.
pub(crate) async fn run_session<H: FetchHost>(
    host: &Driver<'_, H>,
    mut session: Session,
) -> SessionOutcome {
    while let Some(request) = session.next_request() {
        let request = request.clone();
        let call = Call::from(&request);
        if let Some(hit) = host.cached_response(&call).await {
            session.refund();
            session.on_response(&hit.as_response());
            continue;
        }
        if request.metered
            && let Err(e) = host.save_usage().await
        {
            tracing::warn!(error = %e, "metered request not sent");
            session.on_transport_error(&e);
            continue;
        }
        match host.send(&call).await {
            Ok(fetched) => session.on_response(&fetched.as_response()),
            Err(e) => session.on_transport_error(&e.to_string()),
        }
    }
    session.finish(host.now)
}

// ---------------------------------------------------------------------------------------------
// Weather
// ---------------------------------------------------------------------------------------------

/// The weather library's response for a fetched one.
fn weather_response(fetched: &Fetched) -> HttpResponse<'_> {
    HttpResponse {
        status: fetched.status,
        body: &fetched.body,
        date: fetched.date,
        expires: fetched.expires,
        last_modified: fetched.last_modified.as_deref(),
    }
}

/// The snapshot's kind for a weather provider's error.
fn weather_error_kind(error: &ProviderError) -> SourceErrorKind {
    match error {
        ProviderError::RateLimited { .. } | ProviderError::Quota(_) => SourceErrorKind::RateLimited,
        ProviderError::Http { status: 404, .. } => SourceErrorKind::NotFound,
        ProviderError::Malformed(_) => SourceErrorKind::Parse,
        ProviderError::Transport(_) => SourceErrorKind::Transport,
        _ => SourceErrorKind::Other,
    }
}

/// What the weather session produced.
pub(crate) struct Weather {
    pub(crate) snapshot: Option<WeatherSnapshot>,
    pub(crate) errors: Vec<ModelSourceError>,
    pub(crate) attribution: Vec<Attribution>,
}

/// Runs whether's session over `chain` at the rounded spot. Every call to Open-Meteo is counted
/// in the ledger it shares with Open-Meteo Marine and the storm check (one limit per address).
pub(crate) async fn fetch_weather<H: FetchHost>(
    host: &Driver<'_, H>,
    chain: &WeatherChain,
    loc: Coordinates,
    user_agent: &str,
) -> Weather {
    let primary: Vec<&dyn Provider> = chain.primary.iter().map(AsRef::as_ref).collect();
    let supplements: Vec<&dyn Provider> = chain.supplements.iter().map(AsRef::as_ref).collect();
    let ctx = WeatherContext {
        now: host.now,
        user_agent: user_agent.to_string(),
        utc_offset_hint: None,
        if_modified_since: None,
        language: "en".to_string(),
    };
    let mut session = FetchSession::new(&primary, &supplements, loc, weather_requirements(), ctx);
    while let Some(request) = session.next_request() {
        let call = Call::from(request);
        let cached = host.cached_response(&call).await;
        if cached.is_none() && call.url.contains("open-meteo.com") {
            let now = host.now;
            host.usage.with(|l| l.count(&OPEN_METEO, Period::Day, now));
            if let Err(e) = host.save_usage().await {
                tracing::warn!(error = %e, "weather request not sent");
                session.on_transport_error(&e);
                continue;
            }
        }
        let fetched = match cached {
            Some(hit) => Ok(hit),
            None => host.send(&call).await,
        };
        match fetched {
            Ok(fetched) => session.on_response(&weather_response(&fetched)),
            Err(e) => session.on_transport_error(&e.to_string()),
        }
    }
    let attempts = match session.finish(host.now) {
        Ok((snapshot, attempts)) => {
            let attribution = std::iter::once(snapshot.provider)
                .chain(snapshot.supplemented_by.iter().copied())
                .filter_map(|id| {
                    let provider = chain
                        .primary
                        .iter()
                        .chain(&chain.supplements)
                        .find(|p| p.id() == id)?;
                    let credit = provider.attribution();
                    Some(Attribution {
                        source: SourceId::new(id.id()),
                        credit: credit.text.to_string(),
                        url: Some(credit.url.to_string()),
                    })
                })
                .collect();
            return Weather {
                errors: weather_errors(&attempts),
                snapshot: Some(snapshot),
                attribution,
            };
        }
        Err(FetchError::AllProvidersFailed(attempts)) => attempts,
        Err(e) => {
            let error = ModelSourceError {
                source: SourceId::new(ProviderId::OpenMeteo.id()),
                measurement: None,
                kind: SourceErrorKind::Other,
                message: format!("weather: {e}"),
            };
            return Weather {
                snapshot: None,
                errors: vec![error],
                attribution: Vec::new(),
            };
        }
    };
    let mut errors = weather_errors(&attempts);
    if attempts.is_empty() {
        errors.push(ModelSourceError {
            source: SourceId::new("weather"),
            measurement: None,
            kind: SourceErrorKind::Other,
            message: "weather: no weather provider configured".into(),
        });
    }
    Weather {
        snapshot: None,
        errors,
        attribution: Vec::new(),
    }
}

/// The failed attempts of the weather session as snapshot errors.
fn weather_errors(attempts: &[WeatherAttempt]) -> Vec<ModelSourceError> {
    attempts
        .iter()
        .filter_map(|a| {
            let error = a.error.as_ref()?;
            Some(ModelSourceError {
                source: SourceId::new(a.provider.id()),
                measurement: None,
                kind: weather_error_kind(error),
                message: format!("weather {:?}: {error}", a.endpoint).to_lowercase(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use underbite_config::schema::Config;
    use underbite_providers::session::usage::UsageLedger;
    use underbite_providers::source::Secrets;

    use super::*;
    use crate::host::TransportError;
    use crate::spot::{FetchOptions, fetch_spot};
    use crate::store;
    use crate::testing::{MemoryHost, battery, now};

    /// The replay host with a response cache: it answers `cached` for every call, or for none.
    struct CachingHost {
        inner: MemoryHost,
        answers_all: bool,
    }

    impl FetchHost for CachingHost {
        fn send(
            &self,
            call: &Call,
        ) -> impl Future<Output = Result<Fetched, TransportError>> + Send {
            self.inner.send(call)
        }

        fn cached(&self, call: &Call) -> impl Future<Output = Option<Fetched>> + Send {
            let hit = if self.answers_all {
                pollster::block_on(self.inner.send(call)).ok()
            } else {
                None
            };
            std::future::ready(hit)
        }

        fn load(&self, namespace: &str, key: &str) -> impl Future<Output = Option<String>> + Send {
            self.inner.load(namespace, key)
        }

        fn store(
            &self,
            namespace: &str,
            key: &str,
            value: String,
        ) -> impl Future<Output = ()> + Send {
            self.inner.store(namespace, key, value)
        }

        fn save_usage(&self, ledger: String) -> impl Future<Output = Result<(), String>> + Send {
            self.inner.save_usage(ledger)
        }
    }

    /// Open-Meteo calls in the ledger a Battery fetch through the host saved.
    fn open_meteo_used(answers_all: bool) -> u32 {
        let host = CachingHost {
            inner: MemoryHost::replay("battery"),
            answers_all,
        };
        let options = FetchOptions {
            allow_metered: true,
            secrets: Secrets::default(),
            user_agent: "underbite/test".into(),
            infer_water_type: false,
        };
        let out = pollster::block_on(fetch_spot(
            &host,
            &battery(),
            &Config::default(),
            now(),
            &options,
        ));
        assert!(out.snapshot.marine.tide.is_some());
        let ledger: UsageLedger = host
            .inner
            .stored(store::USAGE, store::LEDGER_KEY)
            .expect("ledger");
        ledger.used(&OPEN_METEO, now())
    }

    #[test]
    fn calls_a_cache_answers_are_not_counted() {
        assert_eq!(open_meteo_used(true), 0);
    }

    #[test]
    fn calls_no_cache_answers_are_all_counted() {
        // Marine, the storm point and the forecast.
        assert_eq!(open_meteo_used(false), 3);
    }
}
