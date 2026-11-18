//! Underbite's domain core: the spot snapshot model, station discovery, tide and
//! moon astronomy, derived measurements, trips, species, map layers and the view
//! layer, the only place that converts units and formats values.
//!
//! The weather part of a spot is whether's model (`whether_core`), reused as is.
//! Nothing here performs I/O or reads the clock: callers pass `now` explicitly.

pub mod alerts;
pub mod astro;
pub mod datum;
pub mod derive;
pub mod discover;
pub mod import;
pub mod log;
pub mod map;
pub mod model;
pub mod offline;
pub mod provenance;
pub mod regulations;
pub mod series;
pub mod share;
pub mod species;
pub mod time;
pub mod trip;
pub mod units;
pub mod view;
