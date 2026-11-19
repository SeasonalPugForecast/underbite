import Foundation
import UnderbiteKit

/// What the preference pages do with the settings, without any view: edits that keep the
/// settings valid, and the bridge between the core's canonical values and the fields the
/// settings hold. Foundation only, so the package tests compile it too.
///
/// Swift converts and formats nothing. Each field is entered, shown and stepped through the
/// core in the unit the settings store it in (`parseSettingValue`, `formatSettingValue`,
/// `settingStops`), and durations come from `formatDuration`.
nonisolated enum Preferences {
    /// The settings could not take an edit.
    struct Rejected: Error, Equatable, LocalizedError {
        var message: String
        var errorDescription: String? { message }
    }

    /// `json` with `edit` applied, or the reason it is not valid settings (the core's
    /// `validateSettings` finds an error). The settings are never written half edited.
    static func applying(_ json: String, _ edit: (inout Settings) -> Void) throws -> String {
        var settings = try Settings(json: json)
        edit(&settings)
        let edited = try settings.json()
        if let issue = validateSettings(json: edited).first(where: { $0.severity == .error }) {
            throw Rejected(message: issue.path.isEmpty ? issue.message : "\(issue.path): \(issue.message)")
        }
        return edited
    }

    // MARK: - Units

    /// The unit a group of the units screen holds, `nil` when it follows another group.
    static func unit(_ group: String, in units: Settings.Units) -> String? {
        switch group {
        case "discharge": units.discharge
        case "height": units.height
        case "speed": units.speed
        case "water_temp": units.waterTemp
        case "pressure": units.pressure
        case "distance": units.distance
        case "air_temp": units.airTemp
        case "stage": units.stage
        case "lake_level": units.lakeLevel
        case "tide": units.tide
        case "waves": units.waves
        case "current": units.current
        case "wind": units.wind
        default: nil
        }
    }

    /// Chooses `unit` for `group`; `nil` makes a group that follows another follow it again
    /// (a base group ignores `nil`). Only the unit changes: ranges and limits stay as stored.
    static func setUnit(_ unit: String?, for group: String, in units: inout Settings.Units) {
        switch group {
        case "discharge": if let unit { units.discharge = unit }
        case "height": if let unit { units.height = unit }
        case "speed": if let unit { units.speed = unit }
        case "water_temp": if let unit { units.waterTemp = unit }
        case "pressure": if let unit { units.pressure = unit }
        case "distance": if let unit { units.distance = unit }
        case "air_temp": units.airTemp = unit
        case "stage": units.stage = unit
        case "lake_level": units.lakeLevel = unit
        case "tide": units.tide = unit
        case "waves": units.waves = unit
        case "current": units.current = unit
        case "wind": units.wind = unit
        default: break
        }
    }

    // MARK: - Limits

    /// A value of a limit profile.
    enum LimitField: String, CaseIterable, Identifiable {
        case wind, gust, wave, current, airTemp, visibility
        var id: String { rawValue }

        var title: String {
            switch self {
            case .wind: "Wind up to"
            case .gust: "Gusts up to"
            case .wave: "Waves up to"
            case .current: "Current up to"
            case .airTemp: "Air no colder than"
            case .visibility: "Visibility at least"
            }
        }

        /// The measurement the core's value entry knows it as.
        var measurement: Quantity {
            switch self {
            case .wind: .init(field: "limits[].max_wind_kn", low: 0, high: 40)
            case .gust: .init(field: "limits[].max_gust_kn", low: 0, high: 60)
            case .wave: .init(field: "limits[].max_wave_m", low: 0, high: 5)
            case .current: .init(field: "limits[].max_current_kn", low: 0, high: 5)
            case .airTemp: .init(field: "limits[].min_air_temp_c", low: -10, high: 40)
            case .visibility: .init(field: "limits[].min_visibility_km", low: 0, high: 20)
            }
        }

        func value(of limit: Settings.Limit) -> Double? {
            switch self {
            case .wind: limit.maxWindKn
            case .gust: limit.maxGustKn
            case .wave: limit.maxWaveM
            case .current: limit.maxCurrentKn
            case .airTemp: limit.minAirTempC
            case .visibility: limit.minVisibilityKm
            }
        }

        func set(_ value: Double?, on limit: inout Settings.Limit) {
            switch self {
            case .wind: limit.maxWindKn = value
            case .gust: limit.maxGustKn = value
            case .wave: limit.maxWaveM = value
            case .current: limit.maxCurrentKn = value
            case .airTemp: limit.minAirTempC = value
            case .visibility: limit.minVisibilityKm = value
            }
        }
    }

    /// A settings field with a value to enter (the core's field key, such as
    /// `limits[].max_wind_kn`) and the span a slider covers, in the field's stored unit.
    struct Quantity: Equatable {
        var field: String
        var low: Double
        var high: Double
    }

    /// The name to give a profile after its name field was submitted: the trimmed draft, or
    /// `nil` when there is nothing to change (empty, or the name it already has). An empty
    /// draft is never a name, so the field goes back to the stored one.
    static func renamed(draft: String, current: String) -> String? {
        let name = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        return name.isEmpty || name == current ? nil : name
    }

    /// A name for a new profile that no other profile has.
    static func newLimitName(existing: [Settings.Limit]) -> String {
        let taken = Set(existing.map(\.name))
        if !taken.contains("New profile") { return "New profile" }
        var n = 2
        while taken.contains("New profile \(n)") { n += 1 }
        return "New profile \(n)"
    }

    /// An empty profile: safety flags on, no numeric limits until the user sets them.
    static func newLimit(existing: [Settings.Limit]) -> Settings.Limit {
        decoded(
            Settings.Limit.self,
            ["name": newLimitName(existing: existing), "no_small_craft_advisory": true,
                "no_lightning": true])
    }

    /// A range for a gauge with no values yet.
    static func newRiver(site: String) -> Settings.RiverRange {
        decoded(Settings.RiverRange.self, ["site": site])
    }

    /// The settings types have no public initializers: a new one is read from JSON, as the
    /// core writes it.
    private static func decoded<T: Decodable>(_ type: T.Type, _ object: [String: Any]) -> T {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        // INVARIANT: the objects built here carry every field the types require.
        return try! decoder.decode(
            type, from: JSONSerialization.data(withJSONObject: object))
    }

    // MARK: - River ranges

    /// The id the settings key a gauge by (`usgs:01420500`) from what was typed: the digits
    /// alone or with the `usgs:` prefix. `nil` when it is not a gauge number.
    static func riverSite(from text: String) -> String? {
        var body = text.trimmingCharacters(in: .whitespaces)
        if body.lowercased().hasPrefix("usgs:") { body = String(body.dropFirst(5)) }
        guard (8...15).contains(body.count), body.allSatisfy({ $0.isASCII && $0.isNumber }) else {
            return nil
        }
        return "usgs:" + body
    }

    /// The spans of the river ranges, in cubic feet per second.
    static let bestFlow = Quantity(field: "rivers[].best_cfs", low: 0, high: 5000)
    static let flow = Quantity(field: "rivers[].wadeable_below_cfs", low: 0, high: 5000)
    static let floodFlow = Quantity(field: "rivers[].blown_out_above_cfs", low: 0, high: 20000)
    static let troutStress = Quantity(field: "thresholds.trout_stress_c", low: 5, high: 30)
    static func distance(of kind: SourceKind) -> Quantity {
        Quantity(field: "sources.\(kind.fieldGroup).max_distance_km", low: 0, high: 200)
    }

    /// The age field of a source group, in minutes.
    static func ageField(of kind: SourceKind) -> String {
        "sources.\(kind.fieldGroup).max_age_minutes"
    }

    /// The default trout stress threshold of the core, in degrees Celsius.
    static let defaultTroutStressC: Double = {
        // INVARIANT: the core's default settings always decode.
        try! Settings.defaults().thresholds.troutStressC
    }()

    // MARK: - Source limits

    /// The source groups with a freshness and a distance limit, in the order they are listed.
    enum SourceKind: String, CaseIterable, Identifiable {
        case waterTemperature, tide, currents, flow, lakeLevel, waves, sst, clarity
        case waterQuality, marineForecast, weather, storm
        var id: String { rawValue }

        /// The group's name in the settings (`water_temperature`), as the field keys use it.
        var fieldGroup: String {
            var name = ""
            for character in rawValue {
                if character.isUppercase { name += "_" }
                name += character.lowercased()
            }
            return name
        }

        /// The group's name in the user's voice, from the core.
        func title(settingsJSON: String) -> String {
            let titles = (try? sourceGroupTitles(settingsJson: settingsJSON)) ?? []
            return titles.first { $0.id == fieldGroup }?.title ?? fieldGroup
        }

        func group(in sources: Settings.Sources) -> Settings.SourceGroup {
            switch self {
            case .waterTemperature: sources.waterTemperature
            case .tide: sources.tide
            case .currents: sources.currents
            case .flow: sources.flow
            case .lakeLevel: sources.lakeLevel
            case .waves: sources.waves
            case .sst: sources.sst
            case .clarity: sources.clarity
            case .waterQuality: sources.waterQuality
            case .marineForecast: sources.marineForecast
            case .weather: sources.weather
            case .storm: sources.storm
            }
        }

        func set(_ group: Settings.SourceGroup, in sources: inout Settings.Sources) {
            switch self {
            case .waterTemperature: sources.waterTemperature = group
            case .tide: sources.tide = group
            case .currents: sources.currents = group
            case .flow: sources.flow = group
            case .lakeLevel: sources.lakeLevel = group
            case .waves: sources.waves = group
            case .sst: sources.sst = group
            case .clarity: sources.clarity = group
            case .waterQuality: sources.waterQuality = group
            case .marineForecast: sources.marineForecast = group
            case .weather: sources.weather = group
            case .storm: sources.storm = group
            }
        }
    }

    /// How old a reading may be, in minutes; the choices of the freshness picker.
    static let freshnessChoices = [15, 30, 60, 120, 180, 360, 720, 1440]

    // MARK: - Values through the core

    /// A stop of a slider: the value as the settings field holds it, and the core's label.
    struct Stop: Equatable {
        var value: Double
        var label: String
    }

    /// Typing, showing and stepping one settings field through the core: values are in the
    /// field's stored unit, labels and typed text in the user's unit.
    struct Entry {
        let settingsJSON: String
        let quantity: Quantity

        init(_ quantity: Quantity, settingsJSON: String) {
            self.settingsJSON = settingsJSON
            self.quantity = quantity
        }

        /// What was typed (a decimal comma, a unit symbol) as the field's value; the core reads it.
        func field(fromTyped text: String) throws -> Double {
            try parseSettingValue(field: quantity.field, text: text, settingsJson: settingsJSON)
        }

        /// The value as the core writes it in the user's unit.
        func label(of value: Double) -> String {
            (try? formatSettingValue(field: quantity.field, value: value, settingsJson: settingsJSON))
                ?? ""
        }

        /// Round stops across `low...high`, labelled by the core.
        func stops(low: Double, high: Double) throws -> [Stop] {
            try settingStops(
                field: quantity.field, min: low, max: high, settingsJson: settingsJSON
            ).map { Stop(value: $0.value, label: $0.label) }
        }

        func stops() throws -> [Stop] { try stops(low: quantity.low, high: quantity.high) }
    }

    /// A time span in minutes as the core writes it ("2 h").
    static func duration(minutes: Int, settingsJSON: String) -> String {
        (try? formatDuration(seconds: Int64(minutes) * 60, settingsJson: settingsJSON)) ?? ""
    }

    /// The words of a failed settings call: the core's message for a value it cannot read, the
    /// reason an edit was refused, otherwise the error's own description.
    static func message(for error: Error) -> String {
        if let rejected = error as? Rejected { return rejected.message }
        if let found = firstString(in: error) { return found }
        return error.localizedDescription
    }

    /// The first string inside an error value (the payload of the core's enum errors).
    private static func firstString(in value: Any) -> String? {
        if let text = value as? String { return text }
        for child in Mirror(reflecting: value).children {
            if let found = firstString(in: child.value) { return found }
        }
        return nil
    }

    /// The index of the stop nearest `value`; `nil` with no stops.
    static func nearest(_ stops: [Stop], to value: Double) -> Int? {
        stops.indices.min { abs(stops[$0].value - value) < abs(stops[$1].value - value) }
    }

    /// The stops around a stored value, so its own label is there when it is a round one:
    /// the slider's span widened to hold it.
    static func stopsHolding(_ value: Double?, entry: Entry) throws -> [Stop] {
        var low = entry.quantity.low
        var high = entry.quantity.high
        if let value {
            low = min(low, value)
            high = max(high, value)
        }
        return try entry.stops(low: low, high: high)
    }
}
