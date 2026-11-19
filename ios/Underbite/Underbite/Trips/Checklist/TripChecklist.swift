import Foundation
@preconcurrency import UnderbiteKit

/// What the checklist changes of a trip, written onto the trip's own JSON: the ticks and words
/// of its items, and the region of each place (the region names the official pages the core
/// links). Everything else in the trip passes through as it was; the store saves the result
/// through the core, so what is kept is a trip it reads.
nonisolated enum TripChecklistEdit {
    /// A place of the trip as the region choice shows it.
    struct Place: Equatable, Sendable, Identifiable {
        /// The place's key: its spot id.
        var id: String
        var name: String
        /// ISO 3166-2 (`US-NY`); empty when none is set.
        var region: String
    }

    /// The places of the trip `json`, in its order.
    static func places(_ json: String) -> [Place] {
        (object(json)["places"] as? [[String: Any]] ?? []).compactMap { place in
            guard let id = place["spot_id"] as? String else { return nil }
            return Place(
                id: id, name: place["name"] as? String ?? "",
                region: place["region"] as? String ?? "")
        }
    }

    /// The trip with the item `id` ticked or not; `nil` when it has no such item.
    static func ticking(_ json: String, item id: String, done: Bool) -> String? {
        editItem(json, id) { $0["done"] = done }
    }

    /// The trip with the item `id` reading `text`; `nil` when it has no such item or the text
    /// is blank.
    static func renaming(_ json: String, item id: String, to text: String) -> String? {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }
        return editItem(json, id) { $0["text"] = trimmed }
    }

    /// The trip with a new item `text` under `id`, not ticked, after the others; `nil` when the
    /// text is blank or the id taken.
    static func adding(_ json: String, item text: String, id: String) -> String? {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }
        var trip = object(json)
        var items = trip["checklist"] as? [[String: Any]] ?? []
        guard !items.contains(where: { $0["id"] as? String == id }) else { return nil }
        items.append(["id": id, "text": trimmed, "done": false])
        trip["checklist"] = items
        return try? TripStore.text(of: trip)
    }

    /// The trip without the item `id`; `nil` when it has no such item.
    static func removing(_ json: String, item id: String) -> String? {
        var trip = object(json)
        var items = trip["checklist"] as? [[String: Any]] ?? []
        guard let index = items.firstIndex(where: { $0["id"] as? String == id }) else { return nil }
        items.remove(at: index)
        trip["checklist"] = items
        return try? TripStore.text(of: trip)
    }

    /// The trip with the place `id` in `region`, in capitals and without spaces around it; a
    /// blank region takes the place's away. `nil` when the trip has no such place or nothing
    /// changes.
    static func settingRegion(_ json: String, place id: String, to region: String) -> String? {
        let code = region.trimmingCharacters(in: .whitespacesAndNewlines).uppercased()
        var trip = object(json)
        var places = trip["places"] as? [[String: Any]] ?? []
        guard let index = places.firstIndex(where: { $0["spot_id"] as? String == id }) else {
            return nil
        }
        guard (places[index]["region"] as? String ?? "") != code else { return nil }
        if code.isEmpty {
            places[index].removeValue(forKey: "region")
        } else {
            places[index]["region"] = code
        }
        trip["places"] = places
        return try? TripStore.text(of: trip)
    }

    private static func editItem(
        _ json: String, _ id: String, _ edit: (inout [String: Any]) -> Void
    ) -> String? {
        var trip = object(json)
        var items = trip["checklist"] as? [[String: Any]] ?? []
        guard let index = items.firstIndex(where: { $0["id"] as? String == id }) else { return nil }
        edit(&items[index])
        trip["checklist"] = items
        return try? TripStore.text(of: trip)
    }

    private static func object(_ json: String) -> [String: Any] {
        (try? JSONSerialization.jsonObject(with: Data(json.utf8))) as? [String: Any] ?? [:]
    }
}

/// Which offline pack each trip made: the trip remembers only the pack's id, the pack itself
/// belongs to the packs' store. Kept in `trip-packs.json` beside the app's other files; a
/// file that cannot be read is set aside and the links start again.
nonisolated struct TripPackLinks: Codable, Equatable, Sendable {
    static let name = "trip-packs.json"
    static let currentVersion = 1

    var version = TripPackLinks.currentVersion
    /// The pack id, by trip id.
    var packs: [String: String] = [:]

    init() {}

    static func load(_ files: AppFiles) -> TripPackLinks {
        do {
            return try files.decode(TripPackLinks.self, name) ?? TripPackLinks()
        } catch {
            _ = files.setAside(name)
            return TripPackLinks()
        }
    }

    @discardableResult
    func save(_ files: AppFiles) -> Bool {
        files.write(self, Self.name)
    }

    /// The pack of the trip `trip` among `records`; none when it has none or its pack was
    /// deleted.
    func pack(of trip: String, in records: [PackRecord]) -> PackRecord? {
        guard let id = packs[trip] else { return nil }
        return records.first { $0.id == id }
    }

    /// Links that point at a trip or a pack that is gone left out.
    func pruned(trips: Set<String>, packs records: [PackRecord]) -> TripPackLinks {
        let ids = Set(records.map(\.id))
        var out = self
        out.packs = packs.filter { trips.contains($0.key) && ids.contains($0.value) }
        return out
    }
}

/// The trip's side of its offline pack: what the core is told of it and what a new one starts
/// with.
nonisolated enum TripPackLink {
    /// The pack as the trip view words it: ready once a download has completed (its bytes on
    /// the phone and when it was last refreshed), else not downloaded.
    static func appStatus(_ record: PackRecord?, storedBytes: Int64?) -> TripAppStatus {
        guard let saved = record?.info?.refreshedAt else {
            return TripAppStatus(pack: .notDownloaded, sharing: .notShared)
        }
        return TripAppStatus(
            pack: .ready(bytes: UInt64(max(0, storedBytes ?? 0)), savedAt: saved),
            sharing: .notShared)
    }

    /// A new pack for a trip named `name`: the box around its places with coordinates (with
    /// the margin a spot's own area has), its first `maxDays` days, `layers`, and its places
    /// that are saved spots. A trip whose places have no coordinates leaves the area to the
    /// new-pack screen.
    static func prefill(
        name: String, places: [TripSpot], saved: Set<String>, days: [String],
        layers: [String], maxDays: UInt32
    ) -> PackPrefill {
        let points = places.compactMap(\.spot)
        var area: PackBox?
        if let first = points.first {
            let m = PackDraft.spotMargin
            var box = PackBox(
                west: first.longitude, south: first.latitude, east: first.longitude,
                north: first.latitude)
            for p in points.dropFirst() {
                box.west = min(box.west, p.longitude)
                box.east = max(box.east, p.longitude)
                box.south = min(box.south, p.latitude)
                box.north = max(box.north, p.latitude)
            }
            area = PackBox(
                west: box.west - m, south: max(-90, box.south - m), east: box.east + m,
                north: min(90, box.north + m))
        }
        let kept = Array(days.prefix(Int(max(1, maxDays))))
        let spots = places.map(\.key).filter { saved.contains($0) }
        return PackPrefill(
            name: name, area: area, layers: layers,
            dates: kept.isEmpty ? nil : .days(dates: kept),
            spotIDs: spots.isEmpty ? nil : spots)
    }

    /// The layers a trip's pack starts with: the NOAA chart (Apple's base map is not kept
    /// offline), then the overlays `catalogue` shows.
    static func layers(_ catalogue: [LayerInfo]) -> [String] {
        let shown = catalogue.filter { $0.shown && !$0.isBase }.map(\.id)
        return [PackDraft.chart] + shown.filter { $0 != PackDraft.chart }
    }
}
