//! The trip view: a trip's days and places as rows of worded cells, each saying what it is based on.
//!
//! [`build_trip_view`] words a [`TripGrid`]: one [`TripDayRow`] per date, with one
//! [`TripPlaceDay`] per place in the trip's order, so a wide grid can show places as columns and a
//! narrow one can stack them. Every [`TripCell`] carries its basis in words: "certain",
//! "forecast, issued Nov 8, 6:00 AM", "typical for these dates, 1991–2020, station records" or
//! "unavailable: outside the forecast". A cell from past years always names its years and the
//! kind of record, and is never worded as a forecast; one whose kind cannot be found is shown as
//! unavailable instead.
//!
//! The verdict and windows reuse the go/no-go and windows cards' wording: storms, hurricanes,
//! warnings and lightning stay safety text, with their links. The checklist carries the official
//! regulation and licence links of each place's region with the disclaimer (links only, never
//! rules), the user's own items, the gear template of the trip's method, and the offline pack and
//! sharing state the app reports in [`TripAppState`]. Past, today and upcoming are computed here
//! from `now` and each place's own zone, so the display never does date arithmetic.

use std::collections::{BTreeMap, BTreeSet};

use jiff::tz::TimeZone;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::datum::Height;
use crate::model::{MoonDay, Polar, SolunarPeriod, SpotSnapshot, SunTimes, TideEvent, WaterType};
use crate::provenance::SourceId;
use crate::regulations::{self, RegKind, Water};
use crate::time::{self, LocalDate, UnixTime, UtcOffset, day_number};
use crate::trip::gonogo::{DayVerdict, Verdict};
use crate::trip::grid::{
    Basis, Cell, DayWindows, FlowReading, FlowStatus, GridProblem, GridVerdict, PlaceGrid,
    PlaceInputs, PlaceKey, TempStatus, TideDay, TripDay, TripGrid, WaterTempReading, Why,
};
use crate::trip::history::{HistoryKind, TripHistory};
use crate::trip::model::{Fuzz, Method, Trip};
use crate::view::cards::go_no_go::{GoNoGoReasonVm, day_vm};
use crate::view::cards::solunar::kind_label;
use crate::view::cards::sun_moon::{computed_credit, day_label};
use crate::view::cards::tide::credits as tide_credits;
use crate::view::cards::windows::{WindowVm, notices as window_notices, window_vm};
use crate::view::format::Formatter;
use crate::view::strings::{Key, SafetyKey};
use crate::view::{CardId, CreditVm, Notice, NoticeKind, ViewInput, Voice};

// ---------------------------------------------------------------------------------------------
// Input
// ---------------------------------------------------------------------------------------------

/// What the app knows about a trip besides its data; the view words it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TripAppState {
    pub offline: OfflinePack,
    pub sharing: Sharing,
}

/// The trip's offline pack.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum OfflinePack {
    #[default]
    NotDownloaded,
    /// Downloaded: its size and when it was saved.
    Ready { bytes: u64, saved_at: UnixTime },
}

/// Whether the trip is shared live.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Sharing {
    #[default]
    NotShared,
    Live {
        read_only: bool,
    },
}

// ---------------------------------------------------------------------------------------------
// Output
// ---------------------------------------------------------------------------------------------

/// A trip as shown: its places, its days and its checklist.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct TripView {
    /// The trip's name.
    pub title: String,
    /// "Nov 8 to Nov 10"
    pub dates: String,
    /// The first day, `YYYY-MM-DD`, for ordering trips.
    pub first: String,
    /// "Starts in 3 days", "Under way", "This trip is over".
    pub status: String,
    /// The last day is over at every place.
    pub is_past: bool,
    /// The first day has not begun at any place.
    pub is_upcoming: bool,
    /// The columns of the grid, in the trip's order.
    pub places: Vec<TripPlaceColumn>,
    /// One per date, first to last.
    pub days: Vec<TripDayRow>,
    pub checklist: Vec<ChecklistRow>,
    /// Every source a cell rests on.
    pub attribution: Vec<CreditVm>,
    /// Not for navigation, approximate tides, the traditional cue: safety text.
    pub notices: Vec<Notice>,
    /// The trip's notes, as the user wrote them.
    pub notes: String,
    /// The widget form.
    pub compact: TripCompact,
}

/// The "next trip: go/no-go" form for widgets.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct TripCompact {
    /// "Next trip: Hudson weekend"
    pub title: String,
    /// The trip's status: "Starts in 3 days".
    pub when: String,
    /// The worst verdict of the first day not over, across places; absent when no place has one.
    pub verdict: Option<Verdict>,
    /// "Go", "No-go", or "No verdict yet".
    pub headline: String,
    /// "Sun Nov 8 at The Battery: Luna approves."
    pub line: String,
    /// A storm ahead or a hurricane cone on that day.
    pub storm: bool,
    /// The plain meaning first.
    pub accessibility: String,
}

/// One place, the header of its column.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct TripPlaceColumn {
    /// The place's key in the grid: the host's spot id.
    pub key: String,
    pub name: String,
    pub area: Option<String>,
    pub water_type_label: String,
    /// "Approximate location" when the place was shared rounded or by area only.
    pub approximate: Option<String>,
    /// Why the place has no days.
    pub problem: Option<String>,
}

/// One date across the trip's places.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct TripDayRow {
    /// `YYYY-MM-DD`
    pub date: String,
    /// "Sun Nov 8"
    pub label: String,
    /// One per column, in the same order; absent where the place has no such day.
    pub places: Vec<Option<TripPlaceDay>>,
    /// The whole row as one sentence, the plain meaning first.
    pub accessibility: String,
}

/// One place on one date.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct TripPlaceDay {
    /// The column's key.
    pub place: String,
    /// The day is over in the place's zone.
    pub is_past: bool,
    /// The day is today in the place's zone.
    pub is_today: bool,
    /// For colouring; absent when the day has no verdict.
    pub verdict: Option<Verdict>,
    /// A storm ahead or a hurricane cone over the place on this day.
    pub storm: bool,
    /// One per [`TripCellKind`], in its order.
    pub cells: Vec<TripCell>,
    /// The verdict's reasons, storms first; safety reasons carry their links.
    pub reasons: Vec<GoNoGoReasonVm>,
    /// The day's fishing windows with their factors.
    pub windows: Vec<WindowVm>,
    /// The place's day as one sentence, the plain meaning first.
    pub accessibility: String,
}

/// What a cell shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TripCellKind {
    Verdict,
    Windows,
    Tides,
    Flow,
    WaterTemp,
    Sun,
    Moon,
    Solunar,
}

impl TripCellKind {
    /// Every kind, in the order of a day's cells.
    pub const ALL: [Self; 8] = [
        Self::Verdict,
        Self::Windows,
        Self::Tides,
        Self::Flow,
        Self::WaterTemp,
        Self::Sun,
        Self::Moon,
        Self::Solunar,
    ];
}

/// What a cell rests on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CellBasis {
    Certain,
    Forecast,
    /// Past years' values for these dates: history, never a forecast.
    Typical,
    Unavailable,
}

/// One measurement of one place on one day.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct TripCell {
    pub kind: TripCellKind,
    /// "Tide", "River flow", "Snack times".
    pub label: String,
    /// The short value for the grid: "Go", "706 cfs", "Low 1:59 AM"; "—" when unavailable.
    pub value: String,
    /// The details: every high and low, the range against the user's, past years' spread.
    pub lines: Vec<String>,
    pub basis: CellBasis,
    /// "certain", "forecast, issued Nov 8, 9:30 AM", "typical for these dates, 1991–2020,
    /// station records", "unavailable: outside the forecast".
    pub basis_label: String,
    /// The source's name, where there is one.
    pub source: Option<String>,
    /// Label, basis, value and lines; the plain meaning first.
    pub accessibility: String,
}

/// What a checklist row is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChecklistKind {
    /// An official licence page.
    Licence,
    /// An official regulations page.
    Regulations,
    /// The safety text beside the links.
    Disclaimer,
    /// One of the trip's own items.
    Item,
    /// The method's gear template.
    Gear,
    OfflinePack,
    Sharing,
}

/// One line of the checklist.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct ChecklistRow {
    pub kind: ChecklistKind,
    /// Stable across builds: the item's id, the link, or a fixed name.
    pub id: String,
    pub text: String,
    /// "NYSDEC, link checked 2026-11-10"
    pub detail: Option<String>,
    pub link: Option<String>,
    /// Ticked or not, for the trip's items and the offline pack; absent where it does not apply.
    pub done: Option<bool>,
}

// ---------------------------------------------------------------------------------------------
// Building
// ---------------------------------------------------------------------------------------------

/// Words the grid of `trip`.
///
/// `inputs` are the ones the grid was built from: the history there names the kind of record
/// behind each typical cell, and the snapshots the credits. Of `input` only the units, voice,
/// clock, locale and `now` are read, with its snapshot as the fallback for credits; each place
/// is worded in its own zone.
pub fn build_trip_view(
    trip: &Trip,
    grid: &TripGrid,
    inputs: &BTreeMap<PlaceKey, PlaceInputs>,
    app: &TripAppState,
    input: &ViewInput,
) -> TripView {
    let zone = trip.places.first().map_or("UTC", |p| p.time_zone.as_str());
    let f = formatter(input, zone, input.voice);

    let places: Vec<PlaceCtx> = trip
        .places
        .iter()
        .enumerate()
        .map(|(index, place)| {
            let key = PlaceKey::of(place);
            let grid = grid.places.get(&key).filter(|g| g.index == index);
            let zone = grid.map_or(place.time_zone.as_str(), |g| g.time_zone.as_str());
            let tz = time::zone(zone).ok();
            PlaceCtx {
                today: tz
                    .as_ref()
                    .and_then(|tz| time::local_date(input.now, tz).ok()),
                tz,
                inputs: inputs.get(&key),
                key,
                name: place.name.clone(),
                grid,
                fallback: &input.snapshot,
                f: formatter(input, zone, input.voice),
                plain: formatter(input, zone, Voice::Plain),
            }
        })
        .collect();

    let columns = trip
        .places
        .iter()
        .zip(&places)
        .map(|(place, c)| TripPlaceColumn {
            key: c.key.0.clone(),
            name: place.name.clone(),
            area: place.area.clone(),
            water_type_label: f.water_type_label(place.water_type),
            approximate: (place.fuzz != Fuzz::Exact)
                .then(|| f.strings.get(Key::TripApproximateLocation).to_owned()),
            problem: c.grid.and_then(|g| g.problem).map(|p| {
                f.strings
                    .get(match p {
                        GridProblem::Dates => Key::TripProblemDates,
                        GridProblem::TimeZone => Key::TripProblemTimeZone,
                    })
                    .to_owned()
            }),
        })
        .collect();

    let dates: BTreeSet<LocalDate> = places
        .iter()
        .filter_map(|c| c.grid)
        .flat_map(|g| g.days.iter().map(|d| d.date))
        .collect();
    let days: Vec<TripDayRow> = dates
        .iter()
        .map(|&date| {
            let cells: Vec<Option<TripPlaceDay>> = places
                .iter()
                .map(|c| {
                    let day = c.grid?.days.iter().find(|d| d.date == date)?;
                    Some(c.day(day))
                })
                .collect();
            let accessibility = sentences(cells.iter().flatten().map(|d| d.accessibility.as_str()));
            TripDayRow {
                date: iso(date),
                label: day_label(&f, date),
                places: cells,
                accessibility,
            }
        })
        .collect();

    let when = When::of(trip, &places, input.now);
    let status = when.text(&f);
    let compact = compact(trip, &f, &status, &days, &places);
    TripView {
        title: trip.name.clone(),
        dates: if trip.first == trip.last {
            f.date(trip.first)
        } else {
            f.strings.fmt(
                Key::Range,
                &[("from", &f.date(trip.first)), ("to", &f.date(trip.last))],
            )
        },
        first: iso(trip.first),
        status,
        is_past: when == When::Past,
        is_upcoming: matches!(when, When::Upcoming(_)),
        places: columns,
        checklist: checklist(trip, &f, app),
        attribution: attribution(&f, &places),
        notices: notices(&f, &places),
        notes: trip.notes.clone(),
        days,
        compact,
    }
}

/// The soonest trip that is not over, in its widget form.
pub fn next_trip(views: &[TripView]) -> Option<&TripCompact> {
    views
        .iter()
        .filter(|v| !v.is_past)
        .min_by(|a, b| a.first.cmp(&b.first))
        .map(|v| &v.compact)
}

fn formatter(input: &ViewInput, zone: &str, voice: Voice) -> Formatter {
    Formatter::new(input.units, voice, input.clock, zone).with_locale(input.locale)
}

/// `YYYY-MM-DD`.
fn iso(d: LocalDate) -> String {
    format!("{:04}-{:02}-{:02}", d.year, d.month, d.day)
}

/// Where the trip stands against `now`, from each place's own today.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum When {
    /// Days until the first day.
    Upcoming(i64),
    UnderWay,
    Past,
}

impl When {
    fn of(trip: &Trip, places: &[PlaceCtx], now: UnixTime) -> Self {
        let mut todays: Vec<i64> = places
            .iter()
            .filter_map(|c| c.today)
            .map(day_number)
            .collect();
        if todays.is_empty() {
            todays.push(day_number(now.to_local(UtcOffset(0)).date));
        }
        let (first, last) = (day_number(trip.first), day_number(trip.last));
        if todays.iter().all(|&t| t > last) {
            Self::Past
        } else if todays.iter().all(|&t| t < first) {
            Self::Upcoming(todays.iter().map(|&t| first - t).min().unwrap_or(1))
        } else {
            Self::UnderWay
        }
    }

    fn text(self, f: &Formatter) -> String {
        let s = f.strings;
        match self {
            Self::Past => s.get(Key::TripOver).to_owned(),
            Self::UnderWay => s.get(Key::TripUnderWay).to_owned(),
            Self::Upcoming(1) => s.get(Key::TripStartsTomorrow).to_owned(),
            Self::Upcoming(n) => s.fmt(Key::TripStartsInDays, &[("n", &n.to_string())]),
        }
    }
}

/// One place of the trip and what it is worded with.
struct PlaceCtx<'a> {
    key: PlaceKey,
    name: String,
    grid: Option<&'a PlaceGrid>,
    inputs: Option<&'a PlaceInputs>,
    /// The view input's snapshot, for credits when the place has none.
    fallback: &'a SpotSnapshot,
    /// The place's zone, if known.
    tz: Option<TimeZone>,
    /// The local date of `now` in the place's zone; absent for an unknown zone.
    today: Option<LocalDate>,
    /// In the user's voice and the place's zone.
    f: Formatter,
    /// The same in the plain voice, for accessibility.
    plain: Formatter,
}

/// What a cell's basis resolves to once the history behind it is known.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Resolved {
    Certain,
    Forecast(UnixTime),
    Typical {
        years: Option<(i32, i32)>,
        sampled: u32,
        kind: HistoryKind,
    },
    Unavailable(Why),
}

impl Resolved {
    /// A cell's basis; a typical cell without its kind of record is unavailable, and a cell that
    /// claims a basis without a value has no source.
    fn of<T>(cell: &Cell<T>, kind: Option<HistoryKind>) -> Self {
        match (&cell.basis, cell.value.is_some()) {
            (Basis::Unavailable { reason }, _) => Self::Unavailable(*reason),
            (_, false) => Self::Unavailable(Why::NoSource),
            (Basis::Certain, true) => Self::Certain,
            (Basis::Forecast { issued }, true) => Self::Forecast(*issued),
            (
                Basis::Typical {
                    years,
                    years_sampled,
                },
                true,
            ) => match kind {
                Some(kind) => Self::Typical {
                    years: years.map(|y| (y.first, y.last)),
                    sampled: *years_sampled,
                    kind,
                },
                None => Self::Unavailable(Why::NoHistory),
            },
        }
    }

    const fn kind(self) -> CellBasis {
        match self {
            Self::Certain => CellBasis::Certain,
            Self::Forecast(_) => CellBasis::Forecast,
            Self::Typical { .. } => CellBasis::Typical,
            Self::Unavailable(_) => CellBasis::Unavailable,
        }
    }

    fn label(self, f: &Formatter) -> String {
        let s = f.strings;
        match self {
            Self::Certain => s.get(Key::TripBasisCertain).to_owned(),
            Self::Forecast(t) => {
                let when = s.fmt(
                    Key::TripDateTime,
                    &[("date", &f.date_of(t)), ("time", &f.clock(t))],
                );
                s.fmt(Key::TripBasisForecast, &[("when", &when)])
            }
            Self::Typical {
                years,
                sampled,
                kind,
            } => {
                let kind = s.get(match kind {
                    HistoryKind::Station => Key::TripHistoryStation,
                    HistoryKind::Reanalysis => Key::TripHistoryReanalysis,
                    HistoryKind::Model => Key::TripHistoryModel,
                });
                match years {
                    Some((first, last)) => s.fmt(
                        Key::TripBasisTypical,
                        &[
                            ("first", &first.to_string()),
                            ("last", &last.to_string()),
                            ("kind", kind),
                        ],
                    ),
                    None => s.fmt(
                        Key::TripBasisTypicalCount,
                        &[("n", &sampled.to_string()), ("kind", kind)],
                    ),
                }
            }
            Self::Unavailable(why) => {
                let reason = s.get(match why {
                    Why::NoLocation => Key::TripWhyNoLocation,
                    Why::NoSource => Key::TripWhyNoSource,
                    Why::NoForecast => Key::TripWhyNoForecast,
                    Why::BeyondForecast => Key::TripWhyBeyondForecast,
                    Why::NoHistory => Key::TripWhyNoHistory,
                    Why::Past => Key::TripWhyPast,
                    Why::Disabled => Key::TripWhyDisabled,
                    Why::NoCoverage => Key::TripWhyNoCoverage,
                });
                s.fmt(Key::TripBasisUnavailable, &[("reason", reason)])
            }
        }
    }
}

/// A cell's value and lines in one voice.
struct Words {
    value: String,
    lines: Vec<String>,
}

fn label(f: &Formatter, kind: TripCellKind) -> &'static str {
    let s = f.strings;
    match kind {
        TripCellKind::Verdict => s.card_title(CardId::GoNoGo),
        TripCellKind::Windows => s.card_title(CardId::Windows),
        TripCellKind::Tides => s.get(Key::CardTide),
        TripCellKind::Flow => s.get(Key::CardFlow),
        TripCellKind::WaterTemp => s.get(Key::CardWaterTemp),
        TripCellKind::Sun => s.get(Key::Sun),
        TripCellKind::Moon => s.get(Key::Moon),
        TripCellKind::Solunar => s.get(Key::CardSolunar),
    }
}

/// Sentences joined with a full stop, without doubling one a sentence already ends with.
fn sentences<'a>(parts: impl IntoIterator<Item = &'a str>) -> String {
    let mut out = String::new();
    for part in parts.into_iter().filter(|p| !p.is_empty()) {
        if !out.is_empty() {
            out.push_str(if out.ends_with('.') { " " } else { ". " });
        }
        out.push_str(part);
    }
    out
}

/// What `VoiceOver` reads for a cell: label, basis, then the value and lines (the value once when
/// it heads the lines).
fn cell_accessibility(f: &Formatter, kind: TripCellKind, basis: Resolved, w: &Words) -> String {
    let s = f.strings;
    let label = label(f, kind);
    let basis = basis.label(f);
    if w.value == s.get(Key::Missing) {
        return s.fmt(Key::TripCellAccNone, &[("label", label), ("basis", &basis)]);
    }
    let (value, rest) = match w.lines.split_first() {
        Some((first, rest)) if *first == w.value => (first.as_str(), rest),
        _ => (w.value.as_str(), w.lines.as_slice()),
    };
    let head = s.fmt(
        Key::TripCellAcc,
        &[("label", label), ("basis", &basis), ("value", value)],
    );
    sentences(std::iter::once(head.as_str()).chain(rest.iter().map(String::as_str)))
}

impl PlaceCtx<'_> {
    fn snapshot(&self) -> Option<&SpotSnapshot> {
        self.inputs.and_then(|i| i.snapshot.as_ref())
    }

    fn history(&self) -> Option<&TripHistory> {
        self.inputs.and_then(|i| i.history.as_ref())
    }

    /// The name a cell's source is shown by: the weather provider's own, else the table's.
    fn source_name(&self, id: &SourceId) -> String {
        match self.snapshot().and_then(|s| s.weather.as_ref()) {
            Some(w) if w.provider.id() == id.as_str() => w.provider.display_name().to_owned(),
            _ => self.f.strings.source_name(id),
        }
    }

    /// A cell from a grid cell: `words` gives the value in a voice.
    fn cell<T>(
        &self,
        kind: TripCellKind,
        cell: &Cell<T>,
        history: Option<HistoryKind>,
        words: impl Fn(&Formatter, &T) -> Words,
    ) -> TripCell {
        let basis = Resolved::of(cell, history);
        let value = match basis {
            Resolved::Unavailable(_) => None,
            _ => cell.value.as_ref(),
        };
        let say = |f: &Formatter| {
            value.map_or_else(
                || Words {
                    value: f.strings.get(Key::Missing).to_owned(),
                    lines: Vec::new(),
                },
                |v| words(f, v),
            )
        };
        let (w, p) = (say(&self.f), say(&self.plain));
        // The plain meaning; the verdict adds its sentence in the user's voice, as the go/no-go
        // card does.
        let plain = cell_accessibility(&self.plain, kind, basis, &p);
        let accessibility = if kind == TripCellKind::Verdict && w.lines != p.lines {
            sentences(
                [plain.as_str()]
                    .into_iter()
                    .chain(w.lines.iter().map(String::as_str)),
            )
        } else {
            plain
        };
        TripCell {
            kind,
            label: label(&self.f, kind).to_owned(),
            value: w.value,
            lines: w.lines,
            basis: basis.kind(),
            basis_label: basis.label(&self.f),
            source: value
                .and(cell.source.as_ref())
                .map(|id| self.source_name(id)),
            accessibility,
        }
    }

    fn day(&self, day: &TripDay) -> TripPlaceDay {
        let today = self.today.map(day_number);
        let date = day_number(day.date);
        let flow_kind = self
            .history()
            .and_then(|h| h.days.get(&day.date))
            .and_then(|d| d.flow.as_ref())
            .map(|b| b.basis.kind);
        let temp_kind = self
            .history()
            .and_then(|h| h.days.get(&day.date))
            .and_then(|d| d.water_temp.as_ref().or(d.sst.as_ref()))
            .map(|b| b.basis.kind);
        let verdict = day.verdict.value.as_ref();
        // The day's own periods lead; one from the evening before is listed, not shown first.
        let day_start = self
            .tz
            .as_ref()
            .and_then(|tz| time::local_date_bounds(day.date, tz).ok())
            .map_or(UnixTime(i64::MIN), |(start, _)| start);
        let cells = vec![
            self.cell(TripCellKind::Verdict, &day.verdict, None, |f, v| {
                verdict_words(f, day.date, v).0
            }),
            self.cell(TripCellKind::Windows, &day.windows, None, windows_words),
            self.cell(TripCellKind::Tides, &day.tides, None, tide_words),
            self.cell(TripCellKind::Flow, &day.flow, flow_kind, flow_words),
            self.cell(
                TripCellKind::WaterTemp,
                &day.water_temp,
                temp_kind,
                temp_words,
            ),
            self.cell(TripCellKind::Sun, &day.sun, None, sun_words),
            self.cell(TripCellKind::Moon, &day.moon, None, moon_words),
            self.cell(TripCellKind::Solunar, &day.solunar, None, |f, periods| {
                solunar_words(f, periods, day_start)
            }),
        ];
        let sentence: Vec<&str> = cells
            .iter()
            .map(|c| c.accessibility.trim_end_matches('.'))
            .collect();
        let accessibility = self.plain.strings.fmt(
            Key::TripPlaceDayAcc,
            &[
                ("place", &self.name),
                ("day", &day_label(&self.plain, day.date)),
                ("cells", &sentence.join("; ")),
            ],
        );
        TripPlaceDay {
            place: self.key.0.clone(),
            is_past: today.is_some_and(|t| t > date),
            is_today: today == Some(date),
            verdict: verdict.map(|v| v.verdict),
            storm: verdict.is_some_and(|v| !v.storm.is_empty()),
            cells,
            reasons: verdict.map_or_else(Vec::new, |v| verdict_words(&self.f, day.date, v).1),
            windows: day.windows.value.as_ref().map_or_else(Vec::new, |w| {
                w.windows.iter().map(|w| window_vm(&self.f, w)).collect()
            }),
            accessibility,
        }
    }
}

fn sun_words(f: &Formatter, sun: &SunTimes) -> Words {
    let s = f.strings;
    Words {
        value: match (sun.polar, sun.sunrise, sun.sunset) {
            (Some(Polar::AlwaysUp), ..) => s.get(Key::SunStaysUp).to_owned(),
            (Some(Polar::AlwaysDown), ..) => s.get(Key::SunStaysDown).to_owned(),
            (None, Some(rise), Some(set)) => f.clock_range(rise, set),
            _ => s.get(Key::NoSunriseOrSunset).to_owned(),
        },
        lines: Vec::new(),
    }
}

fn moon_words(f: &Formatter, moon: &MoonDay) -> Words {
    let lit = f.percent(Some(moon.illuminated_fraction * 100.0)).full();
    Words {
        value: f.strings.moon_phase(moon.phase).to_owned(),
        lines: vec![f.strings.fmt(Key::TripMoonLit, &[("percent", &lit)])],
    }
}

/// Every period in time order; the first that starts on the day itself leads.
fn solunar_words(f: &Formatter, periods: &[SolunarPeriod], day_start: UnixTime) -> Words {
    let mut periods = periods.to_vec();
    periods.sort_by_key(|p| p.start);
    let lines: Vec<String> = periods
        .iter()
        .map(|p| {
            f.strings.fmt(
                Key::TripSolunarPeriod,
                &[
                    ("label", kind_label(f, p.kind)),
                    ("range", &f.clock_range(p.start, p.end)),
                ],
            )
        })
        .collect();
    let lead = periods
        .iter()
        .position(|p| p.start >= day_start)
        .unwrap_or(0);
    Words {
        value: lines
            .get(lead)
            .cloned()
            .unwrap_or_else(|| f.strings.get(Key::Missing).to_owned()),
        lines,
    }
}

/// The verdict as the go/no-go card words it: label, the day's sentence, and the reasons with
/// storms first.
fn verdict_words(f: &Formatter, date: LocalDate, v: &GridVerdict) -> (Words, Vec<GoNoGoReasonVm>) {
    let day = DayVerdict {
        day: date,
        verdict: v.verdict,
        reasons: v.storm.iter().chain(&v.reasons).cloned().collect(),
        hours: Vec::new(),
    };
    let vm = day_vm(f, &day);
    let lines = if vm.summary.is_empty() {
        Vec::new()
    } else {
        vec![vm.summary]
    };
    (
        Words {
            value: vm.verdict_label,
            lines,
        },
        vm.reasons,
    )
}

fn windows_words(f: &Formatter, w: &DayWindows) -> Words {
    let s = f.strings;
    let vms: Vec<WindowVm> = w.windows.iter().map(|w| window_vm(f, w)).collect();
    let mut lines = Vec::new();
    if let Some(first) = vms.first() {
        let names: Vec<&str> = first.factors.iter().map(|x| x.label.as_str()).collect();
        if !names.is_empty() {
            lines.push(names.join(", "));
        }
    }
    if vms.len() > 1 {
        lines.push(s.fmt(Key::TripMoreWindows, &[("n", &(vms.len() - 1).to_string())]));
    }
    if w.certain_only {
        lines.push(s.get(Key::TripWindowsCertainOnly).to_owned());
    }
    Words {
        value: vms
            .first()
            .map_or_else(|| s.get(Key::NoWindows).to_owned(), |w| w.range.clone()),
        lines,
    }
}

fn tide_event(f: &Formatter, t: &TideDay, e: &TideEvent) -> String {
    let s = f.strings;
    let kind = s.tide_kind(e.kind);
    let time = f.clock(e.time);
    match (t.datum, e.height_m) {
        (Some(datum), Some(metres)) => s.fmt(
            Key::TripTideEventHeight,
            &[
                ("kind", kind),
                ("time", &time),
                ("height", &f.height(Some(Height { datum, metres })).full()),
            ],
        ),
        _ => s.fmt(Key::TripTideEvent, &[("kind", kind), ("time", &time)]),
    }
}

fn tide_words(f: &Formatter, t: &TideDay) -> Words {
    let lines: Vec<String> = t.events.iter().map(|e| tide_event(f, t, e)).collect();
    Words {
        value: lines
            .first()
            .cloned()
            .unwrap_or_else(|| f.strings.get(Key::Missing).to_owned()),
        lines,
    }
}

fn flow_words(f: &Formatter, r: &FlowReading) -> Words {
    let s = f.strings;
    let mut lines = Vec::new();
    let status = match r.status {
        FlowStatus::BlownOut => Some(Key::TripFlowBlownOut),
        FlowStatus::AboveBest => Some(Key::TripFlowAboveBest),
        FlowStatus::InBest => Some(Key::TripFlowInBest),
        FlowStatus::BelowBest => Some(Key::TripFlowBelowBest),
        FlowStatus::Unclassified => None,
    };
    lines.extend(status.map(|k| s.get(k).to_owned()));
    lines.extend(r.wadeable.map(|w| {
        s.get(if w {
            Key::TripWadeable
        } else {
            Key::TripNotWadeable
        })
        .to_owned()
    }));
    if let (Some(lo), Some(hi)) = (r.p10_m3s, r.p90_m3s) {
        lines.push(s.fmt(
            Key::TripMostYears,
            &[
                ("low", &f.discharge(Some(lo)).full()),
                ("high", &f.discharge(Some(hi)).full()),
            ],
        ));
    }
    Words {
        value: f.discharge(Some(r.m3s)).full(),
        lines,
    }
}

fn temp_words(f: &Formatter, r: &WaterTempReading) -> Words {
    let s = f.strings;
    let mut lines = Vec::new();
    lines.extend(r.status.map(|t| {
        s.get(match t {
            TempStatus::Below => Key::TripTempBelow,
            TempStatus::Within => Key::TripTempWithin,
            TempStatus::Above => Key::TripTempAbove,
        })
        .to_owned()
    }));
    if let (Some(lo), Some(hi)) = (r.p10_c, r.p90_c) {
        lines.push(s.fmt(
            Key::TripMostYears,
            &[
                ("low", &f.temperature(Some(lo)).full()),
                ("high", &f.temperature(Some(hi)).full()),
            ],
        ));
    }
    Words {
        value: f.temperature(Some(r.celsius)).full(),
        lines,
    }
}

/// Worst first: no-go, marginal, go, unknown.
const fn severity(v: Verdict) -> u8 {
    match v {
        Verdict::NoGo => 3,
        Verdict::Marginal => 2,
        Verdict::Go => 1,
        Verdict::Unknown => 0,
    }
}

/// The widget form: the first day not over, the worst verdict across places.
fn compact(
    trip: &Trip,
    f: &Formatter,
    when: &str,
    days: &[TripDayRow],
    places: &[PlaceCtx],
) -> TripCompact {
    let title = f.strings.fmt(Key::TripNext, &[("name", &trip.name)]);
    // The first day not over somewhere, at the place with the worst verdict (else the first).
    let pick = days.iter().find_map(|row| {
        row.places
            .iter()
            .enumerate()
            .filter_map(|(i, d)| d.as_ref().filter(|d| !d.is_past).map(|d| (i, d)))
            .max_by_key(|(i, d)| (d.verdict.map_or(0, severity), std::cmp::Reverse(*i)))
            .map(|(i, d)| (row, i, d))
    });
    let grid_day = pick.and_then(|(row, i, _)| {
        places[i]
            .grid?
            .days
            .iter()
            .find(|d| iso(d.date) == row.date)
    });
    let (Some((_, index, day)), Some(grid_day)) = (pick, grid_day) else {
        let over = f.strings.get(Key::TripOver).to_owned();
        return TripCompact {
            accessibility: sentences([title.as_str(), when]),
            title,
            when: when.to_owned(),
            verdict: None,
            headline: over.clone(),
            line: over,
            storm: false,
        };
    };
    let place = &places[index];
    // The headline and the line in one voice, worded in the place's zone.
    let say = |w: &Formatter| {
        let (headline, summary) = match &grid_day.verdict.value {
            Some(v) => {
                let (words, _) = verdict_words(w, grid_day.date, v);
                let summary = words.lines.first().cloned().unwrap_or_default();
                (words.value, summary)
            }
            None => (
                w.strings.get(Key::TripNoVerdict).to_owned(),
                Resolved::of(&grid_day.verdict, None).label(w),
            ),
        };
        let line = w.strings.fmt(
            Key::TripCompactLine,
            &[
                ("day", &day_label(w, grid_day.date)),
                ("place", &place.name),
                ("summary", &summary),
            ],
        );
        (headline, line)
    };
    let (headline, line) = say(&place.f);
    let (plain_headline, plain_line) = say(&place.plain);
    // The plain meaning first, then what the user's voice says differently.
    let mut parts = vec![
        title.as_str(),
        when,
        plain_headline.as_str(),
        plain_line.as_str(),
    ];
    if headline != plain_headline {
        parts.push(&headline);
    }
    if line != plain_line {
        parts.push(&line);
    }
    let accessibility = sentences(parts);
    TripCompact {
        accessibility,
        title,
        when: when.to_owned(),
        verdict: day.verdict,
        headline,
        line,
        storm: day.storm,
    }
}

const fn water_of(t: WaterType) -> Water {
    match t {
        WaterType::RiverStream
        | WaterType::LakeReservoir
        | WaterType::GreatLakes
        | WaterType::Pond
        | WaterType::Ice => Water::Fresh,
        WaterType::TidalRiverEstuary
        | WaterType::BayInshore
        | WaterType::Surf
        | WaterType::Nearshore
        | WaterType::Offshore => Water::Salt,
    }
}

/// The gear template of a method: a stable id and its words.
const fn gear(method: Method) -> &'static [(&'static str, Key)] {
    match method {
        Method::Boat => &[
            ("life-jacket", Key::GearPfd),
            ("vhf", Key::GearVhf),
            ("anchor", Key::GearAnchor),
            ("light", Key::GearLight),
            ("tackle", Key::GearRodsTackle),
            ("pliers", Key::GearPliers),
            ("snacks", Key::GearSnacks),
        ],
        Method::Kayak => &[
            ("life-jacket", Key::GearPfd),
            ("whistle", Key::GearWhistle),
            ("paddle", Key::GearPaddleLeash),
            ("dry-bag", Key::GearDryBag),
            ("light", Key::GearLight),
            ("tackle", Key::GearRodsTackle),
            ("pliers", Key::GearPliers),
            ("snacks", Key::GearSnacks),
        ],
        Method::Shore => &[
            ("tackle", Key::GearRodsTackle),
            ("pliers", Key::GearPliers),
            ("light", Key::GearLight),
            ("rain-jacket", Key::GearRainJacket),
            ("snacks", Key::GearSnacks),
        ],
        Method::Wade => &[
            ("waders", Key::GearWaders),
            ("wading-boots", Key::GearWadingBoots),
            ("wading-staff", Key::GearWadingStaff),
            ("tackle", Key::GearRodsTackle),
            ("pliers", Key::GearPliers),
            ("snacks", Key::GearSnacks),
        ],
        Method::Ice => &[
            ("spud", Key::GearSpud),
            ("ice-picks", Key::GearIcePicks),
            ("ice-cleats", Key::GearIceCleats),
            ("auger", Key::GearAuger),
            ("tackle", Key::GearRodsTackle),
            ("snacks", Key::GearSnacks),
        ],
    }
}

/// Links, the disclaimer, the trip's items, the gear template, the offline pack and sharing.
fn checklist(trip: &Trip, f: &Formatter, app: &TripAppState) -> Vec<ChecklistRow> {
    let s = f.strings;
    let mut rows: Vec<ChecklistRow> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for place in &trip.places {
        let Some(region) = &place.region else {
            continue;
        };
        for link in regulations::links_in(region, water_of(place.water_type), "en") {
            if !seen.insert(link.url.clone()) {
                continue;
            }
            let (kind, title) = match link.kind {
                RegKind::Licence => (ChecklistKind::Licence, SafetyKey::RegLicence),
                RegKind::Freshwater => (ChecklistKind::Regulations, SafetyKey::RegFreshwater),
                RegKind::Saltwater => (ChecklistKind::Regulations, SafetyKey::RegSaltwater),
                RegKind::Federal => (ChecklistKind::Regulations, SafetyKey::RegFederal),
            };
            rows.push(ChecklistRow {
                kind,
                id: link.url.clone(),
                text: s.safety(title).to_owned(),
                detail: Some(s.safety_fmt(
                    if link.verified {
                        SafetyKey::RegChecked
                    } else {
                        SafetyKey::RegUnconfirmed
                    },
                    &[("agency", &link.label), ("date", &link.checked)],
                )),
                link: Some(link.url),
                done: None,
            });
        }
    }
    if !rows.is_empty() {
        rows.push(ChecklistRow {
            kind: ChecklistKind::Disclaimer,
            id: "regulations-disclaimer".to_owned(),
            text: s.safety(SafetyKey::RegulationsDisclaimer).to_owned(),
            detail: None,
            link: None,
            done: None,
        });
    }
    rows.extend(trip.checklist.iter().map(|item| ChecklistRow {
        kind: ChecklistKind::Item,
        id: item.id.clone(),
        text: item.text.clone(),
        detail: None,
        link: None,
        done: Some(item.done),
    }));
    rows.extend(gear(trip.method).iter().map(|(id, key)| ChecklistRow {
        kind: ChecklistKind::Gear,
        id: format!("gear-{id}"),
        text: s.get(*key).to_owned(),
        detail: None,
        link: None,
        done: None,
    }));
    rows.push(ChecklistRow {
        kind: ChecklistKind::OfflinePack,
        id: "offline-pack".to_owned(),
        text: match app.offline {
            OfflinePack::NotDownloaded => s.get(Key::TripOfflineNone).to_owned(),
            OfflinePack::Ready { bytes, saved_at } => s.fmt(
                Key::TripOfflineReady,
                &[
                    ("size", &s.pack_size(bytes)),
                    ("date", &f.date_of(saved_at)),
                ],
            ),
        },
        detail: None,
        link: None,
        done: Some(matches!(app.offline, OfflinePack::Ready { .. })),
    });
    rows.push(ChecklistRow {
        kind: ChecklistKind::Sharing,
        id: "sharing".to_owned(),
        text: s
            .get(match app.sharing {
                Sharing::NotShared => Key::TripNotShared,
                Sharing::Live { read_only: true } => Key::TripSharedReadOnly,
                Sharing::Live { read_only: false } => Key::TripSharedEditable,
            })
            .to_owned(),
        detail: None,
        link: None,
        done: None,
    });
    rows
}

/// The credit a source is owed, from the place's snapshot where it has one.
fn credits_of(f: &Formatter, snapshot: &SpotSnapshot, id: &SourceId) -> Vec<CreditVm> {
    let s = f.strings;
    if let Some(w) = &snapshot.weather
        && w.provider.id() == id.as_str()
    {
        return vec![CreditVm {
            source: w.provider.display_name().to_owned(),
            text: w.provider.attribution().to_owned(),
            link: None,
            licence: None,
        }];
    }
    let fixed = |source: SafetyKey, text: SafetyKey| {
        vec![CreditVm {
            source: s.safety(source).to_owned(),
            text: s.safety(text).to_owned(),
            link: None,
            licence: None,
        }]
    };
    match id.as_str() {
        "nhc" => fixed(SafetyKey::SourceNhc, SafetyKey::CreditNhc),
        "open-meteo" => fixed(SafetyKey::SourceOpenMeteo, SafetyKey::CreditOpenMeteoModels),
        _ if snapshot.attribution.iter().any(|a| a.source == *id) => tide_credits(f, snapshot, id),
        "usgs" => vec![CreditVm {
            source: s.source_name(id),
            text: s.safety(SafetyKey::CreditUsgs).to_owned(),
            link: None,
            licence: Some(s.safety(SafetyKey::LicencePublicDomain).to_owned()),
        }],
        "open-meteo-marine" => vec![CreditVm {
            source: s.source_name(id),
            text: s.safety(SafetyKey::CreditOpenMeteo).to_owned(),
            link: None,
            licence: Some(s.safety(SafetyKey::LicenceCcBy4).to_owned()),
        }],
        _ => tide_credits(f, snapshot, id),
    }
}

/// Every source the cells rest on, the device's own computation first.
fn attribution(f: &Formatter, places: &[PlaceCtx]) -> Vec<CreditVm> {
    let mut out: Vec<CreditVm> = Vec::new();
    let mut push = |c: CreditVm| {
        if !out.contains(&c) {
            out.push(c);
        }
    };
    let days = || {
        places.iter().flat_map(|c| {
            c.grid
                .into_iter()
                .flat_map(move |g| g.days.iter().map(move |d| (c, d)))
        })
    };
    let computed = days().any(|(_, d)| {
        d.sun.value.is_some()
            || d.moon.value.is_some()
            || d.solunar.value.is_some()
            || d.windows.value.is_some()
    });
    if computed {
        push(computed_credit(f.strings));
    }
    for (c, d) in days() {
        let snapshot = c.snapshot().unwrap_or(c.fallback);
        let mut ids: Vec<&SourceId> = [
            d.verdict.source.as_ref(),
            d.tides.source.as_ref(),
            d.flow.source.as_ref(),
            d.water_temp.source.as_ref(),
        ]
        .into_iter()
        .flatten()
        .collect();
        if let Some(v) = &d.verdict.value {
            ids.extend(
                v.storm
                    .iter()
                    .chain(&v.reasons)
                    .filter_map(|r| r.source.as_ref()),
            );
        }
        for id in ids {
            credits_of(f, snapshot, id).into_iter().for_each(&mut push);
        }
        // Gridded history from Open-Meteo is its model providers' data.
        if matches!(d.water_temp.basis, Basis::Typical { .. })
            && d.water_temp.source == Some(SourceId::OPEN_METEO_MARINE)
        {
            push(CreditVm {
                source: f.strings.safety(SafetyKey::SourceModelProviders).to_owned(),
                text: f
                    .strings
                    .safety(SafetyKey::AttrOpenMeteoMarineModels)
                    .to_owned(),
                link: None,
                licence: None,
            });
        }
    }
    out
}

/// The fixed notices the cells call for: never for navigation where tides are shown, the
/// model's or the subordinate station's caveat, and the traditional cue for solunar periods.
fn notices(f: &Formatter, places: &[PlaceCtx]) -> Vec<Notice> {
    use crate::model::TideSourceKind;
    let mut out: Vec<Notice> = Vec::new();
    let mut push = |n: Notice| {
        if !out.contains(&n) {
            out.push(n);
        }
    };
    let days: Vec<&TripDay> = places
        .iter()
        .filter_map(|c| c.grid)
        .flat_map(|g| g.days.iter())
        .collect();
    let tides: Vec<&TideDay> = days.iter().filter_map(|d| d.tides.value.as_ref()).collect();
    if !tides.is_empty() {
        push(f.notice(NoticeKind::NotForNavigation));
    }
    if tides.iter().any(|t| t.kind == TideSourceKind::ModelMean) {
        push(f.sea_level_notice());
    }
    if tides
        .iter()
        .any(|t| t.kind == TideSourceKind::SubordinateStation)
    {
        push(f.notice(NoticeKind::Approximate));
    }
    let windows: Vec<_> = days
        .iter()
        .filter_map(|d| d.windows.value.as_ref())
        .flat_map(|w| w.windows.iter().cloned())
        .collect();
    window_notices(f, &windows).into_iter().for_each(&mut push);
    if days
        .iter()
        .any(|d| d.solunar.value.as_ref().is_some_and(|p| !p.is_empty()))
    {
        push(f.notice(NoticeKind::TraditionalCue));
    }
    out
}

#[cfg(test)]
pub(in crate::view) mod tests {
    use std::fmt::Write as _;

    use serde_json::Value;
    use whether_core::builder::SnapshotBuilder;
    use whether_core::condition::Condition;
    use whether_core::history::YearRange;
    use whether_core::model::{Hourly, ProviderId, WeatherSnapshot};
    use whether_core::units::{Celsius, Meters, Mps};

    use super::*;
    use crate::astro::tide::{HarmonicConstituent, TideHarmonics};
    use crate::datum::{DatumTable, VerticalDatum};
    use crate::provenance::{Provenance, Quality, Relation, StationKind, StationRef, ValueKind};
    use crate::time::{self, local_date_bounds, zone};
    use crate::trip::gonogo::{Check, Reason, ReasonKind};
    use crate::trip::grid::{GridConfig, TideConstants, build};
    use crate::trip::history::{Band, Coverage, HistoryBasis, TypicalDay};
    use crate::trip::model::{ChecklistItem, TripLimits, TripPlace};
    use crate::view::cards::flow::fixtures::{croton_input, metric};

    const HOUR: i64 = 3_600;

    fn date(year: i32, month: u8, day: u8) -> LocalDate {
        LocalDate { year, month, day }
    }

    fn ny_start(d: LocalDate) -> UnixTime {
        local_date_bounds(d, &zone("America/New_York").unwrap())
            .unwrap()
            .0
    }

    /// Hourly weather from `from`: light wind, with gusts over the kayak limit in `gusty` hours.
    fn weather(from: UnixTime, hours: i64, gusty: std::ops::Range<i64>) -> WeatherSnapshot {
        let mut b = SnapshotBuilder::new(ProviderId::OpenMeteo);
        b.set_location(whether_core::testdata::sample_location());
        b.set_hourly(
            (0..hours)
                .map(|i| Hourly {
                    time: UnixTime(from.0 + i * HOUR),
                    condition: Condition::PartlyCloudy,
                    temperature: Some(Celsius(12.0)),
                    wind_speed: Some(Mps(3.0)),
                    wind_gust: Some(Mps(if gusty.contains(&i) { 11.0 } else { 4.5 })),
                    visibility: Some(Meters(20_000.0)),
                    ..Hourly::default()
                })
                .collect(),
        );
        b.finish(from).unwrap()
    }

    fn band(median: f32, source: SourceId, kind: HistoryKind, years: Option<YearRange>) -> Band {
        Band {
            p10: median * 0.8,
            median,
            p90: median * 1.25,
            basis: HistoryBasis {
                source,
                kind,
                years,
                years_sampled: years.map_or(93, |y| u32::try_from(y.last - y.first + 1).unwrap()),
                percentiles: None,
            },
        }
    }

    fn dates(first: LocalDate, n: i64) -> Vec<LocalDate> {
        (0..n)
            .map(|i| {
                let (year, month, day) =
                    whether_core::time::civil_from_days(time::day_number(first) + i);
                date(year, month, day)
            })
            .collect()
    }

    /// Sea surface temperature from the reanalysis for `n` days from `first`.
    fn battery_history(first: LocalDate, n: i64) -> TripHistory {
        let years = Some(YearRange {
            first: 2016,
            last: 2025,
        });
        TripHistory {
            days: dates(first, n)
                .into_iter()
                .map(|d| {
                    let sst = band(
                        11.5,
                        SourceId::OPEN_METEO_MARINE,
                        HistoryKind::Reanalysis,
                        years,
                    );
                    (
                        d,
                        TypicalDay {
                            sst: Some(sst),
                            ..TypicalDay::default()
                        },
                    )
                })
                .collect(),
            sst: Coverage::Available,
            ..TripHistory::default()
        }
    }

    /// Flow from the Statistics API (a count of years only) and water temperature from the gauge.
    fn croton_history(first: LocalDate, n: i64) -> TripHistory {
        TripHistory {
            days: dates(first, n)
                .into_iter()
                .map(|d| {
                    (
                        d,
                        TypicalDay {
                            flow: Some(band(0.42, SourceId::USGS, HistoryKind::Station, None)),
                            water_temp: Some(band(
                                9.0,
                                SourceId::USGS,
                                HistoryKind::Station,
                                Some(YearRange {
                                    first: 1991,
                                    last: 2020,
                                }),
                            )),
                            ..TypicalDay::default()
                        },
                    )
                })
                .collect(),
            flow: Coverage::Available,
            water_temp: Coverage::Available,
            ..TripHistory::default()
        }
    }

    fn coops(name: &str) -> Value {
        let path = format!(
            "{}/../../tests/fixtures/raw/coops/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    /// The Battery's harmonic constants and datums, as recorded.
    fn battery_constants() -> TideConstants {
        let harcon = coops("coops-mdapi-8518750-harcon-metric.json");
        let constituents = harcon["HarmonicConstituents"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| HarmonicConstituent {
                name: c["name"].as_str().unwrap().to_owned(),
                amplitude_m: c["amplitude"].as_f64().unwrap() as f32,
                phase_gmt_deg: c["phase_GMT"].as_f64().unwrap() as f32,
            })
            .collect();
        let datums = coops("coops-mdapi-8518750-datums-metric.json");
        let values: BTreeMap<String, f32> = datums["datums"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|d| Some((d["name"].as_str()?.to_owned(), d["value"].as_f64()? as f32)))
            .collect();
        let harmonics = TideHarmonics::from_datum_table(
            "8518750",
            constituents,
            &DatumTable::new("8518750", values),
            VerticalDatum::Mllw,
        )
        .unwrap();
        let station = StationRef {
            source: SourceId::COOPS,
            id: "8518750".into(),
            name: "The Battery".into(),
            kind: StationKind::TideHarmonic,
            latitude: 40.7006,
            longitude: -74.0142,
            bin: None,
            reference_id: None,
        };
        TideConstants::Harmonic {
            harmonics,
            base: Provenance {
                source: SourceId::COOPS,
                station: Some(station),
                distance_m: None,
                relation: Relation::SameWaterBody,
                observed_at: UnixTime(0),
                kind: ValueKind::Predicted,
                quality: Quality::Approved,
            },
        }
    }

    fn battery() -> TripPlace {
        crate::trip::model::tests::battery()
    }

    fn croton() -> TripPlace {
        TripPlace {
            spot_id: "spot-croton".into(),
            name: "Croton River".into(),
            area: Some("Croton".into()),
            latitude: Some(41.22),
            longitude: Some(-73.86),
            time_zone: "America/New_York".into(),
            water_type: WaterType::RiverStream,
            region: Some("US-NY".into()),
            fuzz: Fuzz::Rounded1Km,
        }
    }

    fn trip(places: Vec<TripPlace>, first: LocalDate, last: LocalDate) -> Trip {
        Trip {
            places,
            first,
            last,
            limits: None,
            checklist: vec![ChecklistItem {
                id: "c1".into(),
                text: "Bucktails".into(),
                done: true,
            }],
            ..crate::trip::model::tests::trip()
        }
    }

    /// The view input: US units, Luna, the 12-hour clock, the kayak limits, at 8 am on Nov 8.
    pub(in crate::view) fn view_input() -> ViewInput {
        let mut input = crate::view::timeline::tests::input();
        input.trip_limits = Some(TripLimits {
            name: "Kayak".into(),
            max_wind_kn: Some(12.0),
            max_gust_kn: Some(18.0),
            ..TripLimits::default()
        });
        input
    }

    /// The Battery with the recorded week of predictions, two days of weather (gusty on the
    /// first afternoon) and the reanalysis' sea temperature; `constants` adds the harmonics.
    fn battery_inputs(input: &ViewInput, first: LocalDate, constants: bool) -> PlaceInputs {
        let mut snapshot = input.snapshot.clone();
        let start = ny_start(date(2026, 11, 8));
        snapshot.fetched_at = UnixTime(input.now.0 - 1_800);
        snapshot.weather = Some(weather(start, 48, 13..17));
        PlaceInputs {
            snapshot: Some(snapshot),
            tide_constants: constants.then(battery_constants),
            history: Some(battery_history(first, 3)),
            species_water_temp_c: Some([13.0, 22.0]),
            ..PlaceInputs::default()
        }
    }

    fn croton_inputs(input: &ViewInput, first: LocalDate) -> PlaceInputs {
        let mut snapshot = croton_input(input.units, input.voice).snapshot;
        snapshot.fetched_at = UnixTime(input.now.0 - 1_800);
        snapshot.weather = Some(weather(ny_start(date(2026, 11, 8)), 48, 0..0));
        PlaceInputs {
            snapshot: Some(snapshot),
            history: Some(croton_history(first, 3)),
            river: crate::view::RiverRanges {
                wadeable_below_m3s: Some(0.6),
                best_min_m3s: Some(0.3),
                best_max_m3s: Some(1.5),
                blown_out_above_m3s: Some(8.0),
            },
            species_water_temp_c: Some([10.0, 18.0]),
            ..PlaceInputs::default()
        }
    }

    /// A trip, its grid and the inputs the grid was built from.
    pub(in crate::view) struct Built {
        pub trip: Trip,
        pub grid: TripGrid,
        pub inputs: BTreeMap<PlaceKey, PlaceInputs>,
    }

    impl Built {
        /// The view with a downloaded pack and a read-only live share.
        pub(in crate::view) fn view(&self, input: &ViewInput) -> TripView {
            let app = TripAppState {
                offline: OfflinePack::Ready {
                    bytes: 48_260_000,
                    saved_at: UnixTime(input.now.0 - 20 * HOUR),
                },
                sharing: Sharing::Live { read_only: true },
            };
            build_trip_view(&self.trip, &self.grid, &self.inputs, &app, input)
        }
    }

    fn built(trip: Trip, inputs: BTreeMap<PlaceKey, PlaceInputs>, input: &ViewInput) -> Built {
        let cfg = GridConfig {
            windows: input.windows,
            limits: input.trip_limits.clone(),
        };
        let grid = build(&trip, &inputs, &cfg, input.now);
        Built { trip, grid, inputs }
    }

    /// Three days at the Battery from Nov 8: forecast, forecast, past its end.
    pub(in crate::view) fn battery_three_days(input: &ViewInput) -> Built {
        let first = date(2026, 11, 8);
        let t = trip(vec![battery()], first, date(2026, 11, 10));
        let inputs = BTreeMap::from([(
            PlaceKey::of(&t.places[0]),
            battery_inputs(input, first, false),
        )]);
        built(t, inputs, input)
    }

    /// Three days in January, past every forecast: harmonic tides, history and the sky.
    pub(in crate::view) fn beyond_the_forecast(input: &ViewInput) -> Built {
        let first = date(2027, 1, 7);
        let t = trip(vec![battery()], first, date(2027, 1, 9));
        let inputs = BTreeMap::from([(
            PlaceKey::of(&t.places[0]),
            battery_inputs(input, first, true),
        )]);
        built(t, inputs, input)
    }

    /// The Battery and the Croton River, Nov 8 to 10, wading.
    pub(in crate::view) fn two_places(input: &ViewInput) -> Built {
        let first = date(2026, 11, 8);
        let mut t = trip(vec![battery(), croton()], first, date(2026, 11, 10));
        t.method = Method::Wade;
        let mut inputs = BTreeMap::new();
        inputs.insert(
            PlaceKey::of(&t.places[0]),
            battery_inputs(input, first, true),
        );
        inputs.insert(PlaceKey::of(&t.places[1]), croton_inputs(input, first));
        built(t, inputs, input)
    }

    /// The view as reviewable text: every cell with its basis.
    fn render(v: &TripView) -> String {
        let mut out = String::new();
        let _ = writeln!(
            out,
            "{} | {} | {} | past {} upcoming {}",
            v.title, v.dates, v.status, v.is_past, v.is_upcoming
        );
        for p in &v.places {
            let _ = writeln!(
                out,
                "column {} | {} | {:?} | {} | {:?} | {:?}",
                p.key, p.name, p.area, p.water_type_label, p.approximate, p.problem
            );
        }
        for row in &v.days {
            let _ = writeln!(out, "{} {}", row.date, row.label);
            for day in row.places.iter().flatten() {
                let _ = writeln!(
                    out,
                    "  {} past {} today {} verdict {:?} storm {}",
                    day.place, day.is_past, day.is_today, day.verdict, day.storm
                );
                for c in &day.cells {
                    let source = c
                        .source
                        .as_ref()
                        .map_or_else(String::new, |s| format!(" ({s})"));
                    let _ = writeln!(
                        out,
                        "    {} = {} [{}]{source}",
                        c.label, c.value, c.basis_label
                    );
                    for line in &c.lines {
                        let _ = writeln!(out, "      {line}");
                    }
                }
                for r in &day.reasons {
                    let _ = writeln!(out, "    reason {:?} {:?}: {}", r.check, r.kind, r.text);
                }
                let _ = writeln!(out, "    windows {}", day.windows.len());
            }
            let _ = writeln!(out, "  acc {}", row.accessibility);
        }
        for c in &v.checklist {
            let _ = writeln!(
                out,
                "check {:?} {} | {} | {:?} | {:?}",
                c.kind, c.id, c.text, c.detail, c.done
            );
        }
        for c in &v.attribution {
            let _ = writeln!(out, "credit {}: {}", c.source, c.text);
        }
        for n in &v.notices {
            let _ = writeln!(out, "notice {:?}: {}", n.kind, n.text);
        }
        let c = &v.compact;
        let _ = writeln!(
            out,
            "compact {} | {} | {} | {}\ncompact acc {}",
            c.title, c.when, c.headline, c.line, c.accessibility
        );
        out
    }

    fn cell(day: &TripPlaceDay, kind: TripCellKind) -> &TripCell {
        day.cells.iter().find(|c| c.kind == kind).unwrap()
    }

    fn day(v: &TripView, row: usize, place: usize) -> &TripPlaceDay {
        v.days[row].places[place].as_ref().unwrap()
    }

    #[test]
    fn three_days_at_the_battery_us_and_luna() {
        let input = view_input();
        let v = battery_three_days(&input).view(&input);
        assert_eq!(v.days.len(), 3);
        assert_eq!(v.status, "Under way");
        assert!(!v.is_past && !v.is_upcoming);
        let today = day(&v, 0, 0);
        assert!(today.is_today && !today.is_past);
        assert_eq!(
            today.cells.iter().map(|c| c.kind).collect::<Vec<_>>(),
            TripCellKind::ALL
        );
        let verdict = cell(today, TripCellKind::Verdict);
        assert_eq!(verdict.basis, CellBasis::Forecast);
        assert_eq!(verdict.basis_label, "forecast, issued Nov 8, 7:30 AM");
        assert_eq!(today.verdict, Some(Verdict::NoGo));
        assert!(verdict.lines[0].starts_with("Luna's not impressed: gusts"));
        // VoiceOver reads the plain meaning first.
        assert!(
            verdict.accessibility.starts_with("Go or no go, forecast"),
            "{}",
            verdict.accessibility
        );
        assert!(verdict.accessibility.contains("Luna's not impressed"));
        let tides = cell(today, TripCellKind::Tides);
        assert_eq!(tides.basis, CellBasis::Certain);
        assert!(tides.lines.iter().all(|l| l.ends_with("ft MLLW")));
        let temp = cell(today, TripCellKind::WaterTemp);
        assert_eq!(temp.basis, CellBasis::Typical);
        assert_eq!(
            temp.basis_label,
            "typical for these dates, 2016–2025, reanalysis"
        );
        // Past the weather's end there is no verdict, and it says why.
        let last = cell(day(&v, 2, 0), TripCellKind::Verdict);
        assert_eq!(last.basis_label, "unavailable: outside the forecast");
        assert_eq!(last.value, "—");
        assert!(
            v.notices
                .iter()
                .any(|n| n.kind == NoticeKind::NotForNavigation)
        );
        insta::assert_snapshot!("battery_us_luna", render(&v));
    }

    #[test]
    fn three_days_at_the_battery_metric_and_plain() {
        let mut input = view_input();
        input.units = metric();
        input.voice = Voice::Plain;
        input.clock = whether_core::presentation::TimeFormat::H24;
        let v = battery_three_days(&input).view(&input);
        let tides = cell(day(&v, 0, 0), TripCellKind::Tides);
        assert!(
            tides.lines.iter().all(|l| l.ends_with(" m MLLW")),
            "{tides:?}"
        );
        let verdict = cell(day(&v, 0, 0), TripCellKind::Verdict);
        assert!(verdict.lines[0].starts_with("No-go: gusts"));
        insta::assert_snapshot!("battery_metric_plain", render(&v));
    }

    #[test]
    fn beyond_the_forecast_tides_are_certain_and_the_rest_is_history() {
        let input = view_input();
        let v = beyond_the_forecast(&input).view(&input);
        assert!(v.is_upcoming);
        assert_eq!(v.status, "60 days to go");
        for row in &v.days {
            let d = row.places[0].as_ref().unwrap();
            assert_eq!(d.verdict, None);
            assert_eq!(
                cell(d, TripCellKind::Verdict).basis_label,
                "unavailable: outside the forecast"
            );
            assert_eq!(cell(d, TripCellKind::Tides).basis, CellBasis::Certain);
            let windows = cell(d, TripCellKind::Windows);
            assert_eq!(windows.basis, CellBasis::Certain);
            assert!(
                windows
                    .lines
                    .contains(&"From tide, light and solunar only".to_owned())
            );
            assert_eq!(cell(d, TripCellKind::WaterTemp).basis, CellBasis::Typical);
        }
        assert_eq!(v.compact.headline, "Luna's waiting for the forecast");
        assert_eq!(
            v.compact.line,
            "Thu Jan 7 at The Battery: unavailable: outside the forecast"
        );
        insta::assert_snapshot!("beyond_the_forecast", render(&v));
    }

    #[test]
    fn two_places_are_columns_with_their_own_sources_and_links() {
        let input = view_input();
        let v = two_places(&input).view(&input);
        assert_eq!(v.places.len(), 2);
        assert_eq!(
            v.places[1].approximate.as_deref(),
            Some("Approximate location")
        );
        for row in &v.days {
            assert_eq!(row.places.len(), 2);
            assert!(row.places.iter().all(Option::is_some));
            // The row reads as one sentence covering both places.
            assert!(row.accessibility.contains("The Battery"));
            assert!(row.accessibility.contains("Croton River"));
        }
        let croton = day(&v, 0, 1);
        let flow = cell(croton, TripCellKind::Flow);
        assert_eq!(
            flow.basis_label,
            "typical for these dates, 93 years, station records"
        );
        assert!(flow.lines.contains(&"In your best range".to_owned()));
        assert_eq!(
            cell(croton, TripCellKind::Tides).basis_label,
            "unavailable: no source here"
        );
        // Salt and fresh water regulations for New York and the licence, then the disclaimer.
        let kinds: Vec<ChecklistKind> = v.checklist.iter().map(|c| c.kind).collect();
        let disclaimer = kinds
            .iter()
            .position(|k| *k == ChecklistKind::Disclaimer)
            .unwrap();
        assert!(kinds[..disclaimer].contains(&ChecklistKind::Licence));
        assert!(
            kinds[..disclaimer]
                .iter()
                .all(|k| matches!(k, ChecklistKind::Licence | ChecklistKind::Regulations))
        );
        assert!(v.checklist[..disclaimer].iter().all(|c| c.link.is_some()));
        assert_eq!(
            v.checklist[disclaimer].text,
            crate::regulations::REGULATIONS_DISCLAIMER
        );
        insta::assert_snapshot!("two_places", render(&v));
    }

    #[test]
    fn history_never_appears_without_its_years_and_kind() {
        let input = view_input();
        let b = two_places(&input);
        let v = b.view(&input);
        let typical: Vec<&TripCell> = v
            .days
            .iter()
            .flat_map(|r| r.places.iter().flatten())
            .flat_map(|d| d.cells.iter())
            .filter(|c| c.basis == CellBasis::Typical)
            .collect();
        assert!(typical.len() >= 9);
        for c in typical {
            assert!(c.basis_label.starts_with("typical for these dates, "));
            assert!(
                ["station records", "reanalysis", "model archive"]
                    .iter()
                    .any(|k| c.basis_label.ends_with(k)),
                "{}",
                c.basis_label
            );
            assert!(c.basis_label.chars().any(|ch| ch.is_ascii_digit()));
            assert!(!c.basis_label.contains("forecast"));
            assert!(c.accessibility.contains(&c.basis_label));
        }
        // Without the history the grid was built from, a typical cell is not shown at all.
        let mut inputs = b.inputs.clone();
        for i in inputs.values_mut() {
            i.history = None;
        }
        let v = build_trip_view(&b.trip, &b.grid, &inputs, &TripAppState::default(), &input);
        let flow = cell(day(&v, 0, 1), TripCellKind::Flow);
        assert_eq!(flow.basis, CellBasis::Unavailable);
        assert_eq!(flow.basis_label, "unavailable: no history for these dates");
        assert_eq!(flow.value, "—");
        assert_eq!(flow.lines, Vec::<String>::new());
        assert_eq!(flow.source, None);
    }

    #[test]
    fn storms_stay_safety_text_with_their_links() {
        let mut input = view_input();
        let mut b = battery_three_days(&input);
        let start = ny_start(date(2026, 11, 9));
        let place = b.grid.places.values_mut().next().unwrap();
        let verdict = place.days[1].verdict.value.as_mut().unwrap();
        verdict.verdict = Verdict::NoGo;
        verdict.storm = vec![Reason {
            check: Check::Hurricane,
            kind: ReasonKind::Fails,
            measured: None,
            limit: None,
            at: Some(start),
            until: Some(UnixTime(start.0 + 30 * HOUR)),
            source: Some(SourceId::from_static("nhc")),
            subject: Some("Hurricane Melissa".into()),
            link: Some("https://www.nhc.noaa.gov/".into()),
        }];
        let mut texts = Vec::new();
        for voice in [Voice::Luna, Voice::Plain] {
            input.voice = voice;
            let v = b.view(&input);
            let d = day(&v, 1, 0);
            assert!(d.storm);
            let storm = &d.reasons[0];
            assert_eq!(storm.check, Check::Hurricane);
            assert_eq!(storm.link.as_deref(), Some("https://www.nhc.noaa.gov/"));
            assert!(storm.text.starts_with("No-go: "), "{}", storm.text);
            let verdict = cell(d, TripCellKind::Verdict);
            assert_eq!(verdict.value, "No-go");
            assert_eq!(verdict.lines[0], storm.text);
            assert!(
                v.attribution
                    .iter()
                    .any(|c| c.source == "NOAA National Hurricane Center")
            );
            texts.push(storm.text.clone());
        }
        // The same words in both voices.
        assert_eq!(texts[0], texts[1]);
    }

    #[test]
    fn past_and_today_follow_each_places_zone() {
        let mut la = croton();
        la.spot_id = "spot-la".into();
        la.name = "Los Angeles River".into();
        la.time_zone = "America/Los_Angeles".into();
        la.region = Some("US-CA".into());
        let first = date(2026, 11, 8);
        let t = trip(vec![battery(), la], first, first);
        // 00:30 on Nov 9 in New York, 21:30 on Nov 8 in Los Angeles.
        let mut input = view_input();
        input.now = UnixTime(ny_start(date(2026, 11, 9)).0 + 1_800);
        let b = built(t, BTreeMap::new(), &input);
        let v = b.view(&input);
        assert!(!v.is_past && !v.is_upcoming);
        assert_eq!(v.status, "Under way");
        let (ny, west) = (day(&v, 0, 0), day(&v, 0, 1));
        assert!(ny.is_past && !ny.is_today);
        assert!(!west.is_past && west.is_today);
        assert_eq!(
            v.compact.line,
            "Sun Nov 8 at Los Angeles River: unavailable: no forecast yet"
        );
        // A day later both are over.
        input.now = UnixTime(input.now.0 + 24 * HOUR);
        let v = b.view(&input);
        assert!(v.is_past);
        assert_eq!(v.status, "This trip is over");
        assert_eq!(v.compact.headline, "This trip is over");
    }

    #[test]
    fn the_next_trip_is_the_soonest_not_over() {
        let input = view_input();
        let soon = battery_three_days(&input).view(&input);
        let later = beyond_the_forecast(&input).view(&input);
        let mut over = soon.clone();
        over.is_past = true;
        over.first = "2026-10-01".into();
        let views = [later.clone(), over, soon.clone()];
        let next = next_trip(&views).unwrap();
        assert_eq!(next, &soon.compact);
        assert_eq!(next.title, "Next trip: Hudson weekend");
        assert_eq!(next.headline, "No-go");
        assert_eq!(next.verdict, Some(Verdict::NoGo));
        assert!(
            next.line
                .starts_with("Sun Nov 8 at The Battery: Luna's not impressed"),
            "{}",
            next.line
        );
        assert!(
            next.accessibility.starts_with(
                "Next trip: Hudson weekend. Under way. No-go. Sun Nov 8 at The Battery: No-go: gusts"
            ),
            "{}",
            next.accessibility
        );
        assert_eq!(
            next_trip(std::slice::from_ref(&later)),
            Some(&later.compact)
        );
        assert_eq!(next_trip(&[]), None);
    }

    #[test]
    fn the_checklist_holds_items_gear_and_the_apps_state() {
        let input = view_input();
        let b = battery_three_days(&input);
        let v = b.view(&input);
        let item = v
            .checklist
            .iter()
            .find(|c| c.kind == ChecklistKind::Item)
            .unwrap();
        assert_eq!((item.text.as_str(), item.done), ("Bucktails", Some(true)));
        let gear_rows: Vec<&str> = v
            .checklist
            .iter()
            .filter(|c| c.kind == ChecklistKind::Gear)
            .map(|c| c.text.as_str())
            .collect();
        assert_eq!(gear_rows[0], "Life jacket");
        assert!(gear_rows.contains(&"Snacks (Luna insists)"));
        let offline = v
            .checklist
            .iter()
            .find(|c| c.kind == ChecklistKind::OfflinePack)
            .unwrap();
        assert_eq!(offline.text, "Offline pack: 48.3 MB, saved Nov 7");
        assert_eq!(offline.done, Some(true));
        let sharing = v.checklist.last().unwrap();
        assert_eq!(sharing.text, "Shared live, read-only");
        let none = build_trip_view(
            &b.trip,
            &b.grid,
            &b.inputs,
            &TripAppState::default(),
            &input,
        );
        let texts: Vec<&str> = none.checklist.iter().map(|c| c.text.as_str()).collect();
        assert!(texts.contains(&"Offline pack: not downloaded"));
        assert!(texts.contains(&"Not shared"));
        for m in [
            Method::Boat,
            Method::Kayak,
            Method::Shore,
            Method::Wade,
            Method::Ice,
        ] {
            assert!(gear(m).len() >= 5, "{m:?}");
        }
    }

    #[test]
    fn schema_snapshot() {
        let schema = schemars::schema_for!(TripView);
        insta::assert_json_snapshot!(schema);
    }
}
