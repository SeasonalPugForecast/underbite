import Foundation

/// What the core's station discovery found for a spot, as the rows of the choices screen:
/// per measurement the stations in the order the core ranked them, each with its pin and
/// exclusion. The core wrote the ranking; this only reads the parts the screen shows.
nonisolated struct StationListing: Equatable, Sendable {
    struct Row: Identifiable, Equatable, Sendable {
        /// The station as the core's pin calls take it: `coops:8518750`, `coops:NYH1927:13`.
        var key: String
        var sourceID: String
        /// The station's id as its source writes it (`8518750`, `USGS-01375000`).
        var stationID: String
        /// The current bin of a CO-OPS current station.
        var bin: Int?
        var name: String
        var pinned: Bool
        var excluded: Bool

        var id: String { key }
    }

    struct Group: Identifiable, Equatable, Sendable {
        var measurement: String
        var title: String
        var rows: [Row]

        var id: String { measurement }
    }

    var groups: [Group]

    var isEmpty: Bool { groups.allSatisfy { $0.rows.isEmpty } }

    /// Reads `discover_stations`'s JSON; nothing readable gives no groups.
    init(json: String?) {
        guard let json, let data = json.data(using: .utf8),
            let decoded = try? JSONDecoder().decode([String: Ranking].self, from: data)
        else {
            groups = []
            return
        }
        groups = decoded.map { measurement, ranking in
            var rows = ranking.stations.map {
                Self.row($0.station, pinned: $0.pinned, excluded: false)
            }
            for dropped in ranking.dropped where dropped.reason.reason == "excluded" {
                rows.append(Self.row(dropped.station, pinned: false, excluded: true))
            }
            return Group(
                measurement: measurement, title: StationMeasurement.title(measurement), rows: rows)
        }
        .filter { !$0.rows.isEmpty }
        .sorted {
            StationMeasurement.rank($0.measurement) < StationMeasurement.rank($1.measurement)
        }
    }

    private static func row(_ station: Ranking.Station, pinned: Bool, excluded: Bool) -> Row {
        var key = "\(station.source):\(station.id)"
        if let bin = station.bin { key += ":\(bin)" }
        return Row(
            key: key, sourceID: station.source, stationID: station.id, bin: station.bin,
            name: station.name,
            pinned: pinned, excluded: excluded)
    }

    private struct Ranking: Decodable {
        var stations: [Kept]
        var dropped: [Dropped]

        struct Station: Decodable {
            var source: String
            var id: String
            var name: String
            var bin: Int?
        }

        struct Kept: Decodable {
            var station: Station
            var pinned: Bool
        }

        struct Dropped: Decodable {
            var station: Station
            var reason: Reason
        }

        struct Reason: Decodable {
            var reason: String
        }
    }
}

/// What a switch on a station's row asks the core to remember.
nonisolated enum StationChoice: Equatable, Sendable {
    case pin, exclude, clear

    /// The choice once the pin switch is set to `on` (off forgets the pin).
    static func pinSwitched(on: Bool) -> StationChoice { on ? .pin : .clear }
    /// The choice once the exclude switch is set to `on` (off forgets the exclusion).
    static func excludeSwitched(on: Bool) -> StationChoice { on ? .exclude : .clear }
}

/// The measurements' titles, in the order the screen lists them: the words the core gives them.
nonisolated enum StationMeasurement {
    private static let known: [(id: String, title: String)] = [
        ("tide", "Tide"),
        ("tidal_current", "Tidal current"),
        ("water_temperature", "Water temperature"),
        ("discharge", "River flow"),
        ("gage_height", "Gage height"),
        ("lake_level", "Lake level"),
        ("turbidity", "Turbidity"),
        ("clarity", "Water clarity"),
        ("dissolved_oxygen", "Dissolved oxygen"),
        ("conductivity", "Conductivity"),
        ("salinity", "Salinity"),
        ("ph", "pH"),
        ("chlorophyll", "Chlorophyll"),
        ("cyanobacteria", "Cyanobacteria"),
        ("ocean_current", "Ocean current"),
        ("waves", "Waves"),
        ("waves_forecast", "Wave forecast"),
        ("sea_surface_temp", "Sea surface temperature"),
        ("sea_surface_temp_forecast", "Sea surface temperature forecast"),
        ("marine_forecast", "Marine forecast"),
        ("marine_alerts", "Marine alerts"),
        ("flood_categories", "Flood categories"),
        ("flow_forecast", "River forecast"),
        ("observed_wind", "Wind"),
        ("observed_pressure", "Pressure"),
    ]

    static func title(_ id: String) -> String {
        known.first { $0.id == id }?.title
            ?? id.replacingOccurrences(of: "_", with: " ").capitalized
    }

    static func rank(_ id: String) -> Int {
        known.firstIndex { $0.id == id } ?? known.count
    }
}
