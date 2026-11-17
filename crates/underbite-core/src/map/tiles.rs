//! Web Mercator tile addressing and the URLs of tiled map sources.
//!
//! The core builds every tile URL. A map engine's `{z}/{x}/{y}` template cannot express the NOAA
//! chart service's level offset or a WMS bounding box, so displays ask for the URL of a
//! [`TileKey`] and fetch it, then ask [`classify`] what came back.
//!
//! The geometry is spherical Web Mercator (EPSG:3857). A tile's `y` counts down from the north,
//! as in the XYZ scheme; GIBS and the NOAA tile cache use the same origin.

use crate::map::{Bbox, GeoPoint, TileKey};
use crate::time::{LocalDate, UnixTime, UtcOffset};
use whether_core::time::{civil_from_days, days_from_civil};

/// The side of the Web Mercator world square in metres.
pub const WORLD_M: f64 = 40_075_016.685_6;
/// Half of [`WORLD_M`]: the coordinate of the square's edges.
pub const ORIGIN_M: f64 = 20_037_508.342_8;
/// The deepest zoom the geometry functions work at (a `u32` tile index and `f64` stay exact).
pub const MAX_ZOOM: u8 = 24;
/// The latitude where Web Mercator ends.
pub const MAX_LATITUDE: f64 = 85.051_128_779_8;
/// The pixel side of every tile the catalogue asks for.
pub const TILE_PX: u32 = 256;

const EARTH_RADIUS_M: f64 = 6_371_008.8;

/// A box in EPSG:3857 metres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds3857 {
    pub min_x: f64,
    pub min_y: f64,
    pub max_x: f64,
    pub max_y: f64,
}

fn clamp_zoom(z: u8) -> u8 {
    z.min(MAX_ZOOM)
}

fn tiles_per_side(z: u8) -> f64 {
    f64::from(1u32 << clamp_zoom(z))
}

/// The north-west corner of a tile.
pub fn tile_to_lonlat(key: TileKey) -> GeoPoint {
    let n = tiles_per_side(key.z);
    let lon = f64::from(key.x) / n * 360.0 - 180.0;
    let lat = (std::f64::consts::PI * (1.0 - 2.0 * f64::from(key.y) / n))
        .sinh()
        .atan()
        .to_degrees();
    GeoPoint { lat, lon }
}

/// A tile's extent in degrees.
pub fn tile_bbox(key: TileKey) -> Bbox {
    let nw = tile_to_lonlat(key);
    let se = tile_to_lonlat(TileKey {
        z: key.z,
        x: key.x.saturating_add(1),
        y: key.y.saturating_add(1),
    });
    Bbox {
        west: nw.lon,
        south: se.lat,
        east: se.lon,
        north: nw.lat,
    }
}

/// A tile's EPSG:3857 bounds.
pub fn tile_bounds_3857(key: TileKey) -> Bounds3857 {
    let s = WORLD_M / tiles_per_side(key.z);
    let min_x = -ORIGIN_M + f64::from(key.x) * s;
    let max_y = ORIGIN_M - f64::from(key.y) * s;
    Bounds3857 {
        min_x,
        min_y: max_y - s,
        max_x: min_x + s,
        max_y,
    }
}

/// The tile at a zoom that holds a point; a point beyond the Mercator limit or a value that is not
/// a number lands on the nearest edge tile.
pub fn lonlat_to_tile(point: GeoPoint, z: u8) -> TileKey {
    let (fx, fy) = fractional_tile(point, z);
    let n = tiles_per_side(z);
    TileKey {
        z: clamp_zoom(z),
        x: fx.floor().clamp(0.0, n - 1.0) as u32,
        y: fy.floor().clamp(0.0, n - 1.0) as u32,
    }
}

fn fractional_tile(point: GeoPoint, z: u8) -> (f64, f64) {
    let n = tiles_per_side(z);
    let lat = point.lat.clamp(-MAX_LATITUDE, MAX_LATITUDE).to_radians();
    let x = (point.lon + 180.0) / 360.0 * n;
    let y = (1.0 - (lat.tan() + 1.0 / lat.cos()).ln() / std::f64::consts::PI) / 2.0 * n;
    (x, y)
}

/// The tiles at a zoom that cover a box, row by row from the north-west, at most `limit` of them.
/// A box edge that lies exactly on a tile edge does not pull in the next tile.
pub fn tiles_covering(bbox: Bbox, z: u8, limit: usize) -> Vec<TileKey> {
    let z = clamp_zoom(z);
    let n = tiles_per_side(z);
    let (fx0, fy0) = fractional_tile(
        GeoPoint {
            lat: bbox.north,
            lon: bbox.west,
        },
        z,
    );
    let (fx1, fy1) = fractional_tile(
        GeoPoint {
            lat: bbox.south,
            lon: bbox.east,
        },
        z,
    );
    let last = n - 1.0;
    let x0 = (fx0 + 1e-7).floor().clamp(0.0, last);
    let y0 = (fy0 + 1e-7).floor().clamp(0.0, last);
    // Rounding can leave an edge a hair past a tile boundary; a tenth of a millionth of a tile
    // is well under a centimetre at any zoom.
    let x1 = ((fx1 - 1e-7).ceil() - 1.0).clamp(x0, last);
    let y1 = ((fy1 - 1e-7).ceil() - 1.0).clamp(y0, last);
    let mut out = Vec::new();
    let mut y = y0 as u32;
    while f64::from(y) <= y1 {
        let mut x = x0 as u32;
        while f64::from(x) <= x1 {
            if out.len() >= limit {
                return out;
            }
            out.push(TileKey { z, x, y });
            x += 1;
        }
        y += 1;
    }
    out
}

/// The great-circle distance between two points in metres (haversine, mean earth radius).
pub fn haversine_m(a: GeoPoint, b: GeoPoint) -> f64 {
    let (p1, p2) = (a.lat.to_radians(), b.lat.to_radians());
    let dp = p2 - p1;
    let dl = (b.lon - a.lon).to_radians();
    let h = (dp / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);
    2.0 * EARTH_RADIUS_M * h.sqrt().min(1.0).asin()
}

/// The initial bearing from `a` to `b` in degrees clockwise from true north, 0 up to 360.
pub fn bearing_deg(a: GeoPoint, b: GeoPoint) -> f64 {
    let (p1, p2) = (a.lat.to_radians(), b.lat.to_radians());
    let dl = (b.lon - a.lon).to_radians();
    let y = dl.sin() * p2.cos();
    let x = p1.cos() * p2.sin() - p1.sin() * p2.cos() * dl.cos();
    y.atan2(x).to_degrees().rem_euclid(360.0)
}

/// How to show a tile deeper than a layer serves: take the ancestor at the layer's deepest zoom and
/// cut the part that the requested tile covers.
///
/// `MapKit` does not over-zoom a tile overlay beyond its maximum zoom, so a display that wants
/// imagery at a deeper zoom fetches [`ancestor`](Self::ancestor), crops
/// [`pixel_rect`](Self::pixel_rect) out of it and scales that to the tile size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OverZoom {
    /// The tile to fetch: the requested tile's ancestor at the layer's maximum zoom (the tile itself
    /// when it is not deeper).
    pub ancestor: TileKey,
    /// The left edge of the wanted part, as a fraction (0 to 1) of the ancestor's width.
    pub left: f64,
    /// The top edge of the wanted part, as a fraction of the ancestor's height.
    pub top: f64,
    /// The side of the wanted part as a fraction of the ancestor's side (1 when not over-zoomed).
    pub size: f64,
}

/// The ancestor of `key` at `max_zoom` and the part of it that `key` covers.
pub fn over_zoom(key: TileKey, max_zoom: u8) -> OverZoom {
    if key.z <= max_zoom {
        return OverZoom {
            ancestor: key,
            left: 0.0,
            top: 0.0,
            size: 1.0,
        };
    }
    let dz = u32::from(key.z - max_zoom);
    let (ax, ay) = (
        key.x.checked_shr(dz).unwrap_or(0),
        key.y.checked_shr(dz).unwrap_or(0),
    );
    let span = 2f64.powi(i32::try_from(dz).unwrap_or(i32::MAX));
    let size = 1.0 / span;
    // The offset inside the ancestor, in tiles of the requested zoom.
    let ox = f64::from(key.x) - f64::from(ax) * span;
    let oy = f64::from(key.y) - f64::from(ay) * span;
    OverZoom {
        ancestor: TileKey {
            z: max_zoom,
            x: ax,
            y: ay,
        },
        left: ox * size,
        top: oy * size,
        size,
    }
}

impl OverZoom {
    /// The part to cut from an ancestor image `tile_px` pixels on a side, as `(x, y, side)` in
    /// pixels. The side drops below one pixel once the requested zoom is more than
    /// `log2(tile_px)` levels deeper than the ancestor's.
    pub fn pixel_rect(&self, tile_px: u32) -> (f64, f64, f64) {
        let px = f64::from(tile_px);
        (self.left * px, self.top * px, self.size * px)
    }
}

/// The GIBS imagery of one layer: its WMTS id, matrix set and tile format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GibsSpec {
    /// The WMTS layer id, such as `GHRSST_L4_MUR_Sea_Surface_Temperature`.
    pub layer: &'static str,
    /// The tile matrix set, such as `GoogleMapsCompatible_Level7`; its level is the deepest zoom.
    pub matrix_set: &'static str,
    /// The file extension: `png` or `jpg`.
    pub ext: &'static str,
    /// The deepest zoom the matrix set has; a deeper request answers 400.
    pub max_zoom: u8,
}

const GIBS_WMTS: &str = "https://gibs.earthdata.nasa.gov/wmts/epsg3857/best";
const NCDS_WMTS: &str = "https://gis.charttools.noaa.gov/arcgis/rest/services/MarineChart_Services/NOAACharts/MapServer/WMTS/tile/1.0.0/MarineChart_Services_NOAACharts/default/default028mm";
const NCDS_WMS: &str = "https://gis.charttools.noaa.gov/arcgis/rest/services/MCS/NOAAChartDisplay/MapServer/exts/MaritimeChartService/WMSServer";
const GEBCO_WMS: &str = "https://wms.gebco.net/mapserv";
const IEM: &str = "https://mesonet.agron.iastate.edu";

/// The NOAA chart tile cache's zooms: its level 0 is Web Mercator zoom 2 and its level 18 is 20.
pub const NCDS_ZOOMS: std::ops::RangeInclusive<u8> = 2..=20;

/// The GIBS tile of a layer. `date` is a UTC day; none asks for `default`, today's partial tile.
pub fn gibs_tile_url(spec: &GibsSpec, date: Option<LocalDate>, key: TileKey) -> String {
    let when = date.map_or_else(
        || String::from("default"),
        |d| format!("{:04}-{:02}-{:02}", d.year, d.month, d.day),
    );
    format!(
        "{GIBS_WMTS}/{}/default/{when}/{}/{}/{}/{}.{}",
        spec.layer, spec.matrix_set, key.z, key.y, key.x, spec.ext
    )
}

/// The pre-rendered NOAA chart tile (row before column, level = zoom - 2); none outside zoom 2 to
/// 20, where the cache has no level.
pub fn ncds_wmts_url(key: TileKey) -> Option<String> {
    if !NCDS_ZOOMS.contains(&key.z) {
        return None;
    }
    Some(format!("{NCDS_WMTS}/{}/{}/{}.png", key.z - 2, key.y, key.x))
}

/// Metres as the services take them: one decimal, and none for a whole number.
fn metres(v: f64) -> String {
    let s = format!("{v:.1}");
    match s.strip_suffix(".0") {
        Some(whole) => whole.to_owned(),
        None => s,
    }
}

fn bbox_param(b: Bounds3857) -> String {
    format!(
        "{},{},{},{}",
        metres(b.min_x),
        metres(b.min_y),
        metres(b.max_x),
        metres(b.max_y)
    )
}

/// A NOAA Chart Display WMS `GetMap` for any box: `groups` is a comma list of display group ids
/// such as `2,3`.
pub fn ncds_wms_bbox_url(groups: &str, bounds: Bounds3857) -> String {
    format!(
        "{NCDS_WMS}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS={groups}&STYLES=&CRS=EPSG:3857&BBOX={}&WIDTH={TILE_PX}&HEIGHT={TILE_PX}&FORMAT=image/png&TRANSPARENT=TRUE",
        bbox_param(bounds)
    )
}

/// A NOAA Chart Display WMS `GetMap` for one tile.
pub fn ncds_wms_url(groups: &str, key: TileKey) -> String {
    ncds_wms_bbox_url(groups, tile_bounds_3857(key))
}

/// The GEBCO shaded-relief WMS (`GEBCO_LATEST_2`) for one tile.
pub fn gebco_wms_url(key: TileKey) -> String {
    format!(
        "{GEBCO_WMS}?service=WMS&version=1.3.0&request=GetMap&layers=GEBCO_LATEST_2&styles=&crs=EPSG:3857&bbox={}&width={TILE_PX}&height={TILE_PX}&format=image/png&transparent=true",
        bbox_param(tile_bounds_3857(key))
    )
}

/// The minutes between NEXRAD composite frames.
pub const RADAR_FRAME_STEP_MIN: u32 = 5;

/// The NEXRAD composite tile (Iowa Environmental Mesonet) `minutes_ago` old, rounded down to a
/// multiple of five; 0 is the latest.
pub fn radar_tile_url(minutes_ago: u32, key: TileKey) -> String {
    let m = minutes_ago - minutes_ago % RADAR_FRAME_STEP_MIN;
    let layer = if m == 0 {
        String::from("nexrad-n0q-900913")
    } else {
        format!("nexrad-n0q-900913-m{m:02}m")
    };
    format!(
        "{IEM}/cache/tile.py/1.0.0/{layer}/{}/{}/{}.png",
        key.z, key.x, key.y
    )
}

/// The GIBS date to ask for: yesterday in UTC, since today's tile is only partly covered.
pub fn gibs_default_date(now: UnixTime) -> LocalDate {
    previous_day(now.local_date(UtcOffset(0)))
}

/// The day before a date, for a day that has no data.
pub fn previous_day(date: LocalDate) -> LocalDate {
    let (year, month, day) = civil_from_days(days_from_civil(date.year, date.month, date.day) - 1);
    LocalDate { year, month, day }
}

/// A service that serves tiles; each answers failure in its own way.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TileService {
    /// NASA GIBS WMTS.
    Gibs,
    /// NOAA Chart Display (the WMTS cache and the WMS).
    Ncds,
    /// The GEBCO WMS.
    Gebco,
    /// The NEXRAD tiles of the Iowa Environmental Mesonet.
    Radar,
}

/// What a tile response means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TileOutcome {
    /// An image to draw.
    Image,
    /// The service has nothing for that date or place; try the previous day or draw nothing.
    NoData,
    /// The request is deeper than the layer's matrix set; use [`over_zoom`] instead.
    TooDeep,
    /// Ask again after this many seconds.
    Retry { after_s: u32 },
    /// Give up on the tile.
    Failed,
}

/// Retries after an HTML or error body: once.
const RETRIES_AFTER_BAD_BODY: u32 = 1;
/// Retries after 429 or 503: twice.
const RETRIES_AFTER_BUSY: u32 = 2;

/// Classifies a tile response. `attempt` counts the requests already made for this tile (0 on the
/// first response), so that a bad answer is retried a bounded number of times.
///
/// An image counts only when the status is 200, `content_type` is an image type and the body is not
/// empty: the NOAA chart service answers a failure with status 200 and an HTML page or a JSON
/// error body.
pub fn classify(
    service: TileService,
    status: u16,
    content_type: Option<&str>,
    body_len: usize,
    attempt: u32,
) -> TileOutcome {
    match status {
        200 => {
            let is_image = content_type.is_some_and(|c| {
                let base = c
                    .split(';')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_ascii_lowercase();
                base == "image/png" || (service == TileService::Gibs && base == "image/jpeg")
            });
            if is_image && body_len > 0 {
                TileOutcome::Image
            } else if attempt < RETRIES_AFTER_BAD_BODY {
                TileOutcome::Retry { after_s: 3 }
            } else {
                TileOutcome::Failed
            }
        }
        404 if matches!(service, TileService::Gibs | TileService::Radar) => TileOutcome::NoData,
        400 if service == TileService::Gibs => TileOutcome::TooDeep,
        429 if attempt < RETRIES_AFTER_BUSY => TileOutcome::Retry { after_s: 30 },
        503 if attempt < RETRIES_AFTER_BUSY => TileOutcome::Retry { after_s: 10 },
        _ => TileOutcome::Failed,
    }
}

/// How long a cached tile stays good, in seconds, from the response's `Cache-Control` header
/// (`no-store` and `no-cache` give 0, `max-age` gives its value), or the service's usual lifetime
/// when the header is missing or has neither: a day for GIBS dated tiles and the chart service, 30
/// days for GEBCO, five minutes for radar.
pub fn cache_lifetime_s(service: TileService, cache_control: Option<&str>) -> u64 {
    if let Some(header) = cache_control {
        let mut max_age = None;
        for part in header.split(',') {
            let part = part.trim().to_ascii_lowercase();
            if part == "no-store" || part == "no-cache" {
                return 0;
            }
            if let Some(v) = part.strip_prefix("max-age=") {
                max_age = v.trim().parse::<u64>().ok();
            }
        }
        if let Some(v) = max_age {
            return v;
        }
    }
    match service {
        TileService::Gibs | TileService::Ncds => 86_400,
        TileService::Gebco => 2_630_000,
        TileService::Radar => 300,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tile(z: u8, x: u32, y: u32) -> TileKey {
        TileKey { z, x, y }
    }

    const MUR: GibsSpec = GibsSpec {
        layer: "GHRSST_L4_MUR_Sea_Surface_Temperature",
        matrix_set: "GoogleMapsCompatible_Level7",
        ext: "png",
        max_zoom: 7,
    };

    fn date(year: i32, month: u8, day: u8) -> LocalDate {
        LocalDate { year, month, day }
    }

    #[test]
    fn corners_and_bounds_follow_the_standard_scheme() {
        let nw = tile_to_lonlat(tile(0, 0, 0));
        assert_eq!(nw.lon, -180.0);
        assert!((nw.lat - MAX_LATITUDE).abs() < 1e-6);
        let b = tile_bounds_3857(tile(1, 1, 1));
        assert!((b.min_x - 0.0).abs() < 1e-6 && (b.max_x - ORIGIN_M).abs() < 1e-6);
        assert!((b.max_y - 0.0).abs() < 1e-6 && (b.min_y + ORIGIN_M).abs() < 1e-6);
        // Tile 7/48/37 spans -75.94 to -73.13 and 38.82 to 40.98 degrees.
        let d = tile_bbox(tile(7, 37, 48));
        assert!((d.west + 75.9375).abs() < 1e-9 && (d.east + 73.125).abs() < 1e-9);
        assert!((d.north - 40.979_898).abs() < 1e-5 && (d.south - 38.822_591).abs() < 1e-5);
    }

    #[test]
    fn a_point_finds_its_tile() {
        let battery = GeoPoint {
            lat: 40.7006,
            lon: -74.0142,
        };
        assert_eq!(lonlat_to_tile(battery, 14), tile(14, 4823, 6160));
        assert_eq!(lonlat_to_tile(battery, 7), tile(7, 37, 48));
        let edge = lonlat_to_tile(
            GeoPoint {
                lat: 90.0,
                lon: 180.0,
            },
            3,
        );
        assert_eq!(edge, tile(3, 7, 0));
        let nan = lonlat_to_tile(
            GeoPoint {
                lat: f64::NAN,
                lon: f64::NAN,
            },
            3,
        );
        assert_eq!(nan.z, 3);
    }

    #[test]
    fn covering_tiles_stop_at_exact_edges() {
        let one = tiles_covering(tile_bbox(tile(7, 37, 48)), 7, 100);
        assert_eq!(one, [tile(7, 37, 48)]);
        let four = tiles_covering(tile_bbox(tile(7, 37, 48)), 8, 100);
        assert_eq!(
            four,
            [
                tile(8, 74, 96),
                tile(8, 75, 96),
                tile(8, 74, 97),
                tile(8, 75, 97)
            ]
        );
        assert_eq!(tiles_covering(tile_bbox(tile(7, 37, 48)), 8, 3).len(), 3);
        let world = Bbox {
            west: -180.0,
            south: -90.0,
            east: 180.0,
            north: 90.0,
        };
        assert_eq!(tiles_covering(world, 2, 1000).len(), 16);
    }

    #[test]
    fn distance_and_bearing_between_known_points() {
        let battery = GeoPoint {
            lat: 40.7006,
            lon: -74.0142,
        };
        let buoy = GeoPoint {
            lat: 40.251,
            lon: -73.164,
        };
        let d = haversine_m(battery, buoy);
        assert!((d - 87_583.0).abs() < 50.0, "{d}");
        let b = bearing_deg(battery, buoy);
        assert!((b - 125.0).abs() < 1.5, "{b}");
        assert_eq!(haversine_m(battery, battery), 0.0);
        let origin = GeoPoint { lat: 0.0, lon: 0.0 };
        let east = bearing_deg(origin, GeoPoint { lat: 0.0, lon: 1.0 });
        assert!((east - 90.0).abs() < 1e-9);
        let west = bearing_deg(
            origin,
            GeoPoint {
                lat: 0.0,
                lon: -1.0,
            },
        );
        assert!((west - 270.0).abs() < 1e-9);
        let quarter = haversine_m(
            origin,
            GeoPoint {
                lat: 0.0,
                lon: 90.0,
            },
        );
        assert!((quarter - EARTH_RADIUS_M * std::f64::consts::FRAC_PI_2).abs() < 1e-3);
    }

    #[test]
    fn over_zoom_cuts_the_part_of_the_ancestor() {
        let same = over_zoom(tile(7, 37, 48), 7);
        assert_eq!(
            (same.ancestor, same.left, same.top, same.size),
            (tile(7, 37, 48), 0.0, 0.0, 1.0)
        );
        let nw = over_zoom(tile(8, 74, 96), 7);
        assert_eq!(nw.ancestor, tile(7, 37, 48));
        assert_eq!((nw.left, nw.top, nw.size), (0.0, 0.0, 0.5));
        let se = over_zoom(tile(8, 75, 97), 7);
        assert_eq!(se.ancestor, tile(7, 37, 48));
        assert_eq!((se.left, se.top, se.size), (0.5, 0.5, 0.5));
        assert_eq!(se.pixel_rect(256), (128.0, 128.0, 128.0));
        // Three levels deeper: the last tile of the ancestor's row, the first of its column.
        let deep_key = tile(10, 37 * 8 + 7, 48 * 8);
        let deep = over_zoom(deep_key, 7);
        assert_eq!(deep.ancestor, tile(7, 37, 48));
        assert_eq!((deep.left, deep.top, deep.size), (0.875, 0.0, 0.125));
        assert_eq!(deep.pixel_rect(256), (224.0, 0.0, 32.0));
        // The ancestor's bounds contain the requested tile's bounds.
        let (a, t) = (tile_bounds_3857(deep.ancestor), tile_bounds_3857(deep_key));
        assert!(a.min_x <= t.min_x + 1e-6 && t.max_x <= a.max_x + 1e-6);
        assert!(a.min_y <= t.min_y + 1e-6 && t.max_y <= a.max_y + 1e-6);
        // The cut part is where the requested tile's centre falls inside the ancestor.
        let ab = tile_bounds_3857(deep.ancestor);
        let left = ab.min_x + deep.left * (ab.max_x - ab.min_x);
        assert!((left - t.min_x).abs() < 1e-3);
        // The corners of the world: the zoom-0 ancestor holds everything.
        let last = over_zoom(tile(5, 31, 31), 0);
        assert_eq!(last.ancestor, tile(0, 0, 0));
        assert_eq!(
            (last.left, last.top, last.size),
            (31.0 / 32.0, 31.0 / 32.0, 1.0 / 32.0)
        );
        // Far too deep: no panic, a sliver.
        let far = over_zoom(tile(200, u32::MAX, u32::MAX), 7);
        assert_eq!(far.ancestor.z, 7);
        assert!(far.size < 1e-9);
    }

    #[test]
    fn gibs_urls_match_the_recorded_tiles() {
        let key = tile(7, 37, 48);
        let d = Some(date(2026, 11, 7));
        assert_eq!(
            gibs_tile_url(&MUR, d, key),
            "https://gibs.earthdata.nasa.gov/wmts/epsg3857/best/GHRSST_L4_MUR_Sea_Surface_Temperature/default/2026-11-07/GoogleMapsCompatible_Level7/7/48/37.png"
        );
        let chl = GibsSpec {
            layer: "VIIRS_NOAA20_Chlorophyll_a",
            ..MUR
        };
        assert_eq!(
            gibs_tile_url(&chl, d, key),
            "https://gibs.earthdata.nasa.gov/wmts/epsg3857/best/VIIRS_NOAA20_Chlorophyll_a/default/2026-11-07/GoogleMapsCompatible_Level7/7/48/37.png"
        );
        let tc = GibsSpec {
            layer: "VIIRS_NOAA20_CorrectedReflectance_TrueColor",
            matrix_set: "GoogleMapsCompatible_Level9",
            ext: "jpg",
            max_zoom: 9,
        };
        assert_eq!(
            gibs_tile_url(&tc, d, key),
            "https://gibs.earthdata.nasa.gov/wmts/epsg3857/best/VIIRS_NOAA20_CorrectedReflectance_TrueColor/default/2026-11-07/GoogleMapsCompatible_Level9/7/48/37.jpg"
        );
        assert!(gibs_tile_url(&MUR, None, key).contains("/default/default/"));
    }

    #[test]
    fn ncds_urls_match_the_recorded_tiles() {
        let base = "https://gis.charttools.noaa.gov/arcgis/rest/services/MarineChart_Services/NOAACharts/MapServer/WMTS/tile/1.0.0/MarineChart_Services_NOAACharts/default/default028mm";
        assert_eq!(
            ncds_wmts_url(tile(14, 4823, 6160)).as_deref(),
            Some(format!("{base}/12/6160/4823.png").as_str())
        );
        assert_eq!(
            ncds_wmts_url(tile(16, 19291, 24640)).as_deref(),
            Some(format!("{base}/14/24640/19291.png").as_str())
        );
        assert_eq!(ncds_wmts_url(tile(1, 0, 0)), None);
        assert_eq!(ncds_wmts_url(tile(21, 0, 0)), None);
        assert!(ncds_wmts_url(tile(2, 0, 0)).unwrap().contains("/0/0/0.png"));
        // The request recorded with the fixtures.
        let b = Bounds3857 {
            min_x: -8_243_208.0,
            min_y: 4_965_255.0,
            max_x: -8_234_302.0,
            max_y: 4_974_067.0,
        };
        assert_eq!(
            ncds_wms_bbox_url("2,3", b),
            "https://gis.charttools.noaa.gov/arcgis/rest/services/MCS/NOAAChartDisplay/MapServer/exts/MaritimeChartService/WMSServer?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=2,3&STYLES=&CRS=EPSG:3857&BBOX=-8243208,4965255,-8234302,4974067&WIDTH=256&HEIGHT=256&FORMAT=image/png&TRANSPARENT=TRUE"
        );
        assert!(ncds_wms_url("2,3", tile(12, 1205, 1540)).contains("BBOX="));
    }

    #[test]
    fn the_gebco_tile_matches_the_recorded_request() {
        let b = tile_bounds_3857(tile(8, 75, 96));
        for (got, want) in [
            (b.min_x, -8_296_780.7),
            (b.min_y, 4_852_834.1),
            (b.max_x, -8_140_237.8),
            (b.max_y, 5_009_377.1),
        ] {
            assert!((got - want).abs() < 1.0, "{got} vs {want}");
        }
        // The URL differs from the recorded one by at most a tenth of a metre in the last digit.
        let url = gebco_wms_url(tile(8, 75, 96));
        let (head, rest) = url.split_once("&bbox=").unwrap();
        assert_eq!(
            head,
            "https://wms.gebco.net/mapserv?service=WMS&version=1.3.0&request=GetMap&layers=GEBCO_LATEST_2&styles=&crs=EPSG:3857"
        );
        let (nums, tail) = rest.split_once('&').unwrap();
        assert_eq!(
            tail,
            "width=256&height=256&format=image/png&transparent=true"
        );
        let got: Vec<f64> = nums.split(',').map(|n| n.parse().unwrap()).collect();
        assert_eq!(got.len(), 4);
    }

    #[test]
    fn radar_urls_match_the_composite_layers() {
        let k = tile(8, 75, 96);
        assert_eq!(
            radar_tile_url(0, k),
            "https://mesonet.agron.iastate.edu/cache/tile.py/1.0.0/nexrad-n0q-900913/8/75/96.png"
        );
        assert_eq!(
            radar_tile_url(15, k),
            "https://mesonet.agron.iastate.edu/cache/tile.py/1.0.0/nexrad-n0q-900913-m15m/8/75/96.png"
        );
        assert_eq!(radar_tile_url(17, k), radar_tile_url(15, k));
    }

    #[test]
    fn dates_step_back_across_months_and_years() {
        // 2026-11-16 12:00 UTC.
        assert_eq!(
            gibs_default_date(UnixTime(1_794_830_400)),
            date(2026, 11, 15)
        );
        // 2027-01-01 00:01 UTC.
        assert_eq!(
            gibs_default_date(UnixTime(1_798_761_660)),
            date(2026, 12, 31)
        );
        assert_eq!(previous_day(date(2026, 3, 1)), date(2026, 2, 28));
        assert_eq!(previous_day(date(2028, 3, 1)), date(2028, 2, 29));
        assert_eq!(previous_day(date(2027, 1, 1)), date(2026, 12, 31));
    }

    #[test]
    fn responses_are_classified_per_service() {
        use TileOutcome::*;
        use TileService::*;
        assert_eq!(classify(Ncds, 200, Some("image/png"), 9_500, 0), Image);
        assert_eq!(
            classify(Ncds, 200, Some("image/png; charset=x"), 1, 0),
            Image
        );
        // The chart service answers a failure with 200 and a page or a JSON error.
        assert_eq!(
            classify(Ncds, 200, Some("text/html"), 800, 0),
            Retry { after_s: 3 }
        );
        assert_eq!(classify(Ncds, 200, Some("text/html"), 800, 1), Failed);
        assert_eq!(
            classify(Ncds, 200, Some("application/json"), 155, 0),
            Retry { after_s: 3 }
        );
        assert_eq!(classify(Ncds, 200, None, 10, 1), Failed);
        assert_eq!(classify(Ncds, 200, Some("image/png"), 0, 1), Failed);
        assert_eq!(classify(Gibs, 200, Some("image/jpeg"), 12_700, 0), Image);
        assert_eq!(classify(Ncds, 200, Some("image/jpeg"), 12_700, 1), Failed);
        assert_eq!(classify(Gibs, 404, Some("text/html"), 300, 0), NoData);
        assert_eq!(classify(Gibs, 400, Some("text/html"), 300, 0), TooDeep);
        assert_eq!(classify(Ncds, 404, None, 0, 0), Failed);
        assert_eq!(classify(Gebco, 400, None, 0, 0), Failed);
        assert_eq!(classify(Radar, 404, None, 0, 0), NoData);
        assert_eq!(classify(Gibs, 429, None, 0, 0), Retry { after_s: 30 });
        assert_eq!(classify(Radar, 503, None, 0, 1), Retry { after_s: 10 });
        assert_eq!(classify(Radar, 503, None, 0, 2), Failed);
        assert_eq!(classify(Gibs, 500, None, 0, 0), Failed);
    }

    #[test]
    fn cache_lifetimes_follow_the_header_then_the_service() {
        use TileService::*;
        assert_eq!(
            cache_lifetime_s(Gibs, Some("max-age=0, no-store, no-cache")),
            0
        );
        assert_eq!(
            cache_lifetime_s(Ncds, Some("public, max-age=86400")),
            86_400
        );
        assert_eq!(
            cache_lifetime_s(Gebco, Some("max-age=2630000, public")),
            2_630_000
        );
        assert_eq!(cache_lifetime_s(Gebco, None), 2_630_000);
        assert_eq!(cache_lifetime_s(Ncds, Some("public")), 86_400);
        assert_eq!(cache_lifetime_s(Ncds, Some("max-age=soon")), 86_400);
        assert_eq!(cache_lifetime_s(Radar, None), 300);
    }
}
