//! The Rust core for the iOS app, through `UniFFI`: fetching a spot through the Swift host, the
//! view, the settings and the map.

uniffi::setup_scaffolding!();

pub mod fetch;
pub mod map;
pub mod settings;
pub mod view;
