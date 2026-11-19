//! Copernicus Marine WMTS: the per-dataset capabilities and the colour legends of the map layers
//! the catalogue lists behind the Copernicus opt-in (`copernicus-…`).
//!
//! The tiles themselves are requested by the map (`underbite_core::map::tiles::copernicus_tile_url`).
//! What this module adds is what the map needs to choose a day: [`capabilities_url`] names the
//! small per-dataset document (never the service-wide one, about 70 MB), [`parse_capabilities`]
//! reads each layer's styles and `time` range from it, and [`legend_url`] and [`parse_legend`] give
//! the colour table and value range of a style, which the app draws itself.
//!
//! The service is open without an account; the opt-in is the app's own switch. Nothing here keeps
//! records of use: tiles live in the tile cache and nothing else is stored.

use quick_xml::events::Event;
use quick_xml::reader::Reader;
use serde::Deserialize;
use underbite_core::map::catalogue::CopernicusSpec;
use underbite_core::map::tiles::COPERNICUS_WMTS;
use underbite_core::time::{LocalDate, parse_date};

use crate::request::encode_path_segment;
use crate::source::SourceError;

fn parse_error(message: impl Into<String>) -> SourceError {
    SourceError::Parse(message.into())
}

/// The capabilities document of the dataset a layer is in: one product and dataset, a few dozen
/// kilobytes. Fetched by the build tooling or on demand, never as the service-wide document.
pub fn capabilities_url(spec: &CopernicusSpec) -> String {
    format!(
        "{COPERNICUS_WMTS}/{}/{}?SERVICE=WMTS&REQUEST=GetCapabilities",
        spec.product, spec.dataset
    )
}

/// The JSON legend of a layer's style: the 256-step colour table and the value range. The
/// `LegendURL` hrefs in the capabilities are encoded twice and answer HTTP 400, so this builds the
/// URL with a single encoding.
pub fn legend_url(spec: &CopernicusSpec) -> String {
    format!(
        "{COPERNICUS_WMTS}?SERVICE=WMTS&REQUEST=GetLegend&LAYER={}&STYLE={}&FORMAT=application%2Fjson",
        encode_path_segment(&spec.layer()),
        encode_path_segment(spec.style)
    )
}

/// The days a layer has data for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimeRange {
    /// The first day.
    pub first: LocalDate,
    /// The last day: for the model layers days ahead of today, for observations the newest day.
    pub last: LocalDate,
    /// The day the service answers with when a request has no `TIME`.
    pub default: LocalDate,
}

impl TimeRange {
    /// Whether the service has a picture for `day`.
    pub fn contains(&self, day: LocalDate) -> bool {
        let key = |d: LocalDate| (d.year, d.month, d.day);
        (key(self.first)..=key(self.last)).contains(&key(day))
    }
}

/// One layer of a capabilities document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayerCapabilities {
    /// `<product>/<dataset>/<variable>`, as the `LAYER` parameter takes it.
    pub identifier: String,
    /// The styles, the default first.
    pub styles: Vec<String>,
    /// The `time` dimension; none when the layer has no daily range.
    pub time: Option<TimeRange>,
}

/// The first ten characters of an ISO 8601 instant, as a date.
fn day_of(instant: &str) -> Result<LocalDate, SourceError> {
    let head = instant
        .trim()
        .get(..10)
        .ok_or_else(|| parse_error(format!("bad time `{instant}`")))?;
    parse_date(head).map_err(|e| parse_error(format!("bad time `{instant}`: {e}")))
}

/// A `start/end/P1D` interval and its default day. A step other than one day is refused: the map
/// asks for one picture per day.
fn time_range(interval: &str, default: &str) -> Result<TimeRange, SourceError> {
    let mut parts = interval.trim().split('/');
    let (Some(first), Some(last), Some(step), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(parse_error(format!("time interval `{interval}`")));
    };
    if step != "P1D" {
        return Err(parse_error(format!("time step `{step}`, not one day")));
    }
    Ok(TimeRange {
        first: day_of(first)?,
        last: day_of(last)?,
        default: day_of(default)?,
    })
}

#[derive(Default)]
struct Partial {
    identifier: Option<String>,
    styles: Vec<String>,
    default_style: Option<String>,
    dimension: Option<String>,
    time_default: Option<String>,
    time_interval: Option<String>,
}

impl Partial {
    fn finish(self) -> Result<LayerCapabilities, SourceError> {
        let identifier = self
            .identifier
            .ok_or_else(|| parse_error("layer without an identifier"))?;
        let mut styles = self.styles;
        if let Some(default) = self.default_style
            && let Some(at) = styles.iter().position(|s| *s == default)
        {
            styles[..=at].rotate_right(1);
        }
        let time = match (self.time_interval, self.time_default) {
            (Some(interval), Some(default)) => Some(time_range(&interval, &default)?),
            (Some(interval), None) => {
                let last = interval.split('/').nth(1).unwrap_or_default();
                Some(time_range(&interval, last)?)
            }
            _ => None,
        };
        Ok(LayerCapabilities {
            identifier,
            styles,
            time,
        })
    }
}

/// Parses a dataset's capabilities into its layers, in document order.
///
/// # Errors
/// [`SourceError::Parse`] when the XML is not a capabilities document, or a layer has no
/// identifier or a `time` dimension that is not daily.
pub fn parse_capabilities(text: &str) -> Result<Vec<LayerCapabilities>, SourceError> {
    let mut reader = Reader::from_str(text);
    let mut path: Vec<String> = Vec::new();
    let mut layers = Vec::new();
    let mut current: Option<Partial> = None;
    let mut style_is_default = false;
    let mut seen_root = false;
    loop {
        let event = reader
            .read_event()
            .map_err(|e| parse_error(format!("capabilities: {e}")))?;
        match event {
            Event::Start(e) => {
                let name = AsRef::<str>::as_ref(&e.local_name()).to_owned();
                match name.as_str() {
                    "Capabilities" => seen_root = true,
                    "Layer" => current = Some(Partial::default()),
                    "Style" => {
                        style_is_default = e.attributes().flatten().any(|a| {
                            AsRef::<str>::as_ref(&a.key.local_name()) == "isDefault"
                                && a.value.as_ref() == "true"
                        });
                    }
                    "Dimension" => {
                        if let Some(p) = current.as_mut() {
                            p.dimension = None;
                        }
                    }
                    _ => {}
                }
                path.push(name);
            }
            Event::End(e) => {
                let name = AsRef::<str>::as_ref(&e.local_name()).to_owned();
                path.pop();
                if name == "Layer"
                    && let Some(p) = current.take()
                {
                    layers.push(p.finish()?);
                }
            }
            Event::Text(t) => {
                let Some(p) = current.as_mut() else { continue };
                let text = t.xml10_content();
                let text = text.trim();
                if text.is_empty() {
                    continue;
                }
                let n = path.len();
                let here = path.last().map(String::as_str);
                let parent = n
                    .checked_sub(2)
                    .and_then(|i| path.get(i))
                    .map(String::as_str);
                match (parent, here) {
                    (Some("Layer"), Some("Identifier")) => p.identifier = Some(text.to_owned()),
                    (Some("Style"), Some("Identifier")) => {
                        p.styles.push(text.to_owned());
                        if style_is_default {
                            p.default_style = Some(text.to_owned());
                        }
                    }
                    (Some("Dimension"), Some("Identifier")) => p.dimension = Some(text.to_owned()),
                    (Some("Dimension"), Some("Default"))
                        if p.dimension.as_deref() == Some("time") =>
                    {
                        p.time_default = Some(text.to_owned());
                    }
                    (Some("Dimension"), Some("Value"))
                        if p.dimension.as_deref() == Some("time") =>
                    {
                        p.time_interval = Some(text.to_owned());
                    }
                    _ => {}
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if !seen_root {
        return Err(parse_error("no <Capabilities> element"));
    }
    if !path.is_empty() {
        return Err(parse_error("capabilities end inside an element"));
    }
    Ok(layers)
}

/// The capabilities of the layer `spec` names, from its dataset's document.
///
/// # Errors
/// [`SourceError::Parse`] when the document is unreadable, or [`SourceError::NotFound`] when it
/// does not list the layer (the dataset's version suffix has moved on).
pub fn layer_capabilities(
    text: &str,
    spec: &CopernicusSpec,
) -> Result<LayerCapabilities, SourceError> {
    let wanted = spec.layer();
    parse_capabilities(text)?
        .into_iter()
        .find(|l| l.identifier == wanted)
        .ok_or(SourceError::NotFound(Some(wanted)))
}

/// A style's colour table and value range, as the legend JSON gives them.
#[derive(Clone, Debug, PartialEq)]
pub struct Legend {
    /// The variable's name, such as `Temperature`.
    pub variable_name: String,
    /// The unit the range is in (`degrees_C`, `kelvin`, `m s-1`), when the service gives one.
    pub units: Option<String>,
    /// The value at the first colour: fixed per layer, not the day's range.
    pub value_min: f64,
    /// The value at the last colour.
    pub value_max: f64,
    /// Whether the colours are spread on a logarithmic scale.
    pub log_scale: bool,
    /// The colours from `value_min` to `value_max`.
    pub colours: Vec<[u8; 3]>,
}

#[derive(Deserialize)]
struct LegendFile {
    continuous: LegendContinuous,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegendContinuous {
    #[serde(default)]
    log_scale: bool,
    value_min: f64,
    value_max: f64,
    #[serde(default)]
    variable_name: String,
    cmap: LegendCmap,
    #[serde(default)]
    units: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegendCmap {
    color_map: Vec<[u8; 3]>,
}

/// Parses the legend JSON of a continuous style.
///
/// # Errors
/// [`SourceError::Parse`] when the body is not a continuous legend with a colour table and a
/// finite, increasing value range.
pub fn parse_legend(body: &[u8]) -> Result<Legend, SourceError> {
    let file: LegendFile =
        serde_json::from_slice(body).map_err(|e| parse_error(format!("legend: {e}")))?;
    let c = file.continuous;
    if c.cmap.color_map.is_empty()
        || !(c.value_min.is_finite() && c.value_max.is_finite() && c.value_min < c.value_max)
    {
        return Err(parse_error("legend without a colour table or range"));
    }
    Ok(Legend {
        variable_name: c.variable_name,
        units: c.units,
        value_min: c.value_min,
        value_max: c.value_max,
        log_scale: c.log_scale,
        colours: c.cmap.color_map,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{fixture_bytes, fixture_str};
    use underbite_core::map::catalogue::{self, LAYERS};

    fn day(year: i32, month: u8, d: u8) -> LocalDate {
        LocalDate {
            year,
            month,
            day: d,
        }
    }

    fn specs() -> Vec<(&'static str, CopernicusSpec)> {
        LAYERS
            .iter()
            .filter_map(|l| l.copernicus().map(|s| (l.id, *s)))
            .collect()
    }

    /// The recorded capabilities of each catalogue layer's dataset.
    fn capabilities_of(id: &str) -> String {
        match id {
            "copernicus-currents" => fixture_str(
                "copernicus",
                "wmts-capabilities-glo-phy-cur-anfc-0.083deg-P1D-m.xml",
            ),
            "copernicus-sst" => fixture_str(
                "copernicus",
                "wmts-capabilities-sst-l4-nrt-metoffice-v2.xml",
            ),
            "copernicus-sea-level" => fixture_str(
                "tier2",
                "copernicus-marine-wmts-capabilities-glo-phy-anfc-0.083deg-P1D-m.xml",
            ),
            other => panic!("no recorded capabilities for {other}"),
        }
    }

    #[test]
    fn the_catalogue_lists_three_layers_and_each_is_in_its_recorded_capabilities() {
        let specs = specs();
        let ids: Vec<_> = specs.iter().map(|(id, _)| *id).collect();
        assert_eq!(
            ids,
            [
                "copernicus-currents",
                "copernicus-sst",
                "copernicus-sea-level"
            ]
        );
        for (id, spec) in &specs {
            let caps = layer_capabilities(&capabilities_of(id), spec).unwrap();
            assert_eq!(caps.identifier, spec.layer(), "{id}");
            assert!(
                caps.styles.iter().any(|s| s == spec.style),
                "{id}: the catalogue's style is one the layer offers"
            );
            assert!(caps.time.is_some(), "{id}");
        }
        // Every layer is behind the opt-in and carries its product's DOI path only through the
        // view's credit; the id says which source it is.
        assert!(
            LAYERS
                .iter()
                .filter(|l| l.copernicus().is_some())
                .all(underbite_core::map::catalogue::Layer::needs_copernicus)
        );
        assert!(
            catalogue::find("copernicus-currents")
                .unwrap()
                .not_for_navigation
        );
    }

    #[test]
    fn the_current_layers_catalogue_style_is_in_the_capabilities_with_vectors() {
        let spec = *catalogue::find("copernicus-currents")
            .unwrap()
            .copernicus()
            .unwrap();
        let caps = layer_capabilities(&capabilities_of("copernicus-currents"), &spec).unwrap();
        assert_eq!(caps.styles.len(), 6);
        assert!(
            caps.styles
                .contains(&"cmap:speed,vectorStyle:solidAndVector".to_owned())
        );
        let time = caps.time.unwrap();
        assert_eq!(time.first, day(2022, 6, 1));
        assert_eq!(time.last, day(2026, 11, 26));
        assert_eq!(time.default, day(2026, 11, 17));
        assert!(time.contains(day(2026, 11, 20)));
        assert!(!time.contains(day(2026, 11, 27)));
        assert!(!time.contains(day(2022, 5, 31)));
    }

    #[test]
    fn the_satellite_sst_range_ends_the_day_before_the_current_one() {
        let spec = *catalogue::find("copernicus-sst")
            .unwrap()
            .copernicus()
            .unwrap();
        let caps = layer_capabilities(&capabilities_of("copernicus-sst"), &spec).unwrap();
        let time = caps.time.unwrap();
        // Nanosecond digits in the first instant are read as a day.
        assert_eq!(time.first, day(2024, 1, 17));
        assert_eq!(
            (time.last, time.default),
            (day(2026, 11, 16), day(2026, 11, 16))
        );
    }

    #[test]
    fn the_sea_level_layer_is_found_among_the_datasets_other_variables() {
        let text = capabilities_of("copernicus-sea-level");
        let all = parse_capabilities(&text).unwrap();
        assert_eq!(all.len(), 15);
        let spec = *catalogue::find("copernicus-sea-level")
            .unwrap()
            .copernicus()
            .unwrap();
        let zos = layer_capabilities(&text, &spec).unwrap();
        assert_eq!(zos.styles.first().map(String::as_str), Some("cmap:viridis"));
        assert_eq!(zos.time.unwrap().last, day(2026, 11, 17));
    }

    #[test]
    fn a_dataset_that_has_moved_on_is_not_found() {
        let mut spec = *catalogue::find("copernicus-sst")
            .unwrap()
            .copernicus()
            .unwrap();
        spec.variable = "nope";
        assert!(matches!(
            layer_capabilities(&capabilities_of("copernicus-sst"), &spec),
            Err(SourceError::NotFound(_))
        ));
    }

    #[test]
    fn the_other_recorded_datasets_parse_too() {
        let thetao = parse_capabilities(&fixture_str(
            "copernicus",
            "wmts-capabilities-glo-phy-thetao-anfc-0.083deg-P1D-m.xml",
        ))
        .unwrap();
        assert_eq!(thetao.len(), 1);
        assert_eq!(thetao[0].styles, ["cmap:thermal"]);
        let chl = parse_capabilities(&fixture_str(
            "copernicus",
            "wmts-capabilities-oc-plankton-gapfree-4km-P1D.xml",
        ))
        .unwrap();
        let time = chl[0].time.unwrap();
        // A rolling window of days, the opposite of the model layers' range.
        assert_eq!(
            (time.first, time.last),
            (day(2026, 11, 4), day(2026, 11, 15))
        );
    }

    #[test]
    fn other_documents_and_steps_are_refused() {
        assert!(parse_capabilities("<html/>").is_err());
        assert!(parse_capabilities("<Capabilities><Layer>").is_err());
        assert!(
            time_range(
                "2026-01-01T00:00:00Z/2026-01-10T00:00:00Z/PT6H",
                "2026-01-01"
            )
            .is_err()
        );
        assert!(time_range("2026-01-01T00:00:00Z", "2026-01-01").is_err());
        assert!(time_range("soon/later/P1D", "2026-01-01").is_err());
    }

    #[test]
    fn requests_name_the_dataset_and_encode_the_legend_once() {
        let spec = *catalogue::find("copernicus-sst")
            .unwrap()
            .copernicus()
            .unwrap();
        assert_eq!(
            capabilities_url(&spec),
            "https://wmts.marine.copernicus.eu/teroWmts/SST_GLO_SST_L4_NRT_OBSERVATIONS_010_001/METOFFICE-GLO-SST-L4-NRT-OBS-SST-V2?SERVICE=WMTS&REQUEST=GetCapabilities"
        );
        assert_eq!(
            legend_url(&spec),
            "https://wmts.marine.copernicus.eu/teroWmts?SERVICE=WMTS&REQUEST=GetLegend&LAYER=SST_GLO_SST_L4_NRT_OBSERVATIONS_010_001%2FMETOFFICE-GLO-SST-L4-NRT-OBS-SST-V2%2Fanalysed_sst&STYLE=cmap%3Athermal&FORMAT=application%2Fjson"
        );
    }

    #[test]
    fn the_recorded_legend_has_a_colour_table_and_range() {
        let legend = parse_legend(&fixture_bytes("copernicus", "wmts-legend-thetao.json")).unwrap();
        assert_eq!(legend.variable_name, "Temperature");
        assert_eq!(legend.units.as_deref(), Some("degrees_C"));
        assert!((legend.value_min + 1.741_762_876_510_620_1).abs() < 1e-12);
        assert!((legend.value_max - 30.036_579_132_080_078).abs() < 1e-12);
        assert!(!legend.log_scale);
        assert_eq!(legend.colours.len(), 256);
        assert_eq!(legend.colours[0], [4, 35, 51]);
        assert!(parse_legend(b"{}").is_err());
        assert!(
            parse_legend(
                br#"{"continuous":{"valueMin":2,"valueMax":1,"cmap":{"colorMap":[[0,0,0]]}}}"#
            )
            .is_err()
        );
    }
}
