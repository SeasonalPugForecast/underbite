//! Turbidity, chlorophyll and clarity.

use schemars::JsonSchema;
use serde::Serialize;

use crate::model::{Measurement, TurbidityUnit};
use crate::provenance::{Provenance, Quality};
use crate::series::{Sample, Series};
use crate::time::UnixTime;
use crate::view::cards::flow::{
    card_state, chart_series, chart_vm, credits, is_unreliable, qty, quality_notices, row,
    sample_quality, spark, unreliable_text,
};
use crate::view::format::{Formatter, number};
use crate::view::{
    CardChrome, CardId, ChartVm, Compact, Quantity, Row, Section, SeriesRole, ViewInput,
};

const WEEK_S: i64 = 7 * 86_400;
const KD490_UNAVAILABLE: &str = "Unavailable: no current satellite source";

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct ClarityCard {
    pub chrome: CardChrome,
    pub compact: Compact,
    pub full: ClarityFull,
}

/// The satellite diffuse attenuation coefficient; marked unavailable where there is none.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct Kd490Vm {
    pub available: bool,
    pub value: Option<Quantity>,
    /// "No satellite clarity for this spot".
    pub note: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct ClarityFull {
    /// The unit symbol is the source's own: FNU, FTU or NTU, never converted.
    pub turbidity: Option<Quantity>,
    /// "Clear", "Slightly turbid", ...
    pub description: Option<String>,
    pub chlorophyll: Option<Quantity>,
    pub kd490: Kd490Vm,
    pub chart: Option<ChartVm>,
    pub sections: Vec<Section>,
}

/// A plain-words reading of a turbidity value; the methods differ, so it is a rough guide.
const fn describe_turbidity(value: f32) -> &'static str {
    if value < 2.0 {
        "Clear"
    } else if value < 10.0 {
        "Slightly turbid"
    } else if value < 50.0 {
        "Turbid"
    } else {
        "Very turbid"
    }
}

fn after<T>(samples: &[Sample<T>], from: UnixTime) -> &[Sample<T>] {
    &samples[samples.partition_point(|s| s.time < from)..]
}

#[allow(clippy::too_many_lines)]
pub fn build(input: &ViewInput) -> Option<ClarityCard> {
    let w = &input.snapshot.water;
    let f = Formatter::for_input(input);
    let now = input.now;
    let from = UnixTime(now.0 - WEEK_S);

    let turbidity = w.turbidity.as_ref().filter(|s| !s.is_empty());
    let chlorophyll = w.chlorophyll_ug_l.as_ref().filter(|s| !s.is_empty());
    let kd490 = w.clarity_kd490_per_m.as_ref().filter(|s| !s.is_empty());
    let cyano = w.cyanobacteria_cells_ml.as_ref().filter(|s| !s.is_empty());
    if turbidity.is_none() && chlorophyll.is_none() && kd490.is_none() && cyano.is_none() {
        return None;
    }

    // Turbidity keeps the unit it was measured in; a series that mixes units charts the latest.
    let latest_t = turbidity.and_then(|s| s.latest().map(|l| (s, l)));
    let turbidity_q: Option<Quantity> =
        latest_t.map(|(_, l)| f.turbidity(Some(l.value.value), l.value.unit));
    let description = latest_t.map(|(_, l)| describe_turbidity(l.value.value).to_owned());
    let chlorophyll_q = chlorophyll
        .and_then(|s| s.latest())
        .map(|l| qty(l.value, 1, "µg/L"));
    let kd490_vm = match kd490.and_then(|s| s.latest()) {
        Some(l) => Kd490Vm {
            available: true,
            value: Some(qty(l.value, 2, "1/m")),
            note: "Satellite estimate of how deeply light reaches".to_owned(),
        },
        None => Kd490Vm {
            available: false,
            value: None,
            note: KD490_UNAVAILABLE.to_owned(),
        },
    };

    // Chart: turbidity over the week, in the unit of the latest reading.
    let same_unit = |unit: TurbidityUnit| -> Vec<Sample<f32>> {
        turbidity
            .map(|s| {
                after(s.samples(), from)
                    .iter()
                    .filter(|x| x.value.unit == unit)
                    .map(|x| Sample::new(x.time, x.value.value))
                    .collect()
            })
            .unwrap_or_default()
    };
    let chart: Option<ChartVm> = latest_t.and_then(|(_, l)| {
        let unit = l.value.unit;
        let samples = same_unit(unit);
        let show = |v: f32| f.turbidity(Some(v), unit);
        let series = if samples.len() >= 2 {
            vec![chart_series(
                "Observed",
                SeriesRole::Observed,
                &samples,
                &show,
            )]
        } else {
            Vec::new()
        };
        chart_vm(
            &f,
            "Turbidity",
            unit.label().to_owned(),
            series,
            Vec::new(),
            now,
        )
    });
    let spark_values: Vec<f32> = latest_t
        .map(|(_, l)| same_unit(l.value.unit).iter().map(|s| s.value).collect())
        .unwrap_or_default();

    // Sections.
    let mut sections = Vec::new();
    let mut rows: Vec<Row> = Vec::new();
    if let Some(l) = cyano.and_then(|s| s.latest()) {
        rows.push(row(
            "Cyanobacteria",
            &format!("{} cells/mL", number(l.value, 0)),
        ));
    }
    if !rows.is_empty() {
        sections.push(Section {
            title: "Algae".to_owned(),
            rows,
        });
    }
    let turbidity_quality = latest_t.map(|(s, l)| sample_quality(s, l));
    if let Some(q) = turbidity_quality.filter(|q| is_unreliable(*q)) {
        sections.push(Section {
            title: "Reading quality".to_owned(),
            rows: vec![row("Reliability", unreliable_text(q))],
        });
    }

    // Provenance: turbidity leads, then chlorophyll, satellite and algae sources.
    let provs: Vec<&Provenance> = turbidity
        .map(Series::provenance)
        .into_iter()
        .chain(chlorophyll.map(Series::provenance))
        .chain(kd490.map(Series::provenance))
        .chain(cyano.map(Series::provenance))
        .collect();
    let main = provs.first().copied()?;
    let headline = turbidity_q
        .clone()
        .or_else(|| chlorophyll_q.clone())
        .or_else(|| kd490_vm.value.clone())
        .unwrap_or_else(|| f.turbidity(None, TurbidityUnit::Fnu));
    let mut lines: Vec<String> = Vec::new();
    if let Some(q) = turbidity_quality.filter(|q| is_unreliable(*q)) {
        lines.push(unreliable_text(q).to_owned());
    }
    lines.extend(description.clone());
    if let Some(c) = &chlorophyll_q {
        lines.push(format!("Chlorophyll {}", c.full()));
    }
    lines.push(f.station_footer(main).text);
    lines.truncate(2);

    let title = f.strings.card_title(CardId::Clarity).to_owned();
    let mut accessibility = format!("{title}: {}", headline.full());
    if let Some(d) = &description {
        accessibility.push_str(", ");
        accessibility.push_str(&d.to_lowercase());
    }
    if !kd490_vm.available {
        accessibility.push_str(". Satellite clarity is unavailable: no current source");
    }
    let qualities: Vec<Quality> = provs.iter().map(|p| p.quality).collect();
    let latest = latest_t.map(|(_, l)| l.time);
    let measurement = if turbidity.is_some() {
        Measurement::Turbidity
    } else if chlorophyll.is_some() {
        Measurement::Chlorophyll
    } else if cyano.is_some() {
        Measurement::Cyanobacteria
    } else {
        Measurement::Clarity
    };
    let label = if turbidity.is_some() {
        "Turbidity"
    } else {
        "Clarity"
    };

    Some(ClarityCard {
        chrome: CardChrome {
            id: CardId::Clarity,
            title,
            footnote: Some(f.footnote(&format!("{label} {}", headline.full()), main, now)),
            credits: credits(input, &provs),
            state: card_state(input, &f, measurement, main, latest),
            notices: quality_notices(&f, &qualities),
            accessibility,
        },
        compact: Compact {
            headline,
            lines,
            spark: spark(&spark_values),
        },
        full: ClarityFull {
            turbidity: turbidity_q,
            description,
            chlorophyll: chlorophyll_q,
            kd490: kd490_vm,
            chart,
            sections,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Turbidity;
    use crate::provenance::{SourceId, StationKind, UsgsSiteType, ValueKind};
    use crate::series::Series;
    use crate::view::Voice;
    use crate::view::cards::flow::fixtures::{NOW, input, metric, prov, run, us};

    fn turbidity(p: Provenance, unit: TurbidityUnit, values: &[f32]) -> Series<Turbidity> {
        let n = i64::try_from(values.len()).unwrap_or(0);
        let samples = values
            .iter()
            .enumerate()
            .map(|(i, v)| {
                Sample::new(
                    UnixTime(NOW - (n - 1 - i64::try_from(i).unwrap_or(0)) * 900),
                    Turbidity { value: *v, unit },
                )
            })
            .collect();
        Series::new(p, samples)
    }

    fn bronx(units: crate::view::DisplayUnits, voice: Voice) -> ViewInput {
        let mut input = input(units, voice);
        input.snapshot.spot.name = "Bronx River".into();
        let p = prov(
            SourceId::USGS,
            "01302020",
            "Bronx River at NY Botanical Garden",
            StationKind::UsgsSite(UsgsSiteType::Stream),
            ValueKind::Observed,
            Quality::Provisional,
        );
        input.snapshot.water.turbidity =
            Some(turbidity(p, TurbidityUnit::Fnu, &[2.4, 2.3, 2.2, 2.1]));
        input
    }

    fn card(input: &ViewInput) -> ClarityCard {
        build(input).expect("a clarity card")
    }

    #[test]
    fn bronx_river_turbidity_is_in_fnu_in_every_unit_system() {
        for units in [us(), metric()] {
            let c = card(&bronx(units, Voice::Plain));
            assert_eq!(c.full.turbidity.as_ref().unwrap().full(), "2.1 FNU");
            assert_eq!(c.compact.headline.full(), "2.1 FNU");
            assert_eq!(c.full.description.as_deref(), Some("Slightly turbid"));
            assert_eq!(c.chrome.notices.len(), 1);
        }
    }

    #[test]
    fn buoy_turbidity_stays_in_ftu() {
        let mut input = bronx(us(), Voice::Plain);
        let buoy = prov(
            SourceId::NDBC,
            "44025",
            "Long Island buoy",
            StationKind::Buoy,
            ValueKind::Observed,
            Quality::Approved,
        );
        input.snapshot.water.turbidity = Some(turbidity(buoy, TurbidityUnit::Ftu, &[1.2, 1.4]));
        let c = card(&input);
        assert_eq!(c.full.turbidity.unwrap().full(), "1.4 FTU");
        assert_eq!(c.full.chart.unwrap().unit, "FTU");
    }

    #[test]
    fn kd490_is_unavailable_without_a_satellite_value() {
        let c = card(&bronx(us(), Voice::Plain));
        assert!(!c.full.kd490.available);
        assert!(c.full.kd490.value.is_none());
        assert_eq!(
            c.full.kd490.note,
            "Unavailable: no current satellite source"
        );
    }

    #[test]
    fn chlorophyll_and_cyanobacteria_are_shown_when_present() {
        let mut input = bronx(us(), Voice::Plain);
        let p = prov(
            SourceId::new("cyan"),
            "lake",
            "Lake",
            StationKind::Buoy,
            ValueKind::Satellite,
            Quality::Approved,
        );
        input.snapshot.water.chlorophyll_ug_l = Some(run(p.clone(), NOW, 900, &[2.5]));
        input.snapshot.water.cyanobacteria_cells_ml = Some(run(p, NOW, 900, &[1500.0]));
        let c = card(&input);
        assert_eq!(c.full.chlorophyll.as_ref().unwrap().full(), "2.5 µg/L");
        assert_eq!(c.full.sections[0].rows[0].value, "1500 cells/mL");
    }

    #[test]
    fn no_clarity_data_no_card() {
        assert!(build(&input(us(), Voice::Plain)).is_none());
    }

    #[test]
    fn snapshots_in_both_voices_and_unit_systems() {
        insta::assert_json_snapshot!("bronx_luna_us", card(&bronx(us(), Voice::Luna)));
        insta::assert_json_snapshot!("bronx_plain_metric", card(&bronx(metric(), Voice::Plain)));
    }
}
