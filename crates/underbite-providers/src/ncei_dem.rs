//! The NOAA NCEI DEM global mosaic: image export requests and TIFF decoding (sans-IO).
//!
//! The mosaic is an Esri `ImageServer` of many bathymetric-topographic elevation models; for any
//! point it answers with the finest one that covers it. Values are metres, **positive up** (water
//! is below zero), relative to the vertical datum of the source model: NAVD 88 for the 1/9
//! arc-second CUDEM tiles, EGM 2008 for the Coastal Relief Model, and so on. The datum is never a
//! constant here: [`query_request`] and [`parse_query`] list the sources at a point with theirs.
//! None of them is chart datum.
//!
//! Elevations come from `exportImage` as a float32 TIFF, either for a Web Mercator tile
//! ([`tile_request`], small tiles for shading) or for a working area in degrees
//! ([`area_request`], one request for contours). [`parse_export`] decodes the image and its
//! placement tags into a [`DemImage`], which becomes a map [`Grid`] with
//! [`DemImage::into_grid`]. [`identify_request`] reads the value at one point. The service
//! answers failures with a JSON error object, often with status 200: [`check_response`] finds it.

use std::io::Cursor;

use serde::Deserialize;
use tiff::ColorType;
use tiff::decoder::{Decoder, DecodingResult};
use tiff::tags::Tag;
use underbite_core::map::grid::{Grid, GridError};
use underbite_core::map::tiles::{MAX_ZOOM, ORIGIN_M, WORLD_M, tile_bounds_3857};
use underbite_core::map::{Bbox, GridHost, GridSource, TileKey};
use underbite_core::time::{LocalDate, UnixTime, UtcOffset};

use crate::request::{Response, RoundedPoint};

/// The mosaic's `ImageServer`, without a trailing slash.
pub const BASE_URL: &str =
    "https://gis.ngdc.noaa.gov/arcgis/rest/services/DEM_mosaics/DEM_global_mosaic/ImageServer";

/// The dataset name DEM grids are filed under (`GridSource::dataset`).
pub const DATASET: &str = "DEM_global_mosaic";

/// The variable name DEM grids are filed under (`GridSource::variables`).
pub const VARIABLE: &str = "elevation";

/// The value asked for where the mosaic has no data (`noData=-9999`).
pub const NO_DATA: f32 = -9999.0;

/// The side of a Web Mercator elevation tile, pixels: 128 × 128 float32 is about 67 KB, a quarter
/// of a 256 × 256 one.
pub const DEFAULT_TILE_PX: u32 = 128;

/// The largest image side the service exports, pixels.
pub const MAX_IMAGE_PX: u32 = 20_000;

/// The unit of every value: metres, positive up.
pub const UNIT: &str = "m";

/// Degrees in a request keep this many decimals (about 0.1 m).
const DEGREE_DECIMALS: i32 = 6;

/// The `outFields` of a source query, as the recorded query asks for them.
const QUERY_FIELDS: &str = "Name,ProductName,CellsizeArcseconds,VerticalDatum,DateCompleted,DemName,MetadataURL,LowPS,HighPS";

/// The spherical radius of Web Mercator, metres.
const MERCATOR_RADIUS_M: f64 = WORLD_M / (2.0 * std::f64::consts::PI);

/// The coordinate system of an exported image.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DemCrs {
    /// Longitude and latitude in degrees (EPSG:4326).
    Wgs84,
    /// Web Mercator metres (EPSG:3857).
    WebMercator,
}

impl DemCrs {
    /// The EPSG code the service takes for `bboxSR` and `imageSR`.
    pub const fn epsg(self) -> u32 {
        match self {
            Self::Wgs84 => 4326,
            Self::WebMercator => 3857,
        }
    }
}

/// What a [`DemRequest`] asks for; selects its parser.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DemResource {
    /// A float32 TIFF in a coordinate system: [`parse_export`].
    Export(DemCrs),
    /// The value at one point: [`parse_identify`].
    Identify,
    /// The source models at one point: [`parse_query`].
    Query,
}

/// One GET for the host to send. The URL carries no key; a point is rounded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DemRequest {
    /// The full URL with the query.
    pub url: String,
    /// What it asks for.
    pub resource: DemResource,
}

/// A DEM request could not be built, or its response read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DemError {
    /// The request asks for something that cannot be sent.
    #[error("invalid DEM request: {0}")]
    InvalidRequest(&'static str),
    /// The service answered with its JSON error object.
    #[error("the DEM service failed ({code:?}): {message}")]
    Service {
        /// The error's `code`, usually an HTTP status.
        code: Option<i64>,
        /// The error's `message`.
        message: String,
    },
    /// An HTTP failure without the service's error object.
    #[error("the DEM service answered HTTP {status}")]
    Status {
        /// The status code.
        status: u16,
    },
    /// The body is not a TIFF that decodes.
    #[error("the DEM export is not a readable TIFF: {0}")]
    BadImage(String),
    /// The TIFF decodes but is not a single-band, uncompressed float32 elevation image.
    #[error("unsupported DEM image: {0}")]
    Unsupported(String),
    /// The TIFF lacks the placement tags this reader needs, or states an unusable one.
    #[error("the DEM export has no usable placement: {0}")]
    BadPlacement(&'static str),
    /// The body is not the JSON the request expects.
    #[error("the DEM response is not the expected JSON: {0}")]
    BadJson(String),
    /// The image's axes do not make a grid.
    #[error(transparent)]
    Grid(#[from] GridError),
}

/// A coordinate in degrees as the recorded export writes it: up to six decimals, at least one
/// (`40.0`, `-74.2`).
fn fmt_deg(value: f64) -> String {
    let scale = 10_f64.powi(DEGREE_DECIMALS);
    let rounded = (value * scale).round() / scale;
    // Avoid `-0.0`.
    let rounded = if rounded == 0.0 { 0.0 } else { rounded };
    let text = rounded.to_string();
    if text.contains('.') {
        text
    } else {
        format!("{text}.0")
    }
}

/// Web Mercator metres: one decimal, and none for a whole number.
fn fmt_metres(value: f64) -> String {
    let text = format!("{value:.1}");
    let text = match text.strip_suffix(".0") {
        Some(whole) => whole.to_owned(),
        None => text,
    };
    if text == "-0" { "0".to_owned() } else { text }
}

fn export_url(bbox: [String; 4], crs: DemCrs, width: u32, height: u32) -> String {
    let [min_x, min_y, max_x, max_y] = bbox;
    let sr = crs.epsg();
    format!(
        "{BASE_URL}/exportImage?bbox={min_x},{min_y},{max_x},{max_y}&bboxSR={sr}&imageSR={sr}&size={width},{height}&format=tiff&pixelType=F32&interpolation=RSP_BilinearInterpolation&noData=-9999&f=image"
    )
}

/// The export of a Web Mercator tile at `size_px` × `size_px` pixels ([`DEFAULT_TILE_PX`] for
/// shading), its box from the tile geometry of the map core.
///
/// # Errors
/// [`DemError::InvalidRequest`] for a zoom past [`MAX_ZOOM`], a tile index outside the zoom's
/// range, or a size of 0 or above [`MAX_IMAGE_PX`].
pub fn tile_request(key: TileKey, size_px: u32) -> Result<DemRequest, DemError> {
    if key.z > MAX_ZOOM {
        return Err(DemError::InvalidRequest("zoom"));
    }
    let side = 1u32 << key.z;
    if key.x >= side || key.y >= side {
        return Err(DemError::InvalidRequest("tile index"));
    }
    if size_px == 0 || size_px > MAX_IMAGE_PX {
        return Err(DemError::InvalidRequest("image size"));
    }
    let b = tile_bounds_3857(key);
    let bbox = [b.min_x, b.min_y, b.max_x, b.max_y].map(fmt_metres);
    Ok(DemRequest {
        url: export_url(bbox, DemCrs::WebMercator, size_px, size_px),
        resource: DemResource::Export(DemCrs::WebMercator),
    })
}

/// Pixels along a span at a cell size, never fewer than one. A span that is a whole number of
/// cells up to rounding (1.2° / 0.005° is 240.000…06) is not given an extra pixel.
fn pixels(span: f64, cell: f64) -> f64 {
    (span / cell - 1e-6).ceil().max(1.0)
}

/// The export of an area in degrees with cells of about `cell_deg` degrees, for contours or a
/// whole harbour at once.
///
/// The box is clipped to -180..180 and -90..90. When either side would exceed [`MAX_IMAGE_PX`]
/// pixels, both are scaled down together so the cells stay square and grow; the decoded image's
/// pixel size is the one to use.
///
/// # Errors
/// [`DemError::InvalidRequest`] for a box with a non-finite edge or no area, or a cell size that
/// is not a positive number.
pub fn area_request(area: Bbox, cell_deg: f64) -> Result<DemRequest, DemError> {
    let edges = [area.west, area.south, area.east, area.north];
    if edges.iter().any(|e| !e.is_finite()) {
        return Err(DemError::InvalidRequest("box"));
    }
    if !(cell_deg.is_finite() && cell_deg > 0.0) {
        return Err(DemError::InvalidRequest("cell size"));
    }
    let west = area.west.clamp(-180.0, 180.0);
    let east = area.east.clamp(-180.0, 180.0);
    let south = area.south.clamp(-90.0, 90.0);
    let north = area.north.clamp(-90.0, 90.0);
    if west >= east || south >= north {
        return Err(DemError::InvalidRequest("box"));
    }
    let mut width = pixels(east - west, cell_deg);
    let mut height = pixels(north - south, cell_deg);
    let longest = width.max(height);
    let max = f64::from(MAX_IMAGE_PX);
    if longest > max {
        width = (width * max / longest).round().clamp(1.0, max);
        height = (height * max / longest).round().clamp(1.0, max);
    }
    let bbox = [west, south, east, north].map(fmt_deg);
    Ok(DemRequest {
        // Both are whole numbers in 1..=MAX_IMAGE_PX.
        url: export_url(bbox, DemCrs::Wgs84, width as u32, height as u32),
        resource: DemResource::Export(DemCrs::Wgs84),
    })
}

/// The mosaic's value at a point (the finest source, at its native resolution).
pub fn identify_request(at: RoundedPoint) -> DemRequest {
    let (lat, lon) = at.fmt_param();
    DemRequest {
        url: format!(
            "{BASE_URL}/identify?geometry={lon},{lat}&geometryType=esriGeometryPoint&returnGeometry=false&returnCatalogItems=false&f=json"
        ),
        resource: DemResource::Identify,
    }
}

/// The source models that cover a point, with each one's cell size, vertical datum and date.
pub fn query_request(at: RoundedPoint) -> DemRequest {
    let (lat, lon) = at.fmt_param();
    DemRequest {
        url: format!(
            "{BASE_URL}/query?geometry={lon},{lat}&geometryType=esriGeometryPoint&inSR=4326&spatialRel=esriSpatialRelIntersects&outFields={QUERY_FIELDS}&returnGeometry=false&f=json"
        ),
        resource: DemResource::Query,
    }
}

#[derive(Deserialize)]
struct ErrorEnvelope {
    error: EsriError,
}

#[derive(Deserialize)]
struct EsriError {
    #[serde(default)]
    code: Option<i64>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    details: Vec<String>,
}

/// The service's error object in a body, if the body is one.
fn esri_error(body: &[u8]) -> Option<DemError> {
    let start = body.iter().position(|b| !b.is_ascii_whitespace())?;
    if body[start] != b'{' {
        return None;
    }
    let envelope: ErrorEnvelope = serde_json::from_slice(body).ok()?;
    let e = envelope.error;
    let mut message = e.message.unwrap_or_default();
    if !e.details.is_empty() {
        if !message.is_empty() {
            message.push_str(": ");
        }
        message.push_str(&e.details.join("; "));
    }
    Some(DemError::Service {
        code: e.code,
        message,
    })
}

/// The body of a successful response, or what went wrong.
///
/// # Errors
/// [`DemError::Service`] for the service's JSON error object, whatever the status (it answers
/// with 200), [`DemError::Status`] for any other failure, and [`DemError::BadImage`] for a
/// success that is an HTML page.
pub fn check_response<'a>(response: &Response<'a>) -> Result<&'a [u8], DemError> {
    if let Some(error) = esri_error(response.body) {
        return Err(error);
    }
    match response.status {
        200..=299 => {
            let html = response
                .content_type
                .is_some_and(|c| c.to_ascii_lowercase().contains("html"));
            if html {
                Err(DemError::BadImage("an HTML page".to_owned()))
            } else {
                Ok(response.body)
            }
        }
        status => Err(DemError::Status { status }),
    }
}

/// An exported elevation image: rows from north to south, metres positive up, NaN where the
/// mosaic has no data.
///
/// Built only by [`parse_export`]; `values` holds `width × height` values, row-major.
#[derive(Clone, Debug, PartialEq)]
pub struct DemImage {
    crs: DemCrs,
    width: u32,
    height: u32,
    pixel_size: (f64, f64),
    top_left: (f64, f64),
    values: Vec<f32>,
}

impl DemImage {
    /// The coordinate system of the placement.
    pub fn crs(&self) -> DemCrs {
        self.crs
    }

    /// Columns.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Rows.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// The width and height of a pixel: degrees for [`DemCrs::Wgs84`], metres for
    /// [`DemCrs::WebMercator`].
    pub fn pixel_size(&self) -> (f64, f64) {
        self.pixel_size
    }

    /// The north-west corner of the image (the outer corner of its first pixel) as `(x, y)`:
    /// longitude and latitude, or Web Mercator metres.
    pub fn top_left(&self) -> (f64, f64) {
        self.top_left
    }

    /// The values, row-major from the north-west; NaN where there is no data.
    pub fn values(&self) -> &[f32] {
        &self.values
    }

    /// The value at a row (from the north) and column (from the west); `None` outside the image.
    pub fn value(&self, row: u32, col: u32) -> Option<f32> {
        if row >= self.height || col >= self.width {
            return None;
        }
        let index = row as usize * self.width as usize + col as usize;
        self.values.get(index).copied()
    }

    /// The lowest and highest value, ignoring missing ones; `None` if every value is missing.
    pub fn range(&self) -> Option<(f32, f32)> {
        self.values
            .iter()
            .filter(|v| !v.is_nan())
            .fold(None, |acc, &v| match acc {
                None => Some((v, v)),
                Some((lo, hi)) => Some((lo.min(v), hi.max(v))),
            })
    }

    /// The longitude of each column's centre, west to east, degrees.
    pub fn lons(&self) -> Vec<f64> {
        let (x0, _) = self.top_left;
        let (dx, _) = self.pixel_size;
        (0..self.width)
            .map(|j| {
                let x = x0 + (f64::from(j) + 0.5) * dx;
                match self.crs {
                    DemCrs::Wgs84 => x,
                    DemCrs::WebMercator => (x / MERCATOR_RADIUS_M).to_degrees(),
                }
            })
            .collect()
    }

    /// The latitude of each row's centre, north to south, degrees. For a Web Mercator image each
    /// is the inverse Mercator of the row's centre, so the steps shrink toward the equator.
    pub fn lats_north_to_south(&self) -> Vec<f64> {
        let (_, y0) = self.top_left;
        let (_, dy) = self.pixel_size;
        (0..self.height)
            .map(|i| {
                let y = y0 - (f64::from(i) + 0.5) * dy;
                match self.crs {
                    DemCrs::Wgs84 => y,
                    DemCrs::WebMercator => (y / MERCATOR_RADIUS_M).sinh().atan().to_degrees(),
                }
            })
            .collect()
    }

    /// The image as a map grid at the cell centres, in [`UNIT`], latitudes ascending.
    ///
    /// The grid is filed under [`GridHost::NceiDem`] and the dataset [`DATASET`]; `time` is the
    /// instant it stands for (the DEM has no time of its own). Its `native_step_deg` is the
    /// image's own longitude step.
    ///
    /// # Errors
    /// [`DemError::Grid`] if the axes do not make a grid.
    pub fn into_grid(self, time: UnixTime) -> Result<Grid, DemError> {
        let lats = self.lats_north_to_south();
        let lons = self.lons();
        let native_step_deg = match (lons.first(), lons.last()) {
            (Some(first), Some(last)) if lons.len() > 1 => (last - first) / (lons.len() - 1) as f64,
            _ => 0.0,
        };
        let source = GridSource {
            node: GridHost::NceiDem,
            dataset: DATASET.to_owned(),
            variables: vec![VARIABLE.to_owned()],
            altitude_axis: false,
            native_step_deg,
        };
        Ok(Grid::new(source, time, lats, lons, self.values, UNIT)?)
    }
}

fn bad_image(e: &tiff::TiffError) -> DemError {
    DemError::BadImage(e.to_string())
}

/// The geo keys this reader looks at, from the `GeoKeyDirectory`: the model type (1024: 1
/// projected, 2 geographic) and the raster type (1025: 1 pixel is area, 2 pixel is point).
#[derive(Default)]
struct GeoKeys {
    model_type: Option<u16>,
    raster_type: Option<u16>,
}

impl GeoKeys {
    fn read(directory: &[u16]) -> Self {
        let mut keys = Self::default();
        // A header of four shorts (version, revision, minor, count), then four per key: id,
        // location (0 for a value held in place), count and value.
        let Some(&count) = directory.get(3) else {
            return keys;
        };
        let (entries, _) = directory.get(4..).unwrap_or_default().as_chunks::<4>();
        for entry in entries.iter().take(usize::from(count)) {
            if entry[1] != 0 {
                continue;
            }
            match entry[0] {
                1024 => keys.model_type = Some(entry[3]),
                1025 => keys.raster_type = Some(entry[3]),
                _ => {}
            }
        }
        keys
    }
}

type Tiff<'a> = Decoder<Cursor<&'a [u8]>>;

/// Checks that the image is one uncompressed float32 sample per pixel, of a size this reader
/// takes, and returns its width and height.
fn check_layout(decoder: &mut Tiff<'_>) -> Result<(u32, u32), DemError> {
    let (width, height) = decoder.dimensions().map_err(|e| bad_image(&e))?;
    if width == 0 || height == 0 || width > MAX_IMAGE_PX || height > MAX_IMAGE_PX {
        return Err(DemError::Unsupported(format!(
            "an image of {width} × {height} pixels"
        )));
    }
    let compression = decoder
        .find_tag_unsigned::<u16>(Tag::Compression)
        .map_err(|e| bad_image(&e))?
        .unwrap_or(1);
    if compression != 1 {
        return Err(DemError::Unsupported(format!("compression {compression}")));
    }
    let color = decoder.colortype().map_err(|e| bad_image(&e))?;
    if color != ColorType::Gray(32) {
        return Err(DemError::Unsupported(format!("samples of {color:?}")));
    }
    let sample_format = decoder
        .find_tag_unsigned::<u16>(Tag::SampleFormat)
        .map_err(|e| bad_image(&e))?
        .unwrap_or(1);
    if sample_format != 3 {
        return Err(DemError::Unsupported(format!(
            "sample format {sample_format}, not IEEE float"
        )));
    }
    Ok((width, height))
}

fn f64_tag(decoder: &mut Tiff<'_>, tag: Tag, missing: &'static str) -> Result<Vec<f64>, DemError> {
    decoder
        .find_tag(tag)
        .map_err(|e| bad_image(&e))?
        .ok_or(DemError::BadPlacement(missing))?
        .into_f64_vec()
        .map_err(|e| bad_image(&e))
}

/// Where an image lies, in the units of its coordinate system.
struct Placement {
    pixel_size: (f64, f64),
    top_left: (f64, f64),
}

/// The pixel size and the north-west corner of a `width` × `height` image in `crs`, from its
/// `ModelPixelScale`, `ModelTiepoint` and geo keys.
fn read_placement(
    decoder: &mut Tiff<'_>,
    crs: DemCrs,
    (width, height): (u32, u32),
) -> Result<Placement, DemError> {
    let scale = f64_tag(decoder, Tag::ModelPixelScaleTag, "no ModelPixelScale")?;
    let tiepoint = f64_tag(decoder, Tag::ModelTiepointTag, "no ModelTiepoint")?;
    let keys = match decoder
        .find_tag(Tag::GeoKeyDirectoryTag)
        .map_err(|e| bad_image(&e))?
    {
        Some(value) => GeoKeys::read(&value.into_u16_vec().map_err(|e| bad_image(&e))?),
        None => GeoKeys::default(),
    };
    let (&[sx, sy, ..], &[ti, tj, _, tx, ty, ..]) = (scale.as_slice(), tiepoint.as_slice()) else {
        return Err(DemError::BadPlacement(
            "short ModelPixelScale or ModelTiepoint",
        ));
    };
    if ![sx, sy, ti, tj, tx, ty].iter().all(|v| v.is_finite()) || sx <= 0.0 || sy <= 0.0 {
        return Err(DemError::BadPlacement("pixel scale or tiepoint"));
    }
    let expected_model = match crs {
        DemCrs::WebMercator => 1,
        DemCrs::Wgs84 => 2,
    };
    if keys.model_type.is_some_and(|m| m != expected_model) {
        return Err(DemError::BadPlacement(
            "the model type is not the coordinate system asked for",
        ));
    }
    // A tiepoint refers to the outer corner of its pixel, or to its centre for "pixel is point".
    let centre = if keys.raster_type == Some(2) {
        0.5
    } else {
        0.0
    };
    let top_left = (tx - (ti + centre) * sx, ty + (tj + centre) * sy);
    let (right, bottom) = (
        top_left.0 + f64::from(width) * sx,
        top_left.1 - f64::from(height) * sy,
    );
    let limits = match crs {
        DemCrs::Wgs84 => (180.0, 90.0),
        DemCrs::WebMercator => (ORIGIN_M, ORIGIN_M),
    };
    // Allow a hundredth of a pixel past the edge of the world for rounding.
    let inside = |v: f64, limit: f64, step: f64| v.abs() <= limit + step * 0.01;
    if !(inside(top_left.0, limits.0, sx)
        && inside(right, limits.0, sx)
        && inside(top_left.1, limits.1, sy)
        && inside(bottom, limits.1, sy))
    {
        return Err(DemError::BadPlacement("the image lies outside the world"));
    }
    Ok(Placement {
        pixel_size: (sx, sy),
        top_left,
    })
}

/// The image's `GDAL_NODATA` value, else [`NO_DATA`].
fn read_no_data(decoder: &mut Tiff<'_>) -> Result<f32, DemError> {
    let Some(value) = decoder
        .find_tag(Tag::GdalNodata)
        .map_err(|e| bad_image(&e))?
    else {
        return Ok(NO_DATA);
    };
    let text = value.into_string().map_err(|e| bad_image(&e))?;
    text.trim_matches(|c: char| c == '\0' || c.is_whitespace())
        .parse::<f32>()
        .map_err(|_| DemError::BadPlacement("GDAL_NODATA is not a number"))
}

/// Decodes an `exportImage` TIFF of the coordinate system the request asked for.
///
/// The layout is the one the service sends: one float32 sample per pixel (`SampleFormat` 3),
/// uncompressed, in 128 × 128 tiles padded past the image edge (strips are read too). The
/// padding is dropped. The placement comes from `ModelPixelScale` and `ModelTiepoint`; a
/// `GDAL_NODATA` value (else [`NO_DATA`]) and any non-finite value become NaN.
///
/// # Errors
/// [`DemError::Service`] for the service's JSON error object, [`DemError::BadImage`] for bytes
/// that are not a TIFF the decoder reads, [`DemError::Unsupported`] for any other sample layout
/// or a compressed image, and [`DemError::BadPlacement`] for missing or unusable placement tags,
/// or a model type that contradicts `crs`.
#[allow(clippy::float_cmp)] // the no-data marker is an exact value, not a measurement
pub fn parse_export(body: &[u8], crs: DemCrs) -> Result<DemImage, DemError> {
    if let Some(error) = esri_error(body) {
        return Err(error);
    }
    let mut decoder = Decoder::new(Cursor::new(body)).map_err(|e| bad_image(&e))?;
    let (width, height) = check_layout(&mut decoder)?;
    let Placement {
        pixel_size,
        top_left,
    } = read_placement(&mut decoder, crs, (width, height))?;
    let no_data = read_no_data(&mut decoder)?;

    let mut result = DecodingResult::F32(Vec::new());
    decoder
        .read_image_to_buffer(&mut result)
        .map_err(|e| bad_image(&e))?;
    let DecodingResult::F32(mut values) = result else {
        return Err(DemError::Unsupported(
            "samples that are not float32".to_owned(),
        ));
    };
    let cells = width as usize * height as usize;
    if values.len() < cells {
        return Err(DemError::BadImage(format!(
            "{} values for {width} × {height} pixels",
            values.len()
        )));
    }
    values.truncate(cells);
    for v in &mut values {
        if !v.is_finite() || *v == no_data {
            *v = f32::NAN;
        }
    }
    Ok(DemImage {
        crs,
        width,
        height,
        pixel_size,
        top_left,
        values,
    })
}

#[derive(Deserialize)]
struct Identify {
    #[serde(default)]
    value: Option<serde_json::Value>,
}

/// The value of an `identify` response, metres positive up; `None` where the mosaic has no data
/// (`"NoData"`).
///
/// # Errors
/// [`DemError::Service`] for the service's error object and [`DemError::BadJson`] for anything
/// else that is not an identify result.
pub fn parse_identify(body: &[u8]) -> Result<Option<f32>, DemError> {
    if let Some(error) = esri_error(body) {
        return Err(error);
    }
    let identify: Identify =
        serde_json::from_slice(body).map_err(|e| DemError::BadJson(e.to_string()))?;
    let value = match identify.value {
        Some(serde_json::Value::String(text)) => text.trim().parse::<f32>().ok(),
        Some(serde_json::Value::Number(n)) => n.as_f64().map(|v| v as f32),
        Some(serde_json::Value::Null) | None => None,
        Some(other) => return Err(DemError::BadJson(format!("a value of {other}"))),
    };
    Ok(value.filter(|v| v.is_finite() && *v != NO_DATA))
}

/// One elevation model of the mosaic that covers a point.
#[derive(Clone, Debug, PartialEq)]
pub struct DemSource {
    /// The model's raster name, such as `ncei19_n40x75_w074x25_2015v1` or `crm_vol1_2023`.
    pub name: String,
    /// The model's cell size, arc-seconds.
    pub cell_arcsec: Option<f64>,
    /// The vertical datum as the service names it (`NAVD 88`, `EGM 2008`); `None` where it
    /// states none.
    pub vertical_datum: Option<String>,
    /// The day the model was completed (UTC).
    pub completed: Option<LocalDate>,
}

#[derive(Deserialize)]
struct QueryBody {
    features: Vec<Feature>,
}

#[derive(Deserialize)]
struct Feature {
    attributes: Attributes,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Attributes {
    name: Option<String>,
    cellsize_arcseconds: Option<f64>,
    vertical_datum: Option<String>,
    /// Milliseconds since 1970 (`esriFieldTypeDate`).
    date_completed: Option<f64>,
}

fn non_empty(text: Option<String>) -> Option<String> {
    text.map(|t| t.trim().to_owned()).filter(|t| !t.is_empty())
}

/// The source models of a `query` response, as the service lists them.
///
/// The mosaic's overviews (rasters named `Ov_…`, with neither a cell size nor a date) are not
/// sources and are left out.
///
/// # Errors
/// [`DemError::Service`] for the service's error object and [`DemError::BadJson`] for anything
/// else that is not a query result.
pub fn parse_query(body: &[u8]) -> Result<Vec<DemSource>, DemError> {
    if let Some(error) = esri_error(body) {
        return Err(error);
    }
    let query: QueryBody =
        serde_json::from_slice(body).map_err(|e| DemError::BadJson(e.to_string()))?;
    Ok(query
        .features
        .into_iter()
        .filter_map(|f| {
            let a = f.attributes;
            let name = non_empty(a.name)?;
            let cell_arcsec = a.cellsize_arcseconds.filter(|c| c.is_finite() && *c > 0.0);
            let completed = a
                .date_completed
                .filter(|ms| ms.is_finite())
                .map(|ms| UnixTime((ms / 1000.0).floor() as i64).local_date(UtcOffset(0)));
            if cell_arcsec.is_none() && completed.is_none() {
                return None;
            }
            Some(DemSource {
                name,
                cell_arcsec,
                vertical_datum: non_empty(a.vertical_datum),
                completed,
            })
        })
        .collect())
}

/// The finest source among `sources`: the one the mosaic draws at the point, whose vertical
/// datum the values there are relative to. Sources without a cell size are skipped.
pub fn finest(sources: &[DemSource]) -> Option<&DemSource> {
    sources
        .iter()
        .filter(|s| s.cell_arcsec.is_some())
        .min_by(|a, b| {
            a.cell_arcsec
                .partial_cmp(&b.cell_arcsec)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::Coordinates;
    use crate::testutil::{fixture_bytes, fixture_str};
    use underbite_core::map::GeoPoint;

    const EXPORT: &str = "dem-global-mosaic-export-f32.tif";
    const QUERY: &str = "dem-global-mosaic-query-battery.json";

    fn export() -> DemImage {
        parse_export(&fixture_bytes("bathymetry", EXPORT), DemCrs::Wgs84).unwrap()
    }

    /// The recorded request URLs, from the fixtures' README.
    fn readme_urls() -> Vec<String> {
        fixture_str("bathymetry", "README.md")
            .lines()
            .filter_map(|line| {
                let start = line.find("`https://")? + 1;
                let len = line[start..].find('`')?;
                Some(line[start..start + len].to_owned())
            })
            .collect()
    }

    fn point(lat: f64, lon: f64) -> RoundedPoint {
        RoundedPoint::new(
            Coordinates {
                latitude: lat,
                longitude: lon,
            },
            2,
        )
    }

    #[test]
    fn ncei_dem_export_decodes_the_recorded_tiff() {
        let image = export();
        assert_eq!(image.crs(), DemCrs::Wgs84);
        assert_eq!((image.width(), image.height()), (240, 160));
        assert_eq!(image.values().len(), 240 * 160);
        let (dx, dy) = image.pixel_size();
        assert!((dx - 0.005).abs() < 1e-9 && (dy - 0.005).abs() < 1e-9);
        let (west, north) = image.top_left();
        assert!((west + 74.2).abs() < 1e-9 && (north - 40.8).abs() < 1e-9);
        let (lo, hi) = image.range().unwrap();
        assert!((lo + 83.8).abs() < 0.1, "minimum {lo}");
        assert!((hi - 114.8).abs() < 0.1, "maximum {hi}");
        assert!(
            image
                .values()
                .iter()
                .all(|v| v.is_finite() && *v != NO_DATA)
        );
        // The corners of the image, read by the tile offsets.
        assert!((image.value(0, 0).unwrap() - 40.93).abs() < 0.01);
        assert!((image.value(159, 239).unwrap() + 51.36).abs() < 0.01);
        assert_eq!(image.value(160, 0), None);
        assert_eq!(image.value(0, 240), None);
    }

    #[test]
    fn ncei_dem_export_becomes_a_grid_at_cell_centres() {
        let grid = export().into_grid(UnixTime(0)).unwrap();
        assert_eq!(grid.source.node, GridHost::NceiDem);
        assert_eq!(grid.source.dataset, "DEM_global_mosaic");
        assert!((grid.source.native_step_deg - 0.005).abs() < 1e-9);
        assert_eq!((grid.lats.len(), grid.lons.len()), (160, 240));
        assert_eq!(grid.unit, "m");
        // Latitudes ascend after the north-to-south rows are reversed.
        assert!((grid.lats[0] - 40.0025).abs() < 1e-9);
        assert!((grid.lats[159] - 40.7975).abs() < 1e-9);
        assert!((grid.lons[0] + 74.1975).abs() < 1e-9);
        assert!((grid.lons[239] + 73.0025).abs() < 1e-9);
        assert_eq!(grid.valid_count(), 240 * 160);
        let at = |lat, lon| grid.sample_nearest(GeoPoint { lat, lon }).unwrap();
        // The Battery (40.7006 N, 74.0142 W): the land edge.
        assert!((at(40.7006, -74.0142) - 1.68).abs() < 0.5);
        // Ambrose Channel approach.
        assert!((at(40.4525, -73.8075) + 29.6).abs() < 0.5);
        // Sandy Hook Bay.
        assert!((at(40.4575, -74.0075) + 3.1).abs() < 0.5);
        // The south-east corner, offshore.
        assert!((at(40.0025, -73.0025) + 51.36).abs() < 0.01);
    }

    #[test]
    fn ncei_dem_area_request_reproduces_the_recorded_export() {
        let request = area_request(
            Bbox {
                west: -74.2,
                south: 40.0,
                east: -73.0,
                north: 40.8,
            },
            0.005,
        )
        .unwrap();
        assert_eq!(request.resource, DemResource::Export(DemCrs::Wgs84));
        let recorded = readme_urls();
        assert!(
            recorded.contains(&request.url),
            "{} is not in {recorded:#?}",
            request.url
        );
        assert_eq!(
            request.url,
            "https://gis.ngdc.noaa.gov/arcgis/rest/services/DEM_mosaics/DEM_global_mosaic/ImageServer/exportImage?bbox=-74.2,40.0,-73.0,40.8&bboxSR=4326&imageSR=4326&size=240,160&format=tiff&pixelType=F32&interpolation=RSP_BilinearInterpolation&noData=-9999&f=image"
        );
    }

    #[test]
    fn ncei_dem_query_request_matches_the_recorded_query_but_rounded() {
        let request = query_request(point(40.70, -74.015));
        assert_eq!(request.resource, DemResource::Query);
        // The recorded query was sent at -74.015,40.70; the app sends two decimals at most.
        let recorded = readme_urls()
            .into_iter()
            .find(|u| u.contains("/query?"))
            .unwrap()
            .replace("geometry=-74.015,40.70&", "geometry=-74.02,40.7&");
        assert_eq!(request.url, recorded);
    }

    #[test]
    fn ncei_dem_area_request_caps_the_image_and_keeps_cells_square() {
        let request = area_request(
            Bbox {
                west: -80.0,
                south: 30.0,
                east: -70.0,
                north: 35.0,
            },
            0.0001,
        )
        .unwrap();
        assert!(
            request.url.contains("&size=20000,10000&"),
            "{}",
            request.url
        );
        // A box past the edges of the world is clipped.
        let request = area_request(
            Bbox {
                west: -190.0,
                south: 80.0,
                east: -179.0,
                north: 95.0,
            },
            1.0,
        )
        .unwrap();
        assert!(
            request.url.contains("bbox=-180.0,80.0,-179.0,90.0&")
                && request.url.contains("size=1,10&"),
            "{}",
            request.url
        );
    }

    #[test]
    fn ncei_dem_area_request_rejects_bad_boxes_and_cells() {
        let bbox = Bbox {
            west: -74.2,
            south: 40.0,
            east: -73.0,
            north: 40.8,
        };
        for cell in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(
                area_request(bbox, cell),
                Err(DemError::InvalidRequest("cell size"))
            );
        }
        let flipped = Bbox {
            west: -73.0,
            east: -74.2,
            ..bbox
        };
        let nan = Bbox {
            north: f64::NAN,
            ..bbox
        };
        let flat = Bbox {
            south: 40.8,
            ..bbox
        };
        for b in [flipped, nan, flat] {
            assert_eq!(area_request(b, 0.005), Err(DemError::InvalidRequest("box")));
        }
    }

    #[test]
    fn ncei_dem_tile_request_uses_the_tile_bounds() {
        let key = TileKey {
            z: 12,
            x: 1205,
            y: 1540,
        };
        let request = tile_request(key, DEFAULT_TILE_PX).unwrap();
        assert_eq!(request.resource, DemResource::Export(DemCrs::WebMercator));
        let b = tile_bounds_3857(key);
        assert_eq!(
            request.url,
            format!(
                "{BASE_URL}/exportImage?bbox={},{},{},{}&bboxSR=3857&imageSR=3857&size=128,128&format=tiff&pixelType=F32&interpolation=RSP_BilinearInterpolation&noData=-9999&f=image",
                fmt_metres(b.min_x),
                fmt_metres(b.min_y),
                fmt_metres(b.max_x),
                fmt_metres(b.max_y)
            )
        );
        // s = 40075016.6856 / 2^12 = 9783.94 m: minx = -20037508.3428 + 1205 s.
        assert!(
            request
                .url
                .contains("bbox=-8247861.1,4960457.4,-8238077.2,4970241.3&"),
            "{}",
            request.url
        );
        let world = tile_request(TileKey { z: 0, x: 0, y: 0 }, 256).unwrap();
        assert!(world.url.contains(
            "bbox=-20037508.3,-20037508.3,20037508.3,20037508.3&bboxSR=3857&imageSR=3857&size=256,256&"
        ));
    }

    #[test]
    fn ncei_dem_tile_request_rejects_impossible_tiles() {
        let key = TileKey { z: 3, x: 8, y: 0 };
        assert_eq!(
            tile_request(key, 128),
            Err(DemError::InvalidRequest("tile index"))
        );
        let deep = TileKey { z: 25, x: 0, y: 0 };
        assert_eq!(
            tile_request(deep, 128),
            Err(DemError::InvalidRequest("zoom"))
        );
        let ok = TileKey { z: 3, x: 7, y: 7 };
        assert_eq!(
            tile_request(ok, 0),
            Err(DemError::InvalidRequest("image size"))
        );
        assert_eq!(
            tile_request(ok, MAX_IMAGE_PX + 1),
            Err(DemError::InvalidRequest("image size"))
        );
    }

    #[test]
    fn ncei_dem_point_requests_send_rounded_coordinates() {
        let identify = identify_request(point(40.701_234, -74.015_678));
        assert_eq!(identify.resource, DemResource::Identify);
        assert_eq!(
            identify.url,
            format!(
                "{BASE_URL}/identify?geometry=-74.02,40.7&geometryType=esriGeometryPoint&returnGeometry=false&returnCatalogItems=false&f=json"
            )
        );
        let query = query_request(point(40.701_234, -74.015_678));
        assert!(query.url.contains("geometry=-74.02,40.7&"));
        assert!(query.url.contains(
            "outFields=Name,ProductName,CellsizeArcseconds,VerticalDatum,DateCompleted,"
        ));
    }

    #[test]
    fn ncei_dem_query_lists_the_sources_with_their_datums() {
        let sources = parse_query(&fixture_bytes("bathymetry", QUERY)).unwrap();
        assert_eq!(sources.len(), 3);
        assert_eq!(
            sources[0],
            DemSource {
                name: "ncei19_n40x75_w074x25_2015v1".to_owned(),
                cell_arcsec: Some(0.111_111_112),
                vertical_datum: Some("NAVD 88".to_owned()),
                completed: Some(LocalDate {
                    year: 2021,
                    month: 8,
                    day: 20
                }),
            }
        );
        assert_eq!(sources[1].name, "ETOPO_2022_v1_15s_surface_elev");
        assert_eq!(sources[1].cell_arcsec, Some(15.0));
        assert_eq!(sources[1].vertical_datum, None);
        assert_eq!(sources[2].name, "crm_vol1_2023");
        assert_eq!(sources[2].vertical_datum.as_deref(), Some("EGM 2008"));
        assert_eq!(
            sources[2].completed,
            Some(LocalDate {
                year: 2023,
                month: 9,
                day: 1
            })
        );
        let datums: Vec<_> = sources
            .iter()
            .filter_map(|s| s.vertical_datum.as_deref())
            .collect();
        assert!(datums.contains(&"NAVD 88") && datums.contains(&"EGM 2008"));
        let best = finest(&sources).unwrap();
        assert_eq!(best.name, "ncei19_n40x75_w074x25_2015v1");
        assert_eq!(finest(&[]), None);
    }

    #[test]
    fn ncei_dem_identify_reads_the_string_value() {
        let body = br#"{"objectId":0,"name":"Pixel","value":"-7.51293","location":{"x":-74.015,"y":40.7,"spatialReference":{"wkid":4326}},"properties":null,"catalogItems":null,"catalogItemVisibilities":[]}"#;
        let value = parse_identify(body).unwrap().unwrap();
        assert!((value + 7.512_93).abs() < 1e-5);
        let none = br#"{"objectId":0,"name":"Pixel","value":"NoData","location":{}}"#;
        assert_eq!(parse_identify(none).unwrap(), None);
        let sentinel = br#"{"objectId":0,"name":"Pixel","value":"-9999"}"#;
        assert_eq!(parse_identify(sentinel).unwrap(), None);
        assert!(matches!(
            parse_identify(b"not json"),
            Err(DemError::BadJson(_))
        ));
    }

    #[test]
    fn ncei_dem_service_errors_are_found_in_200_responses() {
        let body = br#"{"error":{"code":400,"message":"Unable to complete operation.","details":["Invalid bbox"]}}"#;
        let expected = DemError::Service {
            code: Some(400),
            message: "Unable to complete operation.: Invalid bbox".to_owned(),
        };
        assert_eq!(
            check_response(&Response::new(200, body)),
            Err(expected.clone())
        );
        assert_eq!(parse_export(body, DemCrs::Wgs84), Err(expected.clone()));
        assert_eq!(parse_query(body), Err(expected.clone()));
        assert_eq!(parse_identify(body), Err(expected));
        assert_eq!(
            check_response(&Response::new(502, b"Bad gateway")),
            Err(DemError::Status { status: 502 })
        );
        let page = Response {
            content_type: Some("text/html; charset=utf-8"),
            ..Response::new(200, b"<html></html>")
        };
        assert!(matches!(check_response(&page), Err(DemError::BadImage(_))));
        let tiff = fixture_bytes("bathymetry", EXPORT);
        let ok = Response {
            content_type: Some("image/tiff"),
            ..Response::new(200, &tiff)
        };
        assert_eq!(check_response(&ok).unwrap().len(), tiff.len());
    }

    #[test]
    fn ncei_dem_export_rejects_the_wrong_coordinate_system() {
        let tiff = fixture_bytes("bathymetry", EXPORT);
        assert!(matches!(
            parse_export(&tiff, DemCrs::WebMercator),
            Err(DemError::BadPlacement(_))
        ));
    }

    #[test]
    fn ncei_dem_export_rejects_other_files_without_panicking() {
        let tiff = fixture_bytes("bathymetry", EXPORT);
        assert!(matches!(
            parse_export(b"", DemCrs::Wgs84),
            Err(DemError::BadImage(_))
        ));
        assert!(matches!(
            parse_export(b"\x89PNG\r\n\x1a\n", DemCrs::Wgs84),
            Err(DemError::BadImage(_))
        ));
        let png = fixture_bytes("bathymetry", "gebco-wms-getmap-3857-z8-tile.png");
        assert!(parse_export(&png, DemCrs::WebMercator).is_err());
        // Cut before the data it needs ends, the file fails cleanly (the last tile is read only as
        // far as the image goes, so its padding may be missing).
        for len in [8, 100, 400, 856, 1000, 66_392, 197_464 + 1_000, 200_000] {
            assert!(
                parse_export(&tiff[..len], DemCrs::Wgs84).is_err(),
                "cut at {len}"
            );
        }
    }

    /// Rewrites the value of one IFD entry of the fixture (a short held in place).
    fn with_short_tag(tag: u16, value: u16) -> Vec<u8> {
        let mut tiff = fixture_bytes("bathymetry", EXPORT);
        let ifd = u32::from_le_bytes(tiff[4..8].try_into().unwrap()) as usize;
        let count = u16::from_le_bytes([tiff[ifd], tiff[ifd + 1]]) as usize;
        let entry = (0..count)
            .map(|i| ifd + 2 + 12 * i)
            .find(|&e| u16::from_le_bytes([tiff[e], tiff[e + 1]]) == tag)
            .unwrap();
        tiff[entry + 8..entry + 10].copy_from_slice(&value.to_le_bytes());
        tiff
    }

    #[test]
    fn ncei_dem_export_rejects_other_sample_layouts() {
        // Deflate.
        assert_eq!(
            parse_export(&with_short_tag(259, 8), DemCrs::Wgs84),
            Err(DemError::Unsupported("compression 8".to_owned()))
        );
        // Signed integers rather than floats.
        assert!(matches!(
            parse_export(&with_short_tag(339, 2), DemCrs::Wgs84),
            Err(DemError::Unsupported(_) | DemError::BadImage(_))
        ));
        // 16-bit samples.
        assert!(parse_export(&with_short_tag(258, 16), DemCrs::Wgs84).is_err());
    }

    #[test]
    fn ncei_dem_web_mercator_rows_are_inverse_mercator() {
        // A Web Mercator placement at the 69.578 m pixels of a recorded export.
        let image = DemImage {
            crs: DemCrs::WebMercator,
            width: 2,
            height: 3,
            pixel_size: (69.578, 69.578),
            top_left: (-8_243_208.0, 4_974_067.0),
            values: vec![1.0, 2.0, 3.0, 4.0, f32::NAN, 6.0],
        };
        let lats = image.lats_north_to_south();
        let lons = image.lons();
        let r = 6_378_137.0_f64;
        let y = 4_974_067.0 - 0.5 * 69.578;
        let expected = (y / r).sinh().atan().to_degrees();
        assert!((lats[0] - expected).abs() < 1e-9);
        assert!((lats[0] - 40.7398).abs() < 1e-3, "{}", lats[0]);
        assert!(lats.windows(2).all(|w| w[0] > w[1]));
        let x = -8_243_208.0 + 0.5 * 69.578;
        assert!((lons[0] - (x / r).to_degrees()).abs() < 1e-9);
        assert!((lons[0] + 74.0496).abs() < 1e-3, "{}", lons[0]);
        let grid = image.into_grid(UnixTime(0)).unwrap();
        // The southern row comes first in the grid.
        assert_eq!(grid.value(0, 1), Some(6.0));
        assert!(grid.value(0, 0).unwrap().is_nan());
        assert_eq!(grid.value(2, 0), Some(1.0));
        assert_eq!(grid.valid_count(), 5);
    }

    #[test]
    fn ncei_dem_geo_keys_read_model_and_raster_type() {
        let keys = GeoKeys::read(&[1, 1, 0, 3, 1024, 0, 1, 2, 1025, 0, 1, 2, 2048, 0, 1, 4326]);
        assert_eq!((keys.model_type, keys.raster_type), (Some(2), Some(2)));
        // Keys held elsewhere, and short directories, are skipped.
        let keys = GeoKeys::read(&[1, 1, 0, 2, 1024, 34736, 1, 0, 1025]);
        assert_eq!((keys.model_type, keys.raster_type), (None, None));
        let keys = GeoKeys::read(&[1, 1]);
        assert_eq!((keys.model_type, keys.raster_type), (None, None));
    }
}
