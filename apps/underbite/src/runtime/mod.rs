//! The host side of a fetch: the HTTP transport, the cache on disk and the host the fetch
//! library runs through.

pub(crate) mod cache;
pub(crate) mod env;
pub(crate) mod host;
pub(crate) mod transport;

#[cfg(test)]
mod live;
