//! Depth from the NCEI DEM: approximate depth (never chart datum), shading and the depth band.
//!
//! DEM heights are metres, positive up, relative to a vertical datum that depends on the source
//! model (NAVD 88 or EGM 2008), not to chart datum. Every depth this module gives is therefore
//! "approximate depth" and carries the datum of the finest source covering the point; it is never
//! compared with charted soundings or tide heights. Where the DEM covers an inland lake bed the
//! value is a bed elevation, not a depth below the lake surface, and stays one unless the caller
//! supplies the lake level in the same datum.
//!
//! The sources a point is covered by come from the provider (`DemSource` in the providers'
//! `ncei_dem` module); core cannot depend on it, so [`DemSourceInfo`] holds what is needed and the
//! providers convert into it.

use crate::map::GeoPoint;
use crate::map::TileKey;
use crate::map::grid::{Grid, half_step, locate, locate_lon};
use crate::map::render::{self, DEPTH, Ramp, RampKind, RampRange, RenderError, Rgba};

/// Cells within this many metres of zero are shore, not water: DEM cells at about 0 m along a
/// coast flicker between land and water from tile to tile, so they are always left transparent.
pub const SHORE_M: f32 = 0.25;

/// The colour of cells inside the depth band: opaque and unlike any colour of the depth ramps.
pub const BAND_COLOUR: Rgba = Rgba::opaque(255, 140, 0);

/// A one-colour ramp used to draw the band from a mask grid.
static BAND_RAMP: Ramp = Ramp {
    id: "depth-band",
    kind: RampKind::Sequential,
    stops: &[BAND_COLOUR, BAND_COLOUR],
    default_range: RampRange::linear(0.0, 1.0),
};

/// What `depth_at` needs to know about one elevation model of the mosaic that covers a point.
///
/// The providers' `ncei_dem::DemSource` maps onto it field for field (`name`, `vertical_datum`,
/// `cell_arcsec`); its completion date is not needed here.
#[derive(Clone, Debug, PartialEq)]
pub struct DemSourceInfo {
    /// The model's raster name, such as `ncei19_n40x75_w074x25_2015v1`.
    pub name: String,
    /// The vertical datum as the service names it (`NAVD 88`, `EGM 2008`); `None` where it
    /// states none.
    pub vertical_datum: Option<String>,
    /// The model's cell size, arc-seconds.
    pub cell_arcsec: Option<f64>,
}

/// The finest source (smallest cell size): the one the mosaic draws at a point, whose vertical
/// datum the values there are relative to. Sources without a cell size are skipped.
pub fn finest_source(sources: &[DemSourceInfo]) -> Option<&DemSourceInfo> {
    sources
        .iter()
        .filter(|s| s.cell_arcsec.is_some_and(f64::is_finite))
        .min_by(|a, b| {
            a.cell_arcsec
                .partial_cmp(&b.cell_arcsec)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
}

/// What kind of water the DEM values are read as.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WaterBody {
    /// Sea: depth is the negative of the elevation where the elevation is below zero.
    Sea,
    /// An inland lake. With the lake's level in the same vertical datum as the DEM (metres) the
    /// depth is the level less the bed elevation; without one the value is only a bed elevation.
    Lake { level_m: Option<f64> },
}

/// A DEM reading: a depth, or a bed elevation where no surface level is known.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DepthReading {
    /// Approximate depth below the water surface, metres, positive down.
    Depth { depth_m: f64 },
    /// The height of a lake bed in the DEM's vertical datum, metres, positive up. Not a depth.
    BedElevation { elevation_m: f64 },
}

/// The value at a point, for the inspector. Never chart datum.
#[derive(Clone, Debug, PartialEq)]
pub struct ApproxDepth {
    pub reading: DepthReading,
    /// The vertical datum of the finest source covering the point; `None` when no source states
    /// one.
    pub vertical_datum: Option<String>,
    /// The raster name of that source.
    pub source: Option<String>,
}

impl ApproxDepth {
    /// The depth in metres, for a reading that is a depth.
    pub fn depth_m(&self) -> Option<f64> {
        match self.reading {
            DepthReading::Depth { depth_m } => Some(depth_m),
            DepthReading::BedElevation { .. } => None,
        }
    }

    /// What the number is, with its datum: `approximate depth (NAVD 88)` or
    /// `bed elevation (EGM 2008)`. The datum reads `datum not stated` when the sources give none.
    pub fn label(&self) -> String {
        let datum = self.vertical_datum.as_deref().unwrap_or("datum not stated");
        match self.reading {
            DepthReading::Depth { .. } => format!("approximate depth ({datum})"),
            DepthReading::BedElevation { .. } => format!("bed elevation ({datum})"),
        }
    }
}

/// The reading for one DEM value; `None` for land and shore, and for a value that is not a number.
fn read_value(value: f32, body: WaterBody) -> Option<DepthReading> {
    if !value.is_finite() {
        return None;
    }
    let value = f64::from(value);
    match body {
        WaterBody::Sea => {
            (value < -f64::from(SHORE_M)).then_some(DepthReading::Depth { depth_m: -value })
        }
        WaterBody::Lake {
            level_m: Some(level),
        } if level.is_finite() => {
            let depth_m = level - value;
            (depth_m > f64::from(SHORE_M)).then_some(DepthReading::Depth { depth_m })
        }
        WaterBody::Lake { .. } => Some(DepthReading::BedElevation { elevation_m: value }),
    }
}

impl DepthReading {
    /// The number, metres: the depth, or the bed elevation.
    fn metres(self) -> f64 {
        match self {
            Self::Depth { depth_m } => depth_m,
            Self::BedElevation { elevation_m } => elevation_m,
        }
    }
}

/// The reading at a point by the rule the tiles draw with: [`Grid::sample_bilinear`] over the
/// cells that are water. A point whose nearest cell centre is land or shore has no reading, and
/// land cells never mix into the value of a point near the coast.
fn read_at(grid: &Grid, point: GeoPoint, body: WaterBody) -> Option<DepthReading> {
    let half = half_step(grid.source.native_step_deg);
    let row = locate(&grid.lats, point.lat, half)?;
    let col = locate_lon(&grid.lons, point.lon, half)?;
    let corners = [
        (row.lo, col.lo, (1.0 - row.w) * (1.0 - col.w)),
        (row.lo, col.hi, (1.0 - row.w) * col.w),
        (row.hi, col.lo, row.w * (1.0 - col.w)),
        (row.hi, col.hi, row.w * col.w),
    ];
    let nearest = corners.iter().copied().max_by(|a, b| a.2.total_cmp(&b.2))?;
    read_value(grid.value(nearest.0, nearest.1)?, body)?;
    let (mut sum, mut weight) = (0.0_f64, 0.0_f64);
    for (ri, ci, wt) in corners {
        let Some(reading) = grid.value(ri, ci).and_then(|v| read_value(v, body)) else {
            continue;
        };
        if wt > 0.0 {
            sum += wt * reading.metres();
            weight += wt;
        }
    }
    if weight <= 0.0 {
        return None;
    }
    let mean = sum / weight;
    Some(match body {
        WaterBody::Lake { level_m: None } => DepthReading::BedElevation { elevation_m: mean },
        _ => DepthReading::Depth { depth_m: mean },
    })
}

/// The approximate depth at a point, or `None` on land, at the shore (within [`SHORE_M`] of
/// zero), where the grid has no value, or outside the grid. The value is the one the tiles show
/// at that point (see [`render_depth_rgba`]).
///
/// `sources` are the models covering the point; the datum comes from the finest of them.
pub fn depth_at(
    grid: &Grid,
    point: GeoPoint,
    sources: &[DemSourceInfo],
    body: WaterBody,
) -> Option<ApproxDepth> {
    let reading = read_at(grid, point, body)?;
    let finest = finest_source(sources);
    Some(ApproxDepth {
        reading,
        vertical_datum: finest.and_then(|s| s.vertical_datum.clone()),
        source: finest.map(|s| s.name.clone()),
    })
}

/// A grid of the same axes holding `f` of each cell, or `NaN` where `f` is `None`.
fn mapped(grid: &Grid, f: impl Fn(f32) -> Option<f32>) -> Grid {
    Grid {
        source: grid.source.clone(),
        time: grid.time,
        lats: grid.lats.clone(),
        lons: grid.lons.clone(),
        values: grid
            .values
            .iter()
            .map(|&v| f(v).unwrap_or(f32::NAN))
            .collect(),
        unit: grid.unit.clone(),
    }
}

/// The depth band as an ordered pair, or none for a band that is not a pair of finite depths.
fn clean_band(band_m: Option<(f32, f32)>) -> Option<(f32, f32)> {
    let (a, b) = band_m?;
    (a.is_finite() && b.is_finite()).then(|| (a.min(b), a.max(b)))
}

/// Renders the depth of a DEM tile as straight-alpha RGBA pixels.
///
/// Water cells are coloured by depth with the ramp named `ramp_id` (the `depth` ramp when none or
/// an unknown id) over its default range; land and shore are transparent, so a coastline looks
/// the same from tile to tile. Pixels whose nearest cell is inside `band_m` (shallow and deep
/// depths, metres, either order) are drawn in [`BAND_COLOUR`]. For a lake without a known level
/// the cells are shaded as bed elevations over the grid's own range (higher is lighter), every
/// valued cell is drawn and there is no band: they are not depths.
///
/// # Errors
/// As [`render::render_rgba`].
pub fn render_depth_rgba(
    grid: &Grid,
    body: WaterBody,
    band_m: Option<(f32, f32)>,
    ramp_id: Option<&str>,
    tile: TileKey,
    size: u32,
) -> Result<Vec<u8>, RenderError> {
    let ramp = ramp_id.and_then(render::ramp).unwrap_or(&DEPTH);
    let depth_of = |v: f32| match read_value(v, body) {
        Some(DepthReading::Depth { depth_m }) => Some(depth_m as f32),
        _ => None,
    };
    if matches!(body, WaterBody::Lake { level_m: None }) {
        let shaded = mapped(grid, |v| v.is_finite().then_some(-v));
        let (low, high) = grid
            .values
            .iter()
            .filter(|v| v.is_finite())
            .fold((f32::MAX, f32::MIN), |(lo, hi), &v| {
                (lo.min(-v), hi.max(-v))
            });
        let range = if low < high {
            RampRange::linear(f64::from(low), f64::from(high))
        } else {
            RampRange::linear(f64::from(low), f64::from(low) + 1.0)
        };
        return render::render_rgba(&[shaded], ramp, range, tile, size);
    }
    let depths = mapped(grid, depth_of);
    let mut pixels = render::render_rgba(
        std::slice::from_ref(&depths),
        ramp,
        ramp.default_range,
        tile,
        size,
    )?;
    if let Some((shallow, deep)) = clean_band(band_m) {
        let mask = mapped(&depths, |d| (d >= shallow && d <= deep).then_some(1.0));
        let band = render::render_rgba(&[mask], &BAND_RAMP, BAND_RAMP.default_range, tile, size)?;
        for (pixel, mark) in pixels
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(band.as_chunks::<4>().0)
        {
            if mark[3] != 0 {
                *pixel = [BAND_COLOUR.r, BAND_COLOUR.g, BAND_COLOUR.b, BAND_COLOUR.a];
            }
        }
    }
    Ok(pixels)
}

/// Renders the depth of a DEM tile as a PNG: [`render_depth_rgba`] then [`render::encode_png`].
///
/// # Errors
/// As [`render_depth_rgba`] and [`render::encode_png`].
pub fn render_depth_tile(
    grid: &Grid,
    body: WaterBody,
    band_m: Option<(f32, f32)>,
    ramp_id: Option<&str>,
    tile: TileKey,
    size: u32,
) -> Result<Vec<u8>, RenderError> {
    let rgba = render_depth_rgba(grid, body, band_m, ramp_id, tile, size)?;
    render::encode_png(&rgba, size, size)
}

/// The credit for the GEBCO grid as the WMS capabilities ask for it: the acknowledgement of the
/// form "Imagery reproduced from the `GEBCO_2026` Grid, GEBCO Compilation Group (2026) GEBCO 2026
/// Grid (doi:…)", followed by "Not for navigation." Reading it from the document keeps the grid
/// year and the DOI current with each release. `None` when the document states no
/// acknowledgement (the display then credits "GEBCO" alone).
pub fn gebco_credit(capabilities_xml: &str) -> Option<String> {
    const LEAD: &str = "of the form ";
    let at = capabilities_xml.find(LEAD)? + LEAD.len();
    let rest = &capabilities_xml[at..];
    let (open, quote_len) = if rest.starts_with("&#39;") {
        ("&#39;", 5)
    } else if rest.starts_with('\'') {
        ("'", 1)
    } else {
        return None;
    };
    let body = &rest[quote_len..];
    let end = body.find(open)?;
    let text = unescape(&body[..end]);
    let text = text.trim();
    (!text.is_empty()).then(|| format!("{text}. Not for navigation."))
}

/// The five predefined XML entities and numeric character references.
fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        let decoded = rest.find(';').filter(|&semi| semi <= 8).and_then(|semi| {
            let name = &rest[1..semi];
            let c = match name {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                _ => name
                    .strip_prefix('#')
                    .and_then(|n| match n.strip_prefix(['x', 'X']) {
                        Some(hex) => u32::from_str_radix(hex, 16).ok(),
                        None => n.parse().ok(),
                    })
                    .and_then(char::from_u32),
            }?;
            Some((c, semi))
        });
        if let Some((c, semi)) = decoded {
            out.push(c);
            rest = &rest[semi + 1..];
        } else {
            out.push('&');
            rest = &rest[1..];
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::{GridHost, GridSource};
    use crate::time::UnixTime;

    /// Two columns of sea at 29.6 m and 3.1 m deep, then a shore cell and land, over a flat strip.
    fn strip() -> Grid {
        let lons: Vec<f64> = (0..4).map(|j| -74.0 + 0.005 * f64::from(j)).collect();
        let lats: Vec<f64> = (0..4).map(|i| 40.4 + 0.005 * f64::from(i)).collect();
        let row = [-29.6_f32, -3.1, -0.1, 12.0];
        let values: Vec<f32> = (0..4).flat_map(|_| row).collect();
        Grid::new(
            GridSource {
                node: GridHost::NceiDem,
                dataset: "DEM_global_mosaic".to_owned(),
                variables: vec!["elevation".to_owned()],
                altitude_axis: false,
                native_step_deg: 0.005,
            },
            UnixTime(0),
            lats,
            lons,
            values,
            "m",
        )
        .unwrap()
    }

    fn sources() -> Vec<DemSourceInfo> {
        vec![
            DemSourceInfo {
                name: "crm_vol1_2023".to_owned(),
                vertical_datum: Some("EGM 2008".to_owned()),
                cell_arcsec: Some(3.0),
            },
            DemSourceInfo {
                name: "ncei19_n40x75_w074x25_2015v1".to_owned(),
                vertical_datum: Some("NAVD 88".to_owned()),
                cell_arcsec: Some(0.33),
            },
            DemSourceInfo {
                name: "unsized".to_owned(),
                vertical_datum: Some("MLLW".to_owned()),
                cell_arcsec: None,
            },
        ]
    }

    fn at(lon_step: f64) -> GeoPoint {
        GeoPoint {
            lat: 40.41,
            lon: -74.0 + 0.005 * lon_step,
        }
    }

    #[test]
    fn dem_depth_is_the_negative_elevation_with_the_finest_datum() {
        let grid = strip();
        let d = depth_at(&grid, at(0.0), &sources(), WaterBody::Sea).unwrap();
        assert!((d.depth_m().unwrap() - 29.6).abs() < 1e-4);
        assert_eq!(d.vertical_datum.as_deref(), Some("NAVD 88"));
        assert_eq!(d.source.as_deref(), Some("ncei19_n40x75_w074x25_2015v1"));
        assert_eq!(d.label(), "approximate depth (NAVD 88)");
    }

    #[test]
    fn dem_land_and_shore_have_no_depth() {
        let grid = strip();
        // The shore cell (-0.1 m) and the land cell.
        assert_eq!(depth_at(&grid, at(2.0), &[], WaterBody::Sea), None);
        assert_eq!(depth_at(&grid, at(3.0), &[], WaterBody::Sea), None);
        // Outside the grid.
        let far = GeoPoint {
            lat: 10.0,
            lon: 0.0,
        };
        assert_eq!(depth_at(&grid, far, &[], WaterBody::Sea), None);
    }

    #[test]
    fn dem_datum_is_unstated_without_sources() {
        let d = depth_at(&strip(), at(1.0), &[], WaterBody::Sea).unwrap();
        assert_eq!((d.vertical_datum.clone(), d.source.clone()), (None, None));
        assert_eq!(d.label(), "approximate depth (datum not stated)");
    }

    #[test]
    fn dem_lake_without_a_level_is_a_bed_elevation() {
        let lake = WaterBody::Lake { level_m: None };
        let d = depth_at(&strip(), at(3.0), &sources(), lake).unwrap();
        assert_eq!(d.depth_m(), None);
        assert_eq!(d.reading, DepthReading::BedElevation { elevation_m: 12.0 });
        assert_eq!(d.label(), "bed elevation (NAVD 88)");
    }

    #[test]
    fn dem_lake_with_a_level_gives_depth_below_the_surface() {
        let lake = WaterBody::Lake {
            level_m: Some(14.0),
        };
        let d = depth_at(&strip(), at(3.0), &sources(), lake).unwrap();
        assert!((d.depth_m().unwrap() - 2.0).abs() < 1e-4);
        // The surface is below the bed at the first column.
        let above = WaterBody::Lake {
            level_m: Some(-40.0),
        };
        assert!(depth_at(&strip(), at(3.0), &[], above).is_none());
    }

    fn pixel_at(rgba: &[u8], tile: TileKey, size: u32, point: GeoPoint) -> [u8; 4] {
        let count = f64::from(1u32 << tile.z);
        let x = (point.lon + 180.0) / 360.0 * count;
        let phi = point.lat.to_radians();
        let y = (1.0 - phi.tan().asinh() / std::f64::consts::PI) / 2.0 * count;
        let px = ((x - f64::from(tile.x)) * f64::from(size)) as usize;
        let py = ((y - f64::from(tile.y)) * f64::from(size)) as usize;
        let at = (py * size as usize + px) * 4;
        [rgba[at], rgba[at + 1], rgba[at + 2], rgba[at + 3]]
    }

    /// The z12 tile over the strip's south-west corner.
    fn strip_tile() -> TileKey {
        let n = f64::from(1u32 << 12);
        let x = ((-74.0 + 180.0) / 360.0 * n) as u32;
        let phi = 40.41_f64.to_radians();
        let y = ((1.0 - phi.tan().asinh() / std::f64::consts::PI) / 2.0 * n) as u32;
        TileKey { z: 12, x, y }
    }

    #[test]
    fn dem_tile_has_transparent_land_and_coloured_water() {
        let tile = strip_tile();
        let rgba = render_depth_rgba(&strip(), WaterBody::Sea, None, None, tile, 64).unwrap();
        let water = pixel_at(&rgba, tile, 64, at(0.0));
        let shore = pixel_at(&rgba, tile, 64, at(2.0));
        let land = pixel_at(&rgba, tile, 64, at(3.0));
        assert_eq!(water[3], 255);
        assert_eq!(shore[3], 0);
        assert_eq!(land[3], 0);
        // Deeper water is darker on the depth ramp.
        let shallow = pixel_at(&rgba, tile, 64, at(1.0));
        let lightness = |p: [u8; 4]| u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2]);
        assert!(lightness(water) < lightness(shallow));
    }

    #[test]
    fn dem_band_highlights_only_cells_inside_it() {
        let tile = strip_tile();
        let band = Some((35.0, 25.0));
        let rgba = render_depth_rgba(&strip(), WaterBody::Sea, band, None, tile, 64).unwrap();
        let hit = pixel_at(&rgba, tile, 64, at(0.0));
        let miss = pixel_at(&rgba, tile, 64, at(1.0));
        assert_eq!(hit, [255, 140, 0, 255]);
        assert_ne!(miss, hit);
        assert_eq!(miss[3], 255);
        let land = pixel_at(&rgba, tile, 64, at(3.0));
        assert_eq!(land[3], 0);
        // A band that is not a pair of numbers highlights nothing.
        let nan = Some((f32::NAN, 35.0));
        let rgba = render_depth_rgba(&strip(), WaterBody::Sea, nan, None, tile, 64).unwrap();
        assert_ne!(pixel_at(&rgba, tile, 64, at(0.0)), hit);
    }

    #[test]
    fn dem_lake_without_a_level_is_shaded_as_elevation_with_no_band() {
        let tile = strip_tile();
        let lake = WaterBody::Lake { level_m: None };
        let rgba = render_depth_rgba(&strip(), lake, Some((0.0, 100.0)), None, tile, 64).unwrap();
        // The land-like cell (12 m) is drawn: it is a bed, not land.
        assert_eq!(pixel_at(&rgba, tile, 64, at(3.0))[3], 255);
        assert_ne!(pixel_at(&rgba, tile, 64, at(0.0)), [255, 140, 0, 255]);
    }

    #[test]
    fn dem_tile_png_has_the_png_signature() {
        let png =
            render_depth_tile(&strip(), WaterBody::Sea, None, None, strip_tile(), 32).unwrap();
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    }

    #[test]
    fn dem_tile_errors_pass_through() {
        let bad = TileKey { z: 3, x: 99, y: 0 };
        assert!(render_depth_tile(&strip(), WaterBody::Sea, None, None, bad, 32).is_err());
        assert_eq!(
            render_depth_rgba(&strip(), WaterBody::Sea, None, None, strip_tile(), 0),
            Err(RenderError::BadSize(0))
        );
    }

    #[test]
    fn gebco_credit_reads_the_acknowledgement_and_its_doi() {
        let xml = "<Abstract>Please acknowledge in a statement of the form &#39;Imagery reproduced from the \
                   GEBCO_2031 Grid, GEBCO Compilation Group (2031) GEBCO 2031 Grid \
                   (doi:10.5285/abc) &amp; friends&#39;. Later text.</Abstract>";
        let credit = gebco_credit(xml).unwrap();
        assert_eq!(
            credit,
            "Imagery reproduced from the GEBCO_2031 Grid, GEBCO Compilation Group (2031) \
             GEBCO 2031 Grid (doi:10.5285/abc) & friends. Not for navigation."
        );
    }

    #[test]
    fn gebco_credit_is_none_without_an_acknowledgement() {
        assert_eq!(gebco_credit("<WMS_Capabilities/>"), None);
        assert_eq!(gebco_credit("of the form plain text"), None);
        assert_eq!(gebco_credit("of the form &#39;unterminated"), None);
    }

    #[test]
    fn xml_entities_are_decoded() {
        assert_eq!(
            unescape("a &amp; b &#39;c&#x27; &lt;&bogus; &"),
            "a & b 'c' <&bogus; &"
        );
    }
}
