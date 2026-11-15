//! NDBC active station list (`activestations.xml`): every station with its position and the
//! sensor flags NDBC publishes.
//!
//! The flags (`met`, `currents`, `waterquality`) are claims: a station can be `met="y"` and
//! report no wind for weeks. The candidates say what the XML claims and carry no observation
//! time; discovery checks the claim against the station's files (see
//! [`Ndbc::coverage_request`](super::Ndbc::coverage_request)).

use std::collections::BTreeMap;

use quick_xml::XmlVersion;
use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::Reader;
use underbite_core::model::{Coverage, Measurement, StationCandidate};
use underbite_core::provenance::{SourceId, StationKind, StationRef};

use crate::request::{Request, Resource, Target};
use crate::source::SourceError;

/// The station list.
pub const URL: &str = "https://www.ndbc.noaa.gov/activestations.xml";

/// Seconds the list may be reused when the server sends no cache headers.
const LIST_AGE_S: u32 = 6 * 3_600;

fn parse_error(message: impl Into<String>) -> SourceError {
    SourceError::Parse(message.into())
}

/// The request for the station list. `target` says what the caller is looking for; the list
/// itself is the same for every target.
pub fn request(target: Target) -> Request {
    Request::new(URL.to_string(), Resource::NdbcActiveStations, target).max_age_hint(LIST_AGE_S)
}

/// The station kind for NDBC's `type` attribute.
fn kind(station_type: &str) -> StationKind {
    match station_type {
        // C-MAN, oil rigs and other fixed platforms.
        "fixed" | "oilrig" | "other" => StationKind::Cman,
        _ => StationKind::Buoy,
    }
}

/// What the station's flags claim it offers.
fn offers(
    station_type: &str,
    met: bool,
    currents: bool,
    water_quality: bool,
) -> BTreeMap<Measurement, Coverage> {
    let mut wanted = Vec::new();
    if met {
        wanted.extend([
            Measurement::ObservedWind,
            Measurement::ObservedPressure,
            Measurement::WaterTemperature,
        ]);
        if station_type == "buoy" {
            wanted.push(Measurement::Waves);
        }
    }
    if currents {
        wanted.push(Measurement::OceanCurrent);
    }
    if water_quality {
        wanted.extend([
            Measurement::WaterTemperature,
            Measurement::Salinity,
            Measurement::Turbidity,
            Measurement::Chlorophyll,
            Measurement::Ph,
        ]);
    }
    wanted
        .into_iter()
        .map(|m| {
            (
                m,
                Coverage {
                    last_value_at: None,
                    primary: true,
                },
            )
        })
        .collect()
}

/// Attributes of one `<station>` element, entities decoded.
fn attributes(element: &BytesStart<'_>) -> Result<BTreeMap<String, String>, SourceError> {
    let mut map = BTreeMap::new();
    for attribute in element.attributes() {
        let attribute = attribute.map_err(|e| parse_error(format!("station attribute: {e}")))?;
        let key = AsRef::<str>::as_ref(&attribute.key).to_string();
        let value = attribute
            .normalized_value(XmlVersion::Implicit1_0)
            .map_err(|e| parse_error(format!("station attribute {key}: {e}")))?;
        map.insert(key, value.into_owned());
    }
    Ok(map)
}

fn candidate(attrs: &BTreeMap<String, String>) -> Result<StationCandidate, SourceError> {
    let get = |key: &str| attrs.get(key).map(String::as_str);
    let id = get("id")
        .filter(|id| !id.is_empty())
        .ok_or_else(|| parse_error("station without id"))?;
    let coordinate = |key: &str| {
        get(key)
            .and_then(|v| v.parse::<f64>().ok())
            .filter(|v| v.is_finite())
            .ok_or_else(|| parse_error(format!("station {id}: bad {key}")))
    };
    let station_type = get("type").unwrap_or_default();
    let yes = |key: &str| get(key) == Some("y");
    let name = get("name").map(str::trim).filter(|n| !n.is_empty());
    Ok(StationCandidate {
        station: StationRef {
            source: SourceId::NDBC,
            id: id.to_string(),
            name: name.map_or_else(|| id.to_ascii_uppercase(), str::to_string),
            kind: kind(station_type),
            latitude: coordinate("lat")?,
            longitude: coordinate("lon")?,
            bin: None,
            reference_id: None,
        },
        offers: offers(
            station_type,
            yes("met"),
            yes("currents"),
            yes("waterquality"),
        ),
    })
}

/// Parses the station list. A station without an id or a position is an error, since the format
/// has changed if it happens.
pub fn parse(text: &str) -> Result<Vec<StationCandidate>, SourceError> {
    let mut reader = Reader::from_str(text);
    let mut out = Vec::new();
    let mut seen_root = false;
    loop {
        let event = reader
            .read_event()
            .map_err(|e| parse_error(format!("station list: {e}")))?;
        match event {
            Event::Start(e) | Event::Empty(e) => match e.local_name().as_ref() {
                "station" => out.push(candidate(&attributes(&e)?)?),
                "stations" => seen_root = true,
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
    }
    if !seen_root {
        return Err(parse_error("no <stations> element"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::fixture_str;

    fn find<'a>(all: &'a [StationCandidate], id: &str) -> &'a StationCandidate {
        all.iter().find(|c| c.station.id == id).unwrap()
    }

    #[test]
    fn the_recorded_list_has_every_station() {
        let all = parse(&fixture_str("ndbc", "ndbc-activestations.xml")).unwrap();
        assert_eq!(all.len(), 1357);
        let ids: std::collections::BTreeSet<_> = all.iter().map(|c| &c.station.id).collect();
        assert_eq!(ids.len(), 1357);
        assert_eq!(all.iter().filter(|c| c.station.id.len() == 7).count(), 18);
        let buoy = find(&all, "44025");
        assert_eq!(buoy.station.kind, StationKind::Buoy);
        assert_eq!(
            (buoy.station.latitude, buoy.station.longitude),
            (40.258, -73.175)
        );
        assert_eq!(buoy.station.name, "LONG ISLAND - 30 NM South of Islip, NY");
        // Claims only: nothing says when a value last arrived.
        assert!(buoy.offers.values().all(|c| c.last_value_at.is_none()));
        assert!(buoy.offers.contains_key(&Measurement::ObservedWind));
        assert!(!buoy.offers.contains_key(&Measurement::Salinity));
        let battery = find(&all, "batn6");
        assert_eq!(battery.station.kind, StationKind::Cman);
        assert!(
            find(&all, "44091")
                .offers
                .contains_key(&Measurement::OceanCurrent)
        );
        assert!(
            find(&all, "44069")
                .offers
                .contains_key(&Measurement::Turbidity)
        );
    }

    #[test]
    fn entities_are_decoded() {
        let text = fixture_str("ndbc", "ndbc-activestations.xml");
        assert!(text.contains("&amp;") || text.contains("&quot;"));
        let all = parse(&text).unwrap();
        assert!(all.iter().any(|c| c.station.name.contains('&')));
        assert!(
            all.iter()
                .all(|c| !c.station.name.contains("&amp;") && !c.station.name.contains("&quot;"))
        );
    }

    #[test]
    fn nearby_stations_for_new_york_harbor() {
        let all = parse(&fixture_str("ndbc", "ndbc-activestations.xml")).unwrap();
        let pos = |id: &str| {
            let s = &find(&all, id).station;
            (s.latitude, s.longitude)
        };
        assert_eq!(pos("44065"), (40.368, -73.701));
        assert_eq!(pos("44069"), (40.699, -73.087));
        assert_eq!(pos("batn6"), (40.701, -74.014));
    }

    #[test]
    fn a_list_without_stations_or_positions_is_an_error() {
        assert!(parse("<html></html>").is_err());
        assert!(parse("<stations><station lat=\"1\" lon=\"2\"/></stations>").is_err());
        assert!(parse("<stations><station id=\"a\" lat=\"x\" lon=\"2\"/></stations>").is_err());
        assert!(parse("<stations><station id=\"a\" lat=\"1\"").is_err());
        let ok = parse("<stations><station id=\"a\" lat=\"1\" lon=\"2\" name=\"\"/></stations>");
        assert_eq!(ok.unwrap()[0].station.name, "A");
    }
}
