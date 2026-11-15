//! Time series of values, each with its provenance.
//!
//! Values are in the canonical units of the quantity they carry (metres, m/s, m³/s, °C, hPa,
//! FNU, PSU) and times are UTC; nothing here converts.

use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};

use crate::provenance::{Provenance, Quality};
use crate::time::UnixTime;

/// One value at one instant.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Sample<T> {
    /// When the value applies, UTC.
    pub time: UnixTime,
    /// The value, in canonical units.
    pub value: T,
    /// Per-sample quality, overriding the series provenance (USGS qualifiers vary per value).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality: Option<Quality>,
}

impl<T> Sample<T> {
    /// A sample with no per-sample quality.
    pub fn new(time: UnixTime, value: T) -> Self {
        Self {
            time,
            value,
            quality: None,
        }
    }
}

/// A flow or drift: speed and the direction the water moves **toward**.
///
/// The "toward" convention is that of CO-OPS `d` and `meanFloodDir` and Open-Meteo
/// `ocean_current_direction`. Wind and waves use "from" directions and are separate types.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Vector {
    /// Speed, m/s.
    pub speed_ms: f32,
    /// Direction the water flows toward, degrees clockwise from true north.
    pub toward_deg: f32,
}

/// Samples of one quantity from one provenance.
///
/// Samples are sorted ascending by time with no duplicate times; [`Series::new`] and
/// deserialization enforce this.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct Series<T> {
    provenance: Provenance,
    samples: Vec<Sample<T>>,
}

#[derive(Deserialize)]
#[serde(bound(deserialize = "T: Deserialize<'de>"))]
struct RawSeries<T> {
    provenance: Provenance,
    samples: Vec<Sample<T>>,
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Series<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = RawSeries::<T>::deserialize(deserializer)?;
        Ok(Series::new(raw.provenance, raw.samples))
    }
}

impl<T> Series<T> {
    /// Sorts the samples by time (stable) and drops later samples with a repeated time.
    pub fn new(provenance: Provenance, mut samples: Vec<Sample<T>>) -> Self {
        samples.sort_by_key(|s| s.time);
        samples.dedup_by_key(|s| s.time);
        Self {
            provenance,
            samples,
        }
    }

    /// Where the samples came from.
    pub fn provenance(&self) -> &Provenance {
        &self.provenance
    }

    /// The samples, ascending by time.
    pub fn samples(&self) -> &[Sample<T>] {
        &self.samples
    }

    /// True when there are no samples.
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// The most recent sample.
    pub fn latest(&self) -> Option<&Sample<T>> {
        self.samples.last()
    }

    /// Samples with `from <= time <= to`.
    pub fn window(&self, from: UnixTime, to: UnixTime) -> &[Sample<T>] {
        let start = self.samples.partition_point(|s| s.time < from);
        let end = self.samples.partition_point(|s| s.time <= to);
        if start >= end {
            &[]
        } else {
            &self.samples[start..end]
        }
    }

    /// Seconds from the latest sample to `now`, never negative; `None` for an empty series.
    pub fn age(&self, now: UnixTime) -> Option<i64> {
        self.latest().map(|s| now.0.saturating_sub(s.time.0).max(0))
    }
}

impl Series<f32> {
    /// The value at `t`, linearly interpolated between the neighbouring samples.
    ///
    /// `None` outside the range of samples, or when the neighbours are further apart than
    /// `max_gap`. An exact sample time returns that sample's value.
    pub fn at(&self, t: UnixTime, max_gap: Duration) -> Option<f32> {
        let i = self.samples.partition_point(|s| s.time <= t);
        let before = self.samples.get(i.checked_sub(1)?)?;
        if before.time == t {
            return Some(before.value);
        }
        let after = self.samples.get(i)?;
        let span = after.time.0 - before.time.0;
        if u64::try_from(span).ok()? > max_gap.as_secs() {
            return None;
        }
        let frac = (t.0 - before.time.0) as f64 / span as f64;
        let (a, b) = (f64::from(before.value), f64::from(after.value));
        Some((a + (b - a) * frac) as f32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provenance::{Relation, SourceId, ValueKind};

    fn prov() -> Provenance {
        Provenance {
            source: SourceId::COOPS,
            station: None,
            distance_m: None,
            relation: Relation::SameWaterBody,
            observed_at: UnixTime(0),
            kind: ValueKind::Observed,
            quality: Quality::Approved,
        }
    }

    fn series(points: &[(i64, f32)]) -> Series<f32> {
        Series::new(
            prov(),
            points
                .iter()
                .map(|&(t, v)| Sample::new(UnixTime(t), v))
                .collect(),
        )
    }

    #[test]
    fn missing_quality_is_left_out_and_null_still_decodes() {
        let s = Sample::new(UnixTime(5), 1.5_f32);
        assert_eq!(
            serde_json::to_string(&s).unwrap(),
            r#"{"time":5,"value":1.5}"#
        );
        let old: Sample<f32> =
            serde_json::from_str(r#"{"time":5,"value":1.5,"quality":null}"#).unwrap();
        assert_eq!(old, s);
        let json = serde_json::to_string(&prov()).unwrap();
        assert!(
            !json.contains("station") && !json.contains("distance_m"),
            "{json}"
        );
    }

    #[test]
    fn constructor_sorts_and_drops_later_duplicates() {
        let s = series(&[(300, 3.0), (100, 1.0), (200, 2.0), (100, 9.0)]);
        let times: Vec<i64> = s.samples().iter().map(|x| x.time.0).collect();
        assert_eq!(times, [100, 200, 300]);
        assert_eq!(s.samples()[0].value, 1.0);
    }

    #[test]
    fn deserialization_enforces_order() {
        let s = series(&[(1, 1.0), (2, 2.0)]);
        let mut v = serde_json::to_value(&s).unwrap();
        v["samples"].as_array_mut().unwrap().reverse();
        let back: Series<f32> = serde_json::from_value(v).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn latest_and_age() {
        let s = series(&[(100, 1.0), (200, 2.0)]);
        assert_eq!(s.latest().unwrap().value, 2.0);
        assert_eq!(s.age(UnixTime(260)), Some(60));
        assert_eq!(s.age(UnixTime(150)), Some(0));
        assert_eq!(series(&[]).age(UnixTime(1)), None);
    }

    #[test]
    fn window_is_inclusive() {
        let s = series(&[(100, 1.0), (200, 2.0), (300, 3.0)]);
        assert_eq!(s.window(UnixTime(100), UnixTime(200)).len(), 2);
        assert_eq!(s.window(UnixTime(101), UnixTime(199)).len(), 0);
        assert_eq!(s.window(UnixTime(300), UnixTime(100)).len(), 0);
    }

    #[test]
    fn at_interpolates_and_respects_range_and_gap() {
        let s = series(&[(0, 0.0), (600, 6.0), (6000, 60.0)]);
        let gap = Duration::from_mins(15);
        assert_eq!(s.at(UnixTime(300), gap), Some(3.0));
        assert_eq!(s.at(UnixTime(600), gap), Some(6.0));
        assert_eq!(s.at(UnixTime(-1), gap), None);
        assert_eq!(s.at(UnixTime(6001), gap), None);
        assert_eq!(s.at(UnixTime(3000), gap), None);
        assert_eq!(s.at(UnixTime(3000), Duration::from_mins(90)), Some(30.0));
    }

    #[test]
    fn vector_round_trips() {
        let v = Vector {
            speed_ms: 0.5,
            toward_deg: 190.0,
        };
        let json = serde_json::to_string(&v).unwrap();
        assert_eq!(serde_json::from_str::<Vector>(&json).unwrap(), v);
    }

    #[test]
    fn schema_snapshot() {
        let schemas = serde_json::json!({
            "Series_f32": schemars::schema_for!(Series<f32>),
            "Series_Vector": schemars::schema_for!(Series<Vector>),
            "Sample_f32": schemars::schema_for!(Sample<f32>),
            "Vector": schemars::schema_for!(Vector),
        });
        insta::assert_json_snapshot!(schemas);
    }
}
