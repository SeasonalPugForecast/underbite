//! The map layer catalogue: groups, layers, sources, legends and availability.
//!
//! Every overlay and base map is a [`Layer`] in [`LAYERS`] or [`BASES`], in drawer order. The data
//! here is static and holds no words: titles, legend captions, credits, preset names and the
//! reasons a layer is unavailable are in [`crate::view::strings`], looked up by layer or preset
//! id. Presets are lists of layer ids to show, in [`PRESETS`].

use crate::map::enc::{self, EncGroup};
use crate::map::render::{self, Ramp, RampRange};
use crate::map::tiles::{
    GibsSpec, TileService, gebco_wms_url, gibs_tile_url, ncds_wms_url, ncds_wmts_url,
    radar_tile_url,
};
use crate::map::{Bbox, GridHost, GridSource, LayerId, TileKey};
use crate::time::LocalDate;

/// The drawer sections.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Group {
    Base,
    Depth,
    /// Reefs and other bottom structure.
    Structure,
    Water,
    Weather,
    Stations,
    Mine,
}

impl Group {
    /// Every group, in drawer order.
    pub const ALL: [Self; 7] = [
        Self::Base,
        Self::Depth,
        Self::Structure,
        Self::Water,
        Self::Weather,
        Self::Stations,
        Self::Mine,
    ];
}

/// How often a layer's data changes, which decides what its date control offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DateMode {
    /// No date: the layer is the same every day (a chart, stations).
    None,
    /// One field per UTC day.
    Daily,
    /// One field per hour.
    Hourly,
    /// A loop of recent frames, newest first.
    Frames,
}

/// Why a listed layer cannot be turned on; the view words it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnavailableReason {
    /// No free source serves the layer yet.
    NoSource,
}

/// Whether a layer can be shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Availability {
    Available,
    /// The layer is listed but cannot be turned on; the reason is shown in place of the switch.
    Unavailable {
        reason: UnavailableReason,
    },
}

/// A base map Apple draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppleBase {
    Standard,
    Satellite,
    Hybrid,
}

/// A tiled raster source and the arguments that make its URL.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TileSource {
    /// NASA GIBS WMTS imagery by date.
    Gibs(GibsSpec),
    /// The NOAA Chart Display pre-rendered tile cache (WMTS).
    NcdsWmts,
    /// A NOAA Chart Display WMS request for display groups, such as `2,3`.
    NcdsWms { groups: &'static str },
    /// The GEBCO shaded-relief WMS.
    GebcoWms,
    /// The NEXRAD composite frames.
    Radar,
}

/// Where a grid layer's data is requested: an ERDDAP dataset and its variables.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridSpec {
    pub node: GridHost,
    pub dataset: &'static str,
    pub variables: &'static [&'static str],
    pub altitude_axis: bool,
    pub native_step_deg: f64,
}

impl GridSpec {
    /// The source description the grid code takes.
    pub fn source(&self) -> GridSource {
        GridSource {
            node: self.node,
            dataset: self.dataset.to_owned(),
            variables: self.variables.iter().map(|v| (*v).to_owned()).collect(),
            altitude_axis: self.altitude_axis,
            native_step_deg: self.native_step_deg,
        }
    }
}

/// How a grid is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GridDraw {
    /// A colour-ramped raster rendered on the device.
    Raster,
    /// Arrows from two components (eastward and northward).
    Arrows,
}

/// A gridded layer: the dataset, a fallback and how it is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridLayer {
    pub primary: GridSpec,
    /// A second dataset to use when the primary has no recent time.
    pub fallback: Option<GridSpec>,
    pub draw: GridDraw,
}

impl GridLayer {
    /// The primary dataset, then the fallback.
    pub fn specs(&self) -> impl Iterator<Item = &GridSpec> {
        std::iter::once(&self.primary).chain(self.fallback.as_ref())
    }

    /// The dataset with this id, among the primary and the fallback.
    pub fn spec(&self, dataset: &str) -> Option<&GridSpec> {
        self.specs().find(|s| s.dataset == dataset)
    }
}

/// What a station layer shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StationKind {
    Tide,
    Current,
    RiverGauge,
    Buoy,
    ForecastGauge,
}

/// What a state agency's feature layer holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeatureKind {
    /// Depth contour lines of surveyed lakes.
    LakeContours,
    /// Artificial reef sites.
    ArtificialReefs,
}

/// A state agency's vector layer: the state, what it holds and where it has data. What the
/// source says about its depths and its terms is worded in the view, by layer id.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FeatureLayer {
    pub state: &'static str,
    pub kind: FeatureKind,
    /// A box around the source's data; the layer asks nothing outside it.
    pub coverage: Bbox,
}

/// What a layer is made of.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LayerKind {
    /// A base map Apple draws.
    AppleBase(AppleBase),
    /// A raster the core addresses by tile.
    Tiles(TileSource),
    /// Gridded data drawn on the device.
    Grid(GridLayer),
    /// Station markers from the stations the app already knows.
    Stations(StationKind),
    /// The user's saved spots.
    Spots,
    /// Lines or points a state agency publishes, drawn on the device.
    Features(FeatureLayer),
    /// Wrecks, obstructions and rocks from NOAA ENC Direct to GIS, drawn on the device from points
    /// the core reads by box (`underbite_providers::encdirect`); the band is chosen by zoom.
    EncPoints,
    /// A layer with no data source yet.
    Unsourced,
}

/// One layer of the drawer. Its words (title, legend caption, credit) are the view's, by `id`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layer {
    /// The stable id, as in the map configuration.
    pub id: &'static str,
    pub group: Group,
    pub kind: LayerKind,
    /// The shallowest zoom the layer serves, when it has a limit.
    pub min_zoom: Option<u8>,
    /// The deepest zoom the layer serves: deeper requests use [`over_zoom`](crate::map::tiles::over_zoom). None for
    /// layers drawn on the device.
    pub max_zoom: Option<u8>,
    pub date_mode: DateMode,
    /// Charts, currents and depth carry "not for navigation".
    pub not_for_navigation: bool,
    pub availability: Availability,
    /// 0 to 1.
    pub default_opacity: f32,
    /// The colour ramp's id, for layers that colour values.
    pub ramp: Option<&'static str>,
}

impl Layer {
    /// The layer's id as the model type.
    pub fn layer_id(&self) -> LayerId {
        LayerId::from_static(self.id)
    }

    /// Whether the layer is a base map.
    pub fn is_base(&self) -> bool {
        self.group == Group::Base
    }

    /// The layer's colour ramp, if it colours values.
    pub fn colour_ramp(&self) -> Option<&'static Ramp> {
        self.ramp.and_then(render::ramp)
    }

    /// The layer's default value range in canonical units (the ramp's own: °C, log10 of mg m⁻³
    /// or m⁻¹, m/s), which a user range overrides with [`RampRange::with_bounds`].
    pub fn default_range(&self) -> Option<RampRange> {
        self.colour_ramp().map(|r| r.default_range)
    }

    /// The layer's grids, for a gridded layer.
    pub fn grid(&self) -> Option<&GridLayer> {
        match &self.kind {
            LayerKind::Grid(g) => Some(g),
            _ => None,
        }
    }

    /// The service that answers this layer's tile requests, for [`classify`](crate::map::tiles::classify).
    pub fn tile_service(&self) -> Option<TileService> {
        match self.kind {
            LayerKind::Tiles(TileSource::Gibs(_)) => Some(TileService::Gibs),
            LayerKind::Tiles(TileSource::NcdsWmts | TileSource::NcdsWms { .. }) => {
                Some(TileService::Ncds)
            }
            LayerKind::Tiles(TileSource::GebcoWms) => Some(TileService::Gebco),
            LayerKind::Tiles(TileSource::Radar) => Some(TileService::Radar),
            _ => None,
        }
    }

    /// The URL of a tile of a tiled layer, or none for a layer that is not tiled or a zoom its
    /// source has no tile for. `date` is the UTC day for dated layers (none asks for the source's
    /// `default`), `minutes_ago` the age of a radar frame.
    pub fn tile_url(
        &self,
        key: TileKey,
        date: Option<LocalDate>,
        minutes_ago: u32,
    ) -> Option<String> {
        let LayerKind::Tiles(source) = self.kind else {
            return None;
        };
        match source {
            TileSource::Gibs(spec) if key.z <= spec.max_zoom => {
                Some(gibs_tile_url(&spec, date, key))
            }
            TileSource::Gibs(_) => None,
            TileSource::NcdsWmts => ncds_wmts_url(key),
            TileSource::NcdsWms { groups } => Some(ncds_wms_url(groups, key)),
            TileSource::GebcoWms => Some(gebco_wms_url(key)),
            TileSource::Radar => Some(radar_tile_url(minutes_ago, key)),
        }
    }
}

const NO_SOURCE: Availability = Availability::Unavailable {
    reason: UnavailableReason::NoSource,
};

const MUR_GIBS: GibsSpec = GibsSpec {
    layer: "GHRSST_L4_MUR_Sea_Surface_Temperature",
    matrix_set: "GoogleMapsCompatible_Level7",
    ext: "png",
    max_zoom: 7,
};
const CHL_GIBS: GibsSpec = GibsSpec {
    layer: "VIIRS_NOAA20_Chlorophyll_a",
    matrix_set: "GoogleMapsCompatible_Level7",
    ext: "png",
    max_zoom: 7,
};

const fn true_colour(layer: &'static str) -> GibsSpec {
    GibsSpec {
        layer,
        matrix_set: "GoogleMapsCompatible_Level9",
        ext: "jpg",
        max_zoom: 9,
    }
}

const fn base(id: &'static str, kind: LayerKind) -> Layer {
    Layer {
        id,
        group: Group::Base,
        kind,
        min_zoom: None,
        max_zoom: None,
        date_mode: DateMode::None,
        not_for_navigation: false,
        availability: Availability::Available,
        default_opacity: 1.0,
        ramp: None,
    }
}

const fn overlay(id: &'static str, group: Group, kind: LayerKind) -> Layer {
    Layer {
        id,
        group,
        kind,
        min_zoom: None,
        max_zoom: None,
        date_mode: DateMode::None,
        not_for_navigation: false,
        availability: Availability::Available,
        default_opacity: 0.7,
        ramp: None,
    }
}

const fn station(id: &'static str, kind: StationKind) -> Layer {
    let mut l = overlay(id, Group::Stations, LayerKind::Stations(kind));
    l.default_opacity = 1.0;
    l
}

const MUR_GRID: GridSpec = GridSpec {
    node: GridHost::Pfeg,
    dataset: "jplMURSST41",
    variables: &["analysed_sst"],
    altitude_axis: false,
    native_step_deg: 0.01,
};
const CHL_GRID: GridSpec = GridSpec {
    node: GridHost::Coastwatch,
    dataset: "noaacwN20VIIRSchlaDaily",
    variables: &["chlor_a"],
    altitude_axis: true,
    native_step_deg: 0.0375,
};
const KD_N21: GridSpec = GridSpec {
    node: GridHost::Coastwatch,
    dataset: "noaacwN21VIIRSkd490NSWUSDaily",
    variables: &["kd_490"],
    altitude_axis: true,
    native_step_deg: 0.0075,
};
const KD_N20: GridSpec = GridSpec {
    node: GridHost::Coastwatch,
    dataset: "noaacwN20VIIRSkd490NSWUSDaily",
    variables: &["kd_490"],
    altitude_axis: true,
    native_step_deg: 0.0075,
};
const HF_GRID: GridSpec = GridSpec {
    node: GridHost::Pfeg,
    dataset: "ucsdHfrE6",
    variables: &["water_u", "water_v"],
    altitude_axis: false,
    native_step_deg: 0.054,
};
const GLSEA_GRID: GridSpec = GridSpec {
    node: GridHost::Glerl,
    dataset: "GLSEA_ACSPO_GCS",
    variables: &["sst"],
    altitude_axis: false,
    native_step_deg: 0.014_015_517,
};

/// The shallowest zoom a grid layer is drawn at. From here a view takes about a dozen block
/// requests; wider views would take hundreds for a layer nobody can read at that scale.
const GRID_MIN_ZOOM: u8 = 5;

const fn grid_layer(id: &'static str, group: Group, grid: GridLayer, ramp: &'static str) -> Layer {
    let mut l = overlay(id, group, LayerKind::Grid(grid));
    l.date_mode = DateMode::Daily;
    l.min_zoom = Some(GRID_MIN_ZOOM);
    l.ramp = Some(ramp);
    l
}

const fn gibs_layer(id: &'static str, spec: GibsSpec, ramp: Option<&'static str>) -> Layer {
    let mut l = overlay(id, Group::Water, LayerKind::Tiles(TileSource::Gibs(spec)));
    l.max_zoom = Some(spec.max_zoom);
    l.date_mode = DateMode::Daily;
    l.ramp = ramp;
    l
}

/// The shallowest zoom the lake contours are drawn at: a lake is a few hundred metres across.
const CONTOUR_MIN_ZOOM: u8 = 10;
/// The shallowest zoom the reef sites are drawn at: sixteen sites along Long Island's shores.
const REEF_MIN_ZOOM: u8 = 7;

const fn feature_layer(
    id: &'static str,
    group: Group,
    feature: FeatureLayer,
    min_zoom: u8,
) -> Layer {
    let mut l = overlay(id, group, LayerKind::Features(feature));
    l.min_zoom = Some(min_zoom);
    l.not_for_navigation = true;
    l.default_opacity = 0.9;
    l
}

/// The shallowest zoom the ENC Direct points are asked at: the coastal band's widest scale
/// (1:600,000) at the latitudes of the test places.
const ENC_POINTS_MIN_ZOOM: u8 = 9;

/// A NOAA Chart Display group drawn as a transparent WMS picture.
const fn enc_picture(id: &'static str, group: Group, enc: EncGroup) -> Layer {
    let mut l = overlay(
        id,
        group,
        LayerKind::Tiles(TileSource::NcdsWms { groups: enc.groups }),
    );
    l.min_zoom = Some(enc.min_zoom);
    l.not_for_navigation = true;
    l
}

const fn unsourced(id: &'static str) -> Layer {
    let mut l = overlay(id, Group::Weather, LayerKind::Unsourced);
    l.availability = NO_SOURCE;
    l.date_mode = DateMode::Hourly;
    l
}

/// The base maps, in the order the picker lists them.
pub const BASES: [Layer; 4] = [
    base("apple-standard", LayerKind::AppleBase(AppleBase::Standard)),
    base(
        "apple-satellite",
        LayerKind::AppleBase(AppleBase::Satellite),
    ),
    base("apple-hybrid", LayerKind::AppleBase(AppleBase::Hybrid)),
    {
        let mut l = base("noaa-chart", LayerKind::Tiles(TileSource::NcdsWmts));
        l.min_zoom = Some(2);
        l.max_zoom = Some(20);
        l.not_for_navigation = true;
        l
    },
];

/// The overlay layers in drawer order.
pub const LAYERS: [Layer; 29] = [
    {
        let mut l = overlay(
            "gebco-relief",
            Group::Depth,
            LayerKind::Tiles(TileSource::GebcoWms),
        );
        l.max_zoom = Some(10);
        l.not_for_navigation = true;
        l.default_opacity = 0.6;
        l
    },
    grid_layer(
        "sst",
        Group::Water,
        GridLayer {
            primary: MUR_GRID,
            fallback: None,
            draw: GridDraw::Raster,
        },
        "thermal",
    ),
    gibs_layer("sst-imagery", MUR_GIBS, Some("thermal")),
    grid_layer(
        "chlorophyll",
        Group::Water,
        GridLayer {
            primary: CHL_GRID,
            fallback: None,
            draw: GridDraw::Raster,
        },
        "algae",
    ),
    gibs_layer("chlorophyll-imagery", CHL_GIBS, Some("algae")),
    grid_layer(
        "clarity",
        Group::Water,
        GridLayer {
            primary: KD_N21,
            fallback: Some(KD_N20),
            draw: GridDraw::Raster,
        },
        "turbidity",
    ),
    gibs_layer(
        "true-color-viirs-n20",
        true_colour("VIIRS_NOAA20_CorrectedReflectance_TrueColor"),
        None,
    ),
    gibs_layer(
        "true-color-viirs-n21",
        true_colour("VIIRS_NOAA21_CorrectedReflectance_TrueColor"),
        None,
    ),
    gibs_layer(
        "true-color-modis-terra",
        true_colour("MODIS_Terra_CorrectedReflectance_TrueColor"),
        None,
    ),
    gibs_layer(
        "true-color-modis-aqua",
        true_colour("MODIS_Aqua_CorrectedReflectance_TrueColor"),
        None,
    ),
    {
        let mut l = grid_layer(
            "hf-radar-currents",
            Group::Water,
            GridLayer {
                primary: HF_GRID,
                fallback: None,
                draw: GridDraw::Arrows,
            },
            "speed",
        );
        l.date_mode = DateMode::Hourly;
        // Arrows are thinned to one per 32 px, so a wide view of them says little.
        l.min_zoom = Some(6);
        l.not_for_navigation = true;
        l.default_opacity = 0.9;
        l
    },
    grid_layer(
        "great-lakes-sst",
        Group::Water,
        GridLayer {
            primary: GLSEA_GRID,
            fallback: None,
            draw: GridDraw::Raster,
        },
        "thermal",
    ),
    {
        let mut l = overlay("radar", Group::Weather, LayerKind::Tiles(TileSource::Radar));
        l.min_zoom = Some(6);
        l.max_zoom = Some(10);
        l.date_mode = DateMode::Frames;
        l.default_opacity = 0.75;
        l
    },
    unsourced("wind"),
    unsourced("pressure"),
    unsourced("waves"),
    station("tide-stations", StationKind::Tide),
    {
        let mut l = station("current-stations", StationKind::Current);
        l.not_for_navigation = true;
        l
    },
    station("usgs-gauges", StationKind::RiverGauge),
    station("buoys", StationKind::Buoy),
    station("nwps-gauges", StationKind::ForecastGauge),
    {
        let mut l = overlay("spots", Group::Mine, LayerKind::Spots);
        l.default_opacity = 1.0;
        l
    },
    feature_layer(
        "lake-contours-in",
        Group::Depth,
        FeatureLayer {
            state: "Indiana",
            kind: FeatureKind::LakeContours,
            // The service's extent: certain Indiana lakes, not all.
            coverage: Bbox {
                west: -87.53,
                south: 37.99,
                east: -84.80,
                north: 41.77,
            },
        },
        CONTOUR_MIN_ZOOM,
    ),
    feature_layer(
        "lake-contours-mn",
        Group::Depth,
        FeatureLayer {
            state: "Minnesota",
            kind: FeatureKind::LakeContours,
            // The state's extent: the layer holds selected lakes only.
            coverage: Bbox {
                west: -97.24,
                south: 43.50,
                east: -89.49,
                north: 49.38,
            },
        },
        CONTOUR_MIN_ZOOM,
    ),
    feature_layer(
        "artificial-reefs-ny",
        Group::Structure,
        FeatureLayer {
            state: "New York",
            kind: FeatureKind::ArtificialReefs,
            // The corners of the sixteen listed sites.
            coverage: Bbox {
                west: -73.856_333,
                south: 40.423_833,
                east: -72.46,
                north: 41.058_333,
            },
        },
        REEF_MIN_ZOOM,
    ),
    enc_picture("enc-depths", Group::Depth, enc::DEPTHS),
    enc_picture("enc-seabed", Group::Structure, enc::SEABED),
    enc_picture("enc-special-areas", Group::Structure, enc::SPECIAL_AREAS),
    {
        let mut l = overlay(
            "enc-structure-points",
            Group::Structure,
            LayerKind::EncPoints,
        );
        l.min_zoom = Some(ENC_POINTS_MIN_ZOOM);
        l.not_for_navigation = true;
        l.default_opacity = 0.9;
        l
    },
];

/// The layer or base with an id.
pub fn find(id: &str) -> Option<&'static Layer> {
    BASES.iter().chain(LAYERS.iter()).find(|l| l.id == id)
}

/// Every id in drawer order: the bases, then the overlays.
pub fn ids() -> impl Iterator<Item = &'static str> {
    BASES.iter().chain(LAYERS.iter()).map(|l| l.id)
}

/// How a preset shows one layer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PresetLayer {
    pub id: &'static str,
    /// None keeps the layer's own default opacity.
    pub opacity: Option<f32>,
}

const fn on(id: &'static str) -> PresetLayer {
    PresetLayer { id, opacity: None }
}

/// A built-in set of layers to show; every other layer is hidden. Its name is the view's, by
/// `id`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Preset {
    /// The kebab-case id stored as the active preset.
    pub id: &'static str,
    /// The base map the preset starts on.
    pub base: &'static str,
    pub layers: &'static [PresetLayer],
}

/// The preset used when the stored one is not known.
pub const DEFAULT_PRESET: &str = "inshore-flats";

/// The built-in presets.
pub const PRESETS: [Preset; 6] = [
    Preset {
        id: "inshore-flats",
        base: "apple-satellite",
        layers: &[
            on("clarity"),
            on("tide-stations"),
            on("current-stations"),
            on("spots"),
        ],
    },
    Preset {
        id: "offshore-pelagics",
        base: "apple-standard",
        layers: &[
            on("sst"),
            on("chlorophyll"),
            on("hf-radar-currents"),
            on("gebco-relief"),
            on("buoys"),
            on("spots"),
        ],
    },
    Preset {
        id: "trout-stream",
        base: "apple-standard",
        layers: &[
            on("usgs-gauges"),
            on("nwps-gauges"),
            on("radar"),
            on("spots"),
        ],
    },
    Preset {
        id: "bass-lake",
        base: "apple-satellite",
        layers: &[on("radar"), on("spots")],
    },
    Preset {
        id: "surf",
        base: "apple-standard",
        layers: &[on("buoys"), on("tide-stations"), on("radar"), on("spots")],
    },
    Preset {
        id: "great-lakes-trolling",
        base: "apple-standard",
        layers: &[
            on("great-lakes-sst"),
            on("buoys"),
            on("radar"),
            on("nwps-gauges"),
            on("spots"),
        ],
    },
];

/// A stored active-preset string that is not a built-in preset's id.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("unknown map preset {0:?}")]
pub struct UnknownPreset(pub String);

/// Resolves a stored active-preset string to a built-in preset: the id, trimmed and compared
/// without regard to case. A user's own preset is looked up by the caller first; callers fall back
/// to [`DEFAULT_PRESET`] for an unknown one.
///
/// # Errors
/// [`UnknownPreset`] when no built-in preset has that id.
pub fn resolve_preset(active: &str) -> Result<&'static Preset, UnknownPreset> {
    let wanted = active.trim();
    PRESETS
        .iter()
        .find(|p| p.id.eq_ignore_ascii_case(wanted))
        .ok_or_else(|| UnknownPreset(active.to_owned()))
}

/// The default preset.
pub fn default_preset() -> &'static Preset {
    // INVARIANT: DEFAULT_PRESET names an entry of PRESETS (a test checks it).
    PRESETS
        .iter()
        .find(|p| p.id == DEFAULT_PRESET)
        .unwrap_or(&PRESETS[0])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::Bbox;
    use crate::map::grid::blocks_for_view;
    use crate::map::tiles::{GibsSpec, TileOutcome, classify};

    /// The ids of the overlays as the configuration lists them (`KNOWN_LAYERS`), in drawer order.
    /// The configuration crate cannot be a dependency here, so it checks its own list against this
    /// one.
    const OVERLAY_IDS: [&str; 29] = [
        "gebco-relief",
        "sst",
        "sst-imagery",
        "chlorophyll",
        "chlorophyll-imagery",
        "clarity",
        "true-color-viirs-n20",
        "true-color-viirs-n21",
        "true-color-modis-terra",
        "true-color-modis-aqua",
        "hf-radar-currents",
        "great-lakes-sst",
        "radar",
        "wind",
        "pressure",
        "waves",
        "tide-stations",
        "current-stations",
        "usgs-gauges",
        "buoys",
        "nwps-gauges",
        "spots",
        "lake-contours-in",
        "lake-contours-mn",
        "artificial-reefs-ny",
        "enc-depths",
        "enc-seabed",
        "enc-special-areas",
        "enc-structure-points",
    ];
    const BASE_IDS: [&str; 4] = [
        "apple-standard",
        "apple-satellite",
        "apple-hybrid",
        "noaa-chart",
    ];

    const CAPS: [&str; 2] = [
        include_str!(
            "../../../../tests/fixtures/raw/gibs/wmts-epsg3857-best-capabilities.excerpt.xml"
        ),
        include_str!(
            "../../../../tests/fixtures/raw/gibs/wmts-epsg3857-best-capabilities.excerpt-2026-11-16.xml"
        ),
    ];
    fn tile(z: u8, x: u32, y: u32) -> TileKey {
        TileKey { z, x, y }
    }

    #[test]
    fn ids_equal_the_configuration_lists() {
        assert_eq!(LAYERS.map(|l| l.id), OVERLAY_IDS);
        assert_eq!(BASES.map(|l| l.id), BASE_IDS);
        let all: Vec<_> = ids().collect();
        let mut sorted = all.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), all.len(), "ids are unique");
    }

    #[test]
    fn charts_currents_and_depth_say_not_for_navigation() {
        for l in BASES.iter().chain(LAYERS.iter()) {
            assert!((0.0..=1.0).contains(&l.default_opacity), "{}", l.id);
            assert_eq!(l.is_base(), BASES.iter().any(|b| b.id == l.id), "{}", l.id);
        }
        for id in [
            "noaa-chart",
            "gebco-relief",
            "hf-radar-currents",
            "current-stations",
        ] {
            assert!(find(id).unwrap().not_for_navigation, "{id}");
        }
    }

    #[test]
    fn every_gibs_layer_is_in_the_capabilities_excerpts() {
        let mut checked = 0;
        for l in BASES.iter().chain(LAYERS.iter()) {
            let LayerKind::Tiles(TileSource::Gibs(spec)) = l.kind else {
                continue;
            };
            let id_tag = format!("<ows:Identifier>{}</ows:Identifier>", spec.layer);
            let block = CAPS
                .iter()
                .flat_map(|c| c.split("<Layer>"))
                .find(|b| b.contains(&id_tag))
                .unwrap_or_else(|| panic!("{} is not in the excerpts", spec.layer));
            assert!(
                block.contains(&format!(
                    "<TileMatrixSet>{}</TileMatrixSet>",
                    spec.matrix_set
                )),
                "{}",
                spec.layer
            );
            let mime = if spec.ext == "jpg" {
                "image/jpeg"
            } else {
                "image/png"
            };
            assert!(
                block.contains(&format!("<Format>{mime}</Format>")),
                "{}",
                spec.layer
            );
            assert!(
                spec.matrix_set
                    .ends_with(&format!("Level{}", spec.max_zoom)),
                "{}",
                spec.layer
            );
            assert_eq!(l.max_zoom, Some(spec.max_zoom));
            checked += 1;
        }
        assert_eq!(checked, 6);
    }

    #[test]
    fn recorded_tiles_are_reproduced_through_the_catalogue() {
        let date = Some(LocalDate {
            year: 2026,
            month: 11,
            day: 7,
        });
        let z7 = tile(7, 37, 48);
        for (id, expect) in [
            (
                "sst-imagery",
                "GHRSST_L4_MUR_Sea_Surface_Temperature/default/2026-11-07/GoogleMapsCompatible_Level7/7/48/37.png",
            ),
            (
                "chlorophyll-imagery",
                "VIIRS_NOAA20_Chlorophyll_a/default/2026-11-07/GoogleMapsCompatible_Level7/7/48/37.png",
            ),
            (
                "true-color-viirs-n20",
                "VIIRS_NOAA20_CorrectedReflectance_TrueColor/default/2026-11-07/GoogleMapsCompatible_Level9/7/48/37.jpg",
            ),
        ] {
            assert_eq!(
                find(id).unwrap().tile_url(z7, date, 0).unwrap(),
                format!("https://gibs.earthdata.nasa.gov/wmts/epsg3857/best/{expect}"),
                "{id}"
            );
        }
        let chart = find("noaa-chart").unwrap();
        assert!(
            chart
                .tile_url(tile(14, 4823, 6160), None, 0)
                .unwrap()
                .ends_with("/12/6160/4823.png")
        );
        assert!(
            chart
                .tile_url(tile(16, 19291, 24640), None, 0)
                .unwrap()
                .ends_with("/14/24640/19291.png")
        );
        assert_eq!(chart.tile_url(tile(1, 0, 0), None, 0), None);
        // Deeper than the matrix set has: no URL, use over-zoom.
        assert_eq!(
            find("sst-imagery")
                .unwrap()
                .tile_url(tile(8, 74, 96), date, 0),
            None
        );
        assert!(
            find("gebco-relief")
                .unwrap()
                .tile_url(tile(8, 75, 96), None, 0)
                .unwrap()
                .contains("bbox=-82967")
        );
        assert!(
            find("radar")
                .unwrap()
                .tile_url(tile(8, 75, 96), None, 10)
                .unwrap()
                .contains("-m10m")
        );
        assert_eq!(find("spots").unwrap().tile_url(z7, None, 0), None);
    }

    #[test]
    fn grid_layers_name_the_documented_datasets() {
        let grid = |id: &str| match find(id).unwrap().kind {
            LayerKind::Grid(g) => g,
            k => panic!("{id}: {k:?}"),
        };
        assert_eq!(grid("sst").primary.source().dataset, "jplMURSST41");
        assert_eq!(grid("sst").primary.source().variables, ["analysed_sst"]);
        assert_eq!(
            grid("chlorophyll").primary.dataset,
            "noaacwN20VIIRSchlaDaily"
        );
        assert!(grid("chlorophyll").primary.altitude_axis);
        let clarity = grid("clarity");
        assert_eq!(clarity.primary.dataset, "noaacwN21VIIRSkd490NSWUSDaily");
        assert_eq!(
            clarity.fallback.unwrap().dataset,
            "noaacwN20VIIRSkd490NSWUSDaily"
        );
        assert_eq!(
            find("clarity").unwrap().availability,
            Availability::Available
        );
        let hf = grid("hf-radar-currents");
        assert_eq!(hf.primary.variables, ["water_u", "water_v"]);
        assert_eq!(hf.draw, GridDraw::Arrows);
        assert_eq!(grid("great-lakes-sst").primary.node, GridHost::Glerl);
        assert_eq!(grid("great-lakes-sst").primary.variables, ["sst"]);
    }

    #[test]
    fn gridded_weather_is_unavailable_with_a_reason() {
        for id in ["wind", "pressure", "waves"] {
            assert_eq!(
                find(id).unwrap().availability,
                Availability::Unavailable {
                    reason: UnavailableReason::NoSource
                }
            );
        }
        let unavailable = LAYERS
            .iter()
            .filter(|l| l.availability != Availability::Available)
            .count();
        assert_eq!(unavailable, 3);
    }

    #[test]
    fn tiled_layers_have_a_service_for_classification() {
        for l in BASES.iter().chain(LAYERS.iter()) {
            let tiled = matches!(l.kind, LayerKind::Tiles(_));
            assert_eq!(l.tile_service().is_some(), tiled, "{}", l.id);
        }
        let ncds = find("noaa-chart").unwrap().tile_service().unwrap();
        assert_eq!(
            classify(ncds, 200, Some("text/html"), 700, 0),
            TileOutcome::Retry { after_s: 3 }
        );
    }

    fn layer(id: &str) -> &'static Layer {
        find(id).unwrap()
    }

    fn ncds_groups(id: &str) -> &'static str {
        match layer(id).kind {
            LayerKind::Tiles(TileSource::NcdsWms { groups }) => groups,
            other => panic!("{id}: {other:?}"),
        }
    }

    #[test]
    fn enc_pictures_are_ncds_groups_with_a_minimum_zoom() {
        use crate::map::tiles::{Bounds3857, ncds_wms_bbox_url};
        for (id, group, expected) in [
            ("enc-depths", Group::Depth, "2"),
            ("enc-seabed", Group::Structure, "3"),
            ("enc-special-areas", Group::Structure, "5"),
        ] {
            let l = layer(id);
            assert_eq!(l.group, group, "{id}");
            assert_eq!(ncds_groups(id), expected);
            assert_eq!(l.tile_service(), Some(TileService::Ncds));
            assert!(l.not_for_navigation, "{id}");
            assert_eq!(l.availability, Availability::Available);
            assert_eq!(l.min_zoom, Some(12), "{id}");
            assert_eq!(l.max_zoom, None);
            let url = l.tile_url(tile(14, 4823, 6160), None, 0).unwrap();
            assert!(url.contains(&format!("&LAYERS={expected}&")), "{url}");
            assert!(url.ends_with("&FORMAT=image/png&TRANSPARENT=TRUE"), "{url}");
        }
        // Asked together, groups 2 and 3 are the request recorded with the fixtures.
        let bounds = Bounds3857 {
            min_x: -8_243_208.0,
            min_y: 4_965_255.0,
            max_x: -8_234_302.0,
            max_y: 4_974_067.0,
        };
        let together = format!(
            "{},{}",
            ncds_groups("enc-depths"),
            ncds_groups("enc-seabed")
        );
        assert_eq!(
            ncds_wms_bbox_url(&together, bounds),
            "https://gis.charttools.noaa.gov/arcgis/rest/services/MCS/NOAAChartDisplay/MapServer/exts/MaritimeChartService/WMSServer?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=2,3&STYLES=&CRS=EPSG:3857&BBOX=-8243208,4965255,-8234302,4974067&WIDTH=256&HEIGHT=256&FORMAT=image/png&TRANSPARENT=TRUE"
        );
    }

    #[test]
    fn enc_points_are_drawn_on_the_device_from_zoom_9() {
        let l = layer("enc-structure-points");
        assert_eq!(l.kind, LayerKind::EncPoints);
        assert_eq!(l.group, Group::Structure);
        assert!(l.not_for_navigation);
        assert_eq!(l.min_zoom, Some(9));
        assert_eq!(l.tile_service(), None);
        assert_eq!(l.tile_url(tile(12, 1, 1), None, 0), None);
    }

    #[test]
    fn presets_use_existing_available_ids_and_resolve_by_id() {
        for p in &PRESETS {
            assert!(find(p.id).is_none(), "preset ids stay apart from layer ids");
            assert!(BASES.iter().any(|b| b.id == p.base), "{}", p.id);
            assert!(!p.layers.is_empty(), "{}", p.id);
            for pl in p.layers {
                let l = LAYERS
                    .iter()
                    .find(|l| l.id == pl.id)
                    .unwrap_or_else(|| panic!("{}: unknown layer {}", p.id, pl.id));
                assert_eq!(
                    l.availability,
                    Availability::Available,
                    "{}: {}",
                    p.id,
                    pl.id
                );
            }
            let mut seen: Vec<_> = p.layers.iter().map(|l| l.id).collect();
            seen.sort_unstable();
            seen.dedup();
            assert_eq!(seen.len(), p.layers.len(), "{}", p.id);
        }
        assert_eq!(
            PRESETS.map(|p| p.id),
            [
                "inshore-flats",
                "offshore-pelagics",
                "trout-stream",
                "bass-lake",
                "surf",
                "great-lakes-trolling"
            ]
        );
        assert_eq!(resolve_preset("surf").unwrap().id, "surf");
        assert_eq!(
            resolve_preset("  Trout-Stream ").unwrap().id,
            "trout-stream"
        );
        assert_eq!(
            resolve_preset("Sunday morning"),
            Err(UnknownPreset("Sunday morning".to_owned()))
        );
        assert_eq!(default_preset().id, DEFAULT_PRESET);
        assert_eq!(resolve_preset(DEFAULT_PRESET).unwrap().id, DEFAULT_PRESET);
    }

    #[test]
    fn every_layer_ramp_resolves_with_a_drawable_default_range() {
        for l in BASES.iter().chain(LAYERS.iter()) {
            if let Some(id) = l.ramp {
                let ramp = render::ramp(id).unwrap_or_else(|| panic!("{}: no ramp {id}", l.id));
                assert_eq!(l.colour_ramp(), Some(ramp));
                assert!(l.default_range().unwrap().is_valid(), "{}", l.id);
            } else {
                assert!(l.colour_ramp().is_none() && l.default_range().is_none());
            }
            if let LayerKind::Grid(_) = l.kind {
                assert!(l.ramp.is_some(), "{}: a grid layer colours values", l.id);
            }
        }
        let range = |id: &str| find(id).unwrap().default_range().unwrap();
        assert_eq!(range("sst"), RampRange::linear(0.0, 30.0));
        assert_eq!(range("great-lakes-sst"), range("sst"));
        assert_eq!(range("chlorophyll"), RampRange::log10(0.03, 30.0));
        // Kd490 seen at 0.08 to 1.48 per metre off New York and 0.28 to 2.94 in Lake Erie.
        assert_eq!(range("clarity"), RampRange::log10(0.02, 3.0));
        assert_eq!(range("hf-radar-currents"), RampRange::linear(0.0, 1.5));
        assert_eq!(find("sst-imagery").unwrap().ramp, Some("thermal"));
    }

    /// A phone-sized map (390 by 844 points) at `zoom` around New York Harbor.
    fn harbor_view(zoom: f64) -> Bbox {
        let (lat, lon): (f64, f64) = (40.70, -74.02);
        let deg_per_pt = 360.0 / (256.0 * zoom.exp2());
        let half_w = 390.0 / 2.0 * deg_per_pt;
        let half_h = 844.0 / 2.0 * deg_per_pt * lat.to_radians().cos();
        Bbox {
            west: lon - half_w,
            south: lat - half_h,
            east: lon + half_w,
            north: lat + half_h,
        }
    }

    #[test]
    fn a_grid_layer_is_drawn_from_a_zoom_where_a_view_needs_a_dozen_requests() {
        for l in &LAYERS {
            let LayerKind::Grid(g) = l.kind else {
                continue;
            };
            let min = l
                .min_zoom
                .unwrap_or_else(|| panic!("{}: no minimum zoom", l.id));
            let zoom = f64::from(min);
            for spec in std::iter::once(g.primary).chain(g.fallback) {
                let n = blocks_for_view(&spec.source(), harbor_view(zoom), zoom).len();
                println!("{} ({}): {n} requests at zoom {min}", l.id, spec.dataset);
                assert!(
                    (1..=12).contains(&n),
                    "{} ({}): {n} requests at zoom {min}",
                    l.id,
                    spec.dataset
                );
            }
        }
        assert_eq!(find("sst").unwrap().min_zoom, Some(5));
        assert_eq!(find("great-lakes-sst").unwrap().min_zoom, Some(5));
    }

    #[test]
    fn gibs_specs_in_the_catalogue_are_the_documented_ones() {
        let spec: GibsSpec = MUR_GIBS;
        assert_eq!(spec.max_zoom, 7);
        assert_eq!(true_colour("X").max_zoom, 9);
    }

    #[test]
    fn state_feature_layers_are_not_for_navigation_with_a_coverage_box() {
        let feature = |id: &str| match find(id).unwrap().kind {
            LayerKind::Features(f) => f,
            other => panic!("{id}: {other:?}"),
        };
        for (id, group, kind) in [
            ("lake-contours-in", Group::Depth, FeatureKind::LakeContours),
            ("lake-contours-mn", Group::Depth, FeatureKind::LakeContours),
            (
                "artificial-reefs-ny",
                Group::Structure,
                FeatureKind::ArtificialReefs,
            ),
        ] {
            let l = find(id).unwrap();
            let f = feature(id);
            assert_eq!(l.group, group, "{id}");
            assert_eq!(f.kind, kind, "{id}");
            assert!(l.not_for_navigation, "{id}");
            assert!(l.min_zoom.is_some(), "{id}");
            assert_eq!(l.availability, Availability::Available, "{id}");
            assert!(f.coverage.west < f.coverage.east, "{id}");
            assert!(f.coverage.south < f.coverage.north, "{id}");
        }
    }

    #[test]
    fn grid_layers_find_their_datasets() {
        let clarity = find("clarity").unwrap().grid().unwrap();
        let ids: Vec<_> = clarity.specs().map(|s| s.dataset).collect();
        assert_eq!(
            ids,
            [
                "noaacwN21VIIRSkd490NSWUSDaily",
                "noaacwN20VIIRSkd490NSWUSDaily"
            ]
        );
        assert_eq!(
            clarity
                .spec("noaacwN20VIIRSkd490NSWUSDaily")
                .unwrap()
                .dataset,
            "noaacwN20VIIRSkd490NSWUSDaily"
        );
        assert!(clarity.spec("jplMURSST41").is_none());
        assert!(find("radar").unwrap().grid().is_none());
        assert_eq!(Group::ALL.len(), 7);
        for l in BASES.iter().chain(LAYERS.iter()) {
            assert!(Group::ALL.contains(&l.group), "{}", l.id);
        }
    }

    #[test]
    fn the_state_coverage_boxes_hold_the_recorded_data() {
        let coverage = |id: &str| match find(id).unwrap().kind {
            LayerKind::Features(f) => f.coverage,
            other => panic!("{id}: {other:?}"),
        };
        let inside = |b: Bbox, lon: f64, lat: f64| {
            (b.west..=b.east).contains(&lon) && (b.south..=b.north).contains(&lat)
        };
        // Crooked Creek Lake, Indiana; Pine Lake, Minnesota; two New York reef corners.
        assert!(inside(coverage("lake-contours-in"), -86.2976, 39.1244));
        assert!(inside(coverage("lake-contours-mn"), -93.07, 46.19));
        assert!(inside(coverage("artificial-reefs-ny"), -73.7175, 40.5296));
        assert!(inside(coverage("artificial-reefs-ny"), -72.46, 41.0));
    }
}
