//! Water temperature and its trend.

use schemars::JsonSchema;
use serde::Serialize;

use crate::model::{Measurement, WaterType};
use crate::provenance::{Provenance, Quality};
use crate::series::Series;
use crate::time::UnixTime;
use crate::view::cards::flow::{
    card_state, chart_series, chart_vm, credits, is_unreliable, observed_after, quality_notices,
    row, sample_quality, spark, trend, unreliable_text,
};
use crate::view::format::{Formatter, number};
use crate::view::{
    CardChrome, CardId, ChartRule, ChartVm, Compact, Quantity, Row, RuleKind, Section, SeriesRole,
    TrendVm, ViewInput,
};

const WEEK_S: i64 = 7 * 86_400;
/// Water temperature counts as moving beyond this change over the trend window, °C.
const TEMP_TREND_C: f32 = 0.2;

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct WaterTempCard {
    pub chrome: CardChrome,
    pub compact: Compact,
    pub full: WaterTempFull,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TempBasis {
    /// A gauge or buoy thermometer.
    Measured,
    /// Sea surface temperature from a model or satellite.
    SeaSurface,
}

/// The trout-stress guidance against the user's threshold.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct TroutStressVm {
    pub exceeded: bool,
    pub threshold: Quantity,
    /// "Consider not fishing for trout above 68 °F."
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct WaterTempFull {
    pub value: Option<Quantity>,
    pub basis: TempBasis,
    pub trend: Option<TrendVm>,
    pub trout_stress: Option<TroutStressVm>,
    pub chart: Option<ChartVm>,
    pub sections: Vec<Section>,
}

/// Fresh water, where trout live.
const fn is_fresh(t: WaterType) -> bool {
    matches!(
        t,
        WaterType::RiverStream
            | WaterType::LakeReservoir
            | WaterType::Pond
            | WaterType::GreatLakes
            | WaterType::Ice
    )
}

#[allow(clippy::too_many_lines)]
pub fn build(input: &ViewInput) -> Option<WaterTempCard> {
    let w = &input.snapshot.water;
    let m = &input.snapshot.marine;
    let f = Formatter::for_input(input);
    let now = input.now;
    let from = UnixTime(now.0 - WEEK_S);

    let measured = w.temperature_c.as_ref().filter(|s| !s.is_empty());
    let sst = m.sea_surface_temp_c.as_ref().filter(|s| !s.is_empty());
    let sst_forecast = m
        .sea_surface_temp_forecast_c
        .as_ref()
        .filter(|s| !s.is_empty());
    let (primary, basis): (&Series<f32>, TempBasis) = match (measured, sst, sst_forecast) {
        (Some(s), _, _) => (s, TempBasis::Measured),
        (None, Some(s), _) | (None, None, Some(s)) => (s, TempBasis::SeaSurface),
        (None, None, None) => return None,
    };
    let latest = primary.latest()?;
    let quality = sample_quality(primary, latest);
    let show = |c: f32| f.temperature(Some(c));
    let value = show(latest.value);
    let forecast_only = measured.is_none() && sst.is_none();

    // Trend over observed values; a forecast-only value has none.
    let trend_vm: Option<TrendVm> = if forecast_only {
        None
    } else {
        trend(&f, primary.samples(), TEMP_TREND_C, &|d| {
            // A change is scaled, not offset.
            let unit = f.units.temperature;
            format!(
                "{} {}",
                number(unit.from_canonical(d) - unit.from_canonical(0.0), 1),
                unit.symbol()
            )
        })
    };

    // Trout stress applies to fresh water and to thermometer readings.
    let trout_stress = input
        .trout_stress_c
        .filter(|_| basis == TempBasis::Measured && is_fresh(input.snapshot.spot.water_type))
        .map(|limit| {
            let threshold = show(limit);
            TroutStressVm {
                exceeded: latest.value > limit && !is_unreliable(quality),
                text: format!("Consider not fishing for trout above {}.", threshold.full()),
                threshold,
            }
        });

    // Chart: the week observed, a forecast kept apart as its own series.
    let mut series = Vec::new();
    let pts = observed_after(primary, from);
    if !pts.is_empty() {
        let (label, role) = if forecast_only {
            ("Forecast sea surface (modelled)", SeriesRole::Modelled)
        } else if basis == TempBasis::Measured {
            ("Observed", SeriesRole::Observed)
        } else {
            ("Observed sea surface", SeriesRole::Observed)
        };
        series.push(chart_series(label, role, pts, &show));
    }
    if basis == TempBasis::SeaSurface
        && !forecast_only
        && let Some(s) = sst_forecast
    {
        let pts = observed_after(s, now);
        if !pts.is_empty() {
            series.push(chart_series(
                "Forecast sea surface (modelled)",
                SeriesRole::Modelled,
                pts,
                &show,
            ));
        }
    }
    let mut rules = Vec::new();
    if let Some(t) = &trout_stress
        && let Some(v) = t.threshold.value
    {
        rules.push(ChartRule {
            kind: RuleKind::Threshold,
            value: v,
            label: format!("Trout stress {}", t.threshold.full()),
        });
    }
    let chart = chart_vm(
        &f,
        "Water temperature",
        f.temperature(None).unit,
        series,
        rules,
        now,
    );

    // Sections: sea surface observed and forecast, never merged.
    let mut sections = Vec::new();
    let mut rows: Vec<Row> = Vec::new();
    if basis == TempBasis::Measured
        && let Some(l) = sst.and_then(Series::latest)
    {
        rows.push(row("Sea surface, observed", &show(l.value).full()));
    }
    if let Some(s) = sst_forecast.filter(|_| !forecast_only)
        && let Some(v) = s.at(now, std::time::Duration::from_hours(6))
    {
        rows.push(row("Sea surface, forecast (modelled)", &show(v).full()));
    }
    if !rows.is_empty() {
        sections.push(Section {
            title: "Sea surface".to_owned(),
            rows,
        });
    }
    if is_unreliable(quality) {
        sections.push(Section {
            title: "Reading quality".to_owned(),
            rows: vec![row("Reliability", unreliable_text(quality))],
        });
    }

    // Compact.
    let prov: &Provenance = primary.provenance();
    let mut lines: Vec<String> = Vec::new();
    if is_unreliable(quality) {
        lines.push(unreliable_text(quality).to_owned());
    }
    if let Some(t) = trout_stress.as_ref().filter(|t| t.exceeded) {
        lines.push(t.text.clone());
    }
    lines.extend(trend_vm.as_ref().map(|t| t.text.clone()));
    lines.push(f.station_footer(prov).text);
    lines.truncate(2);
    let spark_values: Vec<f32> = observed_after(primary, from)
        .iter()
        .map(|s| show(s.value).value.unwrap_or(0.0))
        .collect();

    let measurement = match basis {
        TempBasis::Measured => Measurement::WaterTemperature,
        TempBasis::SeaSurface if forecast_only => Measurement::SeaSurfaceTempForecast,
        TempBasis::SeaSurface => Measurement::SeaSurfaceTemp,
    };
    let title = f.strings.card_title(CardId::WaterTemp).to_owned();
    let mut accessibility = format!("{title}: {}", value.full());
    if let Some(t) = &trend_vm {
        accessibility.push_str(", ");
        accessibility.push_str(&t.text);
    }
    if let Some(t) = trout_stress.as_ref().filter(|t| t.exceeded) {
        accessibility.push_str(". ");
        accessibility.push_str(&t.text);
    }
    if is_unreliable(quality) {
        accessibility.push_str(". ");
        accessibility.push_str(unreliable_text(quality));
    }
    let qualities: Vec<Quality> = vec![quality];

    Some(WaterTempCard {
        chrome: CardChrome {
            id: CardId::WaterTemp,
            title,
            footnote: Some(f.footnote(&format!("Water {}", value.full()), prov, now)),
            credits: credits(input, &[prov]),
            state: card_state(input, &f, measurement, prov, Some(latest.time)),
            notices: quality_notices(&f, &qualities),
            accessibility,
        },
        compact: Compact {
            headline: value.clone(),
            lines,
            spark: spark(&spark_values),
        },
        full: WaterTempFull {
            value: Some(value),
            basis,
            trend: trend_vm,
            trout_stress,
            chart,
            sections,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provenance::{SourceId, StationKind, ValueKind};
    use crate::view::Voice;
    use crate::view::cards::flow::fixtures::{NOW, croton, input, metric, prov, ramp, run, us};

    fn river(units: crate::view::DisplayUnits, voice: Voice, from_c: f32, to_c: f32) -> ViewInput {
        let mut input = input(units, voice);
        input.snapshot.water.temperature_c = Some(run(
            croton(Quality::Approved),
            NOW,
            900,
            &ramp(from_c, to_c, 25),
        ));
        input
    }

    fn card(input: &ViewInput) -> WaterTempCard {
        build(input).expect("a water temperature card")
    }

    #[test]
    fn trout_note_appears_above_the_threshold_in_the_users_unit() {
        let c = card(&river(us(), Voice::Plain, 19.0, 21.0));
        let t = c.full.trout_stress.as_ref().unwrap();
        assert!(t.exceeded);
        assert_eq!(t.text, "Consider not fishing for trout above 68 °F.");
        assert_eq!(c.compact.lines[0], t.text);
        assert_eq!(c.full.value.as_ref().unwrap().full(), "70 °F");
        assert_eq!(c.full.trend.as_ref().unwrap().text, "Rising 1.8 °F in 3 h");
        assert_eq!(
            c.full.chart.as_ref().unwrap().rules[0].label,
            "Trout stress 68 °F"
        );

        let c = card(&river(metric(), Voice::Plain, 19.0, 21.0));
        let t = c.full.trout_stress.as_ref().unwrap();
        assert_eq!(t.text, "Consider not fishing for trout above 20.0 °C.");
        assert_eq!(c.full.trend.as_ref().unwrap().text, "Rising 1.0 °C in 3 h");
    }

    #[test]
    fn trout_note_is_quiet_below_the_threshold_and_off_without_one() {
        let c = card(&river(us(), Voice::Plain, 12.0, 13.0));
        assert!(!c.full.trout_stress.as_ref().unwrap().exceeded);
        assert!(c.compact.lines.iter().all(|l| !l.contains("trout")));
        let mut input = river(us(), Voice::Plain, 19.0, 21.0);
        input.trout_stress_c = None;
        assert!(card(&input).full.trout_stress.is_none());
    }

    #[test]
    fn tidal_water_has_no_trout_note() {
        let mut input = river(us(), Voice::Plain, 19.0, 21.0);
        input.snapshot.spot.water_type = WaterType::TidalRiverEstuary;
        assert!(card(&input).full.trout_stress.is_none());
    }

    #[test]
    fn sea_surface_observed_and_forecast_stay_apart() {
        let mut input = input(us(), Voice::Plain);
        let buoy = prov(
            SourceId::NDBC,
            "44025",
            "Long Island buoy",
            StationKind::Buoy,
            ValueKind::Observed,
            Quality::Approved,
        );
        let model = prov(
            SourceId::OPEN_METEO_MARINE,
            "grid",
            "Open-Meteo Marine",
            StationKind::Buoy,
            ValueKind::Modelled,
            Quality::Approved,
        );
        input.snapshot.spot.water_type = WaterType::BayInshore;
        input.snapshot.marine.sea_surface_temp_c =
            Some(run(buoy, NOW, 3_600, &ramp(14.0, 15.0, 7)));
        input.snapshot.marine.sea_surface_temp_forecast_c =
            Some(run(model, NOW + 6 * 3_600, 3_600, &ramp(15.0, 16.0, 7)));
        let c = card(&input);
        assert_eq!(c.full.basis, TempBasis::SeaSurface);
        assert!(c.full.trout_stress.is_none());
        let chart = c.full.chart.as_ref().unwrap();
        let roles: Vec<SeriesRole> = chart.series.iter().map(|s| s.role).collect();
        assert_eq!(roles, [SeriesRole::Observed, SeriesRole::Modelled]);
        assert_eq!(c.full.value.as_ref().unwrap().full(), "59 °F");

        // With only the forecast the value is modelled and has no trend.
        input.snapshot.marine.sea_surface_temp_c = None;
        let c = card(&input);
        assert!(c.full.trend.is_none());
        assert_eq!(c.full.chart.unwrap().series[0].role, SeriesRole::Modelled);
    }

    #[test]
    fn ice_affected_temperature_is_marked_unreliable() {
        let mut input = river(us(), Voice::Plain, 19.0, 21.0);
        input.snapshot.water.temperature_c = Some(run(
            croton(Quality::IceAffected),
            NOW,
            900,
            &ramp(19.0, 21.0, 25),
        ));
        let c = card(&input);
        assert_eq!(c.compact.lines[0], "Affected by ice: reading unreliable");
        assert!(!c.full.trout_stress.unwrap().exceeded);
    }

    #[test]
    fn no_temperature_no_card() {
        assert!(build(&input(us(), Voice::Plain)).is_none());
    }

    #[test]
    fn snapshots_in_both_voices_and_unit_systems() {
        insta::assert_json_snapshot!("river_luna_us", card(&river(us(), Voice::Luna, 19.0, 21.0)));
        insta::assert_json_snapshot!(
            "river_plain_metric",
            card(&river(metric(), Voice::Plain, 12.0, 13.0))
        );
    }
}
