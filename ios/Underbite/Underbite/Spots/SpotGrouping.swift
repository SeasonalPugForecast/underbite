import Foundation

/// One block of the spot list: the pinned spots, an area, or the spots with no area.
nonisolated struct SpotSection: Identifiable, Equatable, Sendable {
    enum Kind: Equatable, Hashable, Sendable {
        case pinned
        case area(String)
        case unassigned
    }

    var kind: Kind
    var spots: [Spot]

    var id: String {
        switch kind {
        case .pinned: "pinned"
        case .area(let id): "area-" + id
        case .unassigned: "unassigned"
        }
    }
}

/// How the list is grouped and reordered. Pure: the views and the tests share it.
nonisolated enum SpotGrouping {
    /// The pinned spots first, then each area in the user's order (empty ones too, so they can be
    /// removed), then the spots without an area. A pinned spot is listed once, under Pinned.
    static func sections(spots: [Spot], areas: [Area]) -> [SpotSection] {
        let order = Spot.arranged(spots)
        var result: [SpotSection] = []
        let pinned = order.filter(\.pinned)
        if !pinned.isEmpty { result.append(SpotSection(kind: .pinned, spots: pinned)) }
        let rest = order.filter { !$0.pinned }
        let known = Set(areas.map(\.id))
        for area in arrangedAreas(areas) {
            result.append(
                SpotSection(kind: .area(area.id), spots: rest.filter { $0.areaID == area.id }))
        }
        let loose = rest.filter { $0.areaID.map { !known.contains($0) } ?? true }
        if !loose.isEmpty { result.append(SpotSection(kind: .unassigned, spots: loose)) }
        return result
    }

    /// The areas in the user's order, then by name.
    static func arrangedAreas(_ areas: [Area]) -> [Area] {
        areas.sorted { a, b in
            if a.order != b.order { return a.order < b.order }
            if a.name != b.name { return a.name < b.name }
            return a.id < b.id
        }
    }

    /// The whole list's ids after moving rows `source` of one section to `destination` (the
    /// offsets a list's move gives: `destination` counts before the removal). The section's
    /// rows keep their places among the others, so spots of other sections do not shift.
    static func reordered(
        all: [String], section: [String], moving source: IndexSet, to destination: Int
    ) -> [String] {
        var items = section
        let offsets = source.filter { items.indices.contains($0) }.sorted()
        guard !offsets.isEmpty else { return all }
        let moving = offsets.map { items[$0] }
        let before = offsets.filter { $0 < destination }.count
        for offset in offsets.reversed() { items.remove(at: offset) }
        let at = min(max(destination - before, 0), items.count)
        items.insert(contentsOf: moving, at: at)
        let inSection = Set(section)
        var next = items.makeIterator()
        return all.map { inSection.contains($0) ? (next.next() ?? $0) : $0 }
    }
}

/// Coordinates typed by hand: "40.7033, -74.0170" (a comma, a semicolon or spaces between).
nonisolated enum TypedCoordinates {
    static func parse(_ text: String) -> (latitude: Double, longitude: Double)? {
        let parts =
            text
            .split { ",; \t\n".contains($0) }
            .map { Double($0) }
        guard parts.count == 2, let lat = parts[0], let lon = parts[1] else { return nil }
        guard lat.isFinite, lon.isFinite, (-90...90).contains(lat), (-180...180).contains(lon)
        else { return nil }
        return (lat, lon)
    }
}
