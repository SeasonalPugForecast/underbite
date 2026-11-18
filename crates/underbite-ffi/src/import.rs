//! Importing waypoints and tracks across the FFI: GPX, KML, KMZ and CSV files the user opens.
//!
//! The app reads the file and passes its bytes once; the core refuses more than 20 MB or 200,000
//! points. What is imported is returned as records and stays on the device. The file's bytes are
//! never turned into text on this side.

use underbite_core::import::{Format, ImportError as CoreImportError, Imported, detect, import};

/// Why a file could not be imported.
#[derive(Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum ImportError {
    /// The file is not acceptable: too large, of an unknown type or malformed.
    #[error("{message}")]
    Invalid {
        /// The core's words, with a line number where there is one.
        message: String,
    },
}

impl From<CoreImportError> for ImportError {
    fn from(e: CoreImportError) -> Self {
        Self::Invalid {
            message: e.to_string(),
        }
    }
}

/// The file types the importer reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum ImportFormat {
    Gpx,
    Kml,
    Kmz,
    Csv,
}

impl From<Format> for ImportFormat {
    fn from(f: Format) -> Self {
        match f {
            Format::Gpx => Self::Gpx,
            Format::Kml => Self::Kml,
            Format::Kmz => Self::Kmz,
            Format::Csv => Self::Csv,
        }
    }
}

/// A named point from a file.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct ImportedWaypoint {
    pub name: Option<String>,
    /// Degrees north.
    pub latitude: f64,
    /// Degrees east.
    pub longitude: f64,
    /// The time of the point, Unix seconds.
    pub time: Option<i64>,
    /// Description or comment.
    pub notes: Option<String>,
}

/// One point of a track.
#[derive(Clone, Copy, Debug, PartialEq, uniffi::Record)]
pub struct ImportedPoint {
    /// Degrees north.
    pub latitude: f64,
    /// Degrees east.
    pub longitude: f64,
    /// The time of the point, Unix seconds.
    pub time: Option<i64>,
}

/// A track or route: its segments in file order.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct ImportedTrack {
    pub name: Option<String>,
    pub segments: Vec<Vec<ImportedPoint>>,
}

/// What a file held.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct ImportedFile {
    pub format: ImportFormat,
    pub waypoints: Vec<ImportedWaypoint>,
    pub tracks: Vec<ImportedTrack>,
    /// Things to tell the user about how the file was read, one sentence each.
    pub notes: Vec<String>,
    /// A short account of what was read, followed by the notes, one per line.
    pub summary: String,
}

fn record(imported: Imported, format: ImportFormat) -> ImportedFile {
    let summary = imported.summary();
    ImportedFile {
        format,
        waypoints: imported
            .waypoints
            .into_iter()
            .map(|w| ImportedWaypoint {
                name: w.name,
                latitude: w.latitude,
                longitude: w.longitude,
                time: w.time.map(|t| t.0),
                notes: w.notes,
            })
            .collect(),
        tracks: imported
            .tracks
            .into_iter()
            .map(|t| ImportedTrack {
                name: t.name,
                segments: t
                    .segments
                    .into_iter()
                    .map(|s| {
                        s.into_iter()
                            .map(|p| ImportedPoint {
                                latitude: p.latitude,
                                longitude: p.longitude,
                                time: p.time.map(|t| t.0),
                            })
                            .collect()
                    })
                    .collect(),
            })
            .collect(),
        notes: imported.notes,
        summary,
    }
}

/// The type of a file, told from its content first and its name second; `None` when it is not
/// one the importer reads.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn import_detect(bytes: Vec<u8>, file_name: String) -> Option<ImportFormat> {
    detect(&bytes, &file_name).map(Into::into)
}

/// Reads a GPX, KML, KMZ or CSV file. Never panics on a bad file; the error says what is wrong.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn import_file(bytes: Vec<u8>, file_name: String) -> Result<ImportedFile, ImportError> {
    let format = detect(&bytes, &file_name).ok_or(CoreImportError::Unrecognised)?;
    Ok(record(import(&bytes, &file_name)?, format.into()))
}
