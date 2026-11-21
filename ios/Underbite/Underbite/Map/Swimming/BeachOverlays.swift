import MapKit
import UIKit

/// Every beach forecast zone as one overlay with one renderer. Each zone keeps its path, made
/// the first time it is drawn; the server has already simplified the outlines.
nonisolated final class BeachOverlay: NSObject, MKOverlay {
    let coordinate = CLLocationCoordinate2D(latitude: 0, longitude: 0)
    let boundingMapRect = MKMapRect.world
    private let lock = NSLock()
    private var zones: [Drawn] = []
    private var picked: String?

    /// A zone with its path, relative to its bounds' origin.
    nonisolated final class Drawn: @unchecked Sendable {
        let shape: BeachShape
        private let lock = NSLock()
        private var made: CGPath?

        init(_ shape: BeachShape) { self.shape = shape }

        /// The path of every ring, in map points from the bounds' origin.
        var path: CGPath {
            if let held = lock.withLock({ made }) { return held }
            let origin = shape.bounds.origin
            let path = CGMutablePath()
            for polygon in shape.polygons {
                for ring in polygon {
                    guard let first = ring.first else { continue }
                    path.move(to: CGPoint(x: first.x - origin.x, y: first.y - origin.y))
                    for point in ring.dropFirst() {
                        path.addLine(to: CGPoint(x: point.x - origin.x, y: point.y - origin.y))
                    }
                    path.closeSubpath()
                }
            }
            lock.withLock { made = path }
            return path
        }
    }

    /// Replaces the zones and the one picked out; the renderer is told to draw again by its owner.
    func set(_ shapes: [BeachShape], picked: String?) {
        let drawn = shapes.map(Drawn.init)
        lock.withLock {
            zones = drawn
            self.picked = picked
        }
    }

    /// The zones whose bounds meet `rect`, and the id of the one picked out.
    func zones(in rect: MKMapRect) -> (zones: [Drawn], picked: String?) {
        lock.withLock { (zones.filter { $0.shape.bounds.intersects(rect) }, picked) }
    }
}

/// Draws a ``BeachOverlay``: a zone with a category is filled lightly in its colour with an edge
/// in it, one with no forecast issued only has a dashed grey edge. The zone picked out has a
/// heavier edge. Line widths stay the same on screen at every zoom.
nonisolated final class BeachRenderer: MKOverlayRenderer {
    private let beaches: BeachOverlay

    init(_ overlay: BeachOverlay) {
        beaches = overlay
        super.init(overlay: overlay)
    }

    static func colour(_ rgb: UInt32) -> UIColor {
        UIColor(
            red: CGFloat((rgb >> 16) & 0xFF) / 255, green: CGFloat((rgb >> 8) & 0xFF) / 255,
            blue: CGFloat(rgb & 0xFF) / 255, alpha: 1)
    }

    override func draw(_ mapRect: MKMapRect, zoomScale: MKZoomScale, in context: CGContext) {
        let perPoint = 1 / Double(zoomScale)
        let margin = 6 * perPoint
        let found = beaches.zones(in: mapRect.insetBy(dx: -margin, dy: -margin))
        for item in found.zones {
            let shape = item.shape
            let colour = Self.colour(shape.role.rgb)
            let isPicked = shape.id == found.picked
            let origin = point(for: shape.bounds.origin)
            context.saveGState()
            context.translateBy(x: origin.x, y: origin.y)
            context.setLineJoin(.round)
            if shape.role.filled {
                context.addPath(item.path)
                context.setFillColor(colour.withAlphaComponent(shape.role.fillAlpha).cgColor)
                context.fillPath(using: .evenOdd)
            }
            if isPicked || !shape.role.filled {
                context.addPath(item.path)
                context.setStrokeColor(UIColor.white.withAlphaComponent(0.85).cgColor)
                context.setLineWidth(CGFloat((isPicked ? 6 : 4) * perPoint))
                context.strokePath()
            }
            context.addPath(item.path)
            context.setStrokeColor(colour.withAlphaComponent(shape.role.filled ? 0.95 : 0.9).cgColor)
            context.setLineWidth(CGFloat((isPicked ? 4 : 2.5) * perPoint))
            if !shape.role.filled {
                context.setLineDash(
                    phase: 0, lengths: [CGFloat(5 * perPoint), CGFloat(4 * perPoint)])
            }
            context.strokePath()
            context.restoreGState()
        }
    }
}
