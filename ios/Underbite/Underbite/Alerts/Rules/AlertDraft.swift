import Foundation
@preconcurrency import UnderbiteKit

/// A rule as the editor holds it: thresholds are the text typed, in the user's units. Every
/// conversion and every check is the core's: `parse_alert_threshold` reads the text,
/// `alert_threshold_text` writes a saved threshold back, and `put_alert_rule` refuses what
/// cannot be saved, with the field's path and message.
nonisolated struct AlertDraft: Equatable, Sendable {
    /// What the rule watches.
    enum TargetKind: String, CaseIterable, Sendable {
        case spot, trip, box
    }

    /// An area on the map, in degrees.
    struct Box: Equatable, Sendable {
        var south: Double
        var west: Double
        var north: Double
        var east: Double
    }

    /// One condition being edited.
    struct ConditionDraft: Equatable, Identifiable, Sendable {
        enum Kind: String, CaseIterable, Sendable {
            case measurement
            case marineAlert = "marine_alert"
            case windowStarts = "window_starts"
            case stormAhead = "storm_ahead"
            case tripGo = "trip_go"
        }

        var id = UUID()
        var kind: Kind
        /// The metric as rules write it (`"discharge"`).
        var metric = "discharge"
        /// The thresholds as typed, in the unit `alert_metrics` names; blank for none.
        var below = ""
        var above = ""
        /// An NWS event name, matched as a prefix.
        var event = "Small Craft Advisory"
        var withinMinutes = 60
    }

    /// Where a problem shows in the editor.
    enum Field: Hashable, Sendable {
        case name, target, conditions, repeatHours, quietHours, other
        case condition(UUID)
        case below(UUID)
        case above(UUID)
    }

    var id: String
    var name: String
    var enabled: Bool
    var targetKind: TargetKind
    var spotID: String?
    var tripID: String?
    var box: Box?
    var conditions: [ConditionDraft]
    /// Hours before notifying again while the rule holds; `nil` waits until it stops.
    var repeatHours: Int?
    /// Quiet hours in minutes after midnight; `nil` for none.
    var quietStart: Int?
    var quietEnd: Int
    var showName: Bool

    /// The usual repeat when one is turned on, and quiet hours from 10 PM to 6 AM.
    static let defaultRepeatHours = 12
    static let defaultQuietStart = 22 * 60
    static let defaultQuietEnd = 6 * 60

    /// A new rule on `spotID` with one river-flow condition.
    static func new(id: String = UUID().uuidString, spotID: String?) -> AlertDraft {
        AlertDraft(
            id: id, name: "", enabled: true, targetKind: .spot, spotID: spotID, tripID: nil,
            box: nil, conditions: [ConditionDraft(kind: .measurement)], repeatHours: nil,
            quietStart: nil, quietEnd: defaultQuietEnd, showName: true)
    }

    /// A saved rule, its thresholds written in the settings' units.
    init(document: AlertRuleDocument, settingsJSON: String) {
        id = document.id
        name = document.name
        enabled = document.enabled
        targetKind = TargetKind(rawValue: document.target.kind) ?? .spot
        spotID = targetKind == .spot ? document.target.id : nil
        tripID = targetKind == .trip ? document.target.id : nil
        if let s = document.target.south, let w = document.target.west,
            let n = document.target.north, let e = document.target.east
        {
            box = Box(south: s, west: w, north: n, east: e)
        }
        conditions = document.allOf.map { c in
            var draft = ConditionDraft(kind: ConditionDraft.Kind(rawValue: c.kind) ?? .measurement)
            if let metric = c.metric { draft.metric = metric }
            let metric = draft.metric
            func text(_ value: Double?) -> String {
                guard let value else { return "" }
                return
                    (try? alertThresholdText(metric: metric, value: value, settingsJson: settingsJSON))
                    ?? ""
            }
            draft.below = text(c.below)
            draft.above = text(c.above)
            if let event = c.event { draft.event = event }
            if let minutes = c.withinMinutes { draft.withinMinutes = minutes }
            return draft
        }
        repeatHours = document.repeatAfterHours
        quietStart = document.quietHours?.startMinute
        quietEnd = document.quietHours?.endMinute ?? Self.defaultQuietEnd
        showName = document.showName
    }

    private init(
        id: String, name: String, enabled: Bool, targetKind: TargetKind, spotID: String?,
        tripID: String?, box: Box?, conditions: [ConditionDraft], repeatHours: Int?,
        quietStart: Int?, quietEnd: Int, showName: Bool
    ) {
        self.id = id
        self.name = name
        self.enabled = enabled
        self.targetKind = targetKind
        self.spotID = spotID
        self.tripID = tripID
        self.box = box
        self.conditions = conditions
        self.repeatHours = repeatHours
        self.quietStart = quietStart
        self.quietEnd = quietEnd
        self.showName = showName
    }

    /// The kinds of condition a target can have: a box watches gridded values only, and only
    /// a trip has a go/no-go.
    static func kinds(for target: TargetKind) -> [ConditionDraft.Kind] {
        switch target {
        case .spot: [.measurement, .marineAlert, .windowStarts, .stormAhead]
        case .trip: [.measurement, .marineAlert, .windowStarts, .stormAhead, .tripGo]
        case .box: [.measurement]
        }
    }

    /// The core's target: an unset spot, trip or box is an empty id, which the core refuses
    /// with its own words.
    var target: AlertRuleDocument.Target {
        switch targetKind {
        case .spot: .spot(spotID ?? "")
        case .trip: .trip(tripID ?? "")
        case .box:
            if let box {
                .box(south: box.south, west: box.west, north: box.north, east: box.east)
            } else {
                // Refused by the core ("south below north"); the editor asks for an area first.
                .box(south: 0, west: 0, north: 0, east: 0)
            }
        }
    }

    /// The rule the draft stands for, or the threshold fields the core cannot read with their
    /// messages (in the user's units and voice).
    func document(settingsJSON: String) -> (AlertRuleDocument?, [Field: String]) {
        var problems: [Field: String] = [:]
        var all: [AlertRuleDocument.Condition] = []
        for c in conditions {
            var out = AlertRuleDocument.Condition(kind: c.kind.rawValue)
            switch c.kind {
            case .measurement:
                out.metric = c.metric
                out.below = Self.parse(c.below, c.metric, settingsJSON, .below(c.id), &problems)
                out.above = Self.parse(c.above, c.metric, settingsJSON, .above(c.id), &problems)
            case .marineAlert:
                out.event = c.event.trimmingCharacters(in: .whitespacesAndNewlines)
            case .windowStarts:
                out.withinMinutes = c.withinMinutes
            case .stormAhead, .tripGo:
                break
            }
            all.append(out)
        }
        guard problems.isEmpty else { return (nil, problems) }
        let document = AlertRuleDocument(
            id: id, name: name.trimmingCharacters(in: .whitespacesAndNewlines), enabled: enabled,
            target: target, allOf: all, repeatAfterHours: repeatHours,
            quietHours: quietStart.map {
                AlertRuleDocument.QuietHours(startMinute: $0, endMinute: quietEnd)
            }, showName: showName)
        return (document, [:])
    }

    private static func parse(
        _ text: String, _ metric: String, _ settingsJSON: String, _ field: Field,
        _ problems: inout [Field: String]
    ) -> Double? {
        do {
            return try parseAlertThreshold(metric: metric, text: text, settingsJson: settingsJSON)
        } catch {
            problems[field] = AlertWords.message(of: error)
            return nil
        }
    }

    /// The settings with the draft saved in them, or what stops it, by field, in the core's
    /// words.
    func save(into settingsJSON: String) -> Result<String, Problems> {
        let (document, problems) = document(settingsJSON: settingsJSON)
        guard let document else { return .failure(Problems(byField: problems)) }
        do {
            return .success(
                try putAlertRule(settingsJson: settingsJSON, ruleJson: try document.json()))
        } catch AlertError.Invalid(let what, let message) {
            return .failure(Problems(byField: [field(of: what): message]))
        } catch {
            return .failure(Problems(byField: [.other: AlertWords.message(of: error)]))
        }
    }

    /// The editor's field for the core's path of a refused rule (`alerts[2].all_of[1].above`).
    func field(of path: String) -> Field {
        guard let dot = path.firstIndex(of: "."), path.hasPrefix("alerts[") else { return .other }
        let rest = path[path.index(after: dot)...]
        if rest.hasPrefix("name") { return .name }
        if rest.hasPrefix("target") { return .target }
        if rest.hasPrefix("repeat_after_hours") { return .repeatHours }
        if rest.hasPrefix("quiet_hours") { return .quietHours }
        if rest.hasPrefix("all_of[") {
            let digits = rest.dropFirst("all_of[".count).prefix { $0.isNumber }
            if let index = Int(digits), conditions.indices.contains(index) {
                let id = conditions[index].id
                if rest.hasSuffix(".above") { return .above(id) }
                if rest.hasSuffix(".below") { return .below(id) }
                return .condition(id)
            }
        }
        if rest.hasPrefix("all_of") { return .conditions }
        return .other
    }

    /// What stops a draft from being saved.
    struct Problems: Error, Equatable, Sendable {
        var byField: [Field: String]
    }
}

/// The core's message in an error of the alerts calls.
nonisolated enum AlertWords {
    static func message(of error: any Error) -> String {
        switch error {
        case AlertError.Settings(let message): message
        case AlertError.Invalid(_, let message): message
        case AlertError.Threshold(let message): message
        case FetchError.Settings(let message): message
        case FetchError.Invalid(_, let message): message
        default: error.localizedDescription
        }
    }
}
