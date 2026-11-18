import Foundation

/// The part of the world the depth layers fetch elevation for: the view on screen with a margin
/// on each side, at the view's zoom. One area is one request (the core's `dem_area_request`),
/// whose cells are about a screen point at that zoom; the shading and the contours both draw
/// from it. A view the area holds, at the same zoom step, needs nothing new: a small pan or a
/// change of the contour interval draws again from what the store has.
nonisolated struct DepthArea: Equatable, Hashable, Sendable {
    var west: Double
    var south: Double
    var east: Double
    var north: Double
    /// The Web Mercator zoom it was asked for at.
    var zoom: Double

    /// The share of the view's width and height added on each side.
    static let margin = 0.15

    /// `region` with `margin` of its width and height added on each side, clipped to the world
    /// the elevation model covers.
    static func around(_ region: MapRegion, margin: Double = margin) -> DepthArea {
        let dx = (region.east - region.west) * margin
        let dy = (region.north - region.south) * margin
        return DepthArea(
            west: max(region.west - dx, -180), south: max(region.south - dy, -85),
            east: min(region.east + dx, 180), north: min(region.north + dy, 85),
            zoom: region.zoom)
    }

    /// The zoom step an area serves: whole Web Mercator levels.
    static func step(_ zoom: Double) -> Int { Int(zoom.rounded(.down)) }

    /// Whether the area holds the whole of `region`, at the zoom step it was asked for.
    func covers(_ region: MapRegion) -> Bool {
        Self.step(region.zoom) == Self.step(zoom)
            && region.west >= west && region.east <= east
            && region.south >= south && region.north <= north
    }
}

/// The areas fetched lately, newest last: the first place to look for a view's elevation.
nonisolated struct DepthAreas: Equatable, Sendable {
    /// The most areas remembered; the store may have dropped older ones (it asks for them again).
    static let capacity = 8

    private(set) var areas: [DepthArea] = []

    /// The newest area that covers `region`.
    func covering(_ region: MapRegion) -> DepthArea? {
        areas.last { $0.covers(region) }
    }

    /// `area` is in the store: it becomes the newest.
    mutating func hold(_ area: DepthArea) {
        areas.removeAll { $0 == area }
        areas.append(area)
        if areas.count > Self.capacity { areas.removeFirst(areas.count - Self.capacity) }
    }

    /// The store no longer has `area`.
    mutating func forget(_ area: DepthArea) {
        areas.removeAll { $0 == area }
    }
}
