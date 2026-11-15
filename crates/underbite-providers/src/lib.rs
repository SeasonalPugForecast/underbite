//! Underbite's data sources, sans-IO: each source builds HTTP requests for a spot
//! or station and parses the response bytes into parts of a spot snapshot. Network
//! I/O lives only in the host (the CLI runtime, or `URLSessionHost` on iOS).
//!
//! Weather comes from whether's providers and session (`whether_providers`),
//! unchanged.

#[cfg(test)]
pub(crate) mod testutil;

pub mod coops;
pub mod ndbc;
pub mod nwps;
pub mod nws_marine;
pub mod open_meteo_marine;
pub mod request;
pub mod session;
pub mod source;
pub mod storms;
pub mod usgs;
