//! The host side of a fetch: HTTP transport, the response cache and the drive loop
//! that runs the sans-IO sessions.

pub(crate) mod cache;
pub(crate) mod env;
pub(crate) mod fetch;
pub(crate) mod transport;
