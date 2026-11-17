//! Trips: places, dates, species and method, and what the planner derives for them.

pub mod model;

pub use model::{
    ChecklistItem, Fuzz, MAX_TRIP_DAYS, Method, PlaceProblem, SpeciesId, SpeciesIdError, Trip,
    TripError, TripLimits, TripPlace,
};
