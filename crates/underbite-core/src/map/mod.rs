//! Map layers: tile addressing, the layer catalogue, gridded data and on-device raster rendering.
//!
//! Engine-neutral: nothing here knows a map engine; the displays draw what this module describes.

pub mod catalogue;
pub mod grid;
pub mod render;
pub mod tiles;
