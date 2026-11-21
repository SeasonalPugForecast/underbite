import MapKit
import UIKit

/// Every station and spot pin on the map as one overlay with one renderer, however many there
/// are. The marks (``PinCluster``) are set by the ``StationsModule`` for the zoom shown; the
/// renderer draws them a fixed size on screen.
nonisolated final class StationPinOverlay: NSObject, MKOverlay {
    let coordinate = CLLocationCoordinate2D(latitude: 0, longitude: 0)
    let boundingMapRect = MKMapRect.world
    private let lock = NSLock()
    private var clusters: [PinCluster] = []

    private var dimmed = false

    /// Replaces the marks; the renderer is told to draw again by its owner. `dimmed` draws the
    /// stations lighter (see ``StationPins/dimsStations(shown:areaFilling:)``).
    func set(_ clusters: [PinCluster], dimmed: Bool = false) {
        lock.withLock {
            self.clusters = clusters
            self.dimmed = dimmed
        }
    }

    var isDimmed: Bool { lock.withLock { dimmed } }

    /// The marks drawn whose centre lies in `rect`.
    func clusters(in rect: MKMapRect) -> [PinCluster] {
        lock.withLock {
            clusters.filter { $0.drawn && rect.contains(MKMapPoint(x: $0.x, y: $0.y)) }
        }
    }
}

/// Draws a ``StationPinOverlay``: each pin in its kind's shape and symbol, filled with its
/// state's colour (an outline for an unknown state, a "!" badge for a stage that asks for
/// attention); a group of pins as the pin that leads it with their number beside.
nonisolated final class StationPinRenderer: MKOverlayRenderer {
    /// The pins' size on screen, in points.
    static let size = 22.0
    /// The largest mark's half size with its outline, in points: marks this far outside a tile
    /// still reach into it.
    static let margin = 30.0

    private let pins: StationPinOverlay

    init(_ overlay: StationPinOverlay) {
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
        let dimmed = pins.isDimmed
        for mark in marks {
            let at = point(for: MKMapPoint(x: mark.x, y: mark.y))
            context.saveGState()
            // From here on, one unit is one point on screen, centred on the mark.
            context.translateBy(x: at.x, y: at.y)
            context.scaleBy(x: CGFloat(perPoint), y: CGFloat(perPoint))
            if let lead = mark.pins.first {
                let light = dimmed && lead.kind != .spot
                if light {
                    context.scaleBy(x: StationPins.dimmedScale, y: StationPins.dimmedScale)
                    context.setAlpha(StationPins.dimmedAlpha)
                    context.beginTransparencyLayer(auxiliaryInfo: nil)
                }
                PinDrawing.pin(lead.kind, lead.role, count: mark.pins.count, in: context)
                if light { context.endTransparencyLayer() }
            }
            context.restoreGState()
        }
    }
}

/// The drawing of one pin or group, centred on the origin, in points. Used by the renderer (on
/// MapKit's drawing threads) and for the symbols in the pin sheet.
nonisolated enum PinDrawing {
    private static let cache = SymbolCache()

    static func uiColor(_ rgb: PinRGB, alpha: CGFloat = 1) -> UIColor {
        UIColor(
            red: CGFloat(rgb.red) / 255, green: CGFloat(rgb.green) / 255,
            blue: CGFloat(rgb.blue) / 255, alpha: alpha)
    }

    /// One pin: its kind's outline with a light edge and a dark rim (so it holds on imagery and
    /// on the light map), its state's fill, its kind's symbol, and the badge of an alert; with
    /// `count` above one, the pin leading a group, with their number at its top right (and the
    /// alert badge at its top left).
    static func pin(_ kind: PinKind, _ role: PinRole, count: Int = 1, in context: CGContext) {
        let path = outline(kind.shape, size: StationPinRenderer.size)
        let filled = role.mark == .solid || role.mark == .alert
        let colour = uiColor(role.colour)
        let fill = kind == .spot ? uiColor(PinRGB(0x005BB5)) : colour

        context.addPath(path)
        context.setStrokeColor(UIColor.black.withAlphaComponent(0.55).cgColor)
        context.setLineWidth(4.5)
        context.setLineJoin(.round)
        context.strokePath()
        context.addPath(path)
        context.setFillColor(filled || kind == .spot ? fill.cgColor : UIColor.white.cgColor)
        context.fillPath()
        context.addPath(path)
        if filled || kind == .spot {
            context.setStrokeColor(UIColor.white.cgColor)
            context.setLineWidth(2)
        } else {
            context.setStrokeColor(colour.cgColor)
            context.setLineWidth(2.5)
            if role.mark == .unknown { context.setLineDash(phase: 0, lengths: [3, 2.5]) }
        }
        context.strokePath()
        context.setLineDash(phase: 0, lengths: [])

        let tint = kind == .spot ? UIColor.white : (filled ? uiColor(role.glyph) : colour)
        if let image = cache.image(kind.symbol, tint: tint, pointSize: 10) {
            let size = image.size
            image.draw(
                in: CGRect(
                    x: -size.width / 2, y: -size.height / 2 + glyphOffset(kind.shape),
                    width: size.width, height: size.height))
        }
        if role.mark == .alert { badge(at: CGPoint(x: count > 1 ? -9 : 9, y: -9), in: context) }
        if count > 1 { number(count, in: context) }
    }

    /// The number of pins in a group, on an ink capsule at the lead pin's top right.
    private static func number(_ count: Int, in context: CGContext) {
        // A count of marks on screen, not a measured value: nothing for the core to word.
        let text = NSAttributedString(
            string: String(count),
            attributes: [
                .font: UIFont.systemFont(ofSize: 9, weight: .bold),
                .foregroundColor: UIColor.white,
            ])
        let size = text.size()
        let width = max(size.width + 6, 14), height = 14.0
        let capsule = CGRect(x: 12 - width / 2, y: -13 - height / 2, width: width, height: height)
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

    /// The "!" badge of an alert pin.
    private static func badge(at centre: CGPoint, in context: CGContext) {
        let r = 6.0
        let disc = CGRect(x: centre.x - r, y: centre.y - r, width: 2 * r, height: 2 * r)
        context.setFillColor(UIColor.white.cgColor)
        context.fillEllipse(in: disc)
        context.setStrokeColor(uiColor(.ink).cgColor)
        context.setLineWidth(1.5)
        context.strokeEllipse(in: disc)
        if let image = cache.image("exclamationmark", tint: uiColor(.ink), pointSize: 7) {
            let size = image.size
            image.draw(
                in: CGRect(
                    x: centre.x - size.width / 2, y: centre.y - size.height / 2,
                    width: size.width, height: size.height))
        }
    }

    /// Moves a symbol to the visual middle of an outline that is not symmetric.
    private static func glyphOffset(_ shape: PinShape) -> CGFloat {
        switch shape {
        case .shield: -1.5
        case .drop: -2.5
        default: 0
        }
    }

    /// The outline of a pin `size` points across, centred on the origin.
    static func outline(_ shape: PinShape, size: Double) -> CGPath {
        let h = size / 2
        switch shape {
        case .circle:
            return CGPath(ellipseIn: CGRect(x: -h, y: -h, width: size, height: size), transform: nil)
        case .square:
            let s = h * 0.9
            return CGPath(
                roundedRect: CGRect(x: -s, y: -s, width: 2 * s, height: 2 * s), cornerWidth: 4,
                cornerHeight: 4, transform: nil)
        case .diamond:
            let d = h * 1.12
            let path = CGMutablePath()
            path.move(to: CGPoint(x: 0, y: -d))
            path.addLine(to: CGPoint(x: d, y: 0))
            path.addLine(to: CGPoint(x: 0, y: d))
            path.addLine(to: CGPoint(x: -d, y: 0))
            path.closeSubpath()
            return path
        case .hexagon:
            let path = CGMutablePath()
            for i in 0..<6 {
                let angle = Double(i) * .pi / 3
                let p = CGPoint(x: h * 1.05 * cos(angle), y: h * 1.05 * sin(angle))
                if i == 0 { path.move(to: p) } else { path.addLine(to: p) }
            }
            path.closeSubpath()
            return path
        case .shield:
            let path = CGMutablePath()
            let w = h * 0.95
            path.move(to: CGPoint(x: -w, y: -h + 3))
            path.addQuadCurve(to: CGPoint(x: -w + 3, y: -h), control: CGPoint(x: -w, y: -h))
            path.addLine(to: CGPoint(x: w - 3, y: -h))
            path.addQuadCurve(to: CGPoint(x: w, y: -h + 3), control: CGPoint(x: w, y: -h))
            path.addLine(to: CGPoint(x: w, y: h * 0.2))
            path.addLine(to: CGPoint(x: 0, y: h * 1.1))
            path.addLine(to: CGPoint(x: -w, y: h * 0.2))
            path.closeSubpath()
            return path
        case .drop:
            // A map pin: a disc with a point below, the point at the bottom of the mark.
            let r = h * 0.9
            let centre = CGPoint(x: 0, y: -h * 0.25)
            let path = CGMutablePath()
            path.addArc(
                center: centre, radius: r, startAngle: .pi * 0.8, endAngle: .pi * 0.2,
                clockwise: false)
            path.addLine(to: CGPoint(x: 0, y: h * 1.15))
            path.closeSubpath()
            return path
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
