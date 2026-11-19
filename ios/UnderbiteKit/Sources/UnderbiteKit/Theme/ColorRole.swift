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
        case .text: String(localized: "Text and accent", bundle: .module, comment: "Name of a group of colour roles in the theme editor")
        case .cardStates: String(localized: "Card states", bundle: .module, comment: "Name of a group of colour roles in the theme editor")
        case .verdicts: String(localized: "Go / no-go", bundle: .module, comment: "Name of a group of colour roles in the theme editor")
        case .alerts: String(localized: "Alerts", bundle: .module, comment: "Name of a group of colour roles in the theme editor")
        case .catchLog: String(localized: "Catch log", bundle: .module, comment: "Name of a group of colour roles in the theme editor")
        case .flow: String(localized: "River flow", bundle: .module, comment: "Name of a group of colour roles in the theme editor")
        case .flood: String(localized: "Flood stages", bundle: .module, comment: "Name of a group of colour roles in the theme editor")
        case .stations: String(localized: "Station pins", bundle: .module, comment: "Name of a group of colour roles in the theme editor")
        case .tide: String(localized: "Tide", bundle: .module, comment: "Name of a group of colour roles in the theme editor")
        case .timeline: String(localized: "Timeline", bundle: .module, comment: "Name of a group of colour roles in the theme editor")
        case .charts: String(localized: "Charts", bundle: .module, comment: "Name of a group of colour roles in the theme editor")
        case .spots: String(localized: "Spots", bundle: .module, comment: "Name of a group of colour roles in the theme editor")
        }
    }
}

extension ColorRole {
    /// The role's name in the editor.
    public var title: String {
        switch self {
        case .accent: String(localized: "Accent", bundle: .module, comment: "Name of a colour role in the theme editor and in its warnings")
        case .statusWarning: String(localized: "Warning", bundle: .module, comment: "Name of a colour role in the theme editor and in its warnings")
        case .statusError: String(localized: "Error", bundle: .module, comment: "Name of a colour role in the theme editor and in its warnings")
        case .statusSuccess: String(localized: "Done", bundle: .module, comment: "Name of a colour role in the theme editor and in its warnings")
        case .cardFresh: String(localized: "Current", bundle: .module, comment: "Name of a colour role in the theme editor and in its warnings")
        case .cardStale: String(localized: "Too old", bundle: .module, comment: "Name of a colour role in the theme editor and in its warnings")
        case .cardFar: String(localized: "Too far", bundle: .module, comment: "Name of a colour role in the theme editor and in its warnings")
        case .cardUnavailable: String(localized: "Unavailable", bundle: .module, comment: "Name of a colour role in the theme editor and in its warnings")
        case .verdictGo: String(localized: "Go", bundle: .module, comment: "Name of a colour role in the theme editor and in its warnings")
        case .verdictMarginal: String(localized: "Marginal", bundle: .module, comment: "Name of a colour role in the theme editor and in its warnings")
        case .verdictNoGo: String(localized: "No-go", bundle: .module, comment: "Name of a colour role in the theme editor and in its warnings")
        case .verdictUnknown: String(localized: "Unknown", bundle: .module, comment: "Name of a colour role in the theme editor and in its warnings")
        case .alertSevere: String(localized: "Severe and extreme", bundle: .module, comment: "Name of a colour role in the theme editor and in its warnings")
        case .alertModerate: String(localized: "Other alerts", bundle: .module, comment: "Name of a colour role in the theme editor and in its warnings")
        case .fateReleased: String(localized: "Released", bundle: .module, comment: "Name of a colour role in the theme editor and in its warnings")
        case .fateKept: String(localized: "Kept", bundle: .module, comment: "Name of a colour role in the theme editor and in its warnings")
        case .fateKeptForSashimi: String(localized: "Kept for sashimi", bundle: .module, comment: "Name of a colour role in the theme editor and in its warnings")
        case .flowRecordLow: String(localized: "Record low", bundle: .module, comment: "Name of a colour role in the theme editor and in its warnings")
        case .flowMuchBelowNormal: String(localized: "Much below normal", bundle: .module, comment: "Name of a colour role in the theme editor and in its warnings")
        case .flowBelowNormal: String(localized: "Below normal", bundle: .module, comment: "Name of a colour role in the theme editor and in its warnings")
        case .flowNormal: String(localized: "Normal", bundle: .module, comment: "Name of a colour role in the theme editor and in its warnings")
        case .flowAboveNormal: String(localized: "Above normal", bundle: .module, comment: "Name of a colour role in the theme editor and in its warnings")
        case .flowMuchAboveNormal: String(localized: "Much above normal", bundle: .module, comment: "Name of a colour role in the theme editor and in its warnings")
        case .flowRecordHigh: String(localized: "Record high", bundle: .module, comment: "Name of a colour role in the theme editor and in its warnings")
        case .floodAction: String(localized: "Action stage", bundle: .module, comment: "Name of a colour role in the theme editor and in its warnings")
        case .floodMinor: String(localized: "Minor flooding", bundle: .module, comment: "Name of a colour role in the theme editor and in its warnings")
        case .floodModerate: String(localized: "Moderate flooding", bundle: .module, comment: "Name of a colour role in the theme editor and in its warnings")
        case .floodMajor: String(localized: "Major flooding", bundle: .module, comment: "Name of a colour role in the theme editor and in its warnings")
        case .stationNone: String(localized: "No state", bundle: .module, comment: "Name of a colour role in the theme editor and in its warnings")
        case .stationUnknown: String(localized: "State unknown", bundle: .module, comment: "Name of a colour role in the theme editor and in its warnings")
        case .stationNoData: String(localized: "No recent value", bundle: .module, comment: "Name of a colour role in the theme editor and in its warnings")
        case .stationReporting: String(localized: "Reporting", bundle: .module, comment: "Name of a colour role in the theme editor and in its warnings")
        case .stationLow: String(localized: "Low water", bundle: .module, comment: "Name of a colour role in the theme editor and in its warnings")
        case .stationNormal: String(localized: "Normal", bundle: .module, comment: "Name of a colour role in the theme editor and in its warnings")
        case .stationElevated: String(localized: "Action stage", bundle: .module, comment: "Name of a colour role in the theme editor and in its warnings")
        case .stationFloodMinor: String(localized: "Minor flooding", bundle: .module, comment: "Name of a colour role in the theme editor and in its warnings")
        case .stationFloodModerate: String(localized: "Moderate flooding", bundle: .module, comment: "Name of a colour role in the theme editor and in its warnings")
        case .stationFloodMajor: String(localized: "Major flooding", bundle: .module, comment: "Name of a colour role in the theme editor and in its warnings")
        case .tideCurve: String(localized: "Curve", bundle: .module, comment: "Name of a colour role in the theme editor and in its warnings")
        case .tideFill: String(localized: "Fill under the curve", bundle: .module, comment: "Name of a colour role in the theme editor and in its warnings")
        case .tideHigh: String(localized: "High tide", bundle: .module, comment: "Name of a colour role in the theme editor and in its warnings")
        case .tideLow: String(localized: "Low tide", bundle: .module, comment: "Name of a colour role in the theme editor and in its warnings")
        case .tideSlack: String(localized: "Slack water", bundle: .module, comment: "Name of a colour role in the theme editor and in its warnings")
        case .timelineNight: String(localized: "Night", bundle: .module, comment: "Name of a colour role in the theme editor and in its warnings")
        case .timelineSun: String(localized: "Sunrise and sunset", bundle: .module, comment: "Name of a colour role in the theme editor and in its warnings")
        case .timelineSolunarMajor: String(localized: "Solunar major", bundle: .module, comment: "Name of a colour role in the theme editor and in its warnings")
        case .timelineSolunarMinor: String(localized: "Solunar minor", bundle: .module, comment: "Name of a colour role in the theme editor and in its warnings")
        case .timelineWindow: String(localized: "Snack times", bundle: .module, comment: "Name of a colour role in the theme editor and in its warnings")
        case .chartPredicted: String(localized: "Predicted", bundle: .module, comment: "Name of a colour role in the theme editor and in its warnings")
        case .chartObserved: String(localized: "Observed", bundle: .module, comment: "Name of a colour role in the theme editor and in its warnings")
        case .chartForecast: String(localized: "Forecast", bundle: .module, comment: "Name of a colour role in the theme editor and in its warnings")
        case .chartModelled: String(localized: "Modelled", bundle: .module, comment: "Name of a colour role in the theme editor and in its warnings")
        case .chartFloodCategory: String(localized: "Flood stage line", bundle: .module, comment: "Name of a colour role in the theme editor and in its warnings")
        case .chartUserRange: String(localized: "Your range", bundle: .module, comment: "Name of a colour role in the theme editor and in its warnings")
        case .chartThreshold: String(localized: "Threshold", bundle: .module, comment: "Name of a colour role in the theme editor and in its warnings")
        case .chartValueBand: String(localized: "Normal band", bundle: .module, comment: "Name of a colour role in the theme editor and in its warnings")
        case .chartNow: String(localized: "Now", bundle: .module, comment: "Name of a colour role in the theme editor and in its warnings")
        case .spotFavourite: String(localized: "Favourite", bundle: .module, comment: "Name of a colour role in the theme editor and in its warnings")
        case .spotPinned: String(localized: "Pinned", bundle: .module, comment: "Name of a colour role in the theme editor and in its warnings")
        }
    }
}
