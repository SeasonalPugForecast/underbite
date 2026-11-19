import Foundation
import UnderbiteKit

/// What a new pack starts with: the map's view and layers, or a trip's area, days and spots
/// (the trip checklist's "make a pack for this trip"). What is left out is chosen as for the
/// map: the area around the shown spot, the NOAA chart, the newest data, the spots inside.
nonisolated struct PackPrefill: Equatable, Sendable {
    var name: String = ""
    var area: PackBox?
    var layers: [String]?
    var dates: PackDatePolicy?
    var spotIDs: [String]?

    init(
        name: String = "", area: PackBox? = nil, layers: [String]? = nil,
        dates: PackDatePolicy? = nil, spotIDs: [String]? = nil
    ) {
        self.name = name
        self.area = area
        self.layers = layers
        self.dates = dates
        self.spotIDs = spotIDs
    }
}

/// A new pack being made: the choices of the new-pack screen, turned into the core's pack by
/// `pack_new` (never written by hand).
nonisolated struct PackDraft: Equatable, Sendable {
    /// The NOAA chart's layer id: offline it stands in for Apple's base map.
    static let chart = "noaa-chart"
    /// Half the side of the area around a spot, degrees, when nothing else gives one.
    static let spotMargin = 0.04
    /// What an unnamed draft is planned as, for the estimate only (never kept).
    private static let unnamed = "-"

    var name: String
    var area: PackBox?
    /// The index of the zoom choice in the core's presets.
    var zoom: Int
    var layers: [String]
    var dates: PackDatePolicy
    var spotIDs: [String]

    /// A draft from `prefill`, with the core's default zoom choice; an area around `around`
    /// (the shown spot) when the prefill has none, and the spots inside the area when it names
    /// none.
    init(
        prefill: PackPrefill, defaultZoom: Int,
        spots: [(id: String, latitude: Double, longitude: Double)],
        around: (latitude: Double, longitude: Double)? = nil
    ) {
        var area = prefill.area
        if area == nil, let around {
            let m = Self.spotMargin
            area = PackBox(
                west: around.longitude - m, south: around.latitude - m,
                east: around.longitude + m, north: around.latitude + m)
        }
        name = prefill.name
        self.area = area
        zoom = defaultZoom
        layers = prefill.layers ?? [Self.chart]
        dates = prefill.dates ?? .latest
        spotIDs =
            prefill.spotIDs
            ?? spots.filter { spot in
                area?.contains(latitude: spot.latitude, longitude: spot.longitude) ?? false
            }.map(\.id)
    }

    /// The core's pack JSON for the draft under `id` at `now`, with the zooms of `presets`;
    /// the core's words when it cannot be made. For the estimate an unnamed draft is planned
    /// under a stand-in name.
    func json(
        id: String, presets: [PackZoomPreset], settingsJSON: String, now: Int64,
        forEstimate: Bool = false
    ) -> Result<String, PackPlanProblem> {
        guard let area, presets.indices.contains(zoom) else {
            return .failure(PackPlanProblem(message: ""))
        }
        let preset = presets[zoom]
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        do {
            return .success(
                try packNew(
                    id: id, name: forEstimate && trimmed.isEmpty ? Self.unnamed : name,
                    area: area, minZoom: preset.minZoom, maxZoom: preset.maxZoom,
                    layers: layers, dates: dates, spotIds: spotIDs, settingsJson: settingsJSON,
                    now: now))
        } catch {
            return .failure(PackPlanProblem(message: PackErrors.message(of: error)))
        }
    }

    /// `date` as the core's day (`YYYY-MM-DD`) in `calendar`'s zone.
    static func day(_ date: Date, calendar: Calendar) -> String {
        let c = calendar.dateComponents([.year, .month, .day], from: date)
        return String(format: "%04d-%02d-%02d", c.year ?? 0, c.month ?? 0, c.day ?? 0)
    }
}
