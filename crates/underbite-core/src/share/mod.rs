//! Sharing: the `.underbite` file, its versioned envelope, location fuzz and shared species
//! profiles, and the merge of live-shared trips.

pub mod file;
pub mod merge;

pub use file::{
    Document, FORMAT, MAX_BYTES, OnConflict, ProfileImport, ProfileShare, ShareError, SpotShare,
    VERSION, decode, encode, fuzz,
};
