//! The words the view shows, in both voices.
//!
//! One table holds every fixed user-visible text. Playful texts ([`Key`]) have a plain and a Luna
//! variant; safety texts ([`SafetyKey`]) have exactly one, and no function takes a voice for them:
//! the split is by type, so a safety text cannot be given a Luna variant by accident. A later
//! localisation replaces this table.

use whether_core::time::Weekday;

use crate::discover::hydro::{WaterTypeReason, WaterbodyKind};
use crate::map::catalogue::{DateMode, Group, UnavailableReason};
use crate::model::{
    AlertSeverity, Measurement, MoonPhase, PrincipalPhase, TideKind, WaterType, ZoneKind,
};
use crate::provenance::{Relation, SourceId, UsgsSiteType};
use crate::time::{UnixTime, UtcOffset};
use crate::view::format::Formatter;
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

            /// `(plain, luna)`.
            const fn variants(self) -> (&'static str, &'static str) {
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
    GoNoGoBad => (
                "Not recommended: {reason}",
                "Luna's not impressed: {reason}",
            ),
    GoNoGoGood => ("Good conditions.", "Luna approves."),
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
    Upstream => ("upstream", "upstream"),
    Downstream => ("downstream", "downstream"),
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
    RuleWadeableBelow => ("Wadeable below", "Wadeable below"),
    RuleBestFrom => ("Best from", "Best from"),
    RuleBestUpTo => ("Best up to", "Best up to"),
    RuleBlownOutAbove => ("Blown out above", "Blown out above"),
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
    MeasTurbidity => ("Turbidity", "Turbidity"),
    MeasDissolvedOxygen => ("Dissolved oxygen", "Dissolved oxygen"),
    MeasConductivity => ("Conductivity", "Conductivity"),
    MeasSalinity => ("Salinity", "Salinity"),
    MeasPh => ("pH", "pH"),
    MeasChlorophyll => ("Chlorophyll", "Chlorophyll"),
    MeasClarity => ("Water clarity", "Water clarity"),
    MeasCyanobacteria => ("Cyanobacteria", "Cyanobacteria"),
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
    StormPartMorning => "morning",
    StormPartAfternoon => "afternoon",
    StormPartNight => "night",
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
    HurricaneHurricane => "Hurricane",
    HurricaneTropicalStorm => "Tropical Storm",
    HurricaneTropicalDepression => "Tropical Depression",
    HurricaneSubtropicalStorm => "Subtropical Storm",
    HurricaneSubtropicalDepression => "Subtropical Depression",
    HurricanePostTropical => "Post-Tropical Cyclone",
    HurricanePotentialTropical => "Potential Tropical Cyclone",
    HurricaneTyphoon => "Typhoon",
    HurricaneCyclone => "Tropical Cyclone",
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

/// The table, bound to a voice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Strings {
    pub voice: Voice,
}

impl Strings {
    pub const fn new(voice: Voice) -> Self {
        Self { voice }
    }

    /// A playful text in the voice.
    pub const fn get(self, key: Key) -> &'static str {
        let (plain, luna) = key.variants();
        match self.voice {
            Voice::Plain => plain,
            Voice::Luna => luna,
        }
    }

    /// A playful text with its `{name}` placeholders filled.
    pub fn fmt(self, key: Key, args: &[(&str, &str)]) -> String {
        fill(self.get(key), args)
    }

    /// A safety text: the same in every voice.
    #[allow(clippy::unused_self)]
    pub const fn safety(self, key: SafetyKey) -> &'static str {
        key.text()
    }

    /// A safety text with its `{name}` placeholders filled: the same in every voice.
    #[allow(clippy::unused_self)]
    pub fn safety_fmt(self, key: SafetyKey, args: &[(&str, &str)]) -> String {
        fill(key.text(), args)
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
    pub const fn measurement(self, m: Measurement) -> &'static str {
        self.get(match m {
            Measurement::WaterTemperature => Key::MeasWaterTemperature,
            Measurement::Discharge => Key::MeasDischarge,
            Measurement::GageHeight => Key::MeasGageHeight,
            Measurement::LakeLevel => Key::MeasLakeLevel,
            Measurement::Turbidity => Key::MeasTurbidity,
            Measurement::DissolvedOxygen => Key::MeasDissolvedOxygen,
            Measurement::Conductivity => Key::MeasConductivity,
            Measurement::Salinity => Key::MeasSalinity,
            Measurement::Ph => Key::MeasPh,
            Measurement::Chlorophyll => Key::MeasChlorophyll,
            Measurement::Clarity => Key::MeasClarity,
            Measurement::Cyanobacteria => Key::MeasCyanobacteria,
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

    /// How a station relates to the spot: "Same water body", "Upstream", ...
    pub const fn relation(self, relation: Relation) -> &'static str {
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
    pub const fn card_title(self, id: CardId) -> &'static str {
        let luna = matches!(self.voice, Voice::Luna);
        match id {
            CardId::Now => "Now",
            CardId::Tide => "Tide",
            CardId::Current => "Current",
            CardId::Flow => "River flow",
            CardId::WaterTemp => "Water temperature",
            CardId::Clarity => "Water clarity",
            CardId::Waves => "Waves",
            CardId::Wind => "Wind",
            CardId::Pressure => "Pressure",
            CardId::SunMoon => "Sun and moon",
            CardId::Solunar => "Solunar",
            CardId::Windows if luna => "Snack times",
            CardId::Windows => "Fishing windows",
            CardId::GoNoGo if luna => "Luna's verdict",
            CardId::GoNoGo => "Go or no go",
            CardId::StormAhead => "Storm ahead",
            CardId::MarineForecast => "Marine forecast",
            CardId::Stocking => "Stocking",
            CardId::Species => "Species",
            CardId::Alerts => "Alerts",
            CardId::Sources => "Sources",
        }
    }

    /// "High" or "Low".
    pub const fn tide_kind(self, kind: TideKind) -> &'static str {
        self.get(match kind {
            TideKind::High => Key::TideHigh,
            TideKind::Low => Key::TideLow,
        })
    }

    /// "Waxing gibbous", "Full moon", ...
    pub const fn moon_phase(self, phase: MoonPhase) -> &'static str {
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
    pub const fn principal_phase(self, phase: PrincipalPhase) -> &'static str {
        self.get(match phase {
            PrincipalPhase::New => Key::MoonNew,
            PrincipalPhase::FirstQuarter => Key::MoonFirstQuarter,
            PrincipalPhase::Full => Key::MoonFull,
            PrincipalPhase::LastQuarter => Key::MoonLastQuarter,
        })
    }

    /// "Mon", "Tue", ...
    pub const fn weekday(self, day: Weekday) -> &'static str {
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
    pub const fn water_type(self, water_type: WaterType) -> &'static str {
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
}

/// The map's words: the catalogue holds ids and data, the words are here, looked up by layer or
/// preset id. Titles, colour-scale captions, preset names and the unavailable reason are playful
/// texts; credits, acknowledgements, depth references, terms and the captions of depth and chart
/// layers are safety texts, the same in every voice.
impl Strings {
    /// A drawer section's title.
    pub const fn map_group(self, group: Group) -> &'static str {
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
    pub const fn unavailable_reason(self, reason: UnavailableReason) -> &'static str {
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
            format!("{} µg/L", Formatter::number(v, decimals))
        }
        MapQuantity::Kd490 => format!("{} 1/m", Formatter::number(v, 2)),
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
pub const MEASUREMENTS: [Measurement; 25] = [
    Measurement::Tide,
    Measurement::TidalCurrent,
    Measurement::WaterTemperature,
    Measurement::Discharge,
    Measurement::GageHeight,
    Measurement::LakeLevel,
    Measurement::Turbidity,
    Measurement::Clarity,
    Measurement::DissolvedOxygen,
    Measurement::Conductivity,
    Measurement::Salinity,
    Measurement::Ph,
    Measurement::Chlorophyll,
    Measurement::Cyanobacteria,
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

/// A source's name as displayed.
pub fn source_name(id: &SourceId) -> String {
    match id.as_str() {
        "usgs" => "USGS".to_owned(),
        "coops" => "NOAA CO-OPS".to_owned(),
        "coops-harmonics" => "NOAA CO-OPS harmonics".to_owned(),
        "ndbc" => "NOAA NDBC".to_owned(),
        "nwps" => "NOAA NWPS".to_owned(),
        "open-meteo-marine" => "Open-Meteo Marine".to_owned(),
        "nws" => "National Weather Service".to_owned(),
        "usgs-3dhp" => "USGS 3DHP".to_owned(),
        other => other.to_owned(),
    }
}

/// English month abbreviation, 1-based; no locale lookups.
pub const fn month_abbrev(month: u8) -> &'static str {
    match month {
        1 => "Jan",
        2 => "Feb",
        3 => "Mar",
        4 => "Apr",
        5 => "May",
        6 => "Jun",
        7 => "Jul",
        8 => "Aug",
        9 => "Sep",
        10 => "Oct",
        11 => "Nov",
        12 => "Dec",
        _ => "?",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            let (plain, luna) = key.variants();
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
            | Measurement::Turbidity
            | Measurement::DissolvedOxygen
            | Measurement::Conductivity
            | Measurement::Salinity
            | Measurement::Ph
            | Measurement::Chlorophyll
            | Measurement::Clarity
            | Measurement::Cyanobacteria
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
