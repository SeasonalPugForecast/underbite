//! Underbite's data sources, sans-IO: each source builds HTTP requests for a spot
//! or station and parses the response bytes into parts of a spot snapshot. Network
//! I/O lives only in the host (the CLI runtime, or `URLSessionHost` on iOS).
//!
//! Weather comes from whether's providers and session (`whether_providers`),
//! unchanged.

#[cfg(test)]
pub(crate) mod testutil;

pub mod access;
pub mod blooms;
pub mod coops;
pub mod dfo;
pub mod eccc;
pub mod encdirect;
pub mod erddap;
pub mod glerl;
pub mod hydro;
pub mod ncei_dem;
pub mod ndbc;
pub mod nwps;
pub mod nws_marine;
pub mod open_meteo_marine;
pub mod request;
pub mod reservoirs;
pub mod session;
pub mod source;
pub mod species;
pub mod statemaps;
pub mod stocking;
pub mod storms;
pub mod usgs;
