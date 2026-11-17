//! Gridded values (ERDDAP and GLERL grids): blocks with ascending axes.

use crate::map::GridSource;
use crate::time::UnixTime;

/// A grid could not be built.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GridError {
    /// An axis has no points.
    #[error("the grid has an empty axis")]
    EmptyAxis,
    /// An axis holds a value that is not a number, or is not strictly monotonic.
    #[error("the {axis} axis is not strictly monotonic and finite")]
    BadAxis { axis: &'static str },
    /// `values` does not hold one value per cell.
    #[error("{found} values for {lats} latitudes by {lons} longitudes")]
    ShapeMismatch {
        lats: usize,
        lons: usize,
        found: usize,
    },
}

/// A block of values at one time over a latitude and longitude axis.
///
/// `values` is row-major: the value at latitude index `i` and longitude index `j` is
/// `values[i * lons.len() + j]`. A missing value (land, cloud) is NaN. `lats` and `lons` are both
/// ascending. The axes need not be evenly spaced: a Web Mercator row grid is a list of latitudes.
/// Build a grid with [`Grid::new`], which keeps those promises; the fields are public for reading.
#[derive(Clone, Debug, PartialEq)]
pub struct Grid {
    pub source: GridSource,
    /// The instant the values describe.
    pub time: UnixTime,
    /// Degrees north, ascending.
    pub lats: Vec<f64>,
    /// Degrees east, ascending.
    pub lons: Vec<f64>,
    /// NaN where a value is missing.
    pub values: Vec<f32>,
    /// The values' unit as the source states it (`degree_C`, `m s-1`, `mg m-3`).
    pub unit: String,
}

fn strictly_ascending(axis: &[f64]) -> bool {
    axis.iter().all(|v| v.is_finite()) && axis.windows(2).all(|w| w[0] < w[1])
}

impl Grid {
    /// Builds a grid from rows as the source lists them. A descending latitude axis (the VIIRS
    /// datasets list the northernmost row first) is reversed together with its rows so that
    /// `lats` ascends.
    ///
    /// # Errors
    /// [`GridError`] for an empty or non-monotonic axis, a longitude axis that descends, or a
    /// `values` length that is not `lats.len() * lons.len()`.
    pub fn new(
        source: GridSource,
        time: UnixTime,
        mut lats: Vec<f64>,
        lons: Vec<f64>,
        mut values: Vec<f32>,
        unit: impl Into<String>,
    ) -> Result<Self, GridError> {
        if lats.is_empty() || lons.is_empty() {
            return Err(GridError::EmptyAxis);
        }
        if !strictly_ascending(&lons) {
            return Err(GridError::BadAxis { axis: "longitude" });
        }
        if values.len() != lats.len() * lons.len() {
            return Err(GridError::ShapeMismatch {
                lats: lats.len(),
                lons: lons.len(),
                found: values.len(),
            });
        }
        if !strictly_ascending(&lats) {
            lats.reverse();
            if !strictly_ascending(&lats) {
                return Err(GridError::BadAxis { axis: "latitude" });
            }
            values = values
                .chunks_exact(lons.len())
                .rev()
                .flatten()
                .copied()
                .collect();
        }
        Ok(Self {
            source,
            time,
            lats,
            lons,
            values,
            unit: unit.into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::ErddapNode;

    fn source() -> GridSource {
        GridSource {
            node: ErddapNode::Coastwatch,
            dataset: "noaacwN20VIIRSchlaDaily".to_owned(),
            variables: vec!["chlor_a".to_owned()],
            altitude_axis: true,
            native_step_deg: 0.0375,
        }
    }

    fn build(lats: Vec<f64>, lons: Vec<f64>, values: Vec<f32>) -> Result<Grid, GridError> {
        Grid::new(
            source(),
            UnixTime(1_794_000_000),
            lats,
            lons,
            values,
            "mg m-3",
        )
    }

    #[test]
    fn ascending_latitudes_are_kept() {
        let g = build(
            vec![40.0, 40.1],
            vec![-74.0, -73.9, -73.8],
            vec![1., 2., 3., 4., 5., 6.],
        )
        .unwrap();
        assert_eq!(g.lats, [40.0, 40.1]);
        assert_eq!(g.values, [1., 2., 3., 4., 5., 6.]);
    }

    #[test]
    fn descending_latitudes_are_reversed_with_their_rows() {
        let g = build(
            vec![40.2, 40.1, 40.0],
            vec![-74.0, -73.9],
            vec![1., 2., 3., f32::NAN, 5., 6.],
        )
        .unwrap();
        assert_eq!(g.lats, [40.0, 40.1, 40.2]);
        assert_eq!(g.lons, [-74.0, -73.9]);
        assert_eq!(g.values[..2], [5., 6.]);
        assert_eq!(g.values[2], 3.);
        assert!(g.values[3].is_nan());
        assert_eq!(g.values[4..], [1., 2.]);
        assert_eq!(g.unit, "mg m-3");
    }

    #[test]
    fn uneven_latitude_spacing_is_allowed() {
        assert!(build(vec![40.0, 40.1, 40.3], vec![-74.0], vec![1., 2., 3.]).is_ok());
    }

    #[test]
    fn bad_shapes_and_axes_are_refused() {
        assert_eq!(build(vec![], vec![1.0], vec![]), Err(GridError::EmptyAxis));
        assert_eq!(
            build(vec![40.0, 40.1], vec![-74.0], vec![1.0]),
            Err(GridError::ShapeMismatch {
                lats: 2,
                lons: 1,
                found: 1
            })
        );
        assert_eq!(
            build(vec![40.0, 40.0], vec![-74.0], vec![1.0, 2.0]),
            Err(GridError::BadAxis { axis: "latitude" })
        );
        assert_eq!(
            build(vec![40.0], vec![-73.9, -74.0], vec![1.0, 2.0]),
            Err(GridError::BadAxis { axis: "longitude" })
        );
        assert_eq!(
            build(vec![f64::NAN], vec![-74.0], vec![1.0]),
            Err(GridError::BadAxis { axis: "latitude" })
        );
    }
}
