import Foundation

/// An alert rule as the core reads and writes it (`[[alerts]]` in the settings, the `rule_json`
/// of `alert_rules`): what the editor sends to `put_alert_rule`. Optional fields that are `nil`
/// are left out of the JSON, as the core requires (it refuses unknown and misplaced keys).
/// Thresholds are in the core's canonical units; the core alone converts them.
nonisolated struct AlertRuleDocument: Codable, Equatable, Sendable {
    var id: String
    var name: String
    var enabled: Bool
    var target: Target
    var allOf: [Condition]
    /// Hours before the rule may notify again while it holds; `nil` waits until it stops.
    var repeatAfterHours: Int?
    var quietHours: QuietHours?
    /// Whether notifications name the place, the rule and the station.
    var showName: Bool

    /// `spot` and `trip` carry `id`; `box` carries its four sides in degrees.
    struct Target: Codable, Equatable, Sendable {
        var kind: String
        var id: String?
        var south: Double?
        var west: Double?
        var north: Double?
        var east: Double?

        static func spot(_ id: String) -> Target { Target(kind: "spot", id: id) }
        static func trip(_ id: String) -> Target { Target(kind: "trip", id: id) }
        static func box(south: Double, west: Double, north: Double, east: Double) -> Target {
            Target(kind: "box", south: south, west: west, north: north, east: east)
        }
    }

    /// One condition by `kind`: `measurement` (`metric`, `below`, `above`), `marine_alert`
    /// (`event`), `window_starts` (`within_minutes`), `trip_go` and `storm_ahead`.
    struct Condition: Codable, Equatable, Sendable {
        var kind: String
        var metric: String?
        var below: Double?
        var above: Double?
        var event: String?
        var withinMinutes: Int?

        private enum CodingKeys: String, CodingKey {
            case kind, metric, below, above, event
            case withinMinutes = "within_minutes"
        }
    }

    /// Minutes after midnight in the target's zone; crosses midnight when start is after end.
    struct QuietHours: Codable, Equatable, Sendable {
        var startMinute: Int
        var endMinute: Int

        private enum CodingKeys: String, CodingKey {
            case startMinute = "start_minute"
            case endMinute = "end_minute"
        }
    }

    private enum CodingKeys: String, CodingKey {
        case id, name, enabled, target
        case allOf = "all_of"
        case repeatAfterHours = "repeat_after_hours"
        case quietHours = "quiet_hours"
        case showName = "show_name"
    }

    init(
        id: String, name: String, enabled: Bool = true, target: Target, allOf: [Condition],
        repeatAfterHours: Int? = nil, quietHours: QuietHours? = nil, showName: Bool = true
    ) {
        self.id = id
        self.name = name
        self.enabled = enabled
        self.target = target
        self.allOf = allOf
        self.repeatAfterHours = repeatAfterHours
        self.quietHours = quietHours
        self.showName = showName
    }

    /// Rules written before `enabled` or `show_name` were always written take the core's
    /// defaults (both on).
    init(from decoder: any Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        name = try c.decode(String.self, forKey: .name)
        enabled = try c.decodeIfPresent(Bool.self, forKey: .enabled) ?? true
        target = try c.decode(Target.self, forKey: .target)
        allOf = try c.decode([Condition].self, forKey: .allOf)
        repeatAfterHours = try c.decodeIfPresent(Int.self, forKey: .repeatAfterHours)
        quietHours = try c.decodeIfPresent(QuietHours.self, forKey: .quietHours)
        showName = try c.decodeIfPresent(Bool.self, forKey: .showName) ?? true
    }

    /// Reads the core's rule JSON.
    init(json: String) throws {
        self = try JSONDecoder().decode(AlertRuleDocument.self, from: Data(json.utf8))
    }

    /// The rule `id` of the settings JSON, if there is one that can be read.
    static func find(_ id: String, inSettings settingsJSON: String) -> AlertRuleDocument? {
        guard
            let settings = try? JSONSerialization.jsonObject(with: Data(settingsJSON.utf8))
                as? [String: Any],
            let rules = settings["alerts"] as? [Any]
        else { return nil }
        for rule in rules {
            guard let object = rule as? [String: Any], object["id"] as? String == id,
                let data = try? JSONSerialization.data(withJSONObject: object)
            else { continue }
            return try? JSONDecoder().decode(AlertRuleDocument.self, from: data)
        }
        return nil
    }

    /// The JSON the core reads.
    func json() throws -> String {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        return String(decoding: try encoder.encode(self), as: UTF8.self)
    }
}
