import Foundation

/// What the trip editor changes of a trip, read from the trip's JSON and written back onto it:
/// the parts of a trip it does not edit (the checklist, a shared place's fuzz and region, keys
/// a later core adds) pass through as they were. It is the form's state, not a model of the
/// trip: the trip stays the core's JSON, and the core checks it (`trip_problem`).
nonisolated struct TripDraft: Equatable, Sendable {
    /// Empty for a new trip: the store gives it its id.
    var id = ""
    var name = ""
    /// The spot ids of the trip's places, in the trip's order.
    var placeIDs: [String] = []
    /// The first and last day, `YYYY-MM-DD`.
    var first = ""
    var last = ""
    /// The species ids as typed, one or more separated by commas.
    var species = ""
    /// The core's name of the method (`shore`, `kayak`, ...).
    var method = TripDraft.defaultMethod
    /// The name of the limits profile the trip carries a copy of; `nil` for none, when the
    /// settings' first profile judges its days.
    var limits: String?
    var notes = ""
    /// The trip's JSON as it was read, which the edits are written onto.
    var base = "{}"

    /// The methods a trip may have, in the core's names.
    static let methods = ["shore", "wade", "kayak", "boat", "ice"]
    static let defaultMethod = "shore"

    /// The places of the trip as it was read, by spot id: kept as they are while chosen.
    private var basePlaces: [String: [String: Any]] {
        Self.places(in: Self.object(base))
    }

    /// A new trip: `days` days from `first`, at `places`.
    init(first: String, days: Int = 3, places: [String] = []) {
        self.first = first
        last = TripDates.adding(max(days, 1) - 1, to: first) ?? first
        placeIDs = places
    }

    /// The draft of the trip `json`.
    init(json: String) {
        let trip = Self.object(json)
        base = json
        id = trip["id"] as? String ?? ""
        name = trip["name"] as? String ?? ""
        placeIDs = (trip["places"] as? [[String: Any]] ?? []).compactMap { $0["spot_id"] as? String }
        first = trip["first"] as? String ?? ""
        last = trip["last"] as? String ?? ""
        species = (trip["species"] as? [String] ?? []).joined(separator: ", ")
        method = trip["method"] as? String ?? Self.defaultMethod
        limits = (trip["limits"] as? [String: Any])?["name"] as? String
        notes = trip["notes"] as? String ?? ""
    }

    /// The places of the trip as read that are not among `spots` (a trip someone shared),
    /// by spot id, with their names: the editor lists them beside the saved spots.
    func otherPlaces(besides spots: [Spot]) -> [(id: String, name: String)] {
        let saved = Set(spots.map(\.id))
        let places = Self.object(base)["places"] as? [[String: Any]] ?? []
        return places.compactMap { place in
            guard let id = place["spot_id"] as? String, !saved.contains(id) else { return nil }
            return (id, place["name"] as? String ?? "")
        }
    }

    /// Adds the place `id` at the end, or takes it out.
    mutating func toggle(_ id: String) {
        if let index = placeIDs.firstIndex(of: id) {
            placeIDs.remove(at: index)
        } else {
            placeIDs.append(id)
        }
    }

    /// The species ids of `species`: each name separated by commas, semicolons or new lines,
    /// in lower case with every run of other characters a hyphen ("Morone saxatilis" is
    /// `morone-saxatilis`), each once.
    static func speciesIDs(_ text: String) -> [String] {
        var seen = Set<String>()
        return text.split(whereSeparator: { ",;\n".contains($0) }).compactMap { name in
            var id = ""
            var gap = false
            for char in name.lowercased() {
                if char.isASCII, char.isLetter || char.isNumber {
                    if gap, !id.isEmpty { id.append("-") }
                    id.append(char)
                    gap = false
                } else {
                    gap = true
                }
            }
            guard !id.isEmpty, seen.insert(id).inserted else { return nil }
            return id
        }
    }

    /// The trip as JSON: the edits written onto the trip as read. A place keeps what the trip
    /// had of it; a place newly chosen is the saved spot, its area by name, at its exact
    /// position (sharing reduces it). The chosen limits are a copy of that profile from
    /// `settingsJSON`; the copy the trip had is kept while the same name is chosen.
    func json(spots: [Spot], areas: [Area], settingsJSON: String) -> String {
        var trip = Self.object(base)
        // Empty for a new trip: the core reads it, and the store replaces it.
        trip["id"] = id
        trip["name"] = name.trimmingCharacters(in: .whitespacesAndNewlines)
        let kept = basePlaces
        trip["places"] = placeIDs.compactMap { id -> [String: Any]? in
            if let place = kept[id] { return place }
            guard let spot = spots.first(where: { $0.id == id }) else { return nil }
            return Self.place(spot, area: areas.first { $0.id == spot.areaID }?.name)
        }
        trip["first"] = first
        trip["last"] = last
        trip["species"] = Self.speciesIDs(species)
        trip["method"] = method
        let had = trip["limits"] as? [String: Any]
        if let limits {
            if had?["name"] as? String != limits,
                let profile = Self.profiles(settingsJSON).first(where: { $0["name"] as? String == limits })
            {
                trip["limits"] = profile
            }
        } else {
            trip.removeValue(forKey: "limits")
        }
        trip["notes"] = notes
        if trip["checklist"] == nil { trip["checklist"] = [Any]() }
        // A new trip's time is set when it is saved; the core needs one to read it before.
        if trip["modified_at"] == nil { trip["modified_at"] = 0 }
        return (try? TripStore.text(of: trip)) ?? "{}"
    }

    /// The names of the limits profiles in `settingsJSON`, in their order.
    static func profileNames(_ settingsJSON: String) -> [String] {
        profiles(settingsJSON).compactMap { $0["name"] as? String }
    }

    private static func profiles(_ settingsJSON: String) -> [[String: Any]] {
        object(settingsJSON)["limits"] as? [[String: Any]] ?? []
    }

    /// A saved spot as a trip's place.
    private static func place(_ spot: Spot, area: String?) -> [String: Any] {
        var place: [String: Any] = [
            "spot_id": spot.id, "name": spot.name, "latitude": spot.latitude,
            "longitude": spot.longitude, "time_zone": spot.timeZone, "water_type": spot.waterType,
            "fuzz": "exact",
        ]
        if let area, !area.isEmpty { place["area"] = area }
        return place
    }

    private static func places(in trip: [String: Any]) -> [String: [String: Any]] {
        var byID: [String: [String: Any]] = [:]
        for place in trip["places"] as? [[String: Any]] ?? [] {
            if let id = place["spot_id"] as? String, byID[id] == nil { byID[id] = place }
        }
        return byID
    }

    private static func object(_ json: String) -> [String: Any] {
        (try? JSONSerialization.jsonObject(with: Data(json.utf8))) as? [String: Any] ?? [:]
    }
}
