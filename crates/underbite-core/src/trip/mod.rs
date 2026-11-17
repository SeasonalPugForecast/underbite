//! Trips: places, dates, species and method, and what the planner derives for them.

pub mod gonogo;
pub mod history;
pub mod model;
pub mod windows;

pub use model::{
    ChecklistItem, Fuzz, MAX_TRIP_DAYS, Method, PlaceProblem, SpeciesId, SpeciesIdError, Trip,
    TripError, TripLimits, TripPlace,
};
