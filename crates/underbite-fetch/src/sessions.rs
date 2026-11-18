//! The sessions of a spot fetch: Underbite's water and marine session over the discovery plan,
//! whether's weather session and the stocking lists, one request at a time through the same
//! host.

use std::collections::{BTreeMap, BTreeSet};

use underbite_core::discover::rank::{LinkPlace, Plan};
use underbite_core::model::{
    Measurement, Requirements, SourceError as ModelSourceError, SourceErrorKind, Spot,
    StockingEvent,
};
use underbite_core::provenance::{Attribution, SourceId};
use underbite_providers::request::{CacheHint, Coordinates, Place, Target, Window};
use underbite_providers::session::usage::OPEN_METEO;
use underbite_providers::session::{Chain, Link, Session, SessionOutcome};
use underbite_providers::source::{FetchContext, Period, SourceError};
use underbite_providers::stocking::catalogue::{
    Catalogue, Placed, canonical_county, canonical_waterbody, name_matches,
};
use underbite_providers::stocking::socrata_ny::{self, StockingRequest};
use whether_core::model::{ProviderId, WeatherSnapshot};
use whether_providers::{
    Attempt as WeatherAttempt, FetchContext as WeatherContext, FetchError, FetchSession,
    HttpResponse, Provider, ProviderError,
};

use crate::host::{Call, Driver, FetchHost, Fetched};
use crate::registry::{WeatherChain, water_source, weather_requirements};
use crate::source_error;

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

// ---------------------------------------------------------------------------------------------
// Stocking
// ---------------------------------------------------------------------------------------------

/// How far from the spot a stocked water's point may lie, metres. A river's point is one point
/// on its line, often well away from the spot; beyond this no stocked water is near, and the
/// spot is outside the lists' state.
const STOCKING_REACH_M: f64 = 30_000.0;
/// How long a stocking list may be reused, seconds: the lists change a few times a year.
const STOCKING_MAX_AGE_S: u32 = 24 * 3_600;

/// What the stocking lists gave for a spot.
#[derive(Debug, Default)]
pub(crate) struct Stocking {
    /// The actual and planned stockings of the spot's water.
    pub(crate) events: Vec<StockingEvent>,
    /// The lists' credit, once one answered.
    pub(crate) attribution: Option<Attribution>,
    pub(crate) errors: Vec<ModelSourceError>,
}

/// The stocked waters of the bundled catalogue that the spot is on: those near it whose list
/// name or map name is the name the map gives the spot's water, the spot's water body or the
/// spot's own name, nearest first, each water once. `Unsupported` when no stocked water at all
/// is near: the lists cover New York State only.
fn stocked_waters<'c>(
    catalogue: &'c Catalogue,
    spot: &Spot,
    water_name: Option<&str>,
) -> Result<Vec<&'c Placed>, SourceError> {
    let near = catalogue.near(spot.latitude, spot.longitude, STOCKING_REACH_M);
    if near.is_empty() {
        return Err(SourceError::Unsupported(
            "the NYS DEC stocking lists cover New York waters only".into(),
        ));
    }
    let names: Vec<&str> = water_name
        .into_iter()
        .chain(spot.water_body.as_ref().and_then(|w| w.name.as_deref()))
        .chain([spot.name.as_str()])
        .collect();
    let mut out: Vec<&Placed> = Vec::new();
    for (placed, _) in near {
        let on_it = names
            .iter()
            .any(|n| name_matches(&placed.waterbody, n) || name_matches(&placed.name, n));
        let water = canonical_waterbody(&placed.waterbody);
        let seen = out
            .iter()
            .any(|p| canonical_waterbody(&p.waterbody) == water);
        if on_it && !seen {
            out.push(placed);
        }
    }
    Ok(out)
}

/// Whether `event` is a stocking of one of `waters`, compared as the catalogue looks pairs up:
/// the county in its canonical form (an event without one matches on its water), the water
/// ignoring case and spacing.
fn of_waters(event: &StockingEvent, waters: &[&Placed]) -> bool {
    let county = event.county.as_deref().map(canonical_county);
    let water = canonical_waterbody(&event.waterbody);
    waters.iter().any(|p| {
        canonical_waterbody(&p.waterbody) == water
            && county
                .as_ref()
                .is_none_or(|c| *c == canonical_county(&p.county))
    })
}

/// One stocking list, from the host's cache or the portal.
async fn stocking_list<H: FetchHost>(
    host: &Driver<'_, H>,
    request: &StockingRequest,
) -> Result<Vec<StockingEvent>, SourceError> {
    let mut call = Call::get(request.url.clone()).with_cache(CacheHint {
        max_age: Some(STOCKING_MAX_AGE_S),
        ..CacheHint::default()
    });
    call.headers.clone_from(&request.headers);
    let fetched = match host.cached_response(&call).await {
        Some(hit) => hit,
        None => host
            .send(&call)
            .await
            .map_err(|e| SourceError::Transport(e.to_string()))?,
    };
    match fetched.status {
        200..=299 => request.dataset.parse(&fetched.body),
        404 => Err(SourceError::NotFound(None)),
        429 => Err(SourceError::RateLimited {
            retry_after_s: fetched.retry_after,
        }),
        status => Err(SourceError::Status {
            status,
            message: None,
        }),
    }
}

/// The New York DEC lists of what was stocked in the spot's water and what is planned there.
/// The requests name only a county and its waters, never a position. A spot with no stocked
/// water near is outside the lists' state: nothing is asked, and the error says so.
pub(crate) async fn fetch_stocking<H: FetchHost>(
    host: &Driver<'_, H>,
    spot: &Spot,
    water_name: Option<&str>,
) -> Stocking {
    let source = SourceId::NY_DEC_STOCKING;
    let mut out = Stocking::default();
    let catalogue = match Catalogue::bundled() {
        Ok(c) => c,
        Err(e) => {
            let e = SourceError::Parse(e.to_string());
            out.errors
                .push(source_error(source, "stocking catalogue", &e));
            return out;
        }
    };
    let waters = match stocked_waters(&catalogue, spot, water_name) {
        Ok(w) => w,
        Err(e) => {
            tracing::debug!(error = %e, "stocking lists not asked");
            out.errors.push(e.to_model(source, None));
            return out;
        }
    };
    // By county, in the order of each county's nearest water.
    let mut counties: Vec<(String, Vec<&str>)> = Vec::new();
    for p in &waters {
        let county = canonical_county(&p.county);
        match counties.iter_mut().find(|(c, _)| *c == county) {
            Some((_, names)) => names.push(&p.waterbody),
            None => counties.push((county, vec![&p.waterbody])),
        }
    }
    for (county, names) in &counties {
        for request in socrata_ny::requests(county, names) {
            match stocking_list(host, &request).await {
                Ok(events) => {
                    out.attribution = Some(socrata_ny::attribution());
                    out.events
                        .extend(events.into_iter().filter(|e| of_waters(e, &waters)));
                }
                Err(e) => {
                    tracing::warn!(%source, error = %e, "stocking list");
                    out.errors
                        .push(source_error(source.clone(), "stocking list", &e));
                }
            }
        }
    }
    out
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
    fn a_spot_is_matched_to_its_stocked_water_by_name() {
        use crate::testing::croton;

        let catalogue = Catalogue::bundled().expect("catalogue");
        let waters = |spot: &Spot, water: Option<&str>| -> Vec<(String, String)> {
            stocked_waters(&catalogue, spot, water)
                .expect("in New York")
                .into_iter()
                .map(|p| (p.county.clone(), p.waterbody.clone()))
                .collect()
        };
        let croton_river = vec![("Westchester".to_owned(), "Croton River".to_owned())];
        // The map's name of the spot's water, about 18 km from the river's catalogue point.
        assert_eq!(waters(&croton(), Some("Croton River")), croton_river);
        // The spot's own name when the map names nothing, never a part of it.
        let by_name = Spot {
            name: "croton  river".into(),
            ..croton()
        };
        assert_eq!(waters(&by_name, None), croton_river);
        assert_eq!(waters(&croton(), None), []);
        // A lake near the spot is not the spot's water unless the map puts the spot on it.
        assert_eq!(waters(&croton(), Some("New Croton Reservoir")), []);
        let amawalk = Some("Amawalk Reservoir");
        assert_eq!(
            waters(&croton(), amawalk),
            [("Westchester".to_owned(), "Amawalk Reservoir".to_owned())]
        );
    }

    #[test]
    fn events_are_kept_for_the_matched_waters_only() {
        let catalogue = Catalogue::bundled().expect("catalogue");
        let spot = crate::testing::croton();
        let waters = stocked_waters(&catalogue, &spot, Some("Croton River")).expect("waters");
        let event = |county: Option<&str>, water: &str| StockingEvent {
            waterbody: water.into(),
            county: county.map(Into::into),
            town: None,
            when: underbite_core::model::StockingWhen::DateText("April".into()),
            species_text: "Brown Trout".into(),
            number_estimate: None,
            size_m: None,
            planned: true,
            source: SourceId::NY_DEC_STOCKING,
        };
        assert!(of_waters(
            &event(Some("WESTCHESTER"), "croton  River"),
            &waters
        ));
        assert!(of_waters(&event(None, "Croton River"), &waters));
        assert!(!of_waters(&event(Some("Putnam"), "Croton River"), &waters));
        assert!(!of_waters(
            &event(Some("Westchester"), "Kensico Reservoir"),
            &waters
        ));
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
