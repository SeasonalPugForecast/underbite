import Foundation
import UnderbiteKit

/// A place the user fishes. The coordinates are exact and stay on the device: the core rounds
/// what leaves it. The package tests compile this file too.
nonisolated struct Spot: Codable, Hashable, Identifiable, Sendable {
    var id: String
    var name: String
    var latitude: Double
    var longitude: Double
    /// As the core writes it (`"river_stream"`, `"tidal_river_estuary"`, ...).
    var waterType: String
    /// Whether the user chose the water type; until then a fetch may suggest one.
    var waterTypeChosen: Bool
    /// The IANA zone, empty until known.
    var timeZone: String
    var areaID: String?
    var pinned: Bool
    var favourite: Bool
    /// The place in the user's own order, among spots of the same standing.
    var order: Int

    init(
        id: String = UUID().uuidString, name: String, latitude: Double, longitude: Double,
        waterType: String, waterTypeChosen: Bool = true, timeZone: String = "",
        areaID: String? = nil, pinned: Bool = false, favourite: Bool = false, order: Int = 0
    ) {
        self.id = id
        self.name = name
        self.latitude = latitude
        self.longitude = longitude
        self.waterType = waterType
        self.waterTypeChosen = waterTypeChosen
        self.timeZone = timeZone
        self.areaID = areaID
        self.pinned = pinned
        self.favourite = favourite
        self.order = order
    }

    private enum CodingKeys: String, CodingKey {
        case id, name, latitude, longitude, waterType, waterTypeChosen, timeZone, areaID, pinned
        case favourite, order
    }

    /// The water type of a spot saved without one: a fetch suggests the right one, since it
    /// counts as not chosen.
    static let unknownWaterType = "river_stream"

    /// Reads a saved spot; only its position is required. A key a file lacks (an older
    /// version's, or one edited by hand) takes its default, so one odd spot never makes the
    /// whole file unreadable.
    init(from decoder: any Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        latitude = try c.decode(Double.self, forKey: .latitude)
        longitude = try c.decode(Double.self, forKey: .longitude)
        id = (try? c.decodeIfPresent(String.self, forKey: .id)) ?? UUID().uuidString
        name = (try? c.decodeIfPresent(String.self, forKey: .name)) ?? ""
        let waterType = try? c.decodeIfPresent(String.self, forKey: .waterType)
        self.waterType = waterType ?? Self.unknownWaterType
        waterTypeChosen =
            waterType == nil ? false : (try? c.decodeIfPresent(Bool.self, forKey: .waterTypeChosen)) ?? true
        timeZone = (try? c.decodeIfPresent(String.self, forKey: .timeZone)) ?? ""
        areaID = try? c.decodeIfPresent(String.self, forKey: .areaID)
        pinned = (try? c.decodeIfPresent(Bool.self, forKey: .pinned)) ?? false
        favourite = (try? c.decodeIfPresent(Bool.self, forKey: .favourite)) ?? false
        order = (try? c.decodeIfPresent(Int.self, forKey: .order)) ?? 0
    }

    /// The spot as the core takes it.
    var info: SpotInfo {
        SpotInfo(
            id: id, name: name, latitude: latitude, longitude: longitude, waterType: waterType,
            timeZone: timeZone)
    }

    /// `spots` in the order they are listed: pinned first, then the user's order, then name.
    static func arranged(_ spots: [Spot]) -> [Spot] {
        spots.sorted { a, b in
            if a.pinned != b.pinned { return a.pinned }
            if a.order != b.order { return a.order < b.order }
            if a.name != b.name { return a.name < b.name }
            return a.id < b.id
        }
    }
}

/// A group of spots ("Montauk", "Upper Delaware").
nonisolated struct Area: Codable, Hashable, Identifiable, Sendable {
    var id: String = UUID().uuidString
    var name: String
    var order: Int = 0
}

/// The water type a fetch suggested for a spot, with the core's reasons.
nonisolated struct WaterTypeHint: Codable, Hashable, Sendable {
    var waterType: String
    var reasons: [String]
}

/// What the app keeps for one spot between launches. Never holds a key.
nonisolated struct SpotCache: Codable, Equatable, Sendable {
    /// The last fetch's snapshot, as the core wrote it (the input of `view_model`).
    var snapshotJSON: String?
    /// When a fetch last succeeded, Unix seconds.
    var lastSuccess: Int64?
    /// Why the last fetch failed, if it did.
    var error: String?
    /// The water type the last fetch inferred.
    var suggestion: WaterTypeHint?
    /// The stations around the spot, as `discover_stations` returned them.
    var stationsJSON: String?
}
