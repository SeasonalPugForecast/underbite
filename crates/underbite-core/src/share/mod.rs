//! Sharing: the `.underbite` file, its versioned envelope and location fuzz, and the merge of
//! live-shared trips.

pub mod file;
pub mod merge;

pub use file::{Document, FORMAT, MAX_BYTES, ShareError, SpotShare, VERSION, decode, encode, fuzz};
