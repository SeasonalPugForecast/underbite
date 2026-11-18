//! The words the view shows, in both voices.
//!
//! One table holds every fixed user-visible text. Playful texts ([`Key`]) have a plain and a Luna
//! variant; safety texts ([`SafetyKey`]) have exactly one, and no function takes a voice for them:
//! the split is by type, so a safety text cannot be given a Luna variant by accident.
//!
//! The table is English. A [`Strings`] carries the [`Locale`] of the view input: its language
//! picks the words (English, or the pseudo-locale of [`crate::view::packs`] in tests and debug
//! builds), its number and date conventions the way values are written. Sentences with values
//! are templates with `{name}` placeholders, never pieces glued in English word order.

use whether_core::time::Weekday;

use crate::alerts::AlertMetric;
use crate::datum::VerticalDatum;
use crate::discover::hydro::{WaterTypeReason, WaterbodyKind};
use crate::map::breaks::BreakKind;
use crate::map::catalogue::{DateMode, Group, UnavailableReason};
use crate::map::dem::{ApproxDepth, DepthReading};
use crate::map::enc::{SourceDate, StructureKind};
use crate::map::pack::{PlanError, SkipReason};
use crate::model::nearby::{AccessKind, AccessLevel};
use crate::model::{
    AlertSeverity, Measurement, MoonPhase, PrincipalPhase, TideKind, TurbidityUnit, WaterType,
    ZoneKind,
};
use crate::provenance::{Relation, SourceId, UsgsSiteType};
use crate::time::{LocalDate, UnixTime, UtcOffset};
use crate::units::{DischargeUnit, DistanceUnit, HeightUnit, PressureUnit, SpeedUnit, TempUnit};
use crate::view::format::{Formatter, distance_to_metres};
use crate::view::locale::{DateOrder, Language, Locale};
use crate::view::{CardId, NoticeKind, Voice};

/// Declares [`Key`], its list and its table in one place.
macro_rules! keys {
    ($($(#[$doc:meta])* $name:ident => ($plain:expr, $luna:expr $(,)?),)*) => {
        /// Texts that may be playful. Each has a plain and a Luna variant (often the same).
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum Key {
            $($(#[$doc])* $name,)*
        }

        impl Key {
            /// Every key, for tests and for a later localisation.
            pub const ALL: &'static [Self] = &[$(Self::$name,)*];

            /// The English `(plain, luna)`.
            pub(in crate::view) const fn english(self) -> (&'static str, &'static str) {
                match self {
                    $(Self::$name => ($plain, $luna),)*
                }
            }
        }
    };
}

keys! {
    /// `{n}` minutes, hours or days.
    UpdatedJustNow => ("updated just now", "updated just now"),
    UpdatedMinutes => ("updated {n} min ago", "updated {n} min ago"),
    UpdatedHours => ("updated {n} h ago", "updated {n} h ago"),
    UpdatedDays => ("updated {n} d ago", "updated {n} d ago"),
    AgeJustNow => ("just now", "just now"),
    AgeMinutes => ("{n} min ago", "{n} min ago"),
    AgeHours => ("{n} h ago", "{n} h ago"),
    AgeDays => ("{n} d ago", "{n} d ago"),
    /// `{station}`, `{distance}`.
    StationFooter => (
                "{station}, {distance}",
                "Luna's watching {station}, {distance}",
            ),
    /// `{reason}`.
    GoNoGoBad => ("No-go: {reason}", "Luna's not impressed: {reason}"),
    /// `{reason}`.
    GoNoGoMarginal => ("Marginal: {reason}", "Luna's wary: {reason}"),
    GoNoGoGood => ("Good conditions.", "Luna approves."),
    GoNoGoUnknown => ("Not enough data to judge", "Not enough data to judge"),
    /// `{checks}`: the checks that had no data.
    GoNoGoMissing => ("Missing: {checks}", "Missing: {checks}"),
    /// A value over a greatest limit: `{what}`, `{measured}`, `{limit}`.
    GoNoGoOver => (
        "{what} {measured} over your {limit} limit",
        "{what} to {measured}, your limit is {limit}",
    ),
    /// A value under a least limit.
    GoNoGoUnder => (
        "{what} {measured} under your {limit} minimum",
        "{what} down to {measured}, your minimum is {limit}",
    ),
    /// A value within 10 % of a greatest limit.
    GoNoGoNearOver => (
        "{what} {measured} is within 10 % of your {limit} limit",
        "{what} {measured}, close to your limit of {limit}",
    ),
    /// A value within 10 % of a least limit.
    GoNoGoNearUnder => (
        "{what} {measured} is within 10 % of your {limit} minimum",
        "{what} {measured}, close to your minimum of {limit}",
    ),
    GoNoGoBeyondForecast => (
        "Past the end of the forecast",
        "Past the end of the forecast",
    ),
    GoNoGoDayOver => ("This day is over", "This day is over"),
    /// `{day}`, `{summary}`.
    GoNoGoDayLine => ("{day}: {summary}", "{day}: {summary}"),
    VerdictGo => ("Go", "Go"),
    VerdictMarginal => ("Marginal", "Marginal"),
    VerdictUnknown => ("Unknown", "Unknown"),
    CheckForecast => ("forecast", "forecast"),
    CheckWind => ("wind", "wind"),
    CheckGust => ("gusts", "gusts"),
    CheckWaves => ("waves", "waves"),
    CheckCurrent => ("current", "current"),
    CheckAirTemperature => ("air temperature", "air temperature"),
    CheckVisibility => ("visibility", "visibility"),
    CheckFlow => ("river flow", "river flow"),
    /// `{time}`.
    WindowNow => (
        "Fishing window now, until {time}",
        "Snack time now, until {time}",
    ),
    /// `{factors}`: the lower-case names of the factors with no data, joined with commas.
    WindowNoData => ("No data: {factors}", "No data: {factors}"),
    FactorTideStage => ("Tide stage", "Tide stage"),
    FactorTideIncoming => ("Incoming tide", "Incoming tide"),
    FactorTideOutgoing => ("Outgoing tide", "Outgoing tide"),
    FactorCurrentStrength => ("Current strength", "Current strength"),
    FactorLowLight => ("Low light", "Low light"),
    FactorPressureFalling => ("Falling pressure", "Falling pressure"),
    FactorWaterTemp => ("Water temperature in range", "Water temperature in range"),
    FactorFlowInRange => ("River flow in range", "River flow in range"),
    /// The two range factors as plain names, for the list of factors with no data.
    FactorWaterTempName => ("Water temperature", "Water temperature"),
    FactorFlowName => ("River flow", "River flow"),
    /// `{duration}`, `{kind}` (high or low), `{time}`.
    WindowTideNext => (
        "{duration} before the {kind} at {time}",
        "{duration} before the {kind} at {time}",
    ),
    /// `{speed}`.
    WindowCurrentFlood => ("{speed}, flooding", "{speed}, flooding"),
    WindowCurrentEbb => ("{speed}, ebbing", "{speed}, ebbing"),
    /// `{duration}`, `{event}` (sunrise or sunset), `{time}`.
    WindowLightBefore => (
        "{duration} before {event} at {time}",
        "{duration} before {event} at {time}",
    ),
    WindowLightAfter => (
        "{duration} after {event} at {time}",
        "{duration} after {event} at {time}",
    ),
    /// `{label}` (the period's name), `{time}`.
    WindowSolunar => ("{label} around {time}", "{label} around {time}"),
    /// `{amount}`, `{hours}`.
    WindowPressure => (
        "down {amount} in {hours} h",
        "down {amount} in {hours} h",
    ),
    /// `{value}`, `{min}`, `{max}`.
    WindowInRange => (
        "{value}, inside {min} to {max}",
        "{value}, inside {min} to {max}",
    ),
    /// `{range}`.
    NextWindow => ("Next fishing window {range}", "Next snack time {range}"),
    NoWindows => (
                "No fishing windows in this period.",
                "No snack times in this period.",
            ),
    NoData => ("No data for this spot yet.", "Luna found nothing here yet."),
    NoCards => ("Nothing to show.", "Luna has nothing to show."),
    Loading => ("Loading", "Luna is paddling over"),
    Flood => ("Flood", "Flood"),
    Ebb => ("Ebb", "Ebb"),
    Slack => ("Slack", "Slack"),
    Rising => ("Rising", "Rising"),
    Falling => ("Falling", "Falling"),
    Steady => ("Steady", "Steady"),
    /// "Rising 0.4 ft in 3 h": `{direction}`, `{amount}`, `{hours}`.
    TrendOver => (
                "{direction} {amount} in {hours} h",
                "{direction} {amount} in {hours} h",
            ),
    /// `{distance}`: "4.0 mi".
    Upstream => ("{distance} upstream", "{distance} upstream"),
    Downstream => ("{distance} downstream", "{distance} downstream"),
    Modelled => ("modelled", "modelled"),
    Satellite => ("satellite", "satellite"),
    /// `{distance}`, `{limit}`: why a value is greyed.
    FarReason => (
                "{distance} from your spot, beyond your {limit} limit",
                "{distance} from your spot, beyond your {limit} limit",
            ),
    /// `{age}`, `{limit}`.
    StaleReason => (
                "{age} old, beyond your {limit} limit",
                "{age} old, beyond your {limit} limit",
            ),
    PercentileMuchBelow => ("much below normal", "much below normal"),
    PercentileBelow => ("below normal", "below normal"),
    PercentileNormal => ("normal", "normal"),
    PercentileAbove => ("above normal", "above normal"),
    PercentileMuchAbove => ("much above normal", "much above normal"),
    /// `{band}`, `{rank}` (a rank text), `{date}`.
    PercentileText => ("{band}, {rank} for {date}", "{band}, {rank} for {date}"),
    /// `{band}`, `{date}`.
    PercentileNoRank => ("{band} for {date}", "{band} for {date}"),
    /// `{ordinal}`.
    PercentileRank => ("{ordinal} percentile", "{ordinal} percentile"),
    PercentileRankBelow => (
                "below the {ordinal} percentile",
                "below the {ordinal} percentile",
            ),
    PercentileRankAbove => (
                "above the {ordinal} percentile",
                "above the {ordinal} percentile",
            ),
    /// `{date}`.
    PercentileRecordLow => ("record low for {date}", "record low for {date}"),
    PercentileRecordHigh => ("record high for {date}", "record high for {date}"),
    WaterRiver => ("River or stream", "River or stream"),
    WaterLake => ("Lake or reservoir", "Lake or reservoir"),
    WaterTidal => ("Tidal river or estuary", "Tidal river or estuary"),
    WaterBay => ("Bay or inshore", "Bay or inshore"),
    WaterSurf => ("Surf", "Surf"),
    WaterNearshore => ("Nearshore", "Nearshore"),
    WaterOffshore => ("Offshore", "Offshore"),
    WaterGreatLakes => ("Great Lakes", "Great Lakes"),
    WaterPond => ("Pond", "Pond"),
    WaterIce => ("Ice fishing", "Ice fishing"),
    Water => ("Water", "Water"),
    Station => ("Station", "Station"),
    Heights => ("Heights", "Heights"),
    /// The unit pickers' group labels (see `Strings::unit_group`).
    UnitGroupDischarge => ("River flow", "River flow"),
    UnitGroupHeight => ("Heights", "Heights"),
    UnitGroupStage => ("River stage", "River stage"),
    UnitGroupLakeLevel => ("Lake level", "Lake level"),
    UnitGroupTide => ("Tide height", "Tide height"),
    UnitGroupWaves => ("Wave height", "Wave height"),
    UnitGroupSpeed => ("Speeds", "Speeds"),
    UnitGroupCurrent => ("Current speed", "Current speed"),
    UnitGroupWind => ("Wind speed", "Wind speed"),
    UnitGroupWaterTemp => ("Water temperature", "Water temperature"),
    UnitGroupAirTemp => ("Air temperature", "Air temperature"),
    UnitGroupPressure => ("Air pressure", "Air pressure"),
    UnitGroupDistance => ("Distances", "Distances"),
    /// "Same as {group}": a unit picker's choice to follow another group; `{group}` is the group's
    /// name in lower case.
    UnitSameAs => ("Same as {group}", "Same as {group}"),
    /// The names of the source groups (see `Strings::source_group`).
    SourceGroupWaterTemperature => ("Water temperature", "Water temperature"),
    SourceGroupTide => ("Tides", "Tides"),
    SourceGroupCurrents => ("Currents", "Currents"),
    SourceGroupFlow => ("River flow", "River flow"),
    SourceGroupLakeLevel => ("Lake level", "Lake level"),
    SourceGroupWaves => ("Waves", "Waves"),
    SourceGroupSst => ("Sea surface temperature", "Sea surface temperature"),
    SourceGroupClarity => ("Water clarity", "Water clarity"),
    SourceGroupWaterQuality => ("Water quality", "Water quality"),
    SourceGroupMarineForecast => ("Marine forecast", "Marine forecast"),
    SourceGroupWeather => ("Weather", "Weather"),
    SourceGroupStorm => ("Storms", "Storms"),
    /// `{distance}`: "25 km".
    SourceWithin => ("within {distance}", "within {distance}"),
    SourceAnyDistance => ("any distance", "any distance"),
    /// `{age}`: "3 h".
    SourceUnderOld => ("under {age} old", "under {age} old"),
    SourceAnyAge => ("any age", "any age"),
    /// `{distance}`, `{age}`: the two limits of a source group, as the lines above.
    SourceLimits => ("{distance}, {age}", "{distance}, {age}"),
    /// `{n}` limits set on a go/no-go profile.
    LimitCountOne => ("{n} limit", "{n} limit"),
    LimitCountMany => ("{n} limits", "{n} limits"),
    LimitNoSmallCraft => ("no small craft advisory", "no small craft advisory"),
    LimitNoLightning => ("no lightning", "no lightning"),
    LimitNone => ("No limits set", "No limits set"),
    /// `{low}`, `{high}`: the shaded band of a flow chart, "5th percentile to 95th percentile".
    PercentileBand => ("Shaded: {low} to {high}", "Shaded: {low} to {high}"),
    Depth => ("Depth", "Depth"),
    Night => ("Night", "Night"),
    Sunrise => ("Sunrise", "Sunrise"),
    Sunset => ("Sunset", "Sunset"),
    Predicted => ("Predicted", "Predicted"),
    PredictedEvents => ("Predicted events", "Predicted events"),
    Observed => ("Observed", "Observed"),
    ModelledSeaLevel => ("Modelled sea level", "Modelled sea level"),
    /// The headline of a tide card whose source gives times only.
    TimesOnly => ("times only", "times only"),
    TideHigh => ("High", "High"),
    TideLow => ("Low", "Low"),
    MaxFlood => ("Max flood", "Max flood"),
    MaxEbb => ("Max ebb", "Max ebb"),
    /// `{time}`.
    NextSlack => ("Next slack {time}", "Next slack {time}"),
    /// `{kind}` (a tide event name), `{time}`.
    NextEvent => ("Next {kind} {time}", "Next {kind} {time}"),
    /// `{datum}`: "above MLLW".
    AboveDatum => ("above {datum}", "above {datum}"),
    /// `{bin}`, `{depth}`.
    BinAt => ("Bin {bin} at {depth}", "Bin {bin} at {depth}"),
    /// `{bin}`.
    BinOnly => ("Bin {bin}", "Bin {bin}"),
    /// `{depth}`.
    DepthAt => ("At {depth}", "At {depth}"),
    /// `{line}`, `{direction}`.
    TowardLine => ("{line} toward {direction}", "{line} toward {direction}"),
    CurrentSpeed => ("Current speed", "Current speed"),
    /// `{event}`, `{time}`.
    EventAt => ("{event} at {time}", "{event} at {time}"),
    MoonNew => ("New moon", "New moon"),
    MoonWaxingCrescent => ("Waxing crescent", "Waxing crescent"),
    MoonFirstQuarter => ("First quarter", "First quarter"),
    MoonWaxingGibbous => ("Waxing gibbous", "Waxing gibbous"),
    MoonFull => ("Full moon", "Full moon"),
    MoonWaningGibbous => ("Waning gibbous", "Waning gibbous"),
    MoonLastQuarter => ("Last quarter", "Last quarter"),
    MoonWaningCrescent => ("Waning crescent", "Waning crescent"),
    WeekdayMon => ("Mon", "Mon"),
    WeekdayTue => ("Tue", "Tue"),
    WeekdayWed => ("Wed", "Wed"),
    WeekdayThu => ("Thu", "Thu"),
    WeekdayFri => ("Fri", "Fri"),
    WeekdaySat => ("Sat", "Sat"),
    WeekdaySun => ("Sun", "Sun"),
    NauticalDawn => ("Nautical dawn", "Nautical dawn"),
    CivilDawn => ("Civil dawn", "Civil dawn"),
    GoldenHourEnds => ("Golden hour ends", "Golden hour ends"),
    GoldenHourStarts => ("Golden hour starts", "Golden hour starts"),
    CivilDusk => ("Civil dusk", "Civil dusk"),
    NauticalDusk => ("Nautical dusk", "Nautical dusk"),
    SunStaysUp => ("Sun stays up all day", "Sun stays up all day"),
    SunStaysDown => ("Sun stays down all day", "Sun stays down all day"),
    DayLength => ("Day length", "Day length"),
    /// `{hours}`, `{minutes}` (two digits).
    DayLengthValue => ("{hours} h {minutes} min", "{hours} h {minutes} min"),
    Moonrise => ("Moonrise", "Moonrise"),
    Moonset => ("Moonset", "Moonset"),
    MoonOverhead => ("Moon overhead", "Moon overhead"),
    MoonUnderfoot => ("Moon underfoot", "Moon underfoot"),
    NoMoonrise => ("No moonrise today", "No moonrise today"),
    NoMoonset => ("No moonset today", "No moonset today"),
    NotToday => ("Not today", "Not today"),
    /// `{value}`: "62%".
    Lit => ("{value} lit", "{value} lit"),
    Sun => ("Sun", "Sun"),
    Moon => ("Moon", "Moon"),
    NoSunriseOrSunset => ("No sunrise or sunset today", "No sunrise or sunset today"),
    ComputedOnDevice => ("Computed on device", "Computed on device"),
    /// The same, lower case, after a lane title.
    ComputedOnDeviceInline => ("computed on device", "computed on device"),
    SunMoonCredit => (
        "Sun and moon positions from astronomical algorithms",
        "Sun and moon positions from astronomical algorithms",
    ),
    /// The name of a major solunar period; Luna calls it a big nibble.
    SolunarMajor => ("Major", "Big nibble"),
    SolunarMinor => ("Minor", "Small nibble"),
    /// `{label}` (the period's name), `{time}`.
    SolunarNow => ("{label} now, until {time}", "{label} now, until {time}"),
    SolunarNextMajor => ("Next major", "Next major"),
    SolunarNextMinor => ("Next minor", "Next minor"),
    SolunarNone => (
        "no more periods in the forecast",
        "no more periods in the forecast",
    ),
    /// The headline when a solunar period is under way.
    NowText => ("now", "now"),
    UnitMinutes => ("min", "min"),
    UnitHours => ("h", "h"),
    Next => ("Next", "Next"),
    /// `{from}`, `{to}`.
    Range => ("{from} to {to}", "{from} to {to}"),
    AffectedByIce => (
        "Affected by ice: reading unreliable",
        "Affected by ice: reading unreliable",
    ),
    FlaggedBySource => (
        "Flagged by the source: reading unreliable",
        "Flagged by the source: reading unreliable",
    ),
    ProvisionalStatement => ("Provisional data statement", "Provisional data statement"),
    ChartNoData => ("no data", "no data"),
    /// `{title}`, `{now}`, `{high}`, `{low}`.
    ChartNowHighLow => (
        "{title}: now {now}, highest {high}, lowest {low}",
        "{title}: now {now}, highest {high}, lowest {low}",
    ),
    /// `{title}`, `{high}`, `{low}`.
    ChartHighLow => (
        "{title}: highest {high}, lowest {low}",
        "{title}: highest {high}, lowest {low}",
    ),
    /// `{lowest}`, `{lowest_time}`, `{highest}`, `{highest_time}`.
    ExtremesAt => (
        "lowest {lowest} at {lowest_time}, highest {highest} at {highest_time}.",
        "lowest {lowest} at {lowest_time}, highest {highest} at {highest_time}.",
    ),
    /// `{hours}`.
    SteadyOver => ("Steady over {hours} h", "Steady over {hours} h"),
    ClassRecordLow => ("Record low", "Record low"),
    ClassRecordHigh => ("Record high", "Record high"),
    BasisStatistics => ("USGS statistics (beta)", "USGS statistics (beta)"),
    /// `{years}`.
    ApprovedOnly => ("approved data only, {years}", "approved data only, {years}"),
    /// `{years}`.
    ComputedFromDaily => (
        "computed from {years} of daily values",
        "computed from {years} of daily values",
    ),
    /// `{n}`.
    YearOne => ("{n} year", "{n} year"),
    YearMany => ("{n} years", "{n} years"),
    RangeNoDischarge => (
        "No discharge to compare with your ranges",
        "No discharge to compare with your ranges",
    ),
    /// `{reason}`.
    RangesNotApplied => (
        "{reason}: your ranges are not applied",
        "{reason}: your ranges are not applied",
    ),
    /// `{value}`, `{unit}`.
    RangeBlownOut => (
        "Blown out (above {value} {unit})",
        "Blown out (above {value} {unit})",
    ),
    /// `{range}`.
    RangeBest => ("Best range ({range})", "Best range ({range})"),
    /// `{value}`, `{unit}`.
    RangeWadeable => (
        "Wadeable (below {value} {unit})",
        "Wadeable (below {value} {unit})",
    ),
    RangeFishable => (
        "Fishable (outside your best range)",
        "Fishable (outside your best range)",
    ),
    /// `{from}`, `{to}`, `{unit}`.
    RangeBetween => ("{from} to {to} {unit}", "{from} to {to} {unit}"),
    /// `{value}`, `{unit}`.
    RangeFrom => ("from {value} {unit}", "from {value} {unit}"),
    RangeUpTo => ("up to {value} {unit}", "up to {value} {unit}"),
    /// A range's rule on the flow chart: `{value}`, `{unit}`.
    RuleWadeableBelow => ("Wadeable below {value} {unit}", "Wadeable below {value} {unit}"),
    RuleBestFrom => ("Best from {value} {unit}", "Best from {value} {unit}"),
    RuleBestUpTo => ("Best up to {value} {unit}", "Best up to {value} {unit}"),
    RuleBlownOutAbove => ("Blown out above {value} {unit}", "Blown out above {value} {unit}"),
    FloodAction => ("Action", "Action"),
    FloodMinor => ("Minor", "Minor"),
    FloodModerate => ("Moderate", "Moderate"),
    FloodMajor => ("Major", "Major"),
    Discharge => ("Discharge", "Discharge"),
    Level => ("Level", "Level"),
    Stage => ("Stage", "Stage"),
    LakeLevel => ("Lake level", "Lake level"),
    FlowWord => ("Flow", "Flow"),
    Forecast => ("Forecast", "Forecast"),
    RowBasis => ("Basis", "Basis"),
    RowSample => ("Sample", "Sample"),
    RowMedian => ("Median", "Median"),
    RowRecordRange => ("Record range", "Record range"),
    SectionNormal => ("Normal for this date", "Normal for this date"),
    SectionRanges => ("Your ranges", "Your ranges"),
    ForecastCrest => ("Forecast crest", "Forecast crest"),
    SectionReadingQuality => ("Reading quality", "Reading quality"),
    RowReliability => ("Reliability", "Reliability"),
    /// `{threshold}`.
    TroutStressRule => ("Trout stress {threshold}", "Trout stress {threshold}"),
    SeaSurfaceForecast => (
        "Forecast sea surface (modelled)",
        "Forecast sea surface (modelled)",
    ),
    SeaSurfaceObserved => ("Observed sea surface", "Observed sea surface"),
    SeaSurfaceObservedRow => ("Sea surface, observed", "Sea surface, observed"),
    SeaSurfaceForecastRow => (
        "Sea surface, forecast (modelled)",
        "Sea surface, forecast (modelled)",
    ),
    SectionSeaSurface => ("Sea surface", "Sea surface"),
    WaterWord => ("Water", "Water"),
    Kd490Unavailable => (
        "Unavailable: no current satellite source",
        "Unavailable: no current satellite source",
    ),
    Kd490Note => (
        "Satellite estimate of how deeply light reaches",
        "Satellite estimate of how deeply light reaches",
    ),
    SatelliteUnavailable => (
        "Satellite clarity is unavailable: no current source",
        "Satellite clarity is unavailable: no current source",
    ),
    Turbidity => ("Turbidity", "Turbidity"),
    ClarityWord => ("Clarity", "Clarity"),
    Cyanobacteria => ("Cyanobacteria", "Cyanobacteria"),
    /// `{value}`.
    UnitCellsPerMl => ("cells/mL", "cells/mL"),
    CellsPerMl => ("{value} cells/mL", "{value} cells/mL"),
    SectionAlgae => ("Algae", "Algae"),
    Chlorophyll => ("Chlorophyll", "Chlorophyll"),
    ObservedWind => ("Observed wind", "Observed wind"),
    ObservedGusts => ("Observed gusts", "Observed gusts"),
    ForecastWind => ("Forecast wind", "Forecast wind"),
    ForecastGusts => ("Forecast gusts", "Forecast gusts"),
    ObservedPressure => ("Observed pressure", "Observed pressure"),
    ForecastPressure => ("Forecast pressure", "Forecast pressure"),
    /// `{direction}`.
    FromCap => ("From {direction}", "From {direction}"),
    FromLower => ("from {direction}", "from {direction}"),
    /// `{value}`.
    GustsCap => ("Gusts {value}", "Gusts {value}"),
    GustsLower => ("gusts {value}", "gusts {value}"),
    /// `{time}`.
    ObservedAt => ("observed {time}", "observed {time}"),
    BasisObserved => ("observed", "observed"),
    BasisForecast => ("forecast", "forecast"),
    /// `{title}`, `{value}`, `{extra}` (a leading comma and the trend, or nothing).
    ChartNowAcc => (
        "{title} chart: {value} now{extra}.",
        "{title} chart: {value} now{extra}.",
    ),
    /// `{title}`.
    ChartOnly => ("{title} chart.", "{title} chart."),
    Buoy => ("Buoy", "Buoy"),
    ModelWord => ("Model", "Model"),
    SectionSource => ("Source", "Source"),
    /// `{height}`.
    ModelledHeight => ("Modelled {height}", "Modelled {height}"),
    ModelledCap => ("Modelled", "Modelled"),
    ModelledWaves => ("Modelled waves", "Modelled waves"),
    WindWaves => ("Wind waves", "Wind waves"),
    /// `{n}`.
    SwellN => ("Swell {n}", "Swell {n}"),
    WaveHeight => ("Wave height", "Wave height"),
    WavePeriod => ("Wave period", "Wave period"),
    /// `{value}`.
    PeriodSeconds => ("period {value} seconds", "period {value} seconds"),
    ProductZone => ("Zone forecast", "Zone forecast"),
    ProductCwf => ("Coastal waters forecast", "Coastal waters forecast"),
    ProductNsh => ("Nearshore marine forecast", "Nearshore marine forecast"),
    ProductGlf => ("Open lake forecast", "Open lake forecast"),
    ProductOff => ("Offshore waters forecast", "Offshore waters forecast"),
    /// `{time}`.
    IssuedAt => ("Issued {time}", "Issued {time}"),
    /// `{date}`, `{time}`.
    IssuedOn => ("Issued {date}, {time}", "Issued {date}, {time}"),
    /// `{title}`, `{zone}`, `{product}`, `{issued}`, `{period}`, `{text}`.
    MarineForecastAcc => (
        "{title} for {zone}. {product}, {issued}. {period}: {text}",
        "{title} for {zone}. {product}, {issued}. {period}: {text}",
    ),
    /// `{time}`.
    UntilTime => ("Until {time}", "Until {time}"),
    /// `{n}`.
    MoreCount => ("{n} more", "{n} more"),
    /// `{title}`, `{count}`, `{events}`.
    AlertsAcc => (
        "{title}: {count} active. {events}.",
        "{title}: {count} active. {events}.",
    ),
    /// `{models}`.
    WeatherModels => ("Weather models: {models}", "Weather models: {models}"),
    MeasWaterTemperature => ("Water temperature", "Water temperature"),
    MeasDischarge => ("River flow", "River flow"),
    MeasGageHeight => ("Gage height", "Gage height"),
    MeasLakeLevel => ("Lake level", "Lake level"),
    MeasReservoir => ("Reservoir release", "Reservoir release"),
    MeasTurbidity => ("Turbidity", "Turbidity"),
    MeasDissolvedOxygen => ("Dissolved oxygen", "Dissolved oxygen"),
    MeasConductivity => ("Conductivity", "Conductivity"),
    MeasSalinity => ("Salinity", "Salinity"),
    MeasPh => ("pH", "pH"),
    MeasChlorophyll => ("Chlorophyll", "Chlorophyll"),
    MeasClarity => ("Water clarity", "Water clarity"),
    MeasCyanobacteria => ("Cyanobacteria", "Cyanobacteria"),
    MeasBloom => ("Algal bloom", "Algal bloom"),
    MeasTide => ("Tide", "Tide"),
    MeasTidalCurrent => ("Tidal current", "Tidal current"),
    MeasOceanCurrent => ("Ocean current", "Ocean current"),
    MeasWaves => ("Waves", "Waves"),
    MeasWavesForecast => ("Wave forecast", "Wave forecast"),
    MeasSeaSurfaceTemp => ("Sea surface temperature", "Sea surface temperature"),
    MeasSeaSurfaceTempForecast => (
        "Sea surface temperature forecast",
        "Sea surface temperature forecast",
    ),
    MeasMarineForecast => ("Marine forecast", "Marine forecast"),
    MeasMarineAlerts => ("Marine alerts", "Marine alerts"),
    MeasFloodCategories => ("Flood categories", "Flood categories"),
    MeasFlowForecast => ("River forecast", "River forecast"),
    MeasWind => ("Wind", "Wind"),
    MeasPressure => ("Pressure", "Pressure"),
    RelSameWaterBody => ("Same water body", "Same water body"),
    RelUpstream => ("Upstream", "Upstream"),
    RelDownstream => ("Downstream", "Downstream"),
    RelNearby => ("Nearby", "Nearby"),
    RelModelGrid => ("Model grid", "Model grid"),
    RelSatellite => ("Satellite", "Satellite"),
    StationPinned => ("Pinned by you", "Pinned by you"),
    StationExcluded => ("Excluded by you", "Excluded by you"),
    StationWeakAndVariable => (
                "Weak and variable current: no predictions",
                "Weak and variable current: no predictions",
            ),
    /// `{bin}`.
    StationOtherBin => (
                "Bin {bin} of this station is used instead",
                "Bin {bin} of this station is used instead",
            ),
    /// `{age}`: "6 min ago".
    StationReported => ("Latest value {age}", "Latest value {age}"),
    StationListed => (
                "Listed by the source; not checked yet",
                "Listed by the source; Luna has not checked yet",
            ),
    /// `{age}`: "2 h ago".
    StationNotReported => (
                "Checked {age}: no recent values",
                "Checked {age}: no recent values",
            ),
    StationOtherWater => (
                "On another river or a tributary",
                "On another river or a tributary",
            ),
    StationNotPlaced => (
                "Not on the mapped streams, so maybe another river",
                "Not on the mapped streams, so maybe another river",
            ),
    /// `{site}`: the site's id.
    StationForecastGauge => ("Forecast gauge for {site}", "Forecast gauge for {site}"),
    StationSecondary => (
                "Not the station's main product",
                "Not the station's main product",
            ),
    /// `{n}`: the stations left out of one measurement.
    StationsNotUsed => ("{n} not used", "Luna passed over {n}"),
    /// `{kind}`: a zone kind below.
    WtReasonZone => ("in an NWS {kind} marine zone", "in an NWS {kind} marine zone"),
    ZoneCoastal => ("coastal", "coastal"),
    ZoneGreatLakesNearshore => ("Great Lakes nearshore", "Great Lakes nearshore"),
    ZoneGreatLakesOpen => ("Great Lakes open", "Great Lakes open"),
    ZoneOffshore => ("offshore", "offshore"),
    /// `{name}`, `{what}`.
    WtReasonOn => ("on {name} ({what})", "on {name} ({what})"),
    /// `{what}`.
    WtReasonOnUnnamed => ("on an unnamed {what}", "on an unnamed {what}"),
    BodyLake => ("lake", "lake"),
    BodyReservoir => ("reservoir", "reservoir"),
    BodyRiver => ("river", "river"),
    BodyEstuary => ("estuary", "estuary"),
    BodyOther => ("waterbody", "waterbody"),
    /// `{name}`.
    WtReasonBay => (
        "{name} is named as a bay or harbor",
        "{name} is named as a bay or harbor",
    ),
    /// `{order}`.
    StreamOfOrder => ("stream of order {order}", "stream of order {order}"),
    StreamPlain => ("stream", "stream"),
    /// `{kind}`: a site kind below.
    WtReasonUsgs => (
        "the nearest USGS site is {kind}",
        "the nearest USGS site is {kind}",
    ),
    SiteStream => ("on a stream", "on a stream"),
    SiteLake => ("on a lake", "on a lake"),
    SiteEstuary => ("on an estuary", "on an estuary"),
    SiteTidalStream => ("on a tidal stream", "on a tidal stream"),
    SiteOther => ("of another kind", "of another kind"),
    Computed => ("Computed", "Computed"),
    ErrRateLimited => ("Rate limited", "Rate limited"),
    ErrNotFound => ("Not found", "Not found"),
    ErrNoData => ("No data", "No data"),
    ErrStale => ("Out of date", "Out of date"),
    ErrParse => ("Unreadable response", "Unreadable response"),
    ErrTransport => ("Connection problem", "Connection problem"),
    ErrOther => ("Failed", "Failed"),
    UnitSources => ("sources", "sources"),
    /// `{n}`.
    StationOne => ("{n} station", "{n} station"),
    StationMany => ("{n} stations", "{n} stations"),
    ProblemOne => ("{n} problem", "{n} problem"),
    ProblemMany => ("{n} problems", "{n} problems"),
    Gust => ("Gust", "Gust"),
    Twilight => ("Twilight", "Twilight"),
    GoldenHour => ("Golden hour", "Golden hour"),
    LaneLight => ("Light", "Light"),
    /// The placeholder for an unknown value.
    Missing => ("—", "—"),
    /// A spot's page when there are no conditions to show and the last fetch failed.
    PageProblemTitle => (
        "Conditions could not be loaded",
        "Luna couldn't fetch the water",
    ),
    /// `{reason}`: the parser's message.
    SnapshotUnreadable => (
        "The saved conditions could not be read: {reason}",
        "The saved conditions could not be read: {reason}",
    ),
    /// Under the place search of a new spot.
    PlaceSearchFooter => (
        "What you type goes to Apple's Maps service to find the place, with only an approximate area around you when your location is known. Underbite keeps nothing of it.",
        "What you type goes to Apple's Maps service to find the place, with only an approximate area around you when your location is known. Underbite keeps nothing of it.",
    ),
    // -- the map ------------------------------------------------------------------------
    /// The drawer's sections.
    MapGroupBase => ("Base map", "Base map"),
    MapGroupDepth => ("Depth", "Depth"),
    MapGroupStructure => ("Structure", "Structure"),
    MapGroupWater => ("Water", "Water"),
    MapGroupWeather => ("Weather", "Weather"),
    MapGroupStations => ("Stations", "Stations"),
    MapGroupMine => ("Mine", "Mine"),
    /// Layer titles, by the catalogue's layer ids.
    MapLayerAppleStandard => ("Standard", "Standard"),
    MapLayerAppleSatellite => ("Satellite", "Satellite"),
    MapLayerAppleHybrid => ("Hybrid", "Hybrid"),
    MapLayerNoaaChart => ("NOAA chart", "NOAA chart"),
    MapLayerGebcoRelief => ("GEBCO relief", "GEBCO relief"),
    MapLayerSst => ("Sea surface temperature", "Sea surface temperature"),
    MapLayerSstImagery => (
        "Sea surface temperature (imagery)",
        "Sea surface temperature (imagery)",
    ),
    MapLayerChlorophyll => ("Chlorophyll", "Chlorophyll"),
    MapLayerChlorophyllImagery => ("Chlorophyll (imagery)", "Chlorophyll (imagery)"),
    MapLayerClarity => ("Water clarity", "Water clarity"),
    MapLayerTrueColourViirsN20 => ("True colour, VIIRS NOAA-20", "True colour, VIIRS NOAA-20"),
    MapLayerTrueColourViirsN21 => ("True colour, VIIRS NOAA-21", "True colour, VIIRS NOAA-21"),
    MapLayerTrueColourModisTerra => ("True colour, MODIS Terra", "True colour, MODIS Terra"),
    MapLayerTrueColourModisAqua => ("True colour, MODIS Aqua", "True colour, MODIS Aqua"),
    MapLayerHfRadarCurrents => ("Surface currents", "Surface currents"),
    MapLayerGreatLakesSst => ("Great Lakes temperature", "Great Lakes temperature"),
    MapLayerRadar => ("Radar", "Radar"),
    MapLayerWind => ("Wind", "Wind"),
    MapLayerPressure => ("Pressure", "Pressure"),
    MapLayerWaves => ("Waves", "Waves"),
    MapLayerTideStations => ("Tide stations", "Tide stations"),
    MapLayerCurrentStations => ("Current stations", "Current stations"),
    MapLayerUsgsGauges => ("River gauges", "River gauges"),
    MapLayerBuoys => ("Buoys", "Buoys"),
    MapLayerNwpsGauges => ("River forecasts", "River forecasts"),
    MapLayerSpots => ("My spots", "My spots"),
    MapLayerLakeContoursIn => ("Lake contours (Indiana)", "Lake contours (Indiana)"),
    MapLayerLakeContoursMn => ("Lake contours (Minnesota)", "Lake contours (Minnesota)"),
    MapLayerArtificialReefsNy => (
        "Artificial reefs (New York)",
        "Artificial reefs (New York)",
    ),
    MapLayerEncDepths => ("ENC depths and soundings", "ENC depths and soundings"),
    MapLayerEncSeabed => ("ENC seabed and obstructions", "ENC seabed and obstructions"),
    MapLayerEncSpecialAreas => ("ENC special areas", "ENC special areas"),
    MapLayerEncStructurePoints => (
        "Wrecks and obstructions (ENC)",
        "Wrecks and obstructions (ENC)",
    ),
    /// Legend captions of the colour ramps.
    MapLegendColdToWarm => ("Cold to warm", "Cold to warm"),
    MapLegendClearToGreen => ("Clear to green water", "Clear to green water"),
    MapLegendClearToTurbid => ("Clear to turbid", "Clear to turbid"),
    MapLegendSlowToFast => ("Slow to fast", "Slow to fast"),
    /// Under a legend whose range follows the values in view.
    MapLegendFitted => ("Colours fitted to this view", "Colours fitted to this view"),
    /// The built-in presets, by id.
    MapPresetInshoreFlats => ("Inshore flats", "Inshore flats"),
    MapPresetOffshorePelagics => ("Offshore pelagics", "Offshore pelagics"),
    MapPresetTroutStream => ("Trout stream", "Trout stream"),
    MapPresetBassLake => ("Bass lake", "Bass lake"),
    MapPresetSurf => ("Surf", "Surf"),
    MapPresetGreatLakesTrolling => ("Great Lakes trolling", "Great Lakes trolling"),
    /// In place of the switch of a layer that cannot be turned on.
    MapNoSource => ("No source yet", "Luna hasn't found a source yet"),
    /// The point inspector where a layer has no value (land, cloud, outside its data).
    MapNoValue => ("No value here", "Nothing here"),
    /// A surface current at a point: `{speed}`, `{direction}`.
    MapCurrentToward => ("{speed} toward {direction}", "{speed} toward {direction}"),
    /// When an hourly field is valid: `{date}`, `{time}`.
    MapDateTime => ("{date}, {time}", "{date}, {time}"),
    /// A measured line: `{distance}`, `{bearing}`.
    MapMeasure => ("{distance} at {bearing}", "{distance} at {bearing}"),
    // Depth on the map
    /// The depth shading's title in the layer drawer.
    MapDepthTitle => ("Water depth", "Water depth"),
    /// A temperature break's strength: `{change}` ("0.9 °F"), `{distance}` ("mi").
    MapBreakTemperature => ("{change} per {distance}", "{change} per {distance}"),
    /// A chlorophyll break's strength as a ratio of concentrations: `{ratio}` ("1.3"), `{distance}`.
    MapBreakChlorophyll => ("×{ratio} per {distance}", "×{ratio} per {distance}"),
    /// What a charted structure point is.
    MapStructureWreck => ("Wreck", "Wreck"),
    MapStructureObstruction => ("Obstruction", "Obstruction"),
    MapStructureRock => ("Rock", "Rock"),
    MapStructureFishHaven => ("Fish haven", "Fish haven"),
    /// The chart's own category of a structure point: `{category}`.
    MapStructureCategory => ("As charted: {category}", "As charted: {category}"),
    /// When a chart object was surveyed: `{date}` ("2019", "2019-03", "2019-03-14").
    MapStructureSurveyed => ("Surveyed {date}", "Surveyed {date}"),
    /// A reef site's charted depths: `{range}` ("20–70 ft").
    MapReefDepth => ("Depth {range}", "Depth {range}"),
    // Station pins
    /// What a pin on the map is.
    PinTideStation => ("Tide station", "Tide station"),
    PinCurrentStation => ("Current station", "Current station"),
    PinBuoy => ("Buoy", "Buoy"),
    PinCoastalStation => ("Coastal station", "Coastal station"),
    PinRiverGauge => ("River gauge", "River gauge"),
    PinForecastGauge => ("River forecast gauge", "River forecast gauge"),
    /// A pin with its state: `{kind}`, `{state}`.
    PinLabel => ("{kind} · {state}", "{kind} · {state}"),
    /// A state with how old it is: `{state}`, `{age}` ("6 min ago").
    PinStateAge => ("{state}, {age}", "{state}, {age}"),
    /// A gauge whose latest value is too old or missing.
    PinNoRecentValue => ("No recent value", "Nothing lately"),
    /// A forecast gauge with no flood stages set.
    PinNoFloodStages => ("No flood stages", "No flood stages"),
    // Offline
    /// A pack's size: `{n}` ("512", "48.3", "1.5").
    PackSizeKb => ("{n} KB", "{n} KB"),
    PackSizeMb => ("{n} MB", "{n} MB"),
    PackSizeGb => ("{n} GB", "{n} GB"),
    /// A pack's download time.
    PackTimeUnderMinute => ("under a minute", "under a minute"),
    /// `{n}` minutes.
    PackTimeMinutes => ("{n} min", "{n} min"),
    /// `{h}` whole hours.
    PackTimeHours => ("{h} h", "{h} h"),
    /// `{h}` hours and `{m}` minutes.
    PackTimeHoursMinutes => ("{h} h {m} min", "{h} h {m} min"),
    /// A pack's size and time: `{size}`, `{time}`.
    PackSummary => ("{size}, about {time}", "{size}, about {time}"),
    /// A pack larger than the offline limit: `{size}`, `{limit}`.
    PackOverBudget => (
        "About {size}: more than your {limit} offline limit.",
        "About {size}: more than your {limit} offline limit.",
    ),
    /// Why a layer is not in a pack.
    PackSkipUnknown => ("Not a map layer", "Not a map layer"),
    PackSkipUnavailable => ("Not available", "Not available"),
    PackSkipNotPackable => ("Not kept offline", "Not kept offline"),
    PackSkipZoom => ("Not drawn at these zoom levels", "Not drawn at these zoom levels"),
    /// Why a pack cannot be planned.
    PackErrorBox => ("The area is not valid.", "The area is not valid."),
    PackErrorZooms => ("The zoom range is not valid.", "The zoom range is not valid."),
    PackErrorDates => ("The days are not valid.", "The days are not valid."),
    /// `{cap}` downloads.
    PackErrorTooMany => (
        "This pack needs more than {cap} downloads: choose a smaller area or fewer zoom levels.",
        "This pack needs more than {cap} downloads: choose a smaller area or fewer zoom levels.",
    ),
    /// A table that cannot be computed.
    TableNoConstants => (
        "No tide or current table could be computed from the stored constants.",
        "No tide or current table could be computed from the stored constants.",
    ),
    /// A fetch that found no signal: `{time}` is when the other values were fetched.
    OfflineComputedLast => (
        "offline: computed from harmonic constants, other values from {time}",
        "offline: computed from harmonic constants, other values from {time}",
    ),
    OfflineComputedOnly => (
        "offline: computed from harmonic constants",
        "offline: computed from harmonic constants",
    ),
    OfflineLastOnly => ("offline: values from {time}", "offline: values from {time}"),
    // -- card titles ----------------------------------------------------------------------
    CardNow => ("Now", "Now"),
    CardTide => ("Tide", "Tide"),
    CardCurrent => ("Current", "Current"),
    CardFlow => ("River flow", "River flow"),
    CardWaterTemp => ("Water temperature", "Water temperature"),
    CardClarity => ("Water clarity", "Water clarity"),
    CardBloom => ("Algal bloom", "Algal bloom"),
    CardWaves => ("Waves", "Waves"),
    CardWind => ("Wind", "Wind"),
    CardPressure => ("Pressure", "Pressure"),
    CardSunMoon => ("Sun and moon", "Sun and moon"),
    CardSolunar => ("Solunar", "Solunar"),
    CardWindows => ("Fishing windows", "Snack times"),
    CardGoNoGo => ("Go or no go", "Luna's verdict"),
    CardStormAhead => ("Storm ahead", "Storm ahead"),
    CardMarineForecast => ("Marine forecast", "Marine forecast"),
    CardStocking => ("Stocking", "Stocking"),
    CardSpecies => ("Species", "Species"),
    CardAlerts => ("Alerts", "Alerts"),
    CardSources => ("Sources", "Sources"),
    // -- reservoir lines of the flow card ---------------------------------------------------
    SectionReservoir => ("Reservoir", "Reservoir"),
    RowProject => ("Project", "Project"),
    RowOutflow => ("Outflow", "Outflow"),
    RowInflow => ("Inflow", "Inflow"),
    /// `{value}`: "1,200 cfs"; `{age}`: "3 h ago".
    ReservoirOutflowLine => ("Outflow {value} · {age}", "Outflow {value} · {age}"),
    // -- the bloom card's labels --------------------------------------------------------------
    BloomSectionSatellite => ("Satellite estimate", "Satellite estimate"),
    BloomSectionForecast => ("Bloom forecast", "Bloom forecast"),
    BloomRowMax => ("Highest cell count", "Highest cell count"),
    BloomRowComposite => ("Composite date", "Composite date"),
    BloomRowCells => ("Cells with a reading", "Cells with a reading"),
    BloomRowDistance => ("Distance", "Distance"),
    BloomRowBeach => ("Beach", "Beach"),
    BloomRowLevel => ("Respiratory irritation", "Respiratory irritation"),
    BloomRowConfidence => ("Confidence", "Confidence"),
    BloomRowValid => ("Forecast for", "Forecast for"),
    // -- the stocking card ------------------------------------------------------------------
    StockingSectionRecent => ("Recent stockings", "Recent stockings"),
    StockingSectionPlanned => ("Planned stockings", "Planned stockings"),
    StockingNone => ("No recent stocking reported nearby", "No recent stocking reported nearby"),
    StockingEstimateNote => ("Numbers are the agency's estimates.", "Numbers are the agency's estimates."),
    StockingLagNote => ("The list of stockings that took place lags about a year.", "The list of stockings that took place lags about a year."),
    StockingPlannedNote => ("Planned stockings are a schedule and can change.", "Planned stockings are a schedule and can change."),
    StockingUnitOne => ("stocking", "stocking"),
    StockingUnitMany => ("stockings", "stockings"),
    /// `{month}` ("Apr"), `{year}`.
    StockingMonthYear => ("{month} {year}", "{month} {year}"),
    /// `{n}`: "about 1,200".
    StockingAbout => ("about {n}", "about {n}"),
    /// `{when}`: the list's own text for a planned date.
    StockingPlannedWhen => ("planned: {when}", "planned: {when}"),
    /// `{species}`, `{details}`.
    StockingSummary => ("{species} · {details}", "{species} · {details}"),
    // -- nearby lookups on the map: access, species records, stocked waters ---------------
    NearbyZoomInAccess => (
        "Zoom in to look for ramps, piers and public land",
        "Zoom in to look for ramps, piers and public land",
    ),
    NearbyZoomInOsm => (
        "Zoom in to look for ramps, piers and parking from OpenStreetMap",
        "Zoom in to look for ramps, piers and parking from OpenStreetMap",
    ),
    NearbyZoomInRecords => ("Zoom in to look for species records", "Zoom in to look for species records"),
    NearbyZoomInStocking => ("Zoom in to look for stocked waters", "Zoom in to look for stocked waters"),
    /// `{source}`: the source's name.
    NearbyMoreThanShown => (
        "{source} has more here than shown: zoom in to see it all",
        "{source} has more here than shown: zoom in to see it all",
    ),
    /// `{source}`.
    NearbyDailyCap => (
        "{source}: this phone's lookups for today are used up; try again tomorrow",
        "{source}: this phone's lookups for today are used up; try again tomorrow",
    ),
    /// `{source}`, `{wait}`: "2 min".
    NearbyPaused => (
        "{source} asked to slow down; try again in {wait}",
        "{source} asked to slow down; try again in {wait}",
    ),
    /// `{source}`.
    NearbyBusy => ("{source} is busy; try again later", "{source} is busy; try again later"),
    /// `{source}`.
    NearbyFailed => ("{source} could not be reached", "{source} could not be reached"),
    AccessKindRamp => ("Boat ramp", "Boat ramp"),
    AccessKindPier => ("Pier", "Pier"),
    AccessKindFishingSpot => ("Fishing spot", "Fishing spot"),
    AccessKindParking => ("Parking", "Parking"),
    AccessKindPublicLand => ("Public land", "Public land"),
    /// `{title}`, `{kind}`, `{access}`.
    AccessAccessibility => ("{title}, {kind}. {access}", "{title}, {kind}. {access}"),
    /// `{date}` ("Nov 3"), `{year}`.
    RecordSeen => ("Seen {date} {year}", "Seen {date} {year}"),
    RecordDateUnknown => ("Date not recorded", "Date not recorded"),
    /// `{distance}`: "30 m".
    RecordWithin => ("Position good to {distance}", "Position good to {distance}"),
    /// `{distance}`: the blurring cell's radius.
    RecordObscured => (
        "Position blurred by the source, within {distance}",
        "Position blurred by the source, within {distance}",
    ),
    RecordObscuredArea => ("Position blurred by the source", "Position blurred by the source"),
    /// `{licence}`: "CC BY".
    RecordPhoto => ("Photo: {licence}", "Photo: {licence}"),
    /// `{title}`, `{seen}`, `{source}`.
    RecordAccessibility => ("{title}, {seen}, {source}", "{title}, {seen}, {source}"),
    RecordsWhereSeen => (
        "Records show where a species was reported, not where it is now.",
        "Records show where a species was reported, not where it is now.",
    ),
    // -- the catch log --------------------------------------------------------------------
    LogReleased => ("Released", "Released"),
    /// Luna's fun tag; the plain voice says "Kept".
    LogKept => ("Kept", "Kept"),
    LogKeptForSashimi => ("Kept", "Kept for sashimi"),
    LogSpeciesUnknown => ("Unknown species", "Unknown species"),
    /// `{date}`, `{year}`, `{time}`: in the catch's own time zone.
    LogWhen => ("{date} {year} · {time}", "{date} {year} · {time}"),
    LogSizeJoin => ("{length} · {weight}", "{length} · {weight}"),
    /// The spoken line of a row: `{species}`, `{details}` (size, time, place and fate).
    LogRowSummary => ("{species}, {details}", "{species}, {details}"),
    LogJoin => ("{first}, {second}", "{first}, {second}"),
    LogEmptyTitle => ("No catches yet", "Luna's plate is empty"),
    LogEmptyBody => (
        "Log a catch and the conditions at that moment are kept with it.",
        "Log a catch and Luna keeps the conditions beside it. The sashimi plate is waiting.",
    ),
    LogConditionsTitle => ("Conditions", "Conditions"),
    LogExportTitle => ("Export the log", "Export the log"),
    LogExportCsv => ("Spreadsheet (CSV)", "Spreadsheet (CSV)"),
    LogExportGpx => ("Waypoints (GPX)", "Waypoints (GPX)"),
    LogNoConditions => (
        "No conditions were saved with this catch.",
        "No conditions were saved with this catch.",
    ),
    LogLure => ("Lure", "Lure"),
    LogMethod => ("Method", "Method"),
    LogNotes => ("Notes", "Notes"),
    LogLabelTideHeight => ("Tide height", "Tide height"),
    LogLabelPressureTrend => ("Pressure trend", "Pressure trend"),
    LogLabelAirTemperature => ("Air temperature", "Air temperature"),
    LogLabelMoon => ("Moon", "Moon"),
    LogLabelLight => ("Light", "Light"),
    LogHigh => ("high", "high"),
    LogLow => ("low", "low"),
    /// `{direction}`, `{since}`, `{previous}`, `{to}`, `{next}`.
    LogTideStage => (
        "{direction}, {since} after {previous}, {to} before {next}",
        "{direction}, {since} after {previous}, {to} before {next}",
    ),
    /// `{speed}`, `{set}`.
    LogCurrentSet => ("{speed}, {set}", "{speed}, {set}"),
    /// `{value}`, `{direction}`.
    LogToward => ("{value}, toward {direction}", "{value}, toward {direction}"),
    /// `{wind}`, `{gust}`.
    LogWindGust => ("{wind}, gusts {gust}", "{wind}, gusts {gust}"),
    /// `{wind}`, `{direction}`.
    LogWindFrom => ("{wind}, from {direction}", "{wind}, from {direction}"),
    /// `{flow}`, `{percentile}`.
    LogFlowValue => ("{flow}, {percentile}", "{flow}, {percentile}"),
    /// `{phase}`, `{lit}`.
    LogMoonValue => ("{phase}, {lit} lit", "{phase}, {lit} lit"),
    /// `{label}`, `{range}`.
    LogSolunarPeriod => ("{label}, {range}", "{label}, {range}"),
    LogLightDawn => ("Dawn", "Dawn"),
    LogLightDay => ("Day", "Day"),
    LogLightDusk => ("Dusk", "Dusk"),
    LogLightNight => ("Night", "Night"),
    LogComputed => ("Computed on this device", "Computed on this device"),
    /// A condition's footnote: `{age}` is a duration such as "6 min".
    LogAgeBefore => ("{age} before the catch", "{age} before the catch"),
    LogAgeAfter => ("{age} after the catch (a prediction)", "{age} after the catch (a prediction)"),
    LogAgeNow => ("at the time of the catch", "at the time of the catch"),
    /// What the editor says about a number it cannot read; `{example}` is written in the locale.
    LogParseNumber => ("Enter a number, such as {example}.", "Enter a number, such as {example}."),
    LogParseNotPositive => ("Enter more than zero.", "Enter more than zero."),
    LogParseTooLarge => ("That is too large to be a fish.", "That is too large to be a fish."),
    LogParsePounds => ("Enter whole pounds, and the ounces in the other field.", "Enter whole pounds, and the ounces in the other field."),
    LogParseOunces => ("Enter less than 16 ounces.", "Enter less than 16 ounces."),
    // What worked.
    LogInsightsTotal => ("Catches counted: {n}", "Catches counted: {n}"),
    LogInsightsNote => (
        "Counts show when fish were caught, not when fishing is best: the log does not record the time spent fishing without a bite.",
        "Counts show when fish were caught, not when fishing is best: the log does not record the time spent fishing without a bite.",
    ),
    LogInsightsEmpty => (
        "Log some catches to see when your fish were caught.",
        "Luna has nothing to sniff at yet. Log a catch first.",
    ),
    /// `{n}` logged, `{min}` needed.
    LogInsightsFew => (
        "Too few catches to tell: {n} logged, {min} needed.",
        "Too few catches to tell: {n} logged, {min} needed.",
    ),
    /// `{known}` catches with the condition recorded, `{min}` needed.
    LogSectionFew => (
        "Too few catches with this recorded to tell: {known} of the {min} needed.",
        "Too few catches with this recorded to tell: {known} of the {min} needed.",
    ),
    /// `{unknown}` of `{total}` catches.
    LogSectionUnknown => (
        "{unknown} of {total} catches have no record of this.",
        "{unknown} of {total} catches have no record of this.",
    ),
    /// `{n}` of `{known}`, `{subject}`, `{value}`.
    LogSentence => ("{n} of {known} {subject}: {value}", "{n} of {known} {subject}: {value}"),
    LogSubjectCatches => ("catches", "catches"),
    /// `{species}`.
    LogSubjectSpecies => ("{species} catches", "{species} catches"),
    LogNoStandout => (
        "No single value stands out: the catches are spread evenly.",
        "No single value stands out: the catches are spread evenly.",
    ),
    /// `{label}`, `{n}`, `{known}`.
    LogBarSpoken => ("{label}: {n} of {known}", "{label}: {n} of {known}"),
    LogSectionTide => ("Tide direction", "Tide direction"),
    LogSectionSinceTurn => ("Time since the tide turned", "Time since the tide turned"),
    LogSectionHour => ("Hour of day", "Hour of day"),
    LogTideRising => ("Rising tide", "Rising tide"),
    LogTideFalling => ("Falling tide", "Falling tide"),
    /// `{from}`, `{to}` hours and `{turn}` (high or low).
    LogHoursAfter => ("{from}–{to} h after {turn}", "{from}–{to} h after {turn}"),
    LogHoursAfterLast => ("{from}+ h after {turn}", "{from}+ h after {turn}"),
    LogFlowUnder10 => ("Under the 10th percentile", "Under the 10th percentile"),
    LogFlow10To25 => ("10th to 25th percentile", "10th to 25th percentile"),
    LogFlow25To75 => ("25th to 75th percentile", "25th to 75th percentile"),
    LogFlow75To90 => ("75th to 90th percentile", "75th to 90th percentile"),
    LogFlowOver90 => ("Over the 90th percentile", "Over the 90th percentile"),
    LogSolunarMajor => ("Major period", "Big nibble"),
    LogSolunarMinor => ("Minor period", "Small nibble"),
    LogSolunarOutside => ("Outside the periods", "Outside the periods"),
    /// A temperature bin: `{from}`, `{to}`, `{unit}`.
    LogRange => ("{from}–{to} {unit}", "{from}–{to} {unit}"),
    // -- species: the card, the profile page and the season view ---------------------------
    SpeciesStageUnknown => ("Stage unknown", "Stage unknown"),
    SpeciesStageTooCold => ("Too cold", "Too cold"),
    SpeciesStagePreSpawn => ("Pre-spawn", "Pre-spawn"),
    SpeciesStageSpawning => ("Spawning", "Spawning"),
    SpeciesStageActive => ("In its preferred range", "In its preferred range"),
    SpeciesStagePostSpawn => ("Post-spawn or summer", "Post-spawn or summer"),
    SpeciesStageHeatStress => ("Heat stress", "Heat stress"),
    /// `{value}`: the water temperature now.
    SpeciesWaterNow => ("water {value}", "water {value}"),
    /// `{range}`.
    SpeciesSpawnUsually => ("spawn usually {range}", "spawn usually {range}"),
    /// `{range}`.
    SpeciesPrefers => ("prefers {range}", "prefers {range}"),
    /// `{limit}`.
    SpeciesStressedAbove => ("stressed above {limit}", "stressed above {limit}"),
    /// `{first}`, `{second}`: two details of a stage.
    SpeciesJoin => ("{first}, {second}", "{first}, {second}"),
    /// `{species}`, `{stage}`, `{details}`.
    SpeciesLine => ("{species} · {stage} · {details}", "{species} · {stage} · {details}"),
    /// `{species}`, `{stage}`: a line with nothing more to say.
    SpeciesLineBare => ("{species} · {stage}", "{species} · {stage}"),
    SpeciesNoWater => ("no water temperature reading here", "no water temperature reading here"),
    SpeciesNoRanges => (
        "the profile gives no spawning or preferred temperature",
        "the profile gives no spawning or preferred temperature",
    ),
    /// `{from}`, `{to}`, `{unit}`.
    SpeciesRange => ("{from}–{to} {unit}", "{from}–{to} {unit}"),
    SpeciesRowStage => ("Stage", "Stage"),
    SpeciesRowWater => ("Water temperature", "Water temperature"),
    SpeciesRowHeatLimit => ("Heat limit", "Heat limit"),
    /// The heat limit's basis: the user's trout-stress setting.
    SpeciesHeatTroutSetting => ("your trout-stress setting", "your trout-stress setting"),
    /// `{limit}`, `{basis}`.
    SpeciesHeatLimitValue => ("{limit} ({basis})", "{limit} ({basis})"),
    SpeciesFieldScientific => ("Scientific name", "Scientific name"),
    SpeciesFieldCommonNames => ("Common names", "Common names"),
    SpeciesFieldAlias => ("Also called", "Also called"),
    SpeciesFieldWater => ("Water", "Water"),
    SpeciesFieldPreferred => ("Preferred temperature", "Preferred temperature"),
    SpeciesFieldTolerated => ("Tolerated temperature", "Tolerated temperature"),
    SpeciesFieldSpawn => ("Spawning temperature", "Spawning temperature"),
    SpeciesFieldDepth => ("Depth", "Depth"),
    SpeciesFieldHabitat => ("Habitat", "Habitat"),
    SpeciesFieldFlow => ("Tide and current", "Tide and current"),
    SpeciesFieldLight => ("Light", "Light"),
    SpeciesFieldLures => ("Lures and baits", "Lures and baits"),
    SpeciesFieldRegions => ("Regions", "Regions"),
    SpeciesWaterFresh => ("Fresh water", "Fresh water"),
    SpeciesWaterSalt => ("Salt water", "Salt water"),
    SpeciesWaterBoth => ("Fresh and salt water", "Fresh and salt water"),
    SpeciesSeasonSpring => ("Spring", "Spring"),
    SpeciesSeasonSummer => ("Summer", "Summer"),
    SpeciesSeasonAutumn => ("Autumn", "Autumn"),
    SpeciesSeasonWinter => ("Winter", "Winter"),
    /// `{season}`, `{range}`.
    SpeciesDepthSeason => ("{season}: {range}", "{season}: {range}"),
    /// `{range}`: the same depth in every season.
    SpeciesDepthAllYear => ("{range}, all year", "{range}, all year"),
    SpeciesFlowIncoming => ("Incoming tide", "Incoming tide"),
    SpeciesFlowOutgoing => ("Outgoing tide", "Outgoing tide"),
    SpeciesFlowSlackHigh => ("Slack at high water", "Slack at high water"),
    SpeciesFlowSlackLow => ("Slack at low water", "Slack at low water"),
    SpeciesFlowMoving => ("Moving water", "Moving water"),
    SpeciesFlowSlack => ("Slack water", "Slack water"),
    SpeciesLightDawn => ("Dawn", "Dawn"),
    SpeciesLightDay => ("Day", "Day"),
    SpeciesLightDusk => ("Dusk", "Dusk"),
    SpeciesLightNight => ("Night", "Night"),
    SpeciesLightOvercast => ("Overcast", "Overcast"),
    SpeciesNotKnown => ("No source gives this", "No source gives this"),
    /// The source of a value the user typed.
    SpeciesYourEdit => ("Your edit", "Your edit"),
    /// The source of a value the sender of a shared profile typed.
    SpeciesSenderEdit => ("Edited by the sender", "Edited by the sender"),
    SpeciesSeasonNowTitle => ("Now, from the water temperature", "Now, from the water temperature"),
    SpeciesSeasonTypicalTitle => ("Typical months", "Typical months"),
    SpeciesSeasonLegalTitle => ("Regulations", "Regulations"),
    SpeciesInputProfile => ("the profile", "the profile"),
    /// `{first}`, `{last}` (years), `{kind}` (a history kind).
    SpeciesInputWaterYears => (
        "water temperature {first}–{last}, {kind}",
        "water temperature {first}–{last}, {kind}",
    ),
    /// `{n}` years, `{kind}`.
    SpeciesInputWaterCount => (
        "water temperature, {n} years, {kind}",
        "water temperature, {n} years, {kind}",
    ),
    SpeciesInputObservations => ("observations", "observations"),
    /// `{inputs}`: what a month drew on.
    SpeciesTypicalFrom => ("From {inputs}", "From {inputs}"),
    SpeciesTypicalNothing => ("Nothing to go on", "Nothing to go on"),
    SpeciesTypicalNoNormals => (
        "No typical water temperatures for this place yet, so the months cannot be placed.",
        "No typical water temperatures for this place yet, so the months cannot be placed.",
    ),
    /// `{n}` observations in a month.
    SpeciesObservations => ("{n} observed", "{n} observed"),
    SpeciesLegalNoRegion => (
        "Set the spot's region to see the official regulation pages.",
        "Set the spot's region to see the official regulation pages.",
    ),
    /// `{region}`: an ISO 3166-2 code.
    SpeciesLegalNoLinks => (
        "No official pages are listed for {region}.",
        "No official pages are listed for {region}.",
    ),
    SpeciesLegalNotes => ("Your notes", "Your notes"),
    /// `{example}`: a number in the locale.
    SpeciesEditNumber => ("Enter a number, such as {example}.", "Enter a number, such as {example}."),
    SpeciesEditBackwards => (
        "The first number must not be above the second.",
        "The first number must not be above the second.",
    ),
    SpeciesEditOutOfRange => ("That is outside what any fish lives in.", "That is outside what any fish lives in."),
    SpeciesEditEmpty => ("Enter at least one.", "Enter at least one."),
    SpeciesEditChoice => ("Pick one of the choices.", "Pick one of the choices."),
    SpeciesEditBothEnds => ("Enter both ends of the range.", "Enter both ends of the range."),
    // -- unit symbols -----------------------------------------------------------------------
    UnitFahrenheit => ("°F", "°F"),
    UnitCelsius => ("°C", "°C"),
    UnitFeet => ("ft", "ft"),
    UnitMetres => ("m", "m"),
    UnitKnots => ("kn", "kn"),
    UnitMph => ("mph", "mph"),
    UnitKmh => ("km/h", "km/h"),
    UnitMetresPerSecond => ("m/s", "m/s"),
    UnitCfs => ("cfs", "cfs"),
    UnitCubicMetresPerSecond => ("m³/s", "m³/s"),
    UnitHpa => ("hPa", "hPa"),
    UnitInHg => ("inHg", "inHg"),
    UnitMmHg => ("mmHg", "mmHg"),
    UnitKpa => ("kPa", "kPa"),
    UnitKilometres => ("km", "km"),
    UnitMiles => ("mi", "mi"),
    UnitFnu => ("FNU", "FNU"),
    UnitFtu => ("FTU", "FTU"),
    UnitNtu => ("NTU", "NTU"),
    UnitSeconds => ("s", "s"),
    UnitMicrogramsPerLitre => ("µg/L", "µg/L"),
    UnitPerMetre => ("1/m", "1/m"),
    UnitInches => ("in", "in"),
    UnitCentimetres => ("cm", "cm"),
    UnitPounds => ("lb", "lb"),
    UnitOunces => ("oz", "oz"),
    UnitKilograms => ("kg", "kg"),
    UnitGrams => ("g", "g"),
    UnitMillimetres => ("mm", "mm"),
    /// A height's unit with its datum: `{unit}`, `{datum}`: "ft MLLW".
    UnitWithDatum => ("{unit} {datum}", "{unit} {datum}"),
    // -- vertical datums ----------------------------------------------------------------------
    DatumMllw => ("MLLW", "MLLW"),
    DatumMlw => ("MLW", "MLW"),
    DatumMsl => ("MSL", "MSL"),
    DatumMtl => ("MTL", "MTL"),
    DatumNavd88 => ("NAVD88", "NAVD88"),
    DatumNgvd29 => ("NGVD29", "NGVD29"),
    DatumIgld85 => ("IGLD85", "IGLD85"),
    DatumGauge => ("gage datum", "gage datum"),
    DatumChsChart => ("chart datum (CHS)", "chart datum (CHS)"),
    DatumModelMean => ("model mean", "model mean"),
    // -- compass points, clockwise from north --------------------------------------------------
    CompassN => ("N", "N"),
    CompassNne => ("NNE", "NNE"),
    CompassNe => ("NE", "NE"),
    CompassEne => ("ENE", "ENE"),
    CompassE => ("E", "E"),
    CompassEse => ("ESE", "ESE"),
    CompassSe => ("SE", "SE"),
    CompassSse => ("SSE", "SSE"),
    CompassS => ("S", "S"),
    CompassSsw => ("SSW", "SSW"),
    CompassSw => ("SW", "SW"),
    CompassWsw => ("WSW", "WSW"),
    CompassW => ("W", "W"),
    CompassWnw => ("WNW", "WNW"),
    CompassNw => ("NW", "NW"),
    CompassNnw => ("NNW", "NNW"),
    /// How steep the waves are, as NDBC grades it.
    SteepnessVerySteep => ("Very steep", "Very steep"),
    SteepnessSteep => ("Steep", "Steep"),
    SteepnessAverage => ("Average", "Average"),
    SteepnessSwell => ("Swell", "Swell"),
    // -- times and dates ----------------------------------------------------------------------
    /// A 12-hour time: `{time}` ("7:03", "4").
    TimeAm => ("{time} AM", "{time} AM"),
    TimePm => ("{time} PM", "{time} PM"),
    MonthJan => ("Jan", "Jan"),
    MonthFeb => ("Feb", "Feb"),
    MonthMar => ("Mar", "Mar"),
    MonthApr => ("Apr", "Apr"),
    MonthMay => ("May", "May"),
    MonthJun => ("Jun", "Jun"),
    MonthJul => ("Jul", "Jul"),
    MonthAug => ("Aug", "Aug"),
    MonthSep => ("Sep", "Sep"),
    MonthOct => ("Oct", "Oct"),
    MonthNov => ("Nov", "Nov"),
    MonthDec => ("Dec", "Dec"),
    /// A span of time: `{n}`.
    DurationMinutes => ("{n} min", "{n} min"),
    DurationHours => ("{n} h", "{n} h"),
    DurationDays => ("{n} d", "{n} d"),
    /// An ordinal number: `{n}`. English picks the suffix by the last digits (1st, 2nd, 3rd,
    /// 11th); a language with one form gives all four the same text.
    OrdinalSt => ("{n}st", "{n}st"),
    OrdinalNd => ("{n}nd", "{n}nd"),
    OrdinalRd => ("{n}rd", "{n}rd"),
    OrdinalTh => ("{n}th", "{n}th"),
    // -- the trip view --------------------------------------------------------------------
    /// What a trip cell rests on. Astronomy and harmonic tides hold for any date.
    TripBasisCertain => ("certain", "certain"),
    /// `{when}`: the issue date and time.
    TripBasisForecast => ("forecast, issued {when}", "forecast, issued {when}"),
    /// Past years' values: `{first}`, `{last}` (years), `{kind}` (a history kind below).
    TripBasisTypical => (
        "typical for these dates, {first}–{last}, {kind}",
        "typical for these dates, {first}–{last}, {kind}",
    ),
    /// Past years' values when the source gives only a count: `{n}`, `{kind}`.
    TripBasisTypicalCount => (
        "typical for these dates, {n} years, {kind}",
        "typical for these dates, {n} years, {kind}",
    ),
    /// `{reason}`: one of the reasons below.
    TripBasisUnavailable => ("unavailable: {reason}", "unavailable: {reason}"),
    TripHistoryStation => ("station records", "station records"),
    TripHistoryReanalysis => ("reanalysis", "reanalysis"),
    TripHistoryModel => ("model archive", "model archive"),
    TripWhyNoLocation => ("no location for this place", "no location for this place"),
    TripWhyNoSource => ("no source here", "no source here"),
    TripWhyNoForecast => ("no forecast yet", "no forecast yet"),
    TripWhyBeyondForecast => ("outside the forecast", "outside the forecast"),
    TripWhyNoHistory => ("no history for these dates", "no history for these dates"),
    TripWhyPast => ("this day is over", "this day is over"),
    TripWhyDisabled => ("turned off in settings", "turned off in settings"),
    TripWhyNoCoverage => (
        "no tide predictions for this day",
        "no tide predictions for this day",
    ),
    /// An instant: `{date}`, `{time}`.
    TripDateTime => ("{date}, {time}", "{date}, {time}"),
    /// `{n}`: windows after the first.
    TripMoreWindows => ("{n} more", "{n} more"),
    TripWindowsCertainOnly => (
        "From tide, light and solunar only",
        "From tide, light and solunar only",
    ),
    /// A high or low: `{kind}`, `{time}`.
    TripTideEvent => ("{kind} {time}", "{kind} {time}"),
    /// `{kind}`, `{time}`, `{height}` (with its unit and datum).
    TripTideEventHeight => ("{kind} {time}, {height}", "{kind} {time}, {height}"),
    TripFlowInBest => ("In your best range", "In your best range"),
    TripFlowAboveBest => ("Above your best range", "Above your best range"),
    TripFlowBelowBest => ("Below your best range", "Below your best range"),
    TripFlowBlownOut => ("Above your blown-out level", "Blown out"),
    TripWadeable => ("Wadeable", "Wadeable"),
    TripNotWadeable => ("Above your wadeable level", "Too high to wade"),
    /// The 10th to 90th percentile of past years: `{low}`, `{high}` (with units).
    TripMostYears => ("Most years {low} to {high}", "Most years {low} to {high}"),
    TripTempBelow => (
        "Below the species' preferred range",
        "Colder than the fish like it",
    ),
    TripTempWithin => (
        "In the species' preferred range",
        "Just how the fish like it",
    ),
    TripTempAbove => (
        "Above the species' preferred range",
        "Warmer than the fish like it",
    ),
    /// `{percent}`: the lit fraction.
    TripMoonLit => ("{percent} lit", "{percent} lit"),
    /// A solunar period: `{label}`, `{range}`.
    TripSolunarPeriod => ("{label} {range}", "{label} {range}"),
    /// What `VoiceOver` reads for a cell: `{label}`, `{basis}`, `{value}`.
    TripCellAcc => ("{label}, {basis}: {value}", "{label}, {basis}: {value}"),
    /// A cell with nothing to show: `{label}`, `{basis}`.
    TripCellAccNone => ("{label}, {basis}", "{label}, {basis}"),
    /// A place on a day: `{place}`, `{day}`, `{cells}`.
    TripPlaceDayAcc => ("{place}, {day}: {cells}", "{place}, {day}: {cells}"),
    TripApproximateLocation => ("Approximate location", "Approximate location"),
    ShareFuzzTitle => ("Location in the file", "Location in the file"),
    ShareFuzzExact => ("Exact", "Exact"),
    ShareFuzzExactDetail => ("The coordinates as you saved them.", "The coordinates as you saved them."),
    ShareFuzzRounded => ("About 1 km", "About 1 km"),
    ShareFuzzRoundedDetail => ("Rounded to about a kilometre.", "Rounded to about a kilometre."),
    ShareFuzzAreaOnly => ("Area only", "Area only"),
    ShareFuzzAreaOnlyDetail => (
        "The place's name and area, no coordinates.",
        "The place's name and area, no coordinates.",
    ),
    ReceiveTrip => ("Trip to add", "Trip to add"),
    ReceiveSpot => ("Spot to add", "Spot to add"),
    ReceiveTripExists => ("This trip is already on this device.", "This trip is already on this device."),
    ReceiveSpotExists => ("This spot is already in your spots.", "This spot is already in your spots."),
    ReceiveAdd => ("Add", "Add"),
    ReceiveAddCopy => ("Add as a copy", "Add as a copy"),
    ReceiveReplace => ("Replace mine", "Replace mine"),
    ReceiveAdded => ("Added", "Added"),
    TripProblemTimeZone => (
        "Unknown time zone: no days for this place",
        "Unknown time zone: no days for this place",
    ),
    TripProblemDates => (
        "The trip's dates cannot be used",
        "The trip's dates cannot be used",
    ),
    /// `{n}`: days until the first day.
    TripStartsInDays => ("Starts in {n} days", "{n} days to go"),
    TripStartsTomorrow => ("Starts tomorrow", "Starts tomorrow"),
    TripUnderWay => ("Under way", "Under way"),
    TripOver => ("This trip is over", "This trip is over"),
    /// The widget's next trip: `{name}`.
    TripNext => ("Next trip: {name}", "Next trip: {name}"),
    TripNoVerdict => ("No verdict yet", "Luna's waiting for the forecast"),
    /// `{day}`, `{place}`, `{summary}`.
    TripCompactLine => ("{day} at {place}: {summary}", "{day} at {place}: {summary}"),
    TripOfflineNone => ("Offline pack: not downloaded", "Offline pack: not downloaded"),
    /// `{size}`, `{date}`.
    TripOfflineReady => ("Offline pack: {size}, saved {date}", "Offline pack: {size}, saved {date}"),
    TripNotShared => ("Not shared", "Not shared"),
    TripSharedReadOnly => ("Shared live, read-only", "Shared live, read-only"),
    TripSharedEditable => ("Shared live, others can edit", "Shared live, others can edit"),
    /// The gear template of each method.
    GearRodsTackle => ("Rods, reels and tackle", "Rods, reels and tackle"),
    GearPfd => ("Life jacket", "Life jacket"),
    GearWhistle => ("Whistle", "Whistle"),
    GearVhf => ("VHF radio", "VHF radio"),
    GearAnchor => ("Anchor and line", "Anchor and line"),
    GearPaddleLeash => ("Paddle and leash", "Paddle and leash"),
    GearDryBag => ("Dry bag for phone and keys", "Dry bag for phone and keys"),
    GearLight => ("Headlamp or navigation light", "Headlamp or navigation light"),
    GearPliers => ("Pliers and line cutter", "Pliers and line cutter"),
    GearRainJacket => ("Rain jacket", "Rain jacket"),
    GearWaders => ("Waders and wading belt", "Waders and wading belt"),
    GearWadingBoots => ("Wading boots", "Wading boots"),
    GearWadingStaff => ("Wading staff", "Wading staff"),
    GearIcePicks => ("Ice picks", "Ice picks"),
    GearIceCleats => ("Ice cleats", "Ice cleats"),
    GearAuger => ("Auger", "Auger"),
    GearSpud => ("Spud bar to test the ice", "Spud bar to test the ice"),
    GearSnacks => ("Water and snacks", "Snacks (Luna insists)"),
    // -- alert rules, their states and their notifications ----------------------------------
    AlertMetricDischarge => ("River flow", "River flow"),
    AlertMetricGageHeight => ("Gage height", "Gage height"),
    AlertMetricWaterTemperature => ("Water temperature", "Water temperature"),
    AlertMetricSeaSurfaceTemp => ("Sea surface temperature", "Sea surface temperature"),
    AlertMetricWaveHeight => ("Wave height", "Wave height"),
    AlertMetricWind => ("Wind", "Wind"),
    AlertMetricGusts => ("Gusts", "Gusts"),
    AlertMetricPressureTrend => ("Pressure change in 3 h", "Pressure change in 3 h"),
    AlertMetricTideHeight => ("Tide height", "Tide height"),
    AlertMetricChlorophyll => ("Chlorophyll", "Chlorophyll"),
    /// A notification's title when the rule does not show names, by target.
    AlertNameSpot => ("Alert for your spot", "Alert for your spot"),
    AlertNameTrip => ("Alert for your trip", "Alert for your trip"),
    AlertNameArea => ("Alert for your area", "Alert for your area"),
    /// A place a notification does not name, by target.
    AlertPlaceSpot => ("Your spot", "Your spot"),
    AlertPlaceTrip => ("Your trip", "Your trip"),
    AlertPlaceArea => ("Your area", "Your area"),
    /// `{rule}`: the rule's name, or a title above.
    AlertNotifyRepeat => ("{rule}: still met", "{rule}: still met"),
    AlertNotifyUpdate => ("{rule}: update", "{rule}: update"),
    /// `{place}`: a station, source or place; `{what}`: what is measured; `{value}`,
    /// `{threshold}`: "780 cfs", "800 cfs".
    AlertNotifyBelow => (
        "{place}: {what} {value}, under your {threshold}",
        "{place}: {what} {value}, under your {threshold}",
    ),
    AlertNotifyAbove => (
        "{place}: {what} {value}, over your {threshold}",
        "{place}: {what} {value}, over your {threshold}",
    ),
    /// `{low}`, `{high}`: the rule's range.
    AlertNotifyBetween => (
        "{place}: {what} {value}, between your {low} and {high}",
        "{place}: {what} {value}, between your {low} and {high}",
    ),
    AlertNotifyValue => ("{place}: {what} {value}", "{place}: {what} {value}"),
    /// A finding the rule, edited since, no longer describes. `{place}`.
    AlertNotifyMet => ("{place}: your rule is met", "{place}: your rule is met"),
    /// `{place}`, `{range}`: "6:10 to 8:10 AM".
    AlertNotifyWindow => (
        "{place}: fishing window {range}",
        "{place}: snack time {range}",
    ),
    /// `{place}`, `{time}`: when the window ends.
    AlertNotifyWindowNow => (
        "{place}: fishing window now, until {time}",
        "{place}: snack time now, until {time}",
    ),
    /// `{place}`, `{day}`: "Nov 14".
    AlertNotifyTripGo => ("{place}: {day} looks good", "{place}: Luna approves of {day}"),
    /// A time on another day than today. `{date}`, `{time}`.
    AlertDayTime => ("{date}, {time}", "{date}, {time}"),
    /// A rule's conditions for the editor. `{what}`, `{threshold}`.
    AlertRuleBelow => ("{what} under {threshold}", "{what} under {threshold}"),
    AlertRuleAbove => ("{what} over {threshold}", "{what} over {threshold}"),
    /// `{what}`, `{low}`, `{high}`.
    AlertRuleBetween => (
        "{what} between {low} and {high}",
        "{what} between {low} and {high}",
    ),
    /// `{what}`.
    AlertRuleNoThreshold => ("{what}, no threshold set", "{what}, no threshold set"),
    AlertRuleTripGo => ("A go day on the trip", "A day Luna approves of on the trip"),
    /// `{duration}`: "30 min".
    AlertRuleWindow => (
        "A fishing window within {duration}",
        "A snack time within {duration}",
    ),
    /// Two conditions, or one and the rest. `{a}`, `{b}`.
    AlertRuleJoin => ("{a} · {b}", "{a} · {b}"),
    AlertRuleNoConditions => ("No conditions", "No conditions"),
    /// What a rule watches, when the editor cannot name it.
    AlertTargetSpotGone => ("A deleted spot", "A deleted spot"),
    AlertTargetTripGone => ("A deleted trip", "A deleted trip"),
    AlertTargetBox => ("An area on the map", "An area on the map"),
    /// `{duration}`: "12 h".
    AlertRuleRepeat => (
        "Again every {duration} while met",
        "Again every {duration} while met",
    ),
    /// `{range}`: "10:00 PM to 7:00 AM".
    AlertRuleQuiet => ("Quiet hours {range}", "Quiet hours {range}"),
    /// A rule's state on the alerts card. `{time}`.
    AlertRuleMetSince => ("Met since {time}", "Met since {time}"),
    AlertRuleMet => ("Met", "Met"),
    AlertRuleNotMet => ("Not met", "Not met"),
    /// `{what}`: what could not be checked; `{age}`: "5 h".
    AlertRuleStale => (
        "{what}: couldn't check, data {age} old",
        "{what}: couldn't check, data {age} old",
    ),
    /// `{what}`, `{distance}`.
    AlertRuleFar => (
        "{what}: couldn't check, the station is {distance} away",
        "{what}: couldn't check, the station is {distance} away",
    ),
    /// `{what}`.
    AlertRuleFailed => (
        "{what}: couldn't check, the source didn't answer",
        "{what}: couldn't check, the source didn't answer",
    ),
    /// `{what}`.
    AlertRuleNoData => ("{what}: no data", "{what}: no data"),
    /// What could not be checked when the rule no longer has that condition.
    AlertRuleACondition => ("A condition", "A condition"),
    AlertRuleOrphaned => ("Its spot or trip was deleted", "Its spot or trip was deleted"),
    AlertRuleNotChecked => ("Not checked yet", "Not checked yet"),
    AlertRuleOff => ("Off", "Off"),
    /// `{time}`.
    AlertRuleChecked => ("Checked {time}", "Checked {time}"),
    AlertRulesTitle => ("Your alert rules", "Your alert rules"),
    /// `{met}`, `{count}`.
    AlertRulesMet => ("Rules met: {met} of {count}", "Rules met: {met} of {count}"),
    /// One rule for `VoiceOver`: `{name}`, `{state}`.
    AlertRuleAcc => ("{name}: {state}", "{name}: {state}"),
    /// `{rules}`: the rules met; `{list}`: each rule.
    AlertRulesAcc => ("{rules}. {list}.", "{rules}. {list}."),
    /// The alerts card with rules and no alert in effect: `{title}`, `{rules}`.
    AlertsRulesOnlyAcc => ("{title}. {rules}", "{title}. {rules}"),
    /// A threshold field's errors. `{example}`: "12.5".
    AlertThresholdNumber => (
        "Enter a number, such as {example}",
        "Enter a number, such as {example}",
    ),
    /// `{min}`, `{max}`.
    AlertThresholdRange => (
        "Enter a value from {min} to {max}",
        "Enter a value from {min} to {max}",
    ),
}

/// Declares [`SafetyKey`], its list and its table in one place.
macro_rules! safety_keys {
    ($($(#[$doc:meta])* $name:ident => $text:expr,)*) => {
        /// Texts that are never playful: safety, advisories and regulations. No voice parameter
        /// exists for them.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum SafetyKey {
            $($(#[$doc])* $name,)*
        }

        impl SafetyKey {
            /// Every key, for tests and for a later localisation.
            pub const ALL: &'static [Self] = &[$(Self::$name,)*];

            /// The one text.
            pub const fn text(self) -> &'static str {
                match self {
                    $(Self::$name => $text,)*
                }
            }
        }
    };
}

safety_keys! {
    NotForNavigation => "Not for navigation.",
    Modelled => "Modelled data: an estimate, not a measurement.",
    /// The tide card's note when the source is a sea-level model, not a tide prediction.
    ModelledSeaLevel => "Modelled sea level, not a tide prediction; times can be off by hours near the coast.",
    Provisional => "Provisional data, subject to revision.",
    Approximate => "Approximate: check conditions locally before you go.",
    TraditionalCue => "A traditional cue, not a promise.",
    TextOnly => "Text only: open the map for the picture.",
    RegulationsLink => "Check the official source for current regulations; this is not legal advice.",
    /// `{threshold}`: the user's trout-stress temperature.
    TroutStress => "Consider not fishing for trout above {threshold}.",
    /// When an alert is in effect. `{start}`, `{end}`: "Nov 8, 1:48 AM".
    AlertSpan => "{start} – {end}",
    /// `{start}`.
    AlertFrom => "From {start}",
    /// `{end}`.
    AlertUntil => "Until {end}",
    VerdictNoGo => "No-go",
    /// Hard no-gos on the go/no-go card. `{alert}`: the alert's event name as issued.
    NoGoAlert => "No-go: {alert} in effect",
    /// `{alert}`, `{span}`.
    NoGoAlertSpan => "No-go: {alert}, {span}",
    /// The alert's name when the source gave none.
    AlertNameSmallCraft => "Small craft advisory",
    AlertNameMarine => "Marine warning",
    NoGoLightning => "No-go: thunderstorms in the forecast, a lightning risk on the water",
    NoGoStorm => "No-go: a significant storm is forecast",
    /// `{span}`.
    NoGoStormSpan => "No-go: a significant storm is forecast, {span}",
    /// `{name}`, `{span}`.
    NoGoHurricane => "No-go: the forecast cone of {name} covers this spot, {span}. See the National Hurricane Center for the official forecast.",
    /// The checks that safety limits ask for, in "Missing: ...".
    CheckSmallCraft => "small craft advisories",
    CheckMarineWarning => "marine warnings",
    CheckLightning => "lightning",
    CheckStormAhead => "storm outlook",
    CheckHurricane => "hurricane outlook",
    /// An app file that could not be read, moved aside. `{name}`: the name it was kept as.
    FileKeptSpots => "Your spots could not be read. The file was kept as {name} and the list starts empty; nothing is saved over the kept file.",
    /// `{name}`.
    FileKeptSettings => "Your settings could not be read. The file was kept as {name} and the default settings are used; nothing is saved over the kept file.",
    /// An app file that could not be read and could not be moved: it stays where it was.
    FileLeftSpots => "Your spots could not be read. The file was left as it was and is never saved over, so changes to spots are not saved.",
    FileLeftSettings => "Your settings could not be read. The file was left as it was and is never saved over, so changes to settings are not saved.",
    /// How serious an alert is, as the alert source grades it.
    SeverityExtreme => "Extreme",
    SeveritySevere => "Severe",
    SeverityModerate => "Moderate",
    SeverityMinor => "Minor",
    SeverityUnknown => "Unknown severity",
    /// `{amount}`: "14 in".
    StormSnow => "Up to {amount} of snow",
    StormRain => "Up to {amount} of rain",
    StormGusts => "Gusts to {amount}",
    StormAgreementHigh => "High agreement",
    StormAgreementMedium => "Medium agreement",
    StormAgreementLow => "Low agreement",
    /// `{wet}`, `{wet_amount}`, `{noun}`, `{dry}`, `{dry_amount}`.
    StormBrings => "{wet} brings {wet_amount} {noun}; {dry} {dry_amount}.",
    StormOfSnow => "of snow",
    StormOfRain => "of rain",
    /// A part of a day: `{day}` ("Sun").
    StormPartMorning => "{day} morning",
    StormPartAfternoon => "{day} afternoon",
    StormPartNight => "{day} night",
    /// `{window}`.
    StormTitle => "Storm ahead · {window}",
    /// `{window}`.
    StormAlso => "Also a storm ahead · {window}",
    /// `{title}`, `{reaches}`, `{second}`.
    StormHurricaneAcc => "Storm ahead: {title}. {reaches}. {second}.",
    StormModelsDiffer => "models differ",
    StormModelsDisagree => "models disagree",
    /// `{hours}`.
    StormStale => "Checked {hours} h ago; forecasts may have changed.",
    /// `{date}`, `{time}`.
    StormChecked => "Storm outlook checked {date}, {time} · modelled",
    /// A storm by its kind: `{name}` ("Josephine").
    HurricaneHurricane => "Hurricane {name}",
    HurricaneTropicalStorm => "Tropical Storm {name}",
    HurricaneTropicalDepression => "Tropical Depression {name}",
    HurricaneSubtropicalStorm => "Subtropical Storm {name}",
    HurricaneSubtropicalDepression => "Subtropical Depression {name}",
    HurricanePostTropical => "Post-Tropical Cyclone {name}",
    HurricanePotentialTropical => "Potential Tropical Cyclone {name}",
    HurricaneTyphoon => "Typhoon {name}",
    HurricaneCyclone => "Tropical Cyclone {name}",
    /// `{name}`, `{category}`.
    HurricaneCategory => "{name} · Category {category}",
    StormStationary => "Stationary",
    /// `{direction}`, `{speed}`.
    StormMovement => "{direction} at {speed}",
    StormInCone => "This spot is in the forecast cone",
    /// `{date}`.
    StormConeReaches => "The forecast cone reaches this spot on {date}",
    /// `{wind}`, `{pressure}`, `{movement}`.
    StormStrength => "Winds {wind} · {pressure} · {movement}",
    /// `{day}`, `{time}`.
    StormAdvisory => "Advisory {day} {time}",
    StormShownWhen => "Shown when",
    /// `{snow}`, `{precipitation}`, `{gust}`, `{days}`.
    StormShownWhenText => "the models' average reaches about {snow} of snow, {precipitation} of precipitation or gusts of {gust}, within {days} days",
    StormHurricanesLabel => "Hurricanes",
    StormHurricanesText => "NHC's forecast cone reaches this spot within 5 days",
    StormSectionWhy => "Why you see this",
    StormSectionSafety => "Safety",
    StormForecastsChange => "Forecasts change",
    StormForecastsChangeText => "Storm forecasts change from one run to the next and can be wrong about where a storm goes. Check the National Weather Service and the National Hurricane Center, and follow local officials, before you go on or near the water.",
    // -- the map: credits, depth references and terms ---------------------------------------
    MapCreditApple => "Map data: Apple Maps",
    /// The NOAA chart and the Chart Display pictures.
    MapCreditNcds => "NOAA Office of Coast Survey, NOAA Chart Display Service (ENC). Not to be used for navigation.",
    MapCreditEncDirect => "NOAA Office of Coast Survey, ENC Direct to GIS. Not to be used for navigation.",
    /// As GEBCO's WMS states it for the 2026 grid; a release changes the year.
    MapCreditGebco => "Imagery reproduced from the GEBCO_2026 Grid, GEBCO Compilation Group (2026) GEBCO 2026 Grid (doi:10.5285/4f68d5c7-45eb-f999-e063-7086abc036fa)",
    MapCreditMurErddap => "SST: NASA JPL MUR (MEaSUREs), via NOAA CoastWatch ERDDAP",
    MapCreditMurGibs => "SST: NASA JPL MUR (MEaSUREs), via NASA GIBS",
    MapCreditViirsColour => "Ocean colour: NOAA CoastWatch / NOAA STAR VIIRS",
    MapCreditGibsChlorophyll => "Ocean colour: NASA GIBS, NOAA-20 VIIRS",
    MapCreditClarity => "Water clarity: NOAA CoastWatch / NOAA STAR VIIRS",
    MapCreditGibsViirsN20 => "Imagery: NASA GIBS, NOAA-20 VIIRS",
    MapCreditGibsViirsN21 => "Imagery: NASA GIBS, NOAA-21 VIIRS",
    MapCreditGibsModisTerra => "Imagery: NASA GIBS, MODIS Terra",
    MapCreditGibsModisAqua => "Imagery: NASA GIBS, MODIS Aqua",
    MapCreditHfRadar => "Surface currents: NOAA IOOS HF Radar Network",
    MapCreditGlerl => "NOAA Great Lakes Environmental Research Laboratory (CoastWatch Great Lakes Node)",
    MapCreditRadar => "Radar: NOAA NEXRAD via Iowa Environmental Mesonet",
    MapCreditWind => "Wind: no source yet",
    MapCreditPressure => "Pressure: no source yet",
    MapCreditWaves => "Waves: no source yet",
    MapCreditTides => "Tides: NOAA CO-OPS",
    MapCreditCurrents => "Currents: NOAA CO-OPS",
    MapCreditUsgs => "River data: USGS",
    MapCreditBuoys => "Buoys: NOAA National Data Buoy Center",
    MapCreditNwps => "River forecasts: NOAA National Water Prediction Service",
    MapCreditSpots => "Your saved spots, kept on this device",
    MapCreditContoursIn => "Lake contours: Indiana DNR, Division of Fish and Wildlife. Not for navigation.",
    MapCreditContoursMn => "Lake contours: Minnesota Department of Natural Resources (MNDNR). Not for navigation.",
    MapCreditReefsNy => "Artificial reefs: New York State Department of Environmental Conservation. Not for navigation.",
    /// The full GIBS acknowledgement for the sources page; layers carry the short credit.
    MapAckGibs => "We acknowledge the use of imagery provided by services from NASA's Global Imagery Browse Services (GIBS), part of NASA's Earth Science Data and Information System (ESDIS).",
    /// The extra acknowledgement of the MUR sea surface temperature layers.
    MapAckMur => "These data were provided by JPL under support by NASA MEaSUREs program.",
    /// Legend captions that say what a depth or chart layer shows.
    MapLegendGebco => "Offshore depth where the chart has none",
    MapLegendContoursIn => "Depth below the lake surface at survey",
    MapLegendContoursMn => "Depth below the lake surface on the original map",
    MapLegendReefsNy => "General area of each reef site",
    MapLegendEncDepths => "Charted depth areas, contours and soundings, in the chart's own units and datum. Soundings appear at the finer zooms; the exact limit is unverified.",
    MapLegendEncSeabed => "Charted wrecks, obstructions and foul areas. Empty when zoomed out past about 1:137,000 where checked; the limit follows each chart's scale.",
    MapLegendEncSpecialAreas => "Charted restricted and caution areas. The zoom at which they appear is unverified. Rules are in the regulations, not here.",
    MapLegendEncStructurePoints => "Charted wrecks, obstructions and rocks with their category and survey date. No depth is shown: NOAA does not state its unit or datum.",
    /// What a state layer's depths are measured from, as the source states it. Never the DEM's
    /// "approximate depth": contours are below a lake surface, not a datum.
    MapReferenceContoursIn => "Feet below the lake surface; the contours of some lakes are adjusted to the lake's legal gauge level. Survey date per lake.",
    MapReferenceContoursMn => "Depth below the lake surface on the original lake map, in feet (unit per DNR DEM metadata, unconfirmed for the contour layer). Survey dates vary by lake.",
    MapReferenceReefsNy => "Depths as NYSDEC lists them (minimum and maximum, in feet); no datum is stated. A site is the general area where material was placed.",
    /// A state layer's terms, as far as they are known.
    MapLicenceContoursIn => "Terms for the GIS layer are not stated; credit Indiana DNR, Division of Fish and Wildlife.",
    MapLicenceContoursMn => "MNDNR must be acknowledged; the data may not be sold without MNDNR's written permission.",
    MapLicenceReefsNy => "Terms are not stated; credit the New York State Department of Environmental Conservation.",
    // Depth on the map
    /// The depth inspector row's label.
    MapDepthRow => "Approximate depth",
    /// The depth row's value: `{depth}` ("97 ft"), `{datum}` ("EGM 2008").
    MapDepthApprox => "approximate depth {depth} ({datum})",
    /// As above with the elevation model that gave the value: `{model}` ("Coastal Relief Model").
    MapDepthApproxModel => "approximate depth {depth} ({datum}, {model})",
    /// A lake bed's height above the model's datum, where no lake level is known: `{height}`.
    MapDepthBed => "bed elevation {height} ({datum})",
    MapDepthBedModel => "bed elevation {height} ({datum}, {model})",
    MapDepthDatumUnknown => "datum not stated",
    MapDepthModelCrm => "Coastal Relief Model",
    MapDepthModelCudem => "CUDEM",
    MapDepthModelEtopo => "ETOPO 2022",
    MapCreditBathymetry => "Bathymetry: NOAA NCEI (DEM global mosaic, Coastal Relief Model, CUDEM). Not for navigation.",
    MapReferenceBathymetry => "Heights are metres relative to a vertical datum that depends on the source model (NAVD 88 or EGM 2008), not chart datum. They are shown as approximate depth and are never compared with charted soundings or tide heights.",
    MapLegendBathymetry => "Approximate depth below the surface",
    /// Why a charted wreck, obstruction or rock shows no depth.
    MapStructureDepthNotShown => "Charted depth not shown: the service states neither its unit nor its datum.",
    // Station pins
    /// A forecast gauge's observed flood category, as NOAA's National Water Prediction Service
    /// grades it.
    PinNoFlooding => "No flooding",
    PinActionStage => "Action stage",
    PinMinorFlooding => "Minor flooding",
    PinModerateFlooding => "Moderate flooding",
    PinMajorFlooding => "Major flooding",
    PinLowWater => "Low water",
    PinOutOfService => "Out of service",
    // -- sources and their terms ------------------------------------------------------------
    /// A source's name as displayed, by its id.
    SourceUsgs => "USGS",
    SourceCoops => "NOAA CO-OPS",
    SourceCoopsHarmonics => "NOAA CO-OPS harmonics",
    SourceNdbc => "NOAA NDBC",
    SourceNwps => "NOAA NWPS",
    SourceOpenMeteoMarine => "Open-Meteo Marine",
    SourceNws => "National Weather Service",
    /// The bloom card's words: health and satellite wording, plain in every voice.
    BloomAdvisory => "Algal blooms can make people and dogs sick. Keep people and pets out of green, scummy or discoloured water, and follow local health advisories.",
    /// EPA's description of the `CyAN` web application, kept in its own words.
    BloomCyanExperimental => "Experimental: provisional satellite-derived measures of cyanobacteria, which may contain errors and should be considered a research tool. Not a health advisory.",
    /// `{value}`: "9,000 cells/mL"; `{date}`: the composite's date.
    BloomCyanMax => "Satellite estimate up to {value} (composite of {date})",
    BloomNoRecentSatellite => "No recent satellite data",
    /// `{date}`: the last composite's date.
    BloomNoRecentSatelliteLast => "No recent satellite data (last composite {date})",
    BloomNoRecentForecast => "No recent bloom forecast",
    /// `{level}`, `{beach}`, `{distance}`.
    BloomForecastLine => "Respiratory irritation {level} at {beach}, {distance}",
    BloomLevelVeryLow => "very low",
    BloomLevelLow => "low",
    BloomLevelModerate => "moderate",
    BloomLevelHigh => "high",
    BloomLevelUnknown => "unknown",
    /// `{value}`: the source's weighting, 0 to 1.
    BloomConfidence => "{value} of 1",
    SourceUsgs3dhp => "USGS 3DHP",
    SourceGlerl => "NOAA GLERL",
    SourceCwms => "US Army Corps of Engineers",
    SourceRise => "US Bureau of Reclamation",
    SourceCyan => "EPA CyAN",
    SourceNoaaHab => "NOAA NCCOS bloom forecast",
    SourceNyDecStocking => "NYS DEC stocking",
    SourceOverpass => "OpenStreetMap",
    SourcePadus => "USGS PAD-US",
    SourceGbif => "GBIF",
    SourceInaturalist => "iNaturalist",
    SourceUsgsNas => "USGS Nonindigenous Aquatic Species",
    SourceDfoIwls => "Canadian Hydrographic Service",
    SourceEcccHydrometric => "Environment and Climate Change Canada",
    /// The credit of a USGS value whose snapshot carries no attribution.
    CreditUsgs => "U.S. Geological Survey",
    SourceOpenMeteo => "Open-Meteo",
    SourceNhc => "NOAA National Hurricane Center",
    /// The storm card's credits, as the sources ask to be credited.
    CreditOpenMeteoModels => "Weather model data by Open-Meteo.com, with ECMWF, NOAA GFS, GEM and ICON forecasts",
    CreditNhc => "Advisories, forecast tracks and the forecast cone: NOAA National Hurricane Center",
    /// The credits of tide sources, marine forecasts and alerts whose snapshot carries none.
    CreditTideCoops => "Predictions and observations: NOAA CO-OPS",
    CreditTideHarmonics => "Computed from NOAA harmonic constants",
    CreditTideOpenMeteo => "Sea level model: Open-Meteo",
    CreditMarineForecast => "Marine forecast: NOAA National Weather Service",
    CreditMarineAlerts => "Marine alerts: NOAA National Weather Service",
    /// The standard credit lines of the station sources and the wave model.
    CreditObservationsNdbc => "Observations: NOAA National Data Buoy Center",
    CreditObservationsCoops => "Observations: NOAA CO-OPS",
    CreditWaveModel => "Wave model: Open-Meteo.com",
    /// The sources card's credit lines, as the sources ask to be credited.
    CreditCoops => "Tide, water-level and current data: NOAA/NOS CO-OPS",
    CreditHarmonics => "Computed on this device from NOAA CO-OPS harmonic constants",
    CreditNwps => "River forecasts: NOAA National Water Prediction Service",
    CreditNws => "Marine forecasts and alerts: NOAA National Weather Service",
    CreditOpenMeteo => "Weather data by Open-Meteo.com",
    /// What Open-Meteo's model providers are called as a credit.
    SourceModelProviders => "Open-Meteo model providers",
    /// The model providers Open-Meteo's marine data comes from, as its terms ask them credited.
    CreditMarineModels => "Marine models: Copernicus Marine Service (MeteoFrance MFWAM, SMOC), ECMWF (WAM), NOAA NCEP (GFS Wave), DWD (EWAM, GWAM), Copernicus Climate Change Service (ERA5-Ocean)",
    LicencePublicDomain => "Public domain",
    LicenceCcBy4 => "CC BY 4.0",
    /// Beside regulation and licence links: the app links to the official pages and states no
    /// rule itself.
    /// The trip checklist's regulation and licence links.
    RegFreshwater => "Freshwater regulations",
    RegSaltwater => "Saltwater regulations",
    RegFederal => "Federal waters regulations",
    RegLicence => "Fishing licence",
    /// `{agency}`, `{date}` (`YYYY-MM-DD`).
    RegChecked => "{agency}, link checked {date}",
    RegUnconfirmed => "{agency}, link not confirmed when checked {date}",
    /// Before the catch log is shared as a file.
    LogExportWarning => "This file holds the exact place of every catch. Share it only with people you trust with those places.",
    RegulationsDisclaimer => "Fishing regulations change during the year. Underbite does not state limits, seasons or licence rules; open the official page and check it before you fish.",
    /// The licences of species profile values.
    LicenceCcByNc4 => "CC BY-NC 4.0",
    LicencePublished => "Cited from published work",
    LicenceUserEdited => "Typed in, not from a source",
    /// Beside a profile with `FishBase` values, at home and in a shared file.
    SpeciesFishBaseNotice => "Values licensed CC BY-NC come from FishBase (www.fishbase.org) and may be used only non-commercially, with their citation.",
    /// The licences of species records and access data, without a version where the source
    /// states none.
    LicenceCc0 => "CC0",
    LicenceCcBy => "CC BY",
    LicenceCcByNc => "CC BY-NC",
    LicenceOdbl => "ODbL 1.0",
    /// What an access feature's level means: the source's record, never permission.
    AccessOpen => "Open to the public, as the source records it",
    AccessRestricted => "Restricted: a permit, fee, season or membership may apply",
    AccessClosed => "Closed to the public, as the source records it",
    AccessUnknown => "Access not recorded",
    AccessNotPermission => "Access is shown as the source records it, not as permission to fish, launch or park. Check posted rules and the regulations.",
    /// Notifications of marine alerts, storms and hurricanes. `{place}`: a place or "Your
    /// spot"; `{alert}`: the alert's event name as issued.
    AlertNotifyNew => "{place}: {alert} in effect",
    AlertNotifyStronger => "{place}: {alert}, now more severe",
    AlertNotifyExtended => "{place}: {alert}, extended",
    AlertNotifyInForce => "{place}: {alert}, still in effect",
    /// `{place}`.
    AlertNotifyStorm => "{place}: a significant storm is forecast",
    /// `{place}`, `{storm}`: the cyclone's name as issued.
    AlertNotifyHurricane => "{place}: in the forecast cone of {storm}",
    /// The cyclone's name when the source gave none.
    AlertNameCyclone => "a tropical cyclone",
    /// `{text}`: a notification line above; `{span}`: when, as on the alerts card.
    AlertNotifySpan => "{text}, {span}",
    /// A rule's safety conditions for the editor. `{alert}`: the event name it watches.
    AlertRuleMarine => "{alert} in effect",
    AlertRuleStorm => "A significant storm or a hurricane forecast cone",
}

impl SafetyKey {
    /// The key of an alert severity.
    pub const fn severity(severity: AlertSeverity) -> Self {
        match severity {
            AlertSeverity::Extreme => Self::SeverityExtreme,
            AlertSeverity::Severe => Self::SeveritySevere,
            AlertSeverity::Moderate => Self::SeverityModerate,
            AlertSeverity::Minor => Self::SeverityMinor,
            AlertSeverity::Unknown => Self::SeverityUnknown,
        }
    }
}

impl From<NoticeKind> for SafetyKey {
    fn from(kind: NoticeKind) -> Self {
        match kind {
            NoticeKind::NotForNavigation => Self::NotForNavigation,
            NoticeKind::Modelled => Self::Modelled,
            NoticeKind::Provisional => Self::Provisional,
            NoticeKind::Approximate => Self::Approximate,
            NoticeKind::TraditionalCue => Self::TraditionalCue,
            NoticeKind::TextOnly => Self::TextOnly,
        }
    }
}

/// A file the app keeps its own data in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppFile {
    /// The spots and areas.
    Spots,
    /// The settings.
    Settings,
}

/// The table, bound to a voice and a locale.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Strings {
    pub voice: Voice,
    pub locale: Locale,
}

/// A playful text from the language's pack; `None` writes English.
#[cfg(any(test, debug_assertions))]
fn pack_key(language: Language, key: Key, luna: bool) -> Option<&'static str> {
    (language == Language::Pseudo).then(|| crate::view::packs::key(key, luna))
}

#[cfg(not(any(test, debug_assertions)))]
const fn pack_key(_: Language, _: Key, _: bool) -> Option<&'static str> {
    None
}

/// A safety text from the language's pack; `None` writes English.
#[cfg(any(test, debug_assertions))]
fn pack_safety(language: Language, key: SafetyKey) -> Option<&'static str> {
    (language == Language::Pseudo).then(|| crate::view::packs::safety(key))
}

#[cfg(not(any(test, debug_assertions)))]
const fn pack_safety(_: Language, _: SafetyKey) -> Option<&'static str> {
    None
}

impl Strings {
    /// English words, numbers and dates.
    pub const fn new(voice: Voice) -> Self {
        Self::localized(voice, Locale::ENGLISH)
    }

    /// The words, numbers and dates of `locale`.
    pub const fn localized(voice: Voice, locale: Locale) -> Self {
        Self { voice, locale }
    }

    /// A playful text in the voice.
    pub fn get(self, key: Key) -> &'static str {
        let luna = matches!(self.voice, Voice::Luna);
        pack_key(self.locale.language, key, luna).unwrap_or_else(|| {
            let (plain, luna_text) = key.english();
            if luna { luna_text } else { plain }
        })
    }

    /// A playful text with its `{name}` placeholders filled.
    pub fn fmt(self, key: Key, args: &[(&str, &str)]) -> String {
        fill(self.get(key), args)
    }

    /// A safety text: the same in every voice.
    pub fn safety(self, key: SafetyKey) -> &'static str {
        pack_safety(self.locale.language, key).unwrap_or_else(|| key.text())
    }

    /// A safety text with its `{name}` placeholders filled: the same in every voice.
    pub fn safety_fmt(self, key: SafetyKey, args: &[(&str, &str)]) -> String {
        fill(self.safety(key), args)
    }

    // -- numbers, units, dates ------------------------------------------------------------

    /// A number in the locale, without thousands separators: "1020", "12.4".
    pub fn number(self, value: f32, decimals: u8) -> String {
        self.locale.number(value, decimals)
    }

    /// A number in the locale with the thousands grouped: "1,500", "12,345.6".
    pub fn grouped(self, value: f32, decimals: u8) -> String {
        self.locale.grouped(value, decimals)
    }

    /// The symbol of a temperature unit: "°F".
    pub fn temp_unit(self, unit: TempUnit) -> &'static str {
        self.get(match unit {
            TempUnit::F => Key::UnitFahrenheit,
            TempUnit::C => Key::UnitCelsius,
        })
    }

    /// The symbol of a height unit: "ft".
    pub fn height_unit(self, unit: HeightUnit) -> &'static str {
        self.get(match unit {
            HeightUnit::Ft => Key::UnitFeet,
            HeightUnit::M => Key::UnitMetres,
        })
    }

    /// The symbol of a speed unit: "kn".
    pub fn speed_unit(self, unit: SpeedUnit) -> &'static str {
        self.get(match unit {
            SpeedUnit::Kn => Key::UnitKnots,
            SpeedUnit::Mph => Key::UnitMph,
            SpeedUnit::Kmh => Key::UnitKmh,
            SpeedUnit::Ms => Key::UnitMetresPerSecond,
        })
    }

    /// The symbol of a discharge unit: "cfs".
    pub fn discharge_unit(self, unit: DischargeUnit) -> &'static str {
        self.get(match unit {
            DischargeUnit::Cfs => Key::UnitCfs,
            DischargeUnit::M3s => Key::UnitCubicMetresPerSecond,
        })
    }

    /// The symbol of a pressure unit: "inHg".
    pub fn pressure_unit(self, unit: PressureUnit) -> &'static str {
        self.get(match unit {
            PressureUnit::Hpa => Key::UnitHpa,
            PressureUnit::InHg => Key::UnitInHg,
            PressureUnit::MmHg => Key::UnitMmHg,
            PressureUnit::Kpa => Key::UnitKpa,
        })
    }

    /// The symbol of a distance unit: "mi".
    pub fn distance_unit(self, unit: DistanceUnit) -> &'static str {
        self.get(match unit {
            DistanceUnit::Km => Key::UnitKilometres,
            DistanceUnit::Mi => Key::UnitMiles,
        })
    }

    /// The unit a turbidity was measured in: "FNU". The value is never converted.
    pub fn turbidity_unit(self, unit: TurbidityUnit) -> &'static str {
        self.get(match unit {
            TurbidityUnit::Fnu => Key::UnitFnu,
            TurbidityUnit::Ftu => Key::UnitFtu,
            TurbidityUnit::Ntu => Key::UnitNtu,
        })
    }

    /// What a height is measured from: "MLLW", "gage datum".
    pub fn datum(self, datum: VerticalDatum) -> &'static str {
        self.get(match datum {
            VerticalDatum::Mllw => Key::DatumMllw,
            VerticalDatum::Mlw => Key::DatumMlw,
            VerticalDatum::Msl => Key::DatumMsl,
            VerticalDatum::Mtl => Key::DatumMtl,
            VerticalDatum::Navd88 => Key::DatumNavd88,
            VerticalDatum::Ngvd29 => Key::DatumNgvd29,
            VerticalDatum::Igld85 => Key::DatumIgld85,
            VerticalDatum::Gauge => Key::DatumGauge,
            VerticalDatum::ChsChartDatum => Key::DatumChsChart,
            VerticalDatum::ModelMean => Key::DatumModelMean,
        })
    }

    /// The 16-point compass name of a bearing in degrees: "SW".
    pub fn compass(self, degrees: f32) -> &'static str {
        const POINTS: [Key; 16] = [
            Key::CompassN,
            Key::CompassNne,
            Key::CompassNe,
            Key::CompassEne,
            Key::CompassE,
            Key::CompassEse,
            Key::CompassSe,
            Key::CompassSse,
            Key::CompassS,
            Key::CompassSsw,
            Key::CompassSw,
            Key::CompassWsw,
            Key::CompassW,
            Key::CompassWnw,
            Key::CompassNw,
            Key::CompassNnw,
        ];
        // INVARIANT: the bearing is reduced to 0..360 first, so the rounded sector is 0..=16
        // and fits after `% 16`; a NaN bearing casts to 0.
        let sector = (degrees.rem_euclid(360.0) / 22.5).round() as usize % 16;
        self.get(POINTS[sector])
    }

    /// A 16-point compass name as a source writes it ("ESE"), in the locale; `None` for other
    /// text.
    pub fn compass_point(self, text: &str) -> Option<&'static str> {
        const NAMES: [&str; 16] = [
            "N", "NNE", "NE", "ENE", "E", "ESE", "SE", "SSE", "S", "SSW", "SW", "WSW", "W", "WNW",
            "NW", "NNW",
        ];
        let index = NAMES
            .iter()
            .position(|n| n.eq_ignore_ascii_case(text.trim()))?;
        Some(self.compass(index as f32 * 22.5))
    }

    /// A 12-hour time with its AM or PM: `time` is "7:03" or "4".
    pub fn twelve_hour(self, time: &str, pm: bool) -> String {
        self.fmt(
            if pm { Key::TimePm } else { Key::TimeAm },
            &[("time", time)],
        )
    }

    /// The short name of month 1–12: "Nov"; the placeholder for anything else.
    pub fn month(self, month: u8) -> &'static str {
        self.get(match month {
            1 => Key::MonthJan,
            2 => Key::MonthFeb,
            3 => Key::MonthMar,
            4 => Key::MonthApr,
            5 => Key::MonthMay,
            6 => Key::MonthJun,
            7 => Key::MonthJul,
            8 => Key::MonthAug,
            9 => Key::MonthSep,
            10 => Key::MonthOct,
            11 => Key::MonthNov,
            12 => Key::MonthDec,
            _ => Key::Missing,
        })
    }

    /// A short date in the locale's order: "Nov 8" or "8 Nov".
    pub fn date(self, month: u8, day: u8) -> String {
        let month = self.month(month);
        match self.locale.dates {
            DateOrder::MonthDay => format!("{month} {day}"),
            DateOrder::DayMonth => format!("{day} {month}"),
        }
    }

    /// An ordinal number: "1st", "45th", "93rd".
    pub fn ordinal(self, n: u32) -> String {
        let key = match (n % 100, n % 10) {
            (11..=13, _) => Key::OrdinalTh,
            (_, 1) => Key::OrdinalSt,
            (_, 2) => Key::OrdinalNd,
            (_, 3) => Key::OrdinalRd,
            _ => Key::OrdinalTh,
        };
        self.fmt(key, &[("n", &n.to_string())])
    }

    /// A source's name as displayed: "NOAA CO-OPS"; an unknown source shows its id.
    pub fn source_name(self, id: &SourceId) -> String {
        let key = match id.as_str() {
            "usgs" => SafetyKey::SourceUsgs,
            "coops" => SafetyKey::SourceCoops,
            "coops-harmonics" => SafetyKey::SourceCoopsHarmonics,
            "ndbc" => SafetyKey::SourceNdbc,
            "nwps" => SafetyKey::SourceNwps,
            "open-meteo-marine" => SafetyKey::SourceOpenMeteoMarine,
            "nws" => SafetyKey::SourceNws,
            "usgs-3dhp" => SafetyKey::SourceUsgs3dhp,
            "glerl" => SafetyKey::SourceGlerl,
            "cwms" => SafetyKey::SourceCwms,
            "rise" => SafetyKey::SourceRise,
            "cyan" => SafetyKey::SourceCyan,
            "noaa-hab" => SafetyKey::SourceNoaaHab,
            "ny-dec-stocking" => SafetyKey::SourceNyDecStocking,
            "overpass" => SafetyKey::SourceOverpass,
            "padus" => SafetyKey::SourcePadus,
            "gbif" => SafetyKey::SourceGbif,
            "inaturalist" => SafetyKey::SourceInaturalist,
            "usgs-nas" => SafetyKey::SourceUsgsNas,
            "dfo-iwls" => SafetyKey::SourceDfoIwls,
            "eccc-hydrometric" => SafetyKey::SourceEcccHydrometric,
            other => return other.to_owned(),
        };
        self.safety(key).to_owned()
    }

    /// What a unit picker's group is called ("discharge" is "River flow"); `None` for an id
    /// that names no group.
    pub fn unit_group(self, id: &str) -> Option<&'static str> {
        Some(self.get(match id {
            "discharge" => Key::UnitGroupDischarge,
            "height" => Key::UnitGroupHeight,
            "stage" => Key::UnitGroupStage,
            "lake_level" => Key::UnitGroupLakeLevel,
            "tide" => Key::UnitGroupTide,
            "waves" => Key::UnitGroupWaves,
            "speed" => Key::UnitGroupSpeed,
            "current" => Key::UnitGroupCurrent,
            "wind" => Key::UnitGroupWind,
            "water_temp" => Key::UnitGroupWaterTemp,
            "air_temp" => Key::UnitGroupAirTemp,
            "pressure" => Key::UnitGroupPressure,
            "distance" => Key::UnitGroupDistance,
            _ => return None,
        }))
    }

    /// What a source group is called (`water_temperature` is "Water temperature"); `None` for
    /// an id that names no group.
    pub fn source_group(self, id: &str) -> Option<&'static str> {
        Some(self.get(match id {
            "water_temperature" => Key::SourceGroupWaterTemperature,
            "tide" => Key::SourceGroupTide,
            "currents" => Key::SourceGroupCurrents,
            "flow" => Key::SourceGroupFlow,
            "lake_level" => Key::SourceGroupLakeLevel,
            "waves" => Key::SourceGroupWaves,
            "sst" => Key::SourceGroupSst,
            "clarity" => Key::SourceGroupClarity,
            "water_quality" => Key::SourceGroupWaterQuality,
            "marine_forecast" => Key::SourceGroupMarineForecast,
            "weather" => Key::SourceGroupWeather,
            "storm" => Key::SourceGroupStorm,
            _ => return None,
        }))
    }

    /// "Same as river flow": the choice that makes a unit group follow `group` (its name).
    pub fn unit_same_as(self, group: &str) -> String {
        let mut chars = group.chars();
        let lower: String = chars
            .next()
            .map(|c| c.to_lowercase().chain(chars).collect())
            .unwrap_or_default();
        self.fmt(Key::UnitSameAs, &[("group", &lower)])
    }

    /// The limits of a source group in words: "within 25 km, under 3 h old"; `distance` and `age`
    /// are the values already written in the user's unit, `None` for no limit.
    pub fn source_limits(self, distance: Option<&str>, age: Option<&str>) -> String {
        let distance = distance.map_or_else(
            || self.get(Key::SourceAnyDistance).to_owned(),
            |d| self.fmt(Key::SourceWithin, &[("distance", d)]),
        );
        let age = age.map_or_else(
            || self.get(Key::SourceAnyAge).to_owned(),
            |a| self.fmt(Key::SourceUnderOld, &[("age", a)]),
        );
        self.fmt(Key::SourceLimits, &[("distance", &distance), ("age", &age)])
    }

    /// What a go/no-go profile switches on, in words: "2 limits, no lightning"; "No limits set"
    /// when it checks nothing.
    pub fn limit_summary(self, set: usize, no_small_craft: bool, no_lightning: bool) -> String {
        let mut parts: Vec<String> = Vec::new();
        if set > 0 {
            parts.push(self.fmt(
                if set == 1 {
                    Key::LimitCountOne
                } else {
                    Key::LimitCountMany
                },
                &[("n", &set.to_string())],
            ));
        }
        if no_small_craft {
            parts.push(self.get(Key::LimitNoSmallCraft).to_owned());
        }
        if no_lightning {
            parts.push(self.get(Key::LimitNoLightning).to_owned());
        }
        if parts.is_empty() {
            self.get(Key::LimitNone).to_owned()
        } else {
            parts.join(", ")
        }
    }

    /// What the app says of a file of its own it could not read: `kept_as` is the name it was
    /// moved to, `None` when it stayed where it was. Safety text, the same in every voice.
    pub fn file_problem(self, file: AppFile, kept_as: Option<&str>) -> String {
        match (file, kept_as) {
            (AppFile::Spots, Some(name)) => {
                self.safety_fmt(SafetyKey::FileKeptSpots, &[("name", name)])
            }
            (AppFile::Settings, Some(name)) => {
                self.safety_fmt(SafetyKey::FileKeptSettings, &[("name", name)])
            }
            (AppFile::Spots, None) => self.safety(SafetyKey::FileLeftSpots).to_owned(),
            (AppFile::Settings, None) => self.safety(SafetyKey::FileLeftSettings).to_owned(),
        }
    }

    /// The label of a flow chart's shaded band between two percentile labels.
    pub fn percentile_band(self, low: &str, high: &str) -> String {
        self.fmt(Key::PercentileBand, &[("low", low), ("high", high)])
    }

    /// What a measurement is called: "River flow", "Water temperature", ...
    pub fn measurement(self, m: Measurement) -> &'static str {
        self.get(match m {
            Measurement::WaterTemperature => Key::MeasWaterTemperature,
            Measurement::Discharge => Key::MeasDischarge,
            Measurement::GageHeight => Key::MeasGageHeight,
            Measurement::LakeLevel => Key::MeasLakeLevel,
            Measurement::Reservoir => Key::MeasReservoir,
            Measurement::Turbidity => Key::MeasTurbidity,
            Measurement::DissolvedOxygen => Key::MeasDissolvedOxygen,
            Measurement::Conductivity => Key::MeasConductivity,
            Measurement::Salinity => Key::MeasSalinity,
            Measurement::Ph => Key::MeasPh,
            Measurement::Chlorophyll => Key::MeasChlorophyll,
            Measurement::Clarity => Key::MeasClarity,
            Measurement::Cyanobacteria => Key::MeasCyanobacteria,
            Measurement::Bloom => Key::MeasBloom,
            Measurement::Tide => Key::MeasTide,
            Measurement::TidalCurrent => Key::MeasTidalCurrent,
            Measurement::OceanCurrent => Key::MeasOceanCurrent,
            Measurement::Waves => Key::MeasWaves,
            Measurement::WavesForecast => Key::MeasWavesForecast,
            Measurement::SeaSurfaceTemp => Key::MeasSeaSurfaceTemp,
            Measurement::SeaSurfaceTempForecast => Key::MeasSeaSurfaceTempForecast,
            Measurement::MarineForecast => Key::MeasMarineForecast,
            Measurement::MarineAlerts => Key::MeasMarineAlerts,
            Measurement::FloodCategories => Key::MeasFloodCategories,
            Measurement::FlowForecast => Key::MeasFlowForecast,
            Measurement::ObservedWind => Key::MeasWind,
            Measurement::ObservedPressure => Key::MeasPressure,
        })
    }

    /// What an alert rule measures: "River flow", "Gusts", ...
    pub fn alert_metric(self, m: AlertMetric) -> &'static str {
        self.get(match m {
            AlertMetric::Discharge => Key::AlertMetricDischarge,
            AlertMetric::GageHeight => Key::AlertMetricGageHeight,
            AlertMetric::WaterTemperature => Key::AlertMetricWaterTemperature,
            AlertMetric::SeaSurfaceTemp => Key::AlertMetricSeaSurfaceTemp,
            AlertMetric::WaveHeight => Key::AlertMetricWaveHeight,
            AlertMetric::Wind => Key::AlertMetricWind,
            AlertMetric::Gusts => Key::AlertMetricGusts,
            AlertMetric::PressureTrend => Key::AlertMetricPressureTrend,
            AlertMetric::TideHeight => Key::AlertMetricTideHeight,
            AlertMetric::Chlorophyll => Key::AlertMetricChlorophyll,
        })
    }

    /// How a station relates to the spot: "Same water body", "Upstream", ...
    pub fn relation(self, relation: Relation) -> &'static str {
        self.get(match relation {
            Relation::SameWaterBody => Key::RelSameWaterBody,
            Relation::Upstream => Key::RelUpstream,
            Relation::Downstream => Key::RelDownstream,
            Relation::Nearby => Key::RelNearby,
            Relation::Gridded => Key::RelModelGrid,
            Relation::Satellite => Key::RelSatellite,
        })
    }

    /// One reason a water type was suggested, in words: "in an NWS coastal marine zone",
    /// "on Croton River (stream of order 7)".
    pub fn water_type_reason(self, reason: &WaterTypeReason) -> String {
        let named = |name: &Option<String>, what: &str| match name {
            Some(n) => self.fmt(Key::WtReasonOn, &[("name", n), ("what", what)]),
            None => self.fmt(Key::WtReasonOnUnnamed, &[("what", what)]),
        };
        match reason {
            WaterTypeReason::MarineZone { kind } => self.fmt(
                Key::WtReasonZone,
                &[(
                    "kind",
                    self.get(match kind {
                        ZoneKind::Coastal => Key::ZoneCoastal,
                        ZoneKind::GreatLakesNearshore => Key::ZoneGreatLakesNearshore,
                        ZoneKind::GreatLakesOpen => Key::ZoneGreatLakesOpen,
                        ZoneKind::Offshore => Key::ZoneOffshore,
                    }),
                )],
            ),
            WaterTypeReason::Waterbody { name, kind, .. } => named(
                name,
                self.get(match kind {
                    WaterbodyKind::Lake => Key::BodyLake,
                    WaterbodyKind::Reservoir => Key::BodyReservoir,
                    WaterbodyKind::River => Key::BodyRiver,
                    WaterbodyKind::Estuary => Key::BodyEstuary,
                    WaterbodyKind::Other => Key::BodyOther,
                }),
            ),
            WaterTypeReason::BayName { name } => self.fmt(Key::WtReasonBay, &[("name", name)]),
            WaterTypeReason::Stream { name, stream_order } => {
                let what = match stream_order {
                    Some(order) => self.fmt(Key::StreamOfOrder, &[("order", &order.to_string())]),
                    None => self.get(Key::StreamPlain).to_owned(),
                };
                named(name, &what)
            }
            WaterTypeReason::UsgsSite { site_type } => self.fmt(
                Key::WtReasonUsgs,
                &[(
                    "kind",
                    self.get(match site_type {
                        UsgsSiteType::Stream => Key::SiteStream,
                        UsgsSiteType::Lake => Key::SiteLake,
                        UsgsSiteType::Estuary => Key::SiteEstuary,
                        UsgsSiteType::TidalStream => Key::SiteTidalStream,
                        UsgsSiteType::Other => Key::SiteOther,
                    }),
                )],
            ),
        }
    }

    /// The card title.
    pub fn card_title(self, id: CardId) -> &'static str {
        self.get(match id {
            CardId::Now => Key::CardNow,
            CardId::Tide => Key::CardTide,
            CardId::Current => Key::CardCurrent,
            CardId::Flow => Key::CardFlow,
            CardId::WaterTemp => Key::CardWaterTemp,
            CardId::Clarity => Key::CardClarity,
            CardId::Bloom => Key::CardBloom,
            CardId::Waves => Key::CardWaves,
            CardId::Wind => Key::CardWind,
            CardId::Pressure => Key::CardPressure,
            CardId::SunMoon => Key::CardSunMoon,
            CardId::Solunar => Key::CardSolunar,
            CardId::Windows => Key::CardWindows,
            CardId::GoNoGo => Key::CardGoNoGo,
            CardId::StormAhead => Key::CardStormAhead,
            CardId::MarineForecast => Key::CardMarineForecast,
            CardId::Stocking => Key::CardStocking,
            CardId::Species => Key::CardSpecies,
            CardId::Alerts => Key::CardAlerts,
            CardId::Sources => Key::CardSources,
        })
    }

    /// "High" or "Low".
    pub fn tide_kind(self, kind: TideKind) -> &'static str {
        self.get(match kind {
            TideKind::High => Key::TideHigh,
            TideKind::Low => Key::TideLow,
        })
    }

    /// "Waxing gibbous", "Full moon", ...
    pub fn moon_phase(self, phase: MoonPhase) -> &'static str {
        self.get(match phase {
            MoonPhase::New => Key::MoonNew,
            MoonPhase::WaxingCrescent => Key::MoonWaxingCrescent,
            MoonPhase::FirstQuarter => Key::MoonFirstQuarter,
            MoonPhase::WaxingGibbous => Key::MoonWaxingGibbous,
            MoonPhase::Full => Key::MoonFull,
            MoonPhase::WaningGibbous => Key::MoonWaningGibbous,
            MoonPhase::LastQuarter => Key::MoonLastQuarter,
            MoonPhase::WaningCrescent => Key::MoonWaningCrescent,
        })
    }

    /// "New moon", "First quarter", "Full moon" or "Last quarter".
    pub fn principal_phase(self, phase: PrincipalPhase) -> &'static str {
        self.get(match phase {
            PrincipalPhase::New => Key::MoonNew,
            PrincipalPhase::FirstQuarter => Key::MoonFirstQuarter,
            PrincipalPhase::Full => Key::MoonFull,
            PrincipalPhase::LastQuarter => Key::MoonLastQuarter,
        })
    }

    /// "Mon", "Tue", ...
    pub fn weekday(self, day: Weekday) -> &'static str {
        self.get(match day {
            Weekday::Mon => Key::WeekdayMon,
            Weekday::Tue => Key::WeekdayTue,
            Weekday::Wed => Key::WeekdayWed,
            Weekday::Thu => Key::WeekdayThu,
            Weekday::Fri => Key::WeekdayFri,
            Weekday::Sat => Key::WeekdaySat,
            Weekday::Sun => Key::WeekdaySun,
        })
    }

    /// "River or stream", "Tidal river or estuary", ...
    pub fn water_type(self, water_type: WaterType) -> &'static str {
        self.get(match water_type {
            WaterType::RiverStream => Key::WaterRiver,
            WaterType::LakeReservoir => Key::WaterLake,
            WaterType::TidalRiverEstuary => Key::WaterTidal,
            WaterType::BayInshore => Key::WaterBay,
            WaterType::Surf => Key::WaterSurf,
            WaterType::Nearshore => Key::WaterNearshore,
            WaterType::Offshore => Key::WaterOffshore,
            WaterType::GreatLakes => Key::WaterGreatLakes,
            WaterType::Pond => Key::WaterPond,
            WaterType::Ice => Key::WaterIce,
        })
    }

    /// What an access feature is: "Boat ramp", "Public land".
    pub fn access_kind(self, kind: AccessKind) -> &'static str {
        self.get(match kind {
            AccessKind::Ramp => Key::AccessKindRamp,
            AccessKind::Pier => Key::AccessKindPier,
            AccessKind::FishingSpot => Key::AccessKindFishingSpot,
            AccessKind::Parking => Key::AccessKindParking,
            AccessKind::PublicLand => Key::AccessKindPublicLand,
        })
    }

    /// What an access level means, as the source records it (never permission).
    pub fn access_level(self, level: AccessLevel) -> &'static str {
        self.safety(match level {
            AccessLevel::Open => SafetyKey::AccessOpen,
            AccessLevel::Restricted => SafetyKey::AccessRestricted,
            AccessLevel::Closed => SafetyKey::AccessClosed,
            AccessLevel::Unknown => SafetyKey::AccessUnknown,
        })
    }
}

/// The map's words: the catalogue holds ids and data, the words are here, looked up by layer or
/// preset id. Titles, colour-scale captions, preset names and the unavailable reason are playful
/// texts; credits, acknowledgements, depth references, terms and the captions of depth and chart
/// layers are safety texts, the same in every voice.
impl Strings {
    /// A drawer section's title.
    pub fn map_group(self, group: Group) -> &'static str {
        self.get(match group {
            Group::Base => Key::MapGroupBase,
            Group::Depth => Key::MapGroupDepth,
            Group::Structure => Key::MapGroupStructure,
            Group::Water => Key::MapGroupWater,
            Group::Weather => Key::MapGroupWeather,
            Group::Stations => Key::MapGroupStations,
            Group::Mine => Key::MapGroupMine,
        })
    }

    /// A layer's or base map's title; `None` for an id the catalogue does not have.
    pub fn layer_title(self, id: &str) -> Option<&'static str> {
        Some(self.get(match id {
            "apple-standard" => Key::MapLayerAppleStandard,
            "apple-satellite" => Key::MapLayerAppleSatellite,
            "apple-hybrid" => Key::MapLayerAppleHybrid,
            "noaa-chart" => Key::MapLayerNoaaChart,
            "gebco-relief" => Key::MapLayerGebcoRelief,
            "sst" => Key::MapLayerSst,
            "sst-imagery" => Key::MapLayerSstImagery,
            "chlorophyll" => Key::MapLayerChlorophyll,
            "chlorophyll-imagery" => Key::MapLayerChlorophyllImagery,
            "clarity" => Key::MapLayerClarity,
            "true-color-viirs-n20" => Key::MapLayerTrueColourViirsN20,
            "true-color-viirs-n21" => Key::MapLayerTrueColourViirsN21,
            "true-color-modis-terra" => Key::MapLayerTrueColourModisTerra,
            "true-color-modis-aqua" => Key::MapLayerTrueColourModisAqua,
            "hf-radar-currents" => Key::MapLayerHfRadarCurrents,
            "great-lakes-sst" => Key::MapLayerGreatLakesSst,
            "radar" => Key::MapLayerRadar,
            "wind" => Key::MapLayerWind,
            "pressure" => Key::MapLayerPressure,
            "waves" => Key::MapLayerWaves,
            "tide-stations" => Key::MapLayerTideStations,
            "current-stations" => Key::MapLayerCurrentStations,
            "usgs-gauges" => Key::MapLayerUsgsGauges,
            "buoys" => Key::MapLayerBuoys,
            "nwps-gauges" => Key::MapLayerNwpsGauges,
            "spots" => Key::MapLayerSpots,
            "lake-contours-in" => Key::MapLayerLakeContoursIn,
            "lake-contours-mn" => Key::MapLayerLakeContoursMn,
            "artificial-reefs-ny" => Key::MapLayerArtificialReefsNy,
            "enc-depths" => Key::MapLayerEncDepths,
            "enc-seabed" => Key::MapLayerEncSeabed,
            "enc-special-areas" => Key::MapLayerEncSpecialAreas,
            "enc-structure-points" => Key::MapLayerEncStructurePoints,
            _ => return None,
        }))
    }

    /// The caption under a legend whose colour range follows the values in view.
    pub fn legend_fitted(self) -> &'static str {
        self.get(Key::MapLegendFitted)
    }

    /// The short caption under a layer's legend; `None` for a layer without one.
    pub fn layer_legend(self, id: &str) -> Option<&'static str> {
        let playful = match id {
            "sst" | "great-lakes-sst" => Some(Key::MapLegendColdToWarm),
            "chlorophyll" => Some(Key::MapLegendClearToGreen),
            "clarity" => Some(Key::MapLegendClearToTurbid),
            "hf-radar-currents" => Some(Key::MapLegendSlowToFast),
            _ => None,
        };
        if let Some(key) = playful {
            return Some(self.get(key));
        }
        Some(self.safety(match id {
            "gebco-relief" => SafetyKey::MapLegendGebco,
            "lake-contours-in" => SafetyKey::MapLegendContoursIn,
            "lake-contours-mn" => SafetyKey::MapLegendContoursMn,
            "artificial-reefs-ny" => SafetyKey::MapLegendReefsNy,
            "enc-depths" => SafetyKey::MapLegendEncDepths,
            "enc-seabed" => SafetyKey::MapLegendEncSeabed,
            "enc-special-areas" => SafetyKey::MapLegendEncSpecialAreas,
            "enc-structure-points" => SafetyKey::MapLegendEncStructurePoints,
            _ => return None,
        }))
    }

    /// The credit line a layer shows on the map (safety text); `None` for an id the catalogue
    /// does not have.
    pub fn layer_credit(self, id: &str) -> Option<&'static str> {
        Some(self.safety(match id {
            "apple-standard" | "apple-satellite" | "apple-hybrid" => SafetyKey::MapCreditApple,
            "noaa-chart" | "enc-depths" | "enc-seabed" | "enc-special-areas" => {
                SafetyKey::MapCreditNcds
            }
            "enc-structure-points" => SafetyKey::MapCreditEncDirect,
            "gebco-relief" => SafetyKey::MapCreditGebco,
            "sst" => SafetyKey::MapCreditMurErddap,
            "sst-imagery" => SafetyKey::MapCreditMurGibs,
            "chlorophyll" => SafetyKey::MapCreditViirsColour,
            "chlorophyll-imagery" => SafetyKey::MapCreditGibsChlorophyll,
            "clarity" => SafetyKey::MapCreditClarity,
            "true-color-viirs-n20" => SafetyKey::MapCreditGibsViirsN20,
            "true-color-viirs-n21" => SafetyKey::MapCreditGibsViirsN21,
            "true-color-modis-terra" => SafetyKey::MapCreditGibsModisTerra,
            "true-color-modis-aqua" => SafetyKey::MapCreditGibsModisAqua,
            "hf-radar-currents" => SafetyKey::MapCreditHfRadar,
            "great-lakes-sst" => SafetyKey::MapCreditGlerl,
            "radar" => SafetyKey::MapCreditRadar,
            "wind" => SafetyKey::MapCreditWind,
            "pressure" => SafetyKey::MapCreditPressure,
            "waves" => SafetyKey::MapCreditWaves,
            "tide-stations" => SafetyKey::MapCreditTides,
            "current-stations" => SafetyKey::MapCreditCurrents,
            "usgs-gauges" => SafetyKey::MapCreditUsgs,
            "buoys" => SafetyKey::MapCreditBuoys,
            "nwps-gauges" => SafetyKey::MapCreditNwps,
            "spots" => SafetyKey::MapCreditSpots,
            "lake-contours-in" => SafetyKey::MapCreditContoursIn,
            "lake-contours-mn" => SafetyKey::MapCreditContoursMn,
            "artificial-reefs-ny" => SafetyKey::MapCreditReefsNy,
            _ => return None,
        }))
    }

    /// The acknowledgements a layer's source asks for on the sources page, beyond its credit.
    pub fn layer_acknowledgements(self, id: &str) -> Vec<&'static str> {
        let keys: &[SafetyKey] = match id {
            "sst" => &[SafetyKey::MapAckMur],
            "sst-imagery" => &[SafetyKey::MapAckGibs, SafetyKey::MapAckMur],
            "chlorophyll-imagery"
            | "true-color-viirs-n20"
            | "true-color-viirs-n21"
            | "true-color-modis-terra"
            | "true-color-modis-aqua" => &[SafetyKey::MapAckGibs],
            _ => &[],
        };
        keys.iter().map(|&k| self.safety(k)).collect()
    }

    /// What a state layer's depths are measured from, as its source states it.
    pub fn layer_reference(self, id: &str) -> Option<&'static str> {
        Some(self.safety(match id {
            "lake-contours-in" => SafetyKey::MapReferenceContoursIn,
            "lake-contours-mn" => SafetyKey::MapReferenceContoursMn,
            "artificial-reefs-ny" => SafetyKey::MapReferenceReefsNy,
            _ => return None,
        }))
    }

    /// A state layer's terms, as far as they are known.
    pub fn layer_licence(self, id: &str) -> Option<&'static str> {
        Some(self.safety(match id {
            "lake-contours-in" => SafetyKey::MapLicenceContoursIn,
            "lake-contours-mn" => SafetyKey::MapLicenceContoursMn,
            "artificial-reefs-ny" => SafetyKey::MapLicenceReefsNy,
            _ => return None,
        }))
    }

    /// A built-in preset's name; `None` for an id that is not a built-in preset.
    pub fn preset_name(self, id: &str) -> Option<&'static str> {
        Some(self.get(match id {
            "inshore-flats" => Key::MapPresetInshoreFlats,
            "offshore-pelagics" => Key::MapPresetOffshorePelagics,
            "trout-stream" => Key::MapPresetTroutStream,
            "bass-lake" => Key::MapPresetBassLake,
            "surf" => Key::MapPresetSurf,
            "great-lakes-trolling" => Key::MapPresetGreatLakesTrolling,
            _ => return None,
        }))
    }

    /// Why a layer cannot be turned on, shown in place of its switch.
    pub fn unavailable_reason(self, reason: UnavailableReason) -> &'static str {
        match reason {
            UnavailableReason::NoSource => self.get(Key::MapNoSource),
        }
    }
}

/// What a map layer's values measure, which decides how they read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapQuantity {
    /// Water temperature, °C.
    Temperature,
    /// Chlorophyll a, mg m⁻³ (the same number as µg/L).
    Chlorophyll,
    /// The diffuse attenuation coefficient at 490 nm, m⁻¹.
    Kd490,
    /// Current speed, m/s.
    CurrentSpeed,
}

impl MapQuantity {
    /// What a layer's values measure; `None` for a layer whose values the map does not read out.
    pub fn of_layer(id: &str) -> Option<Self> {
        Some(match id {
            "sst" | "sst-imagery" | "great-lakes-sst" => Self::Temperature,
            "chlorophyll" | "chlorophyll-imagery" => Self::Chlorophyll,
            "clarity" => Self::Kd490,
            "hf-radar-currents" => Self::CurrentSpeed,
            _ => return None,
        })
    }
}

/// A map value in canonical units as the user reads it: "54 °F", "1.2 µg/L" (two decimals
/// below 1, none from 10), "0.25 1/m", "0.8 kn". These are the units the conditions cards use.
pub fn map_value(f: &Formatter, quantity: MapQuantity, canonical: f64) -> String {
    let v = canonical as f32;
    match quantity {
        MapQuantity::Temperature => f.temperature(Some(v)).full(),
        MapQuantity::CurrentSpeed => f.current_speed(Some(v)).full(),
        MapQuantity::Chlorophyll => {
            let decimals = u8::from(v.abs() < 1.0) + u8::from(v.abs() < 10.0);
            format!(
                "{} {}",
                f.strings.grouped(v, decimals),
                f.strings.get(Key::UnitMicrogramsPerLitre)
            )
        }
        MapQuantity::Kd490 => format!(
            "{} {}",
            f.strings.grouped(v, 2),
            f.strings.get(Key::UnitPerMetre)
        ),
    }
}

/// A surface current at a point: "0.8 kn toward NE 45°" (the direction the water goes).
pub fn map_current(f: &Formatter, speed_ms: f64, toward_deg: f64) -> String {
    f.strings.fmt(
        Key::MapCurrentToward,
        &[
            ("speed", &map_value(f, MapQuantity::CurrentSpeed, speed_ms)),
            ("direction", &f.direction(Some(toward_deg as f32))),
        ],
    )
}

/// When a map layer's field is valid: the UTC day for a daily field ("Nov 7"), the day and
/// time in the formatter's zone for hourly fields and frames ("Nov 8, 4 PM").
pub fn map_time(f: &Formatter, t: UnixTime, mode: DateMode) -> String {
    match mode {
        DateMode::Daily | DateMode::None => f.date(t.local_date(UtcOffset(0))),
        DateMode::Hourly | DateMode::Frames => f.strings.fmt(
            Key::MapDateTime,
            &[("date", &f.date_of(t)), ("time", &f.clock(t))],
        ),
    }
}

/// A measured line as the user reads it: the distance ("2.4 mi"), the bearing from the start
/// ("NE 45°", degrees clockwise from true north) and both in one line ("2.4 mi at NE 45°").
pub fn map_measure(f: &Formatter, metres: f64, bearing_deg: f64) -> (String, String, String) {
    let distance = f.distance(Some(metres as f32)).full();
    let bearing = f.direction(Some(bearing_deg as f32));
    let line = f.strings.fmt(
        Key::MapMeasure,
        &[("distance", &distance), ("bearing", &bearing)],
    );
    (distance, bearing, line)
}

/// A depth in the user's height unit for a map label: "33 ft", "10 m", "6.1 m" (a decimal only
/// where the value is not whole to a tenth).
pub fn depth_label(f: &Formatter, metres: f64) -> String {
    let unit = f.units.height;
    let v = unit.from_canonical(metres as f32);
    let tenths = (v * 10.0).round() / 10.0;
    let decimals = u8::from(tenths.fract() != 0.0);
    format!(
        "{} {}",
        f.strings.grouped(tenths, decimals),
        f.strings.height_unit(unit)
    )
}

/// When a species record was made: "Seen Nov 3 2026", or "Date not recorded".
pub fn record_seen(f: &Formatter, observed_on: Option<LocalDate>) -> String {
    match observed_on {
        Some(d) => f.strings.fmt(
            Key::RecordSeen,
            &[("date", &f.date(d)), ("year", &d.year.to_string())],
        ),
        None => f.strings.get(Key::RecordDateUnknown).to_owned(),
    }
}

/// How good a species record's position is, in the user's units: "Position good to 98 ft",
/// "Position blurred by the source, within 17 mi"; `None` when the source says nothing. Under a
/// kilometre the radius is a whole length in the height unit, beyond it a distance.
pub fn record_position(
    f: &Formatter,
    obscured: bool,
    uncertainty_m: Option<f32>,
) -> Option<String> {
    let radius = uncertainty_m
        .filter(|m| m.is_finite() && *m >= 0.0)
        .map(|m| {
            if m < 1_000.0 {
                let unit = f.units.height;
                let v = unit.from_canonical(m).round().max(1.0);
                format!(
                    "{} {}",
                    f.strings.grouped(v, 0),
                    f.strings.height_unit(unit)
                )
            } else {
                f.distance(Some(m)).full()
            }
        });
    match (obscured, radius) {
        (true, Some(r)) => Some(f.strings.fmt(Key::RecordObscured, &[("distance", &r)])),
        (true, None) => Some(f.strings.get(Key::RecordObscuredArea).to_owned()),
        (false, Some(r)) => Some(f.strings.fmt(Key::RecordWithin, &[("distance", &r)])),
        (false, None) => None,
    }
}

/// The elevation model a DEM raster belongs to, by its name; `None` for one not recognised.
fn dem_model(f: &Formatter, raster: &str) -> Option<&'static str> {
    let name = raster.to_ascii_lowercase();
    let key = if name.starts_with("crm_") {
        SafetyKey::MapDepthModelCrm
    } else if name.starts_with("ncei19_") {
        SafetyKey::MapDepthModelCudem
    } else if name.starts_with("etopo_2022") {
        SafetyKey::MapDepthModelEtopo
    } else {
        return None;
    };
    Some(f.strings.safety(key))
}

/// A DEM reading for the inspector: "approximate depth 97 ft (EGM 2008, Coastal Relief Model)".
/// The datum is the finest source's; "datum not stated" where it states none. Never chart datum.
pub fn depth_reading(f: &Formatter, reading: &ApproxDepth) -> String {
    let datum = reading
        .vertical_datum
        .as_deref()
        .unwrap_or_else(|| f.strings.safety(SafetyKey::MapDepthDatumUnknown));
    let model = reading.source.as_deref().and_then(|s| dem_model(f, s));
    let (name, value, plain, with_model) = match reading.reading {
        DepthReading::Depth { depth_m } => (
            "depth",
            depth_label(f, depth_m),
            SafetyKey::MapDepthApprox,
            SafetyKey::MapDepthApproxModel,
        ),
        DepthReading::BedElevation { elevation_m } => (
            "height",
            depth_label(f, elevation_m),
            SafetyKey::MapDepthBed,
            SafetyKey::MapDepthBedModel,
        ),
    };
    match model {
        Some(model) => f.strings.safety_fmt(
            with_model,
            &[(name, &value), ("datum", datum), ("model", model)],
        ),
        None => f
            .strings
            .safety_fmt(plain, &[(name, &value), ("datum", datum)]),
    }
}

/// A break's strength in the user's units. Temperature: the change per distance unit in the
/// user's temperature unit ("0.9 °F per mi"). Chlorophyll: the ratio of concentrations across one
/// distance unit ("×1.3 per km"). `per_km` is the canonical gradient: °C per km, or log10 units
/// per km.
pub fn break_strength(f: &Formatter, kind: BreakKind, per_km: f64) -> String {
    let distance = f.units.distance;
    let per_unit = per_km * distance_to_metres(distance, 1.0) / 1_000.0;
    match kind {
        BreakKind::Sst => {
            let t = f.units.temperature;
            let change = t.from_canonical(per_unit as f32) - t.from_canonical(0.0);
            let decimals = u8::from(change.abs() < 10.0);
            let text = format!(
                "{} {}",
                f.strings.grouped(change, decimals),
                f.strings.temp_unit(t)
            );
            f.strings.fmt(
                Key::MapBreakTemperature,
                &[
                    ("change", &text),
                    ("distance", f.strings.distance_unit(distance)),
                ],
            )
        }
        BreakKind::Chlorophyll => {
            let ratio = 10f64.powf(per_unit) as f32;
            let text = f.strings.grouped(ratio, 1 + u8::from(ratio < 2.0));
            f.strings.fmt(
                Key::MapBreakChlorophyll,
                &[
                    ("ratio", &text),
                    ("distance", f.strings.distance_unit(distance)),
                ],
            )
        }
    }
}

/// A charted structure point's title: "Wreck", "Obstruction", "Rock", "Fish haven".
pub fn structure_title(f: &Formatter, kind: StructureKind, fish_haven: bool) -> &'static str {
    f.strings.get(match (kind, fish_haven) {
        (_, true) => Key::MapStructureFishHaven,
        (StructureKind::Wreck, _) => Key::MapStructureWreck,
        (StructureKind::Obstruction, _) => Key::MapStructureObstruction,
        (StructureKind::Rock, _) => Key::MapStructureRock,
    })
}

/// The chart's own category of a point, as charted: "As charted: dangerous wreck". The service
/// cuts long categories, so one that may be cut ends in an ellipsis.
pub fn structure_category(f: &Formatter, category: &str, may_be_cut: bool) -> String {
    let category = if may_be_cut {
        format!("{category}…")
    } else {
        category.to_owned()
    };
    f.strings
        .fmt(Key::MapStructureCategory, &[("category", &category)])
}

/// When a chart object was surveyed: "Surveyed 2019", "Surveyed 2019-03", "Surveyed 2019-03-14".
pub fn structure_surveyed(f: &Formatter, date: SourceDate) -> String {
    let text = match (date.month, date.day) {
        (Some(m), Some(d)) => format!("{:04}-{m:02}-{d:02}", date.year),
        (Some(m), None) => format!("{:04}-{m:02}", date.year),
        _ => format!("{:04}", date.year),
    };
    f.strings.fmt(Key::MapStructureSurveyed, &[("date", &text)])
}

/// A reef site's depth range in the user's unit: "Depth 20–70 ft"; `None` with neither depth.
pub fn reef_depths(f: &Formatter, min_m: Option<f64>, max_m: Option<f64>) -> Option<String> {
    let unit = f.units.height;
    let number = |m: f64| {
        let v = (unit.from_canonical(m as f32) * 10.0).round() / 10.0;
        f.strings.grouped(v, u8::from(v.fract() != 0.0))
    };
    let symbol = f.strings.height_unit(unit);
    let range = match (min_m, max_m) {
        (Some(a), Some(b)) if (a - b).abs() > f64::EPSILON => {
            format!("{}–{} {symbol}", number(a), number(b))
        }
        (Some(m), _) | (None, Some(m)) => format!("{} {symbol}", number(m)),
        (None, None) => return None,
    };
    Some(f.strings.fmt(Key::MapReefDepth, &[("range", &range)]))
}

/// Every kind of water, in the order a picker lists them.
pub const WATER_TYPES: [WaterType; 10] = [
    WaterType::RiverStream,
    WaterType::LakeReservoir,
    WaterType::TidalRiverEstuary,
    WaterType::BayInshore,
    WaterType::Surf,
    WaterType::Nearshore,
    WaterType::Offshore,
    WaterType::GreatLakes,
    WaterType::Pond,
    WaterType::Ice,
];

/// Every measurement, in the order lists of them read: the water's level and movement first,
/// then its temperature and flow, its quality, the sea, then the forecasts and the air.
pub const MEASUREMENTS: [Measurement; 27] = [
    Measurement::Tide,
    Measurement::TidalCurrent,
    Measurement::WaterTemperature,
    Measurement::Discharge,
    Measurement::GageHeight,
    Measurement::LakeLevel,
    Measurement::Reservoir,
    Measurement::Turbidity,
    Measurement::Clarity,
    Measurement::DissolvedOxygen,
    Measurement::Conductivity,
    Measurement::Salinity,
    Measurement::Ph,
    Measurement::Chlorophyll,
    Measurement::Cyanobacteria,
    Measurement::Bloom,
    Measurement::OceanCurrent,
    Measurement::Waves,
    Measurement::WavesForecast,
    Measurement::SeaSurfaceTemp,
    Measurement::SeaSurfaceTempForecast,
    Measurement::MarineForecast,
    Measurement::MarineAlerts,
    Measurement::FloodCategories,
    Measurement::FlowForecast,
    Measurement::ObservedWind,
    Measurement::ObservedPressure,
];

/// Fills `{name}` placeholders; unknown placeholders stay as written.
fn fill(template: &str, args: &[(&str, &str)]) -> String {
    let mut out = template.to_owned();
    for (name, value) in args {
        out = out.replace(&format!("{{{name}}}"), value);
    }
    out
}

/// Offline packs and tables.
impl Strings {
    /// A download size in decimal units: "512 KB", "48.3 MB", "1.5 GB".
    pub fn pack_size(self, bytes: u64) -> String {
        let (key, value, decimals) = if bytes < 1_000_000 {
            (Key::PackSizeKb, bytes.div_ceil(1_000) as f32, 0)
        } else if bytes < 1_000_000_000 {
            let mb = bytes as f32 / 1e6;
            (Key::PackSizeMb, mb, u8::from(mb < 100.0))
        } else {
            (Key::PackSizeGb, bytes as f32 / 1e9, 1)
        };
        self.fmt(key, &[("n", &self.grouped(value, decimals))])
    }

    /// A download time: "under a minute", "12 min", "2 h", "1 h 20 min".
    pub fn pack_duration(self, seconds: u64) -> String {
        let minutes = seconds.div_ceil(60);
        if seconds < 60 {
            return self.get(Key::PackTimeUnderMinute).to_owned();
        }
        if minutes < 60 {
            return self.fmt(Key::PackTimeMinutes, &[("n", &minutes.to_string())]);
        }
        let (h, m) = (minutes / 60, minutes % 60);
        if m == 0 {
            self.fmt(Key::PackTimeHours, &[("h", &h.to_string())])
        } else {
            self.fmt(
                Key::PackTimeHoursMinutes,
                &[("h", &h.to_string()), ("m", &m.to_string())],
            )
        }
    }

    /// Why a layer of a pack is left out.
    pub fn pack_skip(self, reason: SkipReason) -> &'static str {
        self.get(match reason {
            SkipReason::Unknown => Key::PackSkipUnknown,
            SkipReason::Unavailable => Key::PackSkipUnavailable,
            SkipReason::NotPackable => Key::PackSkipNotPackable,
            SkipReason::ZoomOutOfRange => Key::PackSkipZoom,
        })
    }

    /// Why a pack cannot be planned.
    pub fn pack_error(self, error: &PlanError) -> String {
        match error {
            PlanError::InvalidBox => self.get(Key::PackErrorBox).to_owned(),
            PlanError::InvalidZooms => self.get(Key::PackErrorZooms).to_owned(),
            PlanError::InvalidDates => self.get(Key::PackErrorDates).to_owned(),
            PlanError::TooManyItems { cap } => {
                self.fmt(Key::PackErrorTooMany, &[("cap", &cap.to_string())])
            }
        }
    }
}

/// A source's name as displayed, in English: "NOAA CO-OPS". The view uses
/// [`Strings::source_name`], in its locale.
pub fn source_name(id: &SourceId) -> String {
    Strings::new(Voice::Plain).source_name(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_known_source_has_a_name() {
        use crate::provenance::SourceId;
        let s = Strings::new(Voice::Plain);
        for id in [
            SourceId::USGS,
            SourceId::COOPS,
            SourceId::COOPS_HARMONICS,
            SourceId::NDBC,
            SourceId::NWPS,
            SourceId::OPEN_METEO_MARINE,
            SourceId::NWS,
            SourceId::USGS_3DHP,
            SourceId::GLERL,
            SourceId::CWMS,
            SourceId::RISE,
            SourceId::CYAN,
            SourceId::NOAA_HAB,
            SourceId::NY_DEC_STOCKING,
            SourceId::OVERPASS,
            SourceId::PADUS,
            SourceId::GBIF,
            SourceId::INATURALIST,
            SourceId::USGS_NAS,
            SourceId::DFO_IWLS,
            SourceId::ECCC_HYDROMETRIC,
        ] {
            assert_ne!(
                s.source_name(&id),
                id.as_str(),
                "no name for {}",
                id.as_str()
            );
        }
        assert_eq!(
            s.source_name(&SourceId::DFO_IWLS),
            "Canadian Hydrographic Service"
        );
    }

    #[test]
    fn luna_variants_exist_only_for_playful_text() {
        let luna = Strings::new(Voice::Luna);
        let plain = Strings::new(Voice::Plain);
        let args = [("station", "Hudson R at Battery"), ("distance", "0.4 mi")];
        assert_eq!(
            luna.fmt(Key::StationFooter, &args),
            "Luna's watching Hudson R at Battery, 0.4 mi"
        );
        assert_eq!(
            plain.fmt(Key::StationFooter, &args),
            "Hudson R at Battery, 0.4 mi"
        );
        assert_eq!(luna.get(Key::GoNoGoGood), "Luna approves.");
        assert_eq!(luna.card_title(CardId::Windows), "Snack times");
        assert_eq!(plain.card_title(CardId::Windows), "Fishing windows");
        for key in SafetyKey::ALL.iter().copied() {
            assert_eq!(luna.safety(key), plain.safety(key));
            assert!(!key.text().contains("Luna"));
        }
    }

    #[test]
    fn settings_phrases_read_in_both_voices() {
        for voice in [Voice::Plain, Voice::Luna] {
            let s = Strings::new(voice);
            assert_eq!(s.source_group("sst"), Some("Sea surface temperature"));
            assert_eq!(s.source_group("storm"), Some("Storms"));
            assert_eq!(s.source_group("nope"), None);
            assert_eq!(s.unit_same_as("River flow"), "Same as river flow");
            assert_eq!(s.unit_same_as(""), "Same as ");
            assert_eq!(
                s.source_limits(Some("25 km"), Some("3 h")),
                "within 25 km, under 3 h old"
            );
            assert_eq!(s.source_limits(None, None), "any distance, any age");
            assert_eq!(s.limit_summary(0, false, false), "No limits set");
            assert_eq!(s.limit_summary(1, false, false), "1 limit");
            assert_eq!(
                s.limit_summary(3, true, true),
                "3 limits, no small craft advisory, no lightning"
            );
            assert_eq!(s.limit_summary(0, false, true), "no lightning");
            assert_eq!(
                s.percentile_band("5th percentile", "95th percentile"),
                "Shaded: 5th percentile to 95th percentile"
            );
        }
    }

    #[test]
    fn file_problems_are_safety_text_in_every_voice() {
        for voice in [Voice::Plain, Voice::Luna] {
            let s = Strings::new(voice);
            assert_eq!(
                s.file_problem(AppFile::Spots, Some("state.json.bad")),
                "Your spots could not be read. The file was kept as state.json.bad and the list \
                 starts empty; nothing is saved over the kept file."
            );
            let settings = s.file_problem(AppFile::Settings, Some("settings.json.bad"));
            assert!(settings.contains("kept as settings.json.bad"), "{settings}");
            assert!(settings.contains("default settings"), "{settings}");
            let left = s.file_problem(AppFile::Spots, None);
            assert!(left.contains("never saved over"), "{left}");
            assert!(!left.contains("Luna"));
            assert!(
                s.file_problem(AppFile::Settings, None)
                    .contains("changes to settings are not saved")
            );
        }
    }

    #[test]
    fn page_problems_and_the_search_footer_read_in_both_voices() {
        let plain = Strings::new(Voice::Plain);
        let luna = Strings::new(Voice::Luna);
        assert_eq!(
            plain.get(Key::PageProblemTitle),
            "Conditions could not be loaded"
        );
        assert_eq!(
            luna.get(Key::PageProblemTitle),
            "Luna couldn't fetch the water"
        );
        for s in [plain, luna] {
            assert_eq!(
                s.fmt(Key::SnapshotUnreadable, &[("reason", "EOF")]),
                "The saved conditions could not be read: EOF"
            );
            assert!(s.get(Key::PlaceSearchFooter).contains("approximate area"));
        }
    }

    #[test]
    fn unit_groups_are_named_in_both_voices() {
        for voice in [Voice::Plain, Voice::Luna] {
            let s = Strings::new(voice);
            assert_eq!(s.unit_group("discharge"), Some("River flow"));
            assert_eq!(s.unit_group("height"), Some("Heights"));
            assert_eq!(s.unit_group("wind"), Some("Wind speed"));
            assert_eq!(s.unit_group("distance"), Some("Distances"));
            assert_eq!(s.unit_group("nonsense"), None);
        }
    }

    #[test]
    fn every_key_is_listed_and_placeholders_pair_up() {
        let mut seen = std::collections::BTreeSet::new();
        for key in Key::ALL {
            assert!(seen.insert(key), "{key:?} listed twice");
            let (plain, luna) = key.english();
            let names = |s: &str| {
                let mut v: Vec<String> = s
                    .split('{')
                    .skip(1)
                    .filter_map(|p| p.split('}').next().map(str::to_owned))
                    .collect();
                v.sort_unstable();
                v
            };
            assert_eq!(names(plain), names(luna), "{key:?}");
        }
    }

    #[test]
    fn pack_sizes_and_times_read_plainly() {
        let s = Strings::new(Voice::Plain);
        assert_eq!(s.pack_size(0), "0 KB");
        assert_eq!(s.pack_size(12_700), "13 KB");
        assert_eq!(s.pack_size(48_260_000), "48.3 MB");
        assert_eq!(s.pack_size(412_000_000), "412 MB");
        assert_eq!(s.pack_size(1_540_000_000), "1.5 GB");
        assert_eq!(s.pack_duration(20), "under a minute");
        assert_eq!(s.pack_duration(61), "2 min");
        assert_eq!(s.pack_duration(7_200), "2 h");
        assert_eq!(s.pack_duration(4_800), "1 h 20 min");
        assert_eq!(
            s.pack_error(&PlanError::TooManyItems { cap: 50_000 }),
            "This pack needs more than 50000 downloads: choose a smaller area or fewer zoom levels."
        );
    }

    #[test]
    fn water_type_reasons_read_in_both_voices() {
        let reasons = [
            WaterTypeReason::MarineZone {
                kind: ZoneKind::Coastal,
            },
            WaterTypeReason::Waterbody {
                name: None,
                kind: WaterbodyKind::River,
                area_km2: 175.6,
            },
            WaterTypeReason::Stream {
                name: Some("Croton River".into()),
                stream_order: Some(7),
            },
            WaterTypeReason::UsgsSite {
                site_type: UsgsSiteType::TidalStream,
            },
        ];
        for voice in [Voice::Plain, Voice::Luna] {
            let text: Vec<String> = reasons
                .iter()
                .map(|r| Strings::new(voice).water_type_reason(r))
                .collect();
            assert_eq!(
                text,
                [
                    "in an NWS coastal marine zone",
                    "on an unnamed river",
                    "on Croton River (stream of order 7)",
                    "the nearest USGS site is on a tidal stream",
                ]
            );
        }
        let bay = WaterTypeReason::BayName {
            name: "Jamaica Bay".into(),
        };
        assert_eq!(
            Strings::new(Voice::Plain).water_type_reason(&bay),
            "Jamaica Bay is named as a bay or harbor"
        );
    }

    #[test]
    fn the_lists_hold_every_water_type_and_measurement_once() {
        // A new variant fails to compile here until it is placed in its list.
        let water_index = |w: WaterType| match w {
            WaterType::RiverStream => 0,
            WaterType::LakeReservoir => 1,
            WaterType::TidalRiverEstuary => 2,
            WaterType::BayInshore => 3,
            WaterType::Surf => 4,
            WaterType::Nearshore => 5,
            WaterType::Offshore => 6,
            WaterType::GreatLakes => 7,
            WaterType::Pond => 8,
            WaterType::Ice => 9,
        };
        for (i, w) in WATER_TYPES.into_iter().enumerate() {
            assert_eq!(water_index(w), i);
        }
        let measurement_seen = |m: Measurement| match m {
            Measurement::WaterTemperature
            | Measurement::Discharge
            | Measurement::GageHeight
            | Measurement::LakeLevel
            | Measurement::Reservoir
            | Measurement::Turbidity
            | Measurement::DissolvedOxygen
            | Measurement::Conductivity
            | Measurement::Salinity
            | Measurement::Ph
            | Measurement::Chlorophyll
            | Measurement::Clarity
            | Measurement::Cyanobacteria
            | Measurement::Bloom
            | Measurement::Tide
            | Measurement::TidalCurrent
            | Measurement::OceanCurrent
            | Measurement::Waves
            | Measurement::WavesForecast
            | Measurement::SeaSurfaceTemp
            | Measurement::SeaSurfaceTempForecast
            | Measurement::MarineForecast
            | Measurement::MarineAlerts
            | Measurement::FloodCategories
            | Measurement::FlowForecast
            | Measurement::ObservedWind
            | Measurement::ObservedPressure => MEASUREMENTS.contains(&m),
        };
        let distinct: std::collections::BTreeSet<_> = MEASUREMENTS.into_iter().collect();
        assert_eq!(distinct.len(), MEASUREMENTS.len());
        assert!(MEASUREMENTS.into_iter().all(measurement_seen));
    }

    #[test]
    fn access_kinds_and_levels_are_worded_and_levels_are_never_permission() {
        let plain = Strings::new(Voice::Plain);
        let luna = Strings::new(Voice::Luna);
        assert_eq!(plain.access_kind(AccessKind::Ramp), "Boat ramp");
        assert_eq!(plain.access_kind(AccessKind::PublicLand), "Public land");
        for level in [
            AccessLevel::Open,
            AccessLevel::Restricted,
            AccessLevel::Closed,
            AccessLevel::Unknown,
        ] {
            // Safety text: one wording in every voice.
            assert_eq!(plain.access_level(level), luna.access_level(level));
            assert!(!plain.access_level(level).contains("Luna"));
        }
        assert!(
            plain
                .access_level(AccessLevel::Open)
                .contains("as the source records it")
        );
        assert_eq!(
            plain.access_level(AccessLevel::Unknown),
            "Access not recorded"
        );
    }

    #[test]
    fn relations_have_labels() {
        let plain = Strings::new(Voice::Plain);
        assert_eq!(plain.relation(Relation::SameWaterBody), "Same water body");
        assert_eq!(plain.relation(Relation::Gridded), "Model grid");
    }

    #[test]
    fn measurements_have_labels() {
        assert_eq!(
            Strings::new(Voice::Plain).measurement(Measurement::Discharge),
            "River flow"
        );
        for voice in [Voice::Plain, Voice::Luna] {
            let s = Strings::new(voice);
            assert_eq!(s.measurement(Measurement::Reservoir), "Reservoir release");
            assert_eq!(s.measurement(Measurement::Bloom), "Algal bloom");
        }
        let at = |m| MEASUREMENTS.iter().position(|x| *x == m);
        assert_eq!(
            at(Measurement::Reservoir),
            at(Measurement::LakeLevel).map(|i| i + 1)
        );
        assert_eq!(
            at(Measurement::Bloom),
            at(Measurement::Cyanobacteria).map(|i| i + 1)
        );
    }

    mod map {
        use super::*;
        use crate::map::catalogue::{BASES, LAYERS, PRESETS, find};
        use crate::map::enc;

        const GEBCO_CAPS: &str = include_str!(
            "../../../../tests/fixtures/raw/bathymetry/gebco-wms-capabilities-1.3.0.xml"
        );

        fn both() -> [Strings; 2] {
            [Strings::new(Voice::Plain), Strings::new(Voice::Luna)]
        }

        #[test]
        fn every_layer_has_a_title_and_a_credit_in_both_voices() {
            for s in both() {
                for l in BASES.iter().chain(LAYERS.iter()) {
                    let title = s.layer_title(l.id).unwrap_or_else(|| panic!("{}", l.id));
                    assert!(!title.trim().is_empty(), "{}", l.id);
                    let credit = s.layer_credit(l.id).unwrap_or_else(|| panic!("{}", l.id));
                    assert!(!credit.trim().is_empty(), "{}", l.id);
                    assert!(!credit.contains("Luna"), "{}", l.id);
                    if l.ramp.is_some() && l.grid().is_some() {
                        assert!(s.layer_legend(l.id).is_some(), "{}", l.id);
                    }
                }
                assert_eq!(s.layer_title("nope"), None);
                assert_eq!(s.layer_credit("nope"), None);
                assert_eq!(s.layer_legend("radar"), None);
                for g in crate::map::catalogue::Group::ALL {
                    assert_ne!(s.map_group(g), "");
                }
            }
            let plain = Strings::new(Voice::Plain);
            assert_eq!(plain.layer_title("sst"), Some("Sea surface temperature"));
            assert_eq!(plain.layer_title("noaa-chart"), Some("NOAA chart"));
            assert_eq!(plain.layer_legend("sst"), Some("Cold to warm"));
        }

        #[test]
        fn credits_of_charts_and_depth_say_not_for_navigation() {
            let s = Strings::new(Voice::Luna);
            for id in [
                "noaa-chart",
                "enc-depths",
                "enc-seabed",
                "enc-special-areas",
            ] {
                assert_eq!(s.layer_credit(id), Some(enc::ENC_PICTURE_CREDIT), "{id}");
            }
            assert_eq!(
                s.layer_credit("enc-structure-points"),
                Some(enc::ENC_DIRECT_CREDIT)
            );
            assert!(
                s.layer_credit("noaa-chart")
                    .unwrap()
                    .contains("Not to be used for navigation.")
            );
            for id in [
                "lake-contours-in",
                "lake-contours-mn",
                "artificial-reefs-ny",
            ] {
                assert!(
                    s.layer_credit(id).unwrap().contains("Not for navigation"),
                    "{id}"
                );
                assert!(find(id).unwrap().not_for_navigation, "{id}");
            }
            assert!(GEBCO_CAPS.contains(s.layer_credit("gebco-relief").unwrap()));
        }

        #[test]
        fn depth_and_chart_captions_are_safety_text() {
            let [plain, luna] = both();
            for l in BASES.iter().chain(LAYERS.iter()) {
                assert_eq!(
                    plain.layer_legend(l.id),
                    luna.layer_legend(l.id),
                    "{}",
                    l.id
                );
                assert_eq!(
                    plain.layer_credit(l.id),
                    luna.layer_credit(l.id),
                    "{}",
                    l.id
                );
            }
            let legend = |id: &str| plain.layer_legend(id).unwrap();
            assert!(legend("enc-depths").contains("unverified"));
            assert!(legend("enc-special-areas").contains("unverified"));
            assert!(legend("enc-seabed").contains("1:137,000"));
            assert!(legend("enc-structure-points").contains("No depth is shown"));
            assert!(enc::SEABED.note.contains("Lower Bay"));
        }

        #[test]
        fn state_layers_state_their_depth_reference_and_terms() {
            let s = Strings::new(Voice::Plain);
            for id in [
                "lake-contours-in",
                "lake-contours-mn",
                "artificial-reefs-ny",
            ] {
                assert!(!s.layer_licence(id).unwrap().is_empty(), "{id}");
                assert!(
                    !s.layer_reference(id).unwrap().contains("pproximate depth"),
                    "{id}"
                );
            }
            let reference = |id: &str| s.layer_reference(id).unwrap();
            let licence = |id: &str| s.layer_licence(id).unwrap();
            assert!(reference("lake-contours-in").contains("lake surface"));
            assert!(reference("lake-contours-in").contains("gauge"));
            assert!(reference("lake-contours-mn").contains("unconfirmed"));
            assert!(licence("lake-contours-mn").contains("MNDNR must be acknowledged"));
            assert!(licence("lake-contours-mn").contains("sold"));
            assert!(reference("artificial-reefs-ny").contains("no datum"));
            assert!(licence("lake-contours-in").contains("Indiana DNR"));
            assert_eq!(s.layer_reference("sst"), None);
            assert_eq!(s.layer_licence("sst"), None);
        }

        #[test]
        fn imagery_layers_carry_their_acknowledgements() {
            let s = Strings::new(Voice::Luna);
            assert!(s.layer_acknowledgements("true-color-viirs-n20")[0].contains("(GIBS)"));
            assert_eq!(s.layer_acknowledgements("sst-imagery").len(), 2);
            assert!(s.layer_acknowledgements("sst")[0].contains("MEaSUREs"));
            assert_eq!(s.layer_acknowledgements("radar"), Vec::<&str>::new());
        }

        fn formatter(metric: bool) -> Formatter {
            use crate::units::{
                DischargeUnit, DistanceUnit, HeightUnit, PressureUnit, SpeedUnit, TempUnit,
            };
            let (temperature, height, speed, distance) = if metric {
                (TempUnit::C, HeightUnit::M, SpeedUnit::Ms, DistanceUnit::Km)
            } else {
                (TempUnit::F, HeightUnit::Ft, SpeedUnit::Kn, DistanceUnit::Mi)
            };
            let units = crate::view::DisplayUnits {
                temperature,
                air_temperature: temperature,
                height,
                stage: height,
                lake_level: height,
                waves: height,
                discharge: DischargeUnit::Cfs,
                current_speed: speed,
                wind_speed: speed,
                pressure: PressureUnit::Hpa,
                distance,
            };
            Formatter::new(
                units,
                Voice::Plain,
                whether_core::presentation::TimeFormat::H12,
                "America/New_York",
            )
        }

        #[test]
        fn map_values_read_in_the_users_units() {
            let (us, si) = (formatter(false), formatter(true));
            let t = MapQuantity::of_layer("sst").unwrap();
            assert_eq!(map_value(&us, t, 15.0), "59 °F");
            assert_eq!(map_value(&si, t, 15.04), "15.0 °C");
            assert_eq!(MapQuantity::of_layer("great-lakes-sst"), Some(t));
            let chl = MapQuantity::Chlorophyll;
            assert_eq!(map_value(&us, chl, 0.034), "0.03 µg/L");
            assert_eq!(map_value(&us, chl, 2.46), "2.5 µg/L");
            assert_eq!(map_value(&us, chl, 12.4), "12 µg/L");
            assert_eq!(map_value(&si, MapQuantity::Kd490, 0.254), "0.25 1/m");
            assert_eq!(map_value(&us, MapQuantity::CurrentSpeed, 0.5144), "1.0 kn");
            assert_eq!(MapQuantity::of_layer("radar"), None);
            assert_eq!(map_current(&us, 0.5144, 45.0), "1.0 kn toward NE 45°");
        }

        #[test]
        fn species_records_are_dated_and_placed_in_the_users_units() {
            let (us, si) = (formatter(false), formatter(true));
            let day = LocalDate {
                year: 2026,
                month: 11,
                day: 3,
            };
            assert_eq!(record_seen(&us, Some(day)), "Seen Nov 3 2026");
            assert_eq!(record_seen(&us, None), "Date not recorded");
            assert_eq!(
                record_position(&us, false, Some(30.0)).as_deref(),
                Some("Position good to 98 ft")
            );
            assert_eq!(
                record_position(&si, false, Some(30.0)).as_deref(),
                Some("Position good to 30 m")
            );
            assert_eq!(
                record_position(&si, true, Some(27_000.0)).as_deref(),
                Some("Position blurred by the source, within 27 km")
            );
            assert_eq!(
                record_position(&us, true, None).as_deref(),
                Some("Position blurred by the source")
            );
            assert_eq!(record_position(&us, false, None), None);
            assert_eq!(record_position(&us, false, Some(f32::NAN)), None);
        }

        #[test]
        fn depth_words_read_in_the_users_units_and_name_the_datum() {
            let (us, si) = (formatter(false), formatter(true));
            assert_eq!(depth_label(&us, 3.048), "10 ft");
            assert_eq!(depth_label(&us, 29.6), "97.1 ft");
            assert_eq!(depth_label(&si, 6.096), "6.1 m");
            assert_eq!(depth_label(&si, 10.0), "10 m");
            let reading = |datum: Option<&str>, source: Option<&str>| ApproxDepth {
                reading: DepthReading::Depth { depth_m: 29.6 },
                vertical_datum: datum.map(str::to_owned),
                source: source.map(str::to_owned),
            };
            assert_eq!(
                depth_reading(&us, &reading(Some("EGM 2008"), Some("crm_vol1_2023"))),
                "approximate depth 97.1 ft (EGM 2008, Coastal Relief Model)"
            );
            assert_eq!(
                depth_reading(
                    &si,
                    &reading(Some("NAVD 88"), Some("ncei19_n40x75_w074x25_2015v1"))
                ),
                "approximate depth 29.6 m (NAVD 88, CUDEM)"
            );
            assert_eq!(
                depth_reading(&us, &reading(None, Some("mystery"))),
                "approximate depth 97.1 ft (datum not stated)"
            );
            let bed = ApproxDepth {
                reading: DepthReading::BedElevation {
                    elevation_m: -3.048,
                },
                vertical_datum: Some("NAVD 88".to_owned()),
                source: None,
            };
            assert_eq!(depth_reading(&us, &bed), "bed elevation -10 ft (NAVD 88)");
        }

        #[test]
        fn breaks_structure_and_reefs_are_worded_in_the_users_units() {
            let (us, si) = (formatter(false), formatter(true));
            assert_eq!(break_strength(&si, BreakKind::Sst, 0.5), "0.5 °C per km");
            assert_eq!(break_strength(&us, BreakKind::Sst, 0.5), "1.4 °F per mi");
            assert_eq!(
                break_strength(&si, BreakKind::Chlorophyll, 0.0375),
                "×1.09 per km"
            );
            assert_eq!(structure_title(&us, StructureKind::Wreck, false), "Wreck");
            assert_eq!(
                structure_title(&us, StructureKind::Obstruction, true),
                "Fish haven"
            );
            assert_eq!(
                structure_category(&us, "dangerous wreck", false),
                "As charted: dangerous wreck"
            );
            assert!(structure_category(&us, "x", true).ends_with('…'));
            let date = |month, day| SourceDate {
                year: 2019,
                month,
                day,
            };
            assert_eq!(structure_surveyed(&us, date(None, None)), "Surveyed 2019");
            assert_eq!(
                structure_surveyed(&us, date(Some(3), Some(4))),
                "Surveyed 2019-03-04"
            );
            assert_eq!(
                reef_depths(&us, Some(6.096), Some(21.336)).as_deref(),
                Some("Depth 20–70 ft")
            );
            assert_eq!(
                reef_depths(&si, Some(10.0), Some(10.0)).as_deref(),
                Some("Depth 10 m")
            );
            assert_eq!(reef_depths(&si, None, None), None);
        }

        #[test]
        fn map_times_and_measures_read_in_the_users_clock_and_units() {
            let us = formatter(false);
            // 2026-11-07T09:00:00Z, 4 AM in New York.
            let t = UnixTime(1_794_042_000);
            assert_eq!(map_time(&us, t, DateMode::Daily), "Nov 7");
            assert_eq!(map_time(&us, t, DateMode::Hourly), "Nov 7, 4:00 AM");
            let (distance, bearing, line) = map_measure(&us, 3_862.4, 90.0);
            assert_eq!(distance, "2.4 mi");
            assert_eq!(bearing, "E 90°");
            assert_eq!(line, "2.4 mi at E 90°");
            let (km, _, _) = map_measure(&formatter(true), 3_862.4, 0.0);
            assert_eq!(km, "3.9 km");
        }

        #[test]
        fn presets_and_the_unavailable_reason_are_worded() {
            for s in both() {
                for p in &PRESETS {
                    assert!(s.preset_name(p.id).is_some(), "{}", p.id);
                }
                assert_eq!(s.preset_name("surf"), Some("Surf"));
                assert_eq!(s.preset_name("Sunday morning"), None);
            }
            let reason = crate::map::catalogue::UnavailableReason::NoSource;
            assert_eq!(
                Strings::new(Voice::Plain).unavailable_reason(reason),
                "No source yet"
            );
            assert_eq!(
                Strings::new(Voice::Luna).unavailable_reason(reason),
                "Luna hasn't found a source yet"
            );
        }
    }
}
