//! Underbite's domain core: the spot snapshot model, station discovery, tide and
//! moon astronomy, derived measurements, trips, species, map layers and the view
//! layer, the only place that converts units and formats values.
//!
//! The weather part of a spot is whether's model (`whether_core`), reused as is.
//! Nothing here performs I/O or reads the clock: callers pass `now` explicitly.
