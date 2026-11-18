//! The Rust core for the iOS app, through `UniFFI`: fetching a spot through the Swift host, the
//! view, the settings, the map, its depth, station pins and storms, and working offline.

uniffi::setup_scaffolding!();

pub mod alerts;
pub mod depth;
pub mod fetch;
pub mod import;
pub mod log;
pub mod map;
pub mod map_stations;
pub mod nearby;
pub mod offline;
pub mod settings;
pub mod share;
pub mod species;
pub mod storms;
pub mod trip;
pub mod view;
pub mod widgets;
