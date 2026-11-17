import Foundation

/// The settings document (`underbite_config::schema::Config`) as the core reads and writes it:
/// every field is here, so decoding and encoding again drops nothing. The core stays the
/// authority on what is valid and on defaults: hand edited JSON goes through
/// `migrateSettings` and `validateSettings` before it is kept.
///
/// Ids and unit names (`"ft"`, `"kn"`, `"coops"`, `"water_temp"`, `"12h"`, ...) stay strings;
/// the catalogues the core offers (`unitOptions`, `cardCatalogue`, `sourceCatalogue`) say which
/// are valid. Optional fields that are `nil` are left out of the JSON, as the core does.
public struct Settings: Codable, Sendable, Hashable {
    public var version: Int
    public var units: Units
    public var sources: Sources
    public var providers: Providers
    public var cards: Cards
    /// Go/no-go profiles.
    public var limits: [Limit]
    public var thresholds: Thresholds
    public var storms: Storms
    /// The user's own flow ranges, by gauge.
    public var rivers: [RiverRange]
    public var windows: Windows
    public var presentation: Presentation
    public var map: MapSettings

    /// Decodes the core's settings JSON.
    public init(json: String) throws {
        self = try Self.decoder().decode(Settings.self, from: Data(json.utf8))
    }

    /// The core's default settings.
    public static func defaults() throws -> Settings {
        try Settings(json: defaultSettingsJson())
    }

    /// The JSON the core reads (snake_case keys, `nil` fields left out).
    public func json() throws -> String {
        String(decoding: try Self.encoder().encode(self), as: UTF8.self)
    }

    static func decoder() -> JSONDecoder {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return decoder
    }

    static func encoder() -> JSONEncoder {
        let encoder = JSONEncoder()
        encoder.keyEncodingStrategy = .convertToSnakeCase
        encoder.outputFormatting = [.sortedKeys]
        return encoder
    }
}

extension Settings {
    /// The unit for each kind of value. `waterTemp`, `height`, `speed`, `discharge`, `pressure`
    /// and `distance` always have one; the others, when `nil`, follow the group they derive from
    /// (the air temperature follows `waterTemp`, stage, lake level, tide and waves follow
    /// `height`, current and wind follow `speed`).
    public struct Units: Codable, Sendable, Hashable {
        public var discharge: String
        public var height: String
        public var speed: String
        public var waterTemp: String
        public var airTemp: String?
        public var stage: String?
        public var lakeLevel: String?
        public var tide: String?
        public var waves: String?
        public var current: String?
        public var wind: String?
        public var pressure: String
        public var distance: String
    }

    /// An ordered chain of sources: the first that answers wins, the rest fill gaps.
    public struct SourceGroup: Codable, Sendable, Hashable {
        public var chain: [String]
        public var maxDistanceKm: Double?
        public var maxAgeMinutes: Int?
    }

    public struct Sources: Codable, Sendable, Hashable {
        /// Decimal places of the coordinates sent to data providers.
        public var coordinatePrecision: Int
        public var waterTemperature: SourceGroup
        public var tide: SourceGroup
        public var currents: SourceGroup
        public var flow: SourceGroup
        public var lakeLevel: SourceGroup
        public var waves: SourceGroup
        public var sst: SourceGroup
        public var clarity: SourceGroup
        public var waterQuality: SourceGroup
        public var marineForecast: SourceGroup
        public var weather: SourceGroup
        public var storm: SourceGroup
    }

    /// Provider options. Keys never live here: they are in the Keychain.
    public struct Providers: Codable, Sendable, Hashable {
        public var usgs: Usgs
        public var copernicus: Copernicus

        public struct Usgs: Codable, Sendable, Hashable {
            public init() {}
        }

        public struct Copernicus: Codable, Sendable, Hashable {
            public var enabled: Bool
        }
    }

    /// Card order and hidden cards, with an optional layout per water type. A water type
    /// without a layout of its own uses the base one (or the layout the core ships for it).
    public struct Cards: Codable, Sendable, Hashable {
        public var order: [String]
        public var hidden: [String]
        public var river: CardLayout?
        public var lake: CardLayout?
        public var tidal: CardLayout?
        public var bay: CardLayout?
        public var surf: CardLayout?
        public var nearshore: CardLayout?
        public var offshore: CardLayout?
        public var greatLakes: CardLayout?
        public var pond: CardLayout?
        public var ice: CardLayout?
    }

    /// An order and the cards hidden from it. An empty `order` in a water type's layout means
    /// the base order.
    public struct CardLayout: Codable, Sendable, Hashable {
        public var order: [String]
        public var hidden: [String]

        public init(order: [String] = [], hidden: [String] = []) {
            self.order = order
            self.hidden = hidden
        }
    }

    /// A go/no-go profile. Limits are in the core's canonical units (knots and metres, as the
    /// field names say); the editor converts through the core's `parseUserValue` and
    /// `displayStops`.
    public struct Limit: Codable, Sendable, Hashable {
        public var name: String
        public var maxWindKn: Double?
        public var maxGustKn: Double?
        public var maxWaveM: Double?
        public var maxCurrentKn: Double?
        public var minAirTempC: Double?
        public var minVisibilityKm: Double?
        public var noSmallCraftAdvisory: Bool
        public var noLightning: Bool
    }

    public struct Thresholds: Codable, Sendable, Hashable {
        /// Water temperature above which trout fishing is discouraged, in degrees Celsius.
        public var troutStressC: Double
    }

    public struct Storms: Codable, Sendable, Hashable {
        public var hurricaneDistanceKm: Double
    }

    /// The user's ranges for one gauge, as discharge in cubic feet per second.
    public struct RiverRange: Codable, Sendable, Hashable {
        /// `usgs:` and the site number.
        public var site: String
        public var wadeableBelowCfs: Double?
        /// Low and high.
        public var bestCfs: [Double]?
        public var blownOutAboveCfs: Double?
    }

    public struct Windows: Codable, Sendable, Hashable {
        public var factors: WindowFactors
        /// The tidal current speed range, knots `[min, max]`, the current-strength factor looks for.
        public var currentRangeKn: [Double]
        public var showSolunar: Bool
        public var solunarMajorMinutes: Int
        public var solunarMinorMinutes: Int
    }

    /// How much each factor weighs in the fishing windows, from 0 (off) to 1.
    public struct WindowFactors: Codable, Sendable, Hashable {
        public var tideStage: Double
        public var lowLight: Double
        public var solunar: Double
        public var pressureFalling: Double
        public var waterTemp: Double
        public var currentStrength: Double
        public var flowInRange: Double
    }

    /// `voice` is `luna` or `plain`; `clock` is `12h` or `24h`.
    public struct Presentation: Codable, Sendable, Hashable {
        public var voice: String
        public var clock: String
        public var timelineHours: Int
        public var timelineStepMinutes: Int
    }

    /// The map section. Ranges and depths are in the core's canonical units (degrees Celsius,
    /// metres, m/s); a `nil` opacity, ramp or range means the layer's own.
    public struct MapSettings: Codable, Sendable, Hashable {
        /// The base map id.
        public var base: String
        /// The overlay layers in drawing order, first at the bottom.
        public var layers: [MapLayer]
        /// The user's own presets.
        public var presets: [MapPreset]
        /// A built-in preset id or the name of one of the user's presets.
        public var activePreset: String
        /// `auto`, `on` or `off`.
        public var batteryMode: String
        public var depth: MapDepth
        public var breaks: MapBreaks
        public var offlineMaxMb: Int
    }

    public struct MapLayer: Codable, Sendable, Hashable {
        public var id: String
        public var shown: Bool
        public var opacity: Double?
        public var ramp: String?
        /// Low and high.
        public var range: [Double]?
    }

    public struct MapPreset: Codable, Sendable, Hashable {
        public var name: String
        public var layers: [MapLayer]
    }

    public struct MapDepth: Codable, Sendable, Hashable {
        /// Shallow and deep, in metres.
        public var bandM: [Double]?
        public var contourIntervalM: Double
        public var ramp: String?
    }

    public struct MapBreaks: Codable, Sendable, Hashable {
        public var sstCPerKm: Double
        public var chlorophyllLog10PerKm: Double
    }
}
