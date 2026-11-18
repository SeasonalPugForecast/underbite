//! Sharing: the `.underbite` file, its versioned envelope, location fuzz, shared species profiles
//! and map layer presets, and the merge of live-shared trips.

pub mod file;
pub mod merge;

pub use file::{
    COPERNICUS_LAYER_PREFIX, Document, FORMAT, LayersShare, MAX_BYTES, OnConflict, ProfileImport,
    ProfileShare, ShareError, SharedLayer, SpotShare, VERSION, decode, encode, fuzz,
};
