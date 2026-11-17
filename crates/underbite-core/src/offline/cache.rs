//! Which requests may be answered offline from the cache, and a spot's last conditions kept for it.
//!
//! Only long-lived station metadata is served offline: harmonic constants, datums, offsets, bins
//! and the station lists change rarely, so a copy weeks old is as good as a new one. Observations
//! and forecasts never are: an old water level served as current would be a lie, so those
//! requests are refused and the app shows the last snapshot with its age instead.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::model::SpotSnapshot;
use crate::provenance::StationRef;
use crate::time::UnixTime;

const DAY_S: i64 = 86_400;
const WEEK_S: i64 = 7 * DAY_S;

/// Decimal places spot coordinates keep in a stored bundle: about a kilometre.
pub const BUNDLE_COORDINATE_DECIMALS: u8 = 2;

const COOPS_METADATA: &str = "https://api.tidesandcurrents.noaa.gov/mdapi/prod/webapi/";
const NDBC_STATIONS: &str = "https://www.ndbc.noaa.gov/activestations.xml";

/// A kind of long-lived metadata that may answer offline.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetadataKind {
    /// CO-OPS `harcon.json`: harmonic constants of a tide station or a current bin.
    Harmonics,
    /// CO-OPS `datums.json`.
    Datums,
    /// CO-OPS `tidepredoffsets.json`.
    TideOffsets,
    /// CO-OPS `currentpredictionoffsets.json`.
    CurrentOffsets,
    /// CO-OPS `bins.json`.
    Bins,
    /// A CO-OPS station list (`stations.json`).
    CoopsStations,
    /// NDBC's active station list.
    NdbcStations,
}

impl MetadataKind {
    /// How long a copy may be served offline, seconds. Constants, datums and offsets are
    /// revised on epoch updates, years apart; the station lists gain and lose stations a few
    /// times a year.
    pub const fn max_age_s(self) -> i64 {
        match self {
            Self::Harmonics
            | Self::Datums
            | Self::TideOffsets
            | Self::CurrentOffsets
            | Self::Bins => 12 * WEEK_S,
            Self::CoopsStations | Self::NdbcStations => 6 * WEEK_S,
        }
    }

    /// The kind of metadata `url` asks for, or `None` for anything else (observations,
    /// predictions, forecasts and every other source).
    pub fn of_url(url: &str) -> Option<Self> {
        let path = url.split(['?', '#']).next().unwrap_or(url);
        if path == NDBC_STATIONS {
            return Some(Self::NdbcStations);
        }
        let rest = path.strip_prefix(COOPS_METADATA)?;
        if rest == "stations.json" {
            return Some(Self::CoopsStations);
        }
        let (station, file) = rest.strip_prefix("stations/")?.split_once('/')?;
        if station.is_empty() {
            return None;
        }
        match file {
            "harcon.json" => Some(Self::Harmonics),
            "datums.json" => Some(Self::Datums),
            "tidepredoffsets.json" => Some(Self::TideOffsets),
            "currentpredictionoffsets.json" => Some(Self::CurrentOffsets),
            "bins.json" => Some(Self::Bins),
            _ => None,
        }
    }
}

/// The key a response is cached under: the URL without its fragment and without any query
/// parameter that looks like a credential, so a key never reaches the cache.
pub fn cache_key(url: &str) -> String {
    let url = url.split('#').next().unwrap_or(url);
    let Some((path, query)) = url.split_once('?') else {
        return url.to_owned();
    };
    let kept: Vec<&str> = query
        .split('&')
        .filter(|pair| {
            let name = pair.split('=').next().unwrap_or("").to_ascii_lowercase();
            !["key", "token", "secret", "auth", "password", "signature"]
                .iter()
                .any(|word| name.contains(word))
        })
        .collect();
    if kept.is_empty() {
        path.to_owned()
    } else {
        format!("{path}?{}", kept.join("&"))
    }
}

/// A stored metadata response.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedMetadata {
    /// [`cache_key`] of the request URL.
    pub key: String,
    /// The body of a successful response.
    pub body: Vec<u8>,
    /// When it was fetched.
    pub fetched_at: UnixTime,
}

impl CachedMetadata {
    /// Stores `body`, fetched from `url` at `fetched_at`; `None` when `url` is not metadata
    /// worth keeping.
    pub fn new(url: &str, body: Vec<u8>, fetched_at: UnixTime) -> Option<Self> {
        MetadataKind::of_url(url)?;
        Some(Self {
            key: cache_key(url),
            body,
            fetched_at,
        })
    }
}

/// A cached response that may answer a request offline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Served<'a> {
    /// What the response is.
    pub kind: MetadataKind,
    /// The cached body.
    pub body: &'a [u8],
    /// When it was fetched, shown as the data's age.
    pub fetched_at: UnixTime,
    /// Seconds since, never negative.
    pub age_s: i64,
}

/// Why a request cannot be answered from the cache.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum NotServed {
    /// Not long-lived metadata: observations and forecasts are never served as current.
    #[error("not long-lived metadata")]
    NotMetadata,
    /// Nothing cached for this request.
    #[error("nothing cached")]
    Missing,
    /// Cached, but older than the kind's maximum age.
    #[error("cached copy is {age_s} s old, the limit is {max_age_s} s")]
    Expired {
        /// Seconds since it was fetched.
        age_s: i64,
        /// The most it may be.
        max_age_s: i64,
    },
}

/// Answers the request for `url` from `cached` when it is long-lived metadata of the same key
/// and not older than its maximum age at `now`.
pub fn serve_offline<'a>(
    url: &str,
    cached: Option<&'a CachedMetadata>,
    now: UnixTime,
) -> Result<Served<'a>, NotServed> {
    let kind = MetadataKind::of_url(url).ok_or(NotServed::NotMetadata)?;
    let hit = cached
        .filter(|c| c.key == cache_key(url))
        .ok_or(NotServed::Missing)?;
    let age_s = (now.0 - hit.fetched_at.0).max(0);
    if age_s > kind.max_age_s() {
        return Err(NotServed::Expired {
            age_s,
            max_age_s: kind.max_age_s(),
        });
    }
    Ok(Served {
        kind,
        body: &hit.body,
        fetched_at: hit.fetched_at,
        age_s,
    })
}

/// A spot's last snapshot, kept so the conditions page and the map answer without signal.
///
/// A bundle holds no keys and no exact coordinates: the spot's (and the weather location's)
/// coordinates are rounded to [`BUNDLE_COORDINATE_DECIMALS`] places when it is made. The spot
/// of a fresh fetch always replaces it in [`merge_last`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpotBundle {
    /// The spot's id.
    pub spot_id: String,
    /// The snapshot as last fetched.
    pub last_snapshot: SpotSnapshot,
    /// When it was fetched.
    pub fetched_at: UnixTime,
}

fn round_to(value: f64, decimals: u8) -> f64 {
    let scale = 10_f64.powi(i32::from(decimals));
    (value * scale).round() / scale
}

impl SpotBundle {
    /// A bundle of `snapshot` with the coordinates rounded.
    pub fn new(mut snapshot: SpotSnapshot) -> Self {
        let d = BUNDLE_COORDINATE_DECIMALS;
        snapshot.spot.latitude = round_to(snapshot.spot.latitude, d);
        snapshot.spot.longitude = round_to(snapshot.spot.longitude, d);
        if let Some(weather) = snapshot.weather.as_mut() {
            weather.location.latitude = round_to(weather.location.latitude, d);
            weather.location.longitude = round_to(weather.location.longitude, d);
        }
        Self {
            spot_id: snapshot.spot.id.clone(),
            fetched_at: snapshot.fetched_at,
            last_snapshot: snapshot,
        }
    }

    /// Seconds since the snapshot was fetched at `now`, never negative.
    pub fn age_s(&self, now: UnixTime) -> i64 {
        (now.0 - self.fetched_at.0).max(0)
    }
}

/// `fresh` with every measurement it lacks filled from `last`.
///
/// A filled value keeps its own series, so its original sample times and its provenance (the
/// station, the kind, `observed_at`) stay as they were: nothing is moved to now, and a last
/// water temperature reads as the old reading it is. Whatever `fresh` has wins, including
/// computed tide and current tables. The spot, `fetched_at` and errors are `fresh`'s. Stations
/// and credits of `last` that `fresh` lacks are kept, so every shown value stays credited.
/// Alerts and storm checks carry their own times and expiry for the view to judge.
pub fn merge_last(last: &SpotSnapshot, fresh: SpotSnapshot) -> SpotSnapshot {
    let mut out = fresh;
    macro_rules! fill {
        ($group:ident: $($field:ident),+ $(,)?) => {
            $(if out.$group.$field.is_none() {
                out.$group.$field.clone_from(&last.$group.$field);
            })+
        };
    }
    if out.weather.is_none() {
        out.weather.clone_from(&last.weather);
    }
    fill!(water: temperature_c, discharge_m3s, gage_height, level, turbidity,
        dissolved_oxygen_mg_l, conductivity_us_cm, salinity_psu, ph, chlorophyll_ug_l,
        clarity_kd490_per_m, cyanobacteria_cells_ml, discharge_forecast_m3s, stage_forecast,
        flood_categories, discharge_percentile_for_date, level_percentile_for_date);
    fill!(marine: tide, tidal_current, ocean_current, waves, sea_surface_temp_c,
        waves_forecast, sea_surface_temp_forecast_c, observed_wind, observed_pressure_hpa,
        marine_forecast);
    if out.marine.marine_alerts.is_empty() {
        out.marine
            .marine_alerts
            .clone_from(&last.marine.marine_alerts);
    }
    if out.sky.days.is_empty() {
        out.sky.days.clone_from(&last.sky.days);
    }
    if out.storm.is_none() {
        out.storm.clone_from(&last.storm);
    }
    let known = |stations: &[StationRef], s: &StationRef| {
        stations
            .iter()
            .any(|k| k.source == s.source && k.id == s.id && k.bin == s.bin)
    };
    for station in &last.stations {
        if !known(&out.stations, station) {
            out.stations.push(station.clone());
        }
    }
    for credit in &last.attribution {
        if !out.attribution.contains(credit) {
            out.attribution.push(credit.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::testdata::sample_snapshot as snapshot;
    use crate::series::{Sample, Series};

    const HARCON: &str = "https://api.tidesandcurrents.noaa.gov/mdapi/prod/webapi/stations/8518750/harcon.json?units=metric";
    const WATER_LEVEL: &str = "https://api.tidesandcurrents.noaa.gov/api/prod/datagetter?product=water_level&station=8518750&begin_date=20261108&range=24&datum=MLLW&units=metric&time_zone=gmt&format=json";

    #[test]
    fn a_harcon_request_is_served_offline_and_a_water_level_is_not() {
        let t = UnixTime(1_794_000_000);
        let stored = CachedMetadata::new(HARCON, b"{}".to_vec(), t).unwrap();
        let day = UnixTime(t.0 + DAY_S);
        let served = serve_offline(HARCON, Some(&stored), day).unwrap();
        assert_eq!(served.kind, MetadataKind::Harmonics);
        assert_eq!(
            (served.body, served.age_s, served.fetched_at),
            (&b"{}"[..], DAY_S, t)
        );

        assert_eq!(
            serve_offline(WATER_LEVEL, Some(&stored), day),
            Err(NotServed::NotMetadata)
        );
        assert!(CachedMetadata::new(WATER_LEVEL, Vec::new(), t).is_none());
        // A different station is another key.
        let other = HARCON.replace("8518750", "8531680");
        assert_eq!(
            serve_offline(&other, Some(&stored), day),
            Err(NotServed::Missing)
        );
        assert_eq!(serve_offline(HARCON, None, day), Err(NotServed::Missing));
    }

    #[test]
    fn metadata_expires_after_its_limit() {
        let t = UnixTime(1_794_000_000);
        let stored = CachedMetadata::new(HARCON, vec![1], t).unwrap();
        let limit = MetadataKind::Harmonics.max_age_s();
        assert!(serve_offline(HARCON, Some(&stored), UnixTime(t.0 + limit)).is_ok());
        assert_eq!(
            serve_offline(HARCON, Some(&stored), UnixTime(t.0 + limit + 1)),
            Err(NotServed::Expired {
                age_s: limit + 1,
                max_age_s: limit
            })
        );
        // A clock behind the fetch time is an age of zero.
        let early = serve_offline(HARCON, Some(&stored), UnixTime(t.0 - 5)).unwrap();
        assert_eq!(early.age_s, 0);
    }

    #[test]
    fn every_listed_metadata_url_is_classified() {
        let base = "https://api.tidesandcurrents.noaa.gov/mdapi/prod/webapi";
        let cases = [
            (
                format!("{base}/stations/8518750/datums.json?units=metric"),
                MetadataKind::Datums,
            ),
            (
                format!("{base}/stations/8518750/harcon.json?units=metric"),
                MetadataKind::Harmonics,
            ),
            (
                format!("{base}/stations/NYH1927/harcon.json?bin=13"),
                MetadataKind::Harmonics,
            ),
            (
                format!("{base}/stations/8518919/tidepredoffsets.json"),
                MetadataKind::TideOffsets,
            ),
            (
                format!("{base}/stations/ACT3646_1/currentpredictionoffsets.json"),
                MetadataKind::CurrentOffsets,
            ),
            (
                format!("{base}/stations/NYH1927/bins.json?units=metric"),
                MetadataKind::Bins,
            ),
            (
                format!("{base}/stations.json?type=tidepredictions"),
                MetadataKind::CoopsStations,
            ),
            (
                "https://www.ndbc.noaa.gov/activestations.xml".into(),
                MetadataKind::NdbcStations,
            ),
        ];
        for (url, kind) in cases {
            assert_eq!(MetadataKind::of_url(&url), Some(kind), "{url}");
        }
        for url in [
            format!("{base}/stations/8518750.json"),
            format!("{base}/stations/8518750/notices.json"),
            "https://api.tidesandcurrents.noaa.gov/api/prod/datagetter?product=predictions".into(),
            "https://www.ndbc.noaa.gov/data/realtime2/44025.txt".into(),
            "https://example.org/mdapi/prod/webapi/stations/1/harcon.json".into(),
        ] {
            assert_eq!(MetadataKind::of_url(&url), None, "{url}");
        }
    }

    #[test]
    fn keys_never_enter_the_cache_key() {
        assert_eq!(
            cache_key("https://x.example/a.json?units=metric&api_key=SECRET&token=T#frag"),
            "https://x.example/a.json?units=metric"
        );
        assert_eq!(
            cache_key("https://x.example/a.json?key=1"),
            "https://x.example/a.json"
        );
    }

    fn series(time: i64, value: f32) -> Series<f32> {
        let provenance = snapshot().water.temperature_c.unwrap().provenance().clone();
        Series::new(provenance, vec![Sample::new(UnixTime(time), value)])
    }

    #[test]
    fn merge_keeps_the_last_water_temperature_with_its_original_time() {
        let mut last = snapshot();
        let old = series(last.fetched_at.0 - 3 * 3600, 11.5);
        last.water.temperature_c = Some(old.clone());
        let fresh = SpotSnapshot::new(last.spot.clone(), UnixTime(last.fetched_at.0 + 86_400));
        let merged = merge_last(&last, fresh.clone());
        assert_eq!(merged.water.temperature_c, Some(old.clone()));
        assert_eq!(merged.fetched_at, fresh.fetched_at);
        let kept = merged.water.temperature_c.unwrap();
        assert_eq!(kept.samples()[0].time.0, last.fetched_at.0 - 3 * 3600);
        assert_eq!(kept.provenance(), old.provenance());
    }

    #[test]
    fn merge_prefers_fresh_values_and_unions_stations_and_credits() {
        let last = snapshot();
        let mut fresh = SpotSnapshot::new(last.spot.clone(), UnixTime(last.fetched_at.0 + 600));
        let new = series(fresh.fetched_at.0, 14.0);
        fresh.water.temperature_c = Some(new.clone());
        let merged = merge_last(&last, fresh);
        assert_eq!(merged.water.temperature_c, Some(new));
        assert_eq!(merged.marine, last.marine);
        assert_eq!(merged.weather, last.weather);
        assert_eq!(merged.stations, last.stations);
        assert_eq!(merged.attribution, last.attribution);
        assert_eq!(merged.errors.len(), 0);
    }

    #[test]
    fn bundles_round_the_spot_and_report_age() {
        let mut s = snapshot();
        s.spot.latitude = 40.703_312;
        s.spot.longitude = -74.017_044;
        let bundle = SpotBundle::new(s.clone());
        assert_eq!(bundle.spot_id, s.spot.id);
        assert_eq!(bundle.fetched_at, s.fetched_at);
        assert_eq!(bundle.last_snapshot.spot.latitude, 40.70);
        assert_eq!(bundle.last_snapshot.spot.longitude, -74.02);
        assert_eq!(bundle.age_s(UnixTime(s.fetched_at.0 + 90)), 90);
        assert_eq!(bundle.age_s(UnixTime(s.fetched_at.0 - 90)), 0);
        let json = serde_json::to_string(&bundle).unwrap();
        assert!(!json.contains("40.7033"));
        assert_eq!(serde_json::from_str::<SpotBundle>(&json).unwrap(), bundle);
    }
}
