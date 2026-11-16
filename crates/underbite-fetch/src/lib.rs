//! Underbite's spot fetch: the weather, water and storm sessions for a spot, driven through a
//! [`FetchHost`] that does the I/O. The command-line tool and the iOS app run the same fetch.
//!
//! The configuration maps onto the sessions: the registry of water and marine sources a fetch
//! runs, the measurements a spot's water type and cards need, each measurement's configured
//! chain and limits, and the weather providers built from `[sources.weather]`.
//!
//! Discovery comes first: the station lists (CO-OPS, NDBC, USGS sites and NWPS gauges in a box
//! around the rounded spot), the hydrography around a river, lake or estuary spot (3DHP), and
//! the ranking with the spot's pins and exclusions. NDBC stations kept on their list flags alone
//! are confirmed against their files. Lists, rankings and observed coverage are stored through
//! the host; coordinates leave the device only rounded by the fetch context, and the exact spot
//! is used on the device (distances, snapping to the hydrography).
//!
//! Then the sessions run one after the other through the same host, one request at a time: the
//! water and marine session over the discovery plan, whether's weather session, the normals of
//! the latest USGS discharge or lake level for its day, and the storm-ahead check with nearby
//! hurricanes. The sky is computed for the spot's local days, and everything lands in one
//! `SpotSnapshot` with its credits and the errors of the sources that were passed over. The
//! snapshot and the call ledger are saved; every request is logged only through `redact_url`.
//!
//! Nothing here reads the clock, the environment or the file system, or opens a connection:
//! `now`, the options and the configuration are parameters, and the host does the rest.

mod discovery;
mod host;
mod hydrography;
mod lists;
mod normals;
mod registry;
mod sessions;
mod sky;
mod spot;
pub mod store;
mod storms;
mod water_type;

#[cfg(test)]
mod testing;

pub use discovery::{Discovery, discover, stations_near};
pub use host::{Call, FetchHost, Fetched, TransportError, send};
pub use registry::{chain_specs, measurement_limits, requirements};
pub use spot::{FetchOptions, SpotFetch, fetch_spot};

use underbite_core::model::SourceError as ModelSourceError;
use underbite_core::provenance::SourceId;
use underbite_providers::source::SourceError;

/// A snapshot error for `source` that concerns no single measurement.
fn source_error(source: SourceId, what: &str, error: &SourceError) -> ModelSourceError {
    let mut e = error.to_model(source, None);
    e.message = format!("{what}: {}", e.message);
    e
}
