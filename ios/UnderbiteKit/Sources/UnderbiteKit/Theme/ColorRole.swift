import Foundation

/// A painted thing with its own colour in a theme. Where the core names the thing, the raw
/// value is `<family>.<the core's name>` (`verdict.no_go`, `flow.much_below_normal`,
/// `station.flood_minor`, `chart.user_range`), so the lookups below take the view model's
/// strings as they come. Never rename one once shipped: saved themes use them. A role added
/// later is filled from the theme's base preset when a saved theme is loaded.
///
/// Map colour ramps (sea surface temperature, chlorophyll, depth) are the core's and are not
/// roles.
public enum ColorRole: String, CaseIterable, Codable, Sendable {
    // Text and accent: buttons and links, and the words of a warning, an error or a success.
    case accent = "accent"
    case statusWarning = "status.warning", statusError = "status.error", statusSuccess = "status.success"
    // Card states (`CardState`): the mark and the reason beside a value.
    case cardFresh = "card.fresh", cardStale = "card.stale", cardFar = "card.far"
    case cardUnavailable = "card.unavailable"
    // Go / no-go (`Verdict`).
    case verdictGo = "verdict.go", verdictMarginal = "verdict.marginal", verdictNoGo = "verdict.no_go"
    case verdictUnknown = "verdict.unknown"
    // Weather alerts by severity: extreme and severe, then the rest.
    case alertSevere = "alert.severe", alertModerate = "alert.moderate"
    // The catch log's fate tags (`CatchFate`).
    case fateReleased = "fate.released", fateKept = "fate.kept", fateKeptForSashimi = "fate.kept_for_sashimi"
    // River flow against normal for the day of the year (`PercentileClass`), low to high.
    case flowRecordLow = "flow.record_low", flowMuchBelowNormal = "flow.much_below_normal"
    case flowBelowNormal = "flow.below_normal", flowNormal = "flow.normal"
    case flowAboveNormal = "flow.above_normal", flowMuchAboveNormal = "flow.much_above_normal"
    case flowRecordHigh = "flow.record_high"
    // Flood stages on a river card: action, minor, moderate, major.
    case floodAction = "flood.action", floodMinor = "flood.minor", floodModerate = "flood.moderate"
    case floodMajor = "flood.major"
    // Station pins on the map, by the core's `state_role`.
    case stationNone = "station.none", stationUnknown = "station.unknown", stationNoData = "station.no_data"
    case stationReporting = "station.reporting", stationLow = "station.low", stationNormal = "station.normal"
    case stationElevated = "station.elevated", stationFloodMinor = "station.flood_minor"
    case stationFloodModerate = "station.flood_moderate", stationFloodMajor = "station.flood_major"
    // The tide: its curve and the fill under it, high and low marks, slack water.
    case tideCurve = "tide.curve", tideFill = "tide.fill", tideHigh = "tide.high", tideLow = "tide.low"
    case tideSlack = "tide.slack"
    // The timeline's lanes: night on the light lane, sunrise and sunset, solunar periods, and
    // fishing windows (snack times).
    case timelineNight = "timeline.night", timelineSun = "timeline.sun"
    case timelineSolunarMajor = "timeline.solunar_major", timelineSolunarMinor = "timeline.solunar_minor"
    case timelineWindow = "timeline.window"
    // Chart lines by `SeriesRole`, rules by `RuleKind`, the normal band and the now line.
    case chartPredicted = "chart.predicted", chartObserved = "chart.observed"
    case chartForecast = "chart.forecast", chartModelled = "chart.modelled"
    case chartFloodCategory = "chart.flood_category", chartUserRange = "chart.user_range"
    case chartThreshold = "chart.threshold"
    case chartValueBand = "chart.value_band", chartNow = "chart.now"
    // Spots: the favourite star and the pin.
    case spotFavourite = "spot.favourite", spotPinned = "spot.pinned"

    /// How the role is painted, which decides what its colour is checked against
    /// (`ThemeChecks`).
    public enum Kind: Sendable {
        /// Words and the symbols beside them, on a card or the page.
        case text
        /// Lines, dots and marks that carry meaning, on a card or the page.
        case mark
        /// A translucent span with words on it.
        case fill
        /// A map pin's fill, under its white or dark symbol.
        case pin
    }

    /// How the role is painted.
    public var kind: Kind {
        switch group {
        case .text, .cardStates, .verdicts, .alerts, .catchLog: .text
        case .stations: .pin
        case .flow, .flood, .spots: .mark
        case .tide:
            [.tideFill, .tideSlack].contains(self) ? .fill : .mark
        case .timeline:
            self == .timelineSun ? .mark : .fill
        case .charts:
            self == .chartValueBand ? .fill : .mark
        }
    }

    /// The editor's group for this role.
    public var group: ColorRoleGroup {
        let family = rawValue.prefix { $0 != "." }
        return switch family {
        case "card": .cardStates
        case "verdict": .verdicts
        case "alert": .alerts
        case "fate": .catchLog
        case "flow": .flow
        case "flood": .flood
        case "station": .stations
        case "tide": .tide
        case "timeline": .timeline
        case "chart": .charts
        case "spot": .spots
        default: .text
        }
    }

    /// The roles in `group`, in the order they are declared.
    public static func roles(in group: ColorRoleGroup) -> [ColorRole] {
        allCases.filter { $0.group == group }
    }

    // MARK: Lookups by the core's names

    /// The role for a card's state (`fresh`, `stale`, `far`, `unavailable`); `nil` for another.
    public static func cardState(_ state: String) -> ColorRole? {
        ColorRole(rawValue: "card.\(state)")
    }

    /// The role for a go / no-go verdict (`go`, `marginal`, `no_go`); `unknown` for any other.
    public static func verdict(_ verdict: String) -> ColorRole {
        ColorRole(rawValue: "verdict.\(verdict)") ?? .verdictUnknown
    }

    /// The role for an alert's severity: `extreme` and `severe` are severe, the rest moderate.
    public static func alert(severity: String) -> ColorRole {
        ["extreme", "severe"].contains(severity) ? .alertSevere : .alertModerate
    }

    /// The role for a catch's fate (`released`, `kept`, `kept_for_sashimi`); `nil` for another.
    public static func fate(_ fate: String) -> ColorRole? {
        ColorRole(rawValue: "fate.\(fate)")
    }

    /// The role for a flow's percentile class (`much_below_normal`, …); `nil` for another.
    public static func flow(_ percentileClass: String) -> ColorRole? {
        ColorRole(rawValue: "flow.\(percentileClass)")
    }

    /// The role for a flood stage by its name (`action`, `minor`, `moderate`, `major`, in any
    /// case); `nil` for another.
    public static func flood(_ stage: String) -> ColorRole? {
        ColorRole(rawValue: "flood.\(stage.lowercased())")
    }

    /// The role for a station pin's `state_role`; `unknown` for one this version does not know.
    public static func station(_ stateRole: String) -> ColorRole {
        ColorRole(rawValue: "station.\(stateRole)").flatMap { $0.group == .stations ? $0 : nil }
            ?? .stationUnknown
    }

    /// The role for a chart series' role (`predicted`, `observed`, `forecast`, `modelled`);
    /// `predicted` for another.
    public static func series(_ role: String) -> ColorRole {
        switch role {
        case "observed": .chartObserved
        case "forecast": .chartForecast
        case "modelled": .chartModelled
        default: .chartPredicted
        }
    }

    /// The role for a chart rule's kind (`flood_category`, `user_range`, `threshold`);
    /// `threshold` for another.
    public static func rule(_ kind: String) -> ColorRole {
        switch kind {
        case "flood_category": .chartFloodCategory
        case "user_range": .chartUserRange
        default: .chartThreshold
        }
    }

    /// The role for a shaded span's kind (`night`, `slack`, `solunar_major`, `solunar_minor`,
    /// `highlight`); a fishing window for another.
    public static func band(_ kind: String) -> ColorRole {
        switch kind {
        case "night": .timelineNight
        case "slack": .tideSlack
        case "solunar_major": .timelineSolunarMajor
        case "solunar_minor": .timelineSolunarMinor
        default: .timelineWindow
        }
    }

    // MARK: Ordered sets

    /// The flow classes, record low to record high.
    public static let flowClasses: [ColorRole] = [
        .flowRecordLow, .flowMuchBelowNormal, .flowBelowNormal, .flowNormal, .flowAboveNormal,
        .flowMuchAboveNormal, .flowRecordHigh,
    ]
    /// The flood stages, action to major.
    public static let floodStages: [ColorRole] = [.floodAction, .floodMinor, .floodModerate, .floodMajor]
    /// The station states a river gauge steps through, low water to major flooding.
    public static let stationSteps: [ColorRole] = [
        .stationLow, .stationNormal, .stationElevated, .stationFloodMinor, .stationFloodModerate,
        .stationFloodMajor,
    ]
    /// The verdicts.
    public static let verdicts: [ColorRole] = [.verdictGo, .verdictMarginal, .verdictNoGo, .verdictUnknown]
    /// The chart lines, which are drawn together.
    public static let chartLines: [ColorRole] = [.chartPredicted, .chartObserved, .chartForecast, .chartModelled]
    /// The chart rules, which are drawn together.
    public static let chartRules: [ColorRole] = [.chartFloodCategory, .chartUserRange, .chartThreshold]
    /// The fate tags.
    public static let fates: [ColorRole] = [.fateReleased, .fateKept, .fateKeptForSashimi]
}

/// The editor's groups.
public enum ColorRoleGroup: String, CaseIterable, Sendable {
    case text, cardStates, verdicts, alerts, catchLog, flow, flood, stations, tide, timeline, charts, spots

    public var title: String {
        switch self {
        case .text: "Text and accent"
        case .cardStates: "Card states"
        case .verdicts: "Go / no-go"
        case .alerts: "Alerts"
        case .catchLog: "Catch log"
        case .flow: "River flow"
        case .flood: "Flood stages"
        case .stations: "Station pins"
        case .tide: "Tide"
        case .timeline: "Timeline"
        case .charts: "Charts"
        case .spots: "Spots"
        }
    }
}

extension ColorRole {
    /// The role's name in the editor.
    public var title: String {
        switch self {
        case .accent: "Accent"
        case .statusWarning: "Warning"
        case .statusError: "Error"
        case .statusSuccess: "Done"
        case .cardFresh: "Current"
        case .cardStale: "Too old"
        case .cardFar: "Too far"
        case .cardUnavailable: "Unavailable"
        case .verdictGo: "Go"
        case .verdictMarginal: "Marginal"
        case .verdictNoGo: "No-go"
        case .verdictUnknown: "Unknown"
        case .alertSevere: "Severe and extreme"
        case .alertModerate: "Other alerts"
        case .fateReleased: "Released"
        case .fateKept: "Kept"
        case .fateKeptForSashimi: "Kept for sashimi"
        case .flowRecordLow: "Record low"
        case .flowMuchBelowNormal: "Much below normal"
        case .flowBelowNormal: "Below normal"
        case .flowNormal: "Normal"
        case .flowAboveNormal: "Above normal"
        case .flowMuchAboveNormal: "Much above normal"
        case .flowRecordHigh: "Record high"
        case .floodAction: "Action stage"
        case .floodMinor: "Minor flooding"
        case .floodModerate: "Moderate flooding"
        case .floodMajor: "Major flooding"
        case .stationNone: "No state"
        case .stationUnknown: "State unknown"
        case .stationNoData: "No recent value"
        case .stationReporting: "Reporting"
        case .stationLow: "Low water"
        case .stationNormal: "Normal"
        case .stationElevated: "Action stage"
        case .stationFloodMinor: "Minor flooding"
        case .stationFloodModerate: "Moderate flooding"
        case .stationFloodMajor: "Major flooding"
        case .tideCurve: "Curve"
        case .tideFill: "Fill under the curve"
        case .tideHigh: "High tide"
        case .tideLow: "Low tide"
        case .tideSlack: "Slack water"
        case .timelineNight: "Night"
        case .timelineSun: "Sunrise and sunset"
        case .timelineSolunarMajor: "Solunar major"
        case .timelineSolunarMinor: "Solunar minor"
        case .timelineWindow: "Snack times"
        case .chartPredicted: "Predicted"
        case .chartObserved: "Observed"
        case .chartForecast: "Forecast"
        case .chartModelled: "Modelled"
        case .chartFloodCategory: "Flood stage line"
        case .chartUserRange: "Your range"
        case .chartThreshold: "Threshold"
        case .chartValueBand: "Normal band"
        case .chartNow: "Now"
        case .spotFavourite: "Favourite"
        case .spotPinned: "Pinned"
        }
    }
}
