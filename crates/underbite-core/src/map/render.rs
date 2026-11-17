//! Rendering grids into raster tiles: colour ramps, legends and current arrows.
//!
//! Pure and deterministic: no clock and no I/O. Values are in canonical units (°C, mg m⁻³, m/s,
//! metres); this module turns them into colours and positions, never into text. Legends are
//! numeric stops that the view labels.
//!
//! The ramps are sampled (33 of 256 entries each) from Fabio Crameri's Scientific colour maps,
//! which are perceptually uniform and readable with colour-vision deficiency. Colour map data
//! Copyright (c) 2020 Fabio Crameri, released under the MIT License.

use std::f64::consts::PI;
use std::fmt;

use crate::map::grid::{Cell, Grid, half_step, locate, locate_lon};
use crate::map::{Arrow, Bbox, GeoPoint, TileKey};

/// The largest tile edge [`render_tile`] draws, in pixels.
pub const MAX_TILE_SIZE: u32 = 1024;

/// The deepest zoom level a tile key may have.
pub const MAX_ZOOM: u8 = 30;

/// About how far apart [`arrows`] places current arrows on screen, in points of a 256-point tile.
pub const ARROW_SPACING_PX: f64 = 32.0;

/// The latitude where Web Mercator ends, in degrees.
const MERCATOR_MAX_LAT: f64 = 85.051_128_779_806_59;

/// A colour with straight (not premultiplied) alpha.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    /// Fully transparent black: no value.
    pub const TRANSPARENT: Self = Self {
        r: 0,
        g: 0,
        b: 0,
        a: 0,
    };

    /// An opaque colour.
    pub const fn opaque(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }
}

const fn rgb(r: u8, g: u8, b: u8) -> Rgba {
    Rgba::opaque(r, g, b)
}

/// Whether a ramp runs from one end to the other or out from a neutral middle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RampKind {
    /// Lightness changes steadily from the low end to the high end.
    Sequential,
    /// Light in the middle and dark at both ends, for values above and below a centre.
    Diverging,
}

/// A colour ramp: evenly spaced colour stops from the low end of a range to the high end.
#[derive(Debug, PartialEq)]
pub struct Ramp {
    /// The id settings name the ramp by, such as `thermal`.
    pub id: &'static str,
    pub kind: RampKind,
    /// The colours at evenly spaced positions from 0 (low) to 1 (high); colours between two stops
    /// are interpolated.
    pub stops: &'static [Rgba],
    /// The range the ramp spans for the quantity it is meant for, in canonical units.
    pub default_range: RampRange,
}

impl Ramp {
    /// The colour at a position from 0 (low end) to 1 (high end); positions outside are clamped.
    /// Transparent for a position that is not a number or a ramp without stops.
    pub fn colour_at(&self, position: f64) -> Rgba {
        let Some(last) = self.stops.len().checked_sub(1) else {
            return Rgba::TRANSPARENT;
        };
        if position.is_nan() {
            return Rgba::TRANSPARENT;
        }
        let along = position.clamp(0.0, 1.0) * last as f64;
        let below = (along.floor() as usize).min(last);
        let above = (below + 1).min(last);
        let frac = along - below as f64;
        let (from, to) = (self.stops[below], self.stops[above]);
        let mix = |low: u8, high: u8| -> u8 {
            let mixed = f64::from(low) + (f64::from(high) - f64::from(low)) * frac;
            mixed.round().clamp(0.0, 255.0) as u8
        };
        Rgba {
            r: mix(from.r, to.r),
            g: mix(from.g, to.g),
            b: mix(from.b, to.b),
            a: mix(from.a, to.a),
        }
    }

    /// The colour of a value in canonical units over a range; none for a value that is not a
    /// number or a range that cannot be drawn. Values beyond the range take the end colours.
    pub fn colour(&self, range: RampRange, value: f64) -> Option<Rgba> {
        range.position(value).map(|t| self.colour_at(t))
    }
}

/// How values are spread along a ramp.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scale {
    /// Equal steps of value are equal steps along the ramp.
    Linear,
    /// Equal ratios of value are equal steps along the ramp (chlorophyll: log10 of mg m⁻³).
    Log10,
}

/// The values a ramp spans, low end to high end, in canonical units, and how they are spread.
///
/// The scale belongs to the quantity, not to the colours: a chlorophyll layer stays logarithmic
/// whichever ramp it is drawn with. Settings store only the low and high values;
/// [`RampRange::with_bounds`] puts them on a layer's scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RampRange {
    pub low: f64,
    pub high: f64,
    pub scale: Scale,
}

impl RampRange {
    /// A linear range.
    pub const fn linear(low: f64, high: f64) -> Self {
        Self {
            low,
            high,
            scale: Scale::Linear,
        }
    }

    /// A logarithmic range; both ends must be above zero.
    pub const fn log10(low: f64, high: f64) -> Self {
        Self {
            low,
            high,
            scale: Scale::Log10,
        }
    }

    /// The same scale with other ends, such as a range chosen in settings.
    #[must_use]
    pub const fn with_bounds(self, low: f64, high: f64) -> Self {
        Self {
            low,
            high,
            scale: self.scale,
        }
    }

    /// Whether the range can be drawn: finite, low below high, and above zero on a log scale.
    pub fn is_valid(&self) -> bool {
        self.low.is_finite()
            && self.high.is_finite()
            && self.low < self.high
            && (self.scale == Scale::Linear || self.low > 0.0)
    }

    /// The ends on the ramp's axis (log10 of the values on a log scale).
    fn axis(&self) -> Option<(f64, f64)> {
        if !self.is_valid() {
            return None;
        }
        Some(match self.scale {
            Scale::Linear => (self.low, self.high),
            Scale::Log10 => (self.low.log10(), self.high.log10()),
        })
    }

    /// Where a value falls along the ramp, 0 to 1, clamped at the ends. None for a value that is
    /// not a number or a range that cannot be drawn. On a log scale a value at or below zero
    /// takes the low end.
    pub fn position(&self, value: f64) -> Option<f64> {
        let (lo, hi) = self.axis()?;
        if value.is_nan() {
            return None;
        }
        let x = match self.scale {
            Scale::Linear => value,
            Scale::Log10 if value > 0.0 => value.log10(),
            Scale::Log10 => return Some(0.0),
        };
        Some(((x - lo) / (hi - lo)).clamp(0.0, 1.0))
    }

    /// The value at a position along the ramp, 0 to 1; none for a range that cannot be drawn.
    pub fn value_at(&self, position: f64) -> Option<f64> {
        let (lo, hi) = self.axis()?;
        let x = lo + (hi - lo) * position.clamp(0.0, 1.0);
        Some(match self.scale {
            Scale::Linear => x,
            Scale::Log10 => 10f64.powf(x),
        })
    }
}

/// Scientific colour map `batlow`.
#[rustfmt::skip]
const BATLOW: [Rgba; 33] = [
    rgb(0x01, 0x19, 0x59), rgb(0x08, 0x25, 0x5b), rgb(0x0d, 0x31, 0x5d),
    rgb(0x0f, 0x3b, 0x5f), rgb(0x11, 0x43, 0x60), rgb(0x13, 0x4b, 0x61),
    rgb(0x17, 0x52, 0x62), rgb(0x1b, 0x59, 0x62), rgb(0x22, 0x60, 0x61),
    rgb(0x2b, 0x65, 0x5e), rgb(0x35, 0x6a, 0x59), rgb(0x41, 0x6f, 0x53),
    rgb(0x4d, 0x73, 0x4d), rgb(0x5a, 0x77, 0x45), rgb(0x67, 0x7b, 0x3e),
    rgb(0x74, 0x7e, 0x38), rgb(0x82, 0x82, 0x31), rgb(0x8f, 0x86, 0x2d),
    rgb(0x9f, 0x89, 0x2b), rgb(0xaf, 0x8d, 0x2e), rgb(0xbe, 0x90, 0x35),
    rgb(0xcd, 0x92, 0x3f), rgb(0xdb, 0x95, 0x4b), rgb(0xe7, 0x98, 0x5a),
    rgb(0xf1, 0x9d, 0x6b), rgb(0xf8, 0xa2, 0x7e), rgb(0xfc, 0xa8, 0x90),
    rgb(0xfd, 0xae, 0xa2), rgb(0xfd, 0xb4, 0xb4), rgb(0xfd, 0xba, 0xc4),
    rgb(0xfc, 0xbf, 0xd6), rgb(0xfb, 0xc6, 0xe8), rgb(0xfa, 0xcc, 0xfa),
];

/// Scientific colour map `imola`.
#[rustfmt::skip]
const IMOLA: [Rgba; 33] = [
    rgb(0x1a, 0x33, 0xb3), rgb(0x1d, 0x39, 0xb0), rgb(0x20, 0x3e, 0xad),
    rgb(0x23, 0x44, 0xaa), rgb(0x25, 0x49, 0xa8), rgb(0x28, 0x4e, 0xa5),
    rgb(0x2b, 0x53, 0xa2), rgb(0x2d, 0x59, 0xa0), rgb(0x30, 0x5e, 0x9d),
    rgb(0x33, 0x63, 0x9a), rgb(0x37, 0x68, 0x96), rgb(0x3b, 0x6c, 0x92),
    rgb(0x3f, 0x71, 0x8e), rgb(0x44, 0x76, 0x8a), rgb(0x48, 0x7b, 0x86),
    rgb(0x4e, 0x80, 0x82), rgb(0x54, 0x86, 0x7f), rgb(0x5a, 0x8c, 0x7d),
    rgb(0x61, 0x93, 0x7b), rgb(0x68, 0x9b, 0x79), rgb(0x70, 0xa3, 0x77),
    rgb(0x78, 0xab, 0x75), rgb(0x80, 0xb3, 0x73), rgb(0x88, 0xbb, 0x71),
    rgb(0x91, 0xc3, 0x6f), rgb(0x99, 0xcc, 0x6d), rgb(0xa3, 0xd5, 0x6b),
    rgb(0xaf, 0xde, 0x69), rgb(0xbd, 0xe6, 0x67), rgb(0xcd, 0xed, 0x66),
    rgb(0xdd, 0xf4, 0x66), rgb(0xee, 0xf9, 0x66), rgb(0xff, 0xff, 0x66),
];

/// Scientific colour map `lajolla`.
#[rustfmt::skip]
const LAJOLLA: [Rgba; 33] = [
    rgb(0xff, 0xff, 0xcc), rgb(0xfe, 0xfa, 0xbe), rgb(0xfd, 0xf5, 0xaf),
    rgb(0xfc, 0xef, 0xa1), rgb(0xfb, 0xe9, 0x92), rgb(0xf9, 0xe1, 0x84),
    rgb(0xf7, 0xd8, 0x75), rgb(0xf4, 0xce, 0x69), rgb(0xf2, 0xc3, 0x60),
    rgb(0xef, 0xb8, 0x59), rgb(0xed, 0xae, 0x56), rgb(0xeb, 0xa4, 0x54),
    rgb(0xe9, 0x9b, 0x53), rgb(0xe7, 0x91, 0x52), rgb(0xe5, 0x88, 0x51),
    rgb(0xe2, 0x7e, 0x50), rgb(0xde, 0x74, 0x4f), rgb(0xd9, 0x6b, 0x4e),
    rgb(0xd0, 0x61, 0x4c), rgb(0xc5, 0x57, 0x4a), rgb(0xb8, 0x4f, 0x47),
    rgb(0xaa, 0x49, 0x44), rgb(0x9c, 0x44, 0x3f), rgb(0x8d, 0x40, 0x3a),
    rgb(0x7f, 0x3b, 0x34), rgb(0x71, 0x37, 0x2e), rgb(0x63, 0x33, 0x28),
    rgb(0x56, 0x2f, 0x21), rgb(0x49, 0x2a, 0x1b), rgb(0x3c, 0x26, 0x15),
    rgb(0x30, 0x22, 0x10), rgb(0x24, 0x1e, 0x09), rgb(0x1a, 0x1a, 0x01),
];

/// Scientific colour map `devon`, dark to light as published.
#[rustfmt::skip]
const DEVON: [Rgba; 33] = [
    rgb(0x2c, 0x1a, 0x4c), rgb(0x2b, 0x21, 0x54), rgb(0x2a, 0x29, 0x5b),
    rgb(0x29, 0x30, 0x63), rgb(0x29, 0x38, 0x6a), rgb(0x28, 0x3f, 0x72),
    rgb(0x27, 0x47, 0x7b), rgb(0x27, 0x50, 0x84), rgb(0x29, 0x58, 0x8f),
    rgb(0x2c, 0x5f, 0x9a), rgb(0x32, 0x65, 0xa6), rgb(0x39, 0x6b, 0xb1),
    rgb(0x42, 0x72, 0xbc), rgb(0x4f, 0x79, 0xc6), rgb(0x5f, 0x80, 0xcf),
    rgb(0x6f, 0x88, 0xd7), rgb(0x7e, 0x8f, 0xdd), rgb(0x8c, 0x95, 0xe2),
    rgb(0x9a, 0x9b, 0xe7), rgb(0xa6, 0xa3, 0xeb), rgb(0xaf, 0xaa, 0xee),
    rgb(0xb7, 0xb1, 0xf0), rgb(0xbe, 0xb8, 0xf2), rgb(0xc4, 0xbf, 0xf3),
    rgb(0xca, 0xc6, 0xf4), rgb(0xd1, 0xcd, 0xf5), rgb(0xd7, 0xd4, 0xf7),
    rgb(0xde, 0xdb, 0xf8), rgb(0xe4, 0xe2, 0xfa), rgb(0xeb, 0xe9, 0xfb),
    rgb(0xf2, 0xf0, 0xfc), rgb(0xf8, 0xf8, 0xfe), rgb(0xff, 0xff, 0xff),
];

/// `devon` light to dark: shallow water light, deep water dark.
const DEVON_REVERSED: [Rgba; 33] = reversed(DEVON);

/// Scientific colour map `vik`.
#[rustfmt::skip]
const VIK: [Rgba; 33] = [
    rgb(0x00, 0x12, 0x61), rgb(0x02, 0x1f, 0x69), rgb(0x02, 0x2b, 0x71),
    rgb(0x02, 0x37, 0x79), rgb(0x03, 0x44, 0x81), rgb(0x05, 0x51, 0x89),
    rgb(0x0c, 0x5e, 0x92), rgb(0x1c, 0x6d, 0x9c), rgb(0x30, 0x7d, 0xa6),
    rgb(0x48, 0x8d, 0xb2), rgb(0x61, 0x9e, 0xbd), rgb(0x7a, 0xae, 0xc8),
    rgb(0x94, 0xbe, 0xd2), rgb(0xad, 0xcd, 0xdd), rgb(0xc6, 0xdb, 0xe6),
    rgb(0xde, 0xe6, 0xe9), rgb(0xec, 0xe5, 0xe0), rgb(0xee, 0xdb, 0xd0),
    rgb(0xe9, 0xcc, 0xba), rgb(0xe3, 0xbc, 0xa5), rgb(0xdc, 0xac, 0x90),
    rgb(0xd6, 0x9d, 0x7c), rgb(0xcf, 0x8e, 0x68), rgb(0xc9, 0x80, 0x56),
    rgb(0xc3, 0x72, 0x43), rgb(0xbd, 0x64, 0x31), rgb(0xb3, 0x53, 0x1f),
    rgb(0xa5, 0x40, 0x0f), rgb(0x94, 0x2f, 0x06), rgb(0x83, 0x21, 0x06),
    rgb(0x74, 0x15, 0x06), rgb(0x67, 0x0a, 0x07), rgb(0x59, 0x00, 0x08),
];

const fn reversed<const N: usize>(stops: [Rgba; N]) -> [Rgba; N] {
    let mut out = stops;
    let mut i = 0;
    while i < N {
        out[i] = stops[N - 1 - i];
        i += 1;
    }
    out
}

/// Sea and lake surface temperature (`batlow`, dark blue through green and orange to pink),
/// 0 to 30 °C.
pub static THERMAL: Ramp = Ramp {
    id: "thermal",
    kind: RampKind::Sequential,
    stops: &BATLOW,
    default_range: RampRange::linear(0.0, 30.0),
};

/// Chlorophyll (`imola`, blue through green to yellow-green) on a log10 scale, 0.03 to
/// 30 mg m⁻³.
pub static ALGAE: Ramp = Ramp {
    id: "algae",
    kind: RampKind::Sequential,
    stops: &IMOLA,
    default_range: RampRange::log10(0.03, 30.0),
};

/// Current speed (`lajolla`, pale yellow through orange to dark brown), 0 to 1.5 m/s.
pub static SPEED: Ramp = Ramp {
    id: "speed",
    kind: RampKind::Sequential,
    stops: &LAJOLLA,
    default_range: RampRange::linear(0.0, 1.5),
};

/// Water clarity as the diffuse attenuation coefficient Kd490 (`lajolla` again, pale yellow for
/// clear water through orange to dark brown for sediment-laden water) on a log10 scale, 0.02 to
/// 3 m⁻¹. The same stops as [`SPEED`], kept apart so each layer names its own ramp and range.
pub static TURBIDITY: Ramp = Ramp {
    id: "turbidity",
    kind: RampKind::Sequential,
    stops: &LAJOLLA,
    default_range: RampRange::log10(0.02, 3.0),
};

/// Depth below the surface (`devon` reversed, white shallows to dark blue-violet depths),
/// 0 to 100 m.
pub static DEPTH: Ramp = Ramp {
    id: "depth",
    kind: RampKind::Sequential,
    stops: &DEVON_REVERSED,
    default_range: RampRange::linear(0.0, 100.0),
};

/// Departures either side of a centre (`vik`, blue through near-white to red-brown), such as a
/// temperature anomaly, -3 to 3 °C.
pub static DIVERGING: Ramp = Ramp {
    id: "diverging",
    kind: RampKind::Diverging,
    stops: &VIK,
    default_range: RampRange::linear(-3.0, 3.0),
};

/// Every ramp, in the order a picker lists them.
pub static RAMPS: [&Ramp; 6] = [&THERMAL, &ALGAE, &TURBIDITY, &SPEED, &DEPTH, &DIVERGING];

/// The ramp with an id, if this version has it.
pub fn ramp(id: &str) -> Option<&'static Ramp> {
    RAMPS.iter().copied().find(|r| r.id == id)
}

/// One stop of a legend.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LegendStop {
    /// The value at this stop, in the range's canonical units.
    pub value: f64,
    /// How far along the legend bar the stop sits, 0 (low end) to 1 (high end); evenly spaced on
    /// the range's scale, so a gradient through the stops reproduces the ramp.
    pub fraction: f64,
    pub colour: Rgba,
}

/// A legend for a ramp over a range: one stop per ramp colour, values ascending, for the view
/// to label. Empty when the range cannot be drawn.
pub fn legend(ramp: &Ramp, range: RampRange) -> Vec<LegendStop> {
    let Some(last) = ramp.stops.len().checked_sub(1) else {
        return Vec::new();
    };
    if !range.is_valid() {
        return Vec::new();
    }
    ramp.stops
        .iter()
        .enumerate()
        .filter_map(|(i, &colour)| {
            let fraction = if last == 0 {
                0.0
            } else {
                i as f64 / last as f64
            };
            range.value_at(fraction).map(|value| LegendStop {
                value,
                fraction,
                colour,
            })
        })
        .collect()
}

/// A tile could not be rendered.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RenderError {
    /// The tile edge is zero or larger than [`MAX_TILE_SIZE`].
    #[error("a tile edge of {0} pixels is outside 1 to 1024")]
    BadSize(u32),
    /// The zoom is beyond [`MAX_ZOOM`] or the column or row is outside the zoom level.
    #[error("tile {0} is not a Web Mercator tile")]
    BadTile(TileLabel),
    /// The range is not finite, empty, or not above zero on a log scale.
    #[error("the colour range cannot be drawn")]
    BadRange,
    /// The PNG encoder refused the image.
    #[error("PNG encoding failed: {0}")]
    Png(String),
}

/// A tile key as `z/x/y`, for error messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TileLabel(pub TileKey);

impl fmt::Display for TileLabel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}/{}", self.0.z, self.0.x, self.0.y)
    }
}

/// Where a tile's columns and rows fall on one grid's axes.
struct Located<'a> {
    grid: &'a Grid,
    cols: Vec<Option<Cell>>,
    rows: Vec<Option<Cell>>,
}

fn strictly_ascending(axis: &[f64]) -> bool {
    axis.iter().all(|v| v.is_finite()) && axis.windows(2).all(|w| w[0] < w[1])
}

/// Whether a grid keeps the promises [`Grid::new`] makes (its fields are public).
fn usable(grid: &Grid) -> bool {
    !grid.lats.is_empty()
        && !grid.lons.is_empty()
        && grid.values.len() == grid.lats.len() * grid.lons.len()
        && strictly_ascending(&grid.lats)
        && strictly_ascending(&grid.lons)
}

/// The value of a grid at a point: [`Grid::sample_bilinear`], the one sampling rule the tile and
/// the point inspector share.
fn sample(grid: &Grid, at: GeoPoint) -> Option<f64> {
    grid.sample_bilinear(at).map(f64::from)
}

/// The latitude of a Web Mercator position, `y` from 0 (north edge) to 1 (south edge).
fn mercator_lat(y: f64) -> f64 {
    (PI * (1.0 - 2.0 * y)).sinh().atan().to_degrees()
}

/// The Web Mercator position of a latitude, 0 (north edge) to 1 (south edge).
fn mercator_y(lat: f64) -> f64 {
    let phi = lat.clamp(-MERCATOR_MAX_LAT, MERCATOR_MAX_LAT).to_radians();
    (1.0 - phi.tan().asinh() / PI) / 2.0
}

fn check_tile(tile: TileKey, size: u32) -> Result<(), RenderError> {
    if size == 0 || size > MAX_TILE_SIZE {
        return Err(RenderError::BadSize(size));
    }
    let count = 1u64 << tile.z.min(MAX_ZOOM);
    if tile.z > MAX_ZOOM || u64::from(tile.x) >= count || u64::from(tile.y) >= count {
        return Err(RenderError::BadTile(TileLabel(tile)));
    }
    Ok(())
}

/// Renders grids into a square tile of `size` × `size` straight-alpha RGBA pixels, row by row
/// from the north-west corner.
///
/// Each pixel takes the value [`Grid::sample_bilinear`] gives for the first grid that has one at
/// the pixel centre's latitude and longitude (Web Mercator inverted per row and column) and the
/// ramp's colour for it: the point inspector reads the same value the tile shows. A pixel is
/// transparent outside every grid (a grid reaches half a step past its outermost points, so
/// blocks side by side join), and where the nearest grid point has no value, so no colour spreads
/// from the coast onto land or into cloud gaps.
///
/// # Errors
/// [`RenderError`] for a size beyond 1 to [`MAX_TILE_SIZE`], a tile key outside its zoom level,
/// or a range that cannot be drawn.
pub fn render_rgba(
    grids: &[Grid],
    ramp: &Ramp,
    range: RampRange,
    tile: TileKey,
    size: u32,
) -> Result<Vec<u8>, RenderError> {
    check_tile(tile, size)?;
    if !range.is_valid() {
        return Err(RenderError::BadRange);
    }
    let edge = size as usize;
    let world = f64::from(size) * (1u64 << tile.z) as f64;
    let origin_x = f64::from(tile.x) * f64::from(size);
    let origin_y = f64::from(tile.y) * f64::from(size);
    let lons: Vec<f64> = (0..size)
        .map(|px| (origin_x + f64::from(px) + 0.5) / world * 360.0 - 180.0)
        .collect();
    let lats: Vec<f64> = (0..size)
        .map(|py| mercator_lat((origin_y + f64::from(py) + 0.5) / world))
        .collect();

    // Per grid, where each column and row of the tile falls on its axes.
    let located: Vec<Located<'_>> = grids
        .iter()
        .filter(|g| usable(g))
        .map(|g| {
            let half = half_step(g.source.native_step_deg);
            let cols = lons
                .iter()
                .map(|&lon| locate_lon(&g.lons, lon, half))
                .collect();
            let rows = lats.iter().map(|&lat| locate(&g.lats, lat, half)).collect();
            Located {
                grid: g,
                cols,
                rows,
            }
        })
        .collect();

    let mut out = vec![0u8; edge * edge * 4];
    for (py, line) in out.chunks_exact_mut(edge * 4).enumerate() {
        for (px, pixel) in line.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let value = located.iter().find_map(|l| {
                let row = l.rows.get(py).copied().flatten()?;
                let col = l.cols.get(px).copied().flatten()?;
                l.grid.sample_cell(row, col).map(f64::from)
            });
            let colour = value
                .and_then(|v| ramp.colour(range, v))
                .unwrap_or(Rgba::TRANSPARENT);
            *pixel = [colour.r, colour.g, colour.b, colour.a];
        }
    }
    Ok(out)
}

/// Encodes straight-alpha RGBA pixels, row by row, as an 8-bit RGBA PNG.
///
/// # Errors
/// [`RenderError::BadSize`] when the pixels do not fill `width` × `height`, and
/// [`RenderError::Png`] when the encoder refuses them.
pub fn encode_png(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>, RenderError> {
    let expected = u64::from(width) * u64::from(height) * 4;
    if width == 0 || height == 0 || rgba.len() as u64 != expected {
        return Err(RenderError::BadSize(width));
    }
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast);
    let png_error = |e: png::EncodingError| RenderError::Png(e.to_string());
    let mut writer = encoder.write_header().map_err(png_error)?;
    writer.write_image_data(rgba).map_err(png_error)?;
    writer.finish().map_err(png_error)?;
    Ok(bytes)
}

/// Renders grids into a PNG tile (8-bit RGBA, straight alpha) for the map: [`render_rgba`]
/// then [`encode_png`].
///
/// # Errors
/// As [`render_rgba`] and [`encode_png`].
pub fn render_tile(
    grids: &[Grid],
    ramp: &Ramp,
    range: RampRange,
    tile: TileKey,
    size: u32,
) -> Result<Vec<u8>, RenderError> {
    let rgba = render_rgba(grids, ramp, range, tile, size)?;
    encode_png(&rgba, size, size)
}

/// The direction a flow goes toward, degrees clockwise from north in `[0, 360)`, from its
/// eastward and northward parts.
fn toward_deg(u: f64, v: f64) -> f64 {
    let deg = u.atan2(v).to_degrees().rem_euclid(360.0);
    if deg >= 360.0 { 0.0 } else { deg }
}

/// The mean step of an ascending axis; none for a single point.
fn mean_step(axis: &[f64]) -> Option<f64> {
    let (&first, &last) = (axis.first()?, axis.last()?);
    let gaps = axis.len().checked_sub(1).filter(|&n| n > 0)?;
    let step = (last - first) / gaps as f64;
    (step.is_finite() && step > 0.0).then_some(step)
}

/// How many grid steps apart arrows go so that they land about [`ARROW_SPACING_PX`] apart, given
/// one step's length on screen.
fn stride(step_px: f64) -> i64 {
    if step_px.is_finite() && step_px > 0.0 {
        ((ARROW_SPACING_PX / step_px).round() as i64).max(1)
    } else {
        1
    }
}

/// Whether a coordinate is on the thinned lattice: its index counted from 0 in whole steps is a
/// multiple of the stride, so the same points are kept whichever part of the grid is in view.
fn on_lattice(x: f64, step: Option<f64>, every: i64) -> bool {
    match step {
        Some(step) if every > 1 => ((x / step).round() as i64).rem_euclid(every) == 0,
        _ => true,
    }
}

/// Current arrows for a view: grid points inside `bbox`, thinned to about one per
/// [`ARROW_SPACING_PX`] on screen at `zoom` (fractional zoom levels of 256-point tiles).
///
/// `u` holds the eastward part of the flow and `v` the northward part (`water_u`, `water_v`), in
/// m/s. Thinning keeps every n-th row and column of the grid, n chosen per axis from the grid
/// step's length on screen (Web Mercator rows measured at the middle of the box), and counted
/// from whole multiples of the step so arrows stay put while the map pans. Where the grid is
/// already sparser than the spacing every point is kept. Speed is `sqrt(u² + v²)`; the
/// direction is the one the water flows toward, `atan2(u, v)` in degrees clockwise from north.
/// Points where either part is missing are skipped. When the two grids' axes differ, `v` is
/// sampled bilinearly at `u`'s points. Arrows come row by row from the north, west to east;
/// none for a box or zoom that is not finite.
pub fn arrows(u: &Grid, v: &Grid, bbox: Bbox, zoom: f64) -> Vec<Arrow> {
    let finite = [bbox.west, bbox.south, bbox.east, bbox.north, zoom]
        .iter()
        .all(|x| x.is_finite());
    if !finite || bbox.west > bbox.east || bbox.south > bbox.north || !usable(u) || !usable(v) {
        return Vec::new();
    }
    let same_axes = u.lats == v.lats && u.lons == v.lons;
    let world_px = 256.0 * 2f64.powf(zoom.clamp(0.0, f64::from(MAX_ZOOM)));

    let lon_step = mean_step(&u.lons);
    let lon_every = stride(lon_step.map_or(0.0, |s| s / 360.0 * world_px));
    let lat_step = mean_step(&u.lats);
    let mid = f64::midpoint(bbox.south, bbox.north);
    let lat_every = stride(lat_step.map_or(0.0, |s| {
        (mercator_y(mid - s / 2.0) - mercator_y(mid + s / 2.0)) * world_px
    }));

    let width = u.lons.len();
    let mut out = Vec::new();
    for (i, &lat) in u.lats.iter().enumerate().rev() {
        if lat < bbox.south || lat > bbox.north || !on_lattice(lat, lat_step, lat_every) {
            continue;
        }
        for (j, &lon) in u.lons.iter().enumerate() {
            if lon < bbox.west || lon > bbox.east || !on_lattice(lon, lon_step, lon_every) {
                continue;
            }
            let Some(&east) = u.values.get(i * width + j) else {
                continue;
            };
            let north = if same_axes {
                v.values.get(i * width + j).map(|&n| f64::from(n))
            } else {
                sample(v, GeoPoint { lat, lon })
            };
            let (east, Some(north)) = (f64::from(east), north) else {
                continue;
            };
            if !east.is_finite() || !north.is_finite() {
                continue;
            }
            out.push(Arrow {
                at: GeoPoint { lat, lon },
                speed_ms: east.hypot(north) as f32,
                toward_deg: toward_deg(east, north) as f32,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::{ErddapNode, GridSource};
    use crate::time::UnixTime;

    fn source(dataset: &str, variables: &[&str], step: f64) -> GridSource {
        GridSource {
            node: ErddapNode::Pfeg,
            dataset: dataset.to_owned(),
            variables: variables.iter().map(|&v| v.to_owned()).collect(),
            altitude_axis: false,
            native_step_deg: step,
        }
    }

    fn fixture(name: &str) -> serde_json::Value {
        let path = format!(
            "{}/../../tests/fixtures/raw/erddap/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        let text = std::fs::read_to_string(&path).unwrap();
        serde_json::from_str(&text).unwrap()
    }

    /// Grids from an ERDDAP `.json` table, one per value column after time, latitude, longitude.
    fn grids_from_table(table: &serde_json::Value, src: &GridSource) -> Vec<Grid> {
        let rows = table["table"]["rows"].as_array().unwrap();
        let mut lats: Vec<f64> = rows.iter().map(|r| r[1].as_f64().unwrap()).collect();
        let mut lons: Vec<f64> = rows.iter().map(|r| r[2].as_f64().unwrap()).collect();
        lats.sort_by(f64::total_cmp);
        lats.dedup();
        lons.sort_by(f64::total_cmp);
        lons.dedup();
        let columns = rows[0].as_array().unwrap().len() - 3;
        (0..columns)
            .map(|c| {
                let mut values = vec![f32::NAN; lats.len() * lons.len()];
                for r in rows {
                    let i = lats.binary_search_by(|a| a.total_cmp(&r[1].as_f64().unwrap()));
                    let j = lons.binary_search_by(|a| a.total_cmp(&r[2].as_f64().unwrap()));
                    if let Some(v) = r[3 + c].as_f64() {
                        values[i.unwrap() * lons.len() + j.unwrap()] = v as f32;
                    }
                }
                let unit = table["table"]["columnUnits"][3 + c].as_str().unwrap();
                Grid::new(
                    src.clone(),
                    UnixTime(1_794_042_000),
                    lats.clone(),
                    lons.clone(),
                    values,
                    unit,
                )
                .unwrap()
            })
            .collect()
    }

    fn mur() -> Grid {
        let src = source("jplMURSST41", &["analysed_sst"], 0.01);
        grids_from_table(&fixture("jplmursst41-sst-subset-2026-11-07.json"), &src)
            .pop()
            .unwrap()
    }

    fn uniform(value: f32, lats: Vec<f64>, lons: Vec<f64>) -> Grid {
        let n = lats.len() * lons.len();
        Grid::new(
            source("synthetic", &["v"], 0.1),
            UnixTime(0),
            lats,
            lons,
            vec![value; n],
            "m s-1",
        )
        .unwrap()
    }

    fn steps(from: f64, step: f64, count: usize) -> Vec<f64> {
        (0..count).map(|i| from + step * i as f64).collect()
    }

    /// The pixel of a tile that covers a point.
    fn pixel_of(tile: TileKey, size: u32, at: GeoPoint) -> (usize, usize) {
        let world = f64::from(size) * f64::from(1u32 << tile.z);
        let x = (at.lon + 180.0) / 360.0 * world - f64::from(tile.x * size);
        let y = mercator_y(at.lat) * world - f64::from(tile.y * size);
        (x.floor() as usize, y.floor() as usize)
    }

    fn alpha(rgba: &[u8], size: u32, (x, y): (usize, usize)) -> u8 {
        rgba[(y * size as usize + x) * 4 + 3]
    }

    fn fnv1a(bytes: &[u8]) -> u64 {
        bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| {
            (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
        })
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    const MUR_TILE: TileKey = TileKey { z: 7, x: 37, y: 48 };

    #[test]
    fn ramps_are_found_by_id_and_have_distinct_ids() {
        for r in RAMPS {
            assert_eq!(ramp(r.id), Some(r));
            assert!(r.stops.len() >= 2);
            assert!(r.default_range.is_valid(), "{}", r.id);
        }
        let mut ids: Vec<&str> = RAMPS.iter().map(|r| r.id).collect();
        ids.dedup();
        assert_eq!(ids.len(), RAMPS.len());
        assert_eq!(ramp("rainbow"), None);
        assert_eq!(ALGAE.default_range.scale, Scale::Log10);
        assert_eq!(DIVERGING.kind, RampKind::Diverging);
    }

    /// CIE L* of an sRGB colour.
    fn lightness(c: Rgba) -> f64 {
        let lin = |v: u8| {
            let s = f64::from(v) / 255.0;
            if s <= 0.040_45 {
                s / 12.92
            } else {
                ((s + 0.055) / 1.055).powf(2.4)
            }
        };
        let y = 0.0722f64.mul_add(lin(c.b), 0.2126f64.mul_add(lin(c.r), 0.7152 * lin(c.g)));
        if y > 216.0 / 24389.0 {
            116.0f64.mul_add(y.cbrt(), -16.0)
        } else {
            24389.0 / 27.0 * y
        }
    }

    #[test]
    fn sequential_ramps_change_lightness_steadily_and_diverging_peaks_in_the_middle() {
        for r in RAMPS {
            let l: Vec<f64> = r.stops.iter().map(|&c| lightness(c)).collect();
            match r.kind {
                RampKind::Sequential => {
                    let rising = l.windows(2).all(|w| w[1] > w[0]);
                    let falling = l.windows(2).all(|w| w[1] < w[0]);
                    assert!(rising || falling, "{}: {l:?}", r.id);
                }
                RampKind::Diverging => {
                    let mid = l.len() / 2;
                    assert!(l[..=mid].windows(2).all(|w| w[1] > w[0] - 1.0), "{l:?}");
                    assert!(l[mid..].windows(2).all(|w| w[1] < w[0] + 1.0), "{l:?}");
                }
            }
        }
        // Shallow water is drawn light and deep water dark.
        assert!(lightness(DEPTH.stops[0]) > lightness(DEPTH.stops[32]));
    }

    #[test]
    fn colours_interpolate_between_stops_and_clamp_at_the_ends() {
        let r = &THERMAL;
        assert_eq!(r.colour_at(0.0), r.stops[0]);
        assert_eq!(r.colour_at(1.0), r.stops[32]);
        assert_eq!(r.colour_at(-4.0), r.stops[0]);
        assert_eq!(r.colour_at(9.0), r.stops[32]);
        assert_eq!(r.colour_at(f64::NAN), Rgba::TRANSPARENT);
        assert_eq!(r.colour_at(0.5), r.stops[16]);
        let between = r.colour_at(0.5 / 32.0);
        assert_eq!(between, Rgba::opaque(0x05, 0x1f, 0x5a));
        let range = RampRange::linear(10.0, 20.0);
        assert_eq!(r.colour(range, 15.0), Some(r.stops[16]));
        assert_eq!(r.colour(range, 99.0), Some(r.stops[32]));
        assert_eq!(r.colour(range, f64::NAN), None);
        assert_eq!(r.colour(RampRange::linear(1.0, 1.0), 1.0), None);
    }

    #[test]
    fn chlorophyll_maps_log10_of_its_values() {
        let range = ALGAE.default_range.with_bounds(0.1, 10.0);
        assert_eq!(range.scale, Scale::Log10);
        assert!(close(range.position(1.0).unwrap(), 0.5));
        assert!(close(range.position(0.1).unwrap(), 0.0));
        assert!(close(range.position(10.0).unwrap(), 1.0));
        assert!(close(range.position(0.0).unwrap(), 0.0));
        assert!(close(range.value_at(0.5).unwrap(), 1.0));
        assert!(!RampRange::log10(0.0, 10.0).is_valid());
        assert!(!RampRange::linear(5.0, f64::INFINITY).is_valid());
    }

    #[test]
    fn legend_stops_are_monotonic_and_span_the_range() {
        for r in RAMPS {
            let range = r.default_range;
            let stops = legend(r, range);
            assert_eq!(stops.len(), r.stops.len());
            assert!(
                stops.windows(2).all(|w| w[1].value > w[0].value),
                "{}",
                r.id
            );
            assert!(stops.windows(2).all(|w| w[1].fraction > w[0].fraction));
            let (first, last) = (stops[0], stops[stops.len() - 1]);
            assert!(close(first.value, range.low) && close(first.fraction, 0.0));
            assert!((last.value - range.high).abs() < 1e-9 * range.high.abs().max(1.0));
            assert!(close(last.fraction, 1.0));
            assert!(stops.iter().zip(r.stops).all(|(s, &c)| s.colour == c));
        }
        let chl = legend(&ALGAE, RampRange::log10(0.1, 10.0));
        assert!(close(chl[16].value, 1.0));
        assert_eq!(legend(&THERMAL, RampRange::linear(20.0, 10.0)), Vec::new());
    }

    #[test]
    fn mercator_round_trips_per_row() {
        for lat in [-80.0, -38.82, 0.0, 40.7, 85.0] {
            assert!((mercator_lat(mercator_y(lat)) - lat).abs() < 1e-9);
        }
        // Tile 7/37/48 spans latitude 38.82 to 40.98, as GIBS documents for the same tile.
        assert!((mercator_lat(48.0 / 128.0) - 40.98).abs() < 0.005);
        assert!((mercator_lat(49.0 / 128.0) - 38.82).abs() < 0.005);
    }

    /// A coarse class map of a tile: `.` transparent, otherwise the tenth of the ramp the
    /// nearest stop falls in.
    fn class_map(rgba: &[u8], size: u32, ramp: &Ramp, cells: u32) -> String {
        let block = size / cells;
        let mut out = String::new();
        for cy in 0..cells {
            for cx in 0..cells {
                let (x, y) = (
                    (cx * block + block / 2) as usize,
                    (cy * block + block / 2) as usize,
                );
                let p = &rgba[(y * size as usize + x) * 4..][..4];
                if p[3] == 0 {
                    out.push('.');
                    continue;
                }
                let nearest = (0..ramp.stops.len())
                    .min_by_key(|&k| {
                        let s = ramp.stops[k];
                        [(s.r, p[0]), (s.g, p[1]), (s.b, p[2])]
                            .iter()
                            .map(|&(a, b)| (i32::from(a) - i32::from(b)).pow(2))
                            .sum::<i32>()
                    })
                    .unwrap();
                let decile = nearest * 10 / ramp.stops.len();
                out.push(char::from(b'0' + decile as u8));
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn mur_tile_is_coloured_over_water_and_transparent_over_land_and_outside() {
        let grid = mur();
        let range = RampRange::linear(15.5, 16.5);
        let size = 256;
        let rgba = render_rgba(&[grid], &THERMAL, range, MUR_TILE, size).unwrap();
        assert_eq!(rgba.len(), 256 * 256 * 4);

        let water = pixel_of(
            MUR_TILE,
            size,
            GeoPoint {
                lat: 40.3,
                lon: -73.5,
            },
        );
        let land = pixel_of(
            MUR_TILE,
            size,
            GeoPoint {
                lat: 40.2,
                lon: -74.15,
            },
        );
        let outside = pixel_of(
            MUR_TILE,
            size,
            GeoPoint {
                lat: 39.5,
                lon: -75.0,
            },
        );
        assert_eq!(alpha(&rgba, size, water), 255);
        assert_eq!(alpha(&rgba, size, land), 0);
        assert_eq!(alpha(&rgba, size, outside), 0);
        // Straight alpha: transparent pixels carry no colour, opaque ones are not premultiplied.
        assert!(
            rgba.as_chunks::<4>()
                .0
                .iter()
                .all(|p| p[3] == 0 || p[3] == 255)
        );
        assert!(
            rgba.as_chunks::<4>()
                .0
                .iter()
                .filter(|p| p[3] == 0)
                .all(|p| *p == [0; 4])
        );

        let png = render_tile(&[mur()], &THERMAL, range, MUR_TILE, size).unwrap();
        let decoder = png::Decoder::new(std::io::Cursor::new(&png[..]));
        let mut reader = decoder.read_info().unwrap();
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut pixels).unwrap();
        assert_eq!((info.width, info.height), (256, 256));
        assert_eq!(info.color_type, png::ColorType::Rgba);
        assert_eq!(info.bit_depth, png::BitDepth::Eight);
        assert_eq!(pixels, rgba);

        let classes = class_map(&rgba, size, &THERMAL, 16);
        let summary = format!(
            "{classes}png bytes: {}\npng fnv1a: {:016x}\nrgba fnv1a: {:016x}\n",
            png.len(),
            fnv1a(&png),
            fnv1a(&rgba)
        );
        insta::assert_snapshot!("mur_tile_7_37_48", summary);
    }

    #[test]
    fn a_pixel_nearest_a_missing_point_is_transparent_and_the_others_reweight() {
        // Two by two cells, one missing: pixels nearest the missing point are transparent, and the
        // rest take the value of the three present points.
        let g = Grid::new(
            source("synthetic", &["v"], 1.0),
            UnixTime(0),
            vec![40.0, 41.0],
            vec![-74.0, -73.0],
            vec![1.0, 2.0, f32::NAN, 4.0],
            "degree_C",
        )
        .unwrap();
        let tile = TileKey { z: 8, x: 75, y: 96 };
        let rgba = render_rgba(&[g], &THERMAL, RampRange::linear(0.0, 5.0), tile, 64).unwrap();
        let opaque = rgba
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p[3] == 255)
            .count();
        assert!(opaque > 0 && opaque < 64 * 64, "{opaque}");
        // The missing point is the north-west one: the pixels around it are bare.
        let nw = pixel_of(
            tile,
            64,
            GeoPoint {
                lat: 40.9,
                lon: -73.95,
            },
        );
        assert_eq!(alpha(&rgba, 64, nw), 0);
        let se = pixel_of(
            tile,
            64,
            GeoPoint {
                lat: 40.05,
                lon: -73.2,
            },
        );
        assert_eq!(alpha(&rgba, 64, se), 255);
    }

    #[test]
    fn a_pixel_of_a_tile_is_the_colour_of_the_grids_own_sample() {
        // Every pixel of the coastal tile agrees with `Grid::sample_bilinear` at its centre, so
        // the point inspector reads what the tile shows.
        let grid = mur();
        let range = RampRange::linear(15.5, 16.5);
        let size = 256u32;
        let rgba =
            render_rgba(std::slice::from_ref(&grid), &THERMAL, range, MUR_TILE, size).unwrap();
        let world = f64::from(size) * f64::from(1u32 << MUR_TILE.z);
        let (mut coloured, mut bare, mut coast) = (0, 0, 0);
        for py in 0..size {
            for px in 0..size {
                let at = GeoPoint {
                    lat: mercator_lat(
                        (f64::from(MUR_TILE.y) * f64::from(size) + f64::from(py) + 0.5) / world,
                    ),
                    lon: (f64::from(MUR_TILE.x) * f64::from(size) + f64::from(px) + 0.5) / world
                        * 360.0
                        - 180.0,
                };
                let want = grid
                    .sample_bilinear(at)
                    .and_then(|v| THERMAL.colour(range, f64::from(v)))
                    .unwrap_or(Rgba::TRANSPARENT);
                let got = &rgba[((py * size + px) * 4) as usize..][..4];
                assert_eq!(got, [want.r, want.g, want.b, want.a], "{px},{py}");
                if want.a == 0 {
                    bare += 1;
                } else {
                    coloured += 1;
                }
                // A coastal pixel: coloured although one of its four surrounding points is land.
                let (Some(_), Some(row), Some(col)) = (
                    grid.sample_bilinear(at),
                    crate::map::grid::locate(&grid.lats, at.lat, 0.0),
                    crate::map::grid::locate_lon(&grid.lons, at.lon, 0.0),
                ) else {
                    continue;
                };
                let corners = [
                    (row.lo, col.lo),
                    (row.lo, col.hi),
                    (row.hi, col.lo),
                    (row.hi, col.hi),
                ];
                if corners
                    .iter()
                    .any(|&(i, j)| grid.value(i, j).is_some_and(f32::is_nan))
                {
                    coast += 1;
                }
            }
        }
        assert!(coloured > 0 && bare > 0);
        assert!(coast > 0, "the fixture tile has a coast");
    }

    #[test]
    fn grids_cover_half_a_step_beyond_their_edge_points() {
        let g = uniform(10.0, steps(40.0, 0.1, 5), steps(-74.0, 0.1, 5));
        let half = 0.05;
        assert!(
            sample(
                &g,
                GeoPoint {
                    lat: 40.0 - half * 0.9,
                    lon: -73.8
                }
            )
            .is_some()
        );
        assert!(
            sample(
                &g,
                GeoPoint {
                    lat: 40.0 - half * 1.1,
                    lon: -73.8
                }
            )
            .is_none()
        );
        assert!(
            sample(
                &g,
                GeoPoint {
                    lat: 40.2,
                    lon: -73.6 + half * 0.9
                }
            )
            .is_some()
        );
        assert!(
            sample(
                &g,
                GeoPoint {
                    lat: 40.2,
                    lon: -73.6 + half * 1.1
                }
            )
            .is_none()
        );
        assert!(
            sample(
                &g,
                GeoPoint {
                    lat: f64::NAN,
                    lon: -73.8
                }
            )
            .is_none()
        );
    }

    #[test]
    fn bilinear_samples_blend_the_four_neighbours() {
        let g = Grid::new(
            source("synthetic", &["v"], 1.0),
            UnixTime(0),
            vec![0.0, 1.0],
            vec![0.0, 1.0],
            vec![0.0, 10.0, 20.0, 30.0],
            "degree_C",
        )
        .unwrap();
        let v = sample(
            &g,
            GeoPoint {
                lat: 0.5,
                lon: 0.25,
            },
        )
        .unwrap();
        assert!(close(v, 12.5));
        let v = sample(&g, GeoPoint { lat: 1.0, lon: 1.0 }).unwrap();
        assert!(close(v, 30.0));
    }

    #[test]
    fn grids_on_0_to_360_longitudes_are_found() {
        let g = uniform(3.0, steps(40.0, 0.5, 3), steps(285.0, 0.5, 3));
        assert!(
            sample(
                &g,
                GeoPoint {
                    lat: 40.5,
                    lon: -74.5
                }
            )
            .is_some()
        );
    }

    #[test]
    fn the_first_grid_with_a_value_wins() {
        let lats = steps(38.0, 0.25, 13);
        let lons = steps(-76.0, 0.25, 13);
        let mut holes = uniform(1.0, lats.clone(), lons.clone());
        holes
            .values
            .iter_mut()
            .step_by(2)
            .for_each(|v| *v = f32::NAN);
        let full = uniform(9.0, lats, lons);
        let range = RampRange::linear(0.0, 10.0);
        let rgba = render_rgba(&[holes, full], &THERMAL, range, MUR_TILE, 32).unwrap();
        let nine = THERMAL.colour(range, 9.0).unwrap();
        let p = pixel_of(
            MUR_TILE,
            32,
            GeoPoint {
                lat: 39.5,
                lon: -74.5,
            },
        );
        let at = (p.1 * 32 + p.0) * 4;
        assert_eq!(rgba[at..at + 4], [nine.r, nine.g, nine.b, nine.a]);
        // Where both have a value, the first one's is drawn.
        let lats = steps(38.0, 0.25, 13);
        let lons = steps(-76.0, 0.25, 13);
        let first = uniform(1.0, lats.clone(), lons.clone());
        let second = uniform(9.0, lats, lons);
        let rgba = render_rgba(&[first, second], &THERMAL, range, MUR_TILE, 32).unwrap();
        let one = THERMAL.colour(range, 1.0).unwrap();
        assert_eq!(rgba[at..at + 4], [one.r, one.g, one.b, one.a]);
    }

    #[test]
    fn bad_sizes_tiles_and_ranges_are_refused() {
        let g = [mur()];
        let range = RampRange::linear(10.0, 20.0);
        assert_eq!(
            render_tile(&g, &THERMAL, range, MUR_TILE, 0),
            Err(RenderError::BadSize(0))
        );
        assert_eq!(
            render_tile(&g, &THERMAL, range, MUR_TILE, MAX_TILE_SIZE + 1),
            Err(RenderError::BadSize(MAX_TILE_SIZE + 1))
        );
        let off = TileKey { z: 2, x: 4, y: 0 };
        let err = render_tile(&g, &THERMAL, range, off, 256).unwrap_err();
        assert_eq!(err.to_string(), "tile 2/4/0 is not a Web Mercator tile");
        let deep = TileKey { z: 31, x: 0, y: 0 };
        assert!(matches!(
            render_tile(&g, &THERMAL, range, deep, 256),
            Err(RenderError::BadTile(_))
        ));
        assert_eq!(
            render_tile(&g, &THERMAL, RampRange::linear(20.0, 10.0), MUR_TILE, 256),
            Err(RenderError::BadRange)
        );
        assert_eq!(encode_png(&[0; 4], 2, 2), Err(RenderError::BadSize(2)));
    }

    #[test]
    fn a_grid_with_broken_fields_renders_transparent_without_panicking() {
        let mut g = mur();
        g.values.truncate(3);
        let rgba = render_rgba(&[g], &THERMAL, RampRange::linear(10.0, 20.0), MUR_TILE, 16);
        assert!(rgba.unwrap().as_chunks::<4>().0.iter().all(|p| p[3] == 0));
    }

    fn field(u: f32, v: f32) -> (Grid, Grid) {
        let lats = steps(40.0, 0.01, 81);
        let lons = steps(-74.2, 0.01, 121);
        (
            uniform(u, lats.clone(), lons.clone()),
            uniform(v, lats, lons),
        )
    }

    const HARBOR: Bbox = Bbox {
        west: -74.2,
        south: 40.0,
        east: -73.0,
        north: 40.8,
    };

    #[test]
    fn an_eastward_field_points_toward_90_degrees() {
        let (u, v) = field(1.0, 0.0);
        let out = arrows(&u, &v, HARBOR, 10.0);
        assert_ne!(out.len(), 0);
        for a in &out {
            assert!((a.toward_deg - 90.0).abs() < 1e-4);
            assert!((a.speed_ms - 1.0).abs() < 1e-6);
        }
    }

    #[test]
    fn a_northward_field_points_toward_0_degrees() {
        let (u, v) = field(0.0, 1.0);
        let out = arrows(&u, &v, HARBOR, 10.0);
        assert_ne!(out.len(), 0);
        assert!(out.iter().all(|a| a.toward_deg.abs() < 1e-4));
    }

    #[test]
    fn directions_are_toward_clockwise_from_north() {
        assert!((toward_deg(0.0, -1.0) - 180.0).abs() < 1e-9);
        assert!((toward_deg(-1.0, 0.0) - 270.0).abs() < 1e-9);
        assert!((toward_deg(1.0, 1.0) - 45.0).abs() < 1e-9);
        assert!(toward_deg(-1e-300, 1.0).abs() < 1e-9);
        let (u, v) = field(-0.3, -0.4);
        let a = arrows(&u, &v, HARBOR, 9.0)[0];
        assert!((a.speed_ms - 0.5).abs() < 1e-6);
        assert!((f64::from(a.toward_deg) - 216.869_9).abs() < 1e-3);
    }

    #[test]
    fn arrows_are_thinned_to_about_one_per_32_px() {
        let (u, v) = field(0.5, 0.5);
        // Rounding the stride keeps the spacing within two thirds and four thirds of 32 px.
        let (near, far) = (ARROW_SPACING_PX * 2.0 / 3.0, ARROW_SPACING_PX * 4.0 / 3.0);
        for zoom in [8.0, 10.0, 11.5] {
            let out = arrows(&u, &v, HARBOR, zoom);
            let world = 256.0 * 2f64.powf(zoom);
            let px = |a: &Arrow| {
                (
                    (a.at.lon + 180.0) / 360.0 * world,
                    mercator_y(a.at.lat) * world,
                )
            };
            let mut rows: Vec<Vec<(f64, f64)>> = Vec::new();
            for a in &out {
                match rows.last_mut() {
                    Some(row) if (row[0].1 - px(a).1).abs() < 1e-6 => row.push(px(a)),
                    _ => rows.push(vec![px(a)]),
                }
            }
            assert!(rows.len() > 1, "zoom {zoom}");
            for row in &rows {
                assert!(row.len() > 1, "zoom {zoom}");
                for w in row.windows(2) {
                    let gap = w[1].0 - w[0].0;
                    assert!(gap > near && gap < far, "zoom {zoom}: {gap}");
                }
            }
            for w in rows.windows(2) {
                let gap = w[1][0].1 - w[0][0].1;
                assert!(gap > near && gap < far, "zoom {zoom}: {gap}");
            }
            let area = (px(&out[out.len() - 1]).1 - px(&out[0]).1)
                * (rows[0][rows[0].len() - 1].0 - rows[0][0].0);
            let per_arrow = area / out.len() as f64;
            assert!(
                per_arrow < far * far && per_arrow > near * near / 2.0,
                "zoom {zoom}"
            );
        }
        // Zoomed in past the grid's density, every grid point in the box gets an arrow.
        let small = Bbox {
            west: -74.005,
            south: 40.495,
            east: -73.945,
            north: 40.555,
        };
        assert_eq!(arrows(&u, &v, small, 16.0).len(), 6 * 6);
    }

    #[test]
    fn arrows_stay_put_while_the_map_pans() {
        // The panned view comes with its own subset of the same dataset, starting further east.
        let (u, v) = field(0.5, 0.5);
        let lats = steps(40.0, 0.01, 81);
        let lons = steps(-73.93, 0.01, 94);
        let (pu, pv) = (
            uniform(0.5, lats.clone(), lons.clone()),
            uniform(0.5, lats, lons),
        );
        let east = Bbox {
            west: -73.905,
            east: -73.105,
            ..HARBOR
        };
        let key = |a: &Arrow| {
            (
                (a.at.lat * 1e6).round() as i64,
                (a.at.lon * 1e6).round() as i64,
            )
        };
        let inside: Vec<(i64, i64)> = arrows(&u, &v, HARBOR, 10.0)
            .iter()
            .filter(|a| a.at.lon >= east.west && a.at.lon <= east.east)
            .map(key)
            .collect();
        let panned: Vec<(i64, i64)> = arrows(&pu, &pv, east, 10.0).iter().map(key).collect();
        assert_ne!(panned.len(), 0);
        assert_eq!(inside, panned);
    }

    #[test]
    fn arrows_skip_missing_values_and_bad_boxes() {
        let (mut u, v) = field(1.0, 0.0);
        u.values.iter_mut().for_each(|x| *x = f32::NAN);
        assert_eq!(arrows(&u, &v, HARBOR, 10.0), Vec::new());
        let (u, v) = field(1.0, 0.0);
        let flipped = Bbox {
            west: -73.0,
            east: -74.0,
            ..HARBOR
        };
        assert_eq!(arrows(&u, &v, flipped, 10.0), Vec::new());
        assert_eq!(arrows(&u, &v, HARBOR, f64::NAN), Vec::new());
    }

    #[test]
    fn hf_radar_fixture_gives_arrows_inside_the_box() {
        let src = source("ucsdHfrE6", &["water_u", "water_v"], 0.058);
        let mut grids = grids_from_table(
            &fixture("ucsdhfre6-currents-subset-2026-11-08t08.json"),
            &src,
        );
        let v = grids.pop().unwrap();
        let u = grids.pop().unwrap();
        assert_eq!(u.unit, "m s-1");
        let out = arrows(&u, &v, HARBOR, 9.0);
        assert_ne!(out.len(), 0);
        for a in &out {
            assert!(a.at.lat >= HARBOR.south && a.at.lat <= HARBOR.north);
            assert!(a.at.lon >= HARBOR.west && a.at.lon <= HARBOR.east);
            assert!(a.speed_ms.is_finite() && a.speed_ms >= 0.0);
            assert!((0.0..360.0).contains(&a.toward_deg));
        }
        // Ordered north-west to south-east, the same on every run.
        assert!(out.windows(2).all(|w| w[0].at.lat >= w[1].at.lat - 0.3));
        assert_eq!(out, arrows(&u, &v, HARBOR, 9.0));
    }
}
