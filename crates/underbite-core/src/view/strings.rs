//! The words the view shows, in both voices.
//!
//! One table holds every fixed user-visible text. Playful texts ([`Key`]) have a plain and a Luna
//! variant; safety texts ([`SafetyKey`]) have exactly one, and no function takes a voice for them:
//! the split is by type, so a safety text cannot be given a Luna variant by accident. A later
//! localisation replaces this table.

use whether_core::time::Weekday;

use crate::model::{MoonPhase, PrincipalPhase, TideKind, WaterType};
use crate::provenance::SourceId;
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
    /// The placeholder for an unknown value.
    Missing => ("—", "—"),
}

/// Texts that are never playful: safety, advisories and regulations. No voice parameter exists
/// for them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SafetyKey {
    NotForNavigation,
    Modelled,
    /// The tide card's note when the source is a sea-level model, not a tide prediction.
    ModelledSeaLevel,
    Provisional,
    Approximate,
    TraditionalCue,
    TextOnly,
    RegulationsLink,
    /// `{threshold}`: the user's trout-stress temperature.
    TroutStress,
}

impl SafetyKey {
    /// The one text.
    pub const fn text(self) -> &'static str {
        match self {
            Self::NotForNavigation => "Not for navigation.",
            Self::Modelled => "Modelled data: an estimate, not a measurement.",
            Self::ModelledSeaLevel => {
                "Modelled sea level, not a tide prediction; times can be off by hours near the coast."
            }
            Self::Provisional => "Provisional data, subject to revision.",
            Self::Approximate => "Approximate: check conditions locally before you go.",
            Self::TraditionalCue => "A traditional cue, not a promise.",
            Self::TextOnly => "Text only: open the map for the picture.",
            Self::TroutStress => "Consider not fishing for trout above {threshold}.",
            Self::RegulationsLink => {
                "Check the official source for current regulations; this is not legal advice."
            }
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
        for key in [
            SafetyKey::NotForNavigation,
            SafetyKey::Modelled,
            SafetyKey::ModelledSeaLevel,
            SafetyKey::Provisional,
            SafetyKey::Approximate,
            SafetyKey::TraditionalCue,
            SafetyKey::TextOnly,
            SafetyKey::RegulationsLink,
            SafetyKey::TroutStress,
        ] {
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
}
