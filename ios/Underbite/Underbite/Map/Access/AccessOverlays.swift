import MapKit
import UIKit

/// Every public area and line of the access layers as one overlay with one renderer, however
/// many there are. Each shape keeps a path per zoom bucket, thinned for it, made the first time
/// that bucket is drawn.
nonisolated final class AccessShapeOverlay: NSObject, MKOverlay {
    let coordinate = CLLocationCoordinate2D(latitude: 0, longitude: 0)
    let boundingMapRect = MKMapRect.world
    private let lock = NSLock()
    private var shapes: [Drawn] = []

    /// A shape with its opacity and its paths by zoom bucket, relative to its bounds' origin.
    nonisolated final class Drawn: @unchecked Sendable {
        let shape: AccessShape2D
        let opacity: Double
        private let lock = NSLock()
        private var paths: [Int: CGPath] = [:]

        init(_ shape: AccessShape2D, opacity: Double) {
            self.shape = shape
            self.opacity = opacity
        }

        /// The shape's path for a zoom bucket, in map points from its bounds' origin; nil when
        /// nothing is left of it at that zoom.
        func path(bucket: Int) -> CGPath? {
            if let held = lock.withLock({ paths[bucket] }) { return held.isEmpty ? nil : held }
            let tolerance = AccessThinning.tolerance(bucket: bucket)
            let origin = shape.bounds.origin
            let closed = shape.form == .area
            let path = CGMutablePath()
            for ring in shape.rings {
                let thin = AccessThinning.thin(ring, tolerance: tolerance, closed: closed)
                guard let first = thin.first else { continue }
                path.move(to: CGPoint(x: first.x - origin.x, y: first.y - origin.y))
                for point in thin.dropFirst() {
                    path.addLine(to: CGPoint(x: point.x - origin.x, y: point.y - origin.y))
                }
                if closed { path.closeSubpath() }
            }
            lock.withLock { paths[bucket] = path }
            return path.isEmpty ? nil : path
        }
    }

    /// Replaces the shapes; the renderer is told to draw again by its owner.
    func set(_ shapes: [AccessShape2D], opacity: [String: Double]) {
        let drawn = shapes.map { Drawn($0, opacity: opacity[$0.layer] ?? 1) }
        lock.withLock { self.shapes = drawn }
    }

    /// The shapes whose bounds meet `rect`.
    func shapes(in rect: MKMapRect) -> [Drawn] {
        lock.withLock { shapes.filter { $0.shape.bounds.intersects(rect) } }
    }
}

/// Draws an ``AccessShapeOverlay``: areas lightly filled in their access colour with an edge in
/// it (a dashed grey edge for an unknown access), lines in it. Line widths stay the same on
/// screen at every zoom.
nonisolated final class AccessShapeRenderer: MKOverlayRenderer {
    private let shapes: AccessShapeOverlay

    init(_ overlay: AccessShapeOverlay) {
        shapes = overlay
        super.init(overlay: overlay)
    }

    override func draw(_ mapRect: MKMapRect, zoomScale: MKZoomScale, in context: CGContext) {
        let perPoint = 1 / Double(zoomScale)
        let margin = 4 * perPoint
        let drawn = shapes.shapes(in: mapRect.insetBy(dx: -margin, dy: -margin))
        guard !drawn.isEmpty else { return }
        let bucket = AccessThinning.bucket(mapPointsPerPoint: perPoint)
        for item in drawn {
            let shape = item.shape
            guard AccessThinning.visible(shape, mapPointsPerPoint: perPoint),
                let path = item.path(bucket: bucket)
            else { continue }
            let colour = AccessDrawing.uiColor(shape.role.colour)
            let alpha = CGFloat(item.opacity)
            let origin = point(for: shape.bounds.origin)
            context.saveGState()
            context.translateBy(x: origin.x, y: origin.y)
            context.setLineJoin(.round)
            context.setLineCap(.round)
            switch shape.form {
            case .area:
                context.addPath(path)
                context.setFillColor(colour.withAlphaComponent(0.18 * alpha).cgColor)
                context.fillPath(using: .evenOdd)
                context.addPath(path)
                context.setStrokeColor(colour.withAlphaComponent(0.85 * alpha).cgColor)
                context.setLineWidth(CGFloat(1.5 * perPoint))
                if !shape.role.filled {
                    context.setLineDash(
                        phase: 0, lengths: [CGFloat(4 * perPoint), CGFloat(3 * perPoint)])
                }
                context.strokePath()
            case .line:
                context.addPath(path)
                context.setStrokeColor(UIColor.white.withAlphaComponent(0.8 * alpha).cgColor)
                context.setLineWidth(CGFloat(5 * perPoint))
                context.strokePath()
                context.addPath(path)
                context.setStrokeColor(colour.withAlphaComponent(alpha).cgColor)
                context.setLineWidth(CGFloat(3 * perPoint))
                context.strokePath()
            }
            context.restoreGState()
        }
    }
}

/// Every access and stocking mark as one overlay with one renderer. The groups
/// (``AccessCluster``) are set by the ``AccessModule`` for the zoom shown.
nonisolated final class AccessPinOverlay: NSObject, MKOverlay {
    let coordinate = CLLocationCoordinate2D(latitude: 0, longitude: 0)
    let boundingMapRect = MKMapRect.world
    private let lock = NSLock()
    private var clusters: [AccessCluster] = []
    private var opacity: [String: Double] = [:]

    /// Replaces the marks and the layers' opacities; the renderer is told to draw again by its
    /// owner.
    func set(_ clusters: [AccessCluster], opacity: [String: Double]) {
        lock.withLock {
            self.clusters = clusters
            self.opacity = opacity
        }
    }

    /// The marks whose centre lies in `rect`, with the opacity of the layer of each one's lead.
    func clusters(in rect: MKMapRect) -> [(AccessCluster, Double)] {
        lock.withLock {
            clusters.compactMap { cluster in
                guard rect.contains(MKMapPoint(x: cluster.x, y: cluster.y)),
                    let lead = cluster.marks.first
                else { return nil }
                return (cluster, opacity[lead.layer] ?? 1)
            }
        }
    }
}

/// Draws an ``AccessPinOverlay``: each place as a marker in its access colour with its kind's
/// symbol, a badge for a restricted or closed place, a dashed outline for an unknown access; a
/// group as the mark that leads it with their number beside.
nonisolated final class AccessPinRenderer: MKOverlayRenderer {
    /// The largest mark's half size with its badges, in points.
    static let margin = 30.0

    private let pins: AccessPinOverlay

    init(_ overlay: AccessPinOverlay) {
        pins = overlay
        super.init(overlay: overlay)
    }

    override func draw(_ mapRect: MKMapRect, zoomScale: MKZoomScale, in context: CGContext) {
        let perPoint = 1 / Double(zoomScale)
        let margin = Self.margin * perPoint
        let marks = pins.clusters(in: mapRect.insetBy(dx: -margin, dy: -margin))
        guard !marks.isEmpty else { return }
        UIGraphicsPushContext(context)
        defer { UIGraphicsPopContext() }
        for (mark, opacity) in marks {
            guard let lead = mark.marks.first else { continue }
            let at = point(for: MKMapPoint(x: mark.x, y: mark.y))
            context.saveGState()
            // From here on, one unit is one point on screen, centred on the mark.
            context.translateBy(x: at.x, y: at.y)
            context.scaleBy(x: CGFloat(perPoint), y: CGFloat(perPoint))
            context.setAlpha(CGFloat(opacity))
            AccessDrawing.mark(lead.kind, lead.role, count: mark.marks.count, in: context)
            context.restoreGState()
        }
    }
}

/// The drawing of one mark or group, centred on the origin, in points. Used by the renderer (on
/// MapKit's drawing threads) and for the symbols in the place card.
nonisolated enum AccessDrawing {
    /// A marker's size on screen, in points.
    static let size = 26.0
    /// Public land's marker is smaller: the area itself is drawn.
    static let landSize = 18.0

    private static let cache = SymbolCache()

    static func uiColor(_ rgb: PinRGB, alpha: CGFloat = 1) -> UIColor {
        UIColor(
            red: CGFloat(rgb.red) / 255, green: CGFloat(rgb.green) / 255,
            blue: CGFloat(rgb.blue) / 255, alpha: alpha)
    }

    /// One mark: a rounded tag (a circle for public land) with a dark rim and a light edge, so
    /// it holds on imagery and on the light map, filled in its access colour (white with a
    /// dashed edge for an unknown access), its kind's symbol, the badge of a restricted or
    /// closed place, and with `count` above one, the number of places it leads.
    static func mark(
        _ kind: AccessMarkKind, _ role: AccessMarkRole, count: Int = 1, in context: CGContext
    ) {
        let land = kind == .publicLand
        let side = land ? landSize : size
        let path = outline(land: land, size: side)
        let colour = uiColor(role.colour)

        context.addPath(path)
        context.setStrokeColor(UIColor.black.withAlphaComponent(0.55).cgColor)
        context.setLineWidth(4.5)
        context.setLineJoin(.round)
        context.strokePath()
        context.addPath(path)
        context.setFillColor(role.filled ? colour.cgColor : UIColor.white.cgColor)
        context.fillPath()
        context.addPath(path)
        if role.filled {
            context.setStrokeColor(UIColor.white.cgColor)
            context.setLineWidth(2)
        } else {
            context.setStrokeColor(colour.cgColor)
            context.setLineWidth(2.5)
            context.setLineDash(phase: 0, lengths: [3, 2.5])
        }
        context.strokePath()
        context.setLineDash(phase: 0, lengths: [])

        let tint = role.filled ? uiColor(role.glyph) : colour
        if let image = cache.image(kind.symbol, tint: tint, pointSize: land ? 9 : 12) {
            let size = image.size
            image.draw(
                in: CGRect(
                    x: -size.width / 2, y: -size.height / 2, width: size.width,
                    height: size.height))
        }
        let corner = side / 2
        switch role.badge {
        case .none: break
        case .caution:
            badge("exclamationmark", at: CGPoint(x: count > 1 ? -corner : corner, y: -corner), in: context)
        case .closed:
            badge("xmark", at: CGPoint(x: count > 1 ? -corner : corner, y: -corner), in: context)
        }
        if count > 1 { number(count, corner: corner, in: context) }
    }

    /// The outline of a mark `size` points across, centred on the origin.
    static func outline(land: Bool, size: Double) -> CGPath {
        let h = size / 2
        if land {
            return CGPath(ellipseIn: CGRect(x: -h, y: -h, width: size, height: size), transform: nil)
        }
        return CGPath(
            roundedRect: CGRect(x: -h, y: -h, width: size, height: size), cornerWidth: 7,
            cornerHeight: 7, transform: nil)
    }

    /// The number of places in a group, on an ink capsule at the lead mark's top right.
    private static func number(_ count: Int, corner: Double, in context: CGContext) {
        // A count of marks on screen, not a measured value: nothing for the core to word.
        let text = NSAttributedString(
            string: String(count),
            attributes: [
                .font: UIFont.systemFont(ofSize: 10, weight: .bold),
                .foregroundColor: UIColor.white,
            ])
        let size = text.size()
        let width = max(size.width + 7, 16), height = 16.0
        let capsule = CGRect(
            x: corner + 2 - width / 2, y: -corner - 3 - height / 2, width: width, height: height)
        let path = CGPath(
            roundedRect: capsule, cornerWidth: height / 2, cornerHeight: height / 2, transform: nil)
        context.addPath(path)
        context.setFillColor(uiColor(.ink).cgColor)
        context.fillPath()
        context.addPath(path)
        context.setStrokeColor(UIColor.white.cgColor)
        context.setLineWidth(1.5)
        context.strokePath()
        text.draw(at: CGPoint(x: capsule.midX - size.width / 2, y: capsule.midY - size.height / 2))
    }

    /// A white disc with an ink symbol: the badge of a restricted or closed place.
    private static func badge(_ symbol: String, at centre: CGPoint, in context: CGContext) {
        let r = 6.5
        let disc = CGRect(x: centre.x - r, y: centre.y - r, width: 2 * r, height: 2 * r)
        context.setFillColor(UIColor.white.cgColor)
        context.fillEllipse(in: disc)
        context.setStrokeColor(uiColor(.ink).cgColor)
        context.setLineWidth(1.5)
        context.strokeEllipse(in: disc)
        if let image = cache.image(symbol, tint: uiColor(.ink), pointSize: 7) {
            let size = image.size
            image.draw(
                in: CGRect(
                    x: centre.x - size.width / 2, y: centre.y - size.height / 2,
                    width: size.width, height: size.height))
        }
    }

    /// SF Symbol images by name, tint and size, made once; MapKit draws on several threads.
    private final class SymbolCache: @unchecked Sendable {
        private let lock = NSLock()
        private var images: [String: UIImage] = [:]

        func image(_ name: String, tint: UIColor, pointSize: CGFloat) -> UIImage? {
            let key = "\(name)|\(tint.description)|\(pointSize)"
            if let held = lock.withLock({ images[key] }) { return held }
            let configuration = UIImage.SymbolConfiguration(pointSize: pointSize, weight: .bold)
            guard
                let made = UIImage(systemName: name, withConfiguration: configuration)?
                    .withTintColor(tint, renderingMode: .alwaysOriginal)
            else { return nil }
            lock.withLock { images[key] = made }
            return made
        }
    }
}
