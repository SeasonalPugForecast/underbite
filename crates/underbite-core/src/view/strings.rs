//! The words the view shows, in both voices.
//!
//! One table holds every fixed user-visible text. Playful texts ([`Key`]) have a plain and a Luna
//! variant; safety texts ([`SafetyKey`]) have exactly one, and no function takes a voice for them:
//! the split is by type, so a safety text cannot be given a Luna variant by accident. A later
//! localisation replaces this table.

use whether_core::time::Weekday;

use crate::discover::hydro::{WaterTypeReason, WaterbodyKind};
use crate::model::{
    AlertSeverity, Measurement, MoonPhase, PrincipalPhase, TideKind, WaterType, ZoneKind,
};
use crate::provenance::{SourceId, UsgsSiteType};
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
    fn measurements_have_labels() {
        assert_eq!(
            Strings::new(Voice::Plain).measurement(Measurement::Discharge),
            "River flow"
        );
    }
}
