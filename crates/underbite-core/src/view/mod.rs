//! The view model: every value formatted for display, the only place that converts units.
//!
//! Types here are the contract between the core and the displays (the terminal and, over `UniFFI`,
//! Swift). Every user-visible value is preformatted text or a [`Quantity`]; raw numbers appear only
//! where a chart needs them, already in display units. The public types avoid generics, tuples
//! and maps so they cross the foreign-function bridge as plain records and enums.

pub mod cards;
pub mod chart;
pub mod format;
pub mod locale;
pub mod log;
#[cfg(any(test, debug_assertions))]
pub mod packs;
#[cfg(test)]
mod pseudo_tests;
pub mod species;
pub mod stations;
pub mod strings;
pub mod timeline;
pub mod trip;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use whether_core::presentation::TimeFormat;

use crate::model::{Measurement, SpotSnapshot, WaterType};
use crate::species::profile::SpeciesProfile;
use crate::time::UnixTime;
use crate::trip::TripLimits;
use crate::trip::windows::WindowParams;
use crate::units::{DischargeUnit, DistanceUnit, HeightUnit, PressureUnit, SpeedUnit, TempUnit};
use locale::Locale;

use cards::{
    alerts::AlertsCard, bloom::BloomCard, clarity::ClarityCard, current::CurrentCard,
    flow::FlowCard, go_no_go::GoNoGoCard, marine_forecast::MarineForecastCard, now::NowCard,
    pressure::PressureCard, solunar::SolunarCard, sources::SourcesCard, species::SpeciesCard,
    stocking::StockingCard, storm_ahead::StormAheadCard, sun_moon::SunMoonCard, tide::TideCard,
    water_temp::WaterTempCard, waves::WavesCard, wind::WindCard, windows::WindowsCard,
};

// ---------------------------------------------------------------------------------------------
// Input
// ---------------------------------------------------------------------------------------------

/// The words of the playful labels; safety text ignores it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Voice {
    #[default]
    Luna,
    Plain,
}

/// The unit the user reads each kind of value in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DisplayUnits {
    /// Water temperature.
    pub temperature: TempUnit,
    /// Air temperature.
    pub air_temperature: TempUnit,
    /// Tide heights and depths.
    pub height: HeightUnit,
    /// River stage (gage height) and flood stages.
    pub stage: HeightUnit,
    /// Lake and reservoir levels.
    pub lake_level: HeightUnit,
    /// Wave heights.
    pub waves: HeightUnit,
    pub discharge: DischargeUnit,
    pub current_speed: SpeedUnit,
    pub wind_speed: SpeedUnit,
    pub pressure: PressureUnit,
    /// Station distances.
    pub distance: DistanceUnit,
}

/// How far the timeline and the charts reach: a parameter, not a constant (phones and tablets
/// ask for different spans).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TimelineSpan {
    /// Hours covered.
    pub hours: u16,
    /// Minutes between points.
    pub step_minutes: u16,
}

/// The user's distance and age limits for one measurement (canonical units: metres, seconds).
/// A value beyond either is greyed with the reason.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MeasurementLimit {
    pub measurement: Measurement,
    pub max_distance_m: Option<f32>,
    pub max_age_s: Option<i64>,
}

/// The user's ranges for the spot's gauge, as discharge in m³/s.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RiverRanges {
    pub wadeable_below_m3s: Option<f32>,
    pub best_min_m3s: Option<f32>,
    pub best_max_m3s: Option<f32>,
    pub blown_out_above_m3s: Option<f32>,
}

/// Solunar display choices (the periods themselves are in the snapshot).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SolunarSettings {
    /// Hidden periods are not built.
    pub show: bool,
}

/// Everything `build_view` needs.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewInput {
    pub snapshot: SpotSnapshot,
    pub now: UnixTime,
    pub units: DisplayUnits,
    pub voice: Voice,
    pub clock: TimeFormat,
    /// The words and the number and date conventions; given by the caller, never read from the
    /// system.
    pub locale: Locale,
    pub timeline_span: TimelineSpan,
    /// Cards to show, in order, for the spot's water type.
    pub card_order: Vec<CardId>,
    pub limits: Vec<MeasurementLimit>,
    pub river_ranges: Option<RiverRanges>,
    /// Water temperature above which trout fishing is discouraged, °C.
    pub trout_stress_c: Option<f32>,
    pub solunar: SolunarSettings,
    /// What fishing windows are computed with (the configuration's `[windows]`).
    pub windows: WindowParams,
    /// The limits the go/no-go card judges against (the configuration's first `[[limits]]`
    /// profile); the card is not built without them.
    pub trip_limits: Option<TripLimits>,
}

impl ViewInput {
    /// The user's limit for a measurement, if they set one.
    pub fn limit(&self, measurement: Measurement) -> Option<&MeasurementLimit> {
        self.limits.iter().find(|l| l.measurement == measurement)
    }
}

// ---------------------------------------------------------------------------------------------
// Output: the spot
// ---------------------------------------------------------------------------------------------

/// A displayed value: the number in display units, its text and its unit symbol.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Quantity {
    pub value: Option<f32>,
    /// The display text without the unit, or the missing placeholder.
    pub text: String,
    /// The unit symbol, empty when unitless.
    pub unit: String,
}

/// The conditions page of one spot.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct SpotView {
    pub header: SpotHeader,
    /// In the user's order.
    pub cards: Vec<Card>,
    /// Filled when the day timeline is built.
    pub timeline: Option<TimelineVm>,
    /// Page-level messages, such as "no data for this spot yet".
    pub notices: Vec<SpotNotice>,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct SpotHeader {
    pub name: String,
    pub water_type: WaterType,
    /// "River or stream", "Tidal river or estuary", ...
    pub water_type_label: String,
    /// When the snapshot was fetched, unix seconds.
    pub updated_at: i64,
    /// "updated 6 min ago"
    pub updated: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct SpotNotice {
    pub text: String,
}

/// The cards of the dashboard, with the `snake_case` ids the configuration uses.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum CardId {
    Now,
    Tide,
    Current,
    Flow,
    WaterTemp,
    Clarity,
    Bloom,
    Waves,
    Wind,
    Pressure,
    SunMoon,
    Solunar,
    Windows,
    GoNoGo,
    StormAhead,
    MarineForecast,
    Stocking,
    Species,
    Alerts,
    Sources,
}

impl CardId {
    /// Every card, in the default order.
    pub const ALL: [Self; 20] = [
        Self::Now,
        Self::Tide,
        Self::Current,
        Self::Flow,
        Self::WaterTemp,
        Self::Clarity,
        Self::Bloom,
        Self::Waves,
        Self::Wind,
        Self::Pressure,
        Self::SunMoon,
        Self::Solunar,
        Self::Windows,
        Self::GoNoGo,
        Self::StormAhead,
        Self::MarineForecast,
        Self::Stocking,
        Self::Species,
        Self::Alerts,
        Self::Sources,
    ];
}

/// One card. Cards that a later phase adds are not variants yet.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
#[serde(tag = "card", rename_all = "snake_case")]
// Cards are built once per view and moved whole; boxing would only complicate the bridge.
#[allow(clippy::large_enum_variant)]
pub enum Card {
    Now(NowCard),
    Tide(TideCard),
    Current(CurrentCard),
    Flow(FlowCard),
    WaterTemp(WaterTempCard),
    Clarity(ClarityCard),
    Bloom(BloomCard),
    Waves(WavesCard),
    Wind(WindCard),
    Pressure(PressureCard),
    SunMoon(SunMoonCard),
    Solunar(SolunarCard),
    Windows(WindowsCard),
    GoNoGo(GoNoGoCard),
    MarineForecast(MarineForecastCard),
    Stocking(StockingCard),
    Species(SpeciesCard),
    StormAhead(StormAheadCard),
    Alerts(AlertsCard),
    Sources(SourcesCard),
}

impl Card {
    /// What every card shares.
    pub fn chrome(&self) -> &CardChrome {
        match self {
            Self::Now(c) => &c.chrome,
            Self::Tide(c) => &c.chrome,
            Self::Current(c) => &c.chrome,
            Self::Flow(c) => &c.chrome,
            Self::WaterTemp(c) => &c.chrome,
            Self::Clarity(c) => &c.chrome,
            Self::Bloom(c) => &c.chrome,
            Self::Waves(c) => &c.chrome,
            Self::Wind(c) => &c.chrome,
            Self::Pressure(c) => &c.chrome,
            Self::SunMoon(c) => &c.chrome,
            Self::Solunar(c) => &c.chrome,
            Self::Windows(c) => &c.chrome,
            Self::GoNoGo(c) => &c.chrome,
            Self::MarineForecast(c) => &c.chrome,
            Self::Stocking(c) => &c.chrome,
            Self::Species(c) => &c.chrome,
            Self::StormAhead(c) => &c.chrome,
            Self::Alerts(c) => &c.chrome,
            Self::Sources(c) => &c.chrome,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Output: what every card shares
// ---------------------------------------------------------------------------------------------

/// The frame of a card: title, provenance, credits, state, safety notices, accessibility text.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct CardChrome {
    pub id: CardId,
    pub title: String,
    /// The provenance line: "Water 54 °F · Hudson R at Battery, NY · 0.4 mi · 6 min ago".
    /// Cards that combine many sources leave it empty.
    pub footnote: Option<String>,
    pub credits: Vec<CreditVm>,
    pub state: CardState,
    pub notices: Vec<Notice>,
    /// What `VoiceOver` reads for the card: the plain meaning first.
    pub accessibility: String,
}

/// The credit a source is owed.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct CreditVm {
    /// The source's name as displayed.
    pub source: String,
    pub text: String,
    pub link: Option<String>,
    pub licence: Option<String>,
}

/// Whether the card's value can be trusted as current and local.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum CardState {
    Fresh,
    /// Older than the user's age limit; greyed.
    Stale {
        reason: String,
    },
    /// From a station beyond the user's distance limit; greyed.
    Far {
        reason: String,
    },
    Unavailable {
        reason: String,
    },
}

/// The fixed notices. Their text is plain and never in Luna's voice: there is no voice field.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NoticeKind {
    NotForNavigation,
    Modelled,
    Provisional,
    Approximate,
    TraditionalCue,
    TextOnly,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct Notice {
    pub kind: NoticeKind,
    pub text: String,
}

/// The compact form of a card: a headline and at most two short lines.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct Compact {
    pub headline: Quantity,
    /// At most two, not truncated or wrapped for any width.
    pub lines: Vec<String>,
    /// A sparkline in display units, oldest first; empty when the card has none.
    pub spark: Vec<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TrendDirection {
    Rising,
    Falling,
    Steady,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct TrendVm {
    pub direction: TrendDirection,
    /// "Rising 0.4 ft in 3 h"
    pub text: String,
}

/// A time with its label: "Sunrise" "6:42 AM".
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct TimedLabel {
    pub label: String,
    pub time: String,
    /// Unix seconds, for sorting and for drawing.
    pub t: i64,
}

/// A titled block of label and value rows in a full form.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct Section {
    pub title: String,
    pub rows: Vec<Row>,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct Row {
    pub label: String,
    pub value: String,
    /// A second, smaller line.
    pub detail: Option<String>,
}

/// An event table: one header, rows of cells in column order.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct Table {
    pub title: String,
    pub columns: Vec<String>,
    pub rows: Vec<TableRow>,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct TableRow {
    pub cells: Vec<String>,
}

// ---------------------------------------------------------------------------------------------
// Output: charts and the timeline
// ---------------------------------------------------------------------------------------------

/// A chart carries every point with its label; narrow layouts thin them by `major`.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct ChartVm {
    pub title: String,
    /// The y unit symbol, empty when mixed or unitless.
    pub unit: String,
    pub x_ticks: Vec<ChartTick>,
    pub series: Vec<ChartSeries>,
    /// Shaded spans: night, slack water, solunar periods.
    pub bands: Vec<ChartBand>,
    /// Horizontal lines: flood categories, the user's ranges.
    pub rules: Vec<ChartRule>,
    /// Labelled events: highs and lows, now.
    pub markers: Vec<ChartMarker>,
    /// Unix seconds of "now" when it falls inside the chart.
    pub now_t: Option<i64>,
    /// What `VoiceOver` reads: the shape and its extremes in words.
    pub accessibility: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct ChartTick {
    pub t: i64,
    pub label: String,
    pub major: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SeriesRole {
    Predicted,
    Observed,
    Forecast,
    Modelled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SeriesStyle {
    Line,
    Area,
    Bars,
    Dots,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct ChartSeries {
    pub label: String,
    pub role: SeriesRole,
    pub style: SeriesStyle,
    pub points: Vec<ChartPoint>,
}

/// One point: unix seconds, the value in display units, its label ("3.4 ft"), and whether a
/// narrow layout should keep its label.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct ChartPoint {
    pub t: i64,
    pub value: f32,
    pub label: String,
    pub major: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BandKind {
    Night,
    Slack,
    SolunarMajor,
    SolunarMinor,
    Highlight,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct ChartBand {
    pub kind: BandKind,
    pub start_t: i64,
    pub end_t: i64,
    pub label: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RuleKind {
    FloodCategory,
    UserRange,
    Threshold,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct ChartRule {
    pub kind: RuleKind,
    /// In display units.
    pub value: f32,
    pub label: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MarkerKind {
    High,
    Low,
    Slack,
    Sunrise,
    Sunset,
    Now,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct ChartMarker {
    pub kind: MarkerKind,
    pub t: i64,
    /// In display units, when the marker sits on the curve.
    pub value: Option<f32>,
    pub label: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LaneKind {
    Tide,
    Current,
    Wind,
    Pressure,
    Light,
    Solunar,
    Windows,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct TimelineLane {
    pub kind: LaneKind,
    pub label: String,
    pub chart: ChartVm,
    /// Sources of the lane's data: the timeline stands alone (iPad, widgets), away from the
    /// cards' credits.
    pub credits: Vec<CreditVm>,
    /// Notices the lane's data carries, such as "Not for navigation." on tides and currents.
    pub notices: Vec<Notice>,
}

/// The day timeline: lanes that share one time axis.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct TimelineVm {
    pub start_t: i64,
    pub end_t: i64,
    pub step_minutes: u16,
    pub x_ticks: Vec<ChartTick>,
    pub lanes: Vec<TimelineLane>,
}

// ---------------------------------------------------------------------------------------------
// Build
// ---------------------------------------------------------------------------------------------

/// Builds the page: each card in the user's order, skipping cards with nothing to show. No
/// species are chosen, so the species card is left out; see [`build_view_with_species`].
pub fn build_view(input: &ViewInput) -> SpotView {
    build_view_with_species(input, &[])
}

/// Builds the page with the species card for `species`: the chosen profiles with the user's
/// edits laid over them, in the user's order. With none the card is left out.
pub fn build_view_with_species(input: &ViewInput, species: &[SpeciesProfile]) -> SpotView {
    let snapshot = &input.snapshot;
    let cards = input
        .card_order
        .iter()
        .filter_map(|id| match id {
            CardId::Now => cards::now::build(input).map(Card::Now),
            CardId::Tide => cards::tide::build(input).map(Card::Tide),
            CardId::Current => cards::current::build(input).map(Card::Current),
            CardId::Flow => cards::flow::build(input).map(Card::Flow),
            CardId::WaterTemp => cards::water_temp::build(input).map(Card::WaterTemp),
            CardId::Clarity => cards::clarity::build(input).map(Card::Clarity),
            CardId::Bloom => cards::bloom::build(input).map(Card::Bloom),
            CardId::Waves => cards::waves::build(input).map(Card::Waves),
            CardId::Wind => cards::wind::build(input).map(Card::Wind),
            CardId::Pressure => cards::pressure::build(input).map(Card::Pressure),
            CardId::SunMoon => cards::sun_moon::build(input).map(Card::SunMoon),
            CardId::Solunar => cards::solunar::build(input).map(Card::Solunar),
            CardId::MarineForecast => {
                cards::marine_forecast::build(input).map(Card::MarineForecast)
            }
            CardId::StormAhead => cards::storm_ahead::build(input).map(Card::StormAhead),
            CardId::Alerts => cards::alerts::build(input).map(Card::Alerts),
            CardId::Sources => cards::sources::build(input).map(Card::Sources),
            CardId::Stocking => cards::stocking::build(input).map(Card::Stocking),
            CardId::Windows => cards::windows::build(input).map(Card::Windows),
            CardId::GoNoGo => cards::go_no_go::build(input).map(Card::GoNoGo),
            CardId::Species => cards::species::build(input, species).map(Card::Species),
        })
        .collect();
    let f = format::Formatter::for_input(input);
    SpotView {
        header: SpotHeader {
            name: snapshot.spot.name.clone(),
            water_type: snapshot.spot.water_type,
            water_type_label: f.water_type_label(snapshot.spot.water_type),
            updated_at: snapshot.fetched_at.0,
            updated: f.updated(snapshot.fetched_at, input.now),
        },
        cards,
        timeline: timeline::build(input),
        notices: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::snapshot::testdata::sample_snapshot;

    pub(crate) fn sample_input() -> ViewInput {
        let snapshot = sample_snapshot();
        ViewInput {
            now: snapshot.fetched_at,
            snapshot,
            units: DisplayUnits {
                temperature: TempUnit::F,
                air_temperature: TempUnit::F,
                height: HeightUnit::Ft,
                stage: HeightUnit::Ft,
                lake_level: HeightUnit::Ft,
                waves: HeightUnit::Ft,
                discharge: DischargeUnit::Cfs,
                current_speed: SpeedUnit::Kn,
                wind_speed: SpeedUnit::Mph,
                pressure: PressureUnit::InHg,
                distance: DistanceUnit::Mi,
            },
            voice: Voice::Luna,
            clock: TimeFormat::H12,
            locale: Locale::ENGLISH,
            timeline_span: TimelineSpan {
                hours: 24,
                step_minutes: 60,
            },
            card_order: CardId::ALL.to_vec(),
            limits: vec![MeasurementLimit {
                measurement: Measurement::Tide,
                max_distance_m: Some(50_000.0),
                max_age_s: Some(3 * 3600),
            }],
            river_ranges: None,
            trout_stress_c: Some(20.0),
            solunar: SolunarSettings { show: true },
            windows: WindowParams::default(),
            trip_limits: Some(TripLimits {
                name: "Kayak".into(),
                max_wind_kn: Some(12.0),
                max_gust_kn: Some(18.0),
                max_wave_m: Some(0.6),
                max_current_kn: Some(2.0),
                no_small_craft_advisory: true,
                no_lightning: true,
                ..TripLimits::default()
            }),
        }
    }

    #[test]
    fn card_ids_are_snake_case_and_complete() {
        let names: Vec<String> = CardId::ALL
            .iter()
            .map(|id| serde_json::to_string(id).unwrap().replace('"', ""))
            .collect();
        assert_eq!(names.len(), 20);
        assert_eq!(names[4], "water_temp");
        assert_eq!(names[5], "clarity");
        assert_eq!(names[6], "bloom");
        assert_eq!(names[14], "storm_ahead");
        assert_eq!(names[15], "marine_forecast");
        assert_eq!(names[16], "stocking");
    }

    #[test]
    fn view_has_the_header_and_cards_in_the_requested_order() {
        let input = sample_input();
        assert_eq!(
            input.limit(Measurement::Tide).unwrap().max_age_s,
            Some(10_800)
        );
        assert!(input.limit(Measurement::Waves).is_none());
        let view = build_view(&input);
        assert_eq!(view.header.name, input.snapshot.spot.name);
        // Cards appear only for ids in the requested order, in that order; each card's
        // content is tested with the card.
        let order: Vec<CardId> = view.cards.iter().map(|c| c.chrome().id).collect();
        let mut expected = input.card_order.clone();
        expected.retain(|id| order.contains(id));
        assert_eq!(order, expected);
        insta::assert_json_snapshot!("spot_view_header_from_sample", view.header);
    }

    #[test]
    fn schema_snapshot() {
        let schema = schemars::schema_for!(SpotView);
        insta::assert_json_snapshot!(schema);
    }
}
