//! The `.underbite` document: envelope, payloads, encoding, decoding and fuzz.
//!
//! A file is JSON: `{"format":"underbite","v":1,"kind":"trip"|"spot"|"layers"|"profile", <kind>: {…}}`.
//! One file type carries every kind, so the OS registers a single document type. `trip` and `spot`
//! are defined here; `layers` and `profile` are reserved and decode as [`Document::Unsupported`]
//! until their payloads exist. Fields this build does not know are ignored within a known version.
//! A file never carries keys, the catch log or photos.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::trip::model::{Fuzz, PlaceProblem, Trip, TripError, TripPlace};

/// The `format` member of every file.
pub const FORMAT: &str = "underbite";
/// The payload version this build reads and writes.
pub const VERSION: u32 = 1;
/// The most bytes [`decode`] reads and [`encode`] writes.
pub const MAX_BYTES: usize = 1_000_000;

/// The `kind: "spot"` payload: one place and what its owner wrote about it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SpotShare {
    pub place: TripPlace,
    #[serde(default)]
    pub notes: String,
}

/// What a `.underbite` file holds.
#[derive(Clone, Debug, PartialEq)]
pub enum Document {
    /// `kind: "trip"`.
    Trip(Trip),
    /// `kind: "spot"`.
    Spot(SpotShare),
    /// A kind this build reserves but cannot read yet (`layers`, `profile`). It cannot be encoded.
    Unsupported { kind: String },
}

/// Why a file cannot be used.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ShareError {
    /// Its `v` is above this build's: a newer version of the app made it.
    #[error("this file needs a newer version of the app")]
    NeedsNewerVersion,
    /// The last day is before the first.
    #[error("the dates are the wrong way round")]
    DatesBackwards,
    /// The trip covers more than 31 days.
    #[error("the trip is longer than 31 days")]
    TooLong,
    /// Not an `.underbite` file, or one with a place or payload that cannot exist.
    #[error("that is not a file this app can read")]
    Unreadable,
    /// The input is larger than [`MAX_BYTES`].
    #[error("the file is too large")]
    TooLarge,
}

impl From<TripError> for ShareError {
    fn from(e: TripError) -> Self {
        match e {
            TripError::DatesBackwards => Self::DatesBackwards,
            TripError::TooLong => Self::TooLong,
            TripError::NoPlaces | TripError::Place { .. } => Self::Unreadable,
        }
    }
}

impl From<PlaceProblem> for ShareError {
    fn from(_: PlaceProblem) -> Self {
        Self::Unreadable
    }
}

/// Reads a file.
///
/// The version is checked before the payload, so a file from a newer app reports
/// [`ShareError::NeedsNewerVersion`] whatever its payload looks like.
///
/// # Errors
/// [`ShareError`]: too large, not JSON or not this format, a newer `v`, an unknown `kind`, a
/// payload that does not parse, or a trip whose dates or places do not validate.
pub fn decode(bytes: &[u8]) -> Result<Document, ShareError> {
    if bytes.len() > MAX_BYTES {
        return Err(ShareError::TooLarge);
    }
    let Ok(Value::Object(mut file)) = serde_json::from_slice::<Value>(bytes) else {
        return Err(ShareError::Unreadable);
    };
    if file.get("format").and_then(Value::as_str) != Some(FORMAT) {
        return Err(ShareError::Unreadable);
    }
    match file.get("v").and_then(Value::as_u64) {
        Some(v) if v > u64::from(VERSION) => return Err(ShareError::NeedsNewerVersion),
        Some(v) if v >= 1 => {}
        _ => return Err(ShareError::Unreadable),
    }
    let kind = file
        .get("kind")
        .and_then(Value::as_str)
        .ok_or(ShareError::Unreadable)?
        .to_owned();
    match kind.as_str() {
        "trip" => {
            let trip: Trip = payload(&mut file, "trip")?;
            trip.validate()?;
            Ok(Document::Trip(trip))
        }
        "spot" => {
            let spot: SpotShare = payload(&mut file, "spot")?;
            spot.place.validate()?;
            Ok(Document::Spot(spot))
        }
        "layers" | "profile" => Ok(Document::Unsupported { kind }),
        _ => Err(ShareError::Unreadable),
    }
}

fn payload<T: serde::de::DeserializeOwned>(
    file: &mut Map<String, Value>,
    key: &str,
) -> Result<T, ShareError> {
    let value = file.remove(key).ok_or(ShareError::Unreadable)?;
    serde_json::from_value(value).map_err(|_| ShareError::Unreadable)
}

/// Writes a file: pretty-printed JSON with sorted keys, so the same document always gives the same
/// bytes.
///
/// # Errors
/// The same validation errors as [`decode`], [`ShareError::TooLarge`] when the output would exceed
/// [`MAX_BYTES`], and [`ShareError::Unreadable`] for [`Document::Unsupported`].
pub fn encode(doc: &Document) -> Result<String, ShareError> {
    let file = match doc {
        Document::Trip(trip) => {
            trip.validate()?;
            json!({"format": FORMAT, "v": VERSION, "kind": "trip", "trip": trip})
        }
        Document::Spot(spot) => {
            spot.place.validate()?;
            json!({"format": FORMAT, "v": VERSION, "kind": "spot", "spot": spot})
        }
        Document::Unsupported { .. } => return Err(ShareError::Unreadable),
    };
    let text = serde_json::to_string_pretty(&file).map_err(|_| ShareError::Unreadable)?;
    if text.len() > MAX_BYTES {
        return Err(ShareError::TooLarge);
    }
    Ok(text)
}

/// Reduces the location of every place in `doc` to `level`, deterministically (never by random
/// offsets, which two shares could average back).
///
/// A place never gets more precise: asking for a level at or above what a place already carries
/// leaves it alone. The level is recorded on the place so the receiver can say "approximate
/// location". `Exact` changes nothing; `Rounded1Km` rounds to 0.01 degrees; `AreaOnly` drops the
/// coordinates and keeps name, area and region. [`Document::Unsupported`] has no places.
pub fn fuzz(doc: &mut Document, level: Fuzz) {
    match doc {
        Document::Trip(trip) => {
            for place in &mut trip.places {
                fuzz_place(place, level);
            }
        }
        Document::Spot(spot) => fuzz_place(&mut spot.place, level),
        Document::Unsupported { .. } => {}
    }
}

fn fuzz_place(place: &mut TripPlace, level: Fuzz) {
    if level <= place.fuzz {
        return;
    }
    match level {
        Fuzz::Exact => {}
        Fuzz::Rounded1Km => {
            place.latitude = place.latitude.map(hundredth);
            place.longitude = place.longitude.map(hundredth);
        }
        Fuzz::AreaOnly => {
            place.latitude = None;
            place.longitude = None;
        }
    }
    place.fuzz = level;
}

/// `x` rounded to 0.01, with no negative zero.
fn hundredth(x: f64) -> f64 {
    (x * 100.0).round() / 100.0 + 0.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trip::model::tests::{battery, trip};
    use crate::trip::model::{Method, SpeciesId};

    fn spot() -> Document {
        Document::Spot(SpotShare {
            place: battery(),
            notes: "Park at the Battery Place garage.".into(),
        })
    }

    fn encoded(doc: &Document) -> Value {
        serde_json::from_str(&encode(doc).unwrap()).unwrap()
    }

    #[test]
    fn trip_round_trips() {
        let doc = Document::Trip(trip());
        let text = encode(&doc).unwrap();
        assert_eq!(decode(text.as_bytes()), Ok(doc.clone()));
        let v = encoded(&doc);
        assert_eq!(v["format"], "underbite");
        assert_eq!(v["v"], 1);
        assert_eq!(v["kind"], "trip");
        assert_eq!(v["trip"]["first"], "2027-07-12");
        assert_eq!(encode(&doc).unwrap(), text, "deterministic");
    }

    #[test]
    fn spot_round_trips() {
        let doc = spot();
        let text = encode(&doc).unwrap();
        assert_eq!(decode(text.as_bytes()), Ok(doc.clone()));
        assert_eq!(encoded(&doc)["kind"], "spot");
        assert_eq!(encoded(&doc)["spot"]["place"]["name"], "The Battery");
    }

    #[test]
    fn a_newer_version_is_refused_before_the_payload_is_read() {
        let doc = Document::Trip(trip());
        let mut v = encoded(&doc);
        v["v"] = 2.into();
        assert_eq!(
            decode(v.to_string().as_bytes()),
            Err(ShareError::NeedsNewerVersion)
        );
        let odd = br#"{"format":"underbite","v":2,"kind":"cairn","cairn":[1]}"#;
        assert_eq!(decode(odd), Err(ShareError::NeedsNewerVersion));
    }

    #[test]
    fn a_trip_over_31_days_is_too_long() {
        let mut t = trip();
        t.first = crate::time::LocalDate {
            year: 2027,
            month: 7,
            day: 1,
        };
        t.last = crate::time::LocalDate {
            year: 2027,
            month: 8,
            day: 1,
        };
        assert_eq!(encode(&Document::Trip(t.clone())), Err(ShareError::TooLong));
        t.last.day = 31;
        t.last.month = 7;
        let text = encode(&Document::Trip(t.clone())).unwrap();
        let mut v: Value = serde_json::from_str(&text).unwrap();
        assert!(decode(text.as_bytes()).is_ok());
        v["trip"]["last"] = "2027-08-01".into();
        assert_eq!(decode(v.to_string().as_bytes()), Err(ShareError::TooLong));
    }

    #[test]
    fn dates_backwards() {
        let mut v = encoded(&Document::Trip(trip()));
        v["trip"]["last"] = "2027-07-11".into();
        assert_eq!(
            decode(v.to_string().as_bytes()),
            Err(ShareError::DatesBackwards)
        );
        let mut t = trip();
        t.last = crate::time::LocalDate {
            year: 2027,
            month: 7,
            day: 11,
        };
        assert_eq!(encode(&Document::Trip(t)), Err(ShareError::DatesBackwards));
    }

    #[test]
    fn unreadable_inputs() {
        let good = encoded(&Document::Trip(trip()));
        let with = |f: &dyn Fn(&mut Value)| {
            let mut v = good.clone();
            f(&mut v);
            decode(v.to_string().as_bytes())
        };
        assert_eq!(decode(b""), Err(ShareError::Unreadable));
        assert_eq!(decode(b"not json"), Err(ShareError::Unreadable));
        assert_eq!(decode(b"[1,2]"), Err(ShareError::Unreadable));
        assert_eq!(decode(&[0xff, 0xfe]), Err(ShareError::Unreadable));
        assert_eq!(
            with(&|v| v["format"] = "whetherplan".into()),
            Err(ShareError::Unreadable)
        );
        assert_eq!(
            with(&|v| {
                v.as_object_mut().unwrap().remove("format");
            }),
            Err(ShareError::Unreadable)
        );
        assert_eq!(with(&|v| v["v"] = 0.into()), Err(ShareError::Unreadable));
        assert_eq!(with(&|v| v["v"] = "1".into()), Err(ShareError::Unreadable));
        assert_eq!(
            with(&|v| v["kind"] = "cairn".into()),
            Err(ShareError::Unreadable)
        );
        assert_eq!(
            with(&|v| v["kind"] = "spot".into()),
            Err(ShareError::Unreadable),
            "no spot payload"
        );
        assert_eq!(
            with(&|v| {
                v.as_object_mut().unwrap().remove("trip");
            }),
            Err(ShareError::Unreadable)
        );
        assert_eq!(
            with(&|v| v["trip"]["method"] = "swim".into()),
            Err(ShareError::Unreadable)
        );
        assert_eq!(
            with(&|v| v["trip"]["species"] = json!(["Striped Bass"])),
            Err(ShareError::Unreadable)
        );
        assert_eq!(
            with(&|v| v["trip"]["first"] = "2027-02-30".into()),
            Err(ShareError::Unreadable)
        );
        assert_eq!(
            with(&|v| v["trip"]["places"] = json!([])),
            Err(ShareError::Unreadable)
        );
        assert_eq!(
            with(&|v| v["trip"]["places"][0]["latitude"] = 123.0.into()),
            Err(ShareError::Unreadable)
        );
        assert_eq!(
            with(&|v| v["trip"]["places"][0]["time_zone"] = "Nowhere/Land".into()),
            Err(ShareError::Unreadable)
        );
    }

    #[test]
    fn input_over_a_megabyte_is_too_large() {
        let mut text = encode(&Document::Trip(trip())).unwrap();
        text.push_str(&" ".repeat(MAX_BYTES));
        assert_eq!(decode(text.as_bytes()), Err(ShareError::TooLarge));
        let mut t = trip();
        t.notes = "x".repeat(MAX_BYTES);
        assert_eq!(encode(&Document::Trip(t)), Err(ShareError::TooLarge));
        let at_limit = format!(
            "{}{}",
            encode(&Document::Trip(trip())).unwrap(),
            " ".repeat(MAX_BYTES - encode(&Document::Trip(trip())).unwrap().len())
        );
        assert_eq!(at_limit.len(), MAX_BYTES);
        assert!(decode(at_limit.as_bytes()).is_ok());
    }

    #[test]
    fn unknown_fields_in_a_known_version_are_ignored() {
        let mut v = encoded(&Document::Trip(trip()));
        v["future"] = json!({"a": 1});
        v["trip"]["rating"] = 5.into();
        v["trip"]["places"][0]["depth_m"] = 3.0.into();
        v["trip"]["limits"]["max_swell_m"] = 2.0.into();
        assert_eq!(decode(v.to_string().as_bytes()), Ok(Document::Trip(trip())));
    }

    #[test]
    fn reserved_kinds_decode_as_unsupported() {
        for kind in ["layers", "profile"] {
            let text =
                format!(r#"{{"format":"underbite","v":1,"kind":"{kind}","{kind}":{{"x":1}}}}"#);
            assert_eq!(
                decode(text.as_bytes()),
                Ok(Document::Unsupported { kind: kind.into() })
            );
        }
        assert_eq!(
            encode(&Document::Unsupported {
                kind: "layers".into()
            }),
            Err(ShareError::Unreadable)
        );
    }

    #[test]
    fn rounded_fuzz_rounds_to_a_hundredth_of_a_degree() {
        let mut doc = Document::Trip(trip());
        fuzz(&mut doc, Fuzz::Rounded1Km);
        let Document::Trip(t) = &doc else { panic!() };
        let p = &t.places[0];
        assert_eq!(
            (p.latitude, p.longitude, p.fuzz),
            (Some(40.70), Some(-74.02), Fuzz::Rounded1Km)
        );
        assert_eq!(p.name, "The Battery");
        let before = doc.clone();
        fuzz(&mut doc, Fuzz::Rounded1Km);
        assert_eq!(doc, before, "idempotent");
        assert_eq!(decode(encode(&doc).unwrap().as_bytes()), Ok(doc));
    }

    #[test]
    fn fuzz_is_the_same_every_time() {
        let a = {
            let mut d = spot();
            fuzz(&mut d, Fuzz::Rounded1Km);
            d
        };
        let b = {
            let mut d = spot();
            fuzz(&mut d, Fuzz::Rounded1Km);
            d
        };
        assert_eq!(encode(&a), encode(&b));
        // The Croton and a buoy-side point round the same way every time too.
        let mut croton = battery();
        croton.latitude = Some(41.2087);
        croton.longitude = Some(-73.8586);
        fuzz_place(&mut croton, Fuzz::Rounded1Km);
        assert_eq!(
            (croton.latitude, croton.longitude),
            (Some(41.21), Some(-73.86))
        );
    }

    #[test]
    fn no_negative_zero_after_rounding() {
        assert!(hundredth(-0.001).is_sign_positive());
        assert!((hundredth(-0.004)).abs() < f64::EPSILON);
    }

    #[test]
    fn area_only_drops_coordinates_and_keeps_area_and_region() {
        let mut doc = Document::Trip(trip());
        fuzz(&mut doc, Fuzz::AreaOnly);
        let Document::Trip(t) = &doc else { panic!() };
        let p = &t.places[0];
        assert_eq!((p.latitude, p.longitude), (None, None));
        assert_eq!(p.fuzz, Fuzz::AreaOnly);
        assert_eq!(p.area.as_deref(), Some("Lower Hudson"));
        assert_eq!(p.region.as_deref(), Some("US-NY"));
        let v = encoded(&doc);
        assert!(v["trip"]["places"][0].get("latitude").is_none());
        assert!(v["trip"]["places"][0].get("longitude").is_none());
        assert_eq!(v["trip"]["places"][0]["fuzz"], "area_only");
        assert_eq!(decode(encode(&doc).unwrap().as_bytes()), Ok(doc));
    }

    #[test]
    fn fuzz_never_refines_and_exact_changes_nothing() {
        let mut doc = spot();
        fuzz(&mut doc, Fuzz::Exact);
        assert_eq!(doc, spot());
        fuzz(&mut doc, Fuzz::AreaOnly);
        let area = doc.clone();
        fuzz(&mut doc, Fuzz::Rounded1Km);
        assert_eq!(doc, area);
        fuzz(&mut doc, Fuzz::Exact);
        assert_eq!(doc, area);
    }

    #[test]
    fn fuzz_reaches_every_place_and_the_spot_payload() {
        let mut t = trip();
        let mut croton = battery();
        croton.spot_id = "spot-croton".into();
        croton.latitude = Some(41.2087);
        croton.longitude = Some(-73.8586);
        t.places.push(croton);
        let mut doc = Document::Trip(t);
        fuzz(&mut doc, Fuzz::Rounded1Km);
        let Document::Trip(t) = &doc else { panic!() };
        assert!(t.places.iter().all(|p| p.fuzz == Fuzz::Rounded1Km));
        let mut s = spot();
        fuzz(&mut s, Fuzz::AreaOnly);
        let Document::Spot(s) = &s else { panic!() };
        assert_eq!(s.place.fuzz, Fuzz::AreaOnly);
        let mut u = Document::Unsupported {
            kind: "layers".into(),
        };
        fuzz(&mut u, Fuzz::AreaOnly);
    }

    #[test]
    fn the_file_holds_no_display_strings_or_user_units() {
        let text = encode(&Document::Trip(trip())).unwrap();
        for word in ["°F", "mph", "feet", "api_key", "token"] {
            assert!(!text.contains(word), "{word}");
        }
    }

    #[test]
    fn the_wire_shape_and_schemas_are_stable() {
        /// The envelope as a type, for the schema: exactly one payload member matches `kind`.
        #[derive(JsonSchema)]
        #[allow(dead_code)]
        struct File {
            format: String,
            v: u32,
            kind: Kind,
            trip: Option<Trip>,
            spot: Option<SpotShare>,
        }
        #[derive(JsonSchema)]
        #[serde(rename_all = "snake_case")]
        #[allow(dead_code)]
        enum Kind {
            Trip,
            Spot,
            Layers,
            Profile,
        }
        let mut t = trip();
        t.species
            .push(SpeciesId::new("pomatomus-saltatrix").unwrap());
        t.method = Method::Boat;
        let schemas = json!({
            "File": schemars::schema_for!(File),
            "Fuzz": schemars::schema_for!(Fuzz),
        });
        insta::assert_json_snapshot!("schema", schemas);
        insta::assert_snapshot!("trip_file", encode(&Document::Trip(t)).unwrap());
        insta::assert_snapshot!("spot_file", encode(&spot()).unwrap());
    }
}
