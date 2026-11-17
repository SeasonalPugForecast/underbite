import Foundation

/// The display contract from the Rust core (`underbite_core::view`), as `viewModel(...)` returns
/// it: the page of the active spot and the list of all. Every value is already formatted for
/// the settings' units, clock and voice: draw it, never convert or format it.
///
/// Decoding tolerates what later versions add: unknown fields are ignored and a card of an
/// unknown kind decodes as ``Card/unknown(kind:chrome:)``, so one new card never fails the page.
/// Enum-like fields (kinds, states, roles, severities) stay strings for the same reason.
/// The types live in this namespace so they never meet the generated bindings' names.
public enum ViewModel {
    /// A decoder for the core's JSON (snake_case keys).
    public static func decoder() -> JSONDecoder {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return decoder
    }

    /// Decodes the page JSON `viewModel(...)` returns.
    public static func decode(_ json: String) throws -> Page {
        try decoder().decode(Page.self, from: Data(json.utf8))
    }

    /// Builds the page for `spots` showing `active`, with the settings' JSON, and decodes it.
    /// Reads every cached snapshot: call it off the main actor.
    public static func build(
        spots: [SpotEntry], active: Int, settingsJSON: String, now: Date = .now
    ) throws -> Page {
        let json = try viewModel(
            spots: spots, active: UInt32(clamping: active), settingsJson: settingsJSON,
            now: Int64(now.timeIntervalSince1970))
        return try decode(json)
    }

    /// Builds the page with typed settings.
    public static func build(
        spots: [SpotEntry], active: Int, settings: Settings, now: Date = .now
    ) throws -> Page {
        try build(spots: spots, active: active, settingsJSON: settings.json(), now: now)
    }
}

// MARK: - The page

extension ViewModel {
    /// What `viewModel` returns: the spot list, which spot is shown and its page.
    public struct Page: Decodable, Sendable, Hashable {
        public var active: Int
        public var spots: [SpotSummary]
        /// `nil` when the active spot has no snapshot yet, or its snapshot cannot be read.
        public var view: SpotView?
        /// Why the active spot has no view, when there is a reason to give.
        public var problem: PageProblem?
    }

    /// Why a spot's page is empty: the core's title ("Conditions could not be loaded") and
    /// the reason (the last fetch's error, or why the saved snapshot cannot be read).
    public struct PageProblem: Decodable, Sendable, Hashable {
        public var title: String
        public var reason: String
    }

    /// One row of the spot list.
    public struct SpotSummary: Decodable, Sendable, Hashable, Identifiable {
        public var id: String
        public var name: String
        public var waterType: String
        /// "River or stream", "Tidal river or estuary", ...
        public var waterTypeLabel: String
        public var status: SpotStatus
    }

    /// How a spot's data stands.
    public struct SpotStatus: Decodable, Sendable, Hashable {
        /// Unix seconds of the last successful fetch.
        public var lastSuccess: Int64?
        /// "updated 6 min ago".
        public var updated: String?
        public var error: String?
        public var refreshing: Bool
    }

    /// The conditions page of one spot.
    public struct SpotView: Decodable, Sendable, Hashable {
        public var header: SpotHeader
        /// In the user's order.
        public var cards: [Card]
        public var timeline: Timeline?
        /// Page-level messages, such as "no data for this spot yet".
        public var notices: [SpotNotice]
    }

    public struct SpotHeader: Decodable, Sendable, Hashable {
        public var name: String
        public var waterType: String
        public var waterTypeLabel: String
        /// Unix seconds.
        public var updatedAt: Int64
        public var updated: String
    }

    public struct SpotNotice: Decodable, Sendable, Hashable {
        public var text: String
    }
}

// MARK: - Cards

extension ViewModel {
    /// A card of a known kind: its frame, compact form and full form.
    public struct CardContent<Full: Decodable & Sendable & Hashable>: Decodable, Sendable,
        Hashable
    {
        public var chrome: CardChrome
        public var compact: Compact
        public var full: Full
    }

    public typealias NowCard = CardContent<NowFull>
    public typealias TideCard = CardContent<TideFull>
    public typealias CurrentCard = CardContent<CurrentFull>
    public typealias FlowCard = CardContent<FlowFull>
    public typealias WaterTempCard = CardContent<WaterTempFull>
    public typealias ClarityCard = CardContent<ClarityFull>
    public typealias WavesCard = CardContent<WavesFull>
    public typealias WindCard = CardContent<WindFull>
    public typealias PressureCard = CardContent<PressureFull>
    public typealias SunMoonCard = CardContent<SunMoonFull>
    public typealias SolunarCard = CardContent<SolunarFull>
    public typealias MarineForecastCard = CardContent<MarineForecastFull>
    public typealias StormAheadCard = CardContent<StormAheadFull>
    public typealias AlertsCard = CardContent<AlertsFull>
    public typealias SourcesCard = CardContent<SourcesFull>
    public typealias WindowsCard = CardContent<WindowsFull>
    public typealias GoNoGoCard = CardContent<GoNoGoFull>

    /// One card, by its `card` tag. A kind this version does not know is `.unknown`: skip it or
    /// show its frame.
    public enum Card: Decodable, Sendable, Hashable {
        case now(NowCard)
        case tide(TideCard)
        case current(CurrentCard)
        case flow(FlowCard)
        case waterTemp(WaterTempCard)
        case clarity(ClarityCard)
        case waves(WavesCard)
        case wind(WindCard)
        case pressure(PressureCard)
        case sunMoon(SunMoonCard)
        case solunar(SolunarCard)
        case marineForecast(MarineForecastCard)
        case stormAhead(StormAheadCard)
        case alerts(AlertsCard)
        case sources(SourcesCard)
        case windows(WindowsCard)
        case goNoGo(GoNoGoCard)
        /// A card of a later version; `chrome` is set when its frame still decodes.
        case unknown(kind: String, chrome: CardChrome?)

        private enum Key: String, CodingKey { case card, chrome }

        public init(from decoder: Decoder) throws {
            let keys = try decoder.container(keyedBy: Key.self)
            let kind = try keys.decode(String.self, forKey: .card)
            switch kind {
            case "now": self = .now(try NowCard(from: decoder))
            case "tide": self = .tide(try TideCard(from: decoder))
            case "current": self = .current(try CurrentCard(from: decoder))
            case "flow": self = .flow(try FlowCard(from: decoder))
            case "water_temp": self = .waterTemp(try WaterTempCard(from: decoder))
            case "clarity": self = .clarity(try ClarityCard(from: decoder))
            case "waves": self = .waves(try WavesCard(from: decoder))
            case "wind": self = .wind(try WindCard(from: decoder))
            case "pressure": self = .pressure(try PressureCard(from: decoder))
            case "sun_moon": self = .sunMoon(try SunMoonCard(from: decoder))
            case "solunar": self = .solunar(try SolunarCard(from: decoder))
            case "marine_forecast": self = .marineForecast(try MarineForecastCard(from: decoder))
            case "storm_ahead": self = .stormAhead(try StormAheadCard(from: decoder))
            case "alerts": self = .alerts(try AlertsCard(from: decoder))
            case "sources": self = .sources(try SourcesCard(from: decoder))
            case "windows": self = .windows(try WindowsCard(from: decoder))
            case "go_no_go": self = .goNoGo(try GoNoGoCard(from: decoder))
            default:
                let chrome = try? keys.decodeIfPresent(CardChrome.self, forKey: .chrome)
                self = .unknown(kind: kind, chrome: chrome)
            }
        }

        /// The card's frame (title, credits, state, notices, accessibility text), when it has one.
        public var chrome: CardChrome? {
            switch self {
            case .now(let c): c.chrome
            case .tide(let c): c.chrome
            case .current(let c): c.chrome
            case .flow(let c): c.chrome
            case .waterTemp(let c): c.chrome
            case .clarity(let c): c.chrome
            case .waves(let c): c.chrome
            case .wind(let c): c.chrome
            case .pressure(let c): c.chrome
            case .sunMoon(let c): c.chrome
            case .solunar(let c): c.chrome
            case .marineForecast(let c): c.chrome
            case .stormAhead(let c): c.chrome
            case .alerts(let c): c.chrome
            case .sources(let c): c.chrome
            case .windows(let c): c.chrome
            case .goNoGo(let c): c.chrome
            case .unknown(_, let chrome): chrome
            }
        }

        /// The card's compact form (widgets, the trip list), when it has one.
        public var compact: Compact? {
            switch self {
            case .now(let c): c.compact
            case .tide(let c): c.compact
            case .current(let c): c.compact
            case .flow(let c): c.compact
            case .waterTemp(let c): c.compact
            case .clarity(let c): c.compact
            case .waves(let c): c.compact
            case .wind(let c): c.compact
            case .pressure(let c): c.compact
            case .sunMoon(let c): c.compact
            case .solunar(let c): c.compact
            case .marineForecast(let c): c.compact
            case .stormAhead(let c): c.compact
            case .alerts(let c): c.compact
            case .sources(let c): c.compact
            case .windows(let c): c.compact
            case .goNoGo(let c): c.compact
            case .unknown: nil
            }
        }

        /// False for a card this version does not know.
        public var isKnown: Bool {
            if case .unknown = self { return false }
            return true
        }
    }

    /// The frame every card shares.
    public struct CardChrome: Decodable, Sendable, Hashable {
        /// The card's id as the settings write it (`water_temp`, ...).
        public var id: String
        public var title: String
        /// The provenance line; `nil` for cards that combine many sources.
        public var footnote: String?
        public var credits: [Credit]
        public var state: CardState
        public var notices: [Notice]
        /// What VoiceOver reads: the plain meaning first.
        public var accessibility: String
    }

    /// The credit a source is owed.
    public struct Credit: Decodable, Sendable, Hashable {
        public var source: String
        public var text: String
        public var link: String?
        public var licence: String?
    }

    /// Whether a value can be trusted as current and local.
    public enum CardState: Decodable, Sendable, Hashable {
        case fresh
        /// Older than the user's age limit; grey it.
        case stale(reason: String)
        /// From a station beyond the user's distance limit; grey it.
        case far(reason: String)
        case unavailable(reason: String)
        /// A state of a later version.
        case unknown(String)

        private enum Key: String, CodingKey { case state, reason }

        public init(from decoder: Decoder) throws {
            let keys = try decoder.container(keyedBy: Key.self)
            let state = try keys.decode(String.self, forKey: .state)
            let reason = try keys.decodeIfPresent(String.self, forKey: .reason) ?? ""
            switch state {
            case "fresh": self = .fresh
            case "stale": self = .stale(reason: reason)
            case "far": self = .far(reason: reason)
            case "unavailable": self = .unavailable(reason: reason)
            default: self = .unknown(state)
            }
        }

        /// The reason shown with a greyed value, if the state has one.
        public var reason: String? {
            switch self {
            case .stale(let r), .far(let r), .unavailable(let r): r
            case .fresh, .unknown: nil
            }
        }
    }

    /// A fixed notice: plain, never in Luna's voice. `kind` is `not_for_navigation`, `modelled`,
    /// `provisional`, `approximate`, `traditional_cue` or `text_only`.
    public struct Notice: Decodable, Sendable, Hashable {
        public var kind: String
        public var text: String
    }

    /// The compact form: a headline and at most two short lines.
    public struct Compact: Decodable, Sendable, Hashable {
        public var headline: Quantity
        public var lines: [String]
        /// A sparkline in display units, oldest first; empty when the card has none.
        public var spark: [Double]
    }

    /// A displayed value: `text` to show, `value` (display units) to draw with.
    public struct Quantity: Decodable, Sendable, Hashable {
        public var value: Double?
        /// The display text without the unit, or the missing placeholder.
        public var text: String
        /// The unit symbol, empty when unitless.
        public var unit: String
    }

    /// "Rising 0.4 ft in 3 h"; `direction` is `rising`, `falling`, `steady` or `unknown`.
    public struct Trend: Decodable, Sendable, Hashable {
        public var direction: String
        public var text: String
    }

    /// A time with its label ("Sunrise", "6:42 AM"); `t` is Unix seconds, for sorting and drawing.
    public struct TimedLabel: Decodable, Sendable, Hashable {
        public var label: String
        public var time: String
        public var t: Int64
    }

    /// A titled block of label and value rows.
    public struct Section: Decodable, Sendable, Hashable {
        public var title: String
        public var rows: [Row]
    }

    public struct Row: Decodable, Sendable, Hashable {
        public var label: String
        public var value: String
        /// A second, smaller line.
        public var detail: String?
    }

    /// An event table: one header, rows of cells in column order.
    public struct Table: Decodable, Sendable, Hashable {
        public var title: String
        public var columns: [String]
        public var rows: [TableRow]
    }

    public struct TableRow: Decodable, Sendable, Hashable {
        public var cells: [String]
    }
}

// MARK: - Charts and the timeline

extension ViewModel {
    /// A chart carries every point with its label; narrow layouts thin them by `major`.
    public struct Chart: Decodable, Sendable, Hashable {
        public var title: String
        /// The y unit symbol, empty when mixed or unitless.
        public var unit: String
        public var xTicks: [ChartTick]
        public var series: [ChartSeries]
        public var bands: [ChartBand]
        public var rules: [ChartRule]
        public var markers: [ChartMarker]
        /// Unix seconds of "now" when it falls inside the chart.
        public var nowT: Int64?
        /// What VoiceOver reads: the shape and its extremes in words.
        public var accessibility: String
    }

    public struct ChartTick: Decodable, Sendable, Hashable {
        public var t: Int64
        public var label: String
        public var major: Bool
    }

    /// `role` is `predicted`, `observed`, `forecast` or `modelled`; `style` is `line`, `area`,
    /// `bars` or `dots`.
    public struct ChartSeries: Decodable, Sendable, Hashable {
        public var label: String
        public var role: String
        public var style: String
        public var points: [ChartPoint]
    }

    /// One point: Unix seconds, the value in display units and its label ("3.4 ft").
    public struct ChartPoint: Decodable, Sendable, Hashable {
        public var t: Int64
        public var value: Double
        public var label: String
        public var major: Bool
    }

    /// A shaded span; `kind` is `night`, `slack`, `solunar_major`, `solunar_minor` or `highlight`.
    public struct ChartBand: Decodable, Sendable, Hashable {
        public var kind: String
        public var startT: Int64
        public var endT: Int64
        public var label: String
    }

    /// A horizontal line; `kind` is `flood_category`, `user_range` or `threshold`.
    public struct ChartRule: Decodable, Sendable, Hashable {
        public var kind: String
        /// In display units.
        public var value: Double
        public var label: String
    }

    /// A labelled event; `kind` is `high`, `low`, `slack`, `sunrise`, `sunset` or `now`.
    public struct ChartMarker: Decodable, Sendable, Hashable {
        public var kind: String
        public var t: Int64
        /// In display units, when the marker sits on the curve.
        public var value: Double?
        public var label: String
    }

    /// The day timeline: lanes that share one time axis.
    public struct Timeline: Decodable, Sendable, Hashable {
        public var startT: Int64
        public var endT: Int64
        public var stepMinutes: Int
        public var xTicks: [ChartTick]
        public var lanes: [TimelineLane]
    }

    /// `kind` is `tide`, `current`, `wind`, `pressure`, `light`, `solunar` or `windows`. The lane carries
    /// its own credits and notices: the timeline stands alone, away from the cards.
    public struct TimelineLane: Decodable, Sendable, Hashable {
        public var kind: String
        public var label: String
        public var chart: Chart
        public var credits: [Credit]
        public var notices: [Notice]
    }
}

// MARK: - Full forms

extension ViewModel {
    // Windows

    public struct WindowsFull: Decodable, Sendable, Hashable {
        public var days: [WindowDayVm]
        public var sections: [Section]
    }

    /// `date` is the day's label ("Sun Nov 8").
    public struct WindowDayVm: Decodable, Sendable, Hashable {
        public var date: String
        public var windows: [WindowVm]
    }

    /// One window: `range` is its words ("6:10 to 8:00 AM"); `startT` and `endT` are Unix seconds.
    public struct WindowVm: Decodable, Sendable, Hashable {
        public var startT: Int64
        public var endT: Int64
        public var range: String
        public var factors: [WindowFactorVm]
        /// Enabled factors that had no data in the window, by name.
        public var noData: [String]
        /// The same, worded for display ("No data: water temperature, river flow"); none when
        /// every enabled factor had data.
        public var noDataText: String?
    }

    /// `factor` is `tide_stage`, `current_strength`, `low_light`, `solunar`, `pressure_falling`,
    /// `water_temp` or `flow_in_range`.
    public struct WindowFactorVm: Decodable, Sendable, Hashable {
        public var factor: String
        public var label: String
        public var detail: String
    }

    // Go / no-go

    public struct GoNoGoFull: Decodable, Sendable, Hashable {
        /// The name of the limits profile the days are judged against.
        public var profile: String
        public var days: [GoNoGoDayVm]
    }

    /// `verdict` is `go`, `marginal`, `no_go` or `unknown`; `verdictLabel` is its word.
    public struct GoNoGoDayVm: Decodable, Sendable, Hashable {
        public var date: String
        public var verdict: String
        public var verdictLabel: String
        public var summary: String
        public var reasons: [GoNoGoReasonVm]
        public var hours: [GoNoGoHourVm]
    }

    /// `check` names the limit; `kind` is how it came out. `link` is a hurricane's official page.
    public struct GoNoGoReasonVm: Decodable, Sendable, Hashable {
        public var check: String
        public var kind: String
        public var text: String
        public var link: String?
    }

    public struct GoNoGoHourVm: Decodable, Sendable, Hashable {
        public var startT: Int64
        public var label: String
        public var verdict: String
        public var reason: String?
    }

    // Now

    public struct NowFull: Decodable, Sendable, Hashable {
        public var items: [NowItem]
    }

    /// One line of the Now card; `card` is the id of the card it comes from.
    public struct NowItem: Decodable, Sendable, Hashable {
        public var card: String
        public var label: String
        public var headline: Quantity
        public var line: String?
        public var state: CardState
    }

    // Tide

    /// `stage` is `rising`, `falling`, `high_slack`, `low_slack` or `unknown`.
    public struct TideFull: Decodable, Sendable, Hashable {
        public var stage: String
        public var stageLabel: String
        public var heightNow: Quantity?
        public var next: [TideEvent]
        public var datumLabel: String?
        public var heightsShown: Bool
        public var chart: Chart?
        public var days: [TideDay]
        public var sections: [Section]
    }

    /// `kind` is `high` or `low`; `t` is Unix seconds.
    public struct TideEvent: Decodable, Sendable, Hashable {
        public var t: Int64
        public var time: String
        public var kind: String
        public var label: String
        public var height: Quantity?
    }

    public struct TideDay: Decodable, Sendable, Hashable {
        public var date: String
        public var events: [TideEvent]
        public var sunrise: TimedLabel?
        public var sunset: TimedLabel?
        public var moon: MoonPhase?
    }

    // Current

    /// `phase` is `flood`, `ebb`, `slack` or `unknown`.
    public struct CurrentFull: Decodable, Sendable, Hashable {
        public var phase: String
        public var phaseLabel: String
        public var speedNow: Quantity?
        public var directionNow: String?
        public var nextSlack: CurrentEvent?
        public var events: [CurrentEvent]
        public var floodDirection: String?
        public var ebbDirection: String?
        public var binDepth: String?
        public var chart: Chart?
        public var sections: [Section]
    }

    /// `kind` is `max_flood`, `max_ebb` or `slack`.
    public struct CurrentEvent: Decodable, Sendable, Hashable {
        public var t: Int64
        public var time: String
        public var kind: String
        public var label: String
        public var speed: Quantity?
    }

    // Flow

    public struct FlowFull: Decodable, Sendable, Hashable {
        public var discharge: Quantity?
        public var trend: Trend?
        public var gageHeight: Quantity?
        public var percentile: Percentile?
        public var rangeStatus: RangeStatus?
        public var floodCategories: [FloodCategory]
        public var chart: Chart?
        public var stageChart: Chart?
        public var sections: [Section]
    }

    /// `status` is `wadeable`, `best`, `fishable`, `blown_out` or `unknown`.
    public struct RangeStatus: Decodable, Sendable, Hashable {
        public var status: String
        public var text: String
    }

    public struct Percentile: Decodable, Sendable, Hashable {
        public var text: String
        /// The core's name for the class; `classLabel` is the words.
        public var `class`: String?
        public var classLabel: String
        public var basis: String
        public var sample: String
        public var thresholds: [PercentileThreshold]
    }

    public struct PercentileThreshold: Decodable, Sendable, Hashable {
        public var label: String
        public var value: Quantity
    }

    public struct FloodCategory: Decodable, Sendable, Hashable {
        public var label: String
        public var stage: Quantity
    }

    // Water temperature

    /// `basis` is `measured` or `sea_surface`.
    public struct WaterTempFull: Decodable, Sendable, Hashable {
        public var value: Quantity?
        public var basis: String
        public var trend: Trend?
        public var troutStress: TroutStress?
        public var chart: Chart?
        public var sections: [Section]
    }

    public struct TroutStress: Decodable, Sendable, Hashable {
        public var exceeded: Bool
        public var threshold: Quantity
        public var text: String
    }

    // Clarity

    public struct ClarityFull: Decodable, Sendable, Hashable {
        public var turbidity: Quantity?
        public var chlorophyll: Quantity?
        public var kd490: Kd490
        public var chart: Chart?
        public var sections: [Section]
    }

    public struct Kd490: Decodable, Sendable, Hashable {
        public var available: Bool
        public var value: Quantity?
        public var note: String
    }

    // Waves

    public struct WavesFull: Decodable, Sendable, Hashable {
        public var observed: WaveNow?
        public var forecast: WaveNow?
        public var heightChart: Chart?
        public var periodChart: Chart?
        public var sections: [Section]
    }

    public struct WaveNow: Decodable, Sendable, Hashable {
        public var t: Int64
        public var time: String
        public var significantHeight: Quantity?
        public var dominantPeriod: Quantity?
        public var meanPeriod: Quantity?
        public var direction: String?
        public var windWaves: WaveTrain?
        public var swells: [WaveTrain]
        public var steepness: String?
    }

    public struct WaveTrain: Decodable, Sendable, Hashable {
        public var label: String
        public var height: Quantity?
        public var period: Quantity?
        public var direction: String?
    }

    // Wind

    /// `basis` is `observed` or `forecast`; `fromDeg` is where the wind blows from, to rotate an
    /// arrow with.
    public struct WindFull: Decodable, Sendable, Hashable {
        public var basis: String
        public var speed: Quantity?
        public var gust: Quantity?
        public var direction: String?
        public var fromDeg: Double?
        public var hours: [WindHour]
        public var chart: Chart?
        public var sections: [Section]
    }

    public struct WindHour: Decodable, Sendable, Hashable {
        public var t: Int64
        public var time: String
        public var speed: Quantity
        public var gust: Quantity?
        public var direction: String?
        public var fromDeg: Double?
    }

    // Pressure

    public struct PressureFull: Decodable, Sendable, Hashable {
        public var value: Quantity?
        public var trend: Trend?
        public var change: Quantity?
        public var chart: Chart?
        public var sections: [Section]
    }

    // Sun and moon

    public struct SunMoonFull: Decodable, Sendable, Hashable {
        public var days: [SunMoonDay]
        public var sections: [Section]
    }

    public struct SunMoonDay: Decodable, Sendable, Hashable {
        public var date: String
        public var sun: [TimedLabel]
        public var dayLength: String?
        /// Set when the sun does not rise or set that day.
        public var polar: String?
        public var moon: Moon
    }

    /// `phase` is the core's phase name (`waxing_gibbous`, ...); `label` is the words.
    public struct MoonPhase: Decodable, Sendable, Hashable {
        public var phase: String
        public var label: String
        public var illumination: Quantity
    }

    public struct Moon: Decodable, Sendable, Hashable {
        public var phase: MoonPhase
        public var rise: TimedLabel?
        public var set: TimedLabel?
        public var upperTransit: TimedLabel?
        public var lowerTransit: TimedLabel?
        public var nearestPrincipal: TimedLabel?
    }

    // Solunar

    public struct SolunarFull: Decodable, Sendable, Hashable {
        public var days: [SolunarDay]
        public var sections: [Section]
    }

    public struct SolunarDay: Decodable, Sendable, Hashable {
        public var date: String
        public var periods: [SolunarPeriod]
    }

    /// `kind` is `major` or `minor`; `cause` names what makes it (`upper_transit`, ...) and
    /// `causeLabel` says it in words; `range` is the formatted span.
    public struct SolunarPeriod: Decodable, Sendable, Hashable {
        public var kind: String
        public var cause: String
        public var label: String
        public var startT: Int64
        public var endT: Int64
        public var range: String
        public var causeLabel: String
    }

    // Marine forecast

    public struct MarineForecastFull: Decodable, Sendable, Hashable {
        public var zoneId: String
        public var zoneName: String
        public var issued: String
        public var product: String
        public var headline: String?
        public var periods: [ForecastPeriod]
    }

    public struct ForecastPeriod: Decodable, Sendable, Hashable {
        public var name: String
        public var text: String
    }

    // Storm ahead

    public struct StormAheadFull: Decodable, Sendable, Hashable {
        public var event: StormEvent?
        public var hurricanes: [HurricaneAhead]
        public var sections: [Section]
    }

    /// `severity` is `watch`, `strong`, `severe` or `extreme`; `start` and `end` are Unix seconds.
    public struct StormEvent: Decodable, Sendable, Hashable {
        public var title: String
        public var window: String
        public var start: Int64
        public var end: Int64
        public var severity: String
        public var headline: StormHeadline
        public var agreement: StormAgreement?
        public var models: [StormModelRow]
    }

    /// `kind` is `snow`, `rain` or `gusts`.
    public struct StormHeadline: Decodable, Sendable, Hashable {
        public var kind: String
        public var text: String
        public var amount: Quantity
    }

    public struct StormAgreement: Decodable, Sendable, Hashable {
        public var agreement: String
        public var label: String
        public var sentence: String
    }

    public struct StormModelRow: Decodable, Sendable, Hashable {
        public var model: String
        public var snow: Quantity
        public var precipitation: Quantity
        public var gust: Quantity
        public var low: Quantity
        public var lowAt: String
        public var onset: String
    }

    public struct HurricaneAhead: Decodable, Sendable, Hashable {
        public var id: String
        public var name: String
        public var title: String
        public var classification: String
        public var category: Int?
        public var severity: String
        public var wind: Quantity
        public var pressure: Quantity
        public var movement: String
        public var strength: String
        public var advisory: String
        public var advisoryUrl: String?
        public var reaches: String
        /// Unix seconds.
        public var reachesAt: Int64
        public var leavesAt: Int64
    }

    // Alerts

    public struct AlertsFull: Decodable, Sendable, Hashable {
        public var alerts: [Alert]
    }

    /// `origin` is `marine` or `weather`. `description` and `instruction` are the National
    /// Weather Service's words, verbatim.
    public struct Alert: Decodable, Sendable, Hashable, Identifiable {
        public var id: String
        public var origin: String
        public var event: String
        public var headline: String?
        public var description: String?
        public var instruction: String?
        public var severity: String
        public var severityLabel: String
        public var onset: String?
        public var ends: String?
        public var expires: String?
        /// When it is in effect, in words: "Nov 8, 1:48 AM – Nov 9, 6:00 PM", "From …" or
        /// "Until …"; `nil` when the source gave no time.
        public var span: String?
        public var zones: [String]
    }

    // Sources

    public struct SourcesFull: Decodable, Sendable, Hashable {
        public var credits: [Credit]
        public var stations: [StationUse]
        public var errors: [SourceError]
    }

    public struct StationUse: Decodable, Sendable, Hashable {
        public var measurement: String
        public var station: String
        public var stationId: String
        public var source: String
        public var distance: String?
        public var age: String
        public var relation: String
        public var kind: String
        public var state: CardState
    }

    public struct SourceError: Decodable, Sendable, Hashable {
        public var source: String
        public var measurement: String?
        public var kind: String
        public var message: String
    }
}
