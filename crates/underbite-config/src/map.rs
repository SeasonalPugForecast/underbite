//! The map section: which layers are shown and how, presets, the fish-finder settings and the
//! offline storage limit.
//!
//! Layers are named by [`LayerId`]. An id this version does not know decodes fine; migration drops
//! it with a note, and appends any layer the list lacks, hidden. Presets hold layer states only: no
//! coordinates and no keys, so they can be shared unchanged. Ranges, depths and temperatures are in
//! canonical units (°C, metres, m/s); the view converts them for display. Opacity is 0 to 1.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use underbite_core::map::LayerId;

use crate::schema::Issues;

/// The base maps, one of which is chosen at a time.
pub const KNOWN_BASES: [&str; 4] = [
    "apple-standard",
    "apple-satellite",
    "apple-hybrid",
    "noaa-chart",
];

/// The overlay layers in drawer order. The layer list of a settings file is completed from this
/// when it is imported.
pub const KNOWN_LAYERS: [&str; 42] = [
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
    "public-land",
    "access-points",
    "stocking-points",
    "species-records",
    "hab-beaches",
    "catches",
    "tracks",
    "catch-heat",
    "depth-shading",
    "depth-contours",
    "sst-breaks",
    "chlorophyll-breaks",
    "storms",
];

/// How much the map saves power: no animation and fewer layers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BatteryMode {
    /// On while the phone is in Low Power Mode.
    #[default]
    Auto,
    /// Always on.
    On,
    /// Always off.
    Off,
}

/// How one layer is shown.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LayerState {
    pub id: LayerId,
    /// Whether the layer is on the map.
    #[serde(default)]
    pub shown: bool,
    /// 0 (clear) to 1 (solid); none means the layer's own default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opacity: Option<f32>,
    /// The colour ramp by id; none means the layer's own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ramp: Option<String>,
    /// The value range the ramp spans, low and high, in canonical units; none means the layer's
    /// own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<(f32, f32)>,
}

impl LayerState {
    /// A hidden layer with the layer's own ramp and range.
    pub fn hidden(id: LayerId) -> Self {
        Self {
            id,
            shown: false,
            opacity: None,
            ramp: None,
            range: None,
        }
    }
}

/// A named set of layer states the user saved.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MapPreset {
    pub name: String,
    #[serde(default)]
    pub layers: Vec<LayerState>,
}

/// Depth shading, the highlight band and contours.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct DepthConfig {
    /// The depths to highlight, shallow and deep, in metres below the surface; none means no
    /// highlight.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub band_m: Option<(f32, f32)>,
    /// The depth between contour lines, in metres.
    pub contour_interval_m: f32,
    /// The depth colour ramp by id; none means the layer's own.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ramp: Option<String>,
}

impl Default for DepthConfig {
    fn default() -> Self {
        Self {
            band_m: None,
            contour_interval_m: 5.0,
            ramp: None,
        }
    }
}

/// How sharp a change must be to draw as a break. Both are set by the user.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct BreaksConfig {
    /// Sea surface temperature change, in °C per kilometre. The default is 0.5.
    pub sst_c_per_km: f32,
    /// Chlorophyll a change as a difference in log10 of the concentration, per kilometre. The
    /// usual way to say it is 0.15 over 4 km; stored per kilometre that is 0.0375, the default.
    pub chlorophyll_log10_per_km: f32,
}

impl Default for BreaksConfig {
    fn default() -> Self {
        Self {
            sst_c_per_km: 0.5,
            chlorophyll_log10_per_km: 0.0375,
        }
    }
}

/// `[map]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct MapConfig {
    /// The base map: one of [`KNOWN_BASES`].
    pub base: LayerId,
    /// The overlay layers in drawing order, first at the bottom.
    pub layers: Vec<LayerState>,
    /// The user's own presets. The built-in ones belong to the layer catalogue.
    pub presets: Vec<MapPreset>,
    /// The preset applied when the map opens: either the id of a built-in preset (kebab-case,
    /// such as `inshore-flats`; the layer catalogue defines the ids and the core words their
    /// labels) or the `name` of one of the user's own presets. When a user preset's name equals a
    /// built-in id, the user's preset wins.
    #[serde(alias = "preset")]
    pub active_preset: String,
    pub battery_mode: BatteryMode,
    pub depth: DepthConfig,
    pub breaks: BreaksConfig,
    /// Most storage offline packs may use, in megabytes.
    pub offline_max_mb: u32,
}

impl Default for MapConfig {
    fn default() -> Self {
        Self {
            base: LayerId::from_static(KNOWN_BASES[0]),
            layers: KNOWN_LAYERS
                .iter()
                .map(|id| LayerState::hidden(LayerId::from_static(id)))
                .collect(),
            presets: Vec::new(),
            active_preset: "inshore-flats".to_owned(),
            battery_mode: BatteryMode::default(),
            depth: DepthConfig::default(),
            breaks: BreaksConfig::default(),
            offline_max_mb: 1500,
        }
    }
}

fn check_layers(list: &[LayerState], path: &str, issues: &mut Issues) {
    for (i, layer) in list.iter().enumerate() {
        let at = format!("{path}[{i}]");
        let id = layer.id.as_str();
        if !KNOWN_LAYERS.contains(&id) {
            issues.warning(
                format!("{at}.id"),
                format!("`{id}` is not a known map layer and will be skipped"),
            );
        } else if list[..i].iter().any(|l| l.id == layer.id) {
            issues.warning(format!("{at}.id"), format!("`{id}` is listed twice"));
        }
        if let Some(o) = layer.opacity
            && !(o.is_finite() && (0.0..=1.0).contains(&o))
        {
            issues.error(
                format!("{at}.opacity"),
                "must be between 0 (clear) and 1 (solid)",
            );
        }
        if let Some(range) = layer.range {
            check_pair(range, &format!("{at}.range"), issues);
        }
    }
}

fn check_pair((low, high): (f32, f32), path: &str, issues: &mut Issues) {
    if !(low.is_finite() && high.is_finite() && low < high) {
        issues.error(path, "the low end must be below the high end");
    }
}

fn check_positive(value: f32, path: &str, issues: &mut Issues) {
    if !(value.is_finite() && value > 0.0) {
        issues.error(path, "must be greater than 0");
    }
}

impl MapConfig {
    /// Adds every problem in the section to `issues`.
    pub(crate) fn validate(&self, issues: &mut Issues) {
        if !KNOWN_BASES.contains(&self.base.as_str()) {
            issues.warning(
                "map.base",
                format!("`{}` is not a known base map", self.base),
            );
        }
        check_layers(&self.layers, "map.layers", issues);
        for (i, preset) in self.presets.iter().enumerate() {
            let at = format!("map.presets[{i}]");
            if preset.name.trim().is_empty() {
                issues.error(format!("{at}.name"), "a preset name is needed");
            } else if self.presets[..i].iter().any(|p| p.name == preset.name) {
                issues.warning(format!("{at}.name"), "another preset has this name");
            }
            check_layers(&preset.layers, &format!("{at}.layers"), issues);
        }
        if self.active_preset.trim().is_empty() {
            issues.error("map.active_preset", "a preset name is needed");
        }
        if let Some(band) = self.depth.band_m {
            check_pair(band, "map.depth.band_m", issues);
            if band.0 < 0.0 {
                issues.error("map.depth.band_m", "depths are metres below the surface");
            }
        }
        check_positive(
            self.depth.contour_interval_m,
            "map.depth.contour_interval_m",
            issues,
        );
        check_positive(self.breaks.sst_c_per_km, "map.breaks.sst_c_per_km", issues);
        check_positive(
            self.breaks.chlorophyll_log10_per_km,
            "map.breaks.chlorophyll_log10_per_km",
            issues,
        );
        if self.offline_max_mb == 0 {
            issues.warning("map.offline_max_mb", "0 turns offline maps off");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_ids_are_the_catalogues() {
        use underbite_core::map::catalogue::{BASES, LAYERS, PRESETS, resolve_preset};
        let layers: Vec<&str> = LAYERS.iter().map(|l| l.id).collect();
        let bases: Vec<&str> = BASES.iter().map(|l| l.id).collect();
        assert_eq!(layers, KNOWN_LAYERS);
        assert_eq!(bases, KNOWN_BASES);
        assert!(resolve_preset(&MapConfig::default().active_preset).is_ok());
        assert!(PRESETS.iter().all(|p| KNOWN_BASES.contains(&p.base)));
    }

    fn issues_of(text: &str) -> Vec<String> {
        let c: MapConfig = toml::from_str(text).unwrap();
        let mut issues = Issues::default();
        c.validate(&mut issues);
        issues.0.into_iter().map(|i| i.path).collect()
    }

    #[test]
    fn defaults_fill_missing_keys() {
        let m: MapConfig = toml::from_str("offline_max_mb = 500").unwrap();
        assert_eq!(
            (m.active_preset.as_str(), m.offline_max_mb),
            ("inshore-flats", 500)
        );
        assert_eq!(m.layers.len(), KNOWN_LAYERS.len());
        assert_eq!(m.depth.contour_interval_m, 5.0);
        assert_eq!(m.breaks, BreaksConfig::default());
    }

    #[test]
    fn the_old_preset_key_still_reads() {
        let m: MapConfig = toml::from_str("preset = \"Surf\"").unwrap();
        assert_eq!(m.active_preset, "Surf");
    }

    #[test]
    fn defaults_validate_clean_and_start_hidden() {
        let c = MapConfig::default();
        let mut issues = Issues::default();
        c.validate(&mut issues);
        assert_eq!(issues.0, []);
        assert!(
            c.layers
                .iter()
                .all(|l| *l == LayerState::hidden(l.id.clone()))
        );
    }

    #[test]
    fn the_default_section_serialises_as_documented() {
        let text = toml::to_string_pretty(&MapConfig::default()).unwrap();
        assert!(text.starts_with("base = \"apple-standard\"\n"), "{text}");
        assert!(text.contains("active_preset = \"inshore-flats\"\n"));
        assert!(text.contains("battery_mode = \"auto\"\n"));
        assert!(
            text.contains("[depth]\ncontour_interval_m = 5.0\n"),
            "{text}"
        );
        assert!(text.contains("[breaks]\nsst_c_per_km = 0.5\nchlorophyll_log10_per_km = 0.0375\n"));
        assert!(text.contains("offline_max_mb = 1500\n"));
    }

    #[test]
    fn presets_and_layer_settings_round_trip_through_toml() {
        let mut c = MapConfig::default();
        c.presets.push(MapPreset {
            name: "Trout stream".to_owned(),
            layers: vec![
                LayerState {
                    id: LayerId::new("usgs-gauges"),
                    shown: true,
                    opacity: Some(0.75),
                    ramp: Some("flow".to_owned()),
                    range: Some((0.5, 12.5)),
                },
                LayerState::hidden(LayerId::new("radar")),
            ],
        });
        c.depth.band_m = Some((4.5, 7.5));
        c.battery_mode = BatteryMode::On;
        c.layers[1].shown = true;
        let text = toml::to_string_pretty(&c).unwrap();
        let back: MapConfig = toml::from_str(&text).unwrap();
        assert_eq!(back, c);
        let json: MapConfig = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
        assert_eq!(json, c);
    }

    #[test]
    fn unknown_keys_are_refused() {
        assert!(toml::from_str::<MapConfig>("zoom = 3").is_err());
        assert!(toml::from_str::<MapConfig>("[depth]\nfeet = 3").is_err());
        assert!(toml::from_str::<MapConfig>("[[layers]]\nid = \"sst\"\nglow = 1").is_err());
    }

    #[test]
    fn bad_values_are_named() {
        let paths = issues_of(
            "active_preset = \" \"\n[depth]\nband_m = [9.0, 3.0]\ncontour_interval_m = 0.0\n\
             [breaks]\nsst_c_per_km = -1.0\n\
             [[layers]]\nid = \"sst\"\nopacity = 1.5\nrange = [20.0, 10.0]\n\
             [[layers]]\nid = \"sst\"\n\
             [[layers]]\nid = \"mystery\"\n\
             [[presets]]\nname = \"\"",
        );
        assert_eq!(
            paths,
            [
                "map.layers[0].opacity",
                "map.layers[0].range",
                "map.layers[1].id",
                "map.layers[2].id",
                "map.presets[0].name",
                "map.active_preset",
                "map.depth.band_m",
                "map.depth.contour_interval_m",
                "map.breaks.sst_c_per_km",
            ]
        );
    }
}
