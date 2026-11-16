//! Formatting of values, units, times and provenance footnotes.
//!
//! The only place values are converted and rounded for display. Conversions go through
//! [`crate::units`]; times are rendered in the spot's IANA zone (never a fixed offset); nothing
//! here depends on the locale, so output is deterministic.

use jiff::Timestamp;
use jiff::tz::TimeZone;
use whether_core::presentation::TimeFormat;

use crate::datum::{Height, VerticalDatum};
use crate::model::{PercentileClass, TurbidityUnit, WaterType};
use crate::provenance::{Provenance, Relation, ValueKind};
use crate::time::{LocalDate, UnixTime, UtcOffset, zone};
use crate::units::PressureUnit;
use crate::units::{DischargeUnit, DistanceUnit, pressure_from_hpa};
use crate::view::strings::{Key, SafetyKey, Strings, month_abbrev, source_name};
use crate::view::{DisplayUnits, Notice, NoticeKind, Quantity, ViewInput, Voice};

pub use whether_core::view::format::number;

/// The tidal phase of a current.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Flood,
    Ebb,
    Slack,
}

/// A station footer: the text shown (in the voice) and the plain text for `VoiceOver`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Footer {
    pub text: String,
    pub accessibility: String,
}

impl Quantity {
    /// A quantity with its unit: "1,200 cfs". An unknown value is the placeholder alone.
    pub fn full(&self) -> String {
        if self.value.is_none() || self.unit.is_empty() {
            self.text.clone()
        } else if self.unit == "%" {
            format!("{}%", self.text)
        } else {
            format!("{} {}", self.text, self.unit)
        }
    }
}

/// Formats values for one spot, in the user's units, voice and clock.
#[derive(Clone, Debug)]
pub struct Formatter {
    pub units: DisplayUnits,
    pub strings: Strings,
    pub clock: TimeFormat,
    tz: TimeZone,
}

impl Formatter {
    /// A formatter for a zone name; an unknown zone falls back to UTC.
    pub fn new(units: DisplayUnits, voice: Voice, clock: TimeFormat, time_zone: &str) -> Self {
        Self {
            units,
            strings: Strings::new(voice),
            clock,
            tz: zone(time_zone).unwrap_or(TimeZone::UTC),
        }
    }

    /// A formatter for a view input, in its spot's zone.
    pub fn for_input(input: &ViewInput) -> Self {
        Self::new(
            input.units,
            input.voice,
            input.clock,
            &input.snapshot.spot.time_zone,
        )
    }

    fn missing(&self, unit: &str) -> Quantity {
        Quantity {
            value: None,
            text: self.strings.get(Key::Missing).to_owned(),
            unit: unit.to_owned(),
        }
    }

    fn quantity(value: f32, decimals: u8, unit: &str) -> Quantity {
        Quantity {
            value: Some(value),
            text: number(value, decimals),
            unit: unit.to_owned(),
        }
    }

    // -- values ----------------------------------------------------------------------------

    /// "1,200 cfs" (whole numbers, grouped), "12.4 cfs" below 100 cfs, "340 m³/s" or "3.4 m³/s"
    /// below 10 m³/s: small flows keep the decimal they are read to.
    pub fn discharge(&self, m3s: Option<f32>) -> Quantity {
        let unit = self.units.discharge;
        let Some(m3s) = m3s else {
            return self.missing(unit.symbol());
        };
        let v = unit.from_canonical(m3s);
        let small = match unit {
            DischargeUnit::Cfs => 100.0,
            DischargeUnit::M3s => 10.0,
        };
        let decimals = u8::from(v.abs() < small);
        Quantity {
            value: Some(v),
            text: Self::number(v, decimals),
            unit: unit.symbol().to_owned(),
        }
    }

    /// A number with the thousands grouped: "1,500", "12,345.6".
    pub fn number(value: f32, decimals: u8) -> String {
        let text = number(value, decimals);
        match text.split_once('.') {
            Some((whole, fraction)) => format!("{}.{fraction}", group_thousands(whole)),
            None => group_thousands(&text),
        }
    }

    /// A height above a datum: "4.2 ft MLLW". The datum is part of the unit. A height in a
    /// model's mean sea level is never printed as a number.
    pub fn height(&self, h: Option<Height>) -> Quantity {
        let unit = self.units.height;
        let Some(h) = h else {
            return self.missing(unit.symbol());
        };
        if h.datum == VerticalDatum::ModelMean {
            return self.missing("");
        }
        let label = format!("{} {}", unit.symbol(), datum_label(h.datum));
        Self::quantity(unit.from_canonical(h.metres), 1, &label)
    }

    /// A length or a difference that has no datum: wave height, depth, a rise: "3.3 ft".
    pub fn length(&self, metres: Option<f32>) -> Quantity {
        let unit = self.units.height;
        match metres {
            Some(m) => Self::quantity(unit.from_canonical(m), 1, unit.symbol()),
            None => self.missing(unit.symbol()),
        }
    }

    /// A tidal current: one decimal, "1.4 kn".
    pub fn current_speed(&self, ms: Option<f32>) -> Quantity {
        let unit = self.units.current_speed;
        match ms {
            Some(v) => Self::quantity(unit.from_canonical(v), 1, unit.symbol()),
            None => self.missing(unit.symbol()),
        }
    }

    /// Wind: whole numbers, one decimal in m/s.
    pub fn wind_speed(&self, ms: Option<f32>) -> Quantity {
        let unit = self.units.wind_speed;
        let decimals = u8::from(unit == crate::units::SpeedUnit::Ms);
        match ms {
            Some(v) => Self::quantity(unit.from_canonical(v), decimals, unit.symbol()),
            None => self.missing(unit.symbol()),
        }
    }

    /// "54 °F" (whole degrees) or "12.3 °C".
    pub fn temperature(&self, c: Option<f32>) -> Quantity {
        let unit = self.units.temperature;
        let decimals = u8::from(unit == crate::units::TempUnit::C);
        match c {
            Some(v) => Self::quantity(unit.from_canonical(v), decimals, unit.symbol()),
            None => self.missing(unit.symbol()),
        }
    }

    /// "30.12 inHg", "1020 hPa", "765 mmHg", "102.0 kPa".
    pub fn pressure(&self, hpa: Option<f32>) -> Quantity {
        let unit = self.units.pressure;
        let decimals = match unit {
            PressureUnit::Hpa | PressureUnit::MmHg => 0,
            PressureUnit::InHg => 2,
            PressureUnit::Kpa => 1,
        };
        match hpa {
            Some(v) => Self::quantity(pressure_from_hpa(unit, v), decimals, unit.symbol()),
            None => self.missing(unit.symbol()),
        }
    }

    /// A station distance in the user's unit: "0.4 mi", "12 mi".
    pub fn distance(&self, metres: Option<f32>) -> Quantity {
        let unit = self.units.distance;
        let Some(m) = metres else {
            return self.missing(unit.symbol());
        };
        let v = match unit {
            DistanceUnit::Km => m / 1_000.0,
            DistanceUnit::Mi => m / 1_609.344,
        };
        Self::quantity(v, u8::from(v < 10.0), unit.symbol())
    }

    /// Turbidity keeps the unit it was measured in; it is never converted: "2.1 FNU".
    pub fn turbidity(&self, value: Option<f32>, unit: TurbidityUnit) -> Quantity {
        match value {
            Some(v) => Self::quantity(v, 1, unit.label()),
            None => self.missing(unit.label()),
        }
    }

    /// "62%".
    pub fn percent(&self, v: Option<f32>) -> Quantity {
        match v {
            Some(v) => Self::quantity(v, 0, "%"),
            None => self.missing("%"),
        }
    }

    /// "SW 225°": a 16-point compass name and the rounded bearing the wind or wave comes from.
    pub fn direction(&self, degrees: Option<f32>) -> String {
        let Some(d) = degrees else {
            return self.strings.get(Key::Missing).to_owned();
        };
        let d = d.rem_euclid(360.0);
        let shown = number(d, 0);
        let shown = if shown == "360" { "0" } else { &shown };
        format!("{} {shown}°", compass(d))
    }

    /// "Flood", "Ebb" or "Slack".
    pub fn phase(&self, phase: Phase) -> &'static str {
        self.strings.get(match phase {
            Phase::Flood => Key::Flood,
            Phase::Ebb => Key::Ebb,
            Phase::Slack => Key::Slack,
        })
    }

    // -- times -----------------------------------------------------------------------------

    fn offset_at(&self, t: UnixTime) -> UtcOffset {
        Timestamp::from_second(t.0).map_or(UtcOffset(0), |ts| {
            UtcOffset(self.tz.to_offset(ts).seconds())
        })
    }

    /// A time of day in the spot's zone: "7:03 AM" or "07:03".
    pub fn clock(&self, t: UnixTime) -> String {
        let l = t.to_local(self.offset_at(t));
        match self.clock {
            TimeFormat::H24 => format!("{:02}:{:02}", l.hour, l.minute),
            TimeFormat::H12 => {
                let (h, ampm) = twelve_hour(l.hour);
                format!("{h}:{:02} {ampm}", l.minute)
            }
        }
    }

    /// An axis label: "4 PM" or "16:00".
    pub fn hour_label(&self, t: UnixTime) -> String {
        let l = t.to_local(self.offset_at(t));
        match self.clock {
            TimeFormat::H24 => format!("{:02}:00", l.hour),
            TimeFormat::H12 => {
                let (h, ampm) = twelve_hour(l.hour);
                format!("{h} {ampm}")
            }
        }
    }

    /// "Nov 8".
    pub fn date(&self, d: LocalDate) -> String {
        format!("{} {}", month_abbrev(d.month), d.day)
    }

    /// The date of an instant in the spot's zone: "Nov 8".
    pub fn date_of(&self, t: UnixTime) -> String {
        self.date(t.to_local(self.offset_at(t)).date)
    }

    /// "just now", "6 min ago", "3 h ago", "2 d ago"; a time in the future reads "just now".
    pub fn age(&self, seconds: i64) -> String {
        let s = seconds.max(0);
        let (key, n) = if s < 60 {
            (Key::AgeJustNow, 0)
        } else if s < 3_600 {
            (Key::AgeMinutes, s / 60)
        } else if s < 86_400 {
            (Key::AgeHours, s / 3_600)
        } else {
            (Key::AgeDays, s / 86_400)
        };
        self.strings.fmt(key, &[("n", &n.to_string())])
    }

    /// The header line: "updated 6 min ago".
    pub fn updated(&self, fetched: UnixTime, now: UnixTime) -> String {
        let s = (now.0 - fetched.0).max(0);
        let (key, n) = if s < 60 {
            (Key::UpdatedJustNow, 0)
        } else if s < 3_600 {
            (Key::UpdatedMinutes, s / 60)
        } else if s < 86_400 {
            (Key::UpdatedHours, s / 3_600)
        } else {
            (Key::UpdatedDays, s / 86_400)
        };
        self.strings.fmt(key, &[("n", &n.to_string())])
    }

    /// "normal, 45th percentile for Nov 8". The class comes from the model (it is decided in the
    /// derivation, not here); `rank` adds the percentile. A clamped rank reads "below the 5th
    /// percentile" or "above the 95th percentile". Without a class there is no text.
    pub fn percentile(
        &self,
        class: Option<PercentileClass>,
        rank: Option<f32>,
        rank_clamped: bool,
        on: LocalDate,
    ) -> Option<String> {
        let s = self.strings;
        let date = self.date(on);
        let band = match class? {
            PercentileClass::RecordLow => {
                return Some(s.fmt(Key::PercentileRecordLow, &[("date", &date)]));
            }
            PercentileClass::RecordHigh => {
                return Some(s.fmt(Key::PercentileRecordHigh, &[("date", &date)]));
            }
            PercentileClass::MuchBelowNormal => Key::PercentileMuchBelow,
            PercentileClass::BelowNormal => Key::PercentileBelow,
            PercentileClass::Normal => Key::PercentileNormal,
            PercentileClass::AboveNormal => Key::PercentileAbove,
            PercentileClass::MuchAboveNormal => Key::PercentileMuchAbove,
        };
        let band = s.get(band);
        let Some(rank) = rank else {
            return Some(s.fmt(Key::PercentileNoRank, &[("band", band), ("date", &date)]));
        };
        let rank = rank.clamp(0.0, 100.0);
        let text = if rank_clamped {
            let (key, bound) = if rank < 50.0 {
                (Key::PercentileRankBelow, 5)
            } else {
                (Key::PercentileRankAbove, 95)
            };
            s.fmt(key, &[("ordinal", &ordinal(bound))])
        } else {
            s.fmt(
                Key::PercentileRank,
                &[("ordinal", &ordinal(round_u32(rank)))],
            )
        };
        Some(s.fmt(
            Key::PercentileText,
            &[("band", band), ("rank", &text), ("date", &date)],
        ))
    }

    // -- provenance ------------------------------------------------------------------------

    /// "Hudson R at Battery, NY · 0.4 mi · 6 min ago", the parts after the value.
    fn provenance_parts(&self, p: &Provenance, now: UnixTime) -> Vec<String> {
        let mut parts = vec![match &p.station {
            Some(s) => s.name.clone(),
            None => source_name(&p.source),
        }];
        let distance = p.distance_m.map(|m| self.distance(Some(m)).full());
        match (p.relation, distance) {
            (Relation::Upstream, Some(d)) => {
                parts.push(format!("{d} {}", self.strings.get(Key::Upstream)));
            }
            (Relation::Downstream, Some(d)) => {
                parts.push(format!("{d} {}", self.strings.get(Key::Downstream)));
            }
            (Relation::Gridded, _) => parts.push(self.strings.get(Key::Modelled).to_owned()),
            (Relation::Satellite, _) => parts.push(self.strings.get(Key::Satellite).to_owned()),
            (_, Some(d)) => parts.push(d),
            (_, None) => {}
        }
        if p.kind != ValueKind::Predicted {
            parts.push(self.age(now.0 - p.observed_at.0));
        }
        parts
    }

    /// The provenance footnote, always plain: "Water 54 °F · Hudson R at Battery, NY · 0.4 mi ·
    /// 6 min ago". `value` is the label and value ("Water 54 °F").
    pub fn footnote(&self, value: &str, p: &Provenance, now: UnixTime) -> String {
        let mut parts = vec![value.to_owned()];
        parts.extend(self.provenance_parts(p, now));
        parts.join(" · ")
    }

    /// The station footer in the user's voice ("Luna's watching Hudson R at Battery, 0.4 mi"),
    /// with the plain text for `VoiceOver`. Without a station or distance it is the source name.
    pub fn station_footer(&self, p: &Provenance) -> Footer {
        let station = match &p.station {
            Some(s) => s.name.clone(),
            None => source_name(&p.source),
        };
        let distance = self.distance(p.distance_m).full();
        let args = [
            ("station", station.as_str()),
            ("distance", distance.as_str()),
        ];
        let plain = Strings::new(Voice::Plain).fmt(Key::StationFooter, &args);
        let text = if p.distance_m.is_some() {
            self.strings.fmt(Key::StationFooter, &args)
        } else {
            station.clone()
        };
        let plain = if p.distance_m.is_some() {
            plain
        } else {
            station
        };
        Footer {
            text,
            accessibility: plain,
        }
    }

    /// Why a value beyond the user's distance limit is greyed.
    pub fn far_reason(&self, distance_m: f32, limit_m: f32) -> String {
        self.strings.fmt(
            Key::FarReason,
            &[
                ("distance", &self.distance(Some(distance_m)).full()),
                ("limit", &self.distance(Some(limit_m)).full()),
            ],
        )
    }

    /// Why a value older than the user's age limit is greyed.
    pub fn stale_reason(&self, age_s: i64, limit_s: i64) -> String {
        let bare = |s: i64| self.age(s).trim_end_matches(" ago").to_owned();
        self.strings.fmt(
            Key::StaleReason,
            &[("age", &bare(age_s)), ("limit", &bare(limit_s))],
        )
    }

    // -- fixed texts -----------------------------------------------------------------------

    /// A fixed notice: plain text in every voice.
    pub fn notice(&self, kind: NoticeKind) -> Notice {
        Notice {
            kind,
            text: self.strings.safety(SafetyKey::from(kind)).to_owned(),
        }
    }

    /// "River or stream" and so on.
    pub fn water_type_label(&self, water_type: WaterType) -> String {
        self.strings.water_type(water_type).to_owned()
    }
}

fn datum_label(datum: VerticalDatum) -> &'static str {
    match datum {
        VerticalDatum::Mllw => "MLLW",
        VerticalDatum::Mlw => "MLW",
        VerticalDatum::Msl => "MSL",
        VerticalDatum::Mtl => "MTL",
        VerticalDatum::Navd88 => "NAVD88",
        VerticalDatum::Ngvd29 => "NGVD29",
        VerticalDatum::Igld85 => "IGLD85",
        VerticalDatum::Gauge => "gage datum",
        VerticalDatum::ModelMean => "model mean",
    }
}

fn compass(degrees: f32) -> &'static str {
    const NAMES: [&str; 16] = [
        "N", "NNE", "NE", "ENE", "E", "ESE", "SE", "SSE", "S", "SSW", "SW", "WSW", "W", "WNW",
        "NW", "NNW",
    ];
    let idx = round_u32(degrees / 22.5) as usize % 16;
    NAMES[idx]
}

fn round_u32(x: f32) -> u32 {
    // INVARIANT: callers pass values in 0..=360, so the rounded value fits.
    x.round() as u32
}

fn ordinal(n: u32) -> String {
    let suffix = match (n % 100, n % 10) {
        (11..=13, _) => "th",
        (_, 1) => "st",
        (_, 2) => "nd",
        (_, 3) => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}

fn twelve_hour(hour: u8) -> (u8, &'static str) {
    let ampm = if hour < 12 { "AM" } else { "PM" };
    let h = match hour % 12 {
        0 => 12,
        h => h,
    };
    (h, ampm)
}

/// "1200" to "1,200"; handles a leading minus.
fn group_thousands(digits: &str) -> String {
    let (sign, body) = digits
        .strip_prefix('-')
        .map_or(("", digits), |rest| ("-", rest));
    let mut out = String::with_capacity(body.len() + body.len() / 3);
    for (i, c) in body.chars().enumerate() {
        if i > 0 && (body.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    format!("{sign}{out}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provenance::{Quality, SourceId, StationKind, StationRef};
    use crate::units::{HeightUnit, SpeedUnit, TempUnit};

    fn us() -> DisplayUnits {
        DisplayUnits {
            temperature: TempUnit::F,
            height: HeightUnit::Ft,
            discharge: DischargeUnit::Cfs,
            current_speed: SpeedUnit::Kn,
            wind_speed: SpeedUnit::Mph,
            pressure: PressureUnit::InHg,
            distance: DistanceUnit::Mi,
        }
    }

    fn metric() -> DisplayUnits {
        DisplayUnits {
            temperature: TempUnit::C,
            height: HeightUnit::M,
            discharge: DischargeUnit::M3s,
            current_speed: SpeedUnit::Ms,
            wind_speed: SpeedUnit::Kmh,
            pressure: PressureUnit::Hpa,
            distance: DistanceUnit::Km,
        }
    }

    fn fmt(units: DisplayUnits, voice: Voice, clock: TimeFormat) -> Formatter {
        Formatter::new(units, voice, clock, "America/New_York")
    }

    fn battery(distance_m: Option<f32>, relation: Relation, observed: i64) -> Provenance {
        Provenance {
            source: SourceId::COOPS,
            station: Some(StationRef {
                source: SourceId::COOPS,
                id: "8518750".into(),
                name: "Hudson R at Battery, NY".into(),
                kind: StationKind::WaterLevel,
                latitude: 40.7003,
                longitude: -74.0142,
                bin: None,
                reference_id: None,
            }),
            distance_m,
            relation,
            observed_at: UnixTime(observed),
            kind: ValueKind::Observed,
            quality: Quality::Approved,
        }
    }

    #[test]
    fn numbers_group_and_never_show_negative_zero() {
        assert_eq!(group_thousands("1200"), "1,200");
        assert_eq!(group_thousands("999"), "999");
        assert_eq!(group_thousands("1234567"), "1,234,567");
        assert_eq!(group_thousands("-4500"), "-4,500");
        assert_eq!(number(-0.04, 1), "0.0");
        assert_eq!(Formatter::number(1500.0, 0), "1,500");
        assert_eq!(Formatter::number(1_234_567.8, 1), "1,234,567.8");
        assert_eq!(Formatter::number(-4500.0, 0), "-4,500");
        assert_eq!(Formatter::number(12.5, 1), "12.5");
        assert_eq!(ordinal(1), "1st");
        assert_eq!(ordinal(12), "12th");
        assert_eq!(ordinal(45), "45th");
        assert_eq!(ordinal(93), "93rd");
    }

    #[test]
    fn every_unit_choice() {
        let u = fmt(us(), Voice::Plain, TimeFormat::H12);
        let m = fmt(metric(), Voice::Plain, TimeFormat::H24);
        // 34 m3/s is about 1200 cfs.
        assert_eq!(u.discharge(Some(33.98)).full(), "1,200 cfs");
        assert_eq!(m.discharge(Some(34.0)).full(), "34 m³/s");
        assert_eq!(m.discharge(Some(340.0)).full(), "340 m³/s");
        // Small flows keep one decimal: below 100 cfs and below 10 m³/s.
        assert_eq!(u.discharge(Some(0.351)).full(), "12.4 cfs");
        assert_eq!(u.discharge(Some(2.8)).full(), "98.9 cfs");
        assert_eq!(u.discharge(Some(3.0)).full(), "106 cfs");
        assert_eq!(m.discharge(Some(3.4)).full(), "3.4 m³/s");
        assert_eq!(m.discharge(Some(9.96)).full(), "10.0 m³/s");
        let h = Height {
            datum: VerticalDatum::Mllw,
            metres: 1.28,
        };
        assert_eq!(u.height(Some(h)).full(), "4.2 ft MLLW");
        assert_eq!(m.height(Some(h)).full(), "1.3 m MLLW");
        assert_eq!(u.length(Some(1.0)).full(), "3.3 ft");
        assert_eq!(m.length(Some(1.0)).full(), "1.0 m");
        assert_eq!(u.current_speed(Some(0.72)).full(), "1.4 kn");
        assert_eq!(m.current_speed(Some(0.72)).full(), "0.7 m/s");
        assert_eq!(u.wind_speed(Some(5.0)).full(), "11 mph");
        assert_eq!(m.wind_speed(Some(5.0)).full(), "18 km/h");
        assert_eq!(u.temperature(Some(12.2)).full(), "54 °F");
        assert_eq!(m.temperature(Some(12.2)).full(), "12.2 °C");
        assert_eq!(u.pressure(Some(1019.8)).full(), "30.11 inHg");
        assert_eq!(m.pressure(Some(1019.8)).full(), "1020 hPa");
        assert_eq!(u.distance(Some(644.0)).full(), "0.4 mi");
        assert_eq!(u.distance(Some(24_140.0)).full(), "15 mi");
        assert_eq!(m.distance(Some(644.0)).full(), "0.6 km");
        assert_eq!(u.turbidity(Some(2.1), TurbidityUnit::Fnu).full(), "2.1 FNU");
        assert_eq!(m.turbidity(Some(2.1), TurbidityUnit::Ntu).full(), "2.1 NTU");
        assert_eq!(u.percent(Some(62.4)).full(), "62%");
        assert_eq!(u.discharge(None).full(), "—");
    }

    #[test]
    fn model_mean_heights_are_never_numbers() {
        let u = fmt(us(), Voice::Plain, TimeFormat::H12);
        let q = u.height(Some(Height {
            datum: VerticalDatum::ModelMean,
            metres: 0.9,
        }));
        assert_eq!(q.value, None);
        assert_eq!(q.full(), "—");
        let g = u.height(Some(Height {
            datum: VerticalDatum::Gauge,
            metres: 1.0,
        }));
        assert_eq!(g.full(), "3.3 ft gage datum");
    }

    #[test]
    fn directions_and_phases() {
        let f = fmt(us(), Voice::Luna, TimeFormat::H12);
        assert_eq!(f.direction(Some(225.0)), "SW 225°");
        assert_eq!(f.direction(Some(0.0)), "N 0°");
        assert_eq!(f.direction(Some(359.7)), "N 0°");
        assert_eq!(f.direction(Some(-90.0)), "W 270°");
        assert_eq!(f.direction(None), "—");
        assert_eq!(f.phase(Phase::Flood), "Flood");
        assert_eq!(f.phase(Phase::Ebb), "Ebb");
    }

    #[test]
    fn times_follow_the_spots_zone_and_daylight_saving() {
        let f12 = fmt(us(), Voice::Plain, TimeFormat::H12);
        let f24 = fmt(us(), Voice::Plain, TimeFormat::H24);
        // 2026-11-15 12:03 UTC is 07:03 EST; 2026-07-15 11:03 UTC is 07:03 EDT.
        let winter = UnixTime(1_794_744_180);
        let summer = UnixTime(1_784_113_380);
        assert_eq!(f12.clock(winter), "7:03 AM");
        assert_eq!(f12.clock(summer), "7:03 AM");
        assert_eq!(f24.clock(winter), "07:03");
        assert_eq!(f12.hour_label(UnixTime(winter.0 + 9 * 3600)), "4 PM");
        assert_eq!(f24.hour_label(UnixTime(winter.0 + 9 * 3600)), "16:00");
        assert_eq!(f12.date_of(winter), "Nov 15");
        // 04:30 UTC is still the evening before in New York.
        assert_eq!(f12.date_of(UnixTime(1_794_717_000)), "Nov 14");
    }

    #[test]
    fn ages_and_updates() {
        let f = fmt(us(), Voice::Plain, TimeFormat::H12);
        assert_eq!(f.age(10), "just now");
        assert_eq!(f.age(360), "6 min ago");
        assert_eq!(f.age(7_300), "2 h ago");
        assert_eq!(f.age(200_000), "2 d ago");
        assert_eq!(f.age(-5), "just now");
        assert_eq!(f.updated(UnixTime(0), UnixTime(360)), "updated 6 min ago");
        assert_eq!(f.updated(UnixTime(0), UnixTime(5)), "updated just now");
    }

    #[test]
    fn percentile_text() {
        use crate::derive::percentile::class;
        let f = fmt(us(), Voice::Plain, TimeFormat::H12);
        let day = LocalDate {
            year: 2026,
            month: 11,
            day: 8,
        };
        let p = |c, r, clamped| f.percentile(c, r, clamped, day).unwrap();
        let c = Some(PercentileClass::Normal);
        assert_eq!(p(c, Some(45.0), false), "normal, 45th percentile for Nov 8");
        assert_eq!(p(c, None, false), "normal for Nov 8");
        assert_eq!(
            p(Some(PercentileClass::MuchBelowNormal), Some(4.0), false),
            "much below normal, 4th percentile for Nov 8"
        );
        assert_eq!(
            p(Some(PercentileClass::MuchAboveNormal), Some(95.0), true),
            "much above normal, above the 95th percentile for Nov 8"
        );
        assert_eq!(
            p(Some(PercentileClass::MuchBelowNormal), Some(5.0), true),
            "much below normal, below the 5th percentile for Nov 8"
        );
        assert_eq!(
            p(Some(PercentileClass::RecordLow), Some(0.0), true),
            "record low for Nov 8"
        );
        assert_eq!(
            p(Some(PercentileClass::RecordHigh), Some(100.0), true),
            "record high for Nov 8"
        );
        assert_eq!(f.percentile(None, Some(45.0), false, day), None);
        // Records come from the model's class, decided against the record min and max.
        assert_eq!(
            p(class(0.0, 1.0, Some(2.0), None), Some(0.0), true),
            "record low for Nov 8"
        );
        assert_eq!(
            p(class(100.0, 9.0, None, Some(8.0)), Some(100.0), true),
            "record high for Nov 8"
        );
        // The boundary between normal and above normal is the derivation's: 76.0, not 75.9.
        assert_eq!(
            p(class(75.9, 10.0, None, None), Some(75.9), false),
            "normal, 76th percentile for Nov 8"
        );
        assert_eq!(
            p(class(76.0, 10.0, None, None), Some(76.0), false),
            "above normal, 76th percentile for Nov 8"
        );
    }

    #[test]
    fn footnote_and_station_footer() {
        let now = UnixTime(10_000);
        let p = battery(Some(644.0), Relation::SameWaterBody, 10_000 - 360);
        let f = fmt(us(), Voice::Luna, TimeFormat::H12);
        assert_eq!(
            f.footnote("Water 54 °F", &p, now),
            "Water 54 °F · Hudson R at Battery, NY · 0.4 mi · 6 min ago"
        );
        let footer = f.station_footer(&p);
        assert_eq!(
            footer.text,
            "Luna's watching Hudson R at Battery, NY, 0.4 mi"
        );
        assert_eq!(footer.accessibility, "Hudson R at Battery, NY, 0.4 mi");
        let plain = fmt(us(), Voice::Plain, TimeFormat::H12).station_footer(&p);
        assert_eq!(plain.text, plain.accessibility);
        let up = battery(Some(6_437.0), Relation::Upstream, 10_000);
        assert_eq!(
            f.footnote("Flow 1,200 cfs", &up, now),
            "Flow 1,200 cfs · Hudson R at Battery, NY · 4.0 mi upstream · just now"
        );
        let grid = Provenance {
            station: None,
            distance_m: None,
            relation: Relation::Gridded,
            source: SourceId::OPEN_METEO_MARINE,
            kind: ValueKind::Modelled,
            ..p
        };
        assert_eq!(
            f.footnote("Waves 3.3 ft", &grid, now),
            "Waves 3.3 ft · Open-Meteo Marine · modelled · 6 min ago"
        );
    }

    #[test]
    fn reasons_use_the_users_units() {
        let f = fmt(us(), Voice::Plain, TimeFormat::H12);
        assert_eq!(
            f.far_reason(80_467.0, 48_280.0),
            "50 mi from your spot, beyond your 30 mi limit"
        );
        assert_eq!(
            f.stale_reason(7_300, 3_600),
            "2 h old, beyond your 1 h limit"
        );
    }

    #[test]
    fn notices_ignore_the_voice() {
        let luna = fmt(us(), Voice::Luna, TimeFormat::H12);
        let plain = fmt(us(), Voice::Plain, TimeFormat::H12);
        for kind in [
            NoticeKind::NotForNavigation,
            NoticeKind::Modelled,
            NoticeKind::Provisional,
            NoticeKind::Approximate,
            NoticeKind::TraditionalCue,
            NoticeKind::TextOnly,
        ] {
            assert_eq!(luna.notice(kind), plain.notice(kind));
        }
        assert_eq!(
            luna.notice(NoticeKind::NotForNavigation).text,
            "Not for navigation."
        );
        assert_eq!(
            luna.notice(NoticeKind::TraditionalCue).text,
            "A traditional cue, not a promise."
        );
    }

    #[test]
    fn labels_in_both_voices_and_unit_systems() {
        let mut out = String::new();
        for (name, units, voice) in [
            ("us luna", us(), Voice::Luna),
            ("us plain", us(), Voice::Plain),
            ("metric luna", metric(), Voice::Luna),
            ("metric plain", metric(), Voice::Plain),
        ] {
            let f = fmt(units, voice, TimeFormat::H12);
            let now = UnixTime(1_794_744_180);
            let p = battery(Some(644.0), Relation::SameWaterBody, now.0 - 360);
            let day = LocalDate {
                year: 2026,
                month: 11,
                day: 8,
            };
            let wind = f.direction(Some(225.0));
            let block = format!(
                "[{name}]\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n",
                f.footnote(
                    &format!("Water {}", f.temperature(Some(12.2)).full()),
                    &p,
                    now
                ),
                f.station_footer(&p).text,
                f.discharge(Some(34.0)).full(),
                f.height(Some(Height {
                    datum: VerticalDatum::Mllw,
                    metres: 1.28
                }))
                .full(),
                f.wind_speed(Some(5.0)).full() + " from " + &wind,
                f.pressure(Some(1013.25)).full(),
                f.percentile(Some(PercentileClass::Normal), Some(45.0), false, day)
                    .unwrap_or_default(),
                f.updated(UnixTime(now.0 - 360), now),
                f.strings.get(Key::GoNoGoGood),
                f.strings
                    .fmt(Key::GoNoGoBad, &[("reason", "gusts to 25 kn")]),
                f.water_type_label(WaterType::TidalRiverEstuary),
            );
            out.push_str(&block);
        }
        insta::assert_snapshot!("labels_by_voice_and_units", out);
    }
}
