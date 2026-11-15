//! NDBC real-time files: standard meteorological (`.txt`), spectral wave summary (`.spec`),
//! oceanographic (`.ocean`) and current profile (`.adcp`).
//!
//! Every file starts with two `#` header lines (column names, then units) and holds rows
//! newest first, split on whitespace: `YY MM DD hh mm` in UTC, then the values, `MM` meaning
//! missing. Columns are found by header name on every parse, never by position, and the unit
//! of each column is read from the header. A column that exists but holds no value in a row
//! (or in the whole file) yields no sample: `MM` is never zero. `.adcp` rows are shorter than
//! the header.
//!
//! Left out on purpose: `COND` (NDBC conductivity is in mS/cm at an unstated reference
//! temperature, while the snapshot's conductivity is specific conductance at 25 °C: they are
//! different quantities), `O2PPM` (its unit `ppm` is not one the core converts), `O2%`, `EH`,
//! `DEPTH`, `PTDY` and `TIDE` (CO-OPS is the tide source).

use underbite_core::model::{
    Measurement, Turbidity, TurbidityUnit, WaveDirection, WaveSample, WaveTrain, Wind,
};
use underbite_core::provenance::{Provenance, Quality, SourceId, ValueKind};
use underbite_core::series::{Sample, Series, Vector};
use underbite_core::time::{UnixTime, parse_ndbc};
use underbite_core::units::{Canonical, SourceUnit};

use crate::request::{Place, Request, Window};
use crate::source::{FragmentValue, SourceError};

/// The marker value of a missing cell.
const MISSING: &str = "MM";
/// Columns before the values: `YY MM DD hh mm`.
const TIME_COLUMNS: [&str; 5] = ["YY", "MM", "DD", "hh", "mm"];

fn parse_error(message: impl Into<String>) -> SourceError {
    SourceError::Parse(message.into())
}

/// One data row: its time and the cells after it, as written.
#[derive(Debug)]
struct Row<'a> {
    time: UnixTime,
    cells: Vec<&'a str>,
}

/// A parsed file: header names and units, rows in file order.
#[derive(Debug)]
struct Table<'a> {
    names: Vec<&'a str>,
    units: Vec<&'a str>,
    rows: Vec<Row<'a>>,
}

impl<'a> Table<'a> {
    fn parse(text: &'a str) -> Result<Self, SourceError> {
        let mut lines = text.lines();
        let mut header = |what: &str| {
            lines
                .by_ref()
                .find(|l| !l.trim().is_empty())
                .and_then(|l| l.trim_start().strip_prefix('#'))
                .ok_or_else(|| parse_error(format!("missing {what} header line")))
        };
        let names: Vec<&str> = header("column name")?.split_whitespace().collect();
        let units: Vec<&str> = header("unit")?.split_whitespace().collect();
        if names.len() != units.len() || names.len() < TIME_COLUMNS.len() {
            return Err(parse_error(format!(
                "{} column names but {} units",
                names.len(),
                units.len()
            )));
        }
        if names[..TIME_COLUMNS.len()] != TIME_COLUMNS {
            return Err(parse_error(format!(
                "time columns are {:?}, not {TIME_COLUMNS:?}",
                &names[..TIME_COLUMNS.len()]
            )));
        }
        let mut rows = Vec::new();
        for line in lines {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let tokens: Vec<&str> = line.split_whitespace().collect();
            if tokens.len() < TIME_COLUMNS.len() || tokens.len() > names.len() {
                return Err(parse_error(format!(
                    "row of {} columns under {} header columns: {line:?}",
                    tokens.len(),
                    names.len()
                )));
            }
            let time = parse_ndbc(&tokens[..TIME_COLUMNS.len()].join(" "))
                .map_err(|e| parse_error(e.to_string()))?;
            rows.push(Row {
                time,
                cells: tokens[TIME_COLUMNS.len()..].to_vec(),
            });
        }
        Ok(Self { names, units, rows })
    }

    fn index(&self, name: &str) -> Option<usize> {
        self.names
            .iter()
            .position(|n| *n == name)
            .filter(|i| *i >= TIME_COLUMNS.len())
            .map(|i| i - TIME_COLUMNS.len())
    }

    /// The numeric column `name` read as `kind`, or `None` when the header lacks it.
    fn num(&self, name: &str, kind: Kind) -> Result<Option<Num>, SourceError> {
        let Some(index) = self.index(name) else {
            return Ok(None);
        };
        let unit = self.units[index + TIME_COLUMNS.len()];
        let conv = match kind.label() {
            Some(label) if unit == label => Conv::Plain,
            Some(label) => {
                return Err(parse_error(format!("{name} is in {unit:?}, not {label:?}")));
            }
            None => {
                let parsed =
                    SourceUnit::parse(unit).map_err(|e| parse_error(format!("{name}: {e}")))?;
                // A unit of another group (feet where metres are meant) is not a value of
                // this kind: refuse it instead of reading it wrongly.
                if !kind.accepts(parsed.to_canonical(0.0)) {
                    return Err(parse_error(format!("{name} is in {unit:?}, not {kind:?}")));
                }
                Conv::Unit(parsed)
            }
        };
        Ok(Some(Num { index, conv }))
    }

    /// The text column `name`, as its cell index.
    fn text(&self, name: &str) -> Option<usize> {
        self.index(name)
    }
}

/// What a numeric column measures; fixes the unit it must be written in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Speed,
    Length,
    Temperature,
    Pressure,
    Salinity,
    Micrograms,
    Turbidity,
    /// `degT`, kept as written.
    Degrees,
    /// `sec`, kept as written.
    Seconds,
    /// `-`: pH has no unit.
    Ph,
}

impl Kind {
    /// The unit label of kinds the core does not convert.
    fn label(self) -> Option<&'static str> {
        match self {
            Self::Degrees => Some("degT"),
            Self::Seconds => Some("sec"),
            Self::Ph => Some("-"),
            _ => None,
        }
    }

    fn accepts(self, c: Canonical) -> bool {
        matches!(
            (self, c),
            (Self::Speed, Canonical::Speed(_))
                | (Self::Length, Canonical::Length(_))
                | (Self::Temperature, Canonical::Temperature(_))
                | (Self::Pressure, Canonical::Pressure(_))
                | (Self::Salinity, Canonical::Salinity(_))
                | (Self::Micrograms, Canonical::ConcentrationUgPerL(_))
                | (Self::Turbidity, Canonical::Turbidity(..))
        )
    }
}

#[derive(Clone, Copy, Debug)]
enum Conv {
    Plain,
    Unit(SourceUnit),
}

/// A numeric column: where it is and how to read it.
#[derive(Clone, Copy, Debug)]
struct Num {
    index: usize,
    conv: Conv,
}

impl Num {
    fn value(&self, row: &Row<'_>) -> Result<Option<f32>, SourceError> {
        let Some(cell) = row.cells.get(self.index) else {
            return Ok(None);
        };
        if *cell == MISSING {
            return Ok(None);
        }
        let v: f32 = cell
            .parse()
            .ok()
            .filter(|v: &f32| v.is_finite())
            .ok_or_else(|| parse_error(format!("{cell:?} is not a number")))?;
        Ok(Some(match self.conv {
            Conv::Plain => v,
            Conv::Unit(unit) => match unit.to_canonical(v) {
                Canonical::Speed(v)
                | Canonical::Length(v)
                | Canonical::Temperature(v)
                | Canonical::Pressure(v)
                | Canonical::Salinity(v)
                | Canonical::ConcentrationUgPerL(v)
                | Canonical::Turbidity(v, _) => v,
                other => return Err(parse_error(format!("unexpected quantity {other:?}"))),
            },
        }))
    }

    /// The turbidity unit label of the column, as the header wrote it.
    fn turbidity_unit(&self) -> Option<TurbidityUnit> {
        match self.conv {
            Conv::Unit(SourceUnit::Turbidity(u)) => Some(u),
            _ => None,
        }
    }
}

fn value(col: Option<&Num>, row: &Row<'_>) -> Result<Option<f32>, SourceError> {
    col.map_or(Ok(None), |c| c.value(row))
}

fn cell_text<'a>(col: Option<usize>, row: &Row<'a>) -> Option<&'a str> {
    col.and_then(|i| row.cells.get(i).copied())
        .filter(|c| *c != MISSING)
}

/// A value built from a file, with the time of its newest sample.
#[derive(Debug)]
pub(crate) struct Built {
    pub(crate) measurement: Measurement,
    /// Whether it is the station's main product for the measurement.
    pub(crate) primary: bool,
    pub(crate) value: FragmentValue,
    pub(crate) last_value_at: UnixTime,
}

/// The samples of the rows `f` reads a value from, within `window` when there is one.
fn samples<T>(
    table: &Table<'_>,
    window: Option<Window>,
    mut f: impl FnMut(&Row<'_>) -> Result<Option<T>, SourceError>,
) -> Result<Vec<Sample<T>>, SourceError> {
    let mut out = Vec::new();
    for row in &table.rows {
        if window.is_some_and(|w| row.time < w.from || row.time > w.to) {
            continue;
        }
        if let Some(v) = f(row)? {
            out.push(Sample::new(row.time, v));
        }
    }
    Ok(out)
}

/// What each build needs besides the table.
struct Job<'a, W: Fn(Measurement) -> bool> {
    request: &'a Request,
    window: Option<Window>,
    wants: W,
}

impl<W: Fn(Measurement) -> bool> Job<'_, W> {
    fn provenance(&self, observed_at: UnixTime) -> Result<Provenance, SourceError> {
        let Place::Station {
            station,
            distance_m,
            relation,
        } = &self.request.target.place
        else {
            return Err(SourceError::Unsupported("NDBC data needs a station".into()));
        };
        Ok(Provenance {
            source: SourceId::NDBC,
            station: Some(station.clone()),
            distance_m: *distance_m,
            relation: *relation,
            observed_at,
            kind: ValueKind::Observed,
            // Real-time values have passed automated checks only.
            quality: Quality::Provisional,
        })
    }

    /// Adds `samples` as a series wrapped by `wrap`; nothing when there are none.
    fn add<T>(
        &self,
        out: &mut Vec<Built>,
        measurement: Measurement,
        primary: bool,
        samples: Vec<Sample<T>>,
        wrap: impl FnOnce(Series<T>) -> FragmentValue,
    ) -> Result<(), SourceError> {
        let Some(last_value_at) = samples.iter().map(|s| s.time).max() else {
            return Ok(());
        };
        let series = Series::new(self.provenance(last_value_at)?, samples);
        out.push(Built {
            measurement,
            primary,
            value: wrap(series),
            last_value_at,
        });
        Ok(())
    }

    fn scalar(
        &self,
        out: &mut Vec<Built>,
        table: &Table<'_>,
        measurement: Measurement,
        primary: bool,
        column: (&str, Kind),
    ) -> Result<(), SourceError> {
        if !(self.wants)(measurement) {
            return Ok(());
        }
        let col = table.num(column.0, column.1)?;
        let s = samples(table, self.window, |row| value(col.as_ref(), row))?;
        self.add(out, measurement, primary, s, FragmentValue::Series)
    }
}

/// `.txt`: waves, wind, pressure and water temperature.
pub(crate) fn standard(
    text: &str,
    request: &Request,
    window: Option<Window>,
    wants: impl Fn(Measurement) -> bool,
) -> Result<Vec<Built>, SourceError> {
    let table = Table::parse(text)?;
    let job = Job {
        request,
        window,
        wants,
    };
    let mut out = Vec::new();
    if (job.wants)(Measurement::Waves) {
        let wvht = table.num("WVHT", Kind::Length)?;
        let dpd = table.num("DPD", Kind::Seconds)?;
        let apd = table.num("APD", Kind::Seconds)?;
        let mwd = table.num("MWD", Kind::Degrees)?;
        let s = samples(&table, window, |row| {
            let sample = WaveSample {
                significant_height_m: value(wvht.as_ref(), row)?,
                dominant_period_s: value(dpd.as_ref(), row)?,
                mean_period_s: value(apd.as_ref(), row)?,
                direction_from_deg: value(mwd.as_ref(), row)?,
                ..WaveSample::default()
            };
            Ok((sample != WaveSample::default()).then_some(sample))
        })?;
        job.add(
            &mut out,
            Measurement::Waves,
            true,
            s,
            FragmentValue::WaveSeries,
        )?;
    }
    if (job.wants)(Measurement::ObservedWind) {
        let wspd = table.num("WSPD", Kind::Speed)?;
        let gst = table.num("GST", Kind::Speed)?;
        let wdir = table.num("WDIR", Kind::Degrees)?;
        let s = samples(&table, window, |row| {
            let Some(speed_ms) = value(wspd.as_ref(), row)? else {
                return Ok(None);
            };
            Ok(Some(Wind {
                speed_ms,
                gust_ms: value(gst.as_ref(), row)?,
                from_deg: value(wdir.as_ref(), row)?,
            }))
        })?;
        job.add(
            &mut out,
            Measurement::ObservedWind,
            true,
            s,
            FragmentValue::Wind,
        )?;
    }
    job.scalar(
        &mut out,
        &table,
        Measurement::ObservedPressure,
        true,
        ("PRES", Kind::Pressure),
    )?;
    job.scalar(
        &mut out,
        &table,
        Measurement::WaterTemperature,
        true,
        ("WTMP", Kind::Temperature),
    )?;
    Ok(out)
}

/// `.spec`: waves with their swell and wind-wave parts. Directions of the parts are compass
/// text (`SSE`), not degrees; `STEEPNESS` is text and `N/A` is a value of its own.
pub(crate) fn spectral(
    text: &str,
    request: &Request,
    window: Option<Window>,
    wants: impl Fn(Measurement) -> bool,
) -> Result<Vec<Built>, SourceError> {
    let job = Job {
        request,
        window,
        wants,
    };
    let mut out = Vec::new();
    if !(job.wants)(Measurement::Waves) {
        return Ok(out);
    }
    let table = Table::parse(text)?;
    let wvht = table.num("WVHT", Kind::Length)?;
    let apd = table.num("APD", Kind::Seconds)?;
    let mwd = table.num("MWD", Kind::Degrees)?;
    let swh = table.num("SwH", Kind::Length)?;
    let swp = table.num("SwP", Kind::Seconds)?;
    let wwh = table.num("WWH", Kind::Length)?;
    let wwp = table.num("WWP", Kind::Seconds)?;
    let (swd, wwd, steep) = (
        table.text("SwD"),
        table.text("WWD"),
        table.text("STEEPNESS"),
    );
    let train = |h: Option<f32>, p: Option<f32>, d: Option<&str>| {
        (h.is_some() || p.is_some() || d.is_some()).then(|| WaveTrain {
            height_m: h,
            period_s: p,
            direction_from: d.map(|d| WaveDirection::Compass(d.to_string())),
        })
    };
    let s = samples(&table, window, |row| {
        let sample = WaveSample {
            significant_height_m: value(wvht.as_ref(), row)?,
            dominant_period_s: None,
            mean_period_s: value(apd.as_ref(), row)?,
            direction_from_deg: value(mwd.as_ref(), row)?,
            wind_waves: train(
                value(wwh.as_ref(), row)?,
                value(wwp.as_ref(), row)?,
                cell_text(wwd, row),
            ),
            swells: train(
                value(swh.as_ref(), row)?,
                value(swp.as_ref(), row)?,
                cell_text(swd, row),
            )
            .into_iter()
            .collect(),
            steepness: cell_text(steep, row).map(str::to_string),
        };
        Ok((sample != WaveSample::default()).then_some(sample))
    })?;
    job.add(
        &mut out,
        Measurement::Waves,
        true,
        s,
        FragmentValue::WaveSeries,
    )?;
    Ok(out)
}

/// `.ocean`: water temperature at the sensor, salinity, turbidity (unit label kept),
/// chlorophyll and pH.
pub(crate) fn ocean(
    text: &str,
    request: &Request,
    window: Option<Window>,
    wants: impl Fn(Measurement) -> bool,
) -> Result<Vec<Built>, SourceError> {
    let table = Table::parse(text)?;
    let job = Job {
        request,
        window,
        wants,
    };
    let mut out = Vec::new();
    // The `.txt` `WTMP` is the surface temperature; this one is at the sensor's depth.
    job.scalar(
        &mut out,
        &table,
        Measurement::WaterTemperature,
        false,
        ("OTMP", Kind::Temperature),
    )?;
    job.scalar(
        &mut out,
        &table,
        Measurement::Salinity,
        true,
        ("SAL", Kind::Salinity),
    )?;
    job.scalar(
        &mut out,
        &table,
        Measurement::Chlorophyll,
        true,
        ("CLCON", Kind::Micrograms),
    )?;
    job.scalar(&mut out, &table, Measurement::Ph, true, ("PH", Kind::Ph))?;
    if (job.wants)(Measurement::Turbidity) {
        let turb = table.num("TURB", Kind::Turbidity)?;
        if let Some(unit) = turb.as_ref().and_then(Num::turbidity_unit) {
            let s = samples(&table, window, |row| {
                Ok(value(turb.as_ref(), row)?.map(|value| Turbidity { value, unit }))
            })?;
            job.add(
                &mut out,
                Measurement::Turbidity,
                true,
                s,
                FragmentValue::Turbidity,
            )?;
        }
    }
    Ok(out)
}

/// `.adcp`: the current of the shallowest bin that reported in each row, as the direction the
/// water flows toward and its speed. Rows carry only the bins that reported, so the cells
/// after the last bin written are absent, not missing.
pub(crate) fn adcp(
    text: &str,
    request: &Request,
    window: Option<Window>,
    wants: impl Fn(Measurement) -> bool,
) -> Result<Vec<Built>, SourceError> {
    let job = Job {
        request,
        window,
        wants,
    };
    let mut out = Vec::new();
    if !(job.wants)(Measurement::OceanCurrent) {
        return Ok(out);
    }
    let table = Table::parse(text)?;
    let mut bins = Vec::new();
    for name in &table.names {
        let Some(n) = name.strip_prefix("DEP") else {
            continue;
        };
        let (Some(depth), Some(dir), Some(speed)) = (
            table.num(name, Kind::Length)?,
            table.num(&format!("DIR{n}"), Kind::Degrees)?,
            table.num(&format!("SPD{n}"), Kind::Speed)?,
        ) else {
            continue;
        };
        bins.push((depth, dir, speed));
    }
    let s = samples(&table, window, |row| {
        let mut best: Option<(f32, Vector)> = None;
        for (depth, dir, speed) in &bins {
            let (Some(d), Some(toward_deg), Some(speed_ms)) =
                (depth.value(row)?, dir.value(row)?, speed.value(row)?)
            else {
                continue;
            };
            if best.is_none_or(|(shallowest, _)| d < shallowest) {
                best = Some((
                    d,
                    Vector {
                        speed_ms,
                        toward_deg,
                    },
                ));
            }
        }
        Ok(best.map(|(_, v)| v))
    })?;
    job.add(
        &mut out,
        Measurement::OceanCurrent,
        true,
        s,
        FragmentValue::Vectors,
    )?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use underbite_core::provenance::{Relation, StationKind, StationRef};

    use super::*;
    use crate::request::{Resource, Target};
    use crate::testutil::fixture_str;

    fn station(id: &str, kind: StationKind) -> StationRef {
        StationRef {
            source: SourceId::NDBC,
            id: id.into(),
            name: id.into(),
            kind,
            latitude: 40.0,
            longitude: -73.0,
            bin: None,
            reference_id: None,
        }
    }

    fn request(id: &str, resource: Resource) -> Request {
        Request::new(
            String::new(),
            resource,
            Target {
                place: Place::Station {
                    station: station(id, StationKind::Buoy),
                    distance_m: Some(1_000.0),
                    relation: Relation::Nearby,
                },
                measurements: std::collections::BTreeSet::default(),
                window: Window {
                    from: UnixTime(0),
                    to: UnixTime(i64::MAX / 2),
                },
                datum: None,
            },
        )
    }

    fn all(_: Measurement) -> bool {
        true
    }

    fn by_measurement(built: Vec<Built>) -> BTreeMap<Measurement, Built> {
        built.into_iter().map(|b| (b.measurement, b)).collect()
    }

    fn at(s: &str) -> UnixTime {
        parse_ndbc(s).unwrap()
    }

    #[test]
    fn rows_must_fit_the_header() {
        let head = "#YY  MM DD hh mm WVHT\n#yr  mo dy hr mn m\n";
        assert!(Table::parse(&format!("{head}2026 11 08 13 40 1.0 2.0\n")).is_err());
        assert!(Table::parse("2026 11 08 13 40 1.0\n").is_err());
        assert!(Table::parse("#a b c d e\n#x y z u v\n").is_err());
        let text = format!("{head}2026 11 08 13 40\n");
        let ragged = Table::parse(&text).unwrap();
        assert_eq!(ragged.rows[0].cells.len(), 0);
    }

    #[test]
    fn column_units_are_checked() {
        let feet = "#YY MM DD hh mm WVHT\n#yr mo dy hr mn ft\n2026 11 08 13 40 3.0\n";
        let table = Table::parse(feet).unwrap();
        let col = table.num("WVHT", Kind::Length).unwrap().unwrap();
        let v = col.value(&table.rows[0]).unwrap().unwrap();
        assert!((v - 0.9144).abs() < 1e-4, "{v}");
        let wrong = "#YY MM DD hh mm WVHT\n#yr mo dy hr mn m/s\n2026 11 08 13 40 3.0\n";
        let table = Table::parse(wrong).unwrap();
        assert!(table.num("WVHT", Kind::Length).is_err());
        assert!(table.num("WVHT", Kind::Seconds).is_err());
        assert!(table.num("NOPE", Kind::Length).unwrap().is_none());
    }

    #[test]
    fn long_buoy_file_has_waves_and_water_temperature_only() {
        let text = fixture_str("ndbc", "ndbc-44025.txt");
        let table = Table::parse(&text).unwrap();
        assert_eq!(table.rows.len(), 1700);
        let req = request("44025", Resource::NdbcStandard);
        let built = by_measurement(standard(&text, &req, None, all).unwrap());
        assert_eq!(
            built.keys().copied().collect::<Vec<_>>(),
            [Measurement::WaterTemperature, Measurement::Waves]
        );
        let FragmentValue::WaveSeries(waves) = &built[&Measurement::Waves].value else {
            panic!("not waves");
        };
        assert_eq!(waves.samples().len(), 1700);
        assert_eq!(
            built[&Measurement::Waves].last_value_at,
            at("2026 11 08 13 40")
        );
        let newest = waves.latest().unwrap();
        assert_eq!(newest.time, at("2026 11 08 13 40"));
        assert_eq!(newest.value.significant_height_m, Some(1.8));
        assert_eq!(newest.value.mean_period_s, Some(4.6));
        assert_eq!(newest.value.direction_from_deg, Some(281.0));
        let FragmentValue::Series(temp) = &built[&Measurement::WaterTemperature].value else {
            panic!("not a series");
        };
        assert_eq!(temp.samples().len(), 1700 - 86);
        let p = waves.provenance();
        assert_eq!(p.source, SourceId::NDBC);
        assert_eq!(p.observed_at, newest.time);
        assert_eq!(p.relation, Relation::Nearby);
        assert_eq!(p.quality, Quality::Provisional);
    }

    #[test]
    fn window_cuts_samples_but_not_coverage() {
        let text = fixture_str("ndbc", "ndbc-44025.txt");
        let req = request("44025", Resource::NdbcStandard);
        let window = Window {
            from: at("2026 11 08 00 00"),
            to: at("2026 11 08 23 59"),
        };
        let built = standard(&text, &req, Some(window), |m| m == Measurement::Waves).unwrap();
        assert_eq!(built.len(), 1);
        let FragmentValue::WaveSeries(waves) = &built[0].value else {
            panic!("not waves");
        };
        assert!(waves.samples().iter().all(|s| window.from <= s.time));
        assert_eq!(waves.samples().len(), 26);
    }

    #[test]
    fn harbor_entrance_buoy_has_wind_and_pressure() {
        let text = fixture_str("ndbc", "ndbc-44065-head300.txt");
        let req = request("44065", Resource::NdbcStandard);
        let built = by_measurement(standard(&text, &req, None, all).unwrap());
        for m in [
            Measurement::ObservedWind,
            Measurement::ObservedPressure,
            Measurement::WaterTemperature,
            Measurement::Waves,
        ] {
            assert!(built.contains_key(&m), "{m:?}");
        }
        let FragmentValue::Wind(wind) = &built[&Measurement::ObservedWind].value else {
            panic!("not wind");
        };
        assert_eq!(wind.samples().len(), 293);
        let w = wind.latest().unwrap().value;
        assert!(w.speed_ms > 0.0 && w.from_deg.is_some());
        let FragmentValue::WaveSeries(waves) = &built[&Measurement::Waves].value else {
            panic!("not waves");
        };
        // Waves are present in only some rows.
        assert!(waves.samples().len() < 300 && !waves.is_empty());
        insta::assert_json_snapshot!(
            "buoy_44065_newest",
            serde_json::json!({
                "wind": wind.latest(),
                "waves": waves.latest(),
                "provenance": wind.provenance(),
            })
        );
    }

    #[test]
    fn coastal_station_file_has_six_minute_rows() {
        let text = fixture_str("ndbc", "ndbc-BATN6-head300.txt");
        let req = request("batn6", Resource::NdbcStandard);
        let built = by_measurement(standard(&text, &req, None, all).unwrap());
        assert!(!built.contains_key(&Measurement::ObservedWind));
        assert!(!built.contains_key(&Measurement::Waves));
        let FragmentValue::Series(pressure) = &built[&Measurement::ObservedPressure].value else {
            panic!("not a series");
        };
        let times: Vec<i64> = pressure.samples().iter().map(|s| s.time.0).collect();
        assert!(times.windows(2).all(|w| (w[1] - w[0]) % 360 == 0));
        assert!(times.windows(2).filter(|w| w[1] - w[0] == 360).count() > times.len() - 5);
        assert!(built.contains_key(&Measurement::WaterTemperature));
    }

    #[test]
    fn an_all_missing_row_does_not_count_as_an_observation() {
        let text = "#YY MM DD hh mm WSPD GST WDIR PRES\n#yr mo dy hr mn m/s m/s degT hPa\n\
                    2026 11 08 14 00 MM MM MM MM\n\
                    2026 11 08 13 54 3.0 MM 90 1015.2\n";
        let req = request("batn6", Resource::NdbcStandard);
        let built = by_measurement(standard(text, &req, None, all).unwrap());
        assert_eq!(
            built[&Measurement::ObservedPressure].last_value_at,
            at("2026 11 08 13 54")
        );
        let FragmentValue::Wind(wind) = &built[&Measurement::ObservedWind].value else {
            panic!("not wind");
        };
        assert_eq!(
            wind.latest().unwrap().value,
            Wind {
                speed_ms: 3.0,
                gust_ms: None,
                from_deg: Some(90.0)
            }
        );
    }

    #[test]
    fn spectral_summary_keeps_steepness_text_and_compass_parts() {
        let req = request("44025", Resource::NdbcSpectralSummary);
        let text = fixture_str("ndbc", "ndbc-44025.spec");
        let built = spectral(&text, &req, None, all).unwrap();
        let FragmentValue::WaveSeries(waves) = &built[0].value else {
            panic!("not waves");
        };
        let steepness: std::collections::BTreeSet<_> = waves
            .samples()
            .iter()
            .filter_map(|s| s.value.steepness.as_deref())
            .collect();
        assert_eq!(
            steepness.into_iter().collect::<Vec<_>>(),
            ["AVERAGE", "N/A", "STEEP", "VERY_STEEP"]
        );
        assert!(waves.samples().iter().all(|s| s.value.swells.is_empty()));

        let text = fixture_str("ndbc", "ndbc-44065-spec-head150.spec");
        let built = spectral(
            &text,
            &request("44065", Resource::NdbcSpectralSummary),
            None,
            all,
        )
        .unwrap();
        let FragmentValue::WaveSeries(waves) = &built[0].value else {
            panic!("not waves");
        };
        let compass = waves
            .samples()
            .iter()
            .find_map(|s| s.value.swells.first())
            .and_then(|t| t.direction_from.clone());
        assert!(
            matches!(compass, Some(WaveDirection::Compass(_))),
            "{compass:?}"
        );
        insta::assert_json_snapshot!(
            "spec_44065_head",
            waves.samples().iter().take(3).collect::<Vec<_>>()
        );
    }

    #[test]
    fn ocean_file_reads_present_columns_only() {
        let text = fixture_str("ndbc", "ndbc-44069-ocean-head300.ocean");
        let req = request("44069", Resource::NdbcOcean);
        let built = by_measurement(ocean(&text, &req, None, all).unwrap());
        assert_eq!(
            built.keys().copied().collect::<Vec<_>>(),
            [Measurement::WaterTemperature, Measurement::Salinity]
        );
        assert!(!built[&Measurement::WaterTemperature].primary);
        let FragmentValue::Series(sal) = &built[&Measurement::Salinity].value else {
            panic!("not a series");
        };
        assert_eq!(sal.latest().unwrap().value, 25.75);
        assert_eq!(sal.latest().unwrap().time, at("2026 11 08 14 00"));
    }

    #[test]
    fn turbidity_keeps_the_ftu_label() {
        let text = "#YY MM DD hh mm DEPTH OTMP COND SAL O2% O2PPM CLCON TURB PH EH\n\
                    #yr mo dy hr mn m degC mS/cm psu % ppm ug/l FTU - mv\n\
                    2026 11 08 14 00 1.0 14.4 MM 25.7 MM MM 3.5 2.25 8.1 MM\n";
        let req = request("44069", Resource::NdbcOcean);
        let built = by_measurement(ocean(text, &req, None, all).unwrap());
        let FragmentValue::Turbidity(turb) = &built[&Measurement::Turbidity].value else {
            panic!("not turbidity");
        };
        assert_eq!(
            turb.latest().unwrap().value,
            Turbidity {
                value: 2.25,
                unit: TurbidityUnit::Ftu
            }
        );
        assert!(built.contains_key(&Measurement::Chlorophyll));
        assert!(built.contains_key(&Measurement::Ph));
        assert!(!built.keys().any(|m| *m == Measurement::Conductivity));
    }

    #[test]
    fn current_profile_rows_are_shorter_than_the_header() {
        let text = fixture_str("ndbc", "ndbc-44091-adcp-head100.adcp");
        let req = request("44091", Resource::NdbcAdcp);
        let built = adcp(&text, &req, None, all).unwrap();
        let FragmentValue::Vectors(v) = &built[0].value else {
            panic!("not vectors");
        };
        assert_eq!(v.samples().len(), 100);
        let newest = v.latest().unwrap();
        assert_eq!(newest.time, at("2026 11 08 14 00"));
        assert_eq!(newest.value.toward_deg, 110.0);
        assert!((newest.value.speed_ms - 0.28).abs() < 1e-6);
    }

    #[test]
    fn shallowest_reporting_bin_wins() {
        let text = "#YY MM DD hh mm DEP01 DIR01 SPD01 DEP02 DIR02 SPD02\n\
                    #yr mo dy hr mn m degT cm/s m degT cm/s\n\
                    2026 11 08 14 00 4 90 10 2 180 20\n\
                    2026 11 08 13 30 4 90 10 MM MM MM\n";
        let built = adcp(text, &request("x", Resource::NdbcAdcp), None, all).unwrap();
        let FragmentValue::Vectors(v) = &built[0].value else {
            panic!("not vectors");
        };
        assert_eq!(v.samples()[1].value.toward_deg, 180.0);
        assert_eq!(v.samples()[0].value.toward_deg, 90.0);
    }
}
