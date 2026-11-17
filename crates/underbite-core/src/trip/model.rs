//! The trip model: `Trip`, its places, method, species and checklist.
//!
//! These are the types a `.underbite` file carries (see [`crate::share`]). Everything is in
//! canonical units and calendar dates; the file holds no display strings and no user units, since
//! the person who opens it may use others. Ids and `modified_at` come from the host: the core reads
//! neither the clock nor a random source.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::model::spot::WaterType;
use crate::time::{self, LocalDate, UnixTime};

/// The most days a trip may cover, first and last day included.
pub const MAX_TRIP_DAYS: i64 = 31;

/// How a person fishes; the limits and the gear list follow it.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Method {
    Boat,
    Kayak,
    Shore,
    Wade,
    Ice,
}

/// A species, by its lower-case scientific name with hyphens: `morone-saxatilis`.
///
/// The species catalogue keys its profiles by this id. Only lower-case ASCII letters and digits
/// in words joined by single hyphens are accepted.
#[derive(
    Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(try_from = "String", into = "String")]
#[schemars(with = "String")]
pub struct SpeciesId(String);

/// A string that is not a [`SpeciesId`].
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0:?} is not a species id such as \"morone-saxatilis\"")]
pub struct SpeciesIdError(String);

impl SpeciesId {
    /// The id for `s`.
    ///
    /// # Errors
    /// [`SpeciesIdError`] unless `s` is lower-case words of ASCII letters and digits joined by
    /// single hyphens.
    pub fn new(s: &str) -> Result<Self, SpeciesIdError> {
        let ok = !s.is_empty()
            && s.split('-').all(|w| {
                !w.is_empty()
                    && w.bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
            });
        if ok {
            Ok(Self(s.to_owned()))
        } else {
            Err(SpeciesIdError(s.to_owned()))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for SpeciesId {
    type Error = SpeciesIdError;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::new(&s)
    }
}

impl From<SpeciesId> for String {
    fn from(id: SpeciesId) -> Self {
        id.0
    }
}

/// How much of a place's location a shared file reveals. Ordered from most to least.
///
/// The level a place carries is the level it was fuzzed to, so the receiver can say "approximate
/// location".
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Fuzz {
    /// The coordinates as saved.
    #[default]
    Exact,
    /// Rounded to 0.01 degrees, about a kilometre.
    Rounded1Km,
    /// No coordinates; the area and region only.
    AreaOnly,
}

/// One place of a trip.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TripPlace {
    /// The host's id for the saved spot.
    pub spot_id: String,
    pub name: String,
    /// The area the spot is grouped under, for example "Lower Hudson".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub area: Option<String>,
    /// Degrees north; absent at [`Fuzz::AreaOnly`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latitude: Option<f64>,
    /// Degrees east; absent at [`Fuzz::AreaOnly`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub longitude: Option<f64>,
    /// IANA zone name, for example `America/New_York`.
    pub time_zone: String,
    pub water_type: WaterType,
    /// ISO 3166-2 subdivision, for example `US-NY`; the app fills it, the core never geocodes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    /// What the coordinates were reduced to before sharing.
    #[serde(default)]
    pub fuzz: Fuzz,
}

/// Why a place cannot be used.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PlaceProblem {
    /// A coordinate is out of range or not finite, only one is present, or the coordinates
    /// contradict the fuzz level (missing unless `AreaOnly`, present with it).
    #[error("coordinates are missing, out of range or contradict the fuzz level")]
    Coordinates,
    /// The time zone is not in the IANA database.
    #[error("unknown time zone")]
    TimeZone,
    /// The region is not an ISO 3166-2 code such as `US-NY`.
    #[error("region is not an ISO 3166-2 code")]
    Region,
}

impl TripPlace {
    /// Checks coordinates, zone and region.
    ///
    /// # Errors
    /// The first [`PlaceProblem`] found.
    pub fn validate(&self) -> Result<(), PlaceProblem> {
        match (self.latitude, self.longitude, self.fuzz) {
            (None, None, Fuzz::AreaOnly) => {}
            (Some(lat), Some(lon), Fuzz::Exact | Fuzz::Rounded1Km)
                if lat.is_finite()
                    && lon.is_finite()
                    && (-90.0..=90.0).contains(&lat)
                    && (-180.0..=180.0).contains(&lon) => {}
            _ => return Err(PlaceProblem::Coordinates),
        }
        time::zone(&self.time_zone).map_err(|_| PlaceProblem::TimeZone)?;
        if self.region.as_deref().is_some_and(|r| !is_iso_3166_2(r)) {
            return Err(PlaceProblem::Region);
        }
        Ok(())
    }
}

/// Two upper-case letters, a hyphen, one to three upper-case letters or digits.
fn is_iso_3166_2(s: &str) -> bool {
    let Some((country, sub)) = s.split_once('-') else {
        return false;
    };
    country.len() == 2
        && country.bytes().all(|b| b.is_ascii_uppercase())
        && (1..=3).contains(&sub.len())
        && sub
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
}

/// A copy of the angler's limits, so whoever opens a trip sees the same numbers.
///
/// It mirrors the configuration's limits profile field for field, in canonical units (knots for
/// wind and current, metres, °C, kilometres) whatever the sender displays.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TripLimits {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_wind_kn: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_gust_kn: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_wave_m: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_current_kn: Option<f64>,
    /// Coldest air temperature, °C.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_air_temp_c: Option<f64>,
    /// Least visibility, km.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_visibility_km: Option<f64>,
    #[serde(default)]
    pub no_small_craft_advisory: bool,
    #[serde(default)]
    pub no_lightning: bool,
}

/// One line of a trip's checklist.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ChecklistItem {
    /// The host's id, stable across edits so merges can match items.
    pub id: String,
    pub text: String,
    #[serde(default)]
    pub done: bool,
}

/// A trip: places, calendar days, species and method.
///
/// Never holds keys, the catch log or photos.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Trip {
    /// The host's id (a UUID in the app).
    pub id: String,
    pub name: String,
    pub places: Vec<TripPlace>,
    /// First day, a calendar date written `2027-07-12`.
    #[serde(with = "date_string")]
    #[schemars(with = "String")]
    pub first: LocalDate,
    /// Last day, inclusive.
    #[serde(with = "date_string")]
    #[schemars(with = "String")]
    pub last: LocalDate,
    #[serde(default)]
    pub species: Vec<SpeciesId>,
    pub method: Method,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limits: Option<TripLimits>,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub checklist: Vec<ChecklistItem>,
    /// When the host last changed the trip, seconds since the Unix epoch.
    pub modified_at: UnixTime,
}

/// Why a trip cannot be used.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TripError {
    /// The last day is before the first.
    #[error("the last day is before the first")]
    DatesBackwards,
    /// The trip covers more than [`MAX_TRIP_DAYS`] days.
    #[error("the trip is longer than {MAX_TRIP_DAYS} days")]
    TooLong,
    /// A trip needs a place.
    #[error("the trip has no place")]
    NoPlaces,
    /// A place cannot be used.
    #[error("place {index}: {problem}")]
    Place { index: usize, problem: PlaceProblem },
}

impl Trip {
    /// Checks the dates (in order, at most [`MAX_TRIP_DAYS`]) and every place.
    ///
    /// # Errors
    /// The first [`TripError`] found.
    pub fn validate(&self) -> Result<(), TripError> {
        let days = time::day_number(self.last) - time::day_number(self.first) + 1;
        if days < 1 {
            return Err(TripError::DatesBackwards);
        }
        if days > MAX_TRIP_DAYS {
            return Err(TripError::TooLong);
        }
        if self.places.is_empty() {
            return Err(TripError::NoPlaces);
        }
        for (index, place) in self.places.iter().enumerate() {
            place
                .validate()
                .map_err(|problem| TripError::Place { index, problem })?;
        }
        Ok(())
    }
}

/// Dates on the wire as `YYYY-MM-DD`.
pub(crate) mod date_string {
    use serde::{Deserialize, Deserializer, Serializer};

    use crate::time::{LocalDate, parse_date};

    // serde's `with` calls this with a reference.
    #[allow(clippy::trivially_copy_pass_by_ref)]
    pub(crate) fn serialize<S: Serializer>(date: &LocalDate, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(&format_args!(
            "{:04}-{:02}-{:02}",
            date.year, date.month, date.day
        ))
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<LocalDate, D::Error> {
        let s = String::deserialize(d)?;
        parse_date(&s).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn date(year: i32, month: u8, day: u8) -> LocalDate {
        LocalDate { year, month, day }
    }

    pub(crate) fn battery() -> TripPlace {
        TripPlace {
            spot_id: "spot-battery".into(),
            name: "The Battery".into(),
            area: Some("Lower Hudson".into()),
            latitude: Some(40.7033),
            longitude: Some(-74.0170),
            time_zone: "America/New_York".into(),
            water_type: WaterType::TidalRiverEstuary,
            region: Some("US-NY".into()),
            fuzz: Fuzz::Exact,
        }
    }

    pub(crate) fn trip() -> Trip {
        Trip {
            id: "trip-1".into(),
            name: "Hudson weekend".into(),
            places: vec![battery()],
            first: date(2027, 7, 12),
            last: date(2027, 7, 14),
            species: vec![SpeciesId::new("morone-saxatilis").unwrap()],
            method: Method::Kayak,
            limits: Some(TripLimits {
                name: "Kayak".into(),
                max_wind_kn: Some(12.0),
                max_gust_kn: Some(18.0),
                max_wave_m: Some(0.6),
                no_small_craft_advisory: true,
                no_lightning: true,
                ..TripLimits::default()
            }),
            notes: "Launch at first light.".into(),
            checklist: vec![ChecklistItem {
                id: "c1".into(),
                text: "Licence".into(),
                done: false,
            }],
            modified_at: UnixTime(1_783_000_000),
        }
    }

    #[test]
    fn json_shape() {
        let v = serde_json::to_value(trip()).unwrap();
        assert_eq!(v["first"], "2027-07-12");
        assert_eq!(v["method"], "kayak");
        assert_eq!(v["species"][0], "morone-saxatilis");
        assert_eq!(v["places"][0]["fuzz"], "exact");
        assert_eq!(v["modified_at"], 1_783_000_000);
        let back: Trip = serde_json::from_value(v).unwrap();
        assert_eq!(back, trip());
    }

    #[test]
    fn species_ids() {
        for ok in [
            "morone-saxatilis",
            "salvelinus-fontinalis",
            "esox-lucius",
            "x1",
        ] {
            assert_eq!(SpeciesId::new(ok).unwrap().as_str(), ok);
        }
        for bad in [
            "",
            "Morone-saxatilis",
            "morone_saxatilis",
            "-a",
            "a-",
            "a--b",
            "a b",
            "é",
        ] {
            assert!(SpeciesId::new(bad).is_err(), "{bad}");
        }
        assert!(serde_json::from_str::<SpeciesId>("\"Striped Bass\"").is_err());
    }

    #[test]
    fn dates_must_be_real_calendar_days() {
        let mut v = serde_json::to_value(trip()).unwrap();
        v["last"] = "2027-02-30".into();
        assert!(serde_json::from_value::<Trip>(v.clone()).is_err());
        v["last"] = "2027-07-12T00:00:00Z".into();
        assert!(serde_json::from_value::<Trip>(v).is_err());
    }

    #[test]
    fn a_valid_trip_validates_and_a_one_day_trip_too() {
        assert_eq!(trip().validate(), Ok(()));
        let mut t = trip();
        t.last = t.first;
        assert_eq!(t.validate(), Ok(()));
    }

    #[test]
    fn dates_backwards_and_too_long() {
        let mut t = trip();
        t.last = date(2027, 7, 11);
        assert_eq!(t.validate(), Err(TripError::DatesBackwards));
        t.first = date(2027, 7, 1);
        t.last = date(2027, 7, 31);
        assert_eq!(t.validate(), Ok(()), "31 days");
        t.last = date(2027, 8, 1);
        assert_eq!(t.validate(), Err(TripError::TooLong), "32 days");
    }

    #[test]
    fn places_are_checked() {
        let mut t = trip();
        t.places.clear();
        assert_eq!(t.validate(), Err(TripError::NoPlaces));
        let bad = |f: fn(&mut TripPlace), problem| {
            let mut t = trip();
            t.places.push(battery());
            f(&mut t.places[1]);
            assert_eq!(t.validate(), Err(TripError::Place { index: 1, problem }));
        };
        bad(|p| p.latitude = Some(91.0), PlaceProblem::Coordinates);
        bad(|p| p.longitude = Some(f64::NAN), PlaceProblem::Coordinates);
        bad(|p| p.longitude = None, PlaceProblem::Coordinates);
        bad(|p| p.fuzz = Fuzz::AreaOnly, PlaceProblem::Coordinates);
        bad(|p| p.latitude = None, PlaceProblem::Coordinates);
        bad(
            |p| p.time_zone = "Mars/Olympus".into(),
            PlaceProblem::TimeZone,
        );
        bad(|p| p.region = Some("ny".into()), PlaceProblem::Region);
        bad(|p| p.region = Some("US-NEWY".into()), PlaceProblem::Region);
        let mut area = battery();
        area.fuzz = Fuzz::AreaOnly;
        area.latitude = None;
        area.longitude = None;
        assert_eq!(area.validate(), Ok(()));
        area.region = None;
        assert_eq!(area.validate(), Ok(()));
    }

    #[test]
    fn unknown_fields_are_ignored_and_optionals_default() {
        let json = r#"{"id":"t","name":"n","first":"2027-07-12","last":"2027-07-12",
            "method":"shore","modified_at":0,"future":1,
            "places":[{"spot_id":"s","name":"The Battery","time_zone":"America/New_York",
              "water_type":"surf","latitude":40.7,"longitude":-74.0,"extra":true}]}"#;
        let t: Trip = serde_json::from_str(json).unwrap();
        assert_eq!(t.places[0].fuzz, Fuzz::Exact);
        assert!(t.species.is_empty() && t.notes.is_empty() && t.limits.is_none());
        assert_eq!(t.validate(), Ok(()));
    }

    #[test]
    fn fuzz_levels_order_from_most_to_least_revealing() {
        assert!(Fuzz::Exact < Fuzz::Rounded1Km && Fuzz::Rounded1Km < Fuzz::AreaOnly);
    }
}
