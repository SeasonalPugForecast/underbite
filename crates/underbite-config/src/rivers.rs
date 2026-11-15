//! The user's own flow ranges for a river gauge.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use underbite_core::units::DischargeUnit;
use underbite_core::view::RiverRanges;

/// One `[[rivers]]` entry; discharge in cubic feet per second, as USGS reports it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RiverRange {
    /// The gauge as `usgs:<site number>`, for example `usgs:01420500`.
    pub site: String,
    /// Wadeable below this discharge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wadeable_below_cfs: Option<f64>,
    /// The discharge range the user likes best: `[low, high]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub best_cfs: Option<[f64; 2]>,
    /// Blown out above this discharge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blown_out_above_cfs: Option<f64>,
}

impl RiverRange {
    /// The ranges in the canonical unit the view takes.
    pub fn to_ranges(&self) -> RiverRanges {
        let m3s = |cfs: f64| DischargeUnit::Cfs.to_canonical(cfs as f32);
        RiverRanges {
            wadeable_below_m3s: self.wadeable_below_cfs.map(m3s),
            best_min_m3s: self.best_cfs.map(|b| m3s(b[0])),
            best_max_m3s: self.best_cfs.map(|b| m3s(b[1])),
            blown_out_above_m3s: self.blown_out_above_cfs.map(m3s),
        }
    }

    /// The entry for a gauge written `usgs:<number>`, if the user set one.
    pub fn find<'a>(rivers: &'a [Self], site: &str) -> Option<&'a Self> {
        rivers.iter().find(|r| r.site == site)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_cfs_to_cubic_metres() {
        let r = RiverRange {
            site: "usgs:01420500".into(),
            wadeable_below_cfs: Some(1200.0),
            best_cfs: Some([400.0, 900.0]),
            blown_out_above_cfs: None,
        };
        let v = r.to_ranges();
        assert!((v.wadeable_below_m3s.unwrap() - 33.98).abs() < 0.01);
        assert!((v.best_max_m3s.unwrap() - 25.485).abs() < 0.01);
        assert!(v.blown_out_above_m3s.is_none());
    }
}
