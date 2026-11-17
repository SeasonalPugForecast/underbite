import MapKit
import UIKit
import UnderbiteKit

/// A current layer's arrows as one overlay with one renderer, however many arrows there are (one
/// overlay or annotation per arrow is too slow on older phones). The arrows come from the core's
/// `GridStore.arrows` for the region on screen, thinned and coloured by the core; the renderer only
/// places them: rotated toward the water's heading, as long as the speed's place in the range.
nonisolated final class ArrowOverlay: NSObject, MKOverlay {
    /// One arrow in map points.
    struct Arrow {
        let at: MKMapPoint
        /// Clockwise from north, radians.
        let heading: Double
        /// 0 to 1.
        let fraction: Double
        let colour: CGColor
    }

    let layer: String
    let coordinate = CLLocationCoordinate2D(latitude: 0, longitude: 0)
    let boundingMapRect = MKMapRect.world
    private let lock = NSLock()
    private var arrows: [Arrow] = []

    init(layer: String) {
        self.layer = layer
    }

    /// Replaces the arrows; the renderer is told to draw again by its owner.
    func set(_ arrows: [MapArrow]) {
        let placed = arrows.map { arrow in
            Arrow(
                at: MKMapPoint(CLLocationCoordinate2D(latitude: arrow.lat, longitude: arrow.lon)),
                heading: Double(arrow.towardDeg) * .pi / 180,
                fraction: min(max(Double(arrow.fraction), 0), 1),
                colour: UIColor(
                    red: CGFloat(arrow.colour.red) / 255, green: CGFloat(arrow.colour.green) / 255,
                    blue: CGFloat(arrow.colour.blue) / 255, alpha: CGFloat(arrow.colour.alpha) / 255
                ).cgColor)
        }
        lock.withLock { self.arrows = placed }
    }

    /// The arrows whose base lies in `rect`.
    func arrows(in rect: MKMapRect) -> [Arrow] {
        lock.withLock { arrows.filter { rect.contains($0.at) } }
    }
}

/// Draws an ``ArrowOverlay``: a shaft and a head per arrow, a fixed size on screen whatever the
/// zoom, in the core's colour for the speed.
nonisolated final class ArrowRenderer: MKOverlayRenderer {
    /// The shortest and longest arrow, in points.
    static let shortest = 8.0
    static let longest = 28.0

    private let arrows: ArrowOverlay

    init(_ overlay: ArrowOverlay) {
        arrows = overlay
        super.init(overlay: overlay)
    }

    override func draw(_ mapRect: MKMapRect, zoomScale: MKZoomScale, in context: CGContext) {
        let perPoint = 1 / Double(zoomScale)
        // Arrows starting just outside the tile reach into it.
        let margin = Self.longest * perPoint
        let area = mapRect.insetBy(dx: -margin, dy: -margin)
        context.setLineWidth(CGFloat(2 * perPoint))
        context.setLineCap(.round)
        context.setLineJoin(.round)
        for arrow in arrows.arrows(in: area) {
            let length = (Self.shortest + (Self.longest - Self.shortest) * arrow.fraction) * perPoint
            let head = min(length * 0.4, 7 * perPoint)
            // North is up the map: map points grow southward.
            let dx = sin(arrow.heading), dy = -cos(arrow.heading)
            let tail = point(for: arrow.at)
            let tip = CGPoint(x: tail.x + CGFloat(dx * length), y: tail.y + CGFloat(dy * length))
            let back = (x: -dx * head, y: -dy * head)
            let side = (x: -dy * head * 0.6, y: dx * head * 0.6)
            context.setStrokeColor(arrow.colour)
            context.move(to: tail)
            context.addLine(to: tip)
            context.move(to: CGPoint(x: tip.x + CGFloat(back.x + side.x), y: tip.y + CGFloat(back.y + side.y)))
            context.addLine(to: tip)
            context.addLine(to: CGPoint(x: tip.x + CGFloat(back.x - side.x), y: tip.y + CGFloat(back.y - side.y)))
            context.strokePath()
        }
    }
}
