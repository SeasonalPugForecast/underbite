//! Underbite's configuration: sources, cards, units, limits, rivers, fishing
//! windows and map layers, serialized with serde and described by a JSON Schema,
//! versioned with migrations from older versions.

pub mod alerts;
pub mod cards;
pub mod export;
pub mod limits;
pub mod map;
pub mod migrate;
pub mod rivers;
pub mod schema;
pub mod sources;
pub mod units;
pub mod view_input;
pub mod windows;
